//! `RubriqueApp`: top-level state, panel layout, global shortcuts and dialogs.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, Context, Key, KeyboardShortcut, Modifiers, RichText, Ui, ViewportCommand};

mod agent_history;
mod ask;
mod changes_panel;
mod delegate;
mod delete;
mod drafts;
mod find;
mod helm;
mod images;
mod install;
mod naming;
mod new_entry;
mod open_path;
mod privacy;
mod remote;
mod side_panel;
mod structure;
mod summary;
mod tab_groups;
mod tabs;
mod terminals;
mod voice;

use crate::ai::agents::{self, Agent};
use crate::ai::mention_list::{self, Suggestion};
use crate::ai::skills::{self, Skill, SourceFolder};
use crate::ai::{self, AiTool};
use crate::document::highlighting::Syntax;
use crate::document::media_view::{MediaEvent, image_pixels, show_image, show_video};
use crate::document::merge::MergeSession;
use crate::document::merge_view::{MergeAction, show_merge};
use crate::document::preview_selection;
use crate::document::text_file::{DiskState, Document, Opened, Unsupported};
use crate::document::text_stats::TextStats;
use crate::document::view::{
    BlockEdit, EditorState, PreviewEvent, apply_with_undo, editor_selection, focus_editor, handle_editor_shortcuts,
    set_editor_cursor, show_editor, show_preview, show_read_only_preview, step_editor_history,
};
use crate::explorer::note_marks::NoteMarks;
use crate::explorer::paths::{absolute_path_text, name_text};
use crate::explorer::reorder::move_item;
use crate::explorer::tree::{FsNode, is_markdown_path};
use crate::explorer::view::{
    ExplorerAction, ExplorerEdits, InlineRename, NewEntry, Revealed, SessionEdit, TreeContext, show_explorer,
};
use crate::format::{Language, format};
use crate::icons::icon_text_button;
use crate::icons::{Icon, icon_button};
use crate::install::view::InstallAction;
use crate::markdown::blocks::{SourceBlock, parse_blocks, parse_source_blocks};
use crate::markdown::html::{file_url, html_page, is_html_path, with_base};
use crate::markdown::html_blocks::html_source_blocks;
use crate::markdown::notes::{self, Note, find_notes, insert_note, remove_note, rewrite_note};
use crate::markdown::render::CodeAction;
use crate::markdown::table_edit::{self, TableAction, TableCell};
use crate::media::{MediaKind, format_name, image_uri, media_kind, open_with_system, size_text};
use crate::scratch::{create_scratch, rename_target};
use crate::settings::accessibility::{TextSize, ZoomGesture};
use crate::settings::view::{
    AiInfo, PrivacyInfo, ScratchInfo, SettingsAction, SettingsGroup, TerminalsInfo, VoiceInfo, show_settings,
};
use crate::settings::{Settings, default_agents_folder, default_scratch_folder, default_skills_folder, themes_folder};
use crate::terminal::agent_sessions::SessionHistory;
use crate::terminal::is_terminal_path;
use crate::terminal::monitor::{TerminalMonitor, in_saved_order};
use crate::terminal::session::LiveTerminal;
use crate::terminal::tmux::Tmux;
use crate::terminal::view::show_terminal;
use crate::theme::catalog::Themes;
use crate::theme::code_colors::install_code_colors;
use crate::theme::style::{apply_accessibility, apply_theme, install_fonts, page_fill};
use crate::voice::view::set_memo_states;
use privacy::AfterPassword;

/// How long a status message ("Copied ...") stays visible.
const STATUS_MESSAGE_SECONDS: f64 = 4.0;
const EXPLORER_DEFAULT_WIDTH: f32 = 260.0;
/// Width of the settings panel.
/// Starting height of the AI output panel.
const AI_OUTPUT_HEIGHT: f32 = 230.0;
/// Space around the "cannot display this file" message.
const UNSUPPORTED_MARGIN: f32 = 16.0;
/// How often the open file is checked for changes made by other programs.
const DISK_CHECK_SECONDS: f64 = 2.0;
/// Width of the note dialog.
const NOTE_DIALOG_WIDTH: f32 = 360.0;
/// Quoted text longer than this is shortened in the note dialog and panel.
const NOTE_QUOTE_CHARS: usize = 80;
/// Default width of the notes panel.
const NOTES_PANEL_WIDTH: f32 = 280.0;
/// How often the `@` suggestions are gathered again, in seconds.
const MENTION_REFRESH_SECONDS: f64 = 2.0;
/// Opacity of the warning color behind a note card.
const NOTE_CARD_TINT: f32 = 0.18;
/// Width of the "Rename" dialog.
const RENAME_DIALOG_WIDTH: f32 = 320.0;
/// Inner margin (x, y) of the status bar.
const BAR_MARGIN: (i8, i8) = (10, 6);
/// Opacity of the warning color behind the "changed on disk" bar.
const NOTICE_TINT: f32 = 0.15;
/// Shown before the file name while it has unsaved changes.
const DIRTY_MARKER: &str = "• ";
/// Height of the hairline between groups of status bar buttons.
const STATUS_SEPARATOR_HEIGHT: f32 = 16.0;

const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const TOGGLE_PREVIEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::E);
const ADD_NOTE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::M);
const FORMAT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F);
const OPEN_IN_BROWSER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::B);
const OPEN_PATH: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const NEW_SCRATCH: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const NEW_ENCRYPTED_SCRATCH: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::N);
const TOGGLE_SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
const ASK: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::K);
const RECORD: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::R);
const COPY_OPEN_PATH: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::C);
const CLOSE_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
const FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
/// Find with the replace row: Cmd+Alt+F on macOS, Ctrl+H elsewhere.
#[cfg(target_os = "macos")]
const FIND_REPLACE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::ALT), Key::F);
#[cfg(not(target_os = "macos"))]
const FIND_REPLACE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::H);
const FIND_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::G);
const FIND_PREVIOUS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G);
const NEXT_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Tab);
const PREVIOUS_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab);
const TEXT_LARGER: [KeyboardShortcut; 2] =
    [KeyboardShortcut::new(Modifiers::COMMAND, Key::Plus), KeyboardShortcut::new(Modifiers::COMMAND, Key::Equals)];
const TEXT_SMALLER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus);
const TEXT_DEFAULT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Num0);
const LOCK: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::L);
const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const REDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
/// Redo as on Windows and Linux.
const REDO_Y: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);

/// What a tab shows (and, for the active tab, the right panel).
#[derive(Debug)]
enum OpenFile {
    Nothing,
    Text {
        document: Document,
        /// Parsed preview, rebuilt lazily after the text changes.
        preview: Option<Vec<SourceBlock>>,
        /// Shown when the file changed or vanished on disk.
        disk_notice: Option<DiskState>,
        /// Cursor and undo history while the tab is hidden.
        editor: Option<EditorState>,
        /// A Markdown or HTML file shows its source instead of the
        /// preview. Every file opens in preview; this lasts only for its tab.
        source: bool,
    },
    /// An image (shown in the page) or a video (shown as a card).
    Media {
        path: PathBuf,
        kind: MediaKind,
        /// File size in bytes.
        bytes: u64,
        /// Modification time when read, to notice a new version.
        modified: Option<std::time::SystemTime>,
        /// An image at its own size (with scroll bars) instead of fitted to the page.
        actual_size: bool,
    },
    Unsupported {
        path: PathBuf,
        reason: Unsupported,
    },
    /// An encrypted scratch waiting for the scratch password.
    Locked {
        path: PathBuf,
    },
    /// A tmux session (from the Terminals list or a `.tmux` file).
    Terminal {
        terminal: Box<LiveTerminal>,
    },
}

impl OpenFile {
    /// Path of the shown file, if any.
    fn path(&self) -> Option<&Path> {
        match self {
            OpenFile::Nothing => None,
            OpenFile::Text { document, .. } => Some(&document.path),
            OpenFile::Media { path, .. } | OpenFile::Unsupported { path, .. } | OpenFile::Locked { path } => Some(path),
            OpenFile::Terminal { terminal } => terminal.file.as_deref(),
        }
    }

    /// The tab's name: the file name, or the session name of a terminal
    /// opened from the Terminals list.
    fn title(&self) -> String {
        match (self, self.path()) {
            (_, Some(path)) => name_text(path),
            (OpenFile::Terminal { terminal }, None) => terminal.name.clone(),
            _ => String::new(),
        }
    }

    /// Follow a rename of the file on disk.
    fn set_path(&mut self, target: &Path) {
        match self {
            OpenFile::Nothing => {}
            OpenFile::Text { document, .. } => document.path = target.to_path_buf(),
            OpenFile::Media { path, .. } | OpenFile::Unsupported { path, .. } | OpenFile::Locked { path } => {
                *path = target.to_path_buf();
            }
            OpenFile::Terminal { terminal } => terminal.follow_rename(target),
        }
    }

    /// True when a text file with unsaved changes is open.
    fn is_dirty(&self) -> bool {
        matches!(self, OpenFile::Text { document, .. } if document.is_dirty())
    }

    /// The file is stored encrypted: open with its key, or waiting for the password.
    fn is_encrypted(&self) -> bool {
        match self {
            OpenFile::Text { document, .. } => document.is_encrypted(),
            OpenFile::Locked { .. } => true,
            _ => false,
        }
    }
}

/// An action that closes files, so it may need the "unsaved changes"
/// question first (once for each file with unsaved changes).
#[derive(Debug)]
enum GuardedAction {
    /// Close the active tab.
    CloseTab,
    /// Take a root out of the explorer (closes the tabs inside it).
    RemoveRoot(PathBuf),
    /// Quit; unsaved edits are kept as drafts, so this only asks when
    /// a draft cannot be written.
    Quit,
}

/// Answer to the "unsaved changes" dialog.
#[derive(Clone, Copy, Debug)]
enum UnsavedChoice {
    Save,
    /// Close the tab without saving; the edits come back when the file
    /// opens again.
    KeepForLater,
    Discard,
    Cancel,
}

/// The whole application: owns all state and draws every panel each frame.
#[derive(Debug)]
pub struct RubriqueApp {
    settings: Settings,
    /// Explorer root folders, in the order they were added. Never empty.
    roots: Vec<FsNode>,
    /// Which files have notes, for the explorer's marker.
    note_marks: NoteMarks,
    /// The active tab's file (see `tabs.rs`).
    open_file: OpenFile,
    /// Every open tab, left to right; the active one's slot holds
    /// `OpenFile::Nothing` while its file is in `open_file`.
    tabs: Vec<OpenFile>,
    /// Index of the active tab in `tabs`.
    active_tab: usize,
    /// Action waiting for the user to answer the unsaved-changes dialog.
    pending: Option<GuardedAction>,
    /// Set once the user agreed to quit, so the next close is not blocked.
    quit_confirmed: bool,
    /// Cmd/Ctrl + wheel and pinch zoom gathered toward the next text size.
    zoom_gesture: ZoomGesture,
    /// Message for the status bar and the egui time (seconds) it was first
    /// shown; `NaN` until the next frame fills it in.
    status_message: Option<(String, f64)>,
    /// Cursor and count stats from the last editor frame.
    stats: Option<TextStats>,
    /// Last title sent to the window, to avoid resending it every frame.
    window_title: String,
    /// Built-in and user themes, read at start and on "Reload themes".
    themes: Themes,
    /// The settings panel is shown on the right.
    settings_open: bool,
    /// The find bar, while it is open.
    find: Option<find::FindBar>,
    /// The group open in the settings window.
    settings_group: SettingsGroup,
    /// Ask bar, AI run and its output panel.
    ask: ask::AskState,
    /// Skills from the skills folders, read when Settings or Ask opens.
    skills: Vec<Skill>,
    /// Every skill source's folder and whether it exists, read with the skills.
    skill_folders: Vec<SourceFolder>,
    /// Agents from the agents folders, read at start and when Settings or Ask opens.
    agents: Vec<Agent>,
    /// Hover summaries in the explorer.
    summaries: summary::SummaryState,
    /// Name suggestions for scratch files.
    naming: naming::NamingState,
    /// Asks the window to redraw, from background threads.
    repaint: Context,
    /// Where each AI tool's program was found, in [`AiTool::ALL`] order.
    /// Checked when Settings or Ask opens and when a command changes.
    ai_found: [Option<PathBuf>; 4],
    /// The "Rename" dialog, while it is open.
    renaming: Option<Renaming>,
    /// The file being renamed in its explorer row (after a double click).
    tree_rename: Option<InlineRename>,
    /// The file being renamed in its tab (after a double click on the tab).
    tab_rename: Option<InlineRename>,
    /// The name field in the Terminals list: a new session, or one being
    /// renamed (after a double click).
    session_edit: Option<InlineRename<SessionEdit>>,
    /// A new file, folder or terminal being named at the top of its folder.
    new_entry: Option<InlineRename<NewEntry>>,
    /// The agent sessions each terminal has run.
    agent_history: SessionHistory,
    /// egui time of the last look for agent sessions.
    last_agent_check: f64,
    /// Terminals whose agent sessions are unfolded in the list.
    expanded_terminals: Vec<String>,
    /// Tab groups folded down to their chip, by folder (an empty path for
    /// the files outside every folder).
    folded_groups: Vec<PathBuf>,
    /// `@tmux-` messages waiting for a busy terminal to be idle.
    pending_sends: Vec<delegate::PendingSend>,
    /// What typing `@` suggests, and when it was gathered (egui time):
    /// gathering reads folders, so it is redone every few seconds only.
    mention_cache: (f64, Vec<Suggestion>),
    /// The tree entry "Show in files" pointed at, highlighted until the next
    /// click in the explorer.
    revealed: Option<PathBuf>,
    /// Scroll the tree to `revealed` on the next frame.
    scroll_to_revealed: bool,
    /// The file the "Delete" dialog asks about, while it is open.
    deleting: Option<PathBuf>,
    /// The "Open path" dialog, while it is open.
    open_path: Option<open_path::OpenPathDialog>,
    /// Watches the tmux sessions for the explorer; `None` when tmux is not installed.
    terminals: Option<TerminalMonitor>,
    /// The merge screen for the open file, while it is open.
    merging: Option<MergeSession>,
    /// egui time of the last check for outside changes.
    last_disk_check: f64,
    /// The preview block being edited as Markdown.
    block_edit: Option<BlockEdit>,
    /// Blocks of the open Markdown file that differ from the saved file.
    block_changes: changes_panel::ChangesCache,
    /// The note being written or edited.
    note_draft: Option<NoteDraft>,
    /// Voice memos: recording, playback, transcription.
    voice: voice::VoiceState,
    /// Installing missing tools.
    install: install::InstallState,
    /// Unsaved edits kept between runs.
    drafts: drafts::DraftState,
    /// Scratch key for this session, PIN lock screen and their dialogs.
    privacy: privacy::PrivacyState,
    /// Remote folders: change checks, image downloads, the add dialog.
    remote: remote::RemoteState,
    /// The open file's Helm chart and the Helm panel.
    helm: helm::HelmState,
}

