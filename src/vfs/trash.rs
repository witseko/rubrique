//! Move a local file or folder to the system Trash, so a delete can be
//! undone: through Finder on macOS (which keeps "Put Back" working), and by
//! the freedesktop.org Trash specification on Linux.

use std::io;
use std::path::{Path, PathBuf};

/// Move `path` to the Trash. A symbolic link is trashed itself, not its
/// target.
///
/// # Errors
///
/// The file is gone, permissions, or no Trash for this file's drive.
pub fn move_to_trash(path: &Path) -> io::Result<()> {
    let path = absolute_keeping_link(path)?;
    platform::move_to_trash(&path)
}

/// The absolute path of `path` with its folder resolved, but the last part
/// kept, so a link is not followed to its target.
fn absolute_keeping_link(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "nothing to trash"))?;
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.canonicalize()?,
        _ => std::env::current_dir()?,
    };
    let path = parent.join(name);
    // Fail early, and alike on both systems, when nothing is there.
    path.symlink_metadata()?;
    Ok(path)
}

#[cfg(target_os = "macos")]
mod platform {
    use std::io;
    use std::path::Path;
    use std::process::Command;

    /// Ask Finder to delete the file, as the Finder menu would.
    pub fn move_to_trash(path: &Path) -> io::Result<()> {
        let path = path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not UTF-8"))?;
        let script = format!("tell application \"Finder\" to delete POSIX file \"{}\"", applescript_escape(path));
        let output = Command::new("osascript").arg("-e").arg(script).output()?;
        if output.status.success() {
            Ok(())
        } else {
            let message = String::from_utf8_lossy(&output.stderr);
            Err(io::Error::other(format!("Finder could not move it to the Trash: {}", message.trim())))
        }
    }

    /// Escape `"` and `\` for a string in `AppleScript`.
    pub(super) fn applescript_escape(text: &str) -> String {
        text.replace('\\', "\\\\").replace('"', "\\\"")
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use std::fmt::Write as _;
    use std::fs::{self, OpenOptions};
    use std::io::{self, Write};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    /// Most names tried when the Trash already holds the same file name.
    const MAX_NAME_ATTEMPTS: u32 = 10_000;

    /// Move the file into the Trash of its drive and write its `.trashinfo`.
    pub fn move_to_trash(path: &Path) -> io::Result<()> {
        move_with_home_trash(path, &home_trash()?)
    }

    /// Move the file into the Trash of its own drive, or into `home_trash`
    /// when it is on the home drive or its drive has no usable Trash (for
    /// example a top folder the user cannot write). Only the home Trash
    /// copies across drives, as the specification allows.
    pub(super) fn move_with_home_trash(path: &Path, home_trash: &Path) -> io::Result<()> {
        let device = path.symlink_metadata()?.dev();
        if !same_device(home_trash, device) {
            let top = top_folder(path, device);
            let relative = path.strip_prefix(&top).unwrap_or(path);
            if let Ok(trash) = drive_trash(&top)
                && move_into(&trash, path, relative, false).is_ok()
            {
                return Ok(());
            }
        }
        fs::DirBuilder::new().recursive(true).mode(0o700).create(home_trash)?;
        move_into(home_trash, path, path, true)
    }

    /// `$XDG_DATA_HOME/Trash`, or `~/.local/share/Trash`.
    fn home_trash() -> io::Result<PathBuf> {
        if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|data| Path::new(data).is_absolute()) {
            return Ok(PathBuf::from(data).join("Trash"));
        }
        let home =
            std::env::var_os("HOME").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
        Ok(PathBuf::from(home).join(".local/share/Trash"))
    }

    /// Whether `path` (or its nearest existing folder) is on `device`.
    fn same_device(path: &Path, device: u64) -> bool {
        path.ancestors().find_map(|folder| fs::metadata(folder).ok()).is_some_and(|metadata| metadata.dev() == device)
    }

    /// The highest folder above `path` still on `device`: the drive's mount
    /// point.
    pub(super) fn top_folder(path: &Path, device: u64) -> PathBuf {
        let mut top = path.parent().unwrap_or(path).to_path_buf();
        while let Some(parent) = top.parent() {
            if fs::metadata(parent).map(|metadata| metadata.dev()).ok() != Some(device) {
                break;
            }
            top = parent.to_path_buf();
        }
        top
    }

