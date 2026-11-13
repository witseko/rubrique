//! Image and video files: which files they are, and how to name their size.
//!
//! Pure apart from [`open_with_system`], which starts the system's "open"
//! program.

use std::path::Path;

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
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(program)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|error| format!("Cannot start {program}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
