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

    /// Move the file, using `home_trash` for files on the home drive.
    pub(super) fn move_with_home_trash(path: &Path, home_trash: &Path) -> io::Result<()> {
        let device = path.symlink_metadata()?.dev();
        if same_device(home_trash, device) {
            fs::DirBuilder::new().recursive(true).mode(0o700).create(home_trash)?;
            return move_into(home_trash, path, path);
        }
        let top = top_folder(path, device);
        let trash = drive_trash(&top)?;
        let relative = path.strip_prefix(&top).unwrap_or(path);
        move_into(&trash, path, relative)
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
    fn top_folder(path: &Path, device: u64) -> PathBuf {
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
    /// deletes never share a name), then move the file in.
    fn move_into(trash: &Path, path: &Path, recorded: &Path) -> io::Result<()> {
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
            let moved = file.write_all(contents.as_bytes()).and_then(|()| fs::rename(path, &target));
            if moved.is_err() {
                let _ = fs::remove_file(&info_path);
            }
            return moved;
        }
        Err(io::Error::other("the Trash holds too many files with this name"))
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

    #[test]
    fn refuses_a_missing_file() {
        assert!(move_to_trash(Path::new("/nonexistent-folder-for-rubrique/x")).is_err());
    }
}
