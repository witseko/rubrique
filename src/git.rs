//! Git for the open file's folder: the branch it is on, the branches it
//! could switch to, and the few commands the status bar's branch menu runs
//! (switch, create a branch, pull, commit everything, stash and unstash).
//!
//! Everything runs the `git` program found on `PATH` with an argument list
//! (no shell), in the repository's top folder. These calls block, so the
//! app runs them on a background thread.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What the branch menu shows about a repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoStatus {
    /// The checked-out branch; `None` on a detached HEAD.
    pub current: Option<String>,
    /// Local branches, sorted by name.
    pub local: Vec<String>,
    /// Branches on a remote with no local branch of the same name, sorted.
    pub remote: Vec<RemoteBranch>,
    /// Files with changes not committed, new files included.
    pub changed_files: usize,
    /// Entries in the stash.
    pub stashes: usize,
}

/// A branch that exists only on a remote, like `origin/popup-menus-design`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBranch {
    /// The remote's name (`origin`).
    pub remote: String,
    /// The branch name on that remote (`popup-menus-design`).
    pub name: String,
}

/// Which branch to switch to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchTarget {
    /// A local branch, by name.
    Local(String),
    /// A remote branch: a local branch of the same name is made to track it.
    Remote(RemoteBranch),
}

impl SwitchTarget {
    /// The local branch name this switch ends on.
    pub fn branch_name(&self) -> &str {
        match self {
            SwitchTarget::Local(name) => name,
            SwitchTarget::Remote(branch) => &branch.name,
        }
    }
}

/// The top folder of the git repository holding `path` (a file or a
/// folder), or `None` when it is in none. Looks for a `.git` folder, or a
/// `.git` file (a worktree or a submodule), in `path` and its parents.
pub fn repository_root(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|folder| folder.join(".git").exists()).map(Path::to_path_buf)
}

/// The current branch, every branch `repo` could switch to, and how many
/// changed files and stash entries it has.
///
/// # Errors
///
/// `git`'s own message when it cannot run or fails.
pub fn read_status(repo: &Path) -> Result<RepoStatus, String> {
    let current = run_git(repo, &["branch", "--show-current"])?;
    // Full ref names, so a local branch called `origin/x` is never taken
    // for a remote one.
    let refs = run_git(repo, &["for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes"])?;
    let mut status = parse_branches(current.trim(), &refs);
    // One line per changed or new file; `-z` would also split renames, but
    // a count is all the menu needs.
    status.changed_files = run_git(repo, &["status", "--porcelain"])?.lines().count();
    status.stashes = run_git(repo, &["stash", "list"])?.lines().count();
    Ok(status)
}

/// Why `name` cannot be a new branch, or `None` when it looks fine. A
/// quick check of git's main rules for the menu to show while typing;
/// [`create_branch`] asks git itself before creating it.
pub fn branch_name_problem(name: &str) -> Option<&'static str> {
    const FORBIDDEN: [char; 9] = [' ', '~', '^', ':', '?', '*', '[', '\\', '\t'];
    if name.is_empty() {
        return Some("Type a name for the new branch");
    }
    #[expect(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "git refuses names ending in exactly `.lock`; this is a ref name rule, not a file type"
    )]
    let bad = name.starts_with(['-', '/', '.'])
        || name.ends_with(['/', '.'])
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{")
        || name.contains(FORBIDDEN)
        || name.chars().any(char::is_control);
    bad.then_some("Git does not allow this name (spaces, \"..\", ~ ^ : ? * [ are not allowed)")
}

/// Create the branch `name` from the current commit and switch to it.
/// Changed files come along to the new branch.
///
/// # Errors
///
/// When git refuses the name or a branch with that name exists.
pub fn create_branch(repo: &Path, name: &str) -> Result<String, String> {
    if let Some(problem) = branch_name_problem(name) {
        return Err(problem.to_string());
    }
    run_git(repo, &["check-ref-format", "--branch", name]).map_err(|_| format!("Not a valid branch name: {name}"))?;
    run_git(repo, &["switch", "-c", name])?;
    Ok(format!("Created and switched to {name}."))
}

