//! Unsaved edits kept as drafts: written a few seconds after the last edit
//! and on quit, put back into their tabs on the next start, and deleted once
//! the document is clean or its tab is closed.
//!
//! A tab closed with "Keep for later" moves its draft to the closed drafts
//! instead; opening that file again puts the edits back.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::Context;

use super::{OpenFile, RubriqueApp};
use crate::crypto::Vault;
use crate::document::draft::{self, Draft, DraftStore, Restore};
use crate::document::text_file::{DiskState, Document};
use crate::explorer::paths::name_text;
use crate::settings::drafts_folder;

/// How long after the last edit the draft is written (crash safety).
const DRAFT_DELAY_SECONDS: f64 = 3.0;

/// Folder inside the drafts folder for the edits of tabs closed unsaved.
const CLOSED_FOLDER: &str = "closed";

/// Bookkeeping for the drafts folder.
#[derive(Debug)]
pub(super) struct DraftState {
    /// `None` when there is no config folder.
    store: Option<DraftStore>,
    /// Edits of tabs closed with "Keep for later", waiting for their file
    /// to open again. Not reopened at start. `None` without a config folder.
    closed: Option<DraftStore>,
    /// Drafts on disk, by [`draft::key`], with the revision they hold.
    written: HashMap<PathBuf, u64>,
    /// The active document's revision in the last frame.
    last_revision: u64,
    /// egui time of the last edit seen.
    changed_at: f64,
}

impl Default for DraftState {
    fn default() -> Self {
        match drafts_folder() {
            Some(folder) => DraftState::in_folder(folder),
            None => {
                DraftState { store: None, closed: None, written: HashMap::new(), last_revision: 0, changed_at: 0.0 }
            }
        }
    }
}

impl DraftState {
    /// Drafts kept in `folder`, and the closed ones in a folder inside it.
    fn in_folder(folder: PathBuf) -> Self {
        DraftState {
            closed: Some(DraftStore::new(folder.join(CLOSED_FOLDER))),
            store: Some(DraftStore::new(folder)),
            written: HashMap::new(),
            last_revision: 0,
            changed_at: 0.0,
        }
    }

    /// Keep the unsaved edits of `document` with the closed drafts, so its
    /// tab can close; its regular draft goes. Encrypted documents' edits
    /// are sealed with their key, as regular drafts are.
    ///
    /// # Errors
    ///
    /// Fails when there is no config folder or the draft cannot be written.
    fn keep_for_later(&mut self, document: &Document) -> io::Result<()> {
        let DraftState { store: Some(store), closed: Some(closed), written, .. } = self else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "there is no settings folder to keep them in"));
        };
        closed.write(&Draft::of(document), document.vault())?;
        forget(store, written, &draft::key(&document.path));
        Ok(())
    }

    /// Stop tracking the draft of `path` without deleting it: its tab now
    /// waits for the scratch password, and the draft is restored after.
    pub(super) fn release(&mut self, path: &Path) {
        self.written.remove(&draft::key(path));
    }
}

/// How one draft came back at start.
enum Restored {
    Continued,
    /// The file changed on disk since; the tab asks to merge.
    Merge,
    /// The file is gone; the tab holds the edits.
    Missing(PathBuf),
}

impl RubriqueApp {
    /// Reopen last session's tabs, with their drafts, then the tabs that
    /// only have a draft (opened after the last clean quit, before a crash).
    pub(super) fn reopen_tabs(&mut self, saved_tabs: Vec<PathBuf>) {
        let mut drafts: HashMap<PathBuf, Draft> = match &self.drafts.store {
            Some(store) => store.all(None).into_iter().map(|draft| (draft.path.clone(), draft)).collect(),
            None => HashMap::new(),
        };
        let mut restored = Vec::new();
        for path in saved_tabs {
            if !self.roots.iter().any(|root| path.starts_with(&root.path)) {
                continue;
            }
            match drafts.remove(&draft::key(&path)) {
                Some(found) => restored.extend(self.open_with_draft(found)),
                None if path.is_file() => self.open_in_tab(path),
                None => {}
            }
        }
        let remote_on = self.settings.remote.enabled;
        let mut orphans: Vec<Draft> =
            drafts.into_values().filter(|draft| remote_on || !crate::remote::is_mirror(&draft.path)).collect();
        orphans.sort_by(|a, b| a.path.cmp(&b.path));
        for found in orphans {
            restored.extend(self.open_with_draft(found));
        }
        self.report_restored(&restored);
    }

