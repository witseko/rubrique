//! The branch button in the status bar: shows the git branch of the open
//! file's folder; a click opens a menu to pull, commit, stash, unstash,
//! switch branch or create one. Git runs on a background thread, one
//! command at a time.
//!
//! Git runs only in a repository the user trusts. Its own config and hooks
//! can start programs (`core.fsmonitor` and clean filters even on `git
//! status`), so a repository from a zip or a shared folder gets a "Not
//! trusted" button instead, and nothing runs until the user trusts it.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Context, Key, RichText, Ui};

use super::RubriqueApp;
use crate::explorer::tree::FsNode;
use crate::git::{self, RepoStatus, SwitchTarget};
use crate::icons::{Icon, icon_text_button};
use crate::remote::task::{Task, spawn_with_repaint};

/// Seconds between re-reads of the repository, so a switch or a commit
/// made in a terminal shows up without a click.
const REFRESH_SECONDS: f64 = 5.0;
/// Height of the branch list in the menu before it scrolls.
const LIST_HEIGHT: f32 = 280.0;
/// Width of the branch menu.
const MENU_WIDTH: f32 = 300.0;
/// What the branch button says on a detached HEAD.
const DETACHED: &str = "detached HEAD";
/// Width of the menu that asks to trust a repository.
const TRUST_MENU_WIDTH: f32 = 320.0;

/// The git state of the open file's folder.
#[derive(Debug, Default)]
pub(super) struct GitState {
    /// The open file the repository below was looked up for.
    looked_up_for: Option<PathBuf>,
    /// The repository holding the open file, if any.
    repo: Option<PathBuf>,
    /// What was last read from `repo`.
    status: Option<RepoStatus>,
    /// A read in flight, for the repository it reads.
    reading: Option<(PathBuf, Task<Result<RepoStatus, String>>)>,
    /// egui time of the last read started.
    last_read: f64,
    /// A git command in flight, with what the button says meanwhile.
    job: Option<(String, Task<Result<String, String>>)>,
    /// What is typed in the menu's fields.
    menu: MenuFields,
}

/// The text fields of the branch menu, kept while it is closed.
#[derive(Debug, Default)]
struct MenuFields {
    /// The filter field, which also names a new branch.
    filter: String,
    /// The commit message while the menu shows the commit form.
    commit_message: Option<String>,
}

/// A git command the user picked in the branch menu.
enum MenuChoice {
    Pull,
    Commit(String),
    Stash,
    Unstash,
    Switch(SwitchTarget),
    CreateBranch(String),
}

impl MenuChoice {
    /// What the branch button says while the command runs.
    fn working_label(&self, branch: &str) -> String {
        match self {
            MenuChoice::Pull => format!("Pulling {branch}…"),
            MenuChoice::Commit(_) => "Committing…".to_string(),
            MenuChoice::Stash => "Stashing…".to_string(),
            MenuChoice::Unstash => "Unstashing…".to_string(),
            MenuChoice::Switch(target) => format!("Switching to {}…", target.branch_name()),
            MenuChoice::CreateBranch(name) => format!("Creating {name}…"),
        }
    }

    /// Run the command in `repo`; returns the status message.
    fn run(self, repo: &Path) -> Result<String, String> {
        match self {
            MenuChoice::Pull => git::pull(repo),
            MenuChoice::Commit(message) => git::commit_all(repo, &message),
            MenuChoice::Stash => git::stash(repo),
            MenuChoice::Unstash => git::unstash(repo),
            MenuChoice::Switch(target) => git::switch(repo, &target),
            MenuChoice::CreateBranch(name) => git::create_branch(repo, &name),
        }
    }
}

impl RubriqueApp {
    /// Follow the open file's repository, re-read it now and then, and
    /// pick up finished git work. Call once per frame.
    pub(super) fn poll_git(&mut self, ctx: &Context) {
        let now = ctx.input(|input| input.time);
        self.follow_open_file();
        self.finish_git_job();
        if let Some((repo, task)) = &self.git.reading
            && let Some(result) = task.take()
        {
            // A read for a repository the open file has left is dropped.
            if self.git.repo.as_ref() == Some(repo) {
                // A failed read (git missing, a broken repository) hides the button.
                self.git.status = result.ok();
            }
            self.git.reading = None;
        }
        let due = now - self.git.last_read >= REFRESH_SECONDS;
        if due && self.git.reading.is_none() && self.git.job.is_none() {
            self.read_repo_status(ctx, now);
        }
    }