/// A note in the making (new, or an existing one being edited).
#[derive(Debug)]
struct NoteDraft {
    /// The text the note is about, as shown.
    quote: String,
    /// Where that text is in the source, when known (selection in Edit mode).
    range: Option<std::ops::Range<usize>>,
    /// What the note says.
    text: String,
    /// The note being changed, for Edit.
    existing: Option<Note>,
    /// Why saving failed, shown in the dialog.
    error: Option<String>,
}

/// State of the "Rename" dialog.
#[derive(Debug)]
struct Renaming {
    path: PathBuf,
    /// The name being typed.
    name: String,
    /// Why the last attempt failed, shown under the field.
    error: Option<String>,
}

impl RubriqueApp {
    /// Start with `roots` in the explorer (none on the first run) and, optionally,
    /// `file` open, and install the saved theme.
    pub fn new(ctx: &Context, settings: Settings, roots: Vec<PathBuf>, file: Option<PathBuf>) -> Self {
        install_fonts(ctx);
        let themes = Themes::load(themes_folder().as_deref());
        apply_theme(ctx, &themes, &settings.theme);
        install_code_colors(ctx, settings.code_colors);
        apply_accessibility(ctx, &settings.accessibility);
        // Rubrique steps through its own text sizes instead (see `handle_text_zoom`).
        ctx.options_mut(|options| options.zoom_with_keyboard = false);
        let privacy = privacy::PrivacyState::new(settings.privacy.pin_hash.is_some());
        let mut app = RubriqueApp {
            settings,
            roots: roots.into_iter().map(FsNode::new_root).collect(),
            note_marks: NoteMarks::default(),
            open_file: OpenFile::Nothing,
            tabs: Vec::new(),
            active_tab: 0,
            pending: None,
            quit_confirmed: false,
            zoom_gesture: ZoomGesture::default(),
            status_message: None,
            stats: None,
            window_title: String::new(),
            themes,
            settings_open: false,
            find: None,
            settings_group: SettingsGroup::default(),
            ask: ask::AskState::default(),
            skills: Vec::new(),
            skill_folders: Vec::new(),
            agents: Vec::new(),
            summaries: summary::SummaryState::default(),
            naming: naming::NamingState::default(),
            repaint: ctx.clone(),
            ai_found: [None, None, None, None],
            renaming: None,
            tree_rename: None,
            tab_rename: None,
            session_edit: None,
            new_entry: None,
            pending_sends: Vec::new(),
            folded_groups: Vec::new(),
            agent_history: agent_history::load_history(),
            last_agent_check: f64::NEG_INFINITY,
            expanded_terminals: Vec::new(),
            mention_cache: (f64::NEG_INFINITY, Vec::new()),
            revealed: None,
            scroll_to_revealed: false,
            deleting: None,
            open_path: None,
            terminals: Tmux::find().map(|tmux| TerminalMonitor::start(tmux, ctx)),
            merging: None,
            last_disk_check: 0.0,
            block_edit: None,
            block_changes: changes_panel::ChangesCache::default(),
            note_draft: None,
            voice: voice::VoiceState::default(),
            install: install::InstallState::default(),
            drafts: drafts::DraftState::default(),
            privacy,
            remote: remote::RemoteState::default(),
            helm: helm::HelmState::default(),
        };
        preview_selection::install(ctx);
        egui_extras::install_image_loaders(ctx);
        app.remember_roots();
        // The editor suggests agents when typing `@ai-`, so read them now
        // rather than waiting for Settings or Ask to open.
        app.load_skills_and_agents();
        // Reopen last session's tabs (with their unsaved edits), then the file asked for.
        let saved_tabs = std::mem::take(&mut app.settings.open_tabs);
        app.reopen_tabs(saved_tabs);
        app.sort_tabs_into_groups();
        if let Some(file) = file {
            app.open_in_tab(file);
        }
        app
    }

    // ---------- actions ----------

    /// Run an action, asking about unsaved changes first where it closes them.
    fn request(&mut self, action: GuardedAction) {
        self.finish_block_edit();
        self.step(action);
    }

    /// Run `action`, or, while a file it would close has unsaved changes,
    /// show that file and ask about it. The answer runs `step` again, so
    /// each unsaved file is asked about in turn.
    fn step(&mut self, action: GuardedAction) {
        match action {
            GuardedAction::CloseTab if self.open_file.is_dirty() => self.pending = Some(action),
            GuardedAction::CloseTab => self.close_active_tab_now(),
            GuardedAction::RemoveRoot(folder) => match self.dirty_tab(|path| path.starts_with(&folder)) {
                Some(index) => {
                    self.switch_to_tab(index);
                    self.pending = Some(GuardedAction::RemoveRoot(folder));
                }
                None => self.remove_root_now(&folder),
            },
            GuardedAction::Quit => {
                // Asking is the fallback for when the edits cannot be kept.
                if !self.keep_drafts()
                    && let Some(index) = self.dirty_tab(|_| true)
                {
                    self.switch_to_tab(index);
                    self.pending = Some(GuardedAction::Quit);
                } else {
                    self.quit_confirmed = true;
                    self.repaint.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
    }

    /// Read `path` for a tab: an image or video, a text file, or one that
    /// cannot be shown. `None` (with a status message) when it cannot be read.
    fn load_file(&mut self, path: PathBuf) -> Option<OpenFile> {
        if is_terminal_path(&path) {
            return self.load_terminal(&path);
        }
        if let Some(kind) = media_kind(&path) {
            // A remote image or video is shown from its mirror copy.
            if let Err(error) = crate::vfs::fetch_to_mirror(&path, remote::MAX_MEDIA_BYTES) {
                self.show_status(format!("Cannot open {}: {error}", crate::remote::display(&path)));
                return None;
            }
            if crate::remote::is_mirror(&path) {
                self.repaint.forget_image(&image_uri(&path));
            }
            return match std::fs::metadata(&path) {
                Ok(metadata) => Some(OpenFile::Media {
                    path,
                    kind,
                    bytes: metadata.len(),
                    modified: metadata.modified().ok(),
                    actual_size: false,
                }),
                Err(error) => {
                    self.show_status(format!("Cannot open {}: {error}", crate::remote::display(&path)));
                    None
                }
            };
        }
        match Document::open(&path, self.privacy.vault.as_ref()) {
            Ok(Opened::Text(document)) => {
                if crate::remote::is_mirror(&document.path) {
                    self.fetch_remote_images(&document.path, &document.text);
                }
                Some(OpenFile::Text { document, preview: None, disk_notice: None, editor: None, source: false })
            }
            Ok(Opened::Unsupported(reason)) => Some(OpenFile::Unsupported { path, reason }),
            Ok(Opened::Locked) => Some(OpenFile::Locked { path }),
            Err(error) => {
                self.show_status(format!("Cannot open {}: {error}", crate::remote::display(&path)));
                None
            }
        }
    }

    /// Read `path` again into the active tab (dropping unsaved changes), or
    /// open it in a tab when none is open.
    fn open_file_now(&mut self, path: PathBuf) {
        if self.tabs.is_empty() {
            self.open_in_tab(path);
            return;
        }
        self.stats = None;
        self.merging = None;
        self.block_edit = None;
        self.note_draft = None;
        self.voice.stop_playback();
        crate::document::view::restore_editor_state(&self.repaint, None);
        if let Some(file) = self.load_file(path) {
            self.open_file = file;
        }
    }

    /// Add `path` as one more explorer root, unless it is already one.
    fn add_folder(&mut self, path: PathBuf) {
        if self.roots.iter().any(|root| root.path == path) {
            self.show_status(format!("Already in the explorer: {}", crate::remote::display(&path)));
            return;
        }
        self.roots.push(FsNode::new_root(path));
        self.remember_roots();
        self.sort_tabs_into_groups();
    }

    /// Ask to remove a root; unsaved files inside it need the question.
    fn request_remove_root(&mut self, path: PathBuf) {
        if self.roots.len() > 1 {
            self.request(GuardedAction::RemoveRoot(path));
        }
    }

    /// Move root `from` to go before root `to` (dragged in the explorer),
    /// and its tab group with it.
    fn move_root(&mut self, from: usize, to: usize) {
        move_item(&mut self.roots, from, to);
        self.remember_roots();
        self.sort_tabs_into_groups();
    }

    /// Take a root out of the explorer, keeping at least one, and close the
    /// tabs inside it.
    fn remove_root_now(&mut self, path: &Path) {
        if self.roots.len() <= 1 {
            return;
        }
        self.roots.retain(|root| root.path != path);
        self.close_tabs_inside(path);
        self.remember_roots();
        self.sort_tabs_into_groups();
    }

    /// Save the open document; returns false when saving failed.
    fn save(&mut self) -> bool {
        self.finish_block_edit();
        let OpenFile::Text { document, disk_notice, .. } = &mut self.open_file else {
            return true;
        };
        match document.save() {
            Ok(()) => {
                *disk_notice = None;
                let message = format!("Saved {}", crate::remote::display(&document.path));
                self.show_status(message);
                self.maybe_suggest_scratch_name();
                true
            }
            Err(error) => {
                let message = format!("Save failed: {error}");
                self.show_status(message);
                false
            }
        }
    }

    /// Show a short-lived message in the status bar.
    fn show_status(&mut self, message: String) {
        // The time is filled in on the next frame (see `status_bar`).
        self.status_message = Some((message, f64::NAN));
    }

    /// Put `text` on the clipboard and confirm it in the status bar.
    fn copy_to_clipboard(&mut self, ctx: &Context, text: String) {
        self.show_status(format!("Copied {text}"));
        ctx.copy_text(text);
    }

    /// Switch a Markdown or HTML file between editor and preview (no-op for
    /// other files).
    fn toggle_preview(&mut self) {
        self.finish_block_edit();
        let has_preview = self.open_file.path().is_some_and(|path| is_markdown_path(path) || is_html_path(path));
        if has_preview && let OpenFile::Text { source, .. } = &mut self.open_file {
            *source = !*source;
        }
    }

    /// True when the right panel shows the Markdown preview, where blocks
    /// are edited in place.
    fn showing_preview(&self) -> bool {
        matches!(self.open_file, OpenFile::Text { source: false, .. })
            && self.open_file.path().is_some_and(is_markdown_path)
    }

    /// True when the right panel shows the read-only preview of an HTML file.
    fn showing_html_preview(&self) -> bool {
        matches!(self.open_file, OpenFile::Text { source: false, .. })
            && self.open_file.path().is_some_and(is_html_path)
    }

    /// True when the right panel shows either preview instead of the editor.
    fn showing_any_preview(&self) -> bool {
        self.showing_preview() || self.showing_html_preview()
    }

    /// When the window regains focus, look for changes made by other programs.
    fn check_disk(&mut self) {
        // Remote files: a new version of an image shows when it is opened
        // again; a text file is checked in the background.
        if let OpenFile::Media { path, .. } = &self.open_file
            && crate::remote::is_mirror(path)
        {
            return;
        }
        if let OpenFile::Text { document, .. } = &self.open_file
            && crate::remote::is_mirror(&document.path)
        {
            let path = document.path.clone();
            self.start_remote_check(&path);
            return;
        }
        if let OpenFile::Media { path, kind, bytes, modified, .. } = &mut self.open_file {
            // A new version of an image is shown at once; nothing is ever unsaved here.
            let metadata = std::fs::metadata(&*path).ok();
            let now = metadata.as_ref().and_then(|metadata| metadata.modified().ok());
            if now.is_some() && now != *modified {
                *modified = now;
                *bytes = metadata.map_or(*bytes, |metadata| metadata.len());
                if *kind == MediaKind::Image {
                    self.repaint.forget_image(&image_uri(path));
                }
            }
            return;
        }
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        let state = document.disk_state();
        self.apply_disk_state(&state);
    }

    /// Act on what changed on disk for the open text file: nothing, a
    /// notice, or a silent reload when nothing of ours would be lost.
    fn apply_disk_state(&mut self, state: &DiskState) {
        let OpenFile::Text { document, disk_notice, .. } = &mut self.open_file else {
            return;
        };
        match state {
            DiskState::Unchanged => {}
            DiskState::Deleted => *disk_notice = Some(DiskState::Deleted),
            DiskState::Modified => {
                let Ok(disk_text) = document.read_disk_text() else {
                    *disk_notice = Some(DiskState::Modified);
                    return;
                };
                // Only the time changed, or the disk already holds my text: nothing to tell.
                if disk_text == document.base_text() || disk_text == document.text {
                    document.accept_disk_text(disk_text);
                    *disk_notice = None;
                } else if document.is_dirty() {
                    *disk_notice = Some(DiskState::Modified);
                } else {
                    // Nothing of ours to lose: pick up the new version and say so.
                    let path = document.path.clone();
                    self.open_file_now(path.clone());
                    self.show_status(format!("Reloaded {}: it was changed by another program", name_text(&path)));
                }
            }
        }
    }

    /// Check for outside changes every few seconds while a file is open.
    fn poll_disk(&mut self, ctx: &Context) {
        if !matches!(self.open_file, OpenFile::Text { .. } | OpenFile::Media { .. }) || self.merging.is_some() {
            return;
        }
        let now = ctx.input(|input| input.time);
        if now - self.last_disk_check >= DISK_CHECK_SECONDS {
            self.last_disk_check = now;
            self.check_disk();
        }
        ctx.request_repaint_after(Duration::from_secs_f64(DISK_CHECK_SECONDS));
    }

    // ---------- per-frame input ----------

    /// React to focus changes and to the window's close button.
    fn handle_window_events(&mut self, ctx: &Context) {
        let regained_focus =
            ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(true))));
        if regained_focus {
            self.check_disk();
        }

        if ctx.input(|input| input.viewport().close_requested()) {
            if self.quit_confirmed || self.keep_drafts() || self.dirty_tab(|_| true).is_none() {
                self.pending = None;
                self.save_settings(ctx);
            } else {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                if self.pending.is_none() {
                    self.request(GuardedAction::Quit);
                }
            }
        }
    }