    /// Open the draft's file in a tab with the edits put back. `None` when
    /// there was nothing to restore (the draft is then deleted).
    fn open_with_draft(&mut self, found: Draft) -> Option<Restored> {
        let path = found.path.clone();
        if !path.exists() {
            let vault = if found.sealed { self.privacy.vault.clone() } else { None };
            if found.sealed && vault.is_none() {
                return None;
            }
            let document = Document::missing(&path, found.text, found.base_text, vault);
            self.remember_draft(&document);
            self.add_tab(OpenFile::Text {
                document,
                preview: None,
                disk_notice: Some(DiskState::Deleted),
                editor: None,
                source: false,
            });
            return Some(Restored::Missing(path));
        }
        self.open_in_tab(path.clone());
        let OpenFile::Text { document, disk_notice, .. } = &mut self.open_file else {
            // Not text any more: leave the draft alone rather than lose it.
            return None;
        };
        if document.path != path {
            return None;
        }
        let outcome = draft::decide(&found, Some(&document.text));
        let restored = match outcome {
            Restore::Nothing => None,
            Restore::Continue => Some(Restored::Continued),
            Restore::Merge => {
                *disk_notice = Some(DiskState::Modified);
                Some(Restored::Merge)
            }
            Restore::Missing => unreachable!("the file was just read"),
        };
        if restored.is_some() {
            document.restore_edits(found.text, found.base_text);
            let key = draft::key(&document.path);
            let revision = document.revision();
            self.drafts.written.insert(key, revision);
        } else if let Some(store) = &self.drafts.store {
            // Best effort: a draft equal to the file is harmless if it stays.
            let _ = store.remove(&path);
        }
        restored
    }

    /// Close the active tab without saving, keeping its unsaved edits for
    /// when the file opens again. False (with a status message, and the
    /// tab left open) when there is nothing to keep or it cannot be kept.
    pub(super) fn close_keeping_edits(&mut self) -> bool {
        // A block still being edited belongs to the edits being kept.
        self.finish_block_edit();
        let OpenFile::Text { document, .. } = &self.open_file else {
            return false;
        };
        let name = name_text(&document.path);
        if let Err(error) = self.drafts.keep_for_later(document) {
            self.show_status(format!("Could not keep unsaved edits: {error}"));
            return false;
        }
        self.close_active_tab_now();
        self.show_status(format!("Kept the unsaved edits of {name}; they come back when you open it again"));
        true
    }

    /// Open `path` in a new tab with the edits kept when its tab was closed
    /// unsaved, through the same steps as a draft at start: a file changed
    /// on disk since opens the merge screen. False when no edits are kept
    /// for it (or they cannot be read now, such as an encrypted file's
    /// before the password is entered), so the caller opens it as usual.
    pub(super) fn reopen_kept_edits(&mut self, path: &Path) -> bool {
        let (Some(store), Some(closed)) = (&self.drafts.store, &self.drafts.closed) else {
            return false;
        };
        let Some(found) = closed.read(path, self.privacy.vault.as_deref()) else {
            return false;
        };
        // Back with the regular drafts first, so from here on it is kept,
        // updated and deleted like any other.
        if let Err(error) = closed.move_to(path, store) {
            self.show_status(format!("Could not put back the unsaved edits of {}: {error}", name_text(path)));
            return false;
        }
        let restored: Vec<Restored> = self.open_with_draft(found).into_iter().collect();
        self.report_restored(&restored);
        true
    }

    /// After the scratch password is entered: put back the edits of
    /// encrypted scratches kept while they were locked.
    pub(super) fn restore_sealed_drafts(&mut self) {
        let (Some(store), Some(vault)) = (&self.drafts.store, &self.privacy.vault) else {
            return;
        };
        let sealed: Vec<Draft> = store.all(Some(vault)).into_iter().filter(|found| found.sealed).collect();
        let mut restored = Vec::new();
        for found in sealed {
            if !self.drafts.written.contains_key(&found.path) {
                restored.extend(self.open_with_draft(found));
            }
        }
        self.report_restored(&restored);
    }

    /// Seal the drafts kept with `old` again with `new` (a new password).
    pub(super) fn reseal_drafts(&mut self, old: &Vault, new: &Arc<Vault>) {
        let Some(store) = &self.drafts.store else {
            return;
        };
        for found in store.all(Some(old)).into_iter().filter(|found| found.sealed) {
            if let Err(error) = store.write(&found, Some(new)) {
                eprintln!("rubrique: could not seal the draft of {} again: {error}", found.path.display());
            }
        }
    }

