//! Track moved encrypted scratches for future password changes.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::remote;
use crate::settings::Settings;

/// Record both paths before a move so a restart can find the encrypted file
/// whether the filesystem change completed or was interrupted.
pub(super) fn follow_relocation(
    settings: &mut Settings,
    source: &Path,
    target: &Path,
    encrypted: bool,
    save_settings: impl FnOnce(&Settings) -> io::Result<()>,
    relocate: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let previous = &settings.privacy.moved_scratches;
    let changes = tracked_changes(previous, source, target, encrypted)?;
    if changes.is_empty() {
        return relocate();
    }
    let previous = previous.clone();
    let tracked = &mut settings.privacy.moved_scratches;
    for (old, new) in &changes {
        tracked.extend([old.clone(), new.clone()]);
    }
    tracked.sort_unstable();
    tracked.dedup();
    if let Err(error) = save_settings(settings) {
        settings.privacy.moved_scratches = previous;
        return Err(io::Error::other(format!("could not remember the encrypted file's new location: {error}")));
    }
    let result = relocate();
    if result.is_ok() {
        settings.privacy.moved_scratches.retain(|path| changes.iter().all(|(old, _)| old != path));
    } else {
        settings.privacy.moved_scratches = previous;
    }
    result
}

/// Resolve local paths so renaming through a linked parent follows the same file.
fn tracked_changes(
    tracked: &[PathBuf],
    source: &Path,
    target: &Path,
    encrypted: bool,
) -> io::Result<Vec<(PathBuf, PathBuf)>> {
    if (tracked.is_empty() && !encrypted) || remote::is_mirror(source) || fs::symlink_metadata(source)?.is_symlink() {
        return Ok(Vec::new());
    }
    let source = fs::canonicalize(source)?;
    let mut old_paths: Vec<_> = tracked.iter().filter(|path| path.starts_with(&source)).cloned().collect();
    if encrypted && !old_paths.contains(&source) {
        old_paths.push(source.clone());
    }
    if old_paths.is_empty() {
        return Ok(Vec::new());
    }
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "missing destination folder or name"));
    };
    let target = fs::canonicalize(parent)?.join(name);
    Ok(old_paths
        .into_iter()
        .filter_map(|old| {
            let relative = old.strip_prefix(&source).ok()?;
            let new = if relative.as_os_str().is_empty() { target.clone() } else { target.join(relative) };
            (old != new).then_some((old, new))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Vault;
    use crate::test_support::TempFolder;

    #[test]
    fn renaming_a_folder_keeps_the_scratches_inside_it_tracked() {
        let temp = TempFolder::new();
        let (old, _) = Vault::create("old password");
        let source = temp.write("dev/project/nested/secret.md", old.encrypt(b"secret"));
        let root = temp.path().join("dev/project");
        let renamed = temp.path().join("dev/renamed");
        let destination = renamed.join("nested/secret.md");
        let config = temp.path().join("settings");
        let mut settings = Settings::default();
        settings.privacy.moved_scratches.push(source.clone());

        follow_relocation(
            &mut settings,
            &root,
            &renamed,
            false,
            |settings| {
                assert_eq!(settings.privacy.moved_scratches, [source.clone(), destination.clone()]);
                fs::write(&config, settings.to_text())
            },
            || fs::rename(&root, &renamed),
        )
        .unwrap();

        assert_eq!(settings.privacy.moved_scratches.as_slice(), std::slice::from_ref(&destination));
        let saved = Settings::parse(&fs::read_to_string(config).unwrap());
        let (new, _) = Vault::create("new password");
        let changed = crate::scratch::reencrypt_scratches(
            &temp.path().join("scratches"),
            &saved.privacy.moved_scratches,
            &old,
            &new,
        )
        .unwrap();
        assert_eq!(changed.as_slice(), std::slice::from_ref(&destination));
        assert_eq!(new.decrypt(&fs::read(destination).unwrap()).unwrap(), b"secret");
    }
}
