//! Load a read-only document, presentation, or workbook outside the UI thread.

mod converter;
mod ods;
mod odt;
mod package;
mod pdf;
mod pptx;
mod rtf;
mod table;
mod view;
mod word;
mod xls;
mod xlsx;
mod xml;
mod zip_archive;

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::thread;

use eframe::egui::{ColorImage, Context};

use crate::vfs::{self, Contents, Stamp};
use table::Table;

pub use table::{Delimited, DelimitedPreview};

/// Maximum file size read for a preview, including files fetched over SSH.
const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// Maximum number of bytes of text shown in a document or slide.
const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// Maximum declared size of all contents in an Office ZIP archive.
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

/// Formats that open as read-only documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pdf,
    Doc,
    Docx,
    Odt,
    Rtf,
    Pptx,
    Xls,
    Xlsx,
    Ods,
}

impl Kind {
    /// Identify supported file extensions without regard to case.
    pub fn of_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "doc" => Some(Self::Doc),
            "docx" => Some(Self::Docx),
            "odt" => Some(Self::Odt),
            "rtf" => Some(Self::Rtf),
            "pptx" => Some(Self::Pptx),
            "xls" => Some(Self::Xls),
            "xlsx" => Some(Self::Xlsx),
            "ods" => Some(Self::Ods),
            _ => None,
        }
    }
}

/// Rendered content of a document, or of the selected page, slide, or sheet.
#[derive(Debug)]
enum Content {
    Page { pixels: Option<ColorImage>, page_count: usize },
    Text(String),
    Slide { text: String, slide_count: usize },
    Sheet { names: Vec<String>, table: Table },
}

/// A loaded file snapshot and its displayed content.
#[derive(Debug)]
struct Loaded {
    bytes: Arc<Vec<u8>>,
    stamp: Option<Stamp>,
    content: Content,
}

/// Preview state owned by a tab.
pub struct FilePreview {
    kind: Kind,
    selected: usize,
    loaded: Option<Loaded>,
    error: Option<String>,
    pending: Option<mpsc::Receiver<Result<Loaded, String>>>,
    texture: Option<eframe::egui::TextureHandle>,
}

impl std::fmt::Debug for FilePreview {
    /// Show the tab state without including its file contents.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilePreview")
            .field("kind", &self.kind)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl FilePreview {
    /// Create a preview that starts reading when the tab is shown.
    pub fn new(kind: Kind) -> Self {
        Self { kind, selected: 0, loaded: None, error: None, pending: None, texture: None }
    }

    /// Reset the preview when the local source changes or disappears.
    pub fn check_disk(&mut self, path: &Path) {
        let Some(loaded) = &self.loaded else {
            return;
        };
        if !crate::remote::is_mirror(path)
            && let Ok(stamp) = vfs::current_stamp(path)
            && stamp != loaded.stamp
        {
            *self = Self::new(self.kind);
        }
    }

    /// Load the document or reuse its snapshot to navigate within it.
    fn start(&mut self, ctx: &Context, path: &Path) {
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let path = path.to_path_buf();
        let kind = self.kind;
        let selected = self.selected;
        let snapshot = self.loaded.as_ref().map(|loaded| (Arc::clone(&loaded.bytes), loaded.stamp.clone()));
        let worker = thread::Builder::new().name("file-preview".into()).spawn(move || {
            let result = snapshot.map_or_else(|| read(&path), Ok).and_then(|(bytes, stamp)| {
                let content = load(kind, &bytes, selected)?;
                Ok(Loaded { bytes, stamp, content })
            });
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        match worker {
            Ok(_) => self.pending = Some(receiver),
            Err(error) => self.error = Some(format!("Cannot start the preview: {error}")),
        }
    }

    /// Store a completed preview or report a load failure.
    fn poll(&mut self) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("The preview stopped before it finished.".into()),
        };
        self.pending = None;
        self.texture = None;
        match result {
            Ok(loaded) => self.loaded = Some(loaded),
            Err(error) => self.error = Some(error),
        }
    }
}

/// Read a bounded snapshot using the regular local or remote file entry point.
fn read(path: &Path) -> Result<(Arc<Vec<u8>>, Option<Stamp>), String> {
    if crate::remote::is_mirror(path) {
        return match vfs::read_file(path, MAX_BYTES).map_err(|error| error.to_string())? {
            (Contents::Bytes(bytes), stamp) => Ok((Arc::new(bytes), stamp)),
            (Contents::TooLarge(_), _) => Err(too_large()),
        };
    }
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a regular file to preview.".into());
    }
    if metadata.len() > MAX_BYTES {
        return Err(too_large());
    }
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(too_large());
    }
    Ok((Arc::new(bytes), metadata.modified().ok().map(Stamp::Local)))
}

/// Describe the file preview size limit.
fn too_large() -> String {
    format!("This file exceeds the {} preview limit.", crate::media::size_text(MAX_BYTES))
}

/// Load the content for the selected format and navigation position.
fn load(kind: Kind, bytes: &Arc<Vec<u8>>, selected: usize) -> Result<Content, String> {
    match kind {
        Kind::Pdf => pdf::page(bytes, selected),
        Kind::Doc => word::legacy_text(Arc::clone(bytes)).map(Content::Text),
        Kind::Docx => word::docx_text(bytes).map(Content::Text),
        Kind::Odt => odt::text(bytes).map(Content::Text),
        Kind::Rtf => rtf::text(bytes).map(Content::Text),
        Kind::Pptx => pptx::slide(bytes, selected),
        Kind::Xls | Kind::Xlsx | Kind::Ods => workbook(kind, bytes, selected),
    }
}

/// Open a workbook and read the selected sheet.
fn workbook(kind: Kind, bytes: &[u8], selected: usize) -> Result<Content, String> {
    let (names, table) = match kind {
        Kind::Xls => xls::sheet(bytes, selected)?,
        Kind::Xlsx => {
            check_archive(bytes)?;
            xlsx::sheet(bytes, selected)?
        }
        Kind::Ods => {
            check_archive(bytes)?;
            ods::sheet(bytes, selected)?
        }
        Kind::Pdf | Kind::Doc | Kind::Docx | Kind::Odt | Kind::Rtf | Kind::Pptx => {
            return Err("Choose a workbook to show its sheets.".into());
        }
    };
    Ok(Content::Sheet { names, table })
}

/// Check the declared expansion size before loading an Office archive.
fn check_archive(bytes: &[u8]) -> Result<(), String> {
    if zip_archive::ZipArchive::new(bytes)?.declared_size() > MAX_ARCHIVE_BYTES {
        return Err("This Office archive expands beyond the preview limit.".into());
    }
    Ok(())
}

/// Mark truncated output without splitting a Unicode character.
fn limit_text(mut text: String) -> String {
    if text.len() > MAX_TEXT_BYTES {
        text.truncate(text.floor_char_boundary(MAX_TEXT_BYTES));
        text.push_str("\n\nPreview ended at the text size limit.");
    }
    text
}

/// Open the source with the system default app, after resolving it to an absolute path.
fn open_source(path: &Path) -> Result<(), String> {
    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    if !path.is_file() {
        return Err("There is no local copy to open.".into());
    }
    crate::media::open_with_system(&path)
}
