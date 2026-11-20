//! Install and verify the pinned local Phonon checkpoint.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};

use rustix::fs::{CWD, RenameFlags, renameat_with};
use sha2::{Digest, Sha256};

/// The checkpoint revision shared by the download and the local model folder.
pub const REVISION: &str = "52433c46975bfa13144c56204c9f4ee1c5b148aa";

/// One file from the checkpoint and the bytes expected from the publisher.
struct ModelFile {
    name: &'static str,
    bytes: u64,
    sha256: &'static str,
}

const FILES: &[ModelFile] = &[
    ModelFile {
        name: "config.json",
        bytes: 2_851,
        sha256: "7062ea376e6b158543c8abec8e5af67df171c49ad70eb7e2a0fb25c16f265e06",
    },
    ModelFile {
        name: "tokenizer.json",
        bytes: 277_967,
        sha256: "691a00068fa7b989f2f3e5ae5daa2d1a5bb319f2d5360deafcfb19a8bb6e7936",
    },
    ModelFile {
        name: "default-voice.safetensors",
        bytes: 256_136,
        sha256: "f739e8c4c76ee7f92e7c94a82002748c8cd22e99582ac49f7b9b94cd63786e18",
    },
    ModelFile {
        name: "model.q8.gguf",
        bytes: 64_512_352,
        sha256: "a7d9aad2cd6dc66473f24a08bc6d2939030ef4d751ef82349361fbef3272dbae",
    },
];

/// Check the four file sizes without reading the weights on the UI thread.
pub fn is_installed(folder: &Path) -> bool {
    files_present(folder, FILES)
}

/// Verify the checkpoint before loading it into the speech engine.
pub fn verify(folder: &Path) -> io::Result<()> {
    verify_files(folder, FILES)
}

/// Download the checkpoint and publish its folder after verification.
pub fn download(curl: &Path, folder: &Path) -> io::Result<()> {
    install_files(folder, FILES, |file, path| download_file(curl, file, path))
}

/// Stage files beside the destination so publication is atomic.
fn install_files(
    folder: &Path,
    files: &[ModelFile],
    mut fetch: impl FnMut(&ModelFile, &Path) -> io::Result<()>,
) -> io::Result<()> {
    if folder.symlink_metadata().is_ok() {
        return verify_existing(folder, files);
    }
    let parent = folder.parent().ok_or_else(|| io::Error::other("Missing parent folder for Phonon"))?;
    fs::create_dir_all(parent)?;
    let staging = tempfile::Builder::new().prefix(".phonon-").tempdir_in(parent)?;
    for file in files {
        let path = staging.path().join(file.name);
        fetch(file, &path)?;
        verify_file(&path, file)?;
    }
    match renameat_with(CWD, staging.path(), CWD, folder, RenameFlags::NOREPLACE).map_err(io::Error::from) {
        Ok(()) => {
            let _ = staging.keep();
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => verify_existing(folder, files),
        Err(error) => Err(error),
    }
}

/// Check that all expected assets are regular files of the right size.
fn files_present(folder: &Path, files: &[ModelFile]) -> bool {
    files.iter().all(|file| expected_size(&folder.join(file.name), file.bytes))
}

/// Check a file's type and size without following links to another file.
fn expected_size(path: &Path, bytes: u64) -> bool {
    path.symlink_metadata().is_ok_and(|entry| entry.is_file() && entry.len() == bytes)
}

/// Verify a previous or concurrent install without replacing its files.
fn verify_existing(folder: &Path, files: &[ModelFile]) -> io::Result<()> {
    verify_files(folder, files).map_err(|error| {
        io::Error::other(format!("{error}. Move the existing model folder before retrying: {}", folder.display()))
    })
}

/// Verify each asset in a checkpoint folder.
fn verify_files(folder: &Path, files: &[ModelFile]) -> io::Result<()> {
    files.iter().try_for_each(|file| verify_file(&folder.join(file.name), file))
}

/// Compare a file against its expected size and SHA-256.
fn verify_file(path: &Path, file: &ModelFile) -> io::Result<()> {
    if !expected_size(path, file.bytes) {
        return Err(io::Error::other(format!("{} is missing or has the wrong size", file.name)));
    }
    let mut input = File::open(path)?.take(file.bytes + 1);
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let bytes = input.read(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        digest.update(&buffer[..bytes]);
    }
    let mut hash = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut hash, "{byte:02x}").map_err(io::Error::other)?;
    }
    if hash == file.sha256 {
        return Ok(());
    }
    Err(io::Error::other(format!("{} failed its integrity check", file.name)))
}

/// Fetch one HTTPS asset with a bounded transfer and without shell or curl configuration.
fn download_file(curl: &Path, file: &ModelFile, path: &Path) -> io::Result<()> {
    let mut output = File::create_new(path)?;
    let url = format!("https://huggingface.co/gradium/phonon-31m-en/resolve/{REVISION}/{}", file.name);
    let mut child = Command::new(curl)
        .args([
            "-q",
            "--fail",
            "--silent",
            "--location",
            "--max-redirs",
            "5",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "300",
            "--max-filesize",
        ])
        .arg(file.bytes.to_string())
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let result = match child.stdout.take() {
        Some(input) => io::copy(&mut input.take(file.bytes + 1), &mut output),
        None => Err(io::Error::other("Could not read the download")),
    };
    if !matches!(&result, Ok(bytes) if *bytes == file.bytes) {
        let _ = child.kill();
    }
    let status = child.wait()?;
    if result? == file.bytes && status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("Could not download {}. Check your connection and try again.", file.name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[ModelFile] = &[ModelFile {
        name: "model",
        bytes: 3,
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    }];

    #[test]
    fn corrupt_download_leaves_no_install_and_can_be_retried() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let model = root.path().join("phonon");
        assert!(install_files(&model, FIXTURE, |_, path| fs::write(path, b"bad")).is_err());
        assert_eq!(root.path().read_dir()?.count(), 0);

        install_files(&model, FIXTURE, |_, path| fs::write(path, b"abc"))?;
        assert_eq!(fs::read(model.join("model"))?, b"abc");
        install_files(&model, FIXTURE, |_, _| panic!("an installed model must not download again"))?;
        Ok(())
    }

    #[test]
    fn leaves_an_existing_incomplete_folder_in_place() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let model = root.path().join("phonon");
        fs::create_dir(&model)?;
        fs::write(model.join("model"), b"user file")?;
        assert!(install_files(&model, FIXTURE, |_, _| panic!("must preserve existing files")).is_err());
        assert_eq!(fs::read(model.join("model"))?, b"user file");
        Ok(())
    }

    #[test]
    #[ignore = "downloads the pinned checkpoint into the provided test folder"]
    fn downloads_phonon_checkpoint() -> io::Result<()> {
        let folder = std::env::var_os("RUBRIQUE_PHONON_TEST_DOWNLOAD").expect("set a test destination");
        let curl = crate::install::find_installer("curl").expect("curl is installed");
        download(&curl, Path::new(&folder))?;
        verify(Path::new(&folder))
    }
}
