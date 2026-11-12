//! Rubrique: a fast editor, agentic on demand. This version reviews every
//! change in a merge screen first.

mod ai;
mod app;
mod crypto;
mod date;
mod document;
mod explorer;
mod find;
mod format;
mod helm;
mod highlight;
mod icons;
mod install;
mod markdown;
mod media;
mod open_documents;
mod remote;
mod scratch;
mod settings;
mod shell_path;
mod sublime;
mod terminal;
#[cfg(test)]
mod test_support;
mod theme;
mod touch_id;
mod vfs;
mod voice;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use eframe::egui;

use settings::Settings;

/// Window size (width, height) on the first run, before one is saved.
const DEFAULT_WINDOW_SIZE: (f32, f32) = (1100.0, 720.0);

/// App icon for the window (the same art as the packaged icons).
/// macOS draws its dock icons with a margin built into the art; other systems
/// show the whole picture, so they get the larger Linux tile.
#[cfg(target_os = "macos")]
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../assets/icon/rubrique-256.png");
#[cfg(not(target_os = "macos"))]
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../assets/icon/linux/rubrique-256.png");

const USAGE: &str = "Usage: rubrique [FOLDER | FILE]...

Opens each FOLDER in the explorer, and each FILE's folder; the first FILE opens.
Without arguments, reopens the last folders (none on the first run).";

/// Handle `--help` / `--version`, pick the starting folder, run the window.
fn main() -> ExitCode {
    let arguments: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    match arguments.first().and_then(|argument| argument.to_str()) {
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("-V" | "--version") => {
            println!("rubrique {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    // Before anything looks for a program: Finder and desktop launchers
    // start the app with a bare PATH (see `shell_path`).
    shell_path::extend_path_from_login_shell();
    let settings = Settings::load();
    remote::set_ssh_program(&settings.remote.ssh_program);
    let (roots, file) = match starting_location(&arguments, &settings) {
        Ok(location) => location,
        Err(message) => {
            eprintln!("rubrique: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let (width, height) = settings.window_size.unwrap_or(DEFAULT_WINDOW_SIZE);
    let mut viewport = egui::ViewportBuilder::default().with_title("Rubrique").with_inner_size([width, height]);
    // Window and taskbar icon. A broken image only means no icon, never a failed start.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(WINDOW_ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    // Before the event loop starts: a launch from Finder hands its files over
    // while the app finishes launching.
    open_documents::listen();
    let result = eframe::run_native(
        "rubrique",
        options,
        Box::new(move |creation_context| {
            open_documents::set_waker(creation_context.egui_ctx.clone());
            Ok(Box::new(app::RubriqueApp::new(&creation_context.egui_ctx, settings, roots, file)))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rubrique: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Pick the explorer root folders and an optional file to open (spec 01, "Root folders").
///
/// Without arguments and no saved folder (the first run), the explorer
/// starts empty and the user adds folders from there.
///
/// Errors are messages for the terminal: a missing path.
fn starting_location(arguments: &[PathBuf], settings: &Settings) -> Result<(Vec<PathBuf>, Option<PathBuf>), String> {
    if !arguments.is_empty() {
        let mut roots = Vec::new();
        let mut file = None;
        for argument in arguments {
            let path = std::path::absolute(argument).map_err(|error| format!("{}: {error}", argument.display()))?;
            let root = if path.is_dir() {
                path
            } else if path.is_file() {
                let folder = path.parent().map_or_else(|| PathBuf::from("/"), Path::to_path_buf);
                file.get_or_insert(path);
                folder
            } else {
                return Err(format!("no such file or folder: {}", path.display()));
            };
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        return Ok((roots, file));
    }

    // Remote folders are not checked here (that would wait for the network);
    // the explorer shows an error row when one cannot be reached.
    let usable = |path: &Path, local_check: fn(&Path) -> bool| {
        if remote::is_mirror(path) { settings.remote.enabled } else { local_check(path) }
    };
    let saved: Vec<PathBuf> = settings.folders.iter().filter(|folder| usable(folder, Path::is_dir)).cloned().collect();
    if !saved.is_empty() {
        let file = settings
            .last_file
            .clone()
            .filter(|file| usable(file, Path::is_file) && saved.iter().any(|folder| file.starts_with(folder)));
        return Ok((saved, file));
    }
    Ok((Vec::new(), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    #[test]
    fn folder_argument_opens_that_folder() {
        let temp = TempFolder::new();
        let location = starting_location(&[temp.path().to_path_buf()], &Settings::default()).unwrap();
        assert_eq!(location, (vec![temp.path().to_path_buf()], None));
    }

    #[test]
    fn file_argument_opens_the_file_and_its_folder() {
        let temp = TempFolder::new();
        let file = temp.write("docs/todo.md", "");
        let location = starting_location(std::slice::from_ref(&file), &Settings::default()).unwrap();
        assert_eq!(location, (vec![temp.path().join("docs")], Some(file)));
    }

    #[test]
    fn missing_argument_is_an_error() {
        let temp = TempFolder::new();
        let missing = temp.path().join("nope.md");
        let error = starting_location(&[missing], &Settings::default()).unwrap_err();
        assert!(error.starts_with("no such file or folder"), "{error}");
    }

    #[test]
    fn no_argument_reopens_last_folder_and_file_inside_it() {
        let temp = TempFolder::new();
        let file = temp.write("todo.md", "");
        let settings =
            Settings { folders: vec![temp.path().to_path_buf()], last_file: Some(file.clone()), ..Settings::default() };
        assert_eq!(starting_location(&[], &settings).unwrap(), (vec![temp.path().to_path_buf()], Some(file)));
    }

    #[test]
    fn last_file_outside_last_folder_is_not_reopened() {
        let temp = TempFolder::new();
        let folder = temp.make_folder("notes");
        let outside = temp.write("other.md", "");
        let settings = Settings { folders: vec![folder.clone()], last_file: Some(outside), ..Settings::default() };
        assert_eq!(starting_location(&[], &settings).unwrap(), (vec![folder], None));
    }

    #[test]
    fn first_run_opens_no_folder() {
        assert_eq!(starting_location(&[], &Settings::default()).unwrap(), (Vec::new(), None));
    }

    #[test]
    fn no_argument_and_no_usable_saved_folder_opens_no_folder() {
        let settings = Settings { folders: vec![PathBuf::from("/definitely/not/here")], ..Settings::default() };
        assert_eq!(starting_location(&[], &settings).unwrap(), (Vec::new(), None));
    }

    #[test]
    fn several_arguments_open_several_folders() {
        let temp = TempFolder::new();
        let notes = temp.make_folder("notes");
        let file = temp.write("work/plan.md", "");
        let work = temp.path().join("work");
        let arguments = [notes.clone(), file.clone(), notes.clone()];
        assert_eq!(starting_location(&arguments, &Settings::default()).unwrap(), (vec![notes, work], Some(file)));
    }

    #[test]
    fn saved_folders_come_back_and_missing_ones_are_skipped() {
        let temp = TempFolder::new();
        let first = temp.make_folder("a");
        let second = temp.make_folder("b");
        let file = temp.write("b/todo.md", "");
        let settings = Settings {
            folders: vec![first.clone(), temp.path().join("gone"), second.clone()],
            last_file: Some(file.clone()),
            ..Settings::default()
        };
        assert_eq!(starting_location(&[], &settings).unwrap(), (vec![first, second], Some(file)));
    }
}
