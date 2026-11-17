//! "Clone repository": paste an https or SSH address of a git repository
//! (GitHub or any other host), choose the folder to clone into, and the
//! clone shows up as a new explorer folder. Git runs on a background
//! thread; the dialog stays open with a spinner until it ends, and shows
//! git's reason when it fails.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Context, Key, RichText};

use super::RubriqueApp;
use crate::alert::{self, ButtonKind};
use crate::git;
use crate::remote::task::{Task, spawn_with_repaint};
use crate::scratch::check_name;

/// Width of the "Clone repository" dialog.
const DIALOG_WIDTH: f32 = 460.0;

/// State of the "Clone repository" dialog while it is open.
#[derive(Debug, Default)]
pub(super) struct CloneDialog {
    /// The repository address being typed.
    url: String,
    /// The folder the clone goes into, as a new folder inside it.
    parent: Option<PathBuf>,
    /// The new folder's name; follows the address until the user edits it.
    name: String,
    /// The user typed their own name, so the address no longer fills it.
    name_edited: bool,
    /// Why the last attempt failed, shown above the buttons.
    error: Option<String>,
    /// The clone in flight, with the folder it makes.
    cloning: Option<(PathBuf, Task<Result<String, String>>)>,
}

/// What the user did in the dialog this frame.
#[derive(Clone, Copy, Default)]
struct DialogInput {
    clone: bool,
    cancel: bool,
    choose_parent: bool,
}

impl RubriqueApp {
    /// Show the "Clone repository" dialog (drawn by
    /// [`Self::clone_repo_dialog`]). It starts in the folder that holds the
    /// first explorer folder, where other repositories likely live.
    pub(super) fn start_clone_repo(&mut self) {
        let parent = self
            .roots
            .iter()
            .find(|root| !crate::remote::is_mirror(&root.path))
            .and_then(|root| root.path.parent().map(Path::to_path_buf))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from));
        self.clone_repo = Some(CloneDialog { parent, ..CloneDialog::default() });
    }

    /// Draw the "Clone repository" dialog while it is open, and pick up a
    /// finished clone.
    pub(super) fn clone_repo_dialog(&mut self, ctx: &Context) {
        self.finish_clone();
        let Some(dialog) = &mut self.clone_repo else {
            return;
        };
        let busy = dialog.cloning.is_some();
        let modal = alert::show(ctx, "clone-repository", DIALOG_WIDTH, |ui| dialog_contents(ui, dialog));
        let input = modal.inner;
        // A running clone cannot be stopped halfway, so the dialog waits for it.
        if busy {
            return;
        }
        if input.cancel || modal.should_close() {
            self.clone_repo = None;
            return;
        }
        if input.choose_parent {
            let mut picker = rfd::FileDialog::new().set_title("Clone into this folder");
            if let Some(parent) = &dialog.parent {
                picker = picker.set_directory(parent);
            }
            if let Some(chosen) = picker.pick_folder() {
                dialog.parent = Some(chosen);
                dialog.error = None;
            }
        }
        if input.clone {
            match clone_destination(dialog) {
                Ok(destination) => {
                    let url = dialog.url.trim().to_string();
                    let folder = destination.clone();
                    let task = spawn_with_repaint(ctx, move || git::clone_repository(&url, &folder));
                    dialog.error = None;
                    dialog.cloning = Some((destination, task));
                }
                Err(error) => dialog.error = Some(error),
            }
        }
    }

    /// When the clone is done: add its folder to the explorer and close the
    /// dialog, or show why it failed and keep the dialog for another try.
    fn finish_clone(&mut self) {
        let Some(dialog) = &mut self.clone_repo else {
            return;
        };
        let Some(result) = dialog.cloning.as_ref().and_then(|(_, task)| task.take()) else {
            return;
        };
        let Some((destination, _)) = dialog.cloning.take() else {
            return;
        };
        match result {
            Ok(message) => {
                self.clone_repo = None;
                self.add_folder(destination);
                self.show_status(message);
            }
            Err(reason) => dialog.error = Some(format!("Git: {reason}")),
        }
    }
}

