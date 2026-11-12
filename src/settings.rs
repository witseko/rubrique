//! User settings stored as a plain `key=value` file in the config folder.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::ai::AiSettings;
use crate::remote::settings_text;
use crate::settings::accessibility::AccessibilitySettings;
use crate::settings::explorer_filter::ExplorerFilter;
use crate::settings::layout::LayoutSettings;
use crate::settings::privacy::PrivacySettings;
use crate::settings::text_highlight::TextHighlightSettings;
use crate::theme::catalog::ThemeChoice;
use crate::theme::code_colors::CodeColors;
use crate::voice::VoiceSettings;

pub mod accessibility;
pub mod explorer_filter;
pub mod layout;
pub mod privacy;
pub mod text_highlight;
pub mod view;

/// Everything Rubrique remembers between runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// Explorer root folders of the last session, in order.
    pub folders: Vec<PathBuf>,
    /// File open at the end of the last session (the active tab).
    pub last_file: Option<PathBuf>,
    /// Every tab open at the end of the last session, left to right.
    pub open_tabs: Vec<PathBuf>,
    /// The explorer's file filter (the funnel in the Files header).
    pub explorer_filter: ExplorerFilter,
    /// Mark files that have notes in the explorer.
    pub note_marks: bool,
    /// Inner window size (width, height) in points.
    pub window_size: Option<(f32, f32)>,
    /// Color theme, or "system" to follow the OS.
    pub theme: ThemeChoice,
    /// How syntax is colored: with the theme's colors or VS Code's.
    pub code_colors: CodeColors,
    /// Rules that color matches in plain text files.
    pub text_highlight: TextHighlightSettings,
    /// Text size, editor font size, reduced motion and focus outlines.
    pub accessibility: AccessibilitySettings,
    /// Where new scratch files go; `None` means the default folder.
    pub scratch_folder: Option<PathBuf>,
    /// AI tools and their commands.
    pub ai: AiSettings,
    /// Voice memos: folder, dropped files, transcription.
    pub voice: VoiceSettings,
    /// Encrypted scratches and the app PIN.
    pub privacy: PrivacySettings,
    /// Remote folders over SSH.
    pub remote: crate::remote::RemoteSettings,
    /// Which panels and explorer sections are open.
    pub layout: LayoutSettings,
    /// The order the user dragged the Terminals list into, by session name.
    /// Sessions not in it (new ones) show above, newest first.
    pub terminal_order: Vec<String>,
}

impl Settings {
    /// Read settings from disk; missing or broken files give defaults.
    pub fn load() -> Self {
        config_folder()
            .map(|folder| folder.join("settings"))
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Write settings to the config folder, creating it when needed.
    ///
    /// # Errors
    ///
    /// Fails when there is no home folder or the file cannot be written.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = config_folder().map(|folder| folder.join("settings")) else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no home folder"));
        };
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        fs::write(path, self.to_text())
    }

    /// Parse `key=value` lines. Unknown keys and bad values are ignored.
    pub fn parse(text: &str) -> Self {
        let mut settings = Settings::default();
        let mut width = None;
        let mut height = None;
        // Files from before multiple folders have one `last_folder` line.
        let mut legacy_folder = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            if settings.ai.parse_line(key.trim(), value)
                || settings.voice.parse_line(key.trim(), value)
                || settings.accessibility.parse_line(key.trim(), value)
                || settings.privacy.parse_line(key.trim(), value)
                || settings.remote.parse_line(key.trim(), value)
                || settings.layout.parse_line(key.trim(), value)
                || settings.text_highlight.parse_line(key.trim(), value)
                || settings.explorer_filter.parse_line(key.trim(), value)
            {
                continue;
            }
            match key.trim() {
                "folder" => settings.folders.extend(from_settings_text(value)),
                "last_folder" => legacy_folder = non_empty_path(value),
                "last_file" => settings.last_file = from_settings_text(value),
                "tab" => settings.open_tabs.extend(from_settings_text(value)),
                "terminal_order" if !value.is_empty() => settings.terminal_order.push(value.to_string()),
                "note_marks" => settings.note_marks = value == "true",
                "window_width" => width = value.parse::<f32>().ok(),
                "window_height" => height = value.parse::<f32>().ok(),
                "theme" => settings.theme = ThemeChoice::parse(value),
                "code_colors" => settings.code_colors = CodeColors::parse(value),
                "scratch_folder" => settings.scratch_folder = non_empty_path(value),
                _ => {}
            }
        }
        if settings.folders.is_empty() {
            settings.folders.extend(legacy_folder);
        }
        if let (Some(width), Some(height)) = (width, height) {
            settings.window_size = Some((width, height));
        }
        settings
    }

    /// Serialize to the `key=value` format that [`Settings::parse`] reads.
    pub fn to_text(&self) -> String {
        let path_text =
            |path: &Option<PathBuf>| path.as_ref().map(|path| path.display().to_string()).unwrap_or_default();
        let last_file = self.last_file.as_deref().map(settings_text).unwrap_or_default();
        let window_lines = match self.window_size {
            Some((width, height)) => format!("window_width={width}\nwindow_height={height}\n"),
            None => String::new(),
        };
        let path_lines = |key: &str, paths: &[PathBuf]| {
            paths.iter().fold(String::new(), |mut lines, path| {
                let _ = writeln!(lines, "{key}={}", settings_text(path));
                lines
            })
        };
        let folder_lines = path_lines("folder", &self.folders);
        let tab_lines = path_lines("tab", &self.open_tabs);
        let terminal_lines = self.terminal_order.iter().fold(String::new(), |mut lines, name| {
            let _ = writeln!(lines, "terminal_order={name}");
            lines
        });
        format!(
            "{folder_lines}{tab_lines}{terminal_lines}last_file={}\nnote_marks={}\ntheme={}\ncode_colors={}\nscratch_folder={}\n{window_lines}{}{}{}{}{}{}{}{}",
            last_file,
            self.note_marks,
            self.theme.id(),
            self.code_colors.id(),
            path_text(&self.scratch_folder),
            self.accessibility.to_lines(),
            self.ai.to_lines(),
            self.voice.to_lines(),
            self.privacy.to_lines(),
            self.remote.to_lines(),
            self.layout.to_lines(),
            self.text_highlight.to_lines(),
            self.explorer_filter.to_lines(),
        )
    }
}

