//! Load and save one text file without ever changing bytes the user did not edit.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::crypto::{self, DecryptError, Vault};
use crate::vfs::{self, Contents, Stamp};

const BYTES_PER_MEGABYTE: u64 = 1024 * 1024;

/// Files above this size are not opened (spec: 20 MB).
pub const MAX_FILE_BYTES: u64 = 20 * BYTES_PER_MEGABYTE;

/// How many leading bytes are checked for NUL to spot binary files.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Source of [`Document::revision`] numbers, unique across all documents.
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

/// Why a file cannot be shown in the editor.
#[derive(Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// Over [`MAX_FILE_BYTES`]; holds the file size in bytes.
    TooLarge(u64),
    /// Has a NUL byte near the start.
    Binary,
    /// Text in another encoding (or not text at all).
    NotUtf8,
    /// An encrypted scratch sealed with another password.
    WrongPassword,
}

impl Unsupported {
    /// Human-readable reason for the editor panel.
    pub fn describe(&self) -> String {
        match self {
            Unsupported::TooLarge(bytes) => format!(
                "File is too large ({} MB, limit {} MB).",
                bytes / BYTES_PER_MEGABYTE,
                MAX_FILE_BYTES / BYTES_PER_MEGABYTE
            ),
            Unsupported::Binary => "Binary file.".to_string(),
            Unsupported::NotUtf8 => "Not a UTF-8 text file.".to_string(),
            Unsupported::WrongPassword => {
                "Encrypted with another password. It cannot be read with the current one.".to_string()
            }
        }
    }
}

/// Result of trying to open a file.
#[derive(Debug)]
pub enum Opened {
    /// A displayable text file.
    Text(Document),
    /// A file the editor refuses to show, and why.
    Unsupported(Unsupported),
    /// An encrypted scratch, and no password was entered yet.
    Locked,
}

/// What changed on disk since we last loaded or saved.
#[derive(Debug, PartialEq, Eq)]
pub enum DiskState {
    Unchanged,
    /// Another program changed the file (its modification time differs).
    Modified,
    /// The file is gone.
    Deleted,
}

/// An open text file.
#[derive(Debug)]
pub struct Document {
    /// Path the file was opened from (may be a symlink).
    pub path: PathBuf,
    /// The editable buffer. Saved byte-for-byte as is.
    pub text: String,
    /// Hash of the text last read from or written to disk. `None` after the
    /// user chose to keep their version over a newer one on disk.
    saved_hash: Option<u64>,
    /// The buffer differs from what is on disk.
    dirty: bool,
    /// The file's version when we last read or wrote it.
    disk_modified: Option<Stamp>,
    /// The text as last read from or written to disk: the common starting
    /// point when the file and the buffer both changed (for merging).
    base_text: String,
    /// Changes on every edit and is never shared with another document, so
    /// a draft written at one revision is known to be up to date.
    revision: u64,
    /// The key when the file is stored encrypted; `None` for plain files.
    vault: Option<Arc<Vault>>,
    /// Tells this open of the file from any other, even of the same path, so
    /// a save that finishes in the background lands on the buffer it was
    /// taken from.
    id: u64,
}

impl Document {
    /// Read a file, refusing ones that are too large, binary, or not UTF-8.
    /// An encrypted file is opened with `vault`, or comes back
    /// [`Opened::Locked`] without one.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read (missing, permissions, ...).
    pub fn open(path: &Path, vault: Option<&Arc<Vault>>) -> io::Result<Opened> {
        let (mut bytes, stamp) = match vfs::read_file(path, MAX_FILE_BYTES)? {
            (Contents::Bytes(bytes), stamp) => (bytes, stamp),
            (Contents::TooLarge(size), _) => return Ok(Opened::Unsupported(Unsupported::TooLarge(size))),
        };
        let mut sealed_with = None;
        if crypto::is_encrypted(&bytes) {
            let Some(vault) = vault else {
                return Ok(Opened::Locked);
            };
            bytes = match vault.decrypt(&bytes) {
                Ok(plain) => plain,
                Err(DecryptError::WrongKey | DecryptError::Corrupt) => {
                    return Ok(Opened::Unsupported(Unsupported::WrongPassword));
                }
            };
            sealed_with = Some(Arc::clone(vault));
        }
        let text = match text_from_bytes(bytes) {
            Ok(text) => text,
            Err(reason) => return Ok(Opened::Unsupported(reason)),
        };
        Ok(Opened::Text(Document {
            path: path.to_path_buf(),
            saved_hash: Some(hash_text(&text)),
            base_text: text.clone(),
            text,
            dirty: false,
            disk_modified: stamp,
            revision: next_revision(),
            vault: sealed_with,
            id: next_revision(),
        }))
    }