    /// The Trash on another drive: `$top/.Trash/$uid` when an administrator
    /// made a shared `.Trash` (a real, sticky folder), else `$top/.Trash-$uid`.
    fn drive_trash(top: &Path) -> io::Result<PathBuf> {
        let uid = rustix::process::getuid().as_raw();
        let shared = top.join(".Trash");
        if let Ok(metadata) = shared.symlink_metadata()
            && metadata.is_dir()
            && metadata.permissions().mode() & 0o1000 != 0
        {
            let own = shared.join(uid.to_string());
            if fs::DirBuilder::new().recursive(true).mode(0o700).create(&own).is_ok() {
                return Ok(own);
            }
        }
        let own = top.join(format!(".Trash-{uid}"));
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&own)?;
        if own.symlink_metadata()?.is_dir() {
            Ok(own)
        } else {
            Err(io::Error::other(format!("{} is not a folder", own.display())))
        }
    }

    /// Claim a free name with its `.trashinfo` (created exclusively, so two
    /// deletes never share a name), then move the file in. With
    /// `copy_across_drives`, a file on another drive is copied, then removed.
    fn move_into(trash: &Path, path: &Path, recorded: &Path, copy_across_drives: bool) -> io::Result<()> {
        let files = trash.join("files");
        let info = trash.join("info");
        for folder in [&files, &info] {
            fs::DirBuilder::new().recursive(true).mode(0o700).create(folder)?;
        }
        let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "nothing to trash"))?;
        let contents = trash_info(recorded);
        for attempt in 1..=MAX_NAME_ATTEMPTS {
            let mut candidate = name.to_os_string();
            if attempt > 1 {
                candidate.push(format!(".{attempt}"));
            }
            let mut info_name = candidate.clone();
            info_name.push(".trashinfo");
            let info_path = info.join(&info_name);
            let mut file = match OpenOptions::new().write(true).create_new(true).open(&info_path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            };
            let target = files.join(&candidate);
            if target.symlink_metadata().is_ok() {
                // A file without its info entry; leave it and try the next name.
                drop(file);
                let _ = fs::remove_file(&info_path);
                continue;
            }
            let copied = file.write_all(contents.as_bytes()).and_then(|()| match fs::rename(path, &target) {
                Ok(()) => Ok(false),
                Err(error) if copy_across_drives && error.kind() == io::ErrorKind::CrossesDevices => {
                    match copy_tree(path, &target) {
                        Ok(()) => Ok(true),
                        Err(error) => {
                            let _ = remove_tree(&target);
                            Err(error)
                        }
                    }
                }
                Err(error) => Err(error),
            });
            return match copied {
                Ok(false) => Ok(()),
                // The Trash entry is complete; keep it even if the original
                // cannot be removed, so nothing is lost.
                Ok(true) => remove_tree(path),
                Err(error) => {
                    let _ = fs::remove_file(&info_path);
                    Err(error)
                }
            };
        }
        Err(io::Error::other("the Trash holds too many files with this name"))
    }

    /// Copy a file, folder or link to `target`, keeping links as links.
    pub(super) fn copy_tree(from: &Path, target: &Path) -> io::Result<()> {
        let metadata = from.symlink_metadata()?;
        if metadata.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(from)?, target)
        } else if metadata.is_dir() {
            fs::create_dir(target)?;
            for entry in fs::read_dir(from)? {
                let entry = entry?;
                copy_tree(&entry.path(), &target.join(entry.file_name()))?;
            }
            // Set last, so a read-only folder can still be filled.
            fs::set_permissions(target, metadata.permissions())
        } else if metadata.is_file() {
            fs::copy(from, target).map(|_| ())
        } else {
            // Copying a pipe or a device would read from it.
            Err(io::Error::new(io::ErrorKind::Unsupported, format!("cannot copy {} to the Trash", from.display())))
        }
    }

    /// Remove a file, link or whole folder.
    fn remove_tree(path: &Path) -> io::Result<()> {
        if path.symlink_metadata()?.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }
    }

    /// The `.trashinfo` text: the original path, percent-encoded, and the
    /// local deletion time.
    pub(super) fn trash_info(path: &Path) -> String {
        let now = crate::date::now_local();
        format!(
            "[Trash Info]\nPath={}\nDeletionDate={}T{:02}:{:02}:00\n",
            percent_encode(path.as_os_str().as_bytes()),
            now.date,
            now.hour,
            now.minute
        )
    }

    /// Percent-encode a path as a URL path (RFC 3986), keeping `/`.
    pub(super) fn percent_encode(bytes: &[u8]) -> String {
        let mut encoded = String::with_capacity(bytes.len());
        for &byte in bytes {
            if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                encoded.push(char::from(byte));
            } else {
                // Writing to a String cannot fail.
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
        encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn escapes_quotes_and_backslashes_for_finder() {
        assert_eq!(platform::applescript_escape(r#"/tmp/a "b"\c"#), r#"/tmp/a \"b\"\\c"#);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn encodes_the_original_path_in_the_trash_info() {
        assert_eq!(
            platform::percent_encode(b"/home/a b/r\xc3\xa9sum\xc3\xa9%.md"),
            "/home/a%20b/r%C3%A9sum%C3%A9%25.md"
        );
        let info = platform::trash_info(Path::new("/tmp/x y"));
        assert!(info.starts_with("[Trash Info]\nPath=/tmp/x%20y\nDeletionDate="), "{info}");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn moves_files_into_the_home_trash_with_distinct_names() {
        let root = crate::test_support::TempFolder::new();
        let trash = root.path().join("data/Trash");
        let notes = root.make_folder("notes");
        for _ in 0..2 {
            let file = notes.join("a.md");
            std::fs::write(&file, "x").unwrap();
            platform::move_with_home_trash(&absolute_keeping_link(&file).unwrap(), &trash).unwrap();
            assert!(!file.exists());
        }
        assert!(trash.join("files/a.md").exists());
        assert!(trash.join("files/a.md.2").exists());
        let info = std::fs::read_to_string(trash.join("info/a.md.2.trashinfo")).unwrap();
        assert!(info.contains(&format!("Path={}", notes.join("a.md").display())), "{info}");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn copies_folders_with_their_links() {
        let root = crate::test_support::TempFolder::new();
        let source = root.make_folder("source");
        root.write("source/inner/a.md", "x");
        std::os::unix::fs::symlink("inner/a.md", source.join("link")).unwrap();
        let copy = root.path().join("copy");
        platform::copy_tree(&source, &copy).unwrap();
        assert_eq!(std::fs::read_to_string(copy.join("inner/a.md")).unwrap(), "x");
        assert_eq!(std::fs::read_link(copy.join("link")).unwrap(), Path::new("inner/a.md"));
    }

    /// A file on a drive whose top folder the user cannot write goes to the
    /// home Trash on another drive, copied across. Runs where the temporary
    /// folder's drive has a top folder the user cannot write (such as `/` on
    /// CI) and `/dev/shm` is another drive.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn falls_back_to_the_home_trash_on_another_drive() {
        use std::os::unix::fs::MetadataExt;
        let root = crate::test_support::TempFolder::new();
        let device = root.path().metadata().unwrap().dev();
        let top = platform::top_folder(root.path(), device);
        let top_is_writable = rustix::fs::access(&top, rustix::fs::Access::WRITE_OK).is_ok();
        let shm_is_separate = std::fs::metadata("/dev/shm").is_ok_and(|shm| shm.dev() != device);
        if top_is_writable || !shm_is_separate {
            eprintln!("skipped: needs a read-only top folder for {} and /dev/shm on another drive", top.display());
            return;
        }
        let home = Path::new("/dev/shm").join(format!("rubrique-trash-test-{}", std::process::id()));
        let file = root.write("notes/a.md", "x");
        let result = platform::move_with_home_trash(&file, &home.join("Trash"));
        let trashed = std::fs::read_to_string(home.join("Trash/files/a.md"));
        let _ = std::fs::remove_dir_all(&home);
        result.unwrap();
        assert!(!file.exists());
        assert_eq!(trashed.unwrap(), "x");
    }

    #[test]
    fn refuses_a_missing_file() {
        assert!(move_to_trash(Path::new("/nonexistent-folder-for-rubrique/x")).is_err());
    }
}