/// A path from the settings file: `ssh://host/path` stands for a remote
/// folder's mirror path. An empty value means "not set".
fn from_settings_text(value: &str) -> Option<PathBuf> {
    crate::remote::from_settings_text(value)
}

/// An empty value means "not set".
fn non_empty_path(value: &str) -> Option<PathBuf> {
    (!value.is_empty()).then(|| PathBuf::from(value))
}

impl Settings {
    /// The folder for new scratch files: the chosen one, else the default.
    pub fn scratch_folder_or_default(&self) -> Option<PathBuf> {
        self.scratch_folder.clone().or_else(default_scratch_folder)
    }
}

/// Default folder for scratch files (`scratches` in the config folder).
pub fn default_scratch_folder() -> Option<PathBuf> {
    config_folder().map(|folder| folder.join("scratches"))
}

/// Folder for cached file summaries (`cache/summaries` in the config folder).
pub fn summary_cache_folder() -> Option<PathBuf> {
    config_folder().map(|folder| folder.join("cache").join("summaries"))
}

/// The empty folder an AI tool runs in while it writes a text highlight
/// rule (`cache/highlight-rule` in the config folder).
pub fn highlight_rule_folder() -> Option<PathBuf> {
    config_folder().map(|folder| folder.join("cache").join("highlight-rule"))
}

/// Default folder for skills (`skills` in the config folder).
pub fn default_skills_folder() -> Option<PathBuf> {
    config_folder().map(|folder| folder.join("skills"))
}

/// The file that remembers the agent sessions each terminal ran.
pub fn agent_sessions_file() -> Option<PathBuf> {
    config_folder().map(|folder| folder.join("agent-sessions"))
}

/// Folder for unsaved edits kept between runs (`drafts` in the config folder).
pub fn drafts_folder() -> Option<PathBuf> {
    config_folder().map(|folder| folder.join("drafts"))
}

/// Default folder for agents (`agents` in the config folder).
pub fn default_agents_folder() -> Option<PathBuf> {
    config_folder().map(|folder| folder.join("agents"))
}

/// Where remote files are mirrored (`remote` in the config folder).
pub fn remote_mirror_folder() -> Option<PathBuf> {
    config_folder().map(|folder| folder.join("remote"))
}

/// Folder for the user's own `*.theme` files (`themes` in the config folder).
pub fn themes_folder() -> Option<PathBuf> {
    config_folder().map(|folder| folder.join("themes"))
}

