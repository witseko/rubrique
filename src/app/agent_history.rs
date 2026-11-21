//! The agent sessions each terminal has run: noticed every few seconds
//! while an agent runs, kept in the `agent-sessions` file, and resumed from
//! the Terminals list in the same terminal or a new one.

use std::path::PathBuf;

use eframe::egui::Context;

use super::RubriqueApp;
use crate::settings::agent_sessions_file;
use crate::terminal::agent_sessions::{AgentSession, SessionHistory, current_session};
use crate::terminal::keys::KeyInput;
use crate::terminal::status::TerminalStatus;
use crate::terminal::tmux::new_session_name;

/// How often the terminals running an agent are checked, in seconds
/// (finding a session reads a few files).
const CHECK_SECONDS: f64 = 10.0;

/// Read the remembered agent sessions; none when there is no file yet.
pub(super) fn load_history() -> SessionHistory {
    agent_sessions_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| SessionHistory::parse(&text))
        .unwrap_or_default()
}

impl RubriqueApp {
    /// Every few seconds, note the conversation each agent terminal has
    /// open, and save the history when something new turned up.
    pub(super) fn poll_agent_sessions(&mut self, ctx: &Context) {
        let now = ctx.input(|input| input.time);
        if now - self.last_agent_check < CHECK_SECONDS {
            return;
        }
        self.last_agent_check = now;
        let (Some(monitor), Some(home)) = (&self.terminals, std::env::var_os("HOME").map(PathBuf::from)) else {
            return;
        };
        let mut changed = false;
        // A remote seat's conversations are on its host, not in this home folder.
        for row in monitor.rows().into_iter().filter(|row| !monitor.is_remote(&row.info.name)) {
            let Some(agent) = row.status.agent() else {
                continue;
            };
            if let Some(session) = current_session(&home, agent, &row.info.folder, row.info.last_output, &row.info.name)
            {
                changed |= self.agent_history.record(session);
            }
        }
        if changed {
            self.save_agent_history();
        }
    }

    /// Write the history file. A failure only goes to the status bar: the
    /// sessions still show until the app closes.
    pub(super) fn save_agent_history(&mut self) {
        let Some(path) = agent_sessions_file() else {
            return;
        };
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, self.agent_history.to_text()));
        if let Err(error) = written {
            self.show_status(format!("Cannot save the agent sessions: {error}"));
        }
    }

    /// Show or hide the agent sessions under a terminal in the list.
    pub(super) fn toggle_agent_sessions(&mut self, terminal: String) {
        if let Some(index) = self.expanded_terminals.iter().position(|known| *known == terminal) {
            self.expanded_terminals.remove(index);
        } else {
            self.expanded_terminals.push(terminal);
        }
    }

    /// Resume conversation `id` of `terminal`: in that terminal when it
    /// sits at a shell prompt (or is not running), otherwise, or when
    /// asked, in a new terminal in the conversation's folder.
    pub(super) fn resume_agent_session(&mut self, terminal: &str, id: &str, new_terminal: bool) {
        let Some(session) = self.agent_history.of(terminal).into_iter().find(|session| session.id == id).cloned()
        else {
            return;
        };
        let Some(monitor) = &self.terminals else {
            return;
        };
        let rows = monitor.rows();
        let status = rows.iter().find(|row| row.info.name == terminal).map(|row| row.status.clone());
        let fits_here = !new_terminal && matches!(status, None | Some(TerminalStatus::Shell));
        let target = if fits_here {
            terminal.to_string()
        } else {
            new_session_name(&rows.into_iter().map(|row| row.info).collect::<Vec<_>>())
        };
        match start_and_type(&monitor.tmux_for(&target), &target, &session) {
            Ok(()) => {
                monitor.refresh();
                if !new_terminal && !fits_here {
                    self.show_status(format!("{terminal} is busy, so it opened in {target}"));
                }
                self.open_terminal_session(&target);
            }
            Err(error) => self.show_status(format!("Cannot resume it: {error}")),
        }
    }
}

/// Start `target` in the session's folder (when it is not running) and
/// type the command that resumes the session.
fn start_and_type(tmux: &crate::terminal::tmux::Tmux, target: &str, session: &AgentSession) -> std::io::Result<()> {
    tmux.start(target, &session.folder)?;
    tmux.send(target, &[KeyInput::Text(session.resume_command()), KeyInput::Key("Enter".to_string())])
}
