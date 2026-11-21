//! Image and video file details and helpers for opening paths with the system.

use std::path::Path;
use std::process::{Command, Stdio};

const SYSTEM_OPENER: &str = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };

/// Image formats Rubrique decodes and shows (the `image` crate features in
/// `Cargo.toml`). SVG is left out: it opens as XML text.
const IMAGE_EXTENSIONS: [&str; 10] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff", "jfif"];

/// Video formats. Rubrique cannot play them in the window; it shows a card
/// that opens them in the system player or the browser.
const VIDEO_EXTENSIONS: [&str; 12] =
    ["mp4", "m4v", "mov", "webm", "mkv", "avi", "ogv", "mpg", "mpeg", "wmv", "flv", "3gp"];

/// Largest image file the preview loads, local or downloaded from a remote
/// folder. egui reads the whole file into memory before decoding it, so a
/// link to a disk image or a log named `.png` must not fill the memory; a
/// full-size phone photo or a large screenshot is well under 20 MB.
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// What kind of media a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
}

/// The media kind of `path`, from its extension (any case).
pub fn media_kind(path: &Path) -> Option<MediaKind> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        Some(MediaKind::Image)
    } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

/// Has an image extension Rubrique shows.
pub fn is_image_path(path: &Path) -> bool {
    media_kind(path) == Some(MediaKind::Image)
}

/// Media kind of a link target in Markdown (`![alt](clip.mp4)`), ignoring
/// any `?query` or `#fragment`.
pub fn media_kind_of_url(url: &str) -> Option<MediaKind> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    media_kind(Path::new(path))
}

/// The `file://` address egui's image loader reads `path` from.
pub fn image_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// The file format in capitals, such as "PNG" or "MP4".
pub fn format_name(path: &Path) -> String {
    path.extension().map(|extension| extension.to_string_lossy().to_ascii_uppercase()).unwrap_or_default()
}

/// A file size for people: "812 bytes", "4.2 KB", "31.0 MB".
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 3] = ["KB", "MB", "GB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    #[expect(clippy::cast_precision_loss, reason = "one decimal is shown, so the lost precision never matters")]
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Open `path` with the program the system uses for its type (the video
/// player for a video). Returns an error message for the status bar.
pub fn open_with_system(path: &Path) -> Result<(), String> {
    start_opener(&mut opener_command(path)?)
}

/// Build the opener command for `path`, made absolute so that a name such
/// as `-a` can never be read as an option (`xdg-open` has no `--`).
fn opener_command(path: &Path) -> Result<Command, String> {
    let path = std::path::absolute(path).map_err(|error| format!("Cannot resolve path: {error}"))?;
    let mut command = Command::new(SYSTEM_OPENER);
    command.arg(path);
    Ok(command)
}

/// Open a local folder in Finder or the default Linux file manager.
pub fn open_in_file_manager(path: &Path) -> Result<(), String> {
    start_opener(&mut file_manager_command(path)?)
}

/// Build the opener command after checking that the target is a folder.
fn file_manager_command(path: &Path) -> Result<Command, String> {
    let path = std::path::absolute(path).map_err(|error| format!("Cannot resolve folder path: {error}"))?;
    if !path.is_dir() {
        return Err(format!("Cannot open folder: {}", path.display()));
    }
    let mut command = Command::new(SYSTEM_OPENER);
    // Finder should open the directory even when macOS treats it as an app bundle.
    if cfg!(target_os = "macos") {
        command.args(["-b", "com.apple.finder"]);
    }
    command.arg(path);
    Ok(command)
}

/// Start the system opener and report a failure to launch it.
fn start_opener(command: &mut Command) -> Result<(), String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|error| format!("Cannot start {SYSTEM_OPENER}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    #[test]
    fn file_manager_passes_the_folder_path_as_one_argument() {
        let folder = TempFolder::new();
        let target = folder.path().join("- My notes.app");
        std::fs::create_dir(&target).unwrap();

        let command = file_manager_command(&target).unwrap();
        assert_eq!(command.get_program(), SYSTEM_OPENER);
        let args: Vec<_> = command.get_args().collect();
        let target = target.as_os_str();
        if cfg!(target_os = "macos") {
            assert_eq!(args, ["-b".as_ref(), "com.apple.finder".as_ref(), target]);
        } else {
            assert_eq!(args, [target]);
        }
    }

    #[test]
    fn opener_never_gets_a_path_that_starts_with_a_dash() {
        let command = opener_command(Path::new("-a Calculator")).unwrap();
        assert_eq!(command.get_program(), SYSTEM_OPENER);
        let args: Vec<_> = command.get_args().collect();
        let expected = std::env::current_dir().unwrap().join("-a Calculator");
        assert_eq!(args, [expected.as_os_str()]);
        assert!(Path::new(args[0]).is_absolute());
    }

    #[test]
    fn file_manager_rejects_files_and_missing_folders() {
        let folder = TempFolder::new();
        let file = folder.write("notes.md", "notes");
        assert!(open_in_file_manager(&file).is_err());
        assert!(open_in_file_manager(&folder.path().join("missing")).is_err());
    }

    #[test]
    fn kinds_come_from_the_extension_in_any_case() {
        assert_eq!(media_kind(Path::new("a/photo.PNG")), Some(MediaKind::Image));
        assert_eq!(media_kind(Path::new("shot.jpeg")), Some(MediaKind::Image));
        assert_eq!(media_kind(Path::new("clip.mp4")), Some(MediaKind::Video));
        assert_eq!(media_kind(Path::new("clip.WebM")), Some(MediaKind::Video));
        assert_eq!(media_kind(Path::new("notes.md")), None);
        assert_eq!(media_kind(Path::new("logo.svg")), None);
        assert_eq!(media_kind(Path::new("png")), None);
        assert!(is_image_path(Path::new("a/photo.JPG")));
        assert!(!is_image_path(Path::new("clip.mp4")));
    }

    #[test]
    fn url_kind_ignores_query_and_fragment() {
        assert_eq!(media_kind_of_url("img/a.png?raw=1"), Some(MediaKind::Image));
        assert_eq!(media_kind_of_url("demo.mov#t=3"), Some(MediaKind::Video));
        assert_eq!(media_kind_of_url("page.html"), None);
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(size_text(812), "812 bytes");
        assert_eq!(size_text(4300), "4.2 KB");
        assert_eq!(size_text(31 * 1024 * 1024), "31.0 MB");
        assert_eq!(size_text(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn names_and_uris() {
        assert_eq!(format_name(Path::new("a.jpg")), "JPG");
        assert_eq!(image_uri(Path::new("/tmp/a b.png")), "file:///tmp/a b.png");
    }
}