    /// Global shortcuts; they run before the panels so widgets never see them.
    fn handle_shortcuts(&mut self, ctx: &Context) {
        self.handle_tab_shortcuts(ctx);
        self.handle_text_zoom(ctx);
        if ctx.input_mut(|input| input.consume_shortcut(&COPY_OPEN_PATH))
            && let Some(path) = self.open_file.path()
        {
            let text = absolute_path_text(path);
            self.copy_to_clipboard(ctx, text);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&LOCK)) {
            if self.settings.privacy.pin_hash.is_some() {
                self.lock_app();
            } else {
                self.show_status("Set a PIN in Settings → Privacy to lock the window".to_string());
            }
        }
        if ctx.input_mut(|input| input.consume_shortcut(&TOGGLE_SETTINGS)) {
            self.toggle_settings();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&ASK)) {
            self.check_ai_tools();
            self.load_skills_and_agents();
            self.open_ask(ctx);
        }
        // Shortcuts with more modifiers first: egui lets Cmd+Shift+F and
        // Cmd+Alt+F also match Cmd+F.
        if ctx.input_mut(|input| input.consume_shortcut(&FORMAT)) {
            self.format_text(ctx);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&FIND_REPLACE)) {
            self.open_find(ctx, true);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&FIND)) {
            self.open_find(ctx, false);
        }
        self.find_shortcuts(ctx);
        self.side_panel_shortcut(ctx);
        // After the Structure shortcut: Cmd+O alone would also match Cmd+Shift+O.
        if ctx.input_mut(|input| input.consume_shortcut(&OPEN_PATH)) {
            self.start_open_path();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&ADD_NOTE)) {
            self.start_note(ctx);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&OPEN_IN_BROWSER)) {
            self.open_in_browser(ctx);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&RECORD)) {
            self.toggle_recording(ctx);
        }
        // The longer shortcut first: Cmd+N alone would also match Cmd+Shift+N.
        if ctx.input_mut(|input| input.consume_shortcut(&NEW_ENCRYPTED_SCRATCH)) {
            self.new_scratch(ctx, true);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&NEW_SCRATCH)) {
            self.new_scratch(ctx, self.encrypt_new_scratches());
        }
        if self.settings_open
            && self.pending.is_none()
            && self.renaming.is_none()
            && self.tree_rename.is_none()
            && self.session_edit.is_none()
            && self.new_entry.is_none()
            && self.tab_rename.is_none()
            && self.deleting.is_none()
            && self.open_path.is_none()
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.settings_open = false;
        }
        if ctx.input_mut(|input| input.consume_shortcut(&SAVE)) {
            self.save();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&TOGGLE_PREVIEW)) {
            self.toggle_preview();
            if !self.showing_any_preview() {
                focus_editor(ctx);
            }
        }
        if let OpenFile::Text { document, preview, .. } = &mut self.open_file
            && handle_editor_shortcuts(ctx, document)
        {
            *preview = None;
        }
        self.handle_undo_keys(ctx);
        self.prepare_find(ctx);
    }

    /// Cmd+Z and Shift+Cmd+Z (or Cmd+Y) when no text box takes the keys, as
    /// in the preview: step back or forward through the editor's undo
    /// history, so a deleted block comes back and can be deleted again.
    /// A text box with focus (the editor, a block being edited) keeps its own Undo.
    fn handle_undo_keys(&mut self, ctx: &Context) {
        if !matches!(self.open_file, OpenFile::Text { .. })
            || ctx.egui_wants_keyboard_input()
            || self.merging.is_some()
            || self.settings_open
        {
            return;
        }
        // Redo first: Cmd+Z alone would also match Shift+Cmd+Z.
        let redo = ctx.input_mut(|input| input.consume_shortcut(&REDO) || input.consume_shortcut(&REDO_Y));
        if !redo && !ctx.input_mut(|input| input.consume_shortcut(&UNDO)) {
            return;
        }
        // A block edit still open is put in first, so it is the step undone.
        self.finish_block_edit();
        let OpenFile::Text { document, preview, .. } = &mut self.open_file else {
            return;
        };
        if step_editor_history(ctx, &mut document.text, redo) {
            document.on_text_changed();
            *preview = None;
        } else {
            let what = if redo { "redo" } else { "undo" };
            self.show_status(format!("Nothing to {what}"));
        }
    }

    /// Cmd/Ctrl + plus, minus and 0: the next text size, the one before, the
    /// default. Cmd/Ctrl + mouse wheel and trackpad pinch step the same sizes.
    fn handle_text_zoom(&mut self, ctx: &Context) {
        let current = self.settings.accessibility.text_size;
        let size = if ctx.input_mut(|input| input.consume_shortcut(&TEXT_DEFAULT)) {
            TextSize::Default
        } else if ctx.input_mut(|input| TEXT_LARGER.iter().any(|keys| input.consume_shortcut(keys))) {
            current.larger()
        } else if ctx.input_mut(|input| input.consume_shortcut(&TEXT_SMALLER)) {
            current.smaller()
        } else {
            // egui turns the wheel into zoom while Cmd/Ctrl is down (and
            // leaves the scroll delta at zero, so nothing scrolls), and folds
            // a pinch into the same factor.
            self.zoom_gesture.step(current, ctx.input(egui::InputState::zoom_delta))
        };
        if size != current {
            self.settings.accessibility.text_size = size;
            apply_accessibility(ctx, &self.settings.accessibility);
            self.show_status(format!("Text size: {} ({}%)", size.name(), (size.scale() * 100.0).round()));
        }
    }

    /// The theme and code colors, then the accessibility settings on top of them.
    fn apply_look(&self, ctx: &Context) {
        apply_theme(ctx, &self.themes, &self.settings.theme);
        install_code_colors(ctx, self.settings.code_colors);
        apply_accessibility(ctx, &self.settings.accessibility);
    }

    /// Remember the window size and the open tabs, then write the settings file.
    fn save_settings(&mut self, ctx: &Context) {
        if let Some(rect) = ctx.input(|input| input.viewport().inner_rect) {
            // In points of the native scale, as the window was opened with, not zoomed ones.
            let zoom = ctx.zoom_factor();
            self.settings.window_size = Some((rect.width() * zoom, rect.height() * zoom));
        }
        self.remember_tabs();
        self.write_settings();
    }

    /// Keep the settings' tab list and active file in step with the tabs.
    fn remember_tabs(&mut self) {
        self.settings.open_tabs = self.tab_paths();
        self.settings.last_file = self.open_file.path().map(Path::to_path_buf);
    }

    /// Write the settings file. A failure is only reported on stderr: the
    /// window is closing and there is nowhere else to show it.
    fn write_settings(&self) {
        if let Err(error) = self.settings.save() {
            eprintln!("rubrique: could not save settings: {error}");
        }
    }

    /// "name - Rubrique", with the dirty marker; sent only when it changes.
    fn update_title(&mut self, ctx: &Context) {
        let name = if self.app_locked() { None } else { self.open_file.path().map(name_text) };
        let dirty = if self.open_file.is_dirty() { DIRTY_MARKER } else { "" };
        let title = match name {
            Some(name) => format!("{dirty}{name} - Rubrique"),
            None => "Rubrique".to_string(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    // ---------- panels ----------

    fn toggle_settings(&mut self) {
        self.settings_open = !self.settings_open;
        if self.settings_open {
            self.check_ai_tools();
            self.load_skills_and_agents();
            self.check_voice_tool();
            self.check_terminals();
        }
    }

    /// Read the skills and agents from their folders.
    fn load_skills_and_agents(&mut self) {
        self.skills = skills::load_skills(&self.settings.ai.skill_folders(default_skills_folder().as_deref()));
        self.skill_folders =
            skills::source_folders(self.settings.ai.all_skill_folders(default_skills_folder().as_deref()));
        self.agents = agents::load_agents(&self.settings.ai.agent_folders(default_agents_folder()));
    }

    /// A click in Settings → Skills.
    fn skills_settings_action(&mut self, action: &SettingsAction, skills_folder: Option<&Path>) {
        match action {
            SettingsAction::ChooseSkillsFolder => {
                let mut dialog = rfd::FileDialog::new().set_title("Folder for skills");
                if let Some(current) = skills_folder {
                    dialog = dialog.set_directory(current);
                }
                if let Some(chosen) = dialog.pick_folder() {
                    self.settings.ai.skills_folder = Some(chosen);
                    self.load_skills_and_agents();
                }
            }
            SettingsAction::ResetSkillsFolder => {
                self.settings.ai.skills_folder = None;
                self.load_skills_and_agents();
            }
            SettingsAction::ReloadSkills => {
                self.load_skills_and_agents();
                let count = self.skills.len();
                self.show_status(format!("Found {count} skills"));
            }
            SettingsAction::CreateExampleSkill => match skills_folder.map(skills::create_example) {
                Some(Ok(path)) => {
                    self.load_skills_and_agents();
                    self.show_status(format!("Example skill: {}", path.display()));
                }
                Some(Err(error)) => self.show_status(format!("Cannot create the example skill: {error}")),
                None => {}
            },
            _ => {}
        }
    }

    /// A click in Settings → Agents.
    fn agents_settings_action(&mut self, action: &SettingsAction, agents_folder: Option<&Path>) {
        match action {
            SettingsAction::ChooseAgentsFolder => {
                let mut dialog = rfd::FileDialog::new().set_title("Folder for agents");
                if let Some(current) = agents_folder {
                    dialog = dialog.set_directory(current);
                }
                if let Some(chosen) = dialog.pick_folder() {
                    self.settings.ai.agents_folder = Some(chosen);
                    self.load_skills_and_agents();
                }
            }
            SettingsAction::ResetAgentsFolder => {
                self.settings.ai.agents_folder = None;
                self.load_skills_and_agents();
            }
            SettingsAction::ReloadAgents => {
                self.load_skills_and_agents();
                let count = self.agents.len();
                self.show_status(format!("Found {count} agents"));
            }
            SettingsAction::CreateExampleAgent => match agents_folder.map(agents::create_example) {
                Some(Ok(path)) => {
                    self.load_skills_and_agents();
                    self.show_status(format!("Example agent: {}", path.display()));
                }
                Some(Err(error)) => self.show_status(format!("Cannot create the example agent: {error}")),
                None => {}
            },
            _ => {}
        }
    }

    /// Look for each AI tool's program on `PATH`.
    fn check_ai_tools(&mut self) {
        self.ai_found = AiTool::ALL.map(|tool| ai::find_program(self.settings.ai.command(tool)));
        self.find_ai_recipes();
    }

    /// The settings window over the dimmed app. A click outside closes it.
    fn settings_window(&mut self, ctx: &Context) {
        if !self.settings_open {
            return;
        }
        let visuals = ctx.global_style().visuals.clone();
        let frame = egui::Frame::new()
            .fill(visuals.window_fill)
            .corner_radius(12)
            .shadow(visuals.window_shadow)
            .stroke(visuals.window_stroke);
        let response = egui::Modal::new(egui::Id::unique("settings-window")).frame(frame).show(ctx, |ui| {
            self.settings_panel(ui);
        });
        if response.should_close() {
            self.settings_open = false;
        }
    }

    /// The settings window's content. Theme picks apply at once.
    fn settings_panel(&mut self, ui: &mut Ui) {
        let folder = themes_folder();
        let scratch_folder = self.settings.scratch_folder_or_default();
        let install = self.install_status();
        let scratch = ScratchInfo {
            folder: scratch_folder.as_deref(),
            is_default: self.settings.scratch_folder.is_none() || scratch_folder == default_scratch_folder(),
            remote: &mut self.settings.remote,
        };
        let skills_folder = self.settings.ai.skills_folder.clone().or_else(default_skills_folder);
        let agents_folder = self.settings.ai.agents_folder.clone().or_else(default_agents_folder);
        let ai = AiInfo {
            settings: &mut self.settings.ai,
            found: &self.ai_found,
            skills: &self.skills,
            skill_folders: &self.skill_folders,
            agents: &self.agents,
            agents_folder: agents_folder.as_deref(),
            recipes: &self.install.ai,
            local_model: self.install.local_model.as_ref(),
            install: &install,
        };
        let privacy = PrivacyInfo { settings: &mut self.settings.privacy, unlocked: self.privacy.vault.is_some() };
        let voice = VoiceInfo {
            settings: &mut self.settings.voice,
            found: self.voice.program_found.as_deref(),
            recipe: self.install.voice.as_ref(),
            model_recipe: self.install.voice_model.as_ref(),
            install: &install,
        };
        let tmux = self.terminals.as_ref().map(|monitor| monitor.tmux().program());
        let terminals = TerminalsInfo { found: tmux, recipe: self.install.tmux.as_ref(), install: &install };
        let action = show_settings(
            ui,
            &mut self.settings_group,
            &self.themes,
            &self.settings.theme,
            self.settings.code_colors,
            folder.as_deref(),
            &mut self.settings.accessibility,
            scratch,
            &mut self.settings.note_marks,
            privacy,
            ai,
            voice,
            terminals,
        );
        let Some(action) = action else {
            return;
        };
        match action {
            SettingsAction::PickTheme(_) | SettingsAction::PickCodeColors(_) | SettingsAction::ReloadThemes => {
                self.appearance_settings_action(ui.ctx(), &action, folder.as_deref());
            }
            SettingsAction::AccessibilityChanged => self.apply_look(ui.ctx()),
            SettingsAction::ChooseScratchFolder => {
                let mut dialog = rfd::FileDialog::new().set_title("Folder for scratch files");
                if let Some(current) = &scratch_folder {
                    dialog = dialog.set_directory(current);
                }
                if let Some(chosen) = dialog.pick_folder() {
                    self.settings.scratch_folder = Some(chosen);
                }
            }
            SettingsAction::ResetScratchFolder => self.settings.scratch_folder = None,
            SettingsAction::RemoteChanged => self.remote_settings_changed(),
            SettingsAction::AiCommandChanged => self.check_ai_tools(),
            SettingsAction::ForgetAllowedFolders => {
                self.settings.ai.allowed_folders.clear();
                self.show_status("Rubrique will ask again before the next AI run in each folder".to_string());
            }
            SettingsAction::ChooseSkillsFolder
            | SettingsAction::ResetSkillsFolder
            | SettingsAction::ReloadSkills
            | SettingsAction::CreateExampleSkill => self.skills_settings_action(&action, skills_folder.as_deref()),
            SettingsAction::ChooseAgentsFolder
            | SettingsAction::ResetAgentsFolder
            | SettingsAction::ReloadAgents
            | SettingsAction::CreateExampleAgent => self.agents_settings_action(&action, agents_folder.as_deref()),
            SettingsAction::OpenFile(path) => {
                self.settings_open = false;
                self.open_in_tab(path);
            }
            SettingsAction::ShowFolder(folder) => {
                if let Err(error) = crate::media::open_with_system(&folder) {
                    self.show_status(error);
                }
            }
            SettingsAction::VoiceCommandChanged => self.check_voice_tool(),
            SettingsAction::CheckTerminals => self.check_terminals_again(),
            SettingsAction::Copy(text) => self.install_action(ui.ctx(), InstallAction::Copy(text)),
            SettingsAction::Install(action) => self.install_action(ui.ctx(), action),
            SettingsAction::Close => self.settings_open = false,
            // The Privacy buttons (privacy.rs).
            _ => self.privacy_action(&action),
        }
    }

    /// The Appearance group's actions: pick a theme or code colors (applied
    /// at once), or read the themes folder again.
    fn appearance_settings_action(&mut self, ctx: &Context, action: &SettingsAction, themes_folder: Option<&Path>) {
        match action {
            SettingsAction::PickTheme(choice) => self.settings.theme = choice.clone(),
            SettingsAction::PickCodeColors(choice) => self.settings.code_colors = *choice,
            SettingsAction::ReloadThemes => {
                self.themes = Themes::load(themes_folder);
                let count = self.themes.list.len();
                self.show_status(format!("Loaded {count} themes"));
            }
            _ => return,
        }
        self.apply_look(ctx);
    }

    /// Show the open Markdown (rendered, in the current theme's colors) or
    /// HTML file in the system browser, from a temporary copy so unsaved
    /// changes show too.
    fn open_in_browser(&mut self, ctx: &Context) {
        self.finish_block_edit();
        if matches!(&self.open_file, OpenFile::Text { document, .. } if document.is_encrypted()) {
            // The browser needs a plain copy on disk.
            self.show_status("Encrypted scratches do not open in the browser".to_string());
            return;
        }
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        let path = document.path.clone();
        let folder = path.parent().map(|folder| format!("{}/", file_url(folder))).unwrap_or_default();
        let page = if is_markdown_path(&path) {
            let palette = self.current_palette(ctx);
            html_page(&name_text(&path), &parse_blocks(&document.text), &palette, Some(&folder))
        } else if is_html_path(&path) {
            with_base(&document.text, &folder)
        } else {
            self.show_status("Open in browser works for Markdown and HTML files".to_string());
            return;
        };
        match write_browser_copy(&path, &page) {
            Ok(copy) => {
                ctx.open_url(egui::OpenUrl::new_tab(file_url(&copy)));
                self.show_status(format!("Opened {} in the browser", name_text(&path)));
            }
            Err(error) => self.show_status(format!("Cannot write the browser copy: {error}")),
        }
    }

    /// Pretty-print the selection (its language guessed from the text unless
    /// the file says), or the whole JSON / SQL / XML file.
    fn format_text(&mut self, ctx: &Context) {
        self.finish_block_edit();
        let selection = if self.showing_preview() { None } else { editor_selection(ctx) };
        let OpenFile::Text { document, preview, .. } = &mut self.open_file else {
            return;
        };
        let file_language = Language::from_path(&document.path);
        let (start, end) = match selection {
            Some((first, last)) => (char_to_byte(&document.text, first), char_to_byte(&document.text, last)),
            None if file_language.is_some() => (0, document.text.len()),
            None => {
                self.show_status(
                    "Select JSON, SQL or XML text to format, or open a .json, .sql or .xml file".to_string(),
                );
                return;
            }
        };
        let original = &document.text[start..end];
        let language = file_language.unwrap_or_else(|| Language::guess(original));
        let mut formatted = match format(language, original) {
            Ok(formatted) => formatted,
            Err(error) => {
                let place = if selection.is_some() { " (in the selection)" } else { "" };
                let message = format!("Cannot format as {}{place}: {error}", language.name());
                self.show_status(message);
                return;
            }
        };
        // A selection keeps its own ending: no new line is added after it.
        if selection.is_some() && !original.ends_with('\n') {
            formatted.truncate(formatted.trim_end_matches('\n').len());
        }
        if formatted == original {
            self.show_status(format!("Already formatted as {}", language.name()));
            return;
        }
        document.text.replace_range(start..end, &formatted);
        document.on_text_changed();
        *preview = None;
        let cursor = document.text[..start].chars().count();
        set_editor_cursor(ctx, cursor);
        self.show_status(format!("Formatted as {}", language.name()));
    }

    /// Copy or Prettify on a code block in the preview. Prettify rewrites the
    /// code inside block `index` in the file.
    fn code_block_action(&mut self, ctx: &Context, index: usize, action: CodeAction) {
        let (info, code) = match action {
            CodeAction::Copy(code) => {
                let lines = code.lines().count();
                ctx.copy_text(code);
                let noun = if lines == 1 { "line" } else { "lines" };
                self.show_status(format!("Copied the code ({lines} {noun})"));
                return;
            }
            CodeAction::Prettify { info, code } => (info, code),
        };
        let Some(language) = Language::from_code_info(&info) else {
            return;
        };
        let formatted = match format(language, &code) {
            Ok(formatted) => formatted.trim_end_matches('\n').to_string(),
            Err(error) => {
                self.show_status(format!("Cannot prettify this {}: {error}", language.name()));
                return;
            }
        };
        if formatted == code {
            self.show_status(format!("This {} is already tidy", language.name()));
            return;
        }
        let OpenFile::Text { document, preview, .. } = &mut self.open_file else {
            return;
        };
        let blocks = parse_source_blocks(&document.text);
        // The code as written between the fences; indented code (inside a
        // list) does not match line for line, so it is left to Format.
        let found = blocks.get(index).and_then(|block| {
            let start = block.range.start + document.text[block.range.clone()].find(code.as_str())?;
            Some(start..start + code.len())
        });
        let Some(range) = found else {
            self.show_status(
                "Prettify works on code blocks that are not indented; use Format in Edit mode".to_string(),
            );
            return;
        };
        replace_with_undo(ctx, document, preview, range, &formatted);
        self.show_status(format!("Prettified the {} (save to keep it)", language.name()));
    }

    /// Colors of the theme on screen now (for System, the light or dark one in use).
    fn current_palette(&self, ctx: &Context) -> crate::theme::palette::Palette {
        if let crate::theme::catalog::ThemeChoice::Named(id) = &self.settings.theme
            && let Some(theme) = self.themes.find(id)
        {
            return theme.palette;
        }
        let base = match ctx.theme() {
            egui::Theme::Dark => crate::theme::palette::Base::Dark,
            egui::Theme::Light => crate::theme::palette::Base::Light,
        };
        self.themes.for_base(base).palette
    }

    /// Create a scratch file, encrypted or plain, show its folder in the
    /// explorer, and open it for typing.
    fn new_scratch(&mut self, ctx: &Context, encrypted: bool) {
        let Some(folder) = self.settings.scratch_folder_or_default() else {
            self.show_status("No folder for scratch files: set one in Settings".to_string());
            return;
        };
        let vault = if encrypted {
            let Some(vault) = self.scratch_vault_or_ask(AfterPassword::NewEncryptedScratch) else {
                // The password dialog opens and makes the scratch after.
                return;
            };
            Some(vault)
        } else {
            None
        };
        let path = match create_scratch(&folder, &crate::date::today_iso(), vault.as_deref()) {
            Ok(path) => path,
            Err(error) => {
                self.show_status(format!("Cannot create a scratch in {}: {error}", folder.display()));
                return;
            }
        };
        if self.roots.iter().any(|root| folder.starts_with(&root.path)) {
            self.roots.iter_mut().for_each(FsNode::refresh);
        } else {
            self.add_folder(folder);
        }
        self.open_in_tab(path);
        // A new scratch is for typing, so it opens on its source.
        if let OpenFile::Text { source, .. } = &mut self.open_file {
            *source = true;
        }
        focus_editor(ctx);
    }

    /// Open the "Rename" dialog for `path`.
    fn start_rename(&mut self, path: PathBuf) {
        let name = name_text(&path);
        self.renaming = Some(Renaming { path, name, error: None });
    }

    /// Rename the file on disk and follow it if it is the open one.
    /// Returns an error message to show in the dialog.
    fn finish_rename(&mut self, path: &Path, new_name: &str) -> Result<(), String> {
        let target = rename_target(path, new_name)?;
        if target == path {
            return Ok(());
        }
        crate::vfs::rename(path, &target).map_err(|error| format!("Cannot rename: {error}"))?;
        let uri = image_uri(path);
        // A renamed folder moves every open file inside it too.
        let moved = |old: &Path| -> PathBuf {
            match old.strip_prefix(path) {
                Ok(rest) if rest.as_os_str().is_empty() => target.clone(),
                Ok(rest) => target.join(rest),
                Err(_) => old.to_path_buf(),
            }
        };
        for tab in self.all_tabs_mut() {
            if let Some(old) = tab.path().filter(|old| old.starts_with(path)) {
                let new = moved(old);
                tab.set_path(&new);
            }
        }
        self.repaint.forget_image(&uri);
        if let Some(last) = self.settings.last_file.as_deref().filter(|last| last.starts_with(path)) {
            self.settings.last_file = Some(moved(last));
        }
        self.roots.iter_mut().for_each(FsNode::refresh);
        self.show_status(format!("Renamed to {}", name_text(&target)));
        Ok(())
    }

    /// Rename the file whose tree row was just submitted; on failure the
    /// row stays a field and shows why.
    fn finish_tree_rename(&mut self) {
        if let Some(rename) = self.tree_rename.take_if(|rename| rename.submitted) {
            self.tree_rename = self.try_inline_rename(rename);
        }
    }

    /// Rename the file whose tab was just submitted; on failure the tab
    /// stays a field and the status bar says why (a tab has no room for it).
    fn finish_tab_rename(&mut self) {
        if let Some(rename) = self.tab_rename.take_if(|rename| rename.submitted) {
            self.tab_rename = self.try_inline_rename(rename);
            if let Some(error) = self.tab_rename.as_ref().and_then(|rename| rename.error.clone()) {
                self.show_status(error);
            }
        }
    }

    /// What typing `@` suggests (agents and terminals), gathered again
    /// when older than [`MENTION_REFRESH_SECONDS`].
    fn mentions(&mut self, ctx: &Context) -> Vec<Suggestion> {
        let now = ctx.input(|input| input.time);
        if now - self.mention_cache.0 > MENTION_REFRESH_SECONDS {
            self.mention_cache = (now, self.mention_suggestions());
        }
        self.mention_cache.1.clone()
    }

    /// Close every in-place name field, so only the one about to open shows.
    fn clear_name_fields(&mut self) {
        self.tree_rename = None;
        self.tab_rename = None;
        self.new_entry = None;
        self.session_edit = None;
    }

    /// Rename a file from an in-place field. Returns the field back, with
    /// the error, when renaming failed.
    fn try_inline_rename(&mut self, mut rename: InlineRename) -> Option<InlineRename> {
        let error = self.finish_rename(&rename.target, &rename.name).err()?;
        rename.failed(error);
        Some(rename)
    }

    /// Draw the "Rename" dialog while it is open.
    fn rename_dialog(&mut self, ctx: &Context) {
        let waiting = self.naming.dialog_waiting();
        let Some(renaming) = &mut self.renaming else {
            return;
        };
        let modal = egui::Modal::new(egui::Id::unique("rename-file")).show(ctx, |ui| {
            ui.set_width(RENAME_DIALOG_WIDTH);
            ui.heading("Rename");
            ui.add_space(4.0);
            let field = ui.add(egui::TextEdit::singleline(&mut renaming.name).desired_width(f32::INFINITY));
            if !field.has_focus() && renaming.error.is_none() && !field.lost_focus() {
                field.request_focus();
            }
            let entered = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            if let Some(error) = &renaming.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let rename = ui.button("Rename").clicked() || entered;
                let cancel = ui.button("Cancel").clicked();
                ui.separator();
                let suggest = if waiting {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(RichText::new("Thinking of a name…").weak());
                    false
                } else {
                    let tip = "Ask the Summaries use tool (Settings → AI) for a name from the content";
                    icon_text_button(ui, Icon::Sparkle, "Suggest", tip, false).clicked()
                };
                (rename, cancel, suggest)
            })
            .inner
        });
        let (rename, cancel, suggest) = modal.inner;
        if suggest {
            let path = renaming.path.clone();
            let open_text = match &self.open_file {
                OpenFile::Text { document, .. } if document.path == path => Some(document.text.clone()),
                _ => None,
            };
            let text = open_text.or_else(|| std::fs::read_to_string(&path).ok()).unwrap_or_default();
            if let Err(error) = self.start_naming(path, &text, true)
                && let Some(renaming) = &mut self.renaming
            {
                renaming.error = Some(error);
            }
            return;
        }
        if cancel || modal.should_close() {
            self.renaming = None;
            return;
        }
        if rename {
            let path = renaming.path.clone();
            let name = renaming.name.clone();
            match self.finish_rename(&path, &name) {
                Ok(()) => self.renaming = None,
                Err(error) => {
                    if let Some(renaming) = &mut self.renaming {
                        renaming.error = Some(error);
                    }
                }
            }
        }
    }

    /// Let the user pick a folder in the system dialog and add it as a root.
    fn choose_folder_to_add(&mut self) {
        let start = self.roots.first().and_then(|root| root.path.parent().map(Path::to_path_buf));
        let mut dialog = rfd::FileDialog::new().set_title("Add folder to Rubrique");
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        if let Some(path) = dialog.pick_folder() {
            self.add_folder(path);
        }
    }

    /// Left panel: one file tree per root folder.
    fn explorer(&mut self, ui: &mut Ui) {
        let open_path = self.open_file.path().map(Path::to_path_buf);
        let dirty_files: Vec<PathBuf> = std::iter::once(&self.open_file)
            .chain(&self.tabs)
            .filter(|file| file.is_dirty())
            .filter_map(|file| file.path().map(Path::to_path_buf))
            .collect();
        let terminal_rows =
            self.terminals.as_ref().map(|monitor| in_saved_order(monitor.rows(), &self.settings.terminal_order));
        let scratch_folder = self.settings.scratch_folder_or_default();
        let context = TreeContext {
            root: Path::new(""),
            can_remove_roots: self.roots.len() > 1,
            markdown_only: self.settings.markdown_only,
            open_file: open_path.as_deref(),
            dirty_files: &dirty_files,
            note_marks: self.settings.note_marks.then_some(&self.note_marks),
            remote_enabled: self.settings.remote.enabled,
            terminals: terminal_rows.as_deref(),
            open_terminal: match &self.open_file {
                OpenFile::Terminal { terminal } => Some(terminal.name.as_str()),
                _ => None,
            },
            revealed: self.revealed.as_deref().map(|path| Revealed { path, scroll: self.scroll_to_revealed }),
            root_color: None,
            agent_history: Some(&self.agent_history),
            expanded_terminals: &self.expanded_terminals,
            scratch_folder: scratch_folder.as_deref(),
        };
        let action = show_explorer(
            ui,
            &mut self.roots,
            &mut self.settings.layout,
            &mut self.settings.markdown_only,
            &context,
            &mut ExplorerEdits {
                rename: &mut self.tree_rename,
                new_entry: &mut self.new_entry,
                session: &mut self.session_edit,
            },
        );
        self.scroll_to_revealed = false;
        self.finish_tree_rename();
        self.finish_session_edit();
        self.finish_new_entry();
        // Any click in the explorer ends the "Show in files" highlight.
        if action.is_some() {
            self.revealed = None;
        }
        match action {
            // Opens a tab, or switches to the file's tab when it is open.
            Some(ExplorerAction::OpenFile(path)) => self.open_in_tab(path),
            Some(ExplorerAction::Copy(text)) => self.copy_to_clipboard(ui.ctx(), text),
            Some(ExplorerAction::Refresh) => {
                self.roots.iter_mut().for_each(FsNode::refresh);
                self.note_marks.clear();
                if let Some(monitor) = &self.terminals {
                    monitor.refresh();
                }
            }
            Some(ExplorerAction::OpenTerminal(name)) => self.open_terminal_session(&name),
            Some(ExplorerAction::NewTerminal) => self.request_new_terminal(),
            Some(ExplorerAction::New(folder, kind)) => self.start_new_entry(folder, kind),
            Some(ExplorerAction::ShowTerminalInFiles(name)) => self.show_terminal_in_files(&name),
            Some(ExplorerAction::RenameTerminal(name)) => self.start_terminal_rename(name),
            Some(ExplorerAction::ExitTerminal(name)) => self.exit_terminal(&name),
            Some(ExplorerAction::ToggleAgentSessions(name)) => self.toggle_agent_sessions(name),
            Some(ExplorerAction::ResumeAgentSession { terminal, id, new_terminal }) => {
                self.resume_agent_session(&terminal, &id, new_terminal);
            }
            Some(ExplorerAction::RemoveRoot(path)) => self.request_remove_root(path),
            Some(ExplorerAction::MoveRoot { from, to }) => self.move_root(from, to),
            Some(ExplorerAction::MoveTerminal { from, to }) => {
                // The new order is the list as it was drawn, with the move.
                let mut names: Vec<String> = terminal_rows.iter().flatten().map(|row| row.info.name.clone()).collect();
                move_item(&mut names, from, to);
                self.settings.terminal_order = names;
            }
            Some(ExplorerAction::AddFolder) => self.choose_folder_to_add(),
            Some(ExplorerAction::OpenPath) => self.start_open_path(),
            Some(ExplorerAction::AddRemoteFolder) => self.open_add_remote(),
            Some(ExplorerAction::NewScratch { encrypted }) => {
                let encrypted = encrypted.unwrap_or_else(|| self.encrypt_new_scratches());
                self.new_scratch(ui.ctx(), encrypted);
            }
            Some(ExplorerAction::Rename(path)) => self.start_rename(path),
            Some(ExplorerAction::SetEncrypted { path, encrypt }) => self.set_scratch_encrypted(&path, encrypt),
            Some(ExplorerAction::RenameInTree(path)) => {
                self.clear_name_fields();
                self.tree_rename = Some(InlineRename::new(path));
            }
            Some(ExplorerAction::Delete(path)) => self.request_delete(path),
            None => {}
        }
    }

    /// Right panel: the tabs, then the editor, preview, image, video, or
    /// why the file cannot be shown.
    fn document_area(&mut self, ui: &mut Ui) {
        if !self.tabs.is_empty() {
            self.tab_bar(ui);
            self.finish_tab_rename();
        }
        self.ask.page_rect = Some(ui.available_rect_before_wrap());
        let page = ui.available_rect_before_wrap();
        if self.merging.is_none() {
            self.recording_bar(ui);
        }
        let showing_preview = self.showing_preview();
        let showing_html_preview = self.showing_html_preview();
        let mentions = self.mentions(ui.ctx());
        self.refresh_block_changes();
        let mut notice_choice = None;
        let mut name_choice = None;
        let mut merge_action = None;
        let mut preview_event = None;
        let mut media_event = None;
        if let OpenFile::Locked { path } = &self.open_file {
            let path = path.clone();
            self.locked_tab(ui, &path);
            return;
        }
        match &mut self.open_file {
            OpenFile::Locked { .. } => {}
            OpenFile::Terminal { terminal } => show_terminal(ui, terminal),
            OpenFile::Nothing => {
                ui.centered_and_justified(|ui| ui.label(RichText::new("Pick a file in the explorer").weak()));
            }
            OpenFile::Media { path, kind: MediaKind::Image, actual_size, .. } => {
                media_event = show_image(ui, path, *actual_size);
            }
            OpenFile::Media { path, kind: MediaKind::Video, bytes, .. } => {
                media_event = show_video(ui, path, *bytes);
            }
            OpenFile::Unsupported { path, reason } => unsupported_notice(ui, path, reason),
            OpenFile::Text { document, preview, disk_notice, .. } => {
                if let Some(session) = &mut self.merging {
                    merge_action = show_merge(ui, session, &name_text(&document.path));
                } else if let Some(state) = disk_notice {
                    notice_choice = disk_notice_bar(ui, state);
                }
                let offer = self.naming.offer.as_ref().filter(|(path, _)| *path == document.path);
                if self.merging.is_none()
                    && let Some((_, name)) = offer
                {
                    name_choice = naming::name_offer_bar(ui, name);
                }
                if self.merging.is_some() {
                    // The merge screen fills the page.
                } else if showing_preview {
                    let blocks = preview.get_or_insert_with(|| parse_source_blocks(&document.text));
                    let folder = document.path.parent().unwrap_or(Path::new("/"));
                    set_memo_states(ui.ctx(), self.voice.memo_states(folder, blocks));
                    let output = show_preview(
                        ui,
                        &document.path,
                        blocks,
                        self.block_edit.as_mut(),
                        &mentions,
                        &self.block_changes.list,
                    );
                    preview_event = output.event;
                    self.voice.preview_hover = output.hovered;
                } else if showing_html_preview {
                    let blocks = preview.get_or_insert_with(|| html_source_blocks(&document.text));
                    preview_event = show_read_only_preview(ui, &document.path, blocks);
                } else {
                    let was_dirty = document.is_dirty();
                    let syntax = Syntax::of(&document.path);
                    let line_numbers = self.settings.accessibility.line_numbers;
                    let wrap_lines = self.settings.accessibility.wrap_lines;
                    let find = find::highlight(&mut self.find);
                    if let Some(stats) = show_editor(ui, document, syntax, line_numbers, wrap_lines, find, &mentions) {
                        self.stats = Some(stats);
                    }
                    // Any edit invalidates the cached preview.
                    if was_dirty || document.is_dirty() {
                        *preview = None;
                    }
                }
            }
        }
        if let Some(choice) = notice_choice {
            self.apply_notice_choice(choice);
        }
        if let Some(choice) = name_choice {
            self.apply_name_choice(choice);
        }
        if let Some(action) = merge_action {
            self.finish_merge(action);
        }
        if let Some(event) = preview_event {
            self.preview_action(ui.ctx(), event);
        }
        if let Some(event) = media_event {
            self.media_action(ui.ctx(), event);
        }
        self.drop_hint(ui.ctx(), page);
        if showing_preview && self.block_edit.is_none() && self.note_draft.is_none() && !self.ask.is_open() {
            self.add_note_button(ui.ctx());
        }
    }

    /// Act on what the user did in the preview.
    fn preview_action(&mut self, ctx: &Context, event: PreviewEvent) {
        match event {
            PreviewEvent::StartEdit(index) => self.start_block_edit(index),
            PreviewEvent::StartCellEdit { index, cell } => self.start_cell_edit(index, cell),
            PreviewEvent::Table { index, action } => self.table_action(index, action),
            PreviewEvent::StartNewBlock => self.start_new_block(),
            PreviewEvent::FinishEdit => self.finish_block_edit(),
            PreviewEvent::DeleteBlock(index) => self.delete_block(index),
            PreviewEvent::InsertBlock(index) => self.start_insert_block(index),
            PreviewEvent::Code { index, action } => self.code_block_action(ctx, index, action),
            PreviewEvent::Memo(action) => self.memo_action(ctx, action),
            PreviewEvent::PlayVideo(path) => self.play_video(&path),
            PreviewEvent::RevertChange(change) => self.revert_change(&change),
        }
    }

    // ---------- images and videos ----------

    /// Act on a click on an image or video page.
    fn media_action(&mut self, ctx: &Context, event: MediaEvent) {
        let OpenFile::Media { path, actual_size, .. } = &mut self.open_file else {
            return;
        };
        match event {
            MediaEvent::ToggleSize => *actual_size = !*actual_size,
            MediaEvent::PlayInPlayer => {
                let path = path.clone();
                self.play_video(&path);
            }
            MediaEvent::PlayInBrowser => {
                let path = path.clone();
                self.play_video_in_browser(ctx, &path);
            }
        }
    }

    /// Open a video in the system's video player.
    fn play_video(&mut self, path: &Path) {
        match open_with_system(path) {
            Ok(()) => self.show_status(format!("Opened {} in your video player", name_text(path))),
            Err(error) => self.show_status(error),
        }
    }

    /// Open a video in the browser, from a small page that holds it.
    fn play_video_in_browser(&mut self, ctx: &Context, path: &Path) {
        let page = video_page(&name_text(path), &file_url(path));
        match write_browser_copy(path, &page) {
            Ok(copy) => {
                ctx.open_url(egui::OpenUrl::new_tab(file_url(&copy)));
                self.show_status(format!("Opened {} in the browser", name_text(path)));
            }
            Err(error) => self.show_status(format!("Cannot write the browser page: {error}")),
        }
    }

    /// Status bar for an image or video: size, format, and Fit / Actual size.
    fn media_status(&mut self, ui: &mut Ui) {
        let OpenFile::Media { path, kind, bytes, actual_size, .. } = &mut self.open_file else {
            return;
        };
        match kind {
            MediaKind::Image => {
                let label = if *actual_size { "Actual size" } else { "Fit" };
                let tip = if *actual_size { "Fit the image to the page" } else { "Show the image at its own size" };
                if ui.small_button(label).on_hover_text(tip).clicked() {
                    *actual_size = !*actual_size;
                }
                let pixels = image_pixels(ui.ctx(), path)
                    .map(|[width, height]| format!("{width} × {height} px   "))
                    .unwrap_or_default();
                ui.label(format!("{pixels}{}   {}   ", format_name(path), size_text(*bytes)));
            }
            MediaKind::Video => {
                if ui.small_button("Play").on_hover_text("Open in your video player").clicked() {
                    let path = path.clone();
                    self.play_video(&path);
                    return;
                }
                ui.label(format!("{} video   {}   ", format_name(path), size_text(*bytes)));
            }
        }
    }

    // ---------- editing in the preview ----------

    /// Edit the Markdown of top-level block `index` in place of its preview.
    fn start_block_edit(&mut self, index: usize) {
        let Some(range) = self.block_edit_target(index) else {
            return;
        };
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        let Some(text) = document.text.get(range.clone()) else {
            return;
        };
        self.block_edit =
            Some(BlockEdit { index, range, text: text.to_string(), focused: false, cell: None, inserting: false });
    }

    /// Edit one cell of the table at block `index` in place. A cell the
    /// source does not have (the parser pads short rows) edits the whole
    /// table instead.
    fn start_cell_edit(&mut self, index: usize, cell: TableCell) {
        let Some(range) = self.block_edit_target(index) else {
            return;
        };
        self.edit_cell_in(index, range, cell);
    }

    /// Start editing `cell` of the table whose source is at `block`.
    fn edit_cell_in(&mut self, index: usize, block: Range<usize>, cell: TableCell) {
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        let Some(source) = document.text.get(block.clone()) else {
            return;
        };
        let Some(within) = table_edit::cell_range(source, cell) else {
            self.block_edit = Some(BlockEdit {
                index,
                range: block,
                text: source.to_string(),
                focused: false,
                cell: None,
                inserting: false,
            });
            return;
        };
        let range = block.start + within.start..block.start + within.end;
        let text = document.text.get(range.clone()).unwrap_or_default().to_string();
        self.block_edit = Some(BlockEdit { index, range, text, focused: false, cell: Some(cell), inserting: false });
    }

    /// Add or remove a row or column of the table at block `index`, next to
    /// the cell being edited there. A new row or column is edited next.
    fn table_action(&mut self, index: usize, action: TableAction) {
        let Some(cell) = self.block_edit.as_ref().and_then(|edit| edit.cell) else {
            return;
        };
        let Some(block) = self.block_edit_target(index) else {
            return;
        };
        let OpenFile::Text { document, preview, .. } = &mut self.open_file else {
            return;
        };
        let edited = document.text.get(block.clone()).and_then(|source| table_edit::apply(source, cell, action));
        let Some(edited) = edited else {
            return;
        };
        let new_block = block.start..block.start + edited.source.len();
        replace_with_undo(&self.repaint, document, preview, block, &edited.source);
        if let Some(next) = edited.next {
            self.edit_cell_in(index, new_block, next);
        }
    }

    /// The source range of top-level block `index`, after applying any edit
    /// in progress: its length change moves the blocks after it, and grows
    /// the block a cell edit was inside of.
    fn block_edit_target(&mut self, index: usize) -> Option<Range<usize>> {
        let OpenFile::Text { preview: Some(blocks), .. } = &self.open_file else {
            return None;
        };
        let mut range = blocks.get(index)?.range.clone();
        if let Some(applied) = self.finish_block_edit_with_shift() {
            // `>=`: a block inserted right before this one moves it too.
            if range.start >= applied.at {
                range = shift(range.start, applied.delta)..shift(range.end, applied.delta);
            } else if applied.cell_block == Some(index) {
                range.end = shift(range.end, applied.delta);
            }
        }
        Some(range)
    }

    /// Edit an empty new block just before top-level block `index`. The
    /// blank lines around it are added when the edit is finished with text in it.
    fn start_insert_block(&mut self, index: usize) {
        let Some(block) = self.block_edit_target(index) else {
            return;
        };
        let at = block.start;
        self.block_edit =
            Some(BlockEdit { index, range: at..at, text: String::new(), focused: false, cell: None, inserting: true });
    }

    /// Edit an empty new block at the end of the file. The blank line before
    /// it is added when the edit is finished with text in it.
    fn start_new_block(&mut self) {
        self.finish_block_edit();
        let OpenFile::Text { document, preview: Some(blocks), .. } = &self.open_file else {
            return;
        };
        let end = document.text.len();
        self.block_edit = Some(BlockEdit {
            index: blocks.len(),
            range: end..end,
            text: String::new(),
            focused: false,
            cell: None,
            inserting: true,
        });
    }

    /// Remove top-level block `index` and the blank lines that separated it
    /// from the next block. Typing in that block is dropped with it; an edit
    /// of another block is put in first.
    fn delete_block(&mut self, index: usize) {
        if self.block_edit.as_ref().is_some_and(|edit| edit.index == index && !edit.inserting) {
            self.block_edit = None;
        }
        let Some(block) = self.block_edit_target(index) else {
            return;
        };
        let OpenFile::Text { document, preview, .. } = &mut self.open_file else {
            return;
        };
        let range = deleted_block_range(&document.text, block);
        replace_with_undo(&self.repaint, document, preview, range, "");
        self.show_status("Deleted the block (Cmd+Z brings it back; save to keep it deleted)".to_string());
    }

    /// Put the edited block's Markdown (or cell's text) into the document.
    fn finish_block_edit(&mut self) {
        self.finish_block_edit_with_shift();
    }

    /// Like [`Self::finish_block_edit`]; returns where the text changed and
    /// by how many bytes, so ranges after it can be moved.
    fn finish_block_edit_with_shift(&mut self) -> Option<EditShift> {
        let BlockEdit { index, range, text, cell, .. } = self.block_edit.take()?;
        let OpenFile::Text { document, preview, .. } = &mut self.open_file else {
            return None;
        };
        let new_text = match cell {
            Some(_) => table_edit::escape_cell(&text),
            None if range.is_empty() && range.start == document.text.len() => appended_block(&document.text, &text),
            None if range.is_empty() => inserted_block(&document.text[..range.start], &text),
            None => edited_block(&text).to_string(),
        };
        // A block emptied out stays as it was; a cell may be emptied.
        if cell.is_none() && new_text.is_empty() {
            return None;
        }
        if document.text.get(range.clone()) == Some(new_text.as_str()) {
            return None;
        }
        let old_length = isize::try_from(range.len()).unwrap_or(0);
        let new_length = isize::try_from(new_text.len()).unwrap_or(0);
        replace_with_undo(&self.repaint, document, preview, range.clone(), &new_text);
        Some(EditShift { at: range.start, delta: new_length - old_length, cell_block: cell.map(|_| index) })
    }

    // ---------- notes ----------

    /// Floating "+ Add note" next to text selected in the preview.
    fn add_note_button(&mut self, ctx: &Context) {
        let Some(selection) = preview_selection::current(ctx) else {
            return;
        };
        let area = egui::Area::new(egui::Id::unique("add-note-button"))
            .order(egui::Order::Foreground)
            .fixed_pos(selection.pointer + egui::vec2(10.0, 14.0));
        let shown = area.show(ctx, |ui| {
            ui.button("+ Add note").on_hover_text("Add a note about the selected text (Cmd+Shift+M)").clicked()
        });
        preview_selection::set_button_rect(ctx, shown.response.rect);
        if shown.inner {
            self.start_note(ctx);
        }
    }

    /// Open the note dialog for the selection (preview text, or editor text in Edit mode).
    fn start_note(&mut self, ctx: &Context) {
        self.finish_block_edit();
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        let draft = if self.showing_preview() {
            preview_selection::current(ctx).map(|selection| NoteDraft {
                quote: normalize_space(&selection.text),
                range: None,
                text: String::new(),
                existing: None,
                error: None,
            })
        } else {
            editor_selection(ctx).map(|(first, last)| {
                let range = char_to_byte(&document.text, first)..char_to_byte(&document.text, last);
                let shown: String = document.text[range.clone()].chars().filter(|c| !"*_~`".contains(*c)).collect();
                NoteDraft {
                    quote: normalize_space(&shown),
                    range: Some(range),
                    text: String::new(),
                    existing: None,
                    error: None,
                }
            })
        };
        match draft {
            Some(draft) => self.note_draft = Some(draft),
            None => self.show_status("Select some text first, then add a note".to_string()),
        }
    }

    /// Save the note in the draft into the document. Returns an error to show.
    fn save_note(&mut self, draft: &NoteDraft) -> Result<(), String> {
        let OpenFile::Text { document, preview, .. } = &mut self.open_file else {
            return Ok(());
        };
        if draft.text.trim().is_empty() {
            return Err("Write something first".to_string());
        }
        if let Some(existing) = &draft.existing {
            rewrite_note(&mut document.text, existing, &draft.text);
        } else {
            let range = draft
                .range
                .clone()
                .or_else(|| notes::locate(&document.text, &draft.quote))
                .ok_or("Cannot find this text in the file. Select text inside one paragraph.")?;
            insert_note(&mut document.text, &range, &draft.quote, &draft.text);
        }
        document.on_text_changed();
        *preview = None;
        Ok(())
    }

    /// The dialog for writing a note.
    fn note_dialog(&mut self, ctx: &Context) {
        if self.note_draft.is_none() {
            return;
        }
        let mentions = self.mentions(ctx);
        let Some(draft) = &mut self.note_draft else {
            return;
        };
        let modal = egui::Modal::new(egui::Id::unique("note-dialog")).show(ctx, |ui| {
            ui.set_width(NOTE_DIALOG_WIDTH);
            ui.heading(if draft.existing.is_some() { "Edit note" } else { "Add note" });
            ui.label(
                RichText::new(format!("On \u{201c}{}\u{201d}", shorten(&draft.quote, NOTE_QUOTE_CHARS)))
                    .weak()
                    .italics(),
            );
            ui.add_space(4.0);
            let field_id = egui::Id::unique("note-dialog-field");
            let list_key = mention_list::take_key(ui.ctx(), field_id);
            let output = egui::TextEdit::multiline(&mut draft.text)
                .id(field_id)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("Your note")
                .show(ui);
            mention_list::show(ui, field_id, &output, &mut draft.text, &mentions, list_key);
            let field = output.response;
            if !field.has_focus() && draft.text.is_empty() && draft.error.is_none() {
                field.request_focus();
            }
            if let Some(error) = &draft.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            let save_keys = ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Enter));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let save = ui.button("Save note").on_hover_text("Cmd+Enter").clicked() || save_keys;
                let cancel = ui.button("Cancel").clicked();
                (save, cancel)
            })
            .inner
        });
        let (save, cancel) = modal.inner;
        if cancel || modal.should_close() {
            self.note_draft = None;
            return;
        }
        if save && let Some(draft) = self.note_draft.take() {
            match self.save_note(&draft) {
                Ok(()) => {
                    preview_selection::clear(ctx);
                    self.show_notes_panel();
                    self.show_status(
                        "Note saved in the file as a hidden comment (save the file to keep it)".to_string(),
                    );
                }
                Err(error) => self.note_draft = Some(NoteDraft { error: Some(error), ..draft }),
            }
        }
    }

    /// Right panel in the preview: every note in the file, with Edit and Delete.
    fn notes_panel(&mut self, ui: &mut Ui) {
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        let notes = find_notes(&document.text);
        let mut edit = None;
        let mut delete = None;
        let mut resolve = None;
        let mut stop = false;
        let busy = self.ai_busy();
        let all_state = self.resolving_state(&ask::Resolving::All);
        let note_states: Vec<_> =
            notes.iter().map(|note| self.resolving_state(&ask::Resolving::One(note.quote.clone()))).collect();
        egui::Frame::new().inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            if notes.is_empty() {
                let hint = "No notes in this file. Select text in the preview, then Add note (Cmd+Shift+M).";
                ui.label(RichText::new(hint).weak().size(11.5));
                return;
            }
            ui.horizontal(|ui| {
                let count = if notes.len() == 1 { "1 note".to_string() } else { format!("{} notes", notes.len()) };
                ui.label(RichText::new(count).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        let tip = "Ask AI to resolve every note; the answer opens in merge";
                        if icon_text_button(ui, Icon::Sparkle, "Resolve all", tip, false).clicked() {
                            resolve = Some(ask::Resolving::All);
                        }
                    });
                });
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (index, note) in notes.iter().enumerate() {
                    let tint = ui.visuals().warn_fg_color.gamma_multiply(NOTE_CARD_TINT);
                    egui::Frame::new().fill(tint).corner_radius(8).inner_margin(10).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(format!("\u{201c}{}\u{201d}", shorten(&note.quote, NOTE_QUOTE_CHARS)))
                                .weak()
                                .size(11.5),
                        );
                        ui.label(&note.text);
                        ui.horizontal(|ui| {
                            if let Some((tool, seconds)) = note_states[index].or(all_state) {
                                ui.add(egui::Spinner::new().size(11.0));
                                let working = format!("{} is working · {}", tool.name(), ai::format_elapsed(seconds));
                                ui.label(RichText::new(working).weak().size(11.5));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if icon_text_button(ui, Icon::Stop, "Stop", "Stop the tool", false).clicked() {
                                        stop = true;
                                    }
                                });
                                return;
                            }
                            ui.add_enabled_ui(!busy, |ui| {
                                let tip = "Ask AI to do what the note says; the answer opens in merge";
                                if icon_text_button(ui, Icon::Sparkle, "Resolve", tip, false).clicked() {
                                    resolve = Some(ask::Resolving::One(note.quote.clone()));
                                }
                            });
                            if ui.small_button("Edit").clicked() {
                                edit = Some(index);
                            }
                            if ui.small_button("Delete").clicked() {
                                delete = Some(index);
                            }
                        });
                    });
                    ui.add_space(8.0);
                }
                notes_help(ui, self.settings.ai.default_tool.name());
            });
        });
        if stop {
            self.stop_ai();
        }
        if let Some(which) = resolve {
            self.resolve_notes(ui.ctx(), which);
        }
        if let Some(index) = edit {
            let note = notes[index].clone();
            self.note_draft = Some(NoteDraft {
                quote: note.quote.clone(),
                range: None,
                text: note.text.clone(),
                existing: Some(note),
                error: None,
            });
        }
        if let Some(index) = delete
            && let OpenFile::Text { document, preview, .. } = &mut self.open_file
        {
            remove_note(&mut document.text, &notes[index]);
            document.on_text_changed();
            *preview = None;
            self.show_status("Note deleted (save the file to keep the change)".to_string());
        }
    }

    /// Act on a button of the "changed on disk" bar.
    fn apply_notice_choice(&mut self, choice: NoticeChoice) {
        match choice {
            NoticeChoice::Reload => {
                if let Some(path) = self.open_file.path().map(Path::to_path_buf) {
                    self.open_file_now(path);
                }
            }
            NoticeChoice::Merge => self.start_merge(),
            NoticeChoice::KeepMine => {
                if let OpenFile::Text { document, disk_notice, .. } = &mut self.open_file {
                    document.acknowledge_disk_change();
                    *disk_notice = None;
                }
            }
        }
    }

    /// Open the merge screen: the file on disk against my buffer, from the
    /// text both started from.
    fn start_merge(&mut self) {
        self.finish_block_edit();
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        match document.read_disk_text() {
            Ok(disk_text) => self.merging = Some(MergeSession::new(document.base_text(), &document.text, disk_text)),
            Err(error) => self.show_status(format!("Cannot read the file on disk: {error}")),
        }
    }

    /// Apply or drop the merge result.
    fn finish_merge(&mut self, action: MergeAction) {
        let Some(session) = self.merging.take() else {
            return;
        };
        if action == MergeAction::Cancel {
            return;
        }
        let Some(result) = session.result() else {
            self.merging = Some(session);
            return;
        };
        if let Some(title) = &session.review {
            let transcript = title.starts_with("Transcript");
            if let OpenFile::Text { document, preview, .. } = &mut self.open_file {
                let changed = document.text != result;
                document.text = result;
                document.on_text_changed();
                *preview = None;
                self.ask.job = None;
                let message = match (changed, transcript) {
                    (true, true) => "Transcript added. Save to keep it.",
                    (true, false) => "AI changes applied. Save to keep them.",
                    (false, _) => "Kept the original",
                };
                self.show_status(message.to_string());
            }
            return;
        }
        if let OpenFile::Text { document, preview, disk_notice, .. } = &mut self.open_file {
            document.apply_merge(result, session.disk_text);
            *preview = None;
            *disk_notice = None;
            let message = if document.is_dirty() {
                "Merged. Save to write the result to disk."
            } else {
                "Merged: the result matches the file on disk."
            };
            self.show_status(message.to_string());
        }
    }

    /// Bottom bar: a recent status message, or the open file's path, mode and stats.
    fn status_bar(&mut self, ui: &mut Ui) {
        let now = ui.input(|input| input.time);
        // Always the height of a row of buttons, also with none (a terminal
        // tab, a status message): the bar sets the bottom of the explorer,
        // whose Terminals block is docked there and would jump with it.
        let size = egui::vec2(ui.available_width(), crate::icons::BUTTON_HEIGHT);
        ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if let Some((message, remaining_seconds)) = self.visible_status_message(now) {
                ui.label(message);
                ui.ctx().request_repaint_after(Duration::from_secs_f64(remaining_seconds));
            } else {
                self.document_status(ui);
            }
        });
    }

    /// The status message while it is still on screen, with its remaining
    /// seconds. Clears the message once it has expired.
    fn visible_status_message(&mut self, now: f64) -> Option<(&str, f64)> {
        let (_, shown_at) = self.status_message.as_mut()?;
        if shown_at.is_nan() {
            *shown_at = now;
        }
        let remaining_seconds = STATUS_MESSAGE_SECONDS - (now - *shown_at);
        if remaining_seconds <= 0.0 {
            self.status_message = None;
            return None;
        }
        self.status_message.as_ref().map(|(message, _)| (message.as_str(), remaining_seconds))
    }

    /// Path of the open file on the left; mode and editor stats on the right.
    fn document_status(&mut self, ui: &mut Ui) {
        let Some(path) = self.open_file.path() else {
            return;
        };
        let dirty = if self.open_file.is_dirty() { DIRTY_MARKER } else { "" };
        let path_text = format!("{dirty}{}", crate::remote::display(path));
        let is_text = matches!(self.open_file, OpenFile::Text { .. });
        let showing_preview = self.showing_any_preview();
        let is_markdown = self.open_file.path().is_some_and(is_markdown_path);
        let is_html = self.open_file.path().is_some_and(is_html_path);
        // Right side first, so a long path is shortened instead of overlapping it.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            self.media_status(ui);
            if is_text {
                self.text_file_buttons(ui, is_markdown || is_html, showing_preview);
            }
            let left = egui::Layout::left_to_right(egui::Align::Center);
            ui.allocate_ui_with_layout(ui.available_size(), left, |ui| {
                ui.add(egui::Label::new(path_text.as_str()).truncate()).on_hover_text(path_text.as_str());
            });
        });
    }

    /// The buttons of an open text file, laid out right to left: Ask in the
    /// corner, then what works on this kind of file (browser, Format, Helm),
    /// then the voice memo buttons of a page, each group behind a hairline.
    /// Everything but Ask is an icon; the names and shortcuts are in the tips.
    fn text_file_buttons(&mut self, ui: &mut Ui, is_page: bool, showing_preview: bool) {
        let ask = icon_text_button(
            ui,
            Icon::Sparkle,
            "Ask",
            "Ask AI about the selection, or the whole file (Cmd+K)",
            self.ask.is_open(),
        );
        if ask.clicked() {
            self.check_ai_tools();
            self.load_skills_and_agents();
            self.open_ask(ui.ctx());
        }
        let formattable = self.open_file.path().and_then(Language::from_path);
        if is_page || formattable.is_some() || self.open_chart().is_some() {
            status_separator(ui);
        }
        if is_page {
            let tip = "Open in your web browser (Cmd+Shift+B)";
            if icon_button(ui, Icon::Browser, "Open in browser", tip, false).clicked() {
                self.open_in_browser(ui.ctx());
            }
        }
        if let Some(language) = formattable {
            let tip = format!("Pretty-print this {} (Cmd+Shift+F)", language.name());
            if icon_button(ui, Icon::Format, "Format", &tip, false).clicked() {
                self.format_text(ui.ctx());
            }
        }
        self.helm_button(ui);
        if is_page {
            status_separator(ui);
            self.voice_buttons(ui);
        }
        self.scratch_lock_button(ui);
        if !showing_preview && let Some(stats) = &self.stats {
            ui.label(format!("{}   ", stats.summary()));
        }
        self.changes_status_button(ui);
    }

    /// "Encrypt" on a plain scratch, "Encrypted" (with a menu to undo it)
    /// on an encrypted one. Nothing on other files.
    fn scratch_lock_button(&mut self, ui: &mut Ui) {
        let (Some(encrypted), Some(path)) =
            (self.open_scratch_encrypted(), self.open_file.path().map(Path::to_path_buf))
        else {
            return;
        };
        if !encrypted {
            let tip = "Save this scratch encrypted with the scratch password. AI tools are then off for it";
            if icon_text_button(ui, Icon::Lock, "Encrypt", tip, false).clicked() {
                self.set_scratch_encrypted(&path, true);
            }
            return;
        }
        let tip = "Saved encrypted with the scratch password. AI tools are off for it";
        let response = icon_text_button(ui, Icon::Lock, "Encrypted", tip, true);
        let mut decrypt = false;
        egui::Popup::menu(&response).show(|ui| {
            ui.label(RichText::new("Anyone who can read the file sees its text.").weak());
            decrypt = ui.button("Remove encryption").clicked();
        });
        if decrypt {
            self.set_scratch_encrypted(&path, false);
        }
    }

    /// Ask about unsaved changes while an action is pending, then run it or drop it.
    fn unsaved_dialog(&mut self, ctx: &Context) {
        if self.pending.is_none() {
            return;
        }
        let name = self.open_file.path().map(name_text).unwrap_or_default();
        let Some(choice) = ask_about_unsaved_changes(ctx, &name) else {
            return;
        };
        // The question is answered: whatever happens next, stop asking.
        let Some(action) = self.pending.take() else {
            return;
        };
        let proceed = match choice {
            UnsavedChoice::Save => self.save(),
            UnsavedChoice::Discard => {
                // Back to the file, so the next step sees nothing unsaved here
                // (and the draft goes with the edits).
                if let OpenFile::Text { document, preview, .. } = &mut self.open_file {
                    document.discard_changes();
                    *preview = None;
                }
                true
            }
            // The tab is closed already; any other action goes on with the
            // tabs left (closing this one again would close the next tab).
            UnsavedChoice::KeepForLater => self.close_keeping_edits() && !matches!(action, GuardedAction::CloseTab),
            UnsavedChoice::Cancel => false,
        };
        if proceed {
            self.step(action);
        }
    }
}

