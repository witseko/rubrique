//! "Open path" (Cmd+O): open a file, or show a folder in the explorer,
//! from a typed or pasted path. `~` is the home folder, a relative path
//! starts at the explorer folder of the open file (or the first folder),
//! and `file.md:12` opens the file on line 12.

use std::path::{Component, Path, PathBuf};

use eframe::egui::{self, Context, Key};

use super::{OpenFile, RubriqueApp};
use crate::alert::{self, ButtonKind};
use crate::document::view::jump_editor_to;
use crate::find::char_index;

/// Width of the "Open path" dialog.
const DIALOG_WIDTH: f32 = 420.0;

/// State of the "Open path" dialog while it is open.
#[derive(Debug, Default)]
pub(super) struct OpenPathDialog {
    /// The path being typed.
    text: String,
    /// Why the last attempt failed, shown under the field.
    error: Option<String>,
}

/// Where a typed path leads.
#[derive(Debug, PartialEq, Eq)]
struct Target {
    path: PathBuf,
    /// The 1-based line asked for with `path:line`.
    line: Option<usize>,
    is_folder: bool,
}

impl RubriqueApp {
    /// Show the "Open path" dialog (drawn by [`Self::open_path_dialog`]).
    pub(super) fn start_open_path(&mut self) {
        self.open_path = Some(OpenPathDialog::default());
    }

    /// Draw the "Open path" dialog while it is open.
    pub(super) fn open_path_dialog(&mut self, ctx: &Context) {
        let Some(dialog) = &mut self.open_path else {
            return;
        };
        let modal = alert::show(ctx, "open-path", DIALOG_WIDTH, |ui| {
            alert::title(ui, "Open path");
            alert::body(ui, "Open a file, or show a folder in the explorer. file.md:12 opens it on line 12.");
            let field = alert::text_field(
                ui,
                egui::TextEdit::singleline(&mut dialog.text)
                    .hint_text("~/notes/todo.md, a folder, or file.md:12")
                    .desired_width(f32::INFINITY),
            );
            if !field.has_focus() && dialog.error.is_none() && !field.lost_focus() {
                field.request_focus();
            }
            if field.changed() {
                dialog.error = None;
            }
            let entered = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            if let Some(error) = &dialog.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            let ((), (open, cancel)) = alert::button_row(
                ui,
                |_| (),
                |ui| {
                    let open = alert::button(ui, "Open", ButtonKind::Primary, true).clicked();
                    let cancel = alert::button(ui, "Cancel", ButtonKind::Secondary, false).clicked();
                    (open, cancel)
                },
            );
            (open || entered || alert::enter_for_default(ui), cancel)
        });
        let (open, cancel) = modal.inner;
        if cancel || modal.should_close() {
            self.open_path = None;
            return;
        }
        if !open {
            return;
        }
        let typed = dialog.text.clone();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let base = self.relative_base();
        match resolve(&typed, home.as_deref(), base.as_deref()) {
            Ok(target) => {
                self.open_path = None;
                self.open_target(ctx, target);
            }
            Err(error) => {
                if let Some(dialog) = &mut self.open_path {
                    dialog.error = Some(error);
                }
            }
        }
    }

    /// Open the files and folders the system handed over (Finder's "Open"
    /// and "Open With", see [`crate::open_documents`]). As with command line
    /// arguments, a file outside every explorer folder brings its folder in.
    pub(super) fn open_system_documents(&mut self, ctx: &Context) {
        for path in crate::open_documents::take() {
            let is_folder = path.is_dir();
            if !is_folder
                && let Some(folder) = path.parent()
                && !self.roots.iter().any(|root| path.starts_with(&root.path))
            {
                self.add_folder(folder.to_path_buf());
            }
            self.open_target(ctx, Target { path, line: None, is_folder });
        }
    }

    /// The folder a relative path starts from: the explorer folder holding
    /// the open file, or else the first explorer folder.
    fn relative_base(&self) -> Option<PathBuf> {
        let open = self.open_file.path();
        let holding = open.and_then(|open| self.roots.iter().find(|root| open.starts_with(&root.path)));
        holding.or(self.roots.first()).map(|root| root.path.clone())
    }

    /// Open a file in a tab (on its line, when one was asked for), or show
    /// a folder: revealed when it is inside an explorer folder, added as a
    /// new explorer folder otherwise.
    fn open_target(&mut self, ctx: &Context, target: Target) {
        if target.is_folder {
            if self.roots.iter().any(|root| target.path.starts_with(&root.path)) {
                self.reveal_in_files(target.path);
            } else {
                self.add_folder(target.path);
            }
            return;
        }
        self.open_in_tab(target.path.clone());
        let Some(line) = target.line else {
            return;
        };
        // A line is a place in the source, so a Markdown or HTML file shows it.
        // The check on the path skips the jump when the file could not be read.
        if let OpenFile::Text { document, source, .. } = &mut self.open_file
            && document.path == target.path
        {
            *source = true;
            jump_editor_to(ctx, char_index(&document.text, line_start(&document.text, line)));
        }
    }
}

