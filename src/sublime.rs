//! Reads the unsaved tabs out of Sublime Text, for "Import from Sublime
//! Text". Sublime keeps every open tab's unsaved text in its session file
//! (JSON, written while it runs and when it quits), so the texts can be
//! copied out without Sublime's help.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The session files Sublime Text writes into its `Local` folder: the first
/// while it runs (hot exit), the second when it quits.
const SESSION_FILE_NAMES: [&str; 2] = ["Auto Save Session.sublime_session", "Session.sublime_session"];

/// One unsaved Sublime tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsavedText {
    /// What the tab was called: its own name for a new tab (Sublime names
    /// one after its first line), or the file name for an edited file.
    /// Empty when the tab had no name.
    pub name: String,
    /// The tab's text.
    pub text: String,
}

/// Sublime's `Local` folders on this computer, Sublime Text 4 first, then 3.
fn local_folders() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else {
        match std::env::var_os("XDG_CONFIG_HOME") {
            Some(folder) if Path::new(&folder).is_absolute() => PathBuf::from(folder),
            _ => home.join(".config"),
        }
    };
    let names =
        if cfg!(target_os = "macos") { ["Sublime Text", "Sublime Text 3"] } else { ["sublime-text", "sublime-text-3"] };
    names.iter().map(|name| base.join(name).join("Local")).collect()
}

/// The most recently written session file in `folders`. Sublime updates
/// one or the other depending on how it last stopped, so the newest one
/// holds the tabs as they were last seen.
fn newest_session_file(folders: &[PathBuf]) -> Option<PathBuf> {
    folders
        .iter()
        .flat_map(|folder| SESSION_FILE_NAMES.iter().map(move |name| folder.join(name)))
        .filter_map(|path| {
            let modified = fs::metadata(&path).and_then(|metadata| metadata.modified()).ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// The unsaved texts in a Sublime session file's JSON, window by window in
/// tab order: every new tab with text in it, and every file with edits that
/// are not on disk. Blank tabs and files without edits are left out.
///
/// # Errors
///
/// The JSON cannot be read.
pub fn unsaved_texts(json: &str) -> Result<Vec<UnsavedText>, String> {
    let session: Value =
        serde_json::from_str(json).map_err(|error| format!("Cannot read the Sublime Text session: {error}"))?;
    let buffers = session
        .get("windows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|window| window.get("buffers").and_then(Value::as_array))
        .flatten();
    Ok(buffers.filter_map(unsaved_text).collect())
}

/// The unsaved text of one session buffer, if it has any. A buffer holds
/// `contents` only when its text differs from disk (or it was never saved);
/// it may still hold the same text as the file, so that is checked too.
fn unsaved_text(buffer: &Value) -> Option<UnsavedText> {
    let text = buffer.get("contents").and_then(Value::as_str)?;
    if text.trim().is_empty() {
        return None;
    }
    let name = match buffer.get("file").and_then(Value::as_str) {
        Some(file) => {
            let file = Path::new(file);
            if fs::read_to_string(file).is_ok_and(|on_disk| on_disk == text) {
                return None;
            }
            file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
        }
        None => buffer.pointer("/settings/name").and_then(Value::as_str).unwrap_or_default().to_string(),
    };
    Some(UnsavedText { name, text: text.to_string() })
}

/// The unsaved texts from this computer's Sublime Text, with the session
/// file they came from.
///
/// # Errors
///
/// A message for the user: Sublime Text's session was not found, or it
/// cannot be read.
pub fn read_unsaved_texts() -> Result<(PathBuf, Vec<UnsavedText>), String> {
    let path = newest_session_file(&local_folders()).ok_or("No Sublime Text session found on this computer")?;
    let json = fs::read_to_string(&path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    Ok((path.clone(), unsaved_texts(&json)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    #[test]
    fn finds_new_tabs_and_edited_files() {
        let temp = TempFolder::new();
        let saved = temp.write("saved.txt", "same text");
        let edited = temp.write("plan.txt", "old text");
        let json = serde_json::json!({
            "windows": [
                {"buffers": [
                    {"contents": "shopping list\nmilk", "settings": {"name": "shopping list"}},
                    {"contents": "  \n", "settings": {}},
                    {"file": saved, "contents": "same text"},
                    {"file": "/no/such/file.txt"},
                ]},
                {"buffers": [
                    {"file": edited, "contents": "new text"},
                    {"contents": "no name", "settings": {"buffer_size": 7}},
                ]},
            ]
        });
        assert_eq!(
            unsaved_texts(&json.to_string()).unwrap(),
            vec![
                UnsavedText { name: "shopping list".to_string(), text: "shopping list\nmilk".to_string() },
                UnsavedText { name: "plan".to_string(), text: "new text".to_string() },
                UnsavedText { name: String::new(), text: "no name".to_string() },
            ]
        );
    }

    #[test]
    fn an_empty_session_has_no_texts_and_bad_json_is_an_error() {
        assert_eq!(unsaved_texts("{}").unwrap(), Vec::new());
        assert_eq!(unsaved_texts(r#"{"windows": [{"groups": []}]}"#).unwrap(), Vec::new());
        assert!(unsaved_texts("not json").is_err());
    }

    #[test]
    fn picks_the_newest_session_file() {
        let temp = TempFolder::new();
        let older = temp.write("4/Session.sublime_session", "{}");
        let newer = temp.write("3/Auto Save Session.sublime_session", "{}");
        let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::options().write(true).open(&older).unwrap().set_modified(hour_ago).unwrap();
        let folders = [temp.path().join("4"), temp.path().join("3"), temp.path().join("missing")];
        assert_eq!(newest_session_file(&folders), Some(newer));
        assert_eq!(newest_session_file(&[temp.path().join("missing")]), None);
    }
}
