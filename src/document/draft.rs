//! Drafts: unsaved edits kept aside in the config folder, so quitting (or a
//! crash) never loses them and never touches the file itself.
//!
//! One file per document, named after a hash of its absolute path. It holds
//! a short header, then the text the edits started from, then the edited text.
//! Drafts of encrypted scratches are sealed whole with the same key.
//! Edits kept when a tab closes unsaved wait in a second store of the same
//! kind (the `closed` folder inside the drafts folder) until the file opens.

use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::text_file::{Document, write_atomically};
use crate::crypto::{self, Vault};

/// First line of every draft file; a different one is not read.
const HEADER: &str = "rubrique draft 1";
/// File name extension of draft files.
const EXTENSION: &str = "draft";

/// The unsaved edits of one document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// Absolute path of the document.
    pub path: PathBuf,
    /// The file's text the edits started from.
    pub base_text: String,
    /// The edited text.
    pub text: String,
    /// Kept encrypted, because the document is.
    pub sealed: bool,
}

impl Draft {
    /// The draft of `document` as it is now.
    pub fn of(document: &Document) -> Self {
        Draft {
            path: key(&document.path),
            base_text: document.base_text().to_string(),
            text: document.text.clone(),
            sealed: document.is_encrypted(),
        }
    }
}

/// What to do with a draft at start, given the file's text now (`None`
/// when the file is gone).
#[derive(Debug, PartialEq, Eq)]
pub enum Restore {
    /// The file already holds the edited text: drop the draft.
    Nothing,
    /// The file is as the edits left it: continue editing.
    Continue,
    /// Another program changed the file since: merge the two.
    Merge,
    /// The file was deleted: keep the edits in an unsaved buffer.
    Missing,
}

/// Decide how to restore `draft` against the file's current text.
pub fn decide(draft: &Draft, disk_text: Option<&str>) -> Restore {
    match disk_text {
        None => Restore::Missing,
        Some(disk) if disk == draft.text => Restore::Nothing,
        Some(disk) if disk == draft.base_text => Restore::Continue,
        Some(_) => Restore::Merge,
    }
}

/// The path drafts are stored under: absolute, symlinks left as they are.
pub fn key(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Name of the draft file for `path`: a hash that stays the same between
/// runs and versions (64-bit FNV-1a of the path's bytes).
pub fn file_name(path: &Path) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let hash = key(path)
        .as_os_str()
        .as_bytes()
        .iter()
        .fold(OFFSET, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(PRIME));
    format!("{hash:016x}.{EXTENSION}")
}

/// Draft file contents: header lines, a blank line, the base, the text.
/// Lengths are in bytes, so any text survives as is.
pub fn format(draft: &Draft) -> Vec<u8> {
    let mut bytes = format!("{HEADER}\npath=").into_bytes();
    bytes.extend_from_slice(draft.path.as_os_str().as_bytes());
    bytes.extend_from_slice(format!("\nbase={}\ntext={}\n\n", draft.base_text.len(), draft.text.len()).as_bytes());
    bytes.extend_from_slice(draft.base_text.as_bytes());
    bytes.extend_from_slice(draft.text.as_bytes());
    bytes
}

/// Read what [`format`] wrote; `None` for anything else.
pub fn parse(bytes: &[u8]) -> Option<Draft> {
    let header_end = bytes.windows(2).position(|pair| pair == b"\n\n")?;
    let (header, body) = (&bytes[..header_end], &bytes[header_end + 2..]);
    let mut lines = header.split(|byte| *byte == b'\n');
    if lines.next()? != HEADER.as_bytes() {
        return None;
    }
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(lines.next()?.strip_prefix(b"path=")?));
    let length = |line: Option<&[u8]>, name: &[u8]| -> Option<usize> {
        std::str::from_utf8(line?.strip_prefix(name)?).ok()?.parse().ok()
    };
    let base_length = length(lines.next(), b"base=")?;
    let text_length = length(lines.next(), b"text=")?;
    if body.len() != base_length.checked_add(text_length)? {
        return None;
    }
    let (base, text) = body.split_at(base_length);
    Some(Draft {
        path,
        base_text: String::from_utf8(base.to_vec()).ok()?,
        text: String::from_utf8(text.to_vec()).ok()?,
        sealed: false,
    })
}

/// The drafts folder (`drafts` in the config folder).
#[derive(Debug)]
pub struct DraftStore {
    folder: PathBuf,
}