    /// Write the draft of `path` again, if one is kept, so it is sealed or
    /// plain like its document is now. When that fails the draft is
    /// deleted instead (never left plain beside an encrypted file); the
    /// next pause in typing writes it again.
    pub(super) fn rewrite_draft(&mut self, path: &Path) {
        let key = draft::key(path);
        let (Some(store), true) = (&self.drafts.store, self.drafts.written.contains_key(&key)) else {
            return;
        };
        let document = std::iter::once(&self.open_file).chain(self.tabs.iter()).find_map(|file| match file {
            OpenFile::Text { document, .. } if draft::key(&document.path) == key => Some(document),
            _ => None,
        });
        let written = document.is_some_and(|document| store.write(&Draft::of(document), document.vault()).is_ok());
        if !written {
            forget(store, &mut self.drafts.written, &key);
        }
    }

    /// Note that the draft on disk holds `document` as it is now.
    fn remember_draft(&mut self, document: &Document) {
        self.drafts.written.insert(draft::key(&document.path), document.revision());
    }

    /// Say what came back, and open the merge screen when the file on
    /// screen changed on disk since its edits were kept.
    fn report_restored(&mut self, restored: &[Restored]) {
        if restored.is_empty() {
            return;
        }
        let missing: Vec<String> = restored
            .iter()
            .filter_map(|item| match item {
                Restored::Missing(path) => Some(name_text(path)),
                _ => None,
            })
            .collect();
        let merge = restored.iter().any(|item| matches!(item, Restored::Merge));
        let message = restored_message(restored.len(), &missing, merge);
        self.show_status(message);
        if matches!(&self.open_file, OpenFile::Text { disk_notice: Some(DiskState::Modified), .. }) {
            self.start_merge();
        }
    }

    /// Keep drafts in step with the tabs every frame: deleted at once when
    /// clean or closed, written once edits pause for a few seconds.
    pub(super) fn poll_drafts(&mut self, ctx: &Context) {
        let now = ctx.input(|input| input.time);
        let revision = match &self.open_file {
            OpenFile::Text { document, .. } => document.revision(),
            _ => 0,
        };
        if revision != self.drafts.last_revision {
            self.drafts.last_revision = revision;
            self.drafts.changed_at = now;
        }
        let waited = now - self.drafts.changed_at;
        let write = waited >= DRAFT_DELAY_SECONDS;
        if !write && self.open_file.is_dirty() {
            ctx.request_repaint_after(Duration::from_secs_f64(DRAFT_DELAY_SECONDS - waited));
        }
        if let Err(error) = self.sync_drafts(write) {
            // Try again after another pause rather than every frame.
            self.drafts.changed_at = now;
            self.show_status(format!("Could not keep a draft of unsaved edits: {error}"));
        }
    }

    /// Write every draft that is behind, now (on quit). Says why on failure.
    pub(super) fn keep_drafts(&mut self) -> bool {
        self.finish_block_edit();
        match self.sync_drafts(true) {
            Ok(()) => true,
            Err(error) => {
                self.show_status(format!("Could not keep unsaved edits: {error}"));
                false
            }
        }
    }

    /// Delete drafts of clean or closed documents and, with `write`, write
    /// those of dirty ones that are behind.
    ///
    /// # Errors
    ///
    /// The last draft that could not be written (deleting is best effort).
    fn sync_drafts(&mut self, write: bool) -> io::Result<()> {
        let DraftState { store: Some(store), written, .. } = &mut self.drafts else {
            return Ok(());
        };
        let mut result = Ok(());
        let mut open = HashSet::new();
        let documents = std::iter::once(&self.open_file).chain(self.tabs.iter()).filter_map(|file| match file {
            OpenFile::Text { document, .. } => Some(document),
            _ => None,
        });
        for document in documents {
            let key = draft::key(&document.path);
            if !document.is_dirty() {
                forget(store, written, &key);
            } else if write && written.get(&key) != Some(&document.revision()) {
                match store.write(&Draft::of(document), document.vault()) {
                    Ok(()) => {
                        written.insert(key.clone(), document.revision());
                    }
                    Err(error) => result = Err(error),
                }
            }
            open.insert(key);
        }
        let closed: Vec<PathBuf> = written.keys().filter(|key| !open.contains(*key)).cloned().collect();
        for key in closed {
            forget(store, written, &key);
        }
        result
    }
}

/// "Restored unsaved edits in 2 files", plus the files that are gone and
/// whether some changed on disk since.
fn restored_message(count: usize, missing: &[String], merge: bool) -> String {
    let files = if count == 1 { "1 file".to_string() } else { format!("{count} files") };
    let mut parts = vec![format!("Restored unsaved edits in {files}")];
    if !missing.is_empty() {
        parts.push(format!("deleted on disk: {}", missing.join(", ")));
    }
    if merge {
        parts.push("changed on disk since, merge to continue".to_string());
    }
    parts.join("; ")
}

