//! Run tmux: list sessions, start one, read its screen, type into it.
//!
//! Every call runs one `tmux` process with its arguments as a list, never
//! through a shell, so session names and typed text cannot run commands.
//! Several tmux commands can share one process, separated by a `;` argument.
//!
//! A remote tmux runs on an `OpenRig` host over the shared SSH connection.
//! It lists only the rig seats there, named `seat@rig@host`, so they never
//! clash with local sessions and group by rig and host.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::groups::seat_of;
use super::keys::KeyInput;
use super::screen::{CAPTURE_FORMAT, Screen, parse_capture};
use crate::ai::find_on_path;
use crate::remote::connection;

/// Where Homebrew puts programs; a Mac app opened from the Finder does not
/// have these on its `PATH`.
const BREW_FOLDERS: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "/home/linuxbrew/.linuxbrew/bin"];

/// One line per pane from `list-panes -a`: session, times, program, folder,
/// and whether it is the pane you see in that session.
const LIST_FORMAT: &str = "#{session_name}:#{session_created}:#{window_activity}:#{window_active}#{pane_active}:#{pane_current_command}:#{pane_current_path}";

/// Prints the path of tmux on a remote host: on its login shell's PATH, or
/// in a Homebrew folder. Fails when tmux is not there.
const FIND_REMOTE_TMUX: &str = "p=$(\"${SHELL:-/bin/sh}\" -lc 'command -v tmux' 2>/dev/null) && [ -x \"$p\" ] && echo \"$p\" && exit 0; \
    for p in /opt/homebrew/bin/tmux /usr/local/bin/tmux /home/linuxbrew/.linuxbrew/bin/tmux; do \
    [ -x \"$p\" ] && echo \"$p\" && exit 0; done; exit 1";
/// Runs tmux on a remote host: `$1` is its path, the rest tmux's arguments.
const RUN_REMOTE_TMUX: &str = "p=$1; shift; exec \"$p\" \"$@\"";
/// Longest wait for one remote tmux call.
const REMOTE_TIMEOUT: Duration = Duration::from_secs(10);

/// The tmux paste buffer Rubrique fills for [`Tmux::paste`] (deleted right
/// after pasting).
const PASTE_BUFFER: &str = "rubrique-send";

/// The tmux program, and the server socket to use.
#[derive(Clone, Debug)]
pub struct Tmux {
    program: PathBuf,
    /// `-L <socket>`: a separate tmux server. `None` is the user's normal
    /// server, so Rubrique's sessions also show in `tmux ls`.
    socket: Option<String>,
    /// The SSH host tmux runs on; `None` for this computer.
    host: Option<String>,
}

/// A running session, as the explorer lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub name: String,
    /// When the session started (Unix seconds).
    pub created: i64,
    /// When its window last printed something (Unix seconds).
    pub last_output: i64,
    /// The program in the pane you see: `zsh`, `claude`, `vim`.
    pub command: String,
    pub folder: PathBuf,
}

impl Tmux {
    /// tmux on `PATH` or in a Homebrew folder; `None` when it is not installed.
    pub fn find() -> Option<Tmux> {
        let program = find_on_path("tmux")
            .or_else(|| BREW_FOLDERS.iter().map(|folder| Path::new(folder).join("tmux")).find(|path| path.is_file()))?;
        Some(Tmux { program, socket: None, host: None })
    }

    /// tmux on the SSH host `host`, for its `OpenRig` seats.
    ///
    /// # Errors
    ///
    /// The host cannot be reached, or tmux is not installed there.
    pub fn remote(host: &str) -> io::Result<Tmux> {
        let output = connection::run(host, FIND_REMOTE_TMUX, &[], None, REMOTE_TIMEOUT)?;
        let path = String::from_utf8_lossy(&output).trim().to_string();
        if !path.starts_with('/') {
            return Err(io::Error::other(format!("tmux is not installed on {host}")));
        }
        Ok(Tmux { program: PathBuf::from(path), socket: None, host: Some(host.to_string()) })
    }