/// `value` moved by `delta` bytes.
/// A block typed at the end of the file: a blank line before it unless the
/// file is empty or already ends with one, and a line break after it. Empty
/// input gives an empty string, so nothing is added.
/// The hint under the notes: how to add one and what Resolve does.
/// A hairline between two groups of buttons in the status bar.
fn status_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, STATUS_SEPARATOR_HEIGHT), egui::Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), ui.visuals().widgets.noninteractive.bg_stroke);
}

fn notes_help(ui: &mut Ui, tool: &str) {
    let text = format!(
        "Select text in the preview, then Add note (Cmd+Shift+M). \
         Resolve sends the file and the note to {tool}; the answer opens in merge."
    );
    ui.label(RichText::new(text).weak().size(11.5));
}

/// The Markdown of an edited block without the blank lines Enter leaves
/// before or after it, which belong to no block (the gap to the next block
/// is outside the range being replaced). Empty when only whitespace is left,
/// so a block cleared with a stray Enter stays as it was, like a cleared one.
fn edited_block(block: &str) -> &str {
    let block = block.trim_end();
    // Keep the first line's indentation: it can make an indented code block.
    let Some(text_start) = block.find(|c: char| !c.is_whitespace()) else {
        return "";
    };
    let line_start = block[..text_start].rfind('\n').map_or(0, |newline| newline + 1);
    &block[line_start..]
}

