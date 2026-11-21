//! `@tmux-` mentions in the app: which terminals a mention can name (for
//! the suggestions while typing), the Tasks view of the side panel, and
//! sending a step to its terminal. The message is pasted only when an
//! agent waits at its prompt there: pasted into a shell, its lines would
//! run as commands. Until then it waits, and the terminal opens so an agent
//! can be started in it.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText, Ui};

use super::{OpenFile, RubriqueApp};
use crate::ai::mention_list::Suggestion;
use crate::explorer::tree::{Children, FsNode};
use crate::icons::{Icon, icon_text_button};
use crate::terminal::is_terminal_path;
use crate::terminal::mentions::{Delegation, MENTION_PREFIX, find_delegations, handle_for, mark_sent, message};
use crate::terminal::monitor::{TerminalMonitor, TerminalRow, opened_label, unix_now};
use crate::terminal::status::TerminalStatus;
use crate::terminal::tmux::session_name_for_file;

/// Size of the small text on a task card.
const SMALL_TEXT: f32 = 11.5;
/// How strongly a task card is tinted with the link color.
const CARD_TINT: f32 = 0.12;

/// A terminal a mention can name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TerminalTarget {
    /// What follows `@tmux-`.
    handle: String,
    /// The tmux session it runs in (or will run in).
    session: String,
    /// The `.tmux` file that opens it, when it has one.
    file: Option<PathBuf>,
}

/// A message waiting for an agent to be ready in its terminal.
#[derive(Debug)]
pub(super) struct PendingSend {
    session: String,
    handle: String,
    text: String,
    /// The document the mention is in, and its instruction: once sent,
    /// that mention gets its "sent" mark.
    document: PathBuf,
    instruction: String,
}

impl RubriqueApp {
    /// What typing `@` in a document can suggest: the agents, then the
    /// terminals.
    pub(super) fn mention_suggestions(&self) -> Vec<Suggestion> {
        let agents = self.agents.iter().map(Suggestion::agent);
        let terminals = self.terminal_targets().into_iter().map(|target| Suggestion {
            mention: format!("@{MENTION_PREFIX}{}", target.handle),
            title: "Terminal".to_string(),
            detail: match &target.file {
                Some(file) => file.display().to_string(),
                None => format!("tmux session {}", target.session),
            },
        });
        agents.chain(terminals).collect()
    }

    /// The terminals a mention can name: `.tmux` files next to the open
    /// file and in the folders loaded in the explorer, then the running
    /// sessions that have no such file. The first one with a handle wins.
    fn terminal_targets(&self) -> Vec<TerminalTarget> {
        if self.terminals.is_none() {
            return Vec::new();
        }
        let mut files = Vec::new();
        if let Some(folder) = self.open_file.path().and_then(Path::parent) {
            files.extend(terminal_files_in(folder));
        }
        for root in &self.roots {
            collect_loaded_terminal_files(root, &mut files);
        }
        let mut targets: Vec<TerminalTarget> = Vec::new();
        let mut add = |target: TerminalTarget| {
            if !targets.iter().any(|known| known.handle == target.handle) {
                targets.push(target);
            }
        };
        for file in files {
            let handle = handle_for(&file.file_stem().map(|stem| stem.to_string_lossy()).unwrap_or_default());
            add(TerminalTarget { handle, session: session_name_for_file(&file), file: Some(file) });
        }
        for row in self.terminal_rows() {
            add(TerminalTarget { handle: handle_for(&row.info.name), session: row.info.name, file: None });
        }
        targets
    }

    /// The running sessions; empty without tmux.
    fn terminal_rows(&self) -> Vec<TerminalRow> {
        self.terminals.as_ref().map(TerminalMonitor::rows).unwrap_or_default()
    }