    /// The SSH host tmux runs on; `None` for this computer.
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// The session's name in tmux: a remote seat's name without `@host`.
    pub fn tmux_name<'a>(&self, name: &'a str) -> &'a str {
        self.host.as_deref().and_then(|host| name.strip_suffix(host)?.strip_suffix('@')).unwrap_or(name)
    }

    /// Where the tmux program is, as shown in Settings.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// A tmux using its own server, for tests.
    #[cfg(test)]
    pub fn with_socket(socket: &str) -> Option<Tmux> {
        Tmux::find().map(|tmux| Tmux { socket: Some(socket.to_string()), ..tmux })
    }

    /// Run tmux with `args`; its output, or its error message as the error.
    fn run(&self, args: &[&str]) -> io::Result<String> {
        if let Some(host) = &self.host {
            let mut remote_args = vec![self.program.as_os_str()];
            remote_args.extend(args.iter().map(OsStr::new));
            let output = connection::run(host, RUN_REMOTE_TMUX, &remote_args, None, REMOTE_TIMEOUT)?;
            return Ok(String::from_utf8_lossy(&output).into_owned());
        }
        let mut command = Command::new(&self.program);
        if let Some(socket) = &self.socket {
            command.args(["-L", socket]);
        }
        let output = command.args(args).output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_string()))
        }
    }

    /// Every session with the pane you see in it. Empty when no tmux
    /// server runs (tmux reports that as an error).
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        let output = self.run(&["list-panes", "-a", "-F", LIST_FORMAT]).unwrap_or_default();
        let sessions = parse_session_list(&output);
        match &self.host {
            None => sessions,
            Some(host) => sessions
                .into_iter()
                .filter(|session| seat_of(&session.name).is_some())
                .map(|session| SessionInfo { name: format!("{}@{host}", session.name), ..session })
                .collect(),
        }
    }

    /// True when a session with exactly this name runs.
    pub fn has_session(&self, name: &str) -> bool {
        let name = self.tmux_name(name);
        self.run(&["has-session", "-t", &format!("={name}")]).is_ok()
    }

    /// Start session `name` in `folder`, unless it already runs.
    ///
    /// # Errors
    ///
    /// tmux failed to start it (a bad folder, say).
    pub fn start(&self, name: &str, folder: &Path) -> io::Result<()> {
        if self.has_session(name) {
            return Ok(());
        }
        let name = self.tmux_name(name);
        let folder = folder.to_string_lossy();
        self.run(&["new-session", "-d", "-s", name, "-c", &folder]).map(|_| ())
    }

    /// The screen of the pane you see in session `name`, with colors.
    ///
    /// # Errors
    ///
    /// The session is gone, or tmux failed.
    pub fn capture(&self, name: &str) -> io::Result<Screen> {
        let target = target(self.tmux_name(name));
        let output = self.run(&[
            "display-message",
            "-p",
            "-t",
            &target,
            CAPTURE_FORMAT,
            ";",
            "capture-pane",
            "-p",
            "-e",
            "-t",
            &target,
        ])?;
        parse_capture(&output).ok_or_else(|| io::Error::other("tmux printed something unexpected"))
    }

    /// The last lines of the screen as plain text, to read an agent's state.
    ///
    /// # Errors
    ///
    /// The session is gone, or tmux failed.
    pub fn plain_screen(&self, name: &str) -> io::Result<String> {
        self.run(&["capture-pane", "-p", "-t", &target(self.tmux_name(name))])
    }

    /// Make the session's window `columns` by `rows` characters.
    ///
    /// # Errors
    ///
    /// The session is gone, or tmux failed.
    pub fn resize(&self, name: &str, columns: u16, rows: u16) -> io::Result<()> {
        let (columns, rows) = (columns.to_string(), rows.to_string());
        self.run(&["resize-window", "-t", &target(self.tmux_name(name)), "-x", &columns, "-y", &rows]).map(|_| ())
    }

    /// Type `keys` into the session, all in one tmux call.
    ///
    /// # Errors
    ///
    /// The session is gone, or tmux failed.
    pub fn send(&self, name: &str, keys: &[KeyInput]) -> io::Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let target = target(self.tmux_name(name));
        let mut args: Vec<&str> = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            if index > 0 {
                args.push(";");
            }
            args.extend(["send-keys", "-t", &target]);
            match key {
                // `-l`: the text as it is, not read as key names.
                KeyInput::Text(text) => args.extend(["-l", "--", text]),
                KeyInput::Key(key) => args.push(key),
            }
        }
        self.run(&args).map(|_| ())
    }

    /// Paste `text` into the session as one bracketed paste, then press
    /// Enter. An agent gets a multi-line message as one prompt this way,
    /// where typing it would send each line on its own.
    ///
    /// # Errors
    ///
    /// The session is gone, or tmux failed.
    pub fn paste(&self, name: &str, text: &str) -> io::Result<()> {
        let target = target(self.tmux_name(name));
        let args = [
            "set-buffer",
            "-b",
            PASTE_BUFFER,
            "--",
            text,
            ";",
            "paste-buffer",
            "-p",
            "-d",
            "-b",
            PASTE_BUFFER,
            "-t",
            &target,
            ";",
            "send-keys",
            "-t",
            &target,
            "Enter",
        ];
        self.run(&args).map(|_| ())
    }

    /// Rename a session (when its `.tmux` file is renamed).
    ///
    /// # Errors
    ///
    /// The session is gone, the name is taken, or tmux failed.
    pub fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        self.run(&["rename-session", "-t", &format!("={}", self.tmux_name(from)), self.tmux_name(to)]).map(|_| ())
    }

    /// End session `name` and everything running in it.
    ///
    /// # Errors
    ///
    /// The session is gone, or tmux failed.
    pub fn kill_session(&self, name: &str) -> io::Result<()> {
        self.run(&["kill-session", "-t", &format!("={}", self.tmux_name(name))]).map(|_| ())
    }

    /// Stop the session and everything running in it (tests only: closing
    /// a tab leaves the session running).
    #[cfg(test)]
    pub fn kill_server(&self) {
        let _ = self.run(&["kill-server"]);
    }
}