/// Turn what the user typed into an existing file or folder.
///
/// `home` replaces a leading `~`; `base` is where a relative path starts.
/// A path that does not exist as typed but ends in `:<number>` is tried
/// as `path:line`. Returns the message to show when nothing is found.
fn resolve(typed: &str, home: Option<&Path>, base: Option<&Path>) -> Result<Target, String> {
    let typed = unquote(typed.trim());
    if typed.is_empty() {
        return Err("Type or paste a path.".to_string());
    }
    let path = absolute(typed, home, base)?;
    if let Some(target) = existing(&path, None) {
        return Ok(target);
    }
    // Checked after the plain path, so a file whose name has a colon still opens.
    if let Some((file, line)) = typed.rsplit_once(':')
        && let Ok(line) = line.parse::<usize>()
        && line > 0
        && let Some(target) = existing(&absolute(file, home, base)?, Some(line)).filter(|target| !target.is_folder)
    {
        return Ok(target);
    }
    Err(format!("No such file or folder: {}", path.display()))
}

/// `path` as a [`Target`] when it exists on disk.
fn existing(path: &Path, line: Option<usize>) -> Option<Target> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(Target { path: path.to_path_buf(), line, is_folder: metadata.is_dir() })
}

/// `typed` without one pair of matching quotes around it, as a path
/// copied from a terminal or a file manager often has.
fn unquote(typed: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = typed.strip_prefix(quote).and_then(|rest| rest.strip_suffix(quote)) {
            return inner;
        }
    }
    typed
}

/// `typed` as an absolute path, with `~` and a relative start expanded
/// and `.` and `..` worked out (so the same file is never open twice
/// under two spellings).
fn absolute(typed: &str, home: Option<&Path>, base: Option<&Path>) -> Result<PathBuf, String> {
    let path = if typed == "~" || typed.starts_with("~/") {
        let home = home.ok_or("Cannot find the home folder for ~.")?;
        home.join(typed.trim_start_matches('~').trim_start_matches('/'))
    } else if Path::new(typed).is_absolute() {
        PathBuf::from(typed)
    } else {
        let base = base.ok_or("Add a folder to the explorer first, or type a full path.")?;
        base.join(typed)
    };
    Ok(normalize(&path))
}

/// `path` with `.` dropped and each `..` taking off the part before it,
/// without touching the disk (symbolic links stay as typed, like the tree shows them).
fn normalize(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    clean
}

/// Byte index where 1-based `line` starts in `text`, or the end of the
/// text when it has fewer lines.
fn line_start(text: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    text.match_indices('\n').nth(line - 2).map_or(text.len(), |(at, _)| at + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    #[test]
    fn relative_path_starts_at_the_base_folder() {
        let folder = TempFolder::new();
        let file = folder.write("notes/todo.md", "x");
        let target = resolve("notes/todo.md", None, Some(folder.path())).unwrap();
        assert_eq!(target, Target { path: file.clone(), line: None, is_folder: false });
        // `.` and `..` are worked out, and quotes and spaces around are dropped.
        let target = resolve(" './notes/../notes/todo.md' ", None, Some(folder.path())).unwrap();
        assert_eq!(target.path, file);
    }

    #[test]
    fn tilde_is_the_home_folder() {
        let home = TempFolder::new();
        let file = home.write("todo.md", "x");
        assert_eq!(resolve("~/todo.md", Some(home.path()), None).unwrap().path, file);
        let target = resolve("~", Some(home.path()), None).unwrap();
        assert_eq!(target, Target { path: home.path().to_path_buf(), line: None, is_folder: true });
        // `~name` is a file called that, not another user's home.
        assert!(resolve("~todo.md", Some(home.path()), None).is_err());
    }

    #[test]
    fn absolute_path_ignores_the_base_folder() {
        let folder = TempFolder::new();
        let file = folder.write("a.md", "x");
        let typed = file.display().to_string();
        assert_eq!(resolve(&typed, None, None).unwrap().path, file);
    }

    #[test]
    fn trailing_number_is_a_line() {
        let folder = TempFolder::new();
        let file = folder.write("a.md", "x");
        let target = resolve("a.md:12", None, Some(folder.path())).unwrap();
        assert_eq!(target, Target { path: file, line: Some(12), is_folder: false });
        // A real file named with a colon wins over the line reading.
        let odd = folder.write("b.md:3", "x");
        assert_eq!(resolve("b.md:3", None, Some(folder.path())).unwrap().path, odd);
        // Line 0 and a folder with a line are not places to go.
        assert!(resolve("a.md:0", None, Some(folder.path())).is_err());
        folder.write("sub/c.md", "x");
        assert!(resolve("sub:2", None, Some(folder.path())).is_err());
    }

    #[test]
    fn missing_path_says_which_one() {
        let folder = TempFolder::new();
        let error = resolve("nope.md", None, Some(folder.path())).unwrap_err();
        assert_eq!(error, format!("No such file or folder: {}", folder.path().join("nope.md").display()));
        assert_eq!(resolve("  ", None, None).unwrap_err(), "Type or paste a path.");
        assert!(resolve("relative.md", None, None).unwrap_err().contains("full path"));
        assert!(resolve("~/x", None, None).unwrap_err().contains("home folder"));
    }

    #[test]
    fn line_start_finds_each_line() {
        let text = "one\ntwo\nthree";
        assert_eq!(line_start(text, 0), 0);
        assert_eq!(line_start(text, 1), 0);
        assert_eq!(line_start(text, 2), 4);
        assert_eq!(line_start(text, 3), 8);
        assert_eq!(line_start(text, 9), text.len());
    }
}