fn appended_block(text: &str, block: &str) -> String {
    let block = block.trim_end();
    if block.trim().is_empty() {
        return String::new();
    }
    let separator = if text.is_empty() || text.ends_with("\n\n") {
        ""
    } else if text.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{separator}{block}\n")
}

/// The text to remove to delete the block at `block` (as in
/// `SourceBlock::range`): the block, its line break, and the blank lines
/// after it, so the next block moves up into its place. The last block
/// takes the blank lines before it instead, so the file still ends right
/// after the block above.
fn deleted_block_range(text: &str, block: Range<usize>) -> Range<usize> {
    let mut end = block.end;
    if text[end..].starts_with("\r\n") {
        end += 2;
    } else if text[end..].starts_with('\n') {
        end += 1;
    }
    while let Some(next) = blank_line_end(text, end) {
        end = next;
    }
    let mut start = block.start;
    if end == text.len() {
        // A block starts at a line start, so text[start - 1] is a line break.
        while start > 0 {
            let line_start = text[..start - 1].rfind('\n').map_or(0, |newline| newline + 1);
            if !text[line_start..start - 1].trim().is_empty() {
                break;
            }
            start = line_start;
        }
    }
    start..end
}

/// End (after its line break) of the line starting at `at`, if that line
/// is blank. Only whole blank lines count: spaces before text can make the
/// next block indented code.
fn blank_line_end(text: &str, at: usize) -> Option<usize> {
    let rest = text.get(at..).filter(|rest| !rest.is_empty())?;
    let line_end = rest.find('\n').map_or(text.len(), |newline| at + newline + 1);
    text[at..line_end].trim().is_empty().then_some(line_end)
}