/// Commit every change, new files included, with `message`.
///
/// # Errors
///
/// An empty message, nothing to commit, or git's reason (a failing hook,
/// no user name set).
pub fn commit_all(repo: &Path, message: &str) -> Result<String, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a commit message first".to_string());
    }
    run_git(repo, &["add", "--all"])?;
    run_git(repo, &["commit", "-m", message])?;
    let subject = message.lines().next().unwrap_or(message);
    Ok(format!("Committed: {subject}"))
}

/// Put every change, new files included, in the stash, leaving the files
/// as in the last commit.
///
/// # Errors
///
/// Nothing to stash, or git's reason.
pub fn stash(repo: &Path) -> Result<String, String> {
    let output = run_git(repo, &["stash", "push", "--include-untracked"])?;
    // With nothing to stash git succeeds and says so on standard output.
    if output.starts_with("No local changes") {
        return Err("No changes to stash".to_string());
    }
    Ok("Stashed your changes.".to_string())
}

/// Bring back the newest stash entry and drop it from the stash.
///
/// # Errors
///
/// An empty stash, or changes that clash with it (the entry is then kept).
pub fn unstash(repo: &Path) -> Result<String, String> {
    run_git(repo, &["stash", "pop"])?;
    Ok("Unstashed your changes.".to_string())
}

/// Switch `repo` to `target`. Returns the status message to show.
///
/// # Errors
///
/// `git`'s reason when it refuses, for example local changes that the
/// switch would overwrite. Nothing changes then.
pub fn switch(repo: &Path, target: &SwitchTarget) -> Result<String, String> {
    match target {
        SwitchTarget::Local(name) => {
            check_branch_name(name)?;
            run_git(repo, &["switch", name.as_str()])?;
        }
        SwitchTarget::Remote(branch) => {
            check_branch_name(&branch.name)?;
            check_branch_name(&branch.remote)?;
            let tracked = format!("{}/{}", branch.remote, branch.name);
            run_git(repo, &["switch", "--track", tracked.as_str()])?;
        }
    }
    Ok(format!("Switched to {}.", target.branch_name()))
}

/// Pull the current branch, fast-forward only: a pull that would need a
/// merge is refused instead of leaving conflicts in the files. Returns the
/// status message to show.
///
/// # Errors
///
/// `git`'s reason when the pull fails (no upstream, no network, the
/// branches have diverged). Nothing changes then.
pub fn pull(repo: &Path) -> Result<String, String> {
    let before = run_git(repo, &["rev-parse", "HEAD"])?;
    run_git(repo, &["pull", "--ff-only"])?;
    let after = run_git(repo, &["rev-parse", "HEAD"])?;
    let (before, after) = (before.trim(), after.trim());
    if before == after {
        return Ok("Already up to date.".to_string());
    }
    let range = format!("{before}..{after}");
    let count = run_git(repo, &["rev-list", "--count", range.as_str()])?;
    let branch = run_git(repo, &["branch", "--show-current"])?;
    Ok(pulled_message(branch.trim(), count.trim()))
}

/// "Pulled main: 3 new commits." (or one commit).
fn pulled_message(branch: &str, count: &str) -> String {
    let commits = if count == "1" { "1 new commit".to_string() } else { format!("{count} new commits") };
    format!("Pulled {branch}: {commits}.")
}

/// Refuse a name `git` would read as an option. Names come from `git`'s
/// own list, which never holds one, so this only guards against a bug.
fn check_branch_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.starts_with('-') {
        return Err(format!("Not a branch name: {name}"));
    }
    Ok(())
}

/// Run `git` with `args` in `repo` and return what it printed.
///
/// # Errors
///
/// The first line of `git`'s error output, or why it could not start.
fn run_git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        // Never wait for a password nobody can type: there is no terminal,
        // so a pull that needs one fails with git's message instead.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Could not run git: {error}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    Err(error_message(&String::from_utf8_lossy(&output.stderr)))
}

/// The line of `git`'s error output worth showing in the status bar: the
/// first one, without its `error:` or `fatal:` prefix.
fn error_message(stderr: &str) -> String {
    let line = stderr.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("git failed");
    let line = line.strip_prefix("error:").or_else(|| line.strip_prefix("fatal:")).unwrap_or(line);
    line.trim().trim_end_matches(':').to_string()
}

