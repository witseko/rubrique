//! Move explorer files to a folder chosen in the native picker.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rustix::fs::{CWD, RenameFlags, renameat_with};

use super::{RubriqueApp, privacy::AfterPassword, scratch_paths};
use crate::explorer::paths::name_text;
use crate::explorer::tree::FsNode;
use crate::settings::Settings;
use crate::{crypto, remote, terminal};

impl RubriqueApp {
    /// Open the system folder picker and move the selected files on confirmation.
    pub(super) fn choose_move_folder(&mut self, paths: &[PathBuf]) {
        let title = if paths.len() == 1 { "Move File to Folder" } else { "Move Files to Folder" };
        let mut dialog = rfd::FileDialog::new().set_title(title).set_can_create_directories(true);
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let dev = home.join("dev");
            dialog = dialog.set_directory(if dev.is_dir() { dev } else { home });
        }
        if let Some(folder) = dialog.pick_folder()
            && let Err(error) = self.move_files_to(paths, &folder, &Settings::save)
        {
            self.show_status(error);
        }
    }

    /// Move the batch, or pause for the password before changing any files.
    pub(super) fn move_files_to(
        &mut self,
        paths: &[PathBuf],
        folder: &Path,
        save_settings: &impl Fn(&Settings) -> io::Result<()>,
    ) -> Result<(), String> {
        let folder =
            fs::canonicalize(folder).map_err(|error| format!("Cannot open the destination folder: {error}"))?;
        let moves = plan_moves(paths, &folder)?;
        if moves.is_empty() {
            self.show_status("The selected files are already in that folder".into());
            return Ok(());
        }
        for (_, target) in &moves {
            if self.all_tabs_mut().any(|tab| tab.path() == Some(target)) {
                return Err(format!("Close the destination tab for {} before moving it", name_text(target)));
            }
        }
        if self.wait_for_move_password(&moves, &folder)? {
            return Ok(());
        }
        self.finish_block_edit();
        self.remember_tabs();
        let result = self.apply_moves(&moves, save_settings);
        let completed = result.as_ref().map_or_else(|(count, _)| *count, |()| moves.len());
        if completed > 0 {
            self.roots.iter_mut().for_each(FsNode::refresh);
            if self.roots.iter().any(|root| folder.starts_with(&root.path)) {
                self.sort_tabs_into_groups();
            } else {
                self.add_folder(folder);
            }
        }
        result.map_err(|(_, error)| error)
    }

    /// Open the unlock dialog if a draft in the batch needs the scratch key.
    fn wait_for_move_password(&mut self, moves: &[(PathBuf, PathBuf)], folder: &Path) -> Result<bool, String> {
        if self.privacy.vault.is_some() {
            return Ok(false);
        }
        for (source, _) in moves {
            let draft = self
                .drafts
                .sealed_draft_path(source)
                .map_err(|error| format!("Cannot check unsaved edits for {}: {error}", name_text(source)))?;
            if let Some(encrypted_draft) = draft {
                let paths = moves.iter().map(|(source, _)| source.clone()).collect();
                let after = AfterPassword::MoveFiles { paths, folder: folder.to_path_buf(), encrypted_draft };
                let _ = self.scratch_vault_or_ask(after);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Stop at the first failed move. Each completed move has already updated its tab.
    fn apply_moves(
        &mut self,
        moves: &[(PathBuf, PathBuf)],
        save_settings: &impl Fn(&Settings) -> io::Result<()>,
    ) -> Result<(), (usize, String)> {
        for (count, (source, target)) in moves.iter().enumerate() {
            if let Err(error) = self.move_file(source, target, save_settings) {
                let message =
                    format!("Moved {count} of {} files. Cannot move {}: {error}", moves.len(), name_text(source));
                return Err((count, message));
            }
            self.follow_relocated_path(source, target);
        }
        self.show_status(if let [(_, target)] = moves {
            format!("Moved {}", name_text(target))
        } else {
            format!("Moved {} files", moves.len())
        });
        Ok(())
    }

    /// Move the file as stored and update its draft and password-change paths.
    fn move_file(
        &mut self,
        source: &Path,
        target: &Path,
        save_settings: impl FnOnce(&Settings) -> io::Result<()>,
    ) -> io::Result<()> {
        let encrypted = crypto::read_encryption_marker(source)?;
        scratch_paths::follow_relocation(&mut self.settings, source, target, encrypted, save_settings, || {
            self.drafts.relocate_file(source, target, self.privacy.vault.as_deref(), || {
                renameat_with(CWD, source, CWD, target, RenameFlags::NOREPLACE).map_err(io::Error::from)
            })
        })
    }
}

/// Check the whole selection for invalid sources and name conflicts before moving any file.
fn plan_moves(paths: &[PathBuf], folder: &Path) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    if remote::is_mirror(folder) || !folder.is_dir() {
        return Err("Choose a local destination folder".into());
    }
    let mut moves = Vec::new();
    let mut targets = HashSet::new();
    for source in paths {
        let target =
            move_target(source, folder).map_err(|error| format!("Cannot move {}: {error}", name_text(source)))?;
        let Some(target) = target else {
            continue;
        };
        if !targets.insert(target.clone()) {
            return Err(format!("More than one selected file is named {}", name_text(source)));
        }
        moves.push((source.clone(), target));
    }
    Ok(moves)
}

/// Return the new path, or no move if the file already lives in the chosen folder.
fn move_target(source: &Path, folder: &Path) -> io::Result<Option<PathBuf>> {
    let (Some(name), Some(parent)) = (source.file_name(), source.parent()) else {
        return Err(io::Error::other("the file has no name or parent folder"));
    };
    let parent = fs::canonicalize(parent)?;
    if terminal::is_terminal_path(source)
        || remote::is_mirror(source)
        || remote::is_mirror(&parent)
        || !fs::symlink_metadata(source)?.is_file()
    {
        return Err(io::Error::other("select regular local files to move"));
    }
    if parent == folder {
        return Ok(None);
    }
    let target = folder.join(name);
    match fs::symlink_metadata(&target) {
        Ok(_) => Err(io::Error::new(io::ErrorKind::AlreadyExists, "that name already exists in the destination")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Some(target)),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::Context;

    use super::*;
    use crate::app::{OpenFile, drafts::DraftState};
    use crate::crypto::Vault;
    use crate::test_support::TempFolder;

    /// Build an app with a test scratch root and its own drafts store.
    fn app_in(temp: &TempFolder, scratches: PathBuf) -> RubriqueApp {
        let settings = Settings { scratch_folder: Some(scratches.clone()), ..Settings::default() };
        let mut app = RubriqueApp::new(&Context::default(), settings, vec![scratches], None);
        app.tabs.clear();
        app.open_file = OpenFile::Nothing;
        app.active_tab = 0;
        app.drafts = DraftState::in_folder(temp.path().join("drafts"));
        app
    }

    #[test]
    fn moves_selected_files_and_keeps_open_and_closed_unsaved_edits() {
        let temp = TempFolder::new();
        let first = temp.write("scratches/first.md", "first on disk");
        let second = temp.write("scratches/second.md", "second on disk");
        let target = temp.make_folder("dev/project");
        let mut app = app_in(&temp, temp.path().join("scratches"));
        app.open_in_tab(second.clone());
        edit_active(&mut app, "second unsaved");
        assert!(app.close_keeping_edits());
        app.open_in_tab(first.clone());
        edit_active(&mut app, "first unsaved");
        assert!(app.keep_drafts());

        app.move_files_to(&[first.clone(), second.clone()], &target, &|_| Ok(())).unwrap();

        assert!(!first.exists());
        assert!(!second.exists());
        let new_first = target.join("first.md");
        let new_second = target.join("second.md");
        assert_eq!(fs::read_to_string(&new_first).unwrap(), "first on disk");
        assert_eq!(fs::read_to_string(&new_second).unwrap(), "second on disk");
        assert_active_edit(&app, &new_first, "first unsaved");
        app.open_in_tab(new_second.clone());
        assert_active_edit(&app, &new_second, "second unsaved");
        assert!(app.roots.iter().any(|root| root.path == target));
    }

    #[test]
    fn leaves_the_batch_in_place_if_a_destination_name_is_taken() {
        let temp = TempFolder::new();
        let first = temp.write("scratches/first.md", "first");
        let second = temp.write("scratches/second.md", "second");
        let existing = temp.write("dev/project/second.md", "existing");
        let target = existing.parent().unwrap();
        let mut app = app_in(&temp, temp.path().join("scratches"));

        let error = app.move_files_to(&[first.clone(), second.clone()], target, &|_| Ok(())).unwrap_err();

        assert!(error.contains("second.md"));
        assert!(error.contains("already exists"));
        assert!(first.exists());
        assert!(second.exists());
        assert!(!target.join("first.md").exists());
        assert_eq!(fs::read_to_string(existing).unwrap(), "existing");
    }

    #[test]
    fn keeps_the_original_and_its_draft_if_the_destination_appears_during_a_move() {
        let temp = TempFolder::new();
        let source = temp.write("scratches/note.md", "on disk");
        let folder = temp.make_folder("dev/project");
        let mut app = app_in(&temp, temp.path().join("scratches"));
        app.open_in_tab(source.clone());
        edit_active(&mut app, "unsaved");
        assert!(app.close_keeping_edits());
        let moves = plan_moves(std::slice::from_ref(&source), &folder).unwrap();
        let target = temp.write("dev/project/note.md", "another file");

        assert_eq!(app.apply_moves(&moves, &|_| Ok(())).unwrap_err().0, 0);

        assert_eq!(fs::read_to_string(&source).unwrap(), "on disk");
        assert_eq!(fs::read_to_string(&target).unwrap(), "another file");
        app.open_in_tab(target);
        assert!(!app.open_file.is_dirty());
        app.open_in_tab(source.clone());
        assert_active_edit(&app, &source, "unsaved");
    }

    #[test]
    fn skips_files_already_in_the_destination() {
        let temp = TempFolder::new();
        let destination = temp.make_folder("dev/project");
        let already_there = temp.write("dev/project/first.md", "first");
        assert_eq!(plan_moves(&[already_there], &destination).unwrap(), Vec::new());
    }

    #[test]
    fn moves_a_locked_scratch_as_ciphertext_and_remembers_it_for_password_changes() {
        let temp = TempFolder::new();
        let (old, info) = Vault::create("old password");
        let sealed = old.encrypt(b"secret");
        let source = temp.write("scratches/secret.md", &sealed);
        let plain = temp.write("scratches/plain.md", "plain");
        let folder = temp.make_folder("dev/project");
        let settings_path = temp.path().join("settings");
        let mut app = app_in(&temp, temp.path().join("scratches"));
        app.settings.privacy.vault = Some(info);
        app.open_in_tab(source.clone());
        assert!(matches!(&app.open_file, OpenFile::Locked { path } if path == &source));

        app.move_files_to(&[source.clone(), plain], &folder, &|settings| fs::write(&settings_path, settings.to_text()))
            .unwrap();

        let destination = folder.join("secret.md");
        assert!(!source.exists());
        assert_eq!(fs::read(&destination).unwrap(), sealed);
        assert_eq!(fs::read_to_string(folder.join("plain.md")).unwrap(), "plain");
        assert!(matches!(&app.open_file, OpenFile::Locked { path } if path == &destination));
        assert_eq!(app.settings.privacy.moved_scratches.as_slice(), std::slice::from_ref(&destination));
        let saved = Settings::parse(&fs::read_to_string(settings_path).unwrap());
        let (new, _) = Vault::create("new password");
        let changed = crate::scratch::reencrypt_scratches(
            &saved.scratch_folder.unwrap(),
            &saved.privacy.moved_scratches,
            &old,
            &new,
        )
        .unwrap();
        assert_eq!(changed.as_slice(), std::slice::from_ref(&destination));
        let moved_bytes = fs::read(&destination).unwrap();
        assert!(old.decrypt(&moved_bytes).is_err());
        assert_eq!(new.decrypt(&moved_bytes).unwrap(), b"secret");
    }

    #[test]
    fn keeps_a_moved_scratch_and_its_closed_draft_sealed() {
        let temp = TempFolder::new();
        let old = std::sync::Arc::new(Vault::create("old password").0);
        let sealed = old.encrypt(b"on disk");
        let source = temp.write("scratches/secret.md", &sealed);
        let folder = temp.make_folder("dev/project");
        let mut app = app_in(&temp, temp.path().join("scratches"));
        app.privacy.vault = Some(old.clone());
        app.open_in_tab(source.clone());
        edit_active(&mut app, "unsaved secret");
        assert!(app.close_keeping_edits());

        app.move_files_to(&[source], &folder, &|_| Ok(())).unwrap();

        let destination = folder.join("secret.md");
        assert_eq!(fs::read(&destination).unwrap(), sealed);
        let draft_folder = temp.path().join("drafts/closed");
        let draft_file = fs::read_dir(draft_folder).unwrap().next().unwrap().unwrap().path();
        assert!(crypto::file_is_encrypted(&draft_file));
        let new = std::sync::Arc::new(Vault::create("new password").0);
        crate::scratch::reencrypt_scratches(
            &app.settings.scratch_folder.clone().unwrap(),
            &app.settings.privacy.moved_scratches,
            &old,
            &new,
        )
        .unwrap();
        app.reseal_drafts(&old, &new);
        app.privacy.vault = Some(new);
        app.open_in_tab(destination.clone());
        assert_active_edit(&app, &destination, "unsaved secret");
    }

    #[test]
    fn leaves_the_encrypted_file_in_place_if_its_destination_cannot_be_saved() {
        let temp = TempFolder::new();
        let vault = Vault::create("password").0;
        let sealed = vault.encrypt(b"secret");
        let source = temp.write("scratches/secret.md", &sealed);
        let folder = temp.make_folder("dev/project");
        let mut app = app_in(&temp, temp.path().join("scratches"));

        let error = app
            .move_files_to(std::slice::from_ref(&source), &folder, &|_| Err(io::Error::other("settings unavailable")))
            .unwrap_err();
        assert!(error.contains("could not remember"));
        assert_eq!(fs::read(source).unwrap(), sealed);
        assert!(!folder.join("secret.md").exists());
        assert_eq!(app.settings.privacy.moved_scratches, Vec::<PathBuf>::new());
    }

    /// Type text into the active document and mark it dirty.
    fn edit_active(app: &mut RubriqueApp, text: &str) {
        let OpenFile::Text { document, .. } = &mut app.open_file else {
            panic!("expected an open document");
        };
        document.text = text.into();
        document.on_text_changed();
    }

    /// Check that the active document still holds its unsaved text.
    fn assert_active_edit(app: &RubriqueApp, path: &Path, text: &str) {
        let OpenFile::Text { document, .. } = &app.open_file else {
            panic!("expected an open document");
        };
        assert_eq!(document.path, path);
        assert_eq!(document.text, text);
        assert!(document.is_dirty());
    }
}
