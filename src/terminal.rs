//! Terminals: tmux sessions shown as tabs and listed in the explorer.
//!
//! tmux does the real terminal work (the shell, the screen, colors, vim and
//! agents). Rubrique mirrors the screen of a session and types into it, so
//! closing a tab or the app leaves the session and its programs running.
//! A `.tmux` file in a folder is a terminal too: opening it starts (or
//! reattaches to) a session in that folder.

pub mod agent_sessions;
pub mod ansi;
pub mod groups;
pub mod keys;
pub mod mentions;
pub mod monitor;
pub mod screen;
pub mod session;
pub mod status;
pub mod tmux;
pub mod view;

use std::path::Path;

/// The extension of terminal files.
pub const TERMINAL_EXTENSION: &str = "tmux";

/// What a new `.tmux` file holds, so it explains itself when opened elsewhere.
pub const TERMINAL_FILE_TEXT: &str =
    "# A Rubrique terminal: opening this file in Rubrique starts a tmux session in this folder.\n";

/// True for `.tmux` files, which open as terminals.
pub fn is_terminal_path(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(TERMINAL_EXTENSION))
}
