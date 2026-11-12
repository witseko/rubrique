//! "Import from Sublime Text" (File menu, Settings → Files): copies every
//! unsaved Sublime Text tab into a new scratch, so moving over loses none
//! of the notes Sublime kept without saving them.

use super::RubriqueApp;
use super::privacy::AfterPassword;
use crate::scratch::{import_scratch, scratch_texts};
use crate::sublime::read_unsaved_texts;

impl RubriqueApp {
    /// Import Sublime Text's unsaved tabs into the scratch folder, one
    /// scratch each, and show the folder. Encrypted when new scratches are
    /// (asking for the password first when it was not entered yet). A text
    /// already in a scratch is skipped, so running it again adds nothing.
    pub(super) fn import_from_sublime(&mut self) {
        let Some(folder) = self.settings.scratch_folder_or_default() else {
            self.show_status("No folder for scratch files: set one in Settings".to_string());
            return;
        };
        let (session, texts) = match read_unsaved_texts() {
            Ok(found) => found,
            Err(error) => {
                self.show_status(error);
                return;
            }
        };
        if texts.is_empty() {
            self.show_status(format!("Sublime Text has no unsaved tabs ({})", session.display()));
            return;
        }
        let vault = if self.encrypt_new_scratches() {
            let Some(vault) = self.scratch_vault_or_ask(AfterPassword::ImportFromSublime) else {
                // The password dialog opens and runs the import after.
                return;
            };
            Some(vault)
        } else {
            None
        };
        let date = crate::date::today_iso();
        let mut existing = scratch_texts(&folder, vault.as_deref());
        let mut imported = 0;
        for unsaved in &texts {
            if existing.contains(&unsaved.text) {
                continue;
            }
            if let Err(error) = import_scratch(&folder, &date, &unsaved.name, &unsaved.text, vault.as_deref()) {
                let message = format!("Imported {imported}, then cannot write into {}: {error}", folder.display());
                self.show_scratch_folder(folder);
                self.show_status(message);
                return;
            }
            // Two Sublime tabs with the same text become one scratch.
            existing.push(unsaved.text.clone());
            imported += 1;
        }
        let skipped = texts.len() - imported;
        if imported > 0 {
            self.show_scratch_folder(folder);
        }
        let imported_text = match imported {
            1 => "Imported 1 unsaved Sublime Text tab as a scratch".to_string(),
            count => format!("Imported {count} unsaved Sublime Text tabs as scratches"),
        };
        let skipped_text = if skipped > 0 { format!(" ({skipped} already there)") } else { String::new() };
        self.show_status(format!("{imported_text}{skipped_text}"));
    }
}
