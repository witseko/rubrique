//! Terminals in the app: open a tmux session in a tab (from the Terminals
//! list or a `.tmux` file), start, rename and find them in the file tree.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::new_entry::creation_error;
use super::{OpenFile, RubriqueApp};
use crate::explorer::tree::path_under_root;
use crate::explorer::view::{InlineRename, SessionEdit};
use crate::remote;
use crate::terminal::groups::{SessionGroups, group_terminals, seat_of};
use crate::terminal::is_terminal_path;
use crate::terminal::monitor::{TerminalRow, move_name};
use crate::terminal::session::LiveTerminal;
use crate::terminal::tmux::{file_for_session, new_session_name};
use crate::terminal::{TERMINAL_EXTENSION, TERMINAL_FILE_TEXT};

/// Which explorer sections were already looked at for folding at startup.
#[derive(Debug, Default)]
pub(super) struct StartupFolds {
    terminals: bool,
    openrig: bool,
}

impl RubriqueApp {
    /// Keep the Terminals list checking the remote rigs (none with `OpenRig`
    /// off), and fold empty sections at startup. Called every frame.
    pub(super) fn poll_terminal_sources(&mut self, ctx: &eframe::egui::Context) {
        if let Some(monitor) = &mut self.terminals {
            let hosts: &[String] = if self.settings.openrig_enabled { &self.settings.openrig_rigs } else { &[] };
            monitor.set_remote_hosts(hosts, ctx);
        }
        self.fold_empty_sections_at_startup();
    }

    /// At startup, fold Terminals when no plain tmux session runs and
    /// `OpenRig` when no seat runs, each once its sessions were checked
    /// (remote rigs included). Never unfolds a section.
    fn fold_empty_sections_at_startup(&mut self) {
        let Some(monitor) = &self.terminals else {
            return;
        };
        let openrig = self.settings.openrig_enabled;
        let is_seat = |name: &str| openrig && seat_of(name).is_some();
        let folds = &mut self.startup_folds;
        if folds.terminals && folds.openrig {
            return;
        }
        let rows = monitor.rows();
        if !folds.terminals && monitor.local_checked() {
            folds.terminals = true;
            if !rows.iter().any(|row| !is_seat(&row.info.name)) {
                self.settings.layout.terminals_collapsed = true;
            }
        }
        if !folds.openrig && monitor.all_checked() {
            folds.openrig = true;
            if !rows.iter().any(|row| is_seat(&row.info.name)) {
                self.settings.layout.openrig_collapsed = true;
            }
        }
    }

    /// A `.tmux` file as a tab: its session, started in the file's folder.
    /// `None` (with a status message) when it cannot be opened.
    pub(super) fn load_terminal(&mut self, path: &Path) -> Option<OpenFile> {
        if remote::is_mirror(path) {
            self.show_status("Terminals work in local folders only for now".to_string());
            return None;
        }
        let Some(monitor) = &self.terminals else {
            self.show_status("Install tmux to open terminals (macOS: brew install tmux)".to_string());
            return None;
        };
        match LiveTerminal::open_file(monitor.tmux().clone(), path) {
            Ok(terminal) => {
                monitor.refresh();
                Some(OpenFile::Terminal { terminal: Box::new(terminal) })
            }
            Err(error) => {
                self.show_status(format!("Cannot start the terminal: {error}"));
                None
            }
        }
    }

    /// Show session `name`: switch to its tab, or open one.
    pub(super) fn open_terminal_session(&mut self, name: &str) {
        let shows_session = |file: &OpenFile| matches!(file, OpenFile::Terminal { terminal } if terminal.name == name);
        if let Some(index) = (0..self.tabs.len()).find(|&index| shows_session(self.tab(index))) {
            self.switch_to_tab(index);
            return;
        }
        let Some(monitor) = &self.terminals else {
            return;
        };
        let folder = monitor
            .rows()
            .into_iter()
            .find(|row| row.info.name == name)
            .map_or_else(|| self.new_terminal_folder(), |row| row.info.folder);
        let terminal = LiveTerminal::new(monitor.tmux_for(name), name.to_string(), None, folder);
        self.add_tab(OpenFile::Terminal { terminal: Box::new(terminal) });
    }

    /// The + in the Terminals list: a name field for a new tmux session at
    /// the top of the list.
    pub(super) fn request_new_terminal(&mut self) {
        let Some(monitor) = &self.terminals else {
            return;
        };
        let sessions: Vec<_> = monitor.rows().into_iter().map(|row| row.info).collect();
        let name = new_session_name(&sessions);
        self.clear_name_fields();
        self.settings.layout.terminals_collapsed = false;
        self.session_edit = Some(InlineRename::with_name(SessionEdit::New, name));
    }

