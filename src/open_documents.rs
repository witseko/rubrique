//! Files and folders handed to the app by the system while it runs or
//! starts: on macOS, Finder's "Open" and "Open With", and files dropped on
//! the Dock icon.
//!
//! macOS sends these as an "open documents" Apple Event instead of command
//! line arguments. winit's application delegate does not answer it, so
//! without the handler here `AppKit` shows "Rubrique cannot open files in the
//! ... format". The handler queues the paths; the app takes them on its next
//! frame with [`take`]. On Linux the launcher passes files as arguments
//! (`Exec=rubrique %F`), so nothing is queued there.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use eframe::egui::Context;

#[cfg(target_os = "macos")]
mod macos;

/// Paths received and not yet opened by the app.
static QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// The app's context, to wake an idle window when paths arrive.
static WAKER: OnceLock<Context> = OnceLock::new();

/// Start listening for documents the system asks the app to open.
/// Call in `main` before the event loop starts, so the paths of a launch
/// from Finder are caught too.
pub fn listen() {
    #[cfg(target_os = "macos")]
    macos::listen();
}

/// Wake `ctx` whenever paths arrive (the window may be idle).
pub fn set_waker(ctx: Context) {
    let _ = WAKER.set(ctx);
}

/// Take the paths received since the last call, oldest first.
pub fn take() -> Vec<PathBuf> {
    std::mem::take(&mut *QUEUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

/// Queue `paths` for the app and wake it.
#[cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(dead_code, reason = "called by the macOS handler and the tests only")
)]
fn push(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    QUEUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(paths);
    if let Some(ctx) = WAKER.get() {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_returns_pushed_paths_once() {
        push(vec![PathBuf::from("/a.md"), PathBuf::from("/b.txt")]);
        assert_eq!(take(), vec![PathBuf::from("/a.md"), PathBuf::from("/b.txt")]);
        assert_eq!(take(), Vec::<PathBuf>::new());
    }
}
