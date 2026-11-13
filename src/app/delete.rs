//! "Delete…" in the explorer: ask first, then move the file to the Trash
//! (or delete it for good on a remote folder) and close its tab.

use std::path::{Path, PathBuf};

use eframe::egui::Context;

use super::RubriqueApp;
use crate::alert::{self, ButtonKind};
use crate::explorer::paths::name_text;
use crate::explorer::tree::FsNode;
use crate::media::image_uri;
use crate::remote;

/// Width of the "Delete" dialog.
const DIALOG_WIDTH: f32 = 380.0;

impl RubriqueApp {
    /// Ask whether to delete `path` (the dialog runs in [`Self::delete_dialog`]).
    pub(super) fn request_delete(&mut self, path: PathBuf) {
        self.deleting = Some(path);
    }

    /// Draw the "Delete" dialog while it is open.
    pub(super) fn delete_dialog(&mut self, ctx: &Context) {
        let Some(path) = self.deleting.clone() else {
            return;
        };
        let is_remote = remote::is_mirror(&path);
        let has_unsaved =
            self.all_tabs_mut().any(|tab| tab.path().is_some_and(|open| open.starts_with(&path)) && tab.is_dirty());
        let confirm_label = if is_remote { "Delete" } else { "Move to Trash" };
        let title = format!("Delete \u{201c}{}\u{201d}?", name_text(&path));
        let modal = alert::show(ctx, "delete-file", DIALOG_WIDTH, |ui| {
            alert::title(ui, &title);
            let explanation = if is_remote {
                "It is in a remote folder, which has no Trash: it is deleted for good."
            } else {
                "It goes to the Trash, where you can restore it."
            };
            alert::body(ui, explanation);
            if has_unsaved {
                ui.colored_label(ui.visuals().warn_fg_color, "Its unsaved changes are lost too.");
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
        } else if confirmed {
            self.deleting = None;
            self.delete_now(&path);
        }
    }

    /// Delete the file, close its tab and re-read the tree.
    fn delete_now(&mut self, path: &Path) {
        // The upload would bring the file back.
        if self.remote_save_running(path) {
            self.show_status(format!("Cannot delete {} while it is being saved", name_text(path)));
            return;
        }
        if let Err(error) = crate::vfs::delete(path) {
            self.show_status(format!("Cannot delete {}: {error}", name_text(path)));
            return;
        }
        // The tab would otherwise offer to save the file back.
        self.close_tabs_inside(path);
        self.repaint.forget_image(&image_uri(path));
        if self.settings.last_file.as_deref().is_some_and(|last| last.starts_with(path)) {
            self.settings.last_file = None;
        }
        self.roots.iter_mut().for_each(FsNode::refresh);
        let place = if remote::is_mirror(path) { "Deleted" } else { "Moved to the Trash:" };
        self.show_status(format!("{place} {}", name_text(path)));
    }
}