    /// Start session `name` in the first local folder and show it.
    fn start_named_session(&mut self, name: &str) -> Result<(), String> {
        let Some(monitor) = &self.terminals else {
            return Err("tmux is not installed".to_string());
        };
        if monitor.tmux().has_session(name) {
            return Err(format!("A terminal called \u{201c}{name}\u{201d} is already running"));
        }
        let folder = self.new_terminal_folder();
        monitor.tmux().start(name, &folder).map_err(|error| format!("Cannot start it: {error}"))?;
        monitor.refresh();
        self.open_terminal_session(name);
        Ok(())
    }

    /// "Exit terminal": end session `name` and close its tab, if any.
    pub(super) fn exit_terminal(&mut self, name: &str) {
        let Some(monitor) = &self.terminals else {
            return;
        };
        if let Err(error) = monitor.tmux_for(name).kill_session(name) {
            self.show_status(format!("Cannot exit the terminal: {error}"));
            return;
        }
        monitor.refresh();
        // A later session with the same name starts on top, like any new one.
        self.settings.terminal_order.retain(|ordered| ordered != name);
        let shows_session = |file: &OpenFile| matches!(file, OpenFile::Terminal { terminal } if terminal.name == name);
        for index in (0..self.tabs.len()).rev() {
            if index != self.active_tab && shows_session(&self.tabs[index]) {
                self.close_hidden_tab(index);
            }
        }
        if shows_session(&self.open_file) {
            self.close_active_tab_now();
        }
    }

    /// Fold or unfold a group of the Terminals list, by its key.
    pub(super) fn toggle_terminal_group(&mut self, key: String) {
        if let Some(index) = self.folded_terminal_groups.iter().position(|known| *known == key) {
            self.folded_terminal_groups.remove(index);
        } else {
            self.folded_terminal_groups.push(key);
        }
    }

    /// The groups of the Terminals list for `rows` (as drawn), by explorer
    /// folder then "Other", and of the `OpenRig` list, by rig. With `OpenRig`
    /// off, its seats are plain terminals.
    pub(super) fn terminal_groups(&self, rows: Option<&[TerminalRow]>) -> SessionGroups {
        let roots: Vec<(&str, &Path)> =
            self.roots.iter().map(|root| (root.name.as_str(), root.path.as_path())).collect();
        rows.map(|rows| group_terminals(rows, &roots, self.settings.openrig_enabled)).unwrap_or_default()
    }

    /// A terminal was dragged: save the list as it was drawn (`rows`), with
    /// session `name` moved before `anchor`, or after it when `after`.
    pub(super) fn move_terminal(&mut self, rows: Option<&[TerminalRow]>, name: &str, anchor: &str, after: bool) {
        let mut names: Vec<String> = rows.into_iter().flatten().map(|row| row.info.name.clone()).collect();
        move_name(&mut names, name, anchor, after);
        self.settings.terminal_order = names;
    }

    /// "Show in files": open the tree down to the session's `.tmux` file,
    /// or to the folder it runs in, and highlight it there.
    pub(super) fn show_terminal_in_files(&mut self, name: &str) {
        match self.terminal_place(name) {
            Some(place) => self.reveal_in_files(place),
            None => self.show_status("This terminal's folder is not in the explorer".to_string()),
        }
    }

    /// Where a session sits in the file tree: the `.tmux` file it was
    /// opened from, else the folder it runs in. `None` when that is outside
    /// every explorer folder, or the session is gone.
    fn terminal_place(&self, name: &str) -> Option<PathBuf> {
        // A tab knows its file even after the shell moved to another folder.
        let open_file = std::iter::once(&self.open_file).chain(&self.tabs).find_map(|file| match file {
            OpenFile::Terminal { terminal } if terminal.name == name => terminal.file.clone(),
            _ => None,
        });
        if open_file.is_some() {
            return open_file;
        }
        let row = self.terminals.as_ref()?.rows().into_iter().find(|row| row.info.name == name)?;
        let (root, folder) = self
            .roots
            .iter()
            .find_map(|root| path_under_root(&root.path, &row.info.folder).map(|folder| (&root.path, folder)))?;
        // The shell may have moved below the `.tmux` file's folder, so look
        // in each folder up to the explorer root.
        let file = folder
            .ancestors()
            .take_while(|ancestor| ancestor.starts_with(root))
            .find_map(|ancestor| file_for_session(ancestor, name));
        Some(file.unwrap_or(folder))
    }

    /// Unfold Files, open the folders down to `target`, and highlight and
    /// scroll to it on the next frame.
    pub(super) fn reveal_in_files(&mut self, target: PathBuf) {
        self.settings.layout.files_collapsed = false;
        if let Some(root) = self.roots.iter_mut().find(|root| target.starts_with(&root.path)) {
            root.reveal(&target);
        }
        self.revealed = Some(target);
        self.scroll_to_revealed = true;
    }

