//! "New ▸ File / Folder / Terminal" in a folder's right-click menu: a name
//! field opens at the top of the folder in the tree. A folder or terminal
//! gets a free name filled in, with the part before the extension
//! selected; a file gets an empty name field and an empty extension field.
//! Enter creates the entry (a new file also opens in a tab), Esc cancels.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use super::RubriqueApp;
use super::terminals::create_terminal_file;
use crate::explorer::tree::FsNode;
use crate::explorer::view::{InlineRename, NewEntry, NewKind};
use crate::scratch::check_name;
use crate::terminal::TERMINAL_EXTENSION;

/// Most numbered names tried when suggesting a free one.
const MAX_NAME_NUMBER: u32 = 999;

impl RubriqueApp {
    /// Open `folder` in the tree and show a name field for a new `kind` at
    /// its top.
    pub(super) fn start_new_entry(&mut self, folder: PathBuf, kind: NewKind) {
        self.clear_name_fields();
        self.settings.layout.files_collapsed = false;
        if let Some(root) = self.roots.iter_mut().find(|root| folder.starts_with(&root.path)) {
            root.open_folder(&folder);
        }
        let name = match kind {
            // A file starts with both of its fields empty: no name and no
            // extension is assumed.
            NewKind::File => String::new(),
            NewKind::Folder => free_name(&folder, "new-folder", None),
            NewKind::Terminal => free_name(&folder, "terminal", Some(TERMINAL_EXTENSION)),
        };
        self.new_entry = Some(InlineRename::with_name(NewEntry::new(folder, kind), name));
    }

    /// Create the entry whose name field was just submitted; on failure the
    /// field stays and shows why.
    pub(super) fn finish_new_entry(&mut self) {
        let Some(mut entry) = self.new_entry.take_if(|entry| entry.submitted) else {
            return;
        };
        if let Err(error) = self.create_entry(&entry.target, &entry.full_name()) {
            entry.failed(error);
            self.new_entry = Some(entry);
        }
    }

    /// Create `typed` in the entry's folder, then show it: a file or a
    /// terminal opens in a tab, and the tree highlights the new entry.
    fn create_entry(&mut self, entry: &NewEntry, typed: &str) -> Result<(), String> {
        let path = match entry.kind {
            NewKind::File => create_file(&entry.folder, typed)?,
            NewKind::Folder => create_folder(&entry.folder, typed)?,
            NewKind::Terminal => create_terminal_file(&entry.folder, typed)?,
        };
        self.roots.iter_mut().for_each(FsNode::refresh);
        if entry.kind != NewKind::Folder {
            self.open_in_tab(path.clone());
        }
        self.reveal_in_files(path);
        Ok(())
    }
}

/// Create an empty file called `typed` in `folder`; never replaces one.
fn create_file(folder: &Path, typed: &str) -> Result<PathBuf, String> {
    let name = check_name(typed)?;
    let path = folder.join(name);
    OpenOptions::new().write(true).create_new(true).open(&path).map_err(|error| creation_error(name, &error))?;
    Ok(path)
}

/// Create a folder called `typed` in `folder`.
fn create_folder(folder: &Path, typed: &str) -> Result<PathBuf, String> {
    let name = check_name(typed)?;
    let path = folder.join(name);
    fs::create_dir(&path).map_err(|error| creation_error(name, &error))?;
    Ok(path)
}

/// What to show when creating `name` failed.
pub(super) fn creation_error(name: &str, error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::AlreadyExists => format!("{name} already exists here"),
        _ => format!("Cannot create it: {error}"),
    }
}

/// A name that is free in `folder`: `stem.extension`, then
/// `stem-2.extension`, and so on.
pub(super) fn free_name(folder: &Path, stem: &str, extension: Option<&str>) -> String {
    let full = |number: u32| {
        let stem = if number == 1 { stem.to_string() } else { format!("{stem}-{number}") };
        match extension {
            Some(extension) => format!("{stem}.{extension}"),
            None => stem,
        }
    };
    (1..=MAX_NAME_NUMBER).map(full).find(|name| !folder.join(name).exists()).unwrap_or_else(|| full(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    #[test]
    fn suggests_free_names() {
        let temp = TempFolder::new();
        assert_eq!(free_name(temp.path(), "untitled", Some("md")), "untitled.md");
        temp.write("untitled.md", "");
        assert_eq!(free_name(temp.path(), "untitled", Some("md")), "untitled-2.md");
        temp.make_folder("new-folder");
        assert_eq!(free_name(temp.path(), "new-folder", None), "new-folder-2");
    }

    #[test]
    fn creates_files_and_folders_without_replacing() {
        let temp = TempFolder::new();
        let file = create_file(temp.path(), " plan.md ").unwrap();
        assert_eq!(file, temp.path().join("plan.md"));
        assert_eq!(create_file(temp.path(), "plan.md"), Err("plan.md already exists here".to_string()));
        assert!(create_folder(temp.path(), "docs").unwrap().is_dir());
        assert!(create_folder(temp.path(), "docs").is_err());
        assert!(create_file(temp.path(), "a/b.md").is_err());
        assert!(create_file(temp.path(), "  ").is_err());
    }
}