/// The dialog's fields and buttons; returns what the user did.
fn dialog_contents(ui: &mut egui::Ui, dialog: &mut CloneDialog) -> DialogInput {
    let mut input = DialogInput::default();
    let busy = dialog.cloning.is_some();
    alert::title(ui, "Clone repository");
    alert::body(ui, "Clone a git repository and add it to the explorer.");
    let mut entered = false;
    ui.add_enabled_ui(!busy, |ui| {
        alert::field_label(ui, "Repository address");
        let url_field = alert::text_field(
            ui,
            egui::TextEdit::singleline(&mut dialog.url)
                .hint_text("https://github.com/owner/repo or git@github.com:owner/repo.git")
                .desired_width(f32::INFINITY),
        );
        if !busy && dialog.url.is_empty() && !url_field.has_focus() && !url_field.lost_focus() {
            url_field.request_focus();
        }
        if url_field.changed() {
            dialog.error = None;
            if !dialog.name_edited {
                dialog.name = git::repository_name(&dialog.url).unwrap_or_default();
            }
        }
        alert::field_label(ui, "Clone into");
        ui.horizontal(|ui| {
            let parent = dialog
                .parent
                .as_deref()
                .map_or_else(|| "No folder chosen".to_string(), |path| path.display().to_string());
            input.choose_parent = alert::button(ui, "Choose...", ButtonKind::Secondary, false).clicked();
            ui.add(egui::Label::new(RichText::new(parent).monospace()).truncate());
        });
        alert::field_label(ui, "Folder name");
        let name_field =
            alert::text_field(ui, egui::TextEdit::singleline(&mut dialog.name).desired_width(f32::INFINITY));
        if name_field.changed() {
            dialog.error = None;
            dialog.name_edited = !dialog.name.trim().is_empty();
        }
        entered =
            (url_field.lost_focus() || name_field.lost_focus()) && ui.input(|input| input.key_pressed(Key::Enter));
    });
    if let Some((destination, _)) = &dialog.cloning {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0));
            ui.label(RichText::new(format!("Cloning into {}…", destination.display())).weak());
        });
    }
    if let Some(error) = &dialog.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    let ((), (clone, cancel)) = alert::button_row(
        ui,
        |_| (),
        |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                let clone = alert::button(ui, "Clone", ButtonKind::Primary, true).clicked();
                let cancel = alert::button(ui, "Cancel", ButtonKind::Secondary, false).clicked();
                (clone, cancel)
            })
            .inner
        },
    );
    input.clone = !busy && (clone || entered || alert::enter_for_default(ui));
    input.cancel = cancel;
    input
}

/// The folder the clone will make, after checking the address, the
/// chosen folder and the name.
fn clone_destination(dialog: &CloneDialog) -> Result<PathBuf, String> {
    let url = dialog.url.trim();
    if let Some(problem) = git::clone_url_problem(url) {
        return Err(problem.to_string());
    }
    let parent = dialog.parent.as_deref().ok_or("Choose a folder to clone into")?;
    if !parent.is_dir() {
        return Err(format!("No such folder: {}", parent.display()));
    }
    let name = check_name(&dialog.name)?;
    let destination = parent.join(name);
    if destination.exists() {
        return Err(format!("{name} already exists in that folder"));
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    fn dialog(url: &str, parent: Option<&Path>, name: &str) -> CloneDialog {
        CloneDialog {
            url: url.to_string(),
            parent: parent.map(Path::to_path_buf),
            name: name.to_string(),
            ..CloneDialog::default()
        }
    }

    #[test]
    fn destination_is_the_name_inside_the_chosen_folder() {
        let temp = TempFolder::new();
        let ready = dialog(" git@github.com:owner/repo.git ", Some(temp.path()), "repo");
        assert_eq!(clone_destination(&ready), Ok(temp.path().join("repo")));
    }

    #[test]
    fn destination_refuses_what_cannot_be_cloned() {
        let temp = TempFolder::new();
        temp.make_folder("repo");
        let taken = dialog("https://github.com/owner/repo", Some(temp.path()), "repo");
        assert_eq!(clone_destination(&taken), Err("repo already exists in that folder".to_string()));
        let no_folder = dialog("https://github.com/owner/repo", None, "repo");
        assert_eq!(clone_destination(&no_folder), Err("Choose a folder to clone into".to_string()));
        assert!(clone_destination(&dialog("not a url", Some(temp.path()), "repo")).is_err());
        assert!(clone_destination(&dialog("https://github.com/owner/repo", Some(temp.path()), "a/b")).is_err());
        let missing = temp.path().join("missing");
        assert!(clone_destination(&dialog("https://github.com/owner/repo", Some(&missing), "repo")).is_err());
    }
}