impl DraftStore {
    /// Drafts kept in `folder` (created on the first write).
    pub fn new(folder: PathBuf) -> Self {
        DraftStore { folder }
    }

    /// Where the draft for `path` is kept.
    fn file_for(&self, path: &Path) -> PathBuf {
        self.folder.join(file_name(path))
    }

    /// Keep `draft`, replacing an older one atomically. A sealed draft is
    /// encrypted with `vault`.
    ///
    /// # Errors
    ///
    /// Fails when the folder or the file cannot be written, the path has
    /// a line break (the header could not hold it), or a sealed draft has
    /// no key (it is never written in the clear).
    pub fn write(&self, draft: &Draft, vault: Option<&Vault>) -> io::Result<()> {
        if draft.path.as_os_str().as_bytes().contains(&b'\n') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "file name has a line break"));
        }
        let bytes = match (draft.sealed, vault) {
            (false, _) => format(draft),
            (true, Some(vault)) => vault.encrypt(&format(draft)),
            (true, None) => return Err(io::Error::new(io::ErrorKind::InvalidInput, "no key for an encrypted draft")),
        };
        fs::create_dir_all(&self.folder)?;
        write_atomically(&self.file_for(&draft.path), &bytes)
    }

    /// Delete the draft for `path`; a missing one is fine.
    ///
    /// # Errors
    ///
    /// Fails when the draft exists but cannot be deleted.
    pub fn remove(&self, path: &Path) -> io::Result<()> {
        match fs::remove_file(self.file_for(path)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Every readable draft, sorted by path. Unreadable files are left
    /// alone, and so are encrypted ones without `vault`.
    pub fn all(&self, vault: Option<&Vault>) -> Vec<Draft> {
        let Ok(entries) = fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut drafts: Vec<Draft> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|file| file.extension().is_some_and(|extension| extension == EXTENSION))
            .filter_map(|file| read_draft_file(&file, vault))
            .collect();
        drafts.sort_by(|a, b| a.path.cmp(&b.path));
        drafts
    }

    /// The draft kept for `path`, if there is a readable one (an encrypted
    /// one needs `vault`).
    pub fn read(&self, path: &Path, vault: Option<&Vault>) -> Option<Draft> {
        read_draft_file(&self.file_for(path), vault)
    }

    /// Move the draft of `path` into `other`, replacing one kept there.
    ///
    /// # Errors
    ///
    /// Fails when there is no draft for `path` or it cannot be moved.
    pub fn move_to(&self, path: &Path, other: &DraftStore) -> io::Result<()> {
        fs::create_dir_all(&other.folder)?;
        fs::rename(self.file_for(path), other.file_for(path))
    }
}