/// Build a [`RepoStatus`] (with no change or stash counts yet) from the current branch name (empty when detached)
/// and the full ref names `git for-each-ref` printed, one per line.
fn parse_branches(current: &str, refs: &str) -> RepoStatus {
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in refs.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix("refs/heads/") {
            local.push(name.to_string());
        } else if let Some(rest) = line.strip_prefix("refs/remotes/") {
            // `origin/HEAD` points at another remote branch; it is not one.
            match rest.split_once('/') {
                Some((remote_name, name)) if name != "HEAD" => {
                    remote.push(RemoteBranch { remote: remote_name.to_string(), name: name.to_string() });
                }
                _ => {}
            }
        }
    }
    local.sort();
    // A remote branch already checked out locally is switched to as the
    // local one, so it is listed once.
    remote.retain(|branch| !local.contains(&branch.name));
    remote.sort_by(|a, b| (&a.name, &a.remote).cmp(&(&b.name, &b.remote)));
    let current = (!current.is_empty()).then(|| current.to_string());
    RepoStatus { current, local, remote, changed_files: 0, stashes: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// A repository on `main` with one commit of `notes.md`, and its own
    /// identity so the commit and stash commands work on any machine.
    fn repository(folder: &Path) {
        git(folder, &["init"]);
        git(folder, &["config", "user.name", "Test"]);
        git(folder, &["config", "user.email", "test@example.com"]);
        git(folder, &["config", "commit.gpgsign", "false"]);
        std::fs::write(folder.join("notes.md"), "one\n").unwrap();
        git(folder, &["add", "notes.md"]);
        git(folder, &["commit", "-m", "first"]);
    }

    #[test]
    fn parses_local_and_remote_only_branches() {
        let refs = "refs/heads/main\nrefs/heads/feature\nrefs/remotes/origin/HEAD\n\
                    refs/remotes/origin/main\nrefs/remotes/origin/release\n";
        let branches = parse_branches("main", refs);
        assert_eq!(branches.current.as_deref(), Some("main"));
        assert_eq!(branches.local, ["feature", "main"]);
        assert_eq!(branches.remote, [RemoteBranch { remote: "origin".into(), name: "release".into() }]);
    }

    #[test]
    fn detached_head_has_no_current_branch() {
        assert_eq!(parse_branches("", "refs/heads/main\n").current, None);
    }

    #[test]
    fn error_message_is_the_first_line_without_its_prefix() {
        let stderr = "error: Your local changes would be overwritten by checkout:\n\tnotes.md\nAborting\n";
        assert_eq!(error_message(stderr), "Your local changes would be overwritten by checkout");
        assert_eq!(error_message(""), "git failed");
    }

    #[test]
    fn refuses_a_name_that_looks_like_an_option() {
        let folder = TempFolder::new();
        let target = SwitchTarget::Local("--orphan".into());
        assert!(switch(folder.path(), &target).is_err());
    }

    #[test]
    fn finds_the_repository_root_from_a_file_inside_it() {
        let folder = TempFolder::new();
        repository(folder.path());
        let file = folder.write("docs/deep/page.md", "x");
        assert_eq!(repository_root(&file).as_deref(), Some(folder.path()));
    }

    #[test]
    fn lists_and_switches_branches() {
        let folder = TempFolder::new();
        repository(folder.path());
        git(folder.path(), &["branch", "feature"]);
        let branches = read_status(folder.path()).unwrap();
        assert_eq!(branches.current.as_deref(), Some("main"));
        assert_eq!(branches.local, ["feature", "main"]);

        let message = switch(folder.path(), &SwitchTarget::Local("feature".into())).unwrap();
        assert_eq!(message, "Switched to feature.");
        assert_eq!(read_status(folder.path()).unwrap().current.as_deref(), Some("feature"));
    }

    #[test]
    fn switch_refused_by_local_changes_keeps_the_branch() {
        let folder = TempFolder::new();
        repository(folder.path());
        git(folder.path(), &["switch", "-c", "feature"]);
        std::fs::write(folder.path().join("notes.md"), "two\n").unwrap();
        git(folder.path(), &["commit", "-am", "second"]);
        git(folder.path(), &["switch", "main"]);
        std::fs::write(folder.path().join("notes.md"), "edited\n").unwrap();

        let result = switch(folder.path(), &SwitchTarget::Local("feature".into()));
        assert!(result.unwrap_err().contains("local changes"));
        assert_eq!(read_status(folder.path()).unwrap().current.as_deref(), Some("main"));
    }

    #[test]
    fn pulls_new_commits_and_switches_to_a_remote_branch() {
        let upstream = TempFolder::new();
        repository(upstream.path());
        git(upstream.path(), &["branch", "release"]);
        let clone_parent = TempFolder::new();
        git(clone_parent.path(), &["clone", upstream.path().to_str().unwrap(), "clone"]);
        let clone = clone_parent.path().join("clone");

        assert_eq!(pull(&clone).unwrap(), "Already up to date.");
        std::fs::write(upstream.path().join("notes.md"), "two\n").unwrap();
        git(upstream.path(), &["commit", "-am", "second"]);
        assert_eq!(pull(&clone).unwrap(), "Pulled main: 1 new commit.");
        assert_eq!(std::fs::read_to_string(clone.join("notes.md")).unwrap(), "two\n");

        let branches = read_status(&clone).unwrap();
        let release = RemoteBranch { remote: "origin".into(), name: "release".into() };
        assert_eq!(branches.remote, std::slice::from_ref(&release));
        switch(&clone, &SwitchTarget::Remote(release)).unwrap();
        let branches = read_status(&clone).unwrap();
        assert_eq!(branches.current.as_deref(), Some("release"));
        assert_eq!(branches.remote, Vec::new());
    }

    #[test]
    fn branch_name_problem_catches_names_git_refuses() {
        assert_eq!(branch_name_problem("fix-odds-ttl"), None);
        assert_eq!(branch_name_problem("team/fix"), None);
        for bad in ["", "has space", "a..b", "-x", "end/", "x.lock", "a~b", "a@{b"] {
            assert!(branch_name_problem(bad).is_some(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn creates_a_branch_and_switches_to_it() {
        let folder = TempFolder::new();
        repository(folder.path());
        assert_eq!(create_branch(folder.path(), "new-idea").unwrap(), "Created and switched to new-idea.");
        assert_eq!(read_status(folder.path()).unwrap().current.as_deref(), Some("new-idea"));
        assert!(create_branch(folder.path(), "new-idea").is_err());
        assert!(create_branch(folder.path(), "bad name").is_err());
    }

    #[test]
    fn commits_every_change_including_new_files() {
        let folder = TempFolder::new();
        repository(folder.path());
        std::fs::write(folder.path().join("notes.md"), "two\n").unwrap();
        folder.write("new.md", "new\n");
        assert_eq!(read_status(folder.path()).unwrap().changed_files, 2);

        assert!(commit_all(folder.path(), "  ").is_err());
        assert_eq!(commit_all(folder.path(), "Add new notes\n\nbody").unwrap(), "Committed: Add new notes");
        assert_eq!(read_status(folder.path()).unwrap().changed_files, 0);
        assert!(commit_all(folder.path(), "again").is_err());
    }

    #[test]
    fn stashes_and_unstashes_changes() {
        let folder = TempFolder::new();
        repository(folder.path());
        assert_eq!(stash(folder.path()).unwrap_err(), "No changes to stash");
        std::fs::write(folder.path().join("notes.md"), "edited\n").unwrap();
        folder.write("new.md", "new\n");

        assert_eq!(stash(folder.path()).unwrap(), "Stashed your changes.");
        let status = read_status(folder.path()).unwrap();
        assert_eq!((status.changed_files, status.stashes), (0, 1));
        assert!(!folder.path().join("new.md").exists());

        assert_eq!(unstash(folder.path()).unwrap(), "Unstashed your changes.");
        let status = read_status(folder.path()).unwrap();
        assert_eq!((status.changed_files, status.stashes), (2, 0));
        assert_eq!(std::fs::read_to_string(folder.path().join("notes.md")).unwrap(), "edited\n");
        assert!(unstash(folder.path()).is_err());
    }
}