    /// The Tasks view of the side panel: every `@tmux-` mention in the open
    /// file, with Send.
    pub(super) fn tasks_panel(&mut self, ui: &mut Ui) {
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        let delegations = find_delegations(&document.text);
        let targets = self.terminal_targets();
        let rows = self.terminal_rows();
        let waiting: Vec<bool> = delegations
            .iter()
            .map(|delegation| {
                self.pending_sends.iter().any(|pending| {
                    pending.document == document.path
                        && pending.handle == delegation.handle
                        && pending.instruction == delegation.instruction
                })
            })
            .collect();
        let mut send = None;
        egui::Frame::new().inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if self.settings.openrig_enabled {
                    self.openrig_tasks(ui);
                    ui.separator();
                }
                if delegations.is_empty() {
                    let hint = "Hand a step to a terminal: write @tmux-<terminal>: what to do, in that step. \
                                Send pastes it there with the step as context.";
                    ui.label(RichText::new(hint).weak().size(SMALL_TEXT));
                }
                for (index, delegation) in delegations.iter().enumerate() {
                    let target = targets.iter().find(|target| target.handle == delegation.handle);
                    if task_card(ui, delegation, target, &rows, waiting[index]) {
                        send = Some(index);
                    }
                    ui.add_space(8.0);
                }
            });
        });
        if let Some(index) = send {
            self.send_delegation(index);
        }
    }

    /// Send mention number `index` of the open file to its terminal. When
    /// no agent waits at its prompt there yet, the message waits and the
    /// terminal opens, so an agent can be started in it.
    fn send_delegation(&mut self, index: usize) {
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        let Some(delegation) = find_delegations(&document.text).into_iter().nth(index) else {
            return;
        };
        let Some(target) = self.terminal_targets().into_iter().find(|target| target.handle == delegation.handle) else {
            let handle = &delegation.handle;
            self.show_status(format!("No terminal called {handle}: add {handle}.tmux next to this file"));
            return;
        };
        let pending = PendingSend {
            session: target.session.clone(),
            handle: target.handle.clone(),
            text: message(&delegation, &document.path),
            document: document.path.clone(),
            instruction: delegation.instruction,
        };
        if let Err(error) = self.start_target(&target) {
            self.show_status(format!("Cannot start {}: {error}", target.handle));
            return;
        }
        let ready = self.terminal_rows().iter().any(|row| row.info.name == target.session && is_ready(&row.status));
        if ready {
            self.deliver(&pending);
            return;
        }
        self.show_status(format!(
            "{} gets this once an agent waits at its prompt there: start claude or codex in it",
            target.handle
        ));
        self.pending_sends.push(pending);
        match &target.file {
            Some(file) => self.open_in_tab(file.clone()),
            None => self.open_terminal_session(&target.session),
        }
    }

    /// Paste a message into its terminal and mark its mention sent.
    fn deliver(&mut self, pending: &PendingSend) {
        let pasted = match &self.terminals {
            Some(monitor) => monitor.tmux_for(&pending.session).paste(&pending.session, &pending.text),
            None => return,
        };
        match pasted {
            Ok(()) => {
                self.mark_delivered(pending);
                self.show_status(format!("Sent to {} (save the file to keep the sent mark)", pending.handle));
            }
            Err(error) => self.show_status(format!("Cannot send to {}: {error}", pending.handle)),
        }
    }

    /// Give the delivered mention its "sent" mark, in whichever tab its
    /// document is open.
    fn mark_delivered(&mut self, pending: &PendingSend) {
        let when = opened_label(unix_now());
        for file in self.all_tabs_mut() {
            let OpenFile::Text { document, preview, .. } = file else {
                continue;
            };
            if document.path != pending.document {
                continue;
            }
            let found = find_delegations(&document.text).into_iter().find(|delegation| {
                !delegation.sent && delegation.handle == pending.handle && delegation.instruction == pending.instruction
            });
            if let Some(delegation) = found {
                document.text = mark_sent(&document.text, &delegation, &when);
                document.on_text_changed();
                *preview = None;
            }
            return;
        }
    }

    /// Make sure the target's session runs: a `.tmux` file's session
    /// starts in the file's folder.
    fn start_target(&self, target: &TerminalTarget) -> std::io::Result<()> {
        let (Some(monitor), Some(file)) = (&self.terminals, &target.file) else {
            return Ok(());
        };
        let folder = file.parent().map_or_else(|| PathBuf::from("/"), Path::to_path_buf);
        monitor.tmux().start(&target.session, &folder)?;
        monitor.refresh();
        Ok(())
    }

    /// Send the messages whose terminal now has an agent waiting at its
    /// prompt. Called every frame.
    pub(super) fn poll_pending_sends(&mut self) {
        if self.pending_sends.is_empty() {
            return;
        }
        let rows = self.terminal_rows();
        let ready = |session: &str| rows.iter().any(|row| row.info.name == session && is_ready(&row.status));
        let (ready, waiting): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.pending_sends).into_iter().partition(|pending| ready(&pending.session));
        self.pending_sends = waiting;
        for pending in ready {
            self.deliver(&pending);
        }
    }
}