/// The text to put at a block's start (`before` is everything ahead of it)
/// to insert `block` there: the block and a blank line after it, plus a
/// blank line before it when the text ahead does not end with one. Empty
/// when only whitespace was typed.
fn inserted_block(before: &str, block: &str) -> String {
    let block = edited_block(block);
    if block.is_empty() {
        return String::new();
    }
    let separator = if before.is_empty() || before.ends_with("\n\n") || before.ends_with("\n\r\n") {
        ""
    } else if before.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{separator}{block}\n\n")
}

/// Replace `range` of the document's text with `new_text` as one step of
/// the editor's undo history, so Cmd+Z (in the preview too) puts back what
/// was there. The cursor goes to the end of the new text.
fn replace_with_undo(
    ctx: &Context,
    document: &mut Document,
    preview: &mut Option<Vec<SourceBlock>>,
    range: Range<usize>,
    new_text: &str,
) {
    let cursor = document.text[..range.start].chars().count() + new_text.chars().count();
    apply_with_undo(ctx, &mut document.text, cursor, |text| text.replace_range(range, new_text));
    document.on_text_changed();
    *preview = None;
}

fn shift(value: usize, delta: isize) -> usize {
    value.saturating_add_signed(delta)
}

/// Where a finished block edit changed the text, for moving ranges after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EditShift {
    /// Byte offset of the change.
    at: usize,
    /// Bytes added (negative: removed).
    delta: isize,
    /// For a cell edit, the block the change was inside of; a block edit
    /// replaces its block.
    cell_block: Option<usize>,
}