    /// Look up the repository again when another file is opened. Remote
    /// files are mirrors, never a repository to run git in.
    fn follow_open_file(&mut self) {
        let path = self.open_file.path().map(Path::to_path_buf);
        if path == self.git.looked_up_for {
            return;
        }
        let repo = path.as_deref().filter(|path| !crate::remote::is_mirror(path)).and_then(git::repository_root);
        if repo != self.git.repo {
            self.git.status = None;
            self.git.menu = MenuFields::default();
            // Read the new repository on this frame.
            self.git.last_read = f64::NEG_INFINITY;
        }
        self.git.repo = repo;
        self.git.looked_up_for = path;
    }

    /// The open file's repository when the user trusts it: the only
    /// repository git may run in.
    fn trusted_repo(&self) -> Option<PathBuf> {
        self.git.repo.clone().filter(|repo| self.settings.trusts_repository(repo))
    }

    /// Start reading the open file's repository, when it is trusted.
    fn read_repo_status(&mut self, ctx: &Context, now: f64) {
        self.git.last_read = now;
        let Some(repo) = self.trusted_repo() else {
            return;
        };
        let folder = repo.clone();
        self.git.reading = Some((repo, spawn_with_repaint(ctx, move || git::read_status(&folder))));
    }

    /// Trust the open file's repository, remember it, and read it now.
    fn trust_open_repository(&mut self) {
        let Some(repo) = self.git.repo.clone() else {
            return;
        };
        self.settings.trust_repository(repo);
        self.write_settings();
        self.git.last_read = f64::NEG_INFINITY;
    }

    /// Show the result of a finished command, then re-read what it may
    /// have changed: the repository, the folders and the open file.
    fn finish_git_job(&mut self) {
        let Some(result) = self.git.job.as_ref().and_then(|(_, task)| task.take()) else {
            return;
        };
        self.git.job = None;
        self.git.last_read = f64::NEG_INFINITY;
        self.roots.iter_mut().for_each(FsNode::refresh);
        // Reload first: reloading the open file shows its own message,
        // which would otherwise replace the result of the command.
        self.check_disk();
        match result {
            Ok(message) => self.show_status(message),
            Err(reason) => self.show_status(format!("Git: {reason}")),
        }
    }

    /// Run `choice` in the repository in the background. The fields that
    /// led to it are cleared: the next time the menu opens it is fresh.
    fn start_git_job(&mut self, ctx: &Context, choice: MenuChoice, branch: &str) {
        let Some(repo) = self.trusted_repo() else {
            return;
        };
        let label = choice.working_label(branch);
        self.git.menu = MenuFields::default();
        self.git.job = Some((label, spawn_with_repaint(ctx, move || choice.run(&repo))));
    }

    /// The branch button, or a spinner while git works, or the "Not
    /// trusted" button in a repository the user has not trusted. Nothing
    /// outside a repository. Sits at the left of the status bar, before the
    /// path or a status message.
    pub(super) fn branch_button(&mut self, ui: &mut Ui) {
        if let Some((label, _)) = &self.git.job {
            ui.add(egui::Spinner::new().size(12.0));
            ui.label(RichText::new(label.as_str()).weak());
            super::status_separator(ui);
            return;
        }
        if self.git.repo.is_some() && self.trusted_repo().is_none() {
            self.trust_button(ui);
            return;
        }
        let Some(status) = &self.git.status else {
            return;
        };
        let name = status.current.clone().unwrap_or_else(|| DETACHED.to_string());
        let tip = "Git branch of this file's folder. Click to pull, commit, stash or switch branch";
        let response = icon_text_button(ui, Icon::Branch, &name, tip, false);
        super::status_separator(ui);
        let mut choice = None;
        egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_width(MENU_WIDTH);
            choice = branch_menu(ui, status, &mut self.git.menu);
            if choice.is_some() {
                ui.close();
            }
        });
        if let Some(choice) = choice {
            self.start_git_job(ui.ctx(), choice, &name);
        }
    }

    /// The "Not trusted" button: a click explains why git is off here and
    /// offers to trust the repository.
    fn trust_button(&mut self, ui: &mut Ui) {
        let Some(repo) = &self.git.repo else {
            return;
        };
        let tip = "Git is off in this repository until you trust it. Click for details";
        let response = icon_text_button(ui, Icon::Branch, "Not trusted", tip, false);
        super::status_separator(ui);
        let mut trust = false;
        egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_width(TRUST_MENU_WIDTH);
            trust = trust_menu(ui, repo);
            if trust {
                ui.close();
            }
        });
        if trust {
            self.trust_open_repository();
        }
    }
}