    /// A buffer for a file that no longer exists: `text` with unsaved
    /// changes over `base_text`. Saving creates the file again.
    pub fn missing(path: &Path, text: String, base_text: String, vault: Option<Arc<Vault>>) -> Self {
        Document {
            path: path.to_path_buf(),
            text,
            saved_hash: None,
            dirty: true,
            disk_modified: None,
            base_text,
            revision: next_revision(),
            vault,
            id: next_revision(),
        }
    }

    /// The file is stored encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.vault.is_some()
    }

    /// The key the file is stored with, for its draft.
    pub fn vault(&self) -> Option<&Vault> {
        self.vault.as_deref()
    }

    /// Store the file with `vault` from now on (`None`: plain). The caller
    /// wrote the file that way already, so only the modification time is new.
    pub fn set_vault(&mut self, vault: Option<Arc<Vault>>) {
        self.vault = vault;
        self.disk_modified = vfs::current_stamp(&self.path).ok().flatten();
    }

    /// Put back edits kept from an earlier session: `text` in the buffer,
    /// `base_text` as the text they started from. The file stays the
    /// saved state, so the buffer is dirty when it differs from the file.
    pub fn restore_edits(&mut self, text: String, base_text: String) {
        self.text = text;
        self.base_text = base_text;
        self.on_text_changed();
    }

    /// Drop the unsaved changes: back to the file as it is on disk now (or
    /// the last known text when it cannot be read). Always clean afterwards.
    pub fn discard_changes(&mut self) {
        self.text = self.read_disk_text().unwrap_or_else(|_| self.base_text.clone());
        self.saved_hash = Some(hash_text(&self.text));
        self.base_text.clone_from(&self.text);
        self.disk_modified = vfs::current_stamp(&self.path).ok().flatten();
        self.on_text_changed();
    }

    /// Number that changes on every edit (see the field).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Which open of the file this is (see the field).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// True when the buffer has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Call after every edit. Dirty means "differs from what is on disk",
    /// so undoing back to the saved text makes the document clean again.
    pub fn on_text_changed(&mut self) {
        self.dirty = Some(hash_text(&self.text)) != self.saved_hash;
        self.revision = next_revision();
    }

    /// Write the buffer atomically: temp file in the same folder, flush, rename
    /// (on the host for a remote file). Writes through symlinks.
    ///
    /// # Errors
    ///
    /// Fails when the temp file cannot be written or renamed; the file on
    /// disk is then left as it was.
    pub fn save(&mut self) -> io::Result<()> {
        let stamp = vfs::save(&self.path, &self.stored_bytes())?;
        self.mark_saved(self.text.clone(), stamp);
        Ok(())
    }

    /// The buffer as the file stores it: encrypted with its key, if any.
    pub fn stored_bytes(&self) -> Vec<u8> {
        match &self.vault {
            Some(vault) => vault.encrypt(self.text.as_bytes()),
            None => self.text.as_bytes().to_vec(),
        }
    }

    /// Note that the file now holds `saved_text`, written with the new
    /// `stamp`. A save that ran in the background may have taken the text
    /// before later edits: the buffer then stays dirty, so the next save
    /// writes them.
    pub fn mark_saved(&mut self, saved_text: String, stamp: Option<Stamp>) {
        let saved_hash = hash_text(&saved_text);
        self.saved_hash = Some(saved_hash);
        self.disk_modified = stamp;
        self.base_text = saved_text;
        self.dirty = hash_text(&self.text) != saved_hash;
        if self.dirty {
            // The draft holds the old base text; a new revision writes it again.
            self.revision = next_revision();
        }
    }

    /// The text as last read from or written to disk.
    pub fn base_text(&self) -> &str {
        &self.base_text
    }

    /// Read the file as it is on disk now.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or is no longer UTF-8 text.
    pub fn read_disk_text(&self) -> io::Result<String> {
        let mut bytes = match vfs::read_file(&self.path, MAX_FILE_BYTES)?.0 {
            Contents::Bytes(bytes) => bytes,
            Contents::TooLarge(size) => {
                return Err(io::Error::new(io::ErrorKind::FileTooLarge, Unsupported::TooLarge(size).describe()));
            }
        };
        if let Some(vault) = &self.vault {
            bytes = vault.decrypt(&bytes).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "the file is no longer encrypted with this password")
            })?;
        }
        text_from_bytes(bytes).map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason.describe()))
    }

    /// Note that the disk now holds `disk_text` without touching the buffer:
    /// used when only the modification time changed, or after a merge. The
    /// buffer is dirty exactly when it differs from `disk_text`.
    pub fn accept_disk_text(&mut self, disk_text: String) {
        self.disk_modified = if self.vault.is_some() {
            // A remote stamp is a checksum of the stored (encrypted) bytes,
            // not of the text, so ask for it.
            vfs::current_stamp(&self.path).ok().flatten()
        } else {
            vfs::stamp_of(&self.path, disk_text.as_bytes())
        };
        self.saved_hash = Some(hash_text(&disk_text));
        self.base_text = disk_text;
        self.on_text_changed();
    }

    /// Replace the buffer with a merge result based on `disk_text`.
    pub fn apply_merge(&mut self, result: String, disk_text: String) {
        self.text = result;
        self.accept_disk_text(disk_text);
    }

    /// Compare the file's version on disk with the one we know. For a
    /// remote file this asks the host; the app does that in the background
    /// and uses [`Document::disk_state_from`] instead.
    pub fn disk_state(&self) -> DiskState {
        self.disk_state_from(vfs::current_stamp(&self.path))
    }

    /// Compare `current` (the file's stamp now: `None` when it is gone, an
    /// error when that could not be told) with the one we know.
    pub fn disk_state_from(&self, current: io::Result<Option<Stamp>>) -> DiskState {
        match current {
            // Gone, and we did not know yet ("Keep mine" already knows).
            Ok(None) if self.disk_modified.is_some() => DiskState::Deleted,
            Ok(Some(stamp)) if Some(&stamp) != self.disk_modified.as_ref() => DiskState::Modified,
            // The same version, or no answer from the host: no news.
            _ => DiskState::Unchanged,
        }
    }

    /// Accept the current disk state without reloading ("Keep mine"):
    /// the buffer stays dirty and we stop asking about this change.
    pub fn acknowledge_disk_change(&mut self) {
        // Their version is the new starting point for any later merge.
        if let Ok(disk_text) = self.read_disk_text() {
            self.disk_modified = vfs::stamp_of(&self.path, disk_text.as_bytes());
            self.base_text = disk_text;
        } else {
            self.disk_modified = vfs::current_stamp(&self.path).ok().flatten();
        }
        self.saved_hash = None;
        self.dirty = true;
    }
}

