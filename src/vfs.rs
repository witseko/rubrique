//! File access for local files and remote (mirror path) files alike.
//!
//! Local paths go to `std::fs`. Mirror paths (see [`crate::remote`]) go to
//! the host over SSH, and what is read or written is also kept in the
//! mirror, so images, agents and the browser copy can use it locally.

use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use crate::document::text_file::write_atomically;
use crate::remote::{self, commands};

mod trash;

/// A file's version, to notice when another program changed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stamp {
    /// Modification time of a local file.
    Local(SystemTime),
    /// `cksum` of a remote file ("checksum size"); comparing modification
    /// times would need a different `stat` on every system.
    Remote(String),
}

/// A file's bytes, or why they were not read.
#[derive(Debug, PartialEq, Eq)]
pub enum Contents {
    Bytes(Vec<u8>),
    /// Over the size limit; holds the size in bytes.
    TooLarge(u64),
}

/// Read a file of at most `max_bytes`, with its stamp.
///
/// # Errors
///
/// Missing file, permissions, or the host cannot be reached.
pub fn read_file(path: &Path, max_bytes: u64) -> io::Result<(Contents, Option<Stamp>)> {
    let Some(remote) = remote::from_mirror(path) else {
        let metadata = fs::metadata(path)?;
        if metadata.len() > max_bytes {
            return Ok((Contents::TooLarge(metadata.len()), None));
        }
        let bytes = fs::read(path)?;
        return Ok((Contents::Bytes(bytes), metadata.modified().ok().map(Stamp::Local)));
    };
    match commands::read(&remote.host, &remote.path, max_bytes)? {
        commands::Read::TooLarge(size) => Ok((Contents::TooLarge(size), None)),
        commands::Read::Bytes(bytes) => {
            write_mirror(path, &bytes);
            let stamp = remote_stamp(&bytes);
            Ok((Contents::Bytes(bytes), Some(stamp)))
        }
    }
}

/// The file's stamp now: `Ok(None)` when it is gone, `Err` when that cannot
/// be told (the host does not answer).
///
/// # Errors
///
/// Only for remote files, when the host cannot be reached.
pub fn current_stamp(path: &Path) -> io::Result<Option<Stamp>> {
    let Some(remote) = remote::from_mirror(path) else {
        return Ok(fs::metadata(path).ok().map(|metadata| Stamp::Local(modified(&metadata))));
    };
    Ok(commands::stamp(&remote.host, &remote.path)?.map(Stamp::Remote))
}

/// The stamp of a file that was just read or written with `bytes`. Free for
/// remote files: the checksum is worked out here.
pub fn stamp_of(path: &Path, bytes: &[u8]) -> Option<Stamp> {
    if remote::is_mirror(path) {
        return Some(remote_stamp(bytes));
    }
    fs::metadata(path).ok().map(|metadata| Stamp::Local(modified(&metadata)))
}

fn remote_stamp(bytes: &[u8]) -> Stamp {
    Stamp::Remote(remote::cksum::cksum_text(bytes))
}

fn modified(metadata: &fs::Metadata) -> SystemTime {
    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Replace the file with `contents` so that a failure leaves the old file.
/// Writes through symlinks. Returns the new stamp.
///
/// # Errors
///
/// The file is left as it was.
pub fn save(path: &Path, contents: &[u8]) -> io::Result<Option<Stamp>> {
    let Some(remote) = remote::from_mirror(path) else {
        // Renaming over a link would replace the link with a plain file.
        let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        write_atomically(&target, contents)?;
        return Ok(stamp_of(path, contents));
    };
    commands::write(&remote.host, &remote.path, contents)?;
    write_mirror(path, contents);
    Ok(Some(remote_stamp(contents)))
}

/// Rename a file, never replacing another one.
///
/// # Errors
///
/// The target exists, the file is gone, permissions, or no connection.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    match (remote::from_mirror(from), remote::from_mirror(to)) {
        (None, None) => fs::rename(from, to),
        (Some(from_remote), Some(to_remote)) if from_remote.host == to_remote.host => {
            commands::rename(&from_remote.host, &from_remote.path, &to_remote.path)?;
            // The mirror follows; it is only a cache, so failing here is fine.
            if let Some(folder) = to.parent() {
                let _ = fs::create_dir_all(folder);
            }
            let _ = fs::rename(from, to);
            Ok(())
        }
        _ => Err(io::Error::new(io::ErrorKind::Unsupported, "cannot move files between computers")),
    }
}