/// macOS: `~/Library/Application Support/rubrique`.
/// Linux: `$XDG_CONFIG_HOME/rubrique`, fallback `~/.config/rubrique`.
fn config_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home?.join("Library/Application Support")
    } else {
        match std::env::var_os("XDG_CONFIG_HOME") {
            Some(folder) if Path::new(&folder).is_absolute() => PathBuf::from(folder),
            _ => home?.join(".config"),
        }
    };
    Some(base.join("rubrique"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let remote_folder = crate::remote::testing::mirror_of("server", Path::new("/srv/notes"));
        let settings = Settings {
            folders: vec![PathBuf::from("/home/a/my notes"), PathBuf::from("/work/docs"), remote_folder.clone()],
            last_file: Some(PathBuf::from("/home/a/my notes/x=y.md")),
            open_tabs: vec![PathBuf::from("/home/a/my notes/x=y.md"), PathBuf::from("/work/docs/photo.png")],
            explorer_filter: ExplorerFilter { markdown_only: true, unsaved_only: true, show_closed_terminals: false },
            note_marks: true,
            window_size: Some((1200.0, 800.5)),
            theme: ThemeChoice::Named("dark".to_string()),
            code_colors: CodeColors::VsCode,
            accessibility: AccessibilitySettings {
                text_size: accessibility::TextSize::Larger,
                editor_font_size: 15,
                page_zoom: 120,
                show_menu_bar: false,
                reduce_motion: true,
                strong_focus: true,
                line_numbers: true,
                wrap_lines: false,
            },
            scratch_folder: Some(PathBuf::from("/home/a/scratch")),
            ai: {
                let mut ai = AiSettings::default();
                ai.default_tool = crate::ai::AiTool::Grok;
                ai.set_command(crate::ai::AiTool::Grok, "grok-cli --headless {prompt}");
                ai
            },
            voice: crate::voice::VoiceSettings {
                folder: "audio".to_string(),
                copy_dropped: false,
                auto_transcribe: false,
                command: "yap transcribe {audio}".to_string(),
            },
            privacy: privacy::PrivacySettings {
                encrypt_scratches: true,
                vault: Some(crate::crypto::Vault::create("pw").1),
                pin_hash: Some(crate::crypto::pin_hash("1234")),
                lock_after_minutes: 15,
                touch_id: true,
            },
            remote: crate::remote::RemoteSettings { enabled: true, ssh_program: "/opt/ssh".to_string() },
            layout: layout::LayoutSettings {
                side_panel_open: true,
                side_view: layout::SideView::Notes,
                files_collapsed: false,
                terminals_collapsed: true,
                tab_position: layout::TabPosition::Explorer,
                explorer_view: layout::ExplorerView::Tabs,
            },
            terminal_order: vec!["build".to_string(), "notes agent".to_string()],
            text_highlight: text_highlight::TextHighlightSettings {
                enabled: false,
                rules: vec![crate::highlight::text_rules::HighlightRule::new(
                    "Ticket",
                    r"\bPROJ-[0-9]+\b",
                    crate::highlight::text_rules::RuleColor::Key,
                )],
            },
        };
        let text = settings.to_text();
        assert!(text.contains("folder=ssh://server/srv/notes\n"), "{text}");
        assert_eq!(Settings::parse(&text), settings);
        let with_tab = Settings { open_tabs: vec![remote_folder.join("a.md")], ..Settings::default() };
        assert!(with_tab.to_text().contains("tab=ssh://server/srv/notes/a.md\n"));
        assert_eq!(Settings::parse(&with_tab.to_text()), with_tab);
    }

    #[test]
    fn defaults_for_garbage_and_unknown_keys() {
        let settings = Settings::parse("nonsense\ncolor=blue\nwindow_width=wide\nmarkdown_only=yes\n");
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn round_trip_without_window_size() {
        let settings = Settings {
            explorer_filter: ExplorerFilter { markdown_only: true, ..ExplorerFilter::default() },
            ..Settings::default()
        };
        assert_eq!(Settings::parse(&settings.to_text()), settings);
    }

    #[test]
    fn old_preview_mode_line_is_ignored() {
        // Every file opens in preview now, so the old global mode is not read.
        assert_eq!(Settings::parse("preview_mode=false\n"), Settings::default());
    }

    #[test]
    fn old_last_folder_line_becomes_the_only_folder() {
        let settings = Settings::parse("last_folder=/home/a/notes\n");
        assert_eq!(settings.folders, [PathBuf::from("/home/a/notes")]);
        let both = Settings::parse("last_folder=/old\nfolder=/new\n");
        assert_eq!(both.folders, [PathBuf::from("/new")]);
    }

    #[test]
    fn scratch_folder_falls_back_to_the_default() {
        let custom = Settings { scratch_folder: Some(PathBuf::from("/tmp/s")), ..Settings::default() };
        assert_eq!(custom.scratch_folder_or_default(), Some(PathBuf::from("/tmp/s")));
        assert_eq!(Settings::default().scratch_folder_or_default(), default_scratch_folder());
        assert_eq!(Settings::parse("scratch_folder=\n").scratch_folder, None);
    }

    #[test]
    fn settings_from_before_highlight_rules_get_the_defaults() {
        let old = Settings::parse("theme=dark\nmarkdown_only=true\n");
        assert_eq!(old.text_highlight.rules, crate::highlight::text_rules::default_rules());
    }

    #[test]
    fn theme_defaults_to_indigo() {
        assert_eq!(Settings::parse("").theme, ThemeChoice::Named("dark".to_string()));
        assert_eq!(Settings::parse("theme=system\n").theme, ThemeChoice::System);
        assert_eq!(Settings::parse("theme=light\n").theme, ThemeChoice::Named("light".to_string()));
    }
}