/// Runs of whitespace as single spaces.
fn normalize_space(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// At most `limit` characters, with "…" when cut.
fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut short: String = text.chars().take(limit).collect();
    short.push('…');
    short
}

/// Byte index of character `index` in `text` (the end when past it).
fn char_to_byte(text: &str, index: usize) -> usize {
    text.char_indices().nth(index).map_or(text.len(), |(offset, _)| offset)
}

/// The page for a file that cannot be shown: its name and why.
fn unsupported_notice(ui: &mut Ui, path: &Path, reason: &Unsupported) {
    egui::Frame::new().inner_margin(UNSUPPORTED_MARGIN).show(ui, |ui| {
        ui.heading(name_text(path));
        ui.label(format!("Cannot display this file. {}", reason.describe()));
    });
}

/// Write `page` to a temporary file for the browser, one per source file, and
/// return its path.
fn write_browser_copy(source: &Path, page: &str) -> std::io::Result<PathBuf> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let folder = std::env::temp_dir().join("rubrique-browser");
    std::fs::create_dir_all(&folder)?;
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    let stem = source.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let copy = folder.join(format!("{stem}-{:08x}.html", hasher.finish() & 0xffff_ffff));
    std::fs::write(&copy, page)?;
    Ok(copy)
}

/// A page that plays one video, for the browser.
fn video_page(title: &str, video_url: &str) -> String {
    use crate::markdown::html::escape;
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{}</title>\n\
         <style>html,body{{margin:0;height:100%;background:#111;}}\
         video{{display:block;width:100%;height:100%;object-fit:contain;}}</style></head>\n\
         <body><video controls autoplay src=\"{}\"></video></body></html>\n",
        escape(title),
        escape(video_url)
    )
}

/// Buttons on the "changed on disk" bar.
#[derive(Clone, Copy, Debug)]
enum NoticeChoice {
    /// Open the merge screen.
    Merge,
    /// Load the version on disk, dropping my changes.
    Reload,
    /// Keep my version; the next save overwrites the one on disk.
    KeepMine,
}

/// Draw the "changed on disk" bar; returns the button the user clicked.
fn disk_notice_bar(ui: &mut Ui, state: &DiskState) -> Option<NoticeChoice> {
    let tint = ui.visuals().warn_fg_color.gamma_multiply(NOTICE_TINT);
    egui::Frame::new()
        .fill(tint)
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| disk_notice_row(ui, state)).inner
        })
        .inner
}

