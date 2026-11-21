//! Keep the explorer's Terminals list fresh: every couple of seconds a
//! background thread lists the tmux sessions, reads the screen of the ones
//! running an agent, and works out each one's status. Each remote `OpenRig`
//! host has a thread of its own, so a slow host never holds up the others.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui::Context;

use super::status::{TerminalStatus, is_agent, terminal_status};
use super::tmux::{SessionInfo, Tmux};

/// How often the sessions are checked.
const CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// How long to wait before looking for tmux again on a host that failed.
const REMOTE_RETRY: Duration = Duration::from_secs(30);

/// One session with its status, as the explorer shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRow {
    pub info: SessionInfo,
    pub status: TerminalStatus,
}

/// The running check. Dropping it stops the thread.
#[derive(Debug)]
pub struct TerminalMonitor {
    tmux: Tmux,
    rows: Arc<Mutex<Vec<TerminalRow>>>,
    /// The first check finished, so empty `rows` means no sessions.
    checked: Arc<AtomicBool>,
    /// Wakes the thread for a check now; dropping it stops the thread.
    wake: Sender<()>,
    /// The remote `OpenRig` hosts whose seats are listed too.
    remotes: Vec<RemoteHost>,
}

/// The check of one remote host's seats. Dropping it stops the thread.
#[derive(Debug)]
struct RemoteHost {
    host: String,
    /// tmux on the host, once it was found.
    tmux: Arc<Mutex<Option<Tmux>>>,
    rows: Arc<Mutex<Vec<TerminalRow>>>,
    /// The first check finished (it found seats, none, or no connection).
    checked: Arc<AtomicBool>,
    wake: Sender<()>,
}

impl RemoteHost {
    /// Start checking the seats on `host`.
    fn start(host: &str, ctx: &Context) -> Self {
        let (tmux, rows, checked) = (Arc::default(), Arc::default(), Arc::<AtomicBool>::default());
        let (wake, woken) = mpsc::channel();
        let (thread_host, thread_tmux, thread_rows, thread_checked, ctx) =
            (host.to_string(), Arc::clone(&tmux), Arc::clone(&rows), Arc::clone(&checked), ctx.clone());
        std::thread::spawn(move || {
            remote_loop(&thread_host, &thread_tmux, &woken, (&thread_rows, &thread_checked), &ctx);
        });
        RemoteHost { host: host.to_string(), tmux, rows, checked, wake }
    }

    /// tmux on the host for session `name`, when the host lists it.
    fn tmux_for(&self, name: &str) -> Option<Tmux> {
        let listed = lock(&self.rows).iter().any(|row| row.info.name == name);
        listed.then(|| lock(&self.tmux).clone()).flatten()
    }
}

impl TerminalMonitor {
    /// Start checking the sessions of `tmux`.
    pub fn start(tmux: Tmux, ctx: &Context) -> Self {
        let (rows, checked) = (Arc::default(), Arc::<AtomicBool>::default());
        let (wake, woken) = mpsc::channel();
        let (thread_tmux, thread_rows, thread_checked, ctx) =
            (tmux.clone(), Arc::clone(&rows), Arc::clone(&checked), ctx.clone());
        std::thread::spawn(move || check_loop(&thread_tmux, &woken, (&thread_rows, &thread_checked), &ctx));
        TerminalMonitor { tmux, rows, checked, wake, remotes: Vec::new() }
    }

    /// The local sessions were checked once, so [`TerminalMonitor::rows`]
    /// holds every local session.
    pub fn local_checked(&self) -> bool {
        self.checked.load(Ordering::Acquire)
    }

    /// The local sessions and every remote host were checked once.
    pub fn all_checked(&self) -> bool {
        self.local_checked() && self.remotes.iter().all(|remote| remote.checked.load(Ordering::Acquire))
    }

    /// The local tmux program, to start and open sessions.
    pub fn tmux(&self) -> &Tmux {
        &self.tmux
    }

    /// The tmux that runs session `name`: a remote host's for a seat listed
    /// there, else the local one.
    pub fn tmux_for(&self, name: &str) -> Tmux {
        self.remotes.iter().find_map(|remote| remote.tmux_for(name)).unwrap_or_else(|| self.tmux.clone())
    }