/// The pane you see in session `name`. `=` asks for this exact name, so
/// `build` never picks `build-2`.
fn target(name: &str) -> String {
    format!("={name}:")
}

/// Read `list-panes -a` output, keeping the visible pane of each session.
fn parse_session_list(output: &str) -> Vec<SessionInfo> {
    output
        .lines()
        .filter_map(|line| {
            // Fields are split on `:`, which tmux never allows in a session
            // name (tabs would be nicer, but tmux 3.8 prints them as `_`).
            // The folder comes last, so a `:` inside a path stays in it.
            let fields: Vec<&str> = line.splitn(6, ':').collect();
            let [name, created, last_output, visible, command, folder] = fields[..] else {
                return None;
            };
            (visible == "11").then(|| SessionInfo {
                name: name.to_string(),
                created: created.parse().unwrap_or(0),
                last_output: last_output.parse().unwrap_or(0),
                command: command.to_string(),
                folder: PathBuf::from(folder),
            })
        })
        .collect()
}

/// The session a `.tmux` file opens: its name, then a short code from its
/// full path, so `build.tmux` in two folders gets two sessions.
pub fn session_name_for_file(path: &Path) -> String {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy()).unwrap_or_default();
    // tmux reserves `.` and `:` in names; keep names easy to type too.
    let readable: String =
        stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    format!("{readable}-{:06x}", fnv1a(path.to_string_lossy().as_bytes()) & 0x00ff_ffff)
}

/// The `.tmux` file in `folder` that opens the session `name`, if any.
/// `folder` must be spelled the way the explorer spells it, since the
/// session name comes from the file's full path.
pub fn file_for_session(folder: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| super::is_terminal_path(path) && session_name_for_file(path) == name)
}