    /// Double click on a terminal in the list: edit its name in place. A
    /// session opened from a `.tmux` file is named after that file, so its
    /// file is renamed in the tree instead (renaming only the session would
    /// cut it off from its file).
    pub(super) fn start_terminal_rename(&mut self, name: String) {
        self.clear_name_fields();
        if let Some(file) = self.terminal_place(&name).filter(|place| is_terminal_path(place)) {
            self.reveal_in_files(file.clone());
            self.tree_rename = Some(InlineRename::new(file));
            return;
        }
        self.session_edit = Some(InlineRename::with_name(SessionEdit::Rename(name.clone()), name));
    }

    /// Start or rename the session whose name field was just submitted; on
    /// failure the field stays and shows why.
    pub(super) fn finish_session_edit(&mut self) {
        let Some(mut edit) = self.session_edit.take_if(|edit| edit.submitted) else {
            return;
        };
        let result = match &edit.target {
            SessionEdit::New => clean_terminal_name(&edit.name).and_then(|name| self.start_named_session(&name)),
            SessionEdit::Rename(from) => self.rename_session(from, &edit.name),
        };
        if let Err(error) = result {
            edit.failed(error);
            self.session_edit = Some(edit);
        }
    }

    /// Rename tmux session `from` to the `typed` name, and point its open
    /// tabs at the new name.
    fn rename_session(&mut self, from: &str, typed: &str) -> Result<(), String> {
        let to = clean_terminal_name(typed)?;
        if to == from {
            return Ok(());
        }
        let Some(monitor) = &self.terminals else {
            return Err("tmux is not installed".to_string());
        };
        if monitor.is_remote(from) {
            return Err("A remote rig's terminal keeps its OpenRig name".to_string());
        }
        if monitor.tmux().has_session(&to) {
            return Err(format!("A terminal called \u{201c}{to}\u{201d} is already running"));
        }
        monitor.tmux().rename(from, &to).map_err(|error| format!("Cannot rename it: {error}"))?;
        monitor.refresh();
        self.agent_history.rename_terminal(from, &to);
        self.save_agent_history();
        if let Some(expanded) = self.expanded_terminals.iter_mut().find(|name| *name == from) {
            expanded.clone_from(&to);
        }
        // Keep its place in the list the user dragged into order.
        if let Some(ordered) = self.settings.terminal_order.iter_mut().find(|name| *name == from) {
            ordered.clone_from(&to);
        }
        for file in std::iter::once(&mut self.open_file).chain(&mut self.tabs) {
            if let OpenFile::Terminal { terminal } = file
                && terminal.name == from
            {
                terminal.follow_session_rename(&to);
            }
        }
        Ok(())
    }

    /// Where a new terminal starts: the first local explorer folder, or home.
    fn new_terminal_folder(&self) -> PathBuf {
        self.roots
            .iter()
            .map(|root| root.path.clone())
            .find(|path| !remote::is_mirror(path))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/"))
    }
}

/// Create the `.tmux` file `typed` names in `folder` (with or without
/// the extension). Returns its path.
pub(super) fn create_terminal_file(folder: &Path, typed: &str) -> Result<PathBuf, String> {
    let name = clean_terminal_name(typed)?;
    write_terminal_file(folder, &name).map_err(|error| creation_error(&format!("{name}.{TERMINAL_EXTENSION}"), &error))
}

/// The typed name, trimmed and without a `.tmux` the user may have added.
/// Letters, digits, spaces, `-` and `_` only: the name becomes a tmux
/// session or a file name, and tmux reserves `.` and `:`.
fn clean_terminal_name(typed: &str) -> Result<String, String> {
    let name = typed.trim();
    let name = name.strip_suffix(&format!(".{TERMINAL_EXTENSION}")).unwrap_or(name).trim();
    if name.is_empty() {
        return Err("Type a name".to_string());
    }
    if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')) {
        return Err("Use letters, digits, spaces, - and _".to_string());
    }
    Ok(name.to_string())
}

/// Write `<name>.tmux` in `folder`. Never replaces a file: `create_new`
/// fails with `AlreadyExists` when the name is taken.
fn write_terminal_file(folder: &Path, name: &str) -> std::io::Result<PathBuf> {
    let path = folder.join(format!("{name}.{TERMINAL_EXTENSION}"));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&path)?;
    file.write_all(TERMINAL_FILE_TEXT.as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    #[test]
    fn never_replaces_a_terminal_file() {
        let temp = TempFolder::new();
        let path = write_terminal_file(temp.path(), "terminal").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TERMINAL_FILE_TEXT);
        let taken = write_terminal_file(temp.path(), "terminal").unwrap_err();
        assert_eq!(taken.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn cleans_typed_names() {
        assert_eq!(clean_terminal_name("  build "), Ok("build".to_string()));
        assert_eq!(clean_terminal_name("api server.tmux"), Ok("api server".to_string()));
        assert!(clean_terminal_name("   ").is_err());
        assert!(clean_terminal_name("a/b").is_err());
        assert!(clean_terminal_name("v1.2").is_err());
    }
}