    /// True when session `name` runs on a remote host.
    pub fn is_remote(&self, name: &str) -> bool {
        self.remotes.iter().any(|remote| lock(&remote.rows).iter().any(|row| row.info.name == name))
    }

    /// List the seats of these remote `OpenRig` hosts too: start checking
    /// new ones and stop checking removed ones.
    pub fn set_remote_hosts(&mut self, hosts: &[String], ctx: &Context) {
        self.remotes.retain(|remote| hosts.contains(&remote.host));
        for host in hosts {
            if !self.remotes.iter().any(|remote| remote.host == *host) {
                self.remotes.push(RemoteHost::start(host, ctx));
            }
        }
    }

    /// The sessions from the last check: local ones newest first, then
    /// each remote host's.
    pub fn rows(&self) -> Vec<TerminalRow> {
        let mut rows = lock(&self.rows).clone();
        for remote in &self.remotes {
            rows.extend(lock(&remote.rows).iter().cloned());
        }
        rows
    }

    /// Check now (after a session was started).
    pub fn refresh(&self) {
        let _ = self.wake.send(());
        for remote in &self.remotes {
            let _ = remote.wake.send(());
        }
    }
}

/// Lock `mutex`, also after a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Find tmux on `host`, then check its seats like [`check_loop`]; until the
/// monitor drops the host. A host that cannot be reached lists nothing.
fn remote_loop(
    host: &str,
    tmux: &Mutex<Option<Tmux>>,
    woken: &Receiver<()>,
    (rows, checked): (&Mutex<Vec<TerminalRow>>, &AtomicBool),
    ctx: &Context,
) {
    loop {
        let found = lock(tmux).clone().or_else(|| Tmux::remote(host).ok());
        let fresh = found.as_ref().map(check).unwrap_or_default();
        *lock(tmux) = found;
        store(rows, checked, fresh, ctx);
        let wait = if lock(tmux).is_some() { CHECK_INTERVAL } else { REMOTE_RETRY };
        match woken.recv_timeout(wait) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Check, store the rows when they changed, wait; until the monitor is dropped.
fn check_loop(
    tmux: &Tmux,
    woken: &Receiver<()>,
    (rows, checked): (&Mutex<Vec<TerminalRow>>, &AtomicBool),
    ctx: &Context,
) {
    loop {
        store(rows, checked, check(tmux), ctx);
        match woken.recv_timeout(CHECK_INTERVAL) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Keep `fresh` in `rows` and mark the source checked, redrawing when
/// something changed.
fn store(rows: &Mutex<Vec<TerminalRow>>, checked: &AtomicBool, fresh: Vec<TerminalRow>, ctx: &Context) {
    let mut stored = lock(rows);
    let first = !checked.swap(true, Ordering::AcqRel);
    if first || *stored != fresh {
        *stored = fresh;
        ctx.request_repaint();
    }
}

/// Every session and its status, newest first.
fn check(tmux: &Tmux) -> Vec<TerminalRow> {
    let now = unix_now();
    let mut rows: Vec<TerminalRow> = tmux
        .list_sessions()
        .into_iter()
        .map(|info| {
            // Only an agent's screen says anything about its state.
            let screen =
                if is_agent(&info.command) { tmux.plain_screen(&info.name).unwrap_or_default() } else { String::new() };
            let status = terminal_status(&info.command, now - info.last_output, &screen);
            TerminalRow { info, status }
        })
        .collect();
    rows.sort_by(|a, b| b.info.created.cmp(&a.info.created).then_with(|| a.info.name.cmp(&b.info.name)));
    rows
}

/// `rows` (newest first) in the order the user dragged the list into:
/// sessions missing from `order` (started since) stay on top, newest first,
/// then the others as `order` lists them.
pub fn in_saved_order(mut rows: Vec<TerminalRow>, order: &[String]) -> Vec<TerminalRow> {
    // A stable sort keeps the newest-first order among the new sessions.
    rows.sort_by_key(|row| order.iter().position(|name| *name == row.info.name).map_or(0, |index| index + 1));
    rows
}

/// Move `name` in `names` to go right before `anchor`, or right after it
/// when `after` is true (a drop at the end of a group). Nothing changes
/// when either name is missing.
pub fn move_name(names: &mut Vec<String>, name: &str, anchor: &str, after: bool) {
    if name == anchor || !names.iter().any(|known| known == anchor) {
        return;
    }
    let Some(from) = names.iter().position(|known| known == name) else {
        return;
    };
    let moved = names.remove(from);
    let Some(to) = names.iter().position(|known| known == anchor) else {
        return;
    };
    names.insert(if after { to + 1 } else { to }, moved);
}

/// Seconds since 1970, as tmux counts.
/// Now, in Unix seconds.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

/// When a terminal was opened: `14:02` today, otherwise `2026-11-03 14:02`.
pub fn opened_label(created: i64) -> String {
    let Some(opened) = crate::date::local_at(created) else {
        return String::new();
    };
    let time = format!("{:02}:{:02}", opened.hour, opened.minute);
    if opened.date == crate::date::today_iso() { time } else { format!("{} {time}", opened.date) }
}

/// How long ago a Unix time was, roughly: "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago_label(time: i64) -> String {
    let seconds = (unix_now() - time).max(0);
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty first answer still counts as checked, so "no sessions" can be told
    /// apart from "not looked yet".
    #[test]
    fn an_empty_first_check_marks_the_source_checked() {
        let (rows, checked) = (Mutex::new(Vec::new()), AtomicBool::new(false));
        store(&rows, &checked, Vec::new(), &Context::default());
        assert!(checked.load(Ordering::Acquire));
        store(&rows, &checked, vec![row("build")], &Context::default());
        assert_eq!(lock(&rows).len(), 1);
    }

    fn row(name: &str) -> TerminalRow {
        let info = SessionInfo {
            name: name.to_string(),
            created: 0,
            last_output: 0,
            command: "zsh".to_string(),
            folder: std::path::PathBuf::new(),
        };
        TerminalRow { info, status: TerminalStatus::Shell }
    }

    fn names(rows: &[TerminalRow]) -> Vec<&str> {
        rows.iter().map(|row| row.info.name.as_str()).collect()
    }

    #[test]
    fn saved_order_puts_new_sessions_first() {
        let rows = vec![row("new"), row("newer-old"), row("a"), row("b")];
        let order = ["b".to_string(), "gone".to_string(), "a".to_string(), "newer-old".to_string()];
        assert_eq!(names(&in_saved_order(rows, &order)), ["new", "b", "a", "newer-old"]);
    }

    #[test]
    fn no_saved_order_keeps_newest_first() {
        let rows = vec![row("c"), row("b"), row("a")];
        assert_eq!(names(&in_saved_order(rows, &[])), ["c", "b", "a"]);
    }

    fn moved(name: &str, anchor: &str, after: bool) -> Vec<String> {
        let mut names: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        move_name(&mut names, name, anchor, after);
        names
    }

    #[test]
    fn move_name_puts_it_before_the_anchor() {
        assert_eq!(moved("d", "b", false), ["a", "d", "b", "c"]);
        assert_eq!(moved("a", "c", false), ["b", "a", "c", "d"]);
    }

    #[test]
    fn move_name_after_the_last_of_a_group() {
        assert_eq!(moved("a", "c", true), ["b", "c", "a", "d"]);
        assert_eq!(moved("b", "d", true), ["a", "c", "d", "b"]);
    }

    #[test]
    fn move_name_ignores_missing_names() {
        assert_eq!(moved("gone", "b", false), ["a", "b", "c", "d"]);
        assert_eq!(moved("a", "gone", true), ["a", "b", "c", "d"]);
    }

    #[test]
    fn opened_today_shows_only_the_time() {
        let label = opened_label(unix_now());
        assert_eq!(label.len(), 5, "{label}");
        assert!(opened_label(0).contains("19"), "an old date shows the date");
    }

    #[test]
    fn ago_labels() {
        assert_eq!(ago_label(unix_now()), "just now");
        assert_eq!(ago_label(unix_now() - 300), "5 min ago");
        assert_eq!(ago_label(unix_now() - 7300), "2 h ago");
    }
}
