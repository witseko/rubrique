//! Confirm explorer deletions, then remove the entries and close their tabs.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Context};

use super::RubriqueApp;
use crate::alert::{self, ButtonKind};
use crate::explorer::paths::{absolute_path_text, name_text};
use crate::explorer::tree::FsNode;
use crate::media::image_uri;
use crate::remote;

/// Width of the "Delete" dialog.
const DIALOG_WIDTH: f32 = 380.0;
/// Maximum height of the list of files in a batch confirmation.
const FILE_LIST_HEIGHT: f32 = 120.0;

impl RubriqueApp {
    /// Ask whether to delete the requested files or folder.
    pub(super) fn request_delete(&mut self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.deleting = Some(paths);
        }
    }

    /// Draw the "Delete" dialog while it is open.
    pub(super) fn delete_dialog(&mut self, ctx: &Context) {
        let Some(paths) = self.deleting.as_deref() else {
            return;
        };
        let has_unsaved = std::iter::once(&self.open_file).chain(&self.tabs).any(|tab| {
            tab.is_dirty() && tab.path().is_some_and(|open| paths.iter().any(|path| open.starts_with(path)))
        });
        let (title, explanation, confirm_label) = delete_prompt(paths);
        let modal = alert::show(ctx, "delete-file", DIALOG_WIDTH, |ui| {
            alert::title(ui, &title);
            alert::body(ui, explanation);
            if paths.len() > 1 {
                egui::ScrollArea::vertical().max_height(FILE_LIST_HEIGHT).show(ui, |ui| {
                    for path in paths {
                        ui.label(name_text(path)).on_hover_text(absolute_path_text(path));
                    }
                });
            }
            if has_unsaved {
                ui.colored_label(ui.visuals().warn_fg_color, "Unsaved changes to these files will be lost.");
            }
            // Enter takes Cancel, never the delete: deleting needs a click
            // or Tab to the button.
            let ((), (confirm, cancel)) = alert::button_row(
                ui,
                |_| (),
                |ui| {
                    let confirm = alert::button(ui, confirm_label, ButtonKind::Delete, false);
                    let cancel = alert::button(ui, "Cancel", ButtonKind::Secondary, true);
                    (confirm, cancel)
                },
            );
            let enter = alert::enter_for_default(ui);
            (confirm.clicked(), cancel.clicked() || enter)
        });
        let (confirmed, cancelled) = modal.inner;
        if cancelled || modal.should_close() {
            self.deleting = None;
        } else if confirmed && let Some(paths) = self.deleting.take() {
            self.delete_paths(&paths);
        }
    }

    /// Try every entry, refresh once, and report any partial failure.
    fn delete_paths(&mut self, paths: &[PathBuf]) {
        let mut deleted = 0;
        let mut first_error = None;
        for path in paths {
            match self.delete_now(path) {
                Ok(()) => deleted += 1,
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if deleted > 0 {
            self.roots.iter_mut().for_each(FsNode::refresh);
        }
        let status = match (paths, first_error) {
            ([_], Some(error)) => error,
            ([path], None) => {
                let place = if remote::is_mirror(path) { "Deleted" } else { "Moved to the Trash:" };
                format!("{place} {}", name_text(path))
            }
            (_, Some(error)) => format!("Deleted {deleted} of {} files. {error}", paths.len()),
            (_, None) => format!("Deleted {deleted} files"),
        };
        self.show_status(status);
    }

    /// Remove one entry and clear its tabs and cached state after success.
    fn delete_now(&mut self, path: &Path) -> Result<(), String> {
        // The upload would bring the file back.
        if self.remote_save_running(path) {
            return Err(format!("Cannot delete {} while it is being saved", name_text(path)));
        }
        crate::vfs::delete(path).map_err(|error| format!("Cannot delete {}: {error}", name_text(path)))?;
        // The tab would otherwise offer to save the file back.
        self.close_tabs_inside(path);
        self.repaint.forget_image(&image_uri(path));
        if self.settings.last_file.as_deref().is_some_and(|last| last.starts_with(path)) {
            self.settings.last_file = None;
        }
        Ok(())
    }
}

/// Describe how the targets will be deleted, including mixed local and remote selections.
fn delete_prompt(paths: &[PathBuf]) -> (String, &'static str, &'static str) {
    let remote_count = paths.iter().filter(|path| remote::is_mirror(path)).count();
    let (explanation, confirm_label) = match paths {
        [_] if remote_count == 0 => ("It goes to the Trash, where you can restore it.", "Move to Trash"),
        [_] => ("It is in a remote folder, which has no Trash: it is deleted for good.", "Delete"),
        _ if remote_count == 0 => ("These files go to the Trash, where you can restore them.", "Move to Trash"),
        _ if remote_count == paths.len() => {
            ("These files are permanently deleted from their remote folders.", "Delete")
        }
        _ => ("Local files go to the Trash. Remote files are permanently deleted.", "Delete"),
    };
    let title = match paths {
        [path] => format!("Delete \u{201c}{}\u{201d}?", name_text(path)),
        _ => format!("Delete {} files?", paths.len()),
    };
    (title, explanation, confirm_label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use crate::test_support::TempFolder;

    #[test]
    fn batch_deletion_continues_past_a_failure_and_leaves_other_files() {
        let temp = TempFolder::new();
        let first = temp.write("first.md", "first");
        let folder = temp.make_folder("folder");
        let last = temp.write("last.md", "last");
        let other = temp.write("other.md", "other");
        let mirror = |path| remote::testing::mirror_of("delete-test", path);
        let mut app = RubriqueApp::new(&Context::default(), Settings::default(), vec![mirror(temp.path())], None);

        app.delete_paths(&[mirror(&first), mirror(&folder), mirror(&last)]);

        assert!(!first.exists());
        assert!(folder.is_dir());
        assert!(!last.exists());
        assert!(other.is_file());
        let (status, _) = app.status_message.expect("the app reports the deletion result");
        assert!(status.starts_with("Deleted 2 of 3 files. Cannot delete folder:"));
    }
}
