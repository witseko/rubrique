//! The HTML preview drawn by a Chromium browser (Settings → Files): which
//! browser, the snapshot that runs in the background, and the picture.
//!
//! A new snapshot starts when the file's text or the pane's size changes,
//! after a short pause so typing or resizing does not start one per frame.
//! The last picture stays on screen until the new one is ready. When the
//! browser fails, the built-in preview shows that text, with the reason.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, Context, RichText, TextureHandle, TextureOptions, Ui, vec2};

use super::{RubriqueApp, write_browser_copy};
use crate::chromium::{self, Snapshot, SnapshotRequest};
use crate::document::text_file::Document;
use crate::document::view::{PreviewEvent, show_read_only_preview};
use crate::install::{InstallStatus, Platform, Recipe, chromium_recipe, find_installer};
use crate::markdown::blocks::SourceBlock;
use crate::markdown::html::{file_url, with_base};
use crate::markdown::html_blocks::html_source_blocks;
use crate::settings::view::HtmlPreviewInfo;

/// Pause after the last change before a new snapshot starts.
const SETTLE: Duration = Duration::from_millis(300);

/// Largest picture height in device pixels, whatever the GPU allows: a tall
/// page at 2x is already about 100 MB of pixels at this height.
const MAX_PICTURE_PIXELS: usize = 16_384;

/// The page, its text and the pane size a snapshot was made for.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotKey {
    path: PathBuf,
    text_hash: u64,
    width: u32,
    height: u32,
    /// Pixels per point, in thousandths.
    scale: u32,
}

/// A finished snapshot, decoded on the snapshot's thread.
type SnapshotResult = Result<(ColorImage, bool), String>;

/// The picture on screen.
struct Shown {
    key: SnapshotKey,
    texture: TextureHandle,
    cut: bool,
}

/// Chromium HTML preview state.
#[derive(Default)]
pub(super) struct HtmlSnapshots {
    /// The browser that draws, when one was found.
    browser: Option<PathBuf>,
    /// How to install one, when none was found.
    recipe: Option<Recipe>,
    shown: Option<Shown>,
    running: Option<(SnapshotKey, Receiver<SnapshotResult>)>,
    /// The page and text that failed, and why; the built-in preview shows
    /// them until the text changes.
    failed: Option<(PathBuf, u64, String)>,
    /// The page wanted next, and since when.
    wanted: Option<(SnapshotKey, Instant)>,
    /// A message for the status bar, taken by [`RubriqueApp::poll_html_snapshots`].
    status: Option<String>,
}

impl std::fmt::Debug for HtmlSnapshots {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HtmlSnapshots")
            .field("browser", &self.browser)
            .field("running", &self.running.as_ref().map(|(key, _)| key))
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

/// A hash of the page's text, to see when it changed.
fn text_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl RubriqueApp {
    /// Look for a browser to draw HTML with, and its recipe while none is found.
    pub(super) fn check_html_browser(&mut self) {
        let snapshots = &mut self.html_snapshots;
        snapshots.browser = chromium::find_browser();
        snapshots.recipe = snapshots
            .browser
            .is_none()
            .then(chromium::install_folder)
            .flatten()
            .map(|folder| chromium_recipe(&folder, Platform::current(), &find_installer));
    }

    /// Each frame: say in the status bar why a snapshot failed.
    pub(super) fn poll_html_snapshots(&mut self) {
        if let Some(message) = self.html_snapshots.status.take() {
            self.show_status(message);
        }
    }
}

/// The Files section's view of the Chromium preview: the setting, the
/// browser and its recipe.
pub(super) fn html_preview_info<'a>(
    enabled: &'a mut bool,
    snapshots: &'a HtmlSnapshots,
    install: &'a InstallStatus,
) -> HtmlPreviewInfo<'a> {
    HtmlPreviewInfo { enabled, found: snapshots.browser.as_deref(), recipe: snapshots.recipe.as_ref(), install }
}

impl HtmlSnapshots {
    /// The preview of an HTML file: the browser's picture when `use_browser`
    /// (the setting), a browser was found, and this text did not fail;
    /// otherwise the built-in preview from `blocks`. Encrypted files never go
    /// to the browser: it needs a plain copy on disk.
    pub(super) fn show_preview(
        &mut self,
        ui: &mut Ui,
        document: &Document,
        blocks: &mut Option<Vec<SourceBlock>>,
        use_browser: bool,
    ) -> Option<PreviewEvent> {
        if use_browser && self.browser.is_some() && !document.is_encrypted() {
            let hash = text_hash(&document.text);
            if self.failure(&document.path, hash).is_none() {
                self.show_snapshot(ui, document, hash);
                return None;
            }
        }
        if use_browser && let Some(message) = self.failure(&document.path, text_hash(&document.text)) {
            let text = format!("Chromium could not draw this page ({message}); the built-in preview:");
            ui.label(RichText::new(text).color(ui.visuals().warn_fg_color));
        }
        let blocks = blocks.get_or_insert_with(|| html_source_blocks(&document.text));
        show_read_only_preview(ui, &document.path, blocks)
    }

    /// Why the browser could not draw this text of `path`, if it failed.
    fn failure(&self, path: &Path, hash: u64) -> Option<&str> {
        self.failed
            .as_ref()
            .filter(|(failed_path, failed_hash, _)| failed_path == path && *failed_hash == hash)
            .map(|(_, _, message)| message.as_str())
    }