/// Why git is off in `repo`, and the button that trusts it; true when clicked.
fn trust_menu(ui: &mut Ui, repo: &Path) -> bool {
    ui.label(RichText::new("Trust this repository?").strong());
    ui.label(
        RichText::new(
            "Rubrique runs git here only after you trust it. A repository's own settings can start \
             programs when git runs, so trust only repositories you made or got from a source you trust.",
        )
        .weak()
        .small(),
    );
    ui.label(RichText::new(repo.display().to_string()).monospace().small());
    let mut trust = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            trust = ui.button("Trust this repository").clicked();
        });
    });
    trust
}

/// The menu under the branch button: the commit form while a message is
/// being written, else the branch and its actions, then the branches.
fn branch_menu(ui: &mut Ui, status: &RepoStatus, fields: &mut MenuFields) -> Option<MenuChoice> {
    if fields.commit_message.is_some() {
        return commit_form(ui, status, &mut fields.commit_message);
    }
    let mut choice = menu_header(ui, status, fields);
    ui.separator();
    let list_choice = branch_list(ui, status, &mut fields.filter);
    choice = choice.or(list_choice);
    choice
}

/// The current branch with its change and stash counts, and the row of
/// small action buttons. "Commit…" switches the menu to the commit form.
fn menu_header(ui: &mut Ui, status: &RepoStatus, fields: &mut MenuFields) -> Option<MenuChoice> {
    let mut choice = None;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("On branch").weak().small());
            ui.label(RichText::new(status.current.as_deref().unwrap_or(DETACHED)).strong());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
            ui.label(RichText::new(counts_text(status)).weak().small());
        });
    });
    let has_changes = status.changed_files > 0;
    ui.horizontal(|ui| {
        let tip = "Fetch and fast-forward this branch (git pull --ff-only)";
        if small_button(ui, "Pull", status.current.is_some(), tip) {
            choice = Some(MenuChoice::Pull);
        }
        if small_button(ui, "Commit…", has_changes, "Commit every change, new files too") {
            fields.commit_message = Some(String::new());
        }
        if small_button(ui, "Stash", has_changes, "Put every change aside, new files too (git stash)") {
            choice = Some(MenuChoice::Stash);
        }
        if small_button(ui, "Unstash", status.stashes > 0, "Bring back the last stashed changes (git stash pop)") {
            choice = Some(MenuChoice::Unstash);
        }
    });
    choice
}

/// "3 changed · 1 stashed", or "No changes".
fn counts_text(status: &RepoStatus) -> String {
    let changed = match status.changed_files {
        0 => "No changes".to_string(),
        count => format!("{count} changed"),
    };
    match status.stashes {
        0 => changed,
        count => format!("{changed} · {count} stashed"),
    }
}

/// A small flat button for the menu's action row; true when clicked.
fn small_button(ui: &mut Ui, label: &str, enabled: bool, tip: &str) -> bool {
    let button = ui.add_enabled(enabled, egui::Button::new(label).small());
    button.on_hover_text(tip).on_disabled_hover_text(tip).clicked()
}

/// The commit form: a message field with Cancel and Commit. Enter commits.
fn commit_form(ui: &mut Ui, status: &RepoStatus, message: &mut Option<String>) -> Option<MenuChoice> {
    let text = message.as_mut()?;
    let files = if status.changed_files == 1 {
        "1 changed file".to_string()
    } else {
        format!("{} changed files", status.changed_files)
    };
    ui.label(RichText::new(format!("Commit {files}")).strong());
    let branch = status.current.as_deref().unwrap_or(DETACHED);
    ui.label(RichText::new(format!("On {branch}. Every change is committed, new files too.")).weak().small());
    let field = ui.add(egui::TextEdit::singleline(text).hint_text("Commit message").desired_width(f32::INFINITY));
    let entered = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
    // Type straight away, without clicking the field first. Only while
    // nothing else has focus: asking every frame would take the focus back
    // on the frame Enter gives it up, and Enter would never commit.
    if !entered && ui.memory(|memory| memory.focused().is_none()) {
        field.request_focus();
    }
    let can_commit = !text.trim().is_empty();
    let mut commit = entered && can_commit;
    let mut cancel = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            commit |= ui.add_enabled(can_commit, egui::Button::new("Commit")).clicked();
            cancel = ui.button("Cancel").clicked();
        });
    });
    if cancel {
        *message = None;
        return None;
    }
    commit.then(|| MenuChoice::Commit(text.clone()))
}