/// FNV-1a: a tiny hash that, unlike `DefaultHasher`, never changes between
/// Rust versions, so a file keeps its session name across updates.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// A name for a new terminal: `term-1`, or the next number not in `taken`.
pub fn new_session_name(taken: &[SessionInfo]) -> String {
    // One of the first `taken.len() + 1` numbers is always free.
    (1..=taken.len() + 1)
        .map(|number| format!("term-{number}"))
        .find(|name| taken.iter().all(|session| &session.name != name))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn reads_the_visible_pane_of_each_session() {
        let output = "a:100:200:11:zsh:/tmp\na:100:150:01:vim:/tmp\nb c:5:6:11:claude:/srv/a:b\nbroken line\n";
        let sessions = parse_session_list(output);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].command, "zsh");
        assert_eq!(sessions[1].name, "b c");
        assert_eq!(sessions[1].folder, PathBuf::from("/srv/a:b"));
    }

    #[test]
    fn file_sessions_are_stable_readable_and_unique_per_folder() {
        let first = session_name_for_file(Path::new("/notes/build.tmux"));
        assert_eq!(first, session_name_for_file(Path::new("/notes/build.tmux")));
        assert!(first.starts_with("build-") && first.len() == "build-".len() + 6);
        assert_ne!(first, session_name_for_file(Path::new("/other/build.tmux")));
        assert!(session_name_for_file(Path::new("/a/my notes.v2.tmux")).starts_with("my-notes-v2-"));
    }

    #[test]
    fn finds_the_file_that_opens_a_session() {
        let temp = crate::test_support::TempFolder::new();
        let file = temp.write("build.tmux", "");
        temp.write("notes.md", "");
        assert_eq!(file_for_session(temp.path(), &session_name_for_file(&file)), Some(file));
        assert_eq!(file_for_session(temp.path(), "term-1"), None);
    }

    #[test]
    fn new_names_skip_taken_ones() {
        let taken = |name: &str| SessionInfo {
            name: name.to_string(),
            created: 0,
            last_output: 0,
            command: String::new(),
            folder: PathBuf::new(),
        };
        assert_eq!(new_session_name(&[]), "term-1");
        assert_eq!(new_session_name(&[taken("term-1"), taken("term-3")]), "term-2");
    }

    /// A remote tmux lists only the rig seats, as `seat@rig@host`, and gets
    /// the plain seat name back in every call.
    #[test]
    fn remote_tmux_lists_seats_with_their_host() {
        use std::os::unix::fs::PermissionsExt;

        crate::remote::testing::use_fake_ssh();
        let temp = crate::test_support::TempFolder::new();
        let program = temp.write(
            "tmux",
            "#!/bin/sh\ncd \"${0%/*}\"\nprintf '%s\\n' \"$*\" >> calls\n\
             printf 'builder@demo:1:2:11:claude:/srv\\nshell:1:2:11:zsh:/srv\\n'\n",
        );
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let tmux = Tmux { program, socket: None, host: Some("me@rig-host".to_string()) };

        let names: Vec<String> = tmux.list_sessions().into_iter().map(|session| session.name).collect();
        assert_eq!(names, ["builder@demo@me@rig-host"]);
        tmux.paste("builder@demo@me@rig-host", "it's $(done)").unwrap();
        tmux.kill_session("builder@demo@me@rig-host").unwrap();
        let calls = std::fs::read_to_string(temp.path().join("calls")).unwrap();
        assert!(calls.contains("set-buffer -b rubrique-send -- it's $(done) ; paste-buffer"), "{calls}");
        assert!(calls.contains("-t =builder@demo: Enter"), "{calls}");
        assert!(calls.contains("kill-session -t =builder@demo\n"), "{calls}");
    }

    /// Runs only where tmux is installed; uses its own tmux server.
    #[test]
    fn starts_types_into_and_reads_a_real_session() {
        let socket = format!("rubrique-test-{}", std::process::id());
        let Some(tmux) = Tmux::with_socket(&socket) else {
            return;
        };
        let folder = std::env::temp_dir();
        tmux.start("demo", &folder).unwrap();
        tmux.start("demo", &folder).unwrap();
        tmux.resize("demo", 70, 12).unwrap();
        tmux.send("demo", &[KeyInput::Text("echo rubrique-$((40+2))".to_string()), KeyInput::Key("Enter".to_string())])
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut screen = tmux.capture("demo").unwrap();
        while !screen.plain_text().contains("rubrique-42") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            screen = tmux.capture("demo").unwrap();
        }
        tmux.kill_server();
        assert!(screen.plain_text().contains("rubrique-42"), "screen was:\n{}", screen.plain_text());
        assert_eq!((screen.columns, screen.rows), (70, 12));
    }

    /// Runs only where tmux is installed; uses its own tmux server.
    #[test]
    fn pastes_text_and_presses_enter() {
        let socket = format!("rubrique-paste-test-{}", std::process::id());
        let Some(tmux) = Tmux::with_socket(&socket) else {
            return;
        };
        tmux.start("demo", &std::env::temp_dir()).unwrap();
        tmux.paste("demo", "echo pasted-$((40+2))").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut text = tmux.plain_screen("demo").unwrap();
        while !text.contains("pasted-42") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            text = tmux.plain_screen("demo").unwrap();
        }
        tmux.kill_server();
        assert!(text.contains("pasted-42"), "screen was:\n{text}");
    }
}