/// Read one draft file, decrypting it with `vault` when it is sealed.
/// `None` when it cannot be read or is not a draft, and for a file renamed
/// by hand or a hash collision: its name does not match the path inside.
fn read_draft_file(file: &Path, vault: Option<&Vault>) -> Option<Draft> {
    let bytes = fs::read(file).ok()?;
    let draft = if crypto::is_encrypted(&bytes) {
        Draft { sealed: true, ..parse(&vault?.decrypt(&bytes).ok()?)? }
    } else {
        parse(&bytes)?
    };
    (file.file_name()? == file_name(&draft.path).as_str()).then_some(draft)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    fn draft(path: &str, base_text: &str, text: &str) -> Draft {
        Draft { path: PathBuf::from(path), base_text: base_text.to_string(), text: text.to_string(), sealed: false }
    }

    #[test]
    fn file_name_is_a_stable_hash_of_the_path() {
        let name = file_name(Path::new("/notes/a.md"));
        assert_eq!(name, file_name(Path::new("/notes/a.md")));
        assert_ne!(name, file_name(Path::new("/notes/b.md")));
        assert_eq!(name.len(), 16 + ".draft".len());
        // Fixed algorithm, so drafts are found again after an update.
        assert_eq!(name, "ebfa5db8ed2d5412.draft");
    }

    #[test]
    fn format_and_parse_round_trip() {
        // Blank lines, CRLF, no final newline, an empty base: all must survive.
        let original = draft("/notes/a b.md", "", "# Title\r\n\n\nend");
        assert_eq!(parse(&format(&original)), Some(original));
        let original = draft("/notes/ü.md", "one\n\n", "\n\ntwo");
        assert_eq!(parse(&format(&original)), Some(original));
    }

    #[test]
    fn parse_rejects_other_files() {
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"something else\npath=/a\nbase=0\ntext=0\n\n"), None);
        // Lengths that do not match the body (a truncated file).
        assert_eq!(parse(b"rubrique draft 1\npath=/a\nbase=2\ntext=2\n\nabc"), None);
        assert_eq!(parse(b"rubrique draft 1\npath=/a\nbase=x\ntext=0\n\n"), None);
    }

    #[test]
    fn decide_compares_the_draft_with_the_file() {
        let edits = draft("/a.md", "old\n", "mine\n");
        assert_eq!(decide(&edits, Some("old\n")), Restore::Continue);
        assert_eq!(decide(&edits, Some("mine\n")), Restore::Nothing);
        assert_eq!(decide(&edits, Some("theirs\n")), Restore::Merge);
        assert_eq!(decide(&edits, None), Restore::Missing);
    }

    #[test]
    fn store_writes_lists_and_removes() {
        let temp = TempFolder::new();
        let store = DraftStore::new(temp.path().join("drafts"));
        assert!(store.all(None).is_empty(), "no folder yet");

        let first = draft("/notes/b.md", "1", "2");
        let second = draft("/notes/a.md", "x", "y");
        store.write(&first, None).unwrap();
        store.write(&second, None).unwrap();
        store.write(&draft("/notes/b.md", "1", "3"), None).unwrap();
        assert_eq!(store.all(None), vec![second.clone(), draft("/notes/b.md", "1", "3")]);

        store.remove(Path::new("/notes/b.md")).unwrap();
        store.remove(Path::new("/notes/b.md")).unwrap();
        assert_eq!(store.all(None), vec![second]);
        // Only the draft itself: no temp files left behind.
        assert_eq!(fs::read_dir(temp.path().join("drafts")).unwrap().count(), 1);
    }

    #[test]
    fn read_finds_one_draft_and_move_to_hands_it_over() {
        let temp = TempFolder::new();
        let store = DraftStore::new(temp.path().join("drafts"));
        let closed = DraftStore::new(temp.path().join("drafts/closed"));
        let edits = draft("/notes/a.md", "old", "mine");
        assert_eq!(closed.read(&edits.path, None), None, "nothing kept yet");
        assert!(closed.move_to(&edits.path, &store).is_err(), "nothing to move");

        closed.write(&edits, None).unwrap();
        assert_eq!(closed.read(&edits.path, None), Some(edits.clone()));
        assert_eq!(closed.read(Path::new("/notes/b.md"), None), None);
        // The closed folder sits inside the drafts folder, yet its drafts
        // are not among the ones reopened at start.
        assert_eq!(store.all(None), Vec::<Draft>::new());

        closed.move_to(&edits.path, &store).unwrap();
        assert_eq!(closed.read(&edits.path, None), None);
        assert_eq!(store.all(None), vec![edits]);
    }

    #[test]
    fn store_skips_unreadable_and_misnamed_files() {
        let temp = TempFolder::new();
        let folder = temp.make_folder("drafts");
        temp.write("drafts/junk.draft", "not a draft");
        let moved = format(&draft("/notes/a.md", "", "x"));
        temp.write("drafts/0000000000000000.draft", moved);
        assert_eq!(DraftStore::new(folder).all(None), Vec::<Draft>::new());
    }

    #[test]
    fn store_refuses_paths_with_line_breaks() {
        let temp = TempFolder::new();
        let store = DraftStore::new(temp.path().to_path_buf());
        assert!(store.write(&draft("/notes/a\nb.md", "", "x"), None).is_err());
    }

    #[test]
    fn sealed_drafts_are_encrypted() {
        let temp = TempFolder::new();
        let store = DraftStore::new(temp.path().to_path_buf());
        let vault = Vault::create("pw").0;
        let secret = Draft { sealed: true, ..draft("/s/secret.md", "old", "hidden words") };
        assert!(store.write(&secret, None).is_err(), "never in the clear");
        store.write(&secret, Some(&vault)).unwrap();
        let bytes = fs::read(store.file_for(Path::new("/s/secret.md"))).unwrap();
        assert!(!bytes.windows(6).any(|window| window == b"hidden"));
        assert!(store.all(None).is_empty(), "left alone while locked");
        assert_eq!(store.all(Some(&vault)), vec![secret]);
    }
}