/// Delete the draft for `key` if one was written.
fn forget(store: &DraftStore, written: &mut HashMap<PathBuf, u64>, key: &Path) {
    if written.remove(key).is_some()
        && let Err(error) = store.remove(key)
    {
        eprintln!("rubrique: could not delete the draft of {}: {error}", key.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use crate::test_support::TempFolder;

    /// An app with no tabs whose drafts live in `temp`, never in this
    /// machine's config folder.
    fn app_with_drafts_in(temp: &TempFolder) -> RubriqueApp {
        let mut app = RubriqueApp::new(&Context::default(), Settings::default(), vec![temp.path().to_path_buf()], None);
        // `new` reopens the drafts of the real config folder as tabs.
        app.tabs.clear();
        app.active_tab = 0;
        app.open_file = OpenFile::Nothing;
        app.drafts = DraftState::in_folder(temp.path().join("drafts"));
        app
    }

    /// Replace the active document's text, as typing would.
    fn type_text(app: &mut RubriqueApp, text: &str) {
        let OpenFile::Text { document, .. } = &mut app.open_file else {
            panic!("no text file open");
        };
        document.text = text.to_string();
        document.on_text_changed();
    }

    /// The active document, which must be a text file.
    fn active_document(app: &RubriqueApp) -> &Document {
        match &app.open_file {
            OpenFile::Text { document, .. } => document,
            _ => panic!("no text file open"),
        }
    }

    fn closed_store(app: &RubriqueApp) -> &DraftStore {
        app.drafts.closed.as_ref().unwrap()
    }

    #[test]
    fn keep_for_later_closes_the_tab_and_opening_the_file_puts_the_edits_back() {
        let temp = TempFolder::new();
        let file = temp.write("notes/a.md", "old\n");
        let mut app = app_with_drafts_in(&temp);
        app.open_in_tab(file.clone());
        type_text(&mut app, "mine\n");

        assert!(app.close_keeping_edits());
        assert!(app.tabs.is_empty());
        assert_eq!(fs_text(&file), "old\n", "the file itself is never written");
        assert!(closed_store(&app).read(&file, None).is_some());

        app.open_in_tab(file.clone());
        let document = active_document(&app);
        assert_eq!(document.text, "mine\n");
        assert!(document.is_dirty());
        assert!(app.merging.is_none());
        // A regular draft again: kept and deleted like any other from now on.
        assert!(closed_store(&app).read(&file, None).is_none());
        assert_eq!(app.drafts.store.as_ref().unwrap().all(None).len(), 1);
    }

    #[test]
    fn kept_edits_of_a_file_changed_since_open_the_merge_screen() {
        let temp = TempFolder::new();
        let file = temp.write("notes/a.md", "old\n");
        let mut app = app_with_drafts_in(&temp);
        app.open_in_tab(file.clone());
        type_text(&mut app, "mine\n");
        assert!(app.close_keeping_edits());
        std::fs::write(&file, "theirs\n").unwrap();

        app.open_in_tab(file);
        assert_eq!(active_document(&app).text, "mine\n", "the edits are not dropped");
        assert!(app.merging.is_some(), "nor do they silently replace the newer file");
    }

    #[test]
    fn kept_edits_of_an_encrypted_file_stay_sealed() {
        let temp = TempFolder::new();
        let vault = Arc::new(Vault::create("pw").0);
        let file = temp.write("notes/secret.md", vault.encrypt(b"old\n"));
        let mut app = app_with_drafts_in(&temp);
        app.privacy.vault = Some(Arc::clone(&vault));
        app.open_in_tab(file.clone());
        type_text(&mut app, "hidden words\n");
        assert!(app.close_keeping_edits());

        let closed_folder = temp.path().join("drafts").join(CLOSED_FOLDER);
        for entry in std::fs::read_dir(&closed_folder).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(!bytes.windows(6).any(|window| window == b"hidden"), "never in the clear");
        }
        // Locked: the file opens as usual and the edits wait.
        app.privacy.vault = None;
        app.open_in_tab(file.clone());
        assert!(matches!(app.open_file, OpenFile::Locked { .. }));
        app.close_active_tab_now();

        app.privacy.vault = Some(vault);
        app.open_in_tab(file);
        assert_eq!(active_document(&app).text, "hidden words\n");
    }

    fn fs_text(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn restored_message_counts_files_and_names_missing_ones() {
        assert_eq!(restored_message(2, &[], false), "Restored unsaved edits in 2 files");
        assert_eq!(restored_message(1, &[], false), "Restored unsaved edits in 1 file");
        assert_eq!(
            restored_message(2, &["a.md".to_string()], true),
            "Restored unsaved edits in 2 files; deleted on disk: a.md; changed on disk since, merge to continue"
        );
    }
}