/// The filter field and the branches that match it. A name no branch has
/// gets a first row that creates it; Enter takes the first row.
fn branch_list(ui: &mut Ui, status: &RepoStatus, filter: &mut String) -> Option<MenuChoice> {
    let mut choice = None;
    let field = ui
        .add(egui::TextEdit::singleline(filter).hint_text("Filter, or name a new branch").desired_width(f32::INFINITY));
    let entered = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
    let typed = filter.trim();
    let wanted = typed.to_lowercase();
    let matches = |name: &str| name.to_lowercase().contains(&wanted);
    let local: Vec<&String> = status.local.iter().filter(|name| matches(name)).collect();
    let remote: Vec<_> = status.remote.iter().filter(|branch| matches(&branch.name)).collect();
    let exists = status.local.iter().chain(status.remote.iter().map(|branch| &branch.name)).any(|name| name == typed);
    // What Enter takes: the new branch row when shown, else the first match.
    let mut first = None;
    if !typed.is_empty() && !exists {
        if let Some(problem) = git::branch_name_problem(typed) {
            ui.label(RichText::new(problem).weak().small());
        } else {
            let from = status.current.as_deref().unwrap_or(DETACHED);
            if ui.selectable_label(false, format!("+  Create branch {typed} from {from}")).clicked() {
                choice = Some(MenuChoice::CreateBranch(typed.to_string()));
            }
            first = Some(MenuChoice::CreateBranch(typed.to_string()));
        }
    }
    egui::ScrollArea::vertical().max_height(LIST_HEIGHT).show(ui, |ui| {
        if !local.is_empty() {
            ui.label(RichText::new("LOCAL").weak().small());
        }
        for name in local {
            let current = status.current.as_ref() == Some(name);
            let switch = || MenuChoice::Switch(SwitchTarget::Local(name.clone()));
            if !current && first.is_none() {
                first = Some(switch());
            }
            if ui.selectable_label(current, name.as_str()).clicked() && !current {
                choice = Some(switch());
            }
        }
        if !remote.is_empty() {
            ui.label(RichText::new("REMOTE").weak().small());
        }
        for branch in remote {
            let switch = || MenuChoice::Switch(SwitchTarget::Remote(branch.clone()));
            if first.is_none() {
                first = Some(switch());
            }
            let tip = format!("Make a local {} that tracks {}/{}", branch.name, branch.remote, branch.name);
            let text = format!("{}  ({})", branch.name, branch.remote);
            if ui.selectable_label(false, text).on_hover_text(tip).clicked() {
                choice = Some(switch());
            }
        }
    });
    // Enter on an empty filter does nothing: the first row is then just
    // the top of the list, not something the user aimed at.
    if entered && !typed.is_empty() {
        choice = choice.or(first);
    }
    choice
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::settings::Settings;
    use crate::test_support::TempFolder;

    /// Run git in `folder` for test setup, panicking on failure.
    fn git(folder: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "init.defaultBranch=main"])
            .args(args)
            .current_dir(folder)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    /// Poll until the background read of the repository is done.
    fn wait_for_status(app: &mut RubriqueApp, ctx: &Context) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.git.status.is_none() {
            assert!(Instant::now() < deadline, "the repository was never read");
            std::thread::sleep(Duration::from_millis(10));
            app.poll_git(ctx);
        }
    }

    #[cfg(unix)]
    #[test]
    fn git_runs_only_in_a_trusted_repository() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempFolder::new();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init"]);
        let note = temp.write("repo/notes.md", "one\n");
        git(&repo, &["add", "notes.md"]);
        git(&repo, &["commit", "-m", "first"]);
        // A hostile repository: `git status` starts its fsmonitor program.
        let ran = temp.path().join("ran");
        let hook = temp.write("hook.sh", format!("#!/bin/sh\ntouch '{}'\n", ran.display()));
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        git(&repo, &["config", "core.fsmonitor", hook.to_str().unwrap()]);

        let ctx = Context::default();
        let mut app = RubriqueApp::new(&ctx, Settings::default(), vec![repo.clone()], Some(note));
        app.poll_git(&ctx);
        assert_eq!(app.git.repo.as_deref(), Some(repo.as_path()));
        assert!(app.git.reading.is_none(), "git started in a repository nobody trusted");
        assert!(!ran.exists());

        app.settings.trust_repository(repo.clone());
        app.git.last_read = f64::NEG_INFINITY;
        app.poll_git(&ctx);
        wait_for_status(&mut app, &ctx);
        assert_eq!(app.git.status.as_ref().and_then(|status| status.current.as_deref()), Some("main"));
        // Trusted, the repository's own settings apply again, which shows the
        // program above really is what an untrusted read would have started.
        assert!(ran.exists());
    }
}