/// Validate file bytes as displayable text.
fn text_from_bytes(bytes: Vec<u8>) -> Result<String, Unsupported> {
    let sniff_length = bytes.len().min(BINARY_SNIFF_BYTES);
    if bytes[..sniff_length].contains(&0) {
        return Err(Unsupported::Binary);
    }
    String::from_utf8(bytes).map_err(|_| Unsupported::NotUtf8)
}

/// Fingerprint of the text, to tell whether an edit returned to the saved text.
fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// A revision number no document has used yet.
fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// Replace `target` with `contents` so that a crash leaves either the old or
/// the new file, never a truncated one. Keeps the original permissions.
pub fn write_atomically(target: &Path, contents: &[u8]) -> io::Result<()> {
    let folder = target.parent().unwrap_or(Path::new("."));
    let file_name = target.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    let temp_path = folder.join(format!(".{file_name}.rubrique-{}.tmp", std::process::id()));

    let result = (|| {
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        if let Ok(metadata) = fs::metadata(target) {
            fs::set_permissions(&temp_path, metadata.permissions())?;
        }
        fs::rename(&temp_path, target)
    })();
    if result.is_err() {
        // Best-effort cleanup; the write error is what the caller needs.
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    fn open_text(path: &Path) -> Document {
        match Document::open(path, None).unwrap() {
            Opened::Text(document) => document,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn unchanged_save_is_byte_identical() {
        // Blank lines, trailing spaces, CRLF, no final newline: all must survive.
        let original = "# Title\r\n\r\n\r\n\ntext  \n\t- item\n\n\n";
        let temp = TempFolder::new();
        let path = temp.write("note.md", original);

        let mut document = open_text(&path);
        assert!(!document.is_dirty());
        document.save().unwrap();
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
    }

    #[test]
    fn dirty_clears_when_text_returns_to_saved() {
        let temp = TempFolder::new();
        let path = temp.write("note.md", "hello");
        let mut document = open_text(&path);

        document.text.push('!');
        document.on_text_changed();
        assert!(document.is_dirty());

        document.text.pop();
        document.on_text_changed();
        assert!(!document.is_dirty());
    }

    #[test]
    fn save_writes_text_and_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp = TempFolder::new();
        let path = temp.write("run.sh", "echo a\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();

        let mut document = open_text(&path);
        document.text = "echo b\n".to_string();
        document.on_text_changed();
        document.save().unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "echo b\n");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o750);
        assert!(!document.is_dirty());
        // No temp files left behind.
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn save_through_symlink_keeps_the_link() {
        let temp = TempFolder::new();
        let real = temp.write("real.md", "a");
        let link = temp.path().join("link.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let mut document = open_text(&link);
        document.text = "b".to_string();
        document.save().unwrap();

        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "b");
    }

    #[test]
    fn marking_older_text_saved_keeps_later_edits_dirty() {
        let temp = TempFolder::new();
        let path = temp.write("note.md", "one");
        let mut document = open_text(&path);
        document.text = "two".to_string();
        document.on_text_changed();
        let uploaded = document.text.clone();
        document.text.push_str(" three");
        document.on_text_changed();
        let revision = document.revision();

        document.mark_saved(uploaded, None);
        assert!(document.is_dirty(), "edits after the upload are still unsaved");
        assert_eq!(document.base_text(), "two");
        assert_ne!(document.revision(), revision, "the draft is written again");

        let uploaded = document.text.clone();
        document.mark_saved(uploaded, None);
        assert!(!document.is_dirty());
    }

    #[test]
    fn rejects_binary_non_utf8_and_large_files() {
        let temp = TempFolder::new();
        let binary = temp.write("image.png", [0x89, b'P', b'N', b'G', 0, 1, 2]);
        let latin1 = temp.write("old.txt", [b'c', b'a', b'f', 0xE9]);
        let large = temp.write("big.log", "");
        fs::File::options().write(true).open(&large).unwrap().set_len(MAX_FILE_BYTES + 1).unwrap();

        for (path, expected) in [
            (binary, Unsupported::Binary),
            (latin1, Unsupported::NotUtf8),
            (large, Unsupported::TooLarge(MAX_FILE_BYTES + 1)),
        ] {
            match Document::open(&path, None).unwrap() {
                Opened::Unsupported(reason) => assert_eq!(reason, expected),
                _ => panic!("{} should not open", path.display()),
            }
        }
    }

    #[test]
    fn empty_file_opens() {
        let temp = TempFolder::new();
        let path = temp.write("empty", "");
        assert_eq!(open_text(&path).text, "");
    }

    #[test]
    fn detects_disk_changes_and_deletion() {
        let temp = TempFolder::new();
        let path = temp.write("note.md", "one");
        let mut document = open_text(&path);
        assert_eq!(document.disk_state(), DiskState::Unchanged);

        // Force a different mtime even on filesystems with coarse timestamps.
        let file = fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(std::time::SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(document.disk_state(), DiskState::Modified);

        document.acknowledge_disk_change();
        assert_eq!(document.disk_state(), DiskState::Unchanged);
        assert!(document.is_dirty(), "keeping our version must leave it dirty");

        fs::remove_file(&path).unwrap();
        assert_eq!(document.disk_state(), DiskState::Deleted);
        document.acknowledge_disk_change();
        assert_eq!(document.disk_state(), DiskState::Unchanged, "deletion is reported once");

        // Saving recreates the file.
        document.save().unwrap();
        assert!(path.exists());
        assert!(!document.is_dirty());
    }

    #[test]
    fn merge_bookkeeping_follows_the_disk() {
        let temp = TempFolder::new();
        let path = temp.write("a.md", "one\n");
        let mut document = open_text(&path);
        document.text = "one\nmine\n".to_string();
        document.on_text_changed();
        fs::write(&path, "theirs\none\n").unwrap();
        let disk = document.read_disk_text().unwrap();
        assert_eq!(document.base_text(), "one\n");
        document.apply_merge("theirs\none\nmine\n".to_string(), disk);
        assert!(document.is_dirty());
        assert_eq!(document.base_text(), "theirs\none\n");
        document.accept_disk_text("theirs\none\nmine\n".to_string());
        assert!(!document.is_dirty());
    }

    #[test]
    fn restored_edits_are_dirty_and_discard_returns_to_disk() {
        let temp = TempFolder::new();
        let path = temp.write("a.md", "theirs\n");
        let mut document = open_text(&path);
        let before = document.revision();
        document.restore_edits("mine\n".to_string(), "old\n".to_string());
        assert!(document.is_dirty());
        assert_ne!(document.revision(), before);
        assert_eq!(document.base_text(), "old\n");

        document.discard_changes();
        assert!(!document.is_dirty());
        assert_eq!(document.text, "theirs\n");
        assert_eq!(document.base_text(), "theirs\n");
    }

    #[test]
    fn missing_document_is_dirty_until_saved_or_discarded() {
        let temp = TempFolder::new();
        let path = temp.path().join("gone.md");
        let mut document = Document::missing(&path, "mine".to_string(), "old".to_string(), None);
        assert!(document.is_dirty());
        assert_eq!(document.disk_state(), DiskState::Unchanged, "already known to be missing");

        document.discard_changes();
        assert!(!document.is_dirty());
        assert_eq!(document.text, "old");
        assert!(!path.exists(), "discarding never writes the file");
    }

    #[test]
    fn encrypted_files_open_save_and_lock() {
        let temp = TempFolder::new();
        let (vault, _) = Vault::create("pw");
        let vault = Arc::new(vault);
        let path = temp.path().join("secret.md");
        fs::write(&path, vault.encrypt(b"hidden words")).unwrap();

        assert!(matches!(Document::open(&path, None).unwrap(), Opened::Locked));
        let (other, _) = Vault::create("pw");
        assert!(matches!(
            Document::open(&path, Some(&Arc::new(other))).unwrap(),
            Opened::Unsupported(Unsupported::WrongPassword)
        ));

        let Opened::Text(mut document) = Document::open(&path, Some(&vault)).unwrap() else {
            panic!("should open");
        };
        assert!(document.is_encrypted());
        assert_eq!(document.text, "hidden words");
        document.text.push_str(" and more");
        document.on_text_changed();
        document.save().unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(crypto::is_encrypted(&bytes));
        assert!(!bytes.windows(6).any(|window| window == b"hidden"));
        assert_eq!(document.read_disk_text().unwrap(), "hidden words and more");
        assert_eq!(document.disk_state(), DiskState::Unchanged);
    }

    #[test]
    fn plain_files_ignore_the_vault() {
        let temp = TempFolder::new();
        let path = temp.write("plain.md", "text");
        let vault = Arc::new(Vault::create("pw").0);
        let Opened::Text(mut document) = Document::open(&path, Some(&vault)).unwrap() else {
            panic!("should open");
        };
        assert!(!document.is_encrypted());
        document.save().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"text");
    }
}