/// Delete a file: local files go to the system Trash, so it can be undone;
/// remote files are deleted for good (a host has no Trash we can reach).
///
/// # Errors
///
/// The file is gone, permissions, no Trash on this system, or no connection.
pub fn delete(path: &Path) -> io::Result<()> {
    let Some(remote) = remote::from_mirror(path) else {
        return trash::move_to_trash(path);
    };
    commands::delete(&remote.host, &remote.path)?;
    // The mirror copy is only a cache, so failing here is fine.
    let _ = fs::remove_file(path);
    Ok(())
}

/// True when something is at `path`.
pub fn exists(path: &Path) -> bool {
    match remote::from_mirror(path) {
        Some(remote) => commands::exists(&remote.host, &remote.path),
        None => path.exists(),
    }
}

/// Download a remote file into its mirror (for images and videos, which
/// are shown from local files). Local files are left alone. Returns the size.
///
/// # Errors
///
/// Missing, too large (over `max_bytes`), or no connection.
pub fn fetch_to_mirror(path: &Path, max_bytes: u64) -> io::Result<u64> {
    if !remote::is_mirror(path) {
        return fs::metadata(path).map(|metadata| metadata.len());
    }
    match read_file(path, max_bytes)? {
        (Contents::Bytes(bytes), _) => Ok(bytes.len() as u64),
        (Contents::TooLarge(size), _) => Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            format!("too large to download ({})", crate::media::size_text(size)),
        )),
    }
}

/// Keep a copy of a remote file in its mirror. Best effort: the mirror is
/// only a cache.
pub fn write_mirror(path: &Path, bytes: &[u8]) {
    if let Some(folder) = path.parent()
        && fs::create_dir_all(folder).is_ok()
    {
        let _ = write_atomically(path, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::testing::mirror_of;
    use crate::test_support::TempFolder;

    #[test]
    fn remote_files_round_trip_through_the_mirror() {
        let temp = TempFolder::new();
        let real = temp.write("notes/a.md", "hello");
        let mirror = mirror_of("vfs-host", &real);

        let (contents, stamp) = read_file(&mirror, 1000).unwrap();
        assert_eq!(contents, Contents::Bytes(b"hello".to_vec()));
        assert_eq!(fs::read_to_string(&mirror).unwrap(), "hello");
        assert_eq!(current_stamp(&mirror).unwrap(), stamp);

        let new_stamp = save(&mirror, b"changed").unwrap();
        assert_eq!(fs::read_to_string(&real).unwrap(), "changed");
        assert_eq!(fs::read_to_string(&mirror).unwrap(), "changed");
        assert_eq!(current_stamp(&mirror).unwrap(), new_stamp);
        assert_ne!(new_stamp, stamp);

        fs::remove_file(&real).unwrap();
        assert_eq!(current_stamp(&mirror).unwrap(), None);
    }

    #[test]
    fn remote_rename_and_exists() {
        let temp = TempFolder::new();
        let real = temp.write("a.md", "x");
        let from = mirror_of("vfs-host", &real);
        let to = mirror_of("vfs-host", &temp.path().join("b.md"));
        assert!(exists(&from));
        rename(&from, &to).unwrap();
        assert!(exists(&to) && !exists(&from));
        assert!(temp.path().join("b.md").is_file());
        let local = temp.path().join("c.md");
        assert_eq!(rename(&to, &local).unwrap_err().kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn remote_delete_removes_the_file_and_its_mirror_copy() {
        let temp = TempFolder::new();
        let real = temp.write("a.md", "x");
        let mirror = mirror_of("vfs-host", &real);
        read_file(&mirror, 1000).unwrap();
        assert!(mirror.is_file());
        delete(&mirror).unwrap();
        assert!(!real.exists() && !mirror.exists());
    }

    #[test]
    fn local_files_are_untouched_by_remote_code() {
        let temp = TempFolder::new();
        let path = temp.write("a.md", "local");
        let (contents, stamp) = read_file(&path, 1000).unwrap();
        assert_eq!(contents, Contents::Bytes(b"local".to_vec()));
        assert!(matches!(stamp, Some(Stamp::Local(_))));
        assert_eq!(read_file(&path, 2).unwrap().0, Contents::TooLarge(5));
    }

    #[test]
    fn fetches_media_into_the_mirror() {
        let temp = TempFolder::new();
        let real = temp.write("images/cat.png", "png bytes");
        let mirror = mirror_of("vfs-host", &real);
        assert_eq!(fetch_to_mirror(&mirror, 100).unwrap(), 9);
        assert_eq!(fs::read(&mirror).unwrap(), b"png bytes");
        assert_eq!(fetch_to_mirror(&mirror, 3).unwrap_err().kind(), io::ErrorKind::FileTooLarge);
    }
}