    /// The browser's picture of `document`, starting a new snapshot when
    /// its text or the pane changed.
    fn show_snapshot(&mut self, ui: &mut Ui, document: &Document, hash: u64) {
        let ctx = ui.ctx().clone();
        let size = ui.available_size();
        let scale = ctx.pixels_per_point();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "pane sizes are small and positive"
        )]
        let key = SnapshotKey {
            path: document.path.clone(),
            text_hash: hash,
            width: size.x.max(1.0).round() as u32,
            height: size.y.max(1.0).round() as u32,
            scale: (scale * 1000.0).round() as u32,
        };
        self.finish_snapshot(&ctx);
        self.want_snapshot(&ctx, key, &document.text);
        let shown = self.shown.as_ref().filter(|shown| shown.key.path == document.path);
        match shown {
            Some(shown) => show_picture(ui, shown, scale),
            None if self.running.is_some() || self.wanted.is_some() => {
                ui.centered_and_justified(|ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new());
                        ui.label(RichText::new("Drawing the page with Chromium…").weak());
                    });
                });
            }
            None => {}
        }
    }

    /// Take the finished snapshot, if there is one.
    fn finish_snapshot(&mut self, ctx: &Context) {
        let Some((key, receiver)) = &self.running else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the snapshot stopped".to_string()),
        };
        let key = key.clone();
        self.running = None;
        match result {
            Ok((image, cut)) => {
                let texture = ctx.load_texture("html-snapshot", image, TextureOptions::LINEAR);
                self.shown = Some(Shown { key, texture, cut });
            }
            Err(message) => {
                self.status = Some(format!("Chromium preview failed: {message}"));
                self.failed = Some((key.path, key.text_hash, message));
            }
        }
    }

    /// Start a snapshot of `text` for `key` once nothing runs and the change settled.
    fn want_snapshot(&mut self, ctx: &Context, key: SnapshotKey, text: &str) {
        let up_to_date = self.shown.as_ref().is_some_and(|shown| shown.key == key);
        let in_flight = self.running.as_ref().is_some_and(|(running, _)| *running == key);
        if up_to_date || in_flight {
            self.wanted = None;
            return;
        }
        let first_for_page = self.shown.as_ref().is_none_or(|shown| shown.key.path != key.path);
        let since = match &self.wanted {
            Some((wanted, since)) if *wanted == key => *since,
            // Opening a page draws it at once; edits and resizes settle first.
            _ if first_for_page => Instant::now().checked_sub(SETTLE).unwrap_or_else(Instant::now),
            _ => Instant::now(),
        };
        self.wanted = Some((key.clone(), since));
        if self.running.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let waited = since.elapsed();
        if waited < SETTLE {
            ctx.request_repaint_after(SETTLE.saturating_sub(waited));
            return;
        }
        self.wanted = None;
        self.start_snapshot(ctx, key, text);
    }

    /// Write a copy of the page (so unsaved edits show and relative links
    /// still work) and draw it on a thread.
    fn start_snapshot(&mut self, ctx: &Context, key: SnapshotKey, text: &str) {
        let Some(browser) = self.browser.clone() else {
            return;
        };
        let folder = key.path.parent().map(|folder| format!("{}/", file_url(folder))).unwrap_or_default();
        let copy = match write_browser_copy(&key.path, &with_base(text, &folder)) {
            Ok(copy) => copy,
            Err(error) => {
                let message = format!("cannot write the page copy: {error}");
                self.failed = Some((key.path, key.text_hash, message));
                return;
            }
        };
        let max_side = ctx.input(|input| input.max_texture_side).min(MAX_PICTURE_PIXELS);
        let request = SnapshotRequest {
            browser,
            page_url: file_url(&copy),
            width: key.width,
            height: key.height,
            #[expect(clippy::cast_precision_loss, reason = "thousandths of a small scale")]
            scale: key.scale as f32 / 1000.0,
            max_pixels: u32::try_from(max_side).unwrap_or(u32::MAX),
        };
        let (sender, receiver) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = chromium::take_snapshot(&request).and_then(|snapshot| decode(&snapshot));
            let _ = sender.send(result);
            wake.request_repaint();
        });
        self.running = Some((key, receiver));
    }
}

/// The snapshot's PNG as pixels for a texture.
fn decode(snapshot: &Snapshot) -> SnapshotResult {
    let picture = image::load_from_memory_with_format(&snapshot.png, image::ImageFormat::Png)
        .map_err(|error| format!("cannot read the browser's picture: {error}"))?
        .to_rgba8();
    let size = [picture.width() as usize, picture.height() as usize];
    Ok((ColorImage::from_rgba_unmultiplied(size, picture.as_raw()), snapshot.cut))
}

/// The picture at the size it was drawn for, scrolling down the page.
fn show_picture(ui: &mut Ui, shown: &Shown, scale: f32) {
    let [width, height] = shown.texture.size();
    #[expect(clippy::cast_precision_loss, reason = "texture sizes are far below f32 precision limits")]
    let size = vec2(width as f32, height as f32) / scale;
    egui::ScrollArea::both().id_salt(("html-snapshot", &shown.key.path)).auto_shrink([false, false]).show(ui, |ui| {
        ui.add(egui::Image::new((shown.texture.id(), size)));
        if shown.cut {
            ui.label(
                RichText::new("The page goes on: open it in the browser (Cmd+Shift+B) to see the rest.")
                    .color(ui.visuals().warn_fg_color),
            );
        }
    });
}
