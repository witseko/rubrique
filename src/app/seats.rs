//! The menu of an `OpenRig` seat in the Terminals list: send it a step,
//! copy its address or attach command, attach in the terminal app, change
//! its setup, or stop it.

use std::path::Path;
use std::process::{Command, Stdio};

use eframe::egui::Context;

use super::RubriqueApp;
use crate::alert::{self, ButtonKind};
use crate::explorer::view::SeatCommand;
use crate::openrig::RigHost;

/// Width of the "Stop seat" dialog.
const DIALOG_WIDTH: f32 = 380.0;

/// Where a seat's session runs and how tmux knows it there.
pub(super) struct SeatPlace {
    /// The SSH host; `None` for this computer.
    pub(super) host: Option<String>,
    /// The tmux program on that machine.
    program: std::path::PathBuf,
    /// The session name in tmux, which is also the `OpenRig` address (`seat@rig`).
    pub(super) address: String,
}

impl RubriqueApp {
    /// Carry out an item of seat `name`'s menu.
    pub(super) fn seat_command(&mut self, ctx: &Context, name: String, command: SeatCommand) {
        let Some(place) = self.seat_place(&name) else {
            return;
        };
        match command {
            SeatCommand::SendStep => {
                let rig = place.host.map_or(RigHost::Local, RigHost::Ssh);
                if let Err(error) = self.send_step_to_seat(ctx, rig, place.address) {
                    self.show_status(error);
                }
            }
            SeatCommand::CopyAddress => self.copy_to_clipboard(ctx, place.address),
            SeatCommand::CopyAttach => {
                let command = attach_command(place.host.as_deref(), &place.program, &place.address);
                self.copy_to_clipboard(ctx, command);
            }
            SeatCommand::OpenInTerminalApp => {
                let command = attach_command(place.host.as_deref(), &place.program, &place.address);
                match open_in_terminal_app(&command) {
                    Ok(()) => self.show_status(format!("Opened {} in the terminal app", place.address)),
                    Err(error) => self.show_status(format!("Cannot open the terminal app: {error}")),
                }
            }
            SeatCommand::Setup => self.start_seat_setup(ctx, place),
            SeatCommand::Stop => self.stopping_seat = Some(name),
        }
    }

    /// Where seat `name` runs, from the Terminals list's tmux.
    pub(super) fn seat_place(&self, name: &str) -> Option<SeatPlace> {
        let tmux = self.terminals.as_ref()?.tmux_for(name);
        Some(SeatPlace {
            host: tmux.host().map(str::to_owned),
            program: tmux.program().to_path_buf(),
            address: tmux.tmux_name(name).to_owned(),
        })
    }

    /// Draw the "Stop seat" dialog while it is open.
    pub(super) fn stop_seat_dialog(&mut self, ctx: &Context) {
        let Some(name) = self.stopping_seat.as_deref() else {
            return;
        };
        let Some(place) = self.seat_place(name) else {
            self.stopping_seat = None;
            return;
        };
        let explanation = match &place.host {
            Some(host) => format!("Its tmux session on {host} ends, with the agent and anything else running in it."),
            None => "Its tmux session ends, with the agent and anything else running in it.".to_string(),
        };
        let modal = alert::show(ctx, "stop-seat", DIALOG_WIDTH, |ui| {
            alert::title(ui, &format!("Stop seat \u{201c}{}\u{201d}?", place.address));
            alert::body(ui, &explanation);
            // Enter takes Cancel, never the stop.
            let ((), (confirm, cancel)) = alert::button_row(
                ui,
                |_| (),
                |ui| {
                    let confirm = alert::button(ui, "Stop seat", ButtonKind::Delete, false);
                    let cancel = alert::button(ui, "Cancel", ButtonKind::Secondary, true);
                    (confirm, cancel)
                },
            );
            let enter = alert::enter_for_default(ui);
            (confirm.clicked(), cancel.clicked() || enter)
        });
        let (confirmed, cancelled) = modal.inner;
        if cancelled || modal.should_close() {
            self.stopping_seat = None;
        } else if confirmed && let Some(name) = self.stopping_seat.take() {
            self.exit_terminal(&name);
        }
    }
}

/// The shell command that attaches to `session`: plain `tmux attach` here,
/// or through `ssh -t` with the host's tmux path (a command over ssh does
/// not read the login profile that puts tmux on PATH).
fn attach_command(host: Option<&str>, program: &Path, session: &str) -> String {
    let target = shell_word(session);
    match host {
        None => format!("tmux attach -t {target}"),
        Some(host) => {
            let program = shell_word(&program.to_string_lossy());
            format!("ssh -t {} {program} attach -t {target}", shell_word(host))
        }
    }
}

/// `text` as one shell word: as it is when it holds only safe characters,
/// otherwise in single quotes.
fn shell_word(text: &str) -> String {
    let safe = !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || "@%+=:,./_-".contains(c));
    if safe {
        text.to_string()
    } else {
        String::from_utf8_lossy(&crate::remote::connection::quote(text.as_bytes())).into_owned()
    }
}

/// Run `command` in a new window of the system's terminal app: Terminal on
/// macOS, the default terminal emulator on Linux.
/// On macOS it waits for `osascript`, which returns once Terminal has the
/// window, so a refusal (such as no permission to control Terminal) is
/// reported instead of lost.
fn open_in_terminal_app(command: &str) -> std::io::Result<()> {
    if !cfg!(target_os = "macos") {
        let mut process = Command::new("x-terminal-emulator");
        process.args(["-e", "sh", "-c", command]);
        return process.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ());
    }
    let script =
        format!("tell application \"Terminal\"\n  do script \"{}\"\n  activate\nend tell", applescript_string(command));
    let output = Command::new("osascript").args(["-e", &script]).stdin(Stdio::null()).output()?;
    if output.status.success() {
        return Ok(());
    }
    let errors = String::from_utf8_lossy(&output.stderr);
    let message = errors.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("osascript failed");
    Err(std::io::Error::other(message.trim().to_string()))
}

/// `text` for the inside of an `AppleScript` string: `\` and `"` escaped.
fn applescript_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_commands_for_local_and_remote_seats() {
        assert_eq!(
            attach_command(None, Path::new("/opt/homebrew/bin/tmux"), "dev-impl@my-rig"),
            "tmux attach -t dev-impl@my-rig"
        );
        assert_eq!(
            attach_command(Some("me@gpu-box"), Path::new("/usr/local/bin/tmux"), "lead@rig"),
            "ssh -t me@gpu-box /usr/local/bin/tmux attach -t lead@rig"
        );
        assert_eq!(attach_command(None, Path::new("tmux"), "it's here"), "tmux attach -t 'it'\\''s here'");
    }

    #[test]
    fn applescript_strings_escape_quotes_and_backslashes() {
        assert_eq!(applescript_string(r#"echo "a\b""#), r#"echo \"a\\b\""#);
    }
}