/// An agent waiting at its prompt: the only safe moment to paste. A shell
/// would run the lines as commands, a working agent would get them mid-task,
/// and one asking a question would take them as its answer.
fn is_ready(status: &TerminalStatus) -> bool {
    matches!(status, TerminalStatus::Idle(_))
}

/// One task: the terminal it goes to (with its state), the instruction,
/// the step, and Send (or that it waits for an agent). Returns true when
/// Send was clicked.
fn task_card(
    ui: &mut Ui,
    delegation: &Delegation,
    target: Option<&TerminalTarget>,
    rows: &[TerminalRow],
    waiting: bool,
) -> bool {
    let tint = ui.visuals().hyperlink_color.gamma_multiply(CARD_TINT);
    let mut clicked = false;
    egui::Frame::new().fill(tint).corner_radius(8).inner_margin(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let state = match target {
            None => "no such terminal".to_string(),
            Some(target) => match rows.iter().find(|row| row.info.name == target.session) {
                Some(row) => row.status.label(),
                None => "not running".to_string(),
            },
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("@{MENTION_PREFIX}{}", delegation.handle)).monospace().strong());
            ui.label(RichText::new(state).weak().size(SMALL_TEXT));
        });
        if !delegation.instruction.is_empty() {
            ui.label(&delegation.instruction);
        }
        if !delegation.context.is_empty() {
            ui.label(RichText::new(&delegation.context).weak().size(SMALL_TEXT));
        }
        ui.horizontal(|ui| {
            let (label, tip) = if delegation.sent {
                ("Send again", "Already sent once; paste it into the terminal again")
            } else {
                ("Send", "Paste this and its step into the terminal's agent")
            };
            if waiting {
                let note = "Waiting for an agent at its prompt (start claude or codex there)";
                ui.add(egui::Spinner::new().size(SMALL_TEXT));
                ui.label(RichText::new(note).weak().size(SMALL_TEXT));
                return;
            }
            ui.add_enabled_ui(target.is_some(), |ui| {
                clicked = icon_text_button(ui, Icon::Terminal, label, tip, false).clicked();
            });
            if delegation.sent {
                ui.label(RichText::new("Sent").weak().size(SMALL_TEXT));
            }
        });
    });
    clicked
}

/// The `.tmux` files directly in `folder`.
fn terminal_files_in(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> =
        entries.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| is_terminal_path(path)).collect();
    files.sort();
    files
}

/// Add the `.tmux` files among the entries already read under `node`
/// (folders never opened are not read just for this).
fn collect_loaded_terminal_files(node: &FsNode, files: &mut Vec<PathBuf>) {
    let Children::Loaded(children) = &node.children else {
        return;
    };
    for child in children {
        if child.is_folder() {
            collect_loaded_terminal_files(child, files);
        } else if is_terminal_path(&child.path) {
            files.push(child.path.clone());
        }
    }
}