/// Message and buttons inside the "changed on disk" bar.
fn disk_notice_row(ui: &mut Ui, state: &DiskState) -> Option<NoticeChoice> {
    let mut choice = None;
    if *state == DiskState::Deleted {
        ui.label("This file was deleted on disk. Saving will recreate it.");
        if ui.button("Keep mine").clicked() {
            choice = Some(NoticeChoice::KeepMine);
        }
        return choice;
    }
    ui.label("Changed on disk by another program.").on_hover_text("You can keep working and decide later.");
    if ui.button("Compare and merge…").on_hover_text("See both versions side by side and combine them").clicked() {
        choice = Some(NoticeChoice::Merge);
    }
    if ui.button("Reload theirs").on_hover_text("Load the version on disk and drop your unsaved changes").clicked() {
        choice = Some(NoticeChoice::Reload);
    }
    if ui.button("Keep mine").on_hover_text("Keep your version; saving overwrites the one on disk").clicked() {
        choice = Some(NoticeChoice::KeepMine);
    }
    choice
}

/// Draw the "unsaved changes" modal for the file `name`. Returns the
/// answer once the user gives one; closing the modal counts as Cancel.
fn ask_about_unsaved_changes(ctx: &Context, name: &str) -> Option<UnsavedChoice> {
    let modal = egui::Modal::new(egui::Id::unique("unsaved-changes")).show(ctx, |ui| {
        ui.heading("Unsaved changes");
        ui.label(format!("Save changes to {name}?"));
        ui.add_space(8.0);
        ui.horizontal(unsaved_choice_buttons).inner
    });
    let closed = modal.should_close();
    modal.inner.or(closed.then_some(UnsavedChoice::Cancel))
}

/// The Save / Keep for later / Discard / Cancel buttons; returns the one
/// clicked this frame.
fn unsaved_choice_buttons(ui: &mut Ui) -> Option<UnsavedChoice> {
    let choices = [
        ("Save", "Save the file, then close it", UnsavedChoice::Save),
        (
            "Keep for later",
            "Close without saving; the edits come back when you open the file again",
            UnsavedChoice::KeepForLater,
        ),
        ("Discard", "Drop the edits and close the file", UnsavedChoice::Discard),
        ("Cancel", "Leave the file open", UnsavedChoice::Cancel),
    ];
    let mut choice = None;
    for (title, tip, option) in choices {
        if ui.button(title).on_hover_text(tip).clicked() {
            choice = Some(option);
        }
    }
    choice
}

impl eframe::App for RubriqueApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_window_events(&ctx);
        self.track_activity(&ctx);
        if self.app_locked() {
            // Only the PIN screen; drafts and background work go on.
            self.poll_drafts(&ctx);
            self.poll_voice(&ctx);
            self.poll_install(&ctx);
            self.lock_screen(ui);
            self.update_title(&ctx);
            return;
        }
        self.handle_shortcuts(&ctx);
        self.poll_disk(&ctx);
        self.poll_drafts(&ctx);
        self.poll_naming();
        self.poll_voice(&ctx);
        self.poll_install(&ctx);
        self.poll_remote();
        self.poll_pending_sends();
        self.poll_agent_sessions(&ctx);

        let style = ctx.global_style();
        let bar_frame =
            egui::Frame::side_top_panel(&style).inner_margin(egui::Margin::symmetric(BAR_MARGIN.0, BAR_MARGIN.1));
        // The page fills the whole central area; editor and preview add their own margins.
        let page_frame = egui::Frame::central_panel(&style).fill(page_fill(&style.visuals)).inner_margin(0);

        egui::Panel::bottom("status-bar").frame(bar_frame).show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("explorer").resizable(true).default_size(EXPLORER_DEFAULT_WIDTH).show(ui, |ui| {
            // Settings sit at the bottom of the explorer, out of the way.
            egui::Panel::bottom("explorer-footer").show_separator_line(false).show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let gear = icon_button(ui, Icon::Gear, "Settings", "Settings (Cmd+,)", self.settings_open);
                    if gear.clicked() {
                        self.toggle_settings();
                    }
                });
                ui.add_space(2.0);
            });
            self.explorer(ui);
        });
        self.right_panel(ui);
        if self.ask.job.is_some() && self.merging.is_none() {
            egui::Panel::bottom("ai-output")
                .resizable(true)
                .default_size(AI_OUTPUT_HEIGHT)
                .frame(egui::Frame::side_top_panel(&style).inner_margin(0))
                .show(ui, |ui| self.ai_output_panel(ui));
        }
        if self.helm.panel.is_some() && self.merging.is_none() {
            egui::Panel::bottom("helm-output")
                .resizable(true)
                .default_size(AI_OUTPUT_HEIGHT)
                .frame(egui::Frame::side_top_panel(&style).inner_margin(0))
                .show(ui, |ui| self.helm_panel(ui));
        }
        egui::CentralPanel::default().frame(page_frame).show(ui, |ui| self.document_area(ui));
        self.handle_dropped_files(&ctx);
        self.find_bar(&ctx);

        self.ask_dialogs(&ctx);
        self.install_dialog(&ctx);
        self.hover_summary(&ctx);
        self.settings_window(&ctx);
        self.unsaved_dialog(&ctx);
        self.note_dialog(&ctx);
        self.rename_dialog(&ctx);
        self.delete_dialog(&ctx);
        self.open_path_dialog(&ctx);
        self.add_remote_dialog(&ctx);
        self.secret_dialog(&ctx);
        self.update_title(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Usually done when the window asked to close; this covers other exits.
        self.keep_drafts();
        self.remember_tabs();
        self.write_settings();
    }
}

#[cfg(test)]
mod tests {
    use super::{appended_block, deleted_block_range, edited_block, inserted_block};
    use crate::markdown::blocks::parse_source_blocks;

    /// `text` with its top-level block `index` deleted.
    fn delete(text: &str, index: usize) -> String {
        let block = parse_source_blocks(text)[index].range.clone();
        let mut text = text.to_string();
        text.replace_range(deleted_block_range(&text, block), "");
        text
    }

    #[test]
    fn deleting_a_block_takes_the_blank_lines_after_it() {
        let text = "# One\n\nTwo\n\n\n- three\n- four\n";
        assert_eq!(delete(text, 0), "Two\n\n\n- three\n- four\n");
        assert_eq!(delete(text, 1), "# One\n\n- three\n- four\n");
        assert_eq!(delete("One\r\n\r\nTwo\r\n", 0), "Two\r\n");
        // Spaces before the next block's text are kept: they make it code.
        assert_eq!(delete("One\n\n    code\n", 0), "    code\n");
    }

    #[test]
    fn inserted_block_gets_blank_lines_around_it() {
        let text = "One\n\nTwo\n";
        let at = parse_source_blocks(text)[1].range.start;
        let inserted = inserted_block(&text[..at], "\nNew\n");
        assert_eq!(format!("{}{inserted}{}", &text[..at], &text[at..]), "One\n\nNew\n\nTwo\n");
        // A block right under a heading, with no blank line between them.
        assert_eq!(inserted_block("# Title\n", "New"), "\nNew\n\n");
        assert_eq!(inserted_block("", "New"), "New\n\n");
        assert_eq!(inserted_block("One\n\n", " \n"), "");
    }

    #[test]
    fn deleting_the_last_block_takes_the_blank_lines_before_it() {
        assert_eq!(delete("One\n\nTwo\n", 1), "One\n");
        assert_eq!(delete("One\n\n\nTwo", 1), "One\n");
        assert_eq!(delete("Only\n", 0), "");
        assert_eq!(delete("\n\nOnly\n\n", 0), "");
    }

    #[test]
    fn edited_block_drops_blank_lines_around_it() {
        assert_eq!(edited_block("Text"), "Text");
        assert_eq!(edited_block("Text\n"), "Text");
        assert_eq!(edited_block("\n\nText\n\n"), "Text");
        assert_eq!(edited_block("One\n\nTwo\n"), "One\n\nTwo");
        assert_eq!(edited_block("\n    code\n"), "    code");
        assert_eq!(edited_block("\n"), "");
        assert_eq!(edited_block("  \n \n"), "");
    }

    #[test]
    fn appended_block_gets_a_blank_line_before_it() {
        assert_eq!(appended_block("", "New"), "New\n");
        assert_eq!(appended_block("Old", "New"), "\n\nNew\n");
        assert_eq!(appended_block("Old\n", "New"), "\nNew\n");
        assert_eq!(appended_block("Old\n\n", "New\n"), "New\n");
        assert_eq!(appended_block("Old\n", "  \n"), "");
    }
}
