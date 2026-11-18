//! Draw the file tree and report clicks and copy requests back to the app.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Key, RichText, Ui};
use egui::text::{CCursor, CCursorRange};

use super::note_marks::NoteMarks;
use super::reorder::{ListDrag, draggable};
use super::section::{paint_fold_arrow, section_header};
use super::selection::{FileRows, FileSelection};
use super::terminals::{show_docked_terminals, show_terminal_file, show_terminals_section};
use crate::icons::{Icon, LOCK_MARK_SIZE, paint_icon, paint_lock_mark, small_icon_button, small_toggle_button};
use crate::scratch::is_scratch;
use crate::settings::explorer_filter::ExplorerFilter;
use crate::settings::layout::LayoutSettings;
use crate::terminal::agent_sessions::SessionHistory;
use crate::terminal::is_terminal_path;
use crate::terminal::monitor::TerminalRow;
use crate::terminal::tmux::session_name_for_file;
use crate::theme::style::group_color;

use super::paths::{absolute_path_text, name_text, relative_path_text};
use super::tree::{Children, FsNode, NodeKind, is_markdown_path};

mod new_file;

use new_file::{NamePart, new_entry_row};

/// Something the user asked for in the explorer.
#[derive(Debug)]
pub enum ExplorerAction {
    /// The user clicked a file.
    OpenFile(PathBuf),
    /// Open a local folder in the system file manager.
    OpenInFileManager(PathBuf),
    /// Put this text on the clipboard.
    Copy(String),
    /// Re-read the folders from disk.
    Refresh,
    /// Take this root folder out of the explorer.
    RemoveRoot(PathBuf),
    /// Ask the system for a folder to add as a root.
    AddFolder,
    /// Show the "Open path" dialog, to open a file or folder by its path.
    OpenPath,
    /// Show the "Clone repository" dialog, to clone a git repository and add it.
    CloneRepository,
    /// Ask for a folder on another computer to add as a root.
    AddRemoteFolder,
    /// Create a new scratch file and open it: encrypted or plain, or
    /// `None` for the "Encrypt every new scratch" setting.
    NewScratch { encrypted: Option<bool> },
    /// Ask for a new name for this file in the "Rename" dialog.
    Rename(PathBuf),
    /// Turn this file's row into a name field (a double click).
    RenameInTree(PathBuf),
    /// Ask for a local destination folder for these files.
    MoveFiles(Vec<PathBuf>),
    /// Ask, then delete the requested files or folder.
    Delete(Vec<PathBuf>),
    /// Store this scratch encrypted (`true`) or as plain text.
    SetEncrypted { path: PathBuf, encrypt: bool },
    /// Show this tmux session in a tab.
    OpenTerminal(String),
    /// Start a new tmux session and show it.
    NewTerminal,
    /// Ask, in a name field at the top of this folder, for the name of a
    /// new file, folder or terminal (`.tmux` file) to create in it.
    New(PathBuf, NewKind),
    /// Open the file tree down to this tmux session's `.tmux` file or folder.
    ShowTerminalInFiles(String),
    /// Turn this session's row in the Terminals list into a name field.
    RenameTerminal(String),
    /// End this tmux session and close its tab.
    ExitTerminal(String),
    /// Show or hide the agent sessions under this terminal in the list.
    ToggleAgentSessions(String),
    /// Resume an agent session: in its terminal, or in a new one.
    ResumeAgentSession { terminal: String, id: String, new_terminal: bool },
    /// A root folder was dragged: move root `from` to go before root `to`
    /// (the root count for the end).
    MoveRoot { from: usize, to: usize },
    /// A terminal was dragged: move row `from` of the Terminals list to go
    /// before row `to` (the row count for the end).
    MoveTerminal { from: usize, to: usize },
}

/// What "New" in a folder's menu creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewKind {
    File,
    Folder,
    /// A `.tmux` file: opening it starts a terminal in its folder.
    Terminal,
}

/// A file, folder or terminal about to be created: the name field shows at
/// the top of `folder` in the tree.
#[derive(Debug, PartialEq, Eq)]
pub struct NewEntry {
    pub folder: PathBuf,
    pub kind: NewKind,
    /// A new file's extension, typed in its own field after the name or
    /// picked from the buttons under it. Not used for the other kinds.
    extension: String,
    /// Which of a new file's two fields is being typed in.
    part: NamePart,
}

impl NewEntry {
    /// A new `kind` to be named in `folder`.
    pub fn new(folder: PathBuf, kind: NewKind) -> Self {
        NewEntry { folder, kind, extension: String::new(), part: NamePart::default() }
    }
}

/// The name field in the Terminals list: for a new tmux session (the +
/// button) or for renaming one (a double click).
#[derive(Debug, PartialEq, Eq)]
pub enum SessionEdit {
    New,
    Rename(String),
}

/// The name fields the explorer can show in place of a row.
#[derive(Debug)]
pub struct ExplorerEdits<'a> {
    /// A file renamed in its tree row.
    pub rename: &'a mut Option<InlineRename>,
    /// A new file, folder or terminal being named in its folder.
    pub new_entry: &'a mut Option<InlineRename<NewEntry>>,
    /// A tmux session being named in the Terminals list.
    pub session: &'a mut Option<InlineRename<SessionEdit>>,
}

/// A name being edited right where it shows: a file in its tree row or its
/// tab (`T` is the file's path), a new entry in its folder (`T` is a
/// [`NewEntry`]), or a terminal in the Terminals list (`T` is a [`SessionEdit`]).
#[derive(Debug)]
pub struct InlineRename<T = PathBuf> {
    /// What is being renamed.
    pub target: T,
    /// The name being typed.
    pub name: String,
    /// Why the last attempt failed, shown under the row.
    pub error: Option<String>,
    /// Enter was pressed or the field was left: the app should rename now.
    pub submitted: bool,
    /// The field still needs focus and its name selected (on the first
    /// frame, and again after a failed attempt).
    needs_focus: bool,
}

impl InlineRename {
    /// Rename the file at `path`, starting from its current name.
    pub fn new(path: PathBuf) -> Self {
        let name = name_text(&path);
        InlineRename::with_name(path, name)
    }
}

impl<T> InlineRename<T> {
    /// A name field for `target`, starting with `name` (the part before
    /// the extension selected).
    pub fn with_name(target: T, name: String) -> Self {
        InlineRename { target, name, error: None, submitted: false, needs_focus: true }
    }

    /// Show `error` under the field and give the field back its focus.
    pub fn failed(&mut self, error: String) {
        self.error = Some(error);
        self.submitted = false;
        self.needs_focus = true;
    }
}

/// The file row under the pointer, for the hover summary.
#[derive(Clone, Debug, PartialEq)]
pub struct HoveredFile {
    pub path: PathBuf,
    /// Where the row is, to place the summary next to it.
    pub rect: egui::Rect,
}

/// Remember the hovered file row for [`hovered_file`].
fn set_hovered(ui: &Ui, file: HoveredFile) {
    ui.ctx().data_mut(|data| data.insert_temp(hovered_id(), file));
}

/// The file row under the pointer this frame, if any.
pub fn hovered_file(ctx: &egui::Context) -> Option<HoveredFile> {
    ctx.data_mut(|data| {
        let file = data.get_temp::<HoveredFile>(hovered_id());
        data.remove::<HoveredFile>(hovered_id());
        file
    })
}

fn hovered_id() -> egui::Id {
    egui::Id::unique("explorer-hovered-file")
}

/// Read-only facts the tree needs while drawing.
#[derive(Clone, Copy, Debug)]
pub struct TreeContext<'a> {
    /// Root folder of the entry being drawn, for "Copy relative path".
    pub root: &'a Path,
    /// More than one root is shown, so a root may be removed.
    pub can_remove_roots: bool,
    /// Which files the tree hides.
    pub filter: ExplorerFilter,
    /// File shown in the editor, highlighted in the tree.
    pub open_file: Option<&'a Path>,
    /// Files with unsaved changes, in any tab: shown in the strong text
    /// color with a bullet after the name.
    pub dirty_files: &'a [PathBuf],
    /// Which files have notes, when the marker is turned on.
    pub note_marks: Option<&'a NoteMarks>,
    /// Remote folders are turned on (shows "Add remote folder").
    pub remote_enabled: bool,
    /// The tmux sessions and their status; `None` when tmux is not
    /// installed (no Terminals list then).
    pub terminals: Option<&'a [TerminalRow]>,
    /// The session shown in the active tab, highlighted in the list.
    pub open_terminal: Option<&'a str>,
    /// The entry "Show in files" pointed at, highlighted until the next
    /// click in the explorer.
    pub revealed: Option<Revealed<'a>>,
    /// The color of the root being drawn, when there are several roots
    /// (its tabs are grouped under the same color).
    pub root_color: Option<egui::Color32>,
    /// The agent sessions each terminal has run.
    pub agent_history: Option<&'a SessionHistory>,
    /// The terminals whose agent sessions are unfolded in the list.
    pub expanded_terminals: &'a [String],
    /// The scratch folder: its files get "Encrypt" or "Remove encryption"
    /// in their right-click menu.
    pub scratch_folder: Option<&'a Path>,
}

/// The tree entry "Show in files" pointed at.
#[derive(Clone, Copy, Debug)]
pub struct Revealed<'a> {
    pub path: &'a Path,
    /// Scroll it into view (only on the frame after it was revealed, so the
    /// tree can be scrolled away from it again).
    pub scroll: bool,
}

/// Draw the explorer panel: the Files section (its header with "Add
/// folder", "New scratch", the Markdown filter and refresh, and one tree
/// per root folder) and the Terminals section docked under it. `layout` holds
/// which sections are folded; the headers change it. `edits` holds the name
/// fields shown in place of rows; Esc in a field clears it.
pub fn show_explorer(
    ui: &mut Ui,
    roots: &mut [FsNode],
    layout: &mut LayoutSettings,
    filter: &mut ExplorerFilter,
    context: &TreeContext,
    edits: &mut ExplorerEdits,
    selection: &mut FileSelection,
) -> Option<ExplorerAction> {
    let mut action = None;
    let files_open = !layout.files_collapsed;
    let mut files = FileRows::new(ui, selection, context.open_file);
    // With Files open, Terminals is a panel docked at the bottom (drawn
    // first so it claims its height); with Files folded it follows the
    // Files header and gets the rest of the height.
    if files_open {
        show_docked_terminals(ui, context, &mut layout.terminals_collapsed, edits.session, &mut action);
    }
    if files_header(ui, roots, context, files_open, filter, &mut action) {
        layout.files_collapsed = files_open;
    }
    if !files_open {
        show_terminals_section(ui, context, &mut layout.terminals_collapsed, edits.session, &mut action);
        return action;
    }
    // The filter button in the header may have just changed the filter.
    let context = TreeContext { filter: *filter, ..*context };
    // Rows never wrap (a wrapped name would put the folder arrow over its
    // text); long names scroll sideways instead.
    egui::ScrollArea::both().id_salt("file-tree").auto_shrink([false, false]).show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        if roots.is_empty() {
            show_no_folders(ui, &mut action);
        }
        // Several roots can be dragged up and down to reorder them.
        let mut drag = ListDrag::default();
        for (index, root) in roots.iter_mut().enumerate() {
            let root_path = root.path.clone();
            let root_color = context.can_remove_roots.then(|| group_color(index));
            let context = TreeContext { root: &root_path, root_color, ..context };
            let top = ui.cursor().top();
            let row = show_root(ui, root, &context, edits, &mut files, &mut action);
            drag.add(ui, top, Some(&row));
        }
        if let Some((from, to)) = drag.finish(ui) {
            action = Some(ExplorerAction::MoveRoot { from, to });
        }
        files.update(ui);
    });
    if let Some(ExplorerAction::Delete(paths) | ExplorerAction::MoveFiles(paths)) = &mut action
        && let [path] = paths.as_slice()
    {
        *paths = files.action_targets(path);
    }
    action.or_else(|| files.keyboard_delete(ui).map(ExplorerAction::Delete))
}

/// The "FILES" header with the folder buttons, the Markdown filter and the
/// collapse / expand all button. Returns true when the title was clicked.
fn files_header(
    ui: &mut Ui,
    roots: &mut [FsNode],
    context: &TreeContext,
    expanded: bool,
    filter: &mut ExplorerFilter,
    action: &mut Option<ExplorerAction>,
) -> bool {
    section_header(
        ui,
        "FILES",
        expanded,
        |_| {},
        |ui| {
            // Right to left: the last button added sits furthest left.
            if small_icon_button(ui, Icon::Refresh, HEADER_BUTTON_SIZE, "Refresh", "Re-read all folders from disk")
                .clicked()
            {
                *action = Some(ExplorerAction::Refresh);
            }
            filter_button(ui, filter);
            let scratch_tip = "New scratch file for quick notes (Cmd+N). Right-click for an encrypted one";
            let scratch = small_icon_button(ui, Icon::NewScratch, HEADER_BUTTON_SIZE, "New scratch", scratch_tip);
            if scratch.clicked() {
                *action = Some(ExplorerAction::NewScratch { encrypted: None });
            }
            scratch.context_menu(|ui| {
                if ui.button("New plain scratch").clicked() {
                    *action = Some(ExplorerAction::NewScratch { encrypted: Some(false) });
                }
                if ui.button("New encrypted scratch (Cmd+Shift+N)").clicked() {
                    *action = Some(ExplorerAction::NewScratch { encrypted: Some(true) });
                }
            });
            let remote_tip = "Add a folder on another computer (SSH)";
            if context.remote_enabled
                && small_icon_button(ui, Icon::ServerPlus, HEADER_BUTTON_SIZE, "Add remote folder", remote_tip)
                    .clicked()
            {
                *action = Some(ExplorerAction::AddRemoteFolder);
            }
            let folder_tip = "Add a folder to the explorer (right click to type a path or clone a repository)";
            let folder = small_icon_button(ui, Icon::FolderPlus, HEADER_BUTTON_SIZE, "Add folder", folder_tip);
            if folder.clicked() {
                *action = Some(ExplorerAction::AddFolder);
            }
            folder.context_menu(|ui| {
                if ui.button("Open path... (Cmd+O)").clicked() {
                    *action = Some(ExplorerAction::OpenPath);
                }
                if ui.button("Clone repository...").clicked() {
                    *action = Some(ExplorerAction::CloneRepository);
                }
            });
            fold_all_button(ui, roots);
        },
    )
}

/// The header button that folds every folder while any root is open, and
/// otherwise opens again the folders that were listed before. It reads
/// nothing from disk; "Expand all" in a folder's menu does.
fn fold_all_button(ui: &mut Ui, roots: &mut [FsNode]) {
    if roots.is_empty() {
        return;
    }
    if roots.iter().any(|root| root.expanded) {
        let tip = "Collapse all folders";
        if small_icon_button(ui, Icon::CollapseAll, HEADER_BUTTON_SIZE, "Collapse all", tip).clicked() {
            roots.iter_mut().for_each(FsNode::collapse_all);
        }
    } else {
        let tip = "Expand all folders opened before. Right-click a folder to expand everything in it";
        if small_icon_button(ui, Icon::ExpandAll, HEADER_BUTTON_SIZE, "Expand all", tip).clicked() {
            roots.iter_mut().for_each(FsNode::expand_loaded);
        }
    }
}

/// The funnel in the Files header: a click opens a menu with one box per
/// filter. The funnel looks pressed while any of them is on, and the menu
/// stays open while boxes are ticked.
fn filter_button(ui: &mut Ui, filter: &mut ExplorerFilter) {
    let mut active = filter.any();
    let tip = if active {
        "Some files are hidden by the filter. Click to change it"
    } else {
        "Showing all files. Click to choose which files to show"
    };
    // The click is read from the response below, to open the menu; the
    // flipped flag is not used.
    let button = small_toggle_button(ui, Icon::Filter, HEADER_BUTTON_SIZE, "Filter files", tip, &mut active);
    egui::Popup::menu(&button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.checkbox(&mut filter.markdown_only, "Markdown files only");
        ui.checkbox(&mut filter.unsaved_only, "Unsaved edits only");
        ui.checkbox(&mut filter.show_closed_terminals, "Show closed terminals");
    });
}

/// True when the filter lets `file` into the tree. Unsaved edits narrow
/// everything else down; a terminal file counts as Markdown for the
/// Markdown filter (terminals are not documents to hide), and is hidden
/// only when "Show closed terminals" is off and its session is not running.
fn file_is_shown(file: &FsNode, context: &TreeContext) -> bool {
    let filter = context.filter;
    if filter.unsaved_only && !context.dirty_files.iter().any(|path| path == &file.path) {
        return false;
    }
    if is_terminal_path(&file.path) {
        return filter.show_closed_terminals || terminal_is_running(file, context);
    }
    !filter.markdown_only || is_markdown_path(&file.path)
}

/// True when a `.tmux` file's session is running.
fn terminal_is_running(file: &FsNode, context: &TreeContext) -> bool {
    let session = session_name_for_file(&file.path);
    context.terminals.is_some_and(|rows| rows.iter().any(|row| row.info.name == session))
}

/// True when the filter lets a folder into the tree: always, except with
/// "Unsaved edits only", where only the folders holding an unsaved file stay.
fn folder_is_shown(folder: &FsNode, context: &TreeContext) -> bool {
    !context.filter.unsaved_only || context.dirty_files.iter().any(|path| path.starts_with(&folder.path))
}

/// True when a folder's entries are drawn under it: when it is expanded,
/// and with "Unsaved edits only" also when it holds an unsaved file, so the
/// file is in sight without opening every folder on the way (the filter
/// already hides the folders that hold none). Reads the folder from disk
/// the first time it is shown that way.
fn shows_contents(ui: &Ui, folder: &mut FsNode, context: &TreeContext) -> bool {
    if folder.expanded {
        return true;
    }
    let holds_unsaved = context.filter.unsaved_only
        && folder.kind == NodeKind::Folder
        && context.dirty_files.iter().any(|path| path.starts_with(&folder.path));
    if holds_unsaved && matches!(folder.children, Children::NotLoaded) {
        folder.start_loading(repaint_when_done(ui));
    }
    holds_unsaved
}

/// Repaints the window when a remote folder's listing arrives.
fn repaint_when_done(ui: &Ui) -> impl FnOnce() + Send + 'static {
    let ctx = ui.ctx().clone();
    move || ctx.request_repaint()
}

/// What the tree shows before any folder is added (the first run): a
/// hint and a button for the same folder picker as the header button.
fn show_no_folders(ui: &mut Ui, action: &mut Option<ExplorerAction>) {
    ui.add_space(8.0);
    ui.weak("No folder open yet.");
    if ui.button("Add folder...").clicked() {
        *action = Some(ExplorerAction::AddFolder);
    }
    if ui.button("Clone repository...").clicked() {
        *action = Some(ExplorerAction::CloneRepository);
    }
}

/// A root folder: a bold row that folds its tree open and closed, and
/// can be dragged up and down when there are several. Returns its row.
fn show_root(
    ui: &mut Ui,
    root: &mut FsNode,
    context: &TreeContext,
    edits: &mut ExplorerEdits,
    files: &mut FileRows,
    action: &mut Option<ExplorerAction>,
) -> egui::Response {
    let label = RichText::new(format!("{ICON_GAP}{}", root.name)).strong();
    let mut response = tree_row(ui, is_revealed(context, &root.path), label);
    if context.can_remove_roots {
        response = draggable(&response);
    }
    let response = response.on_hover_text(absolute_path_text(&root.path));
    files.record_focus(&response);
    scroll_if_revealed(ui, context, &root.path, &response);
    let open = shows_contents(ui, root, context);
    paint_arrow(ui, &response, open);
    if let Some(color) = context.root_color {
        paint_root_bar(ui, &response, color);
    }
    click_on_folder(ui, &response, root);
    let mut fold = None;
    root_menu(&response, &root.path, context, &mut fold, action);
    if let Some(fold) = fold {
        fold_all(root, fold);
    }
    if open {
        let indent_id = root.path.clone();
        ui.indent(indent_id, |ui| show_children(ui, root, context, edits, files, action));
    }
    response
}

/// Draw a folder's entries, or why they could not be read.
fn show_children(
    ui: &mut Ui,
    folder: &mut FsNode,
    context: &TreeContext,
    edits: &mut ExplorerEdits,
    files: &mut FileRows,
    action: &mut Option<ExplorerAction>,
) {
    folder.poll_loading();
    match &mut folder.children {
        Children::NotLoaded => {}
        Children::Loading(_) => {
            ui.label(RichText::new("Loading\u{2026}").weak().italics());
        }
        Children::Failed(error) => {
            ui.label(RichText::new(format!("Cannot read: {error}")).weak().italics());
        }
        Children::Loaded(children) => {
            // A new entry is named at the top of its folder.
            if edits.new_entry.as_ref().is_some_and(|new_entry| new_entry.target.folder == folder.path) {
                new_entry_row(ui, edits.new_entry);
            }
            for child in children.iter_mut() {
                if child.is_folder() {
                    if folder_is_shown(child, context) {
                        show_folder(ui, child, context, edits, files, action);
                    }
                } else if file_is_shown(child, context) {
                    let row_id = ui.scope_id().with(&child.path);
                    ui.scope_builder(egui::UiBuilder::new().scope_id(row_id), |ui| {
                        show_file(ui, child, context, edits, files, action);
                    });
                }
            }
        }
    }
}

/// Draw one folder row and, when expanded, its contents indented below.
fn show_folder(
    ui: &mut Ui,
    folder: &mut FsNode,
    context: &TreeContext,
    edits: &mut ExplorerEdits,
    files: &mut FileRows,
    action: &mut Option<ExplorerAction>,
) {
    // A folder being renamed shows a name field in place of its row.
    if edits.rename.as_ref().is_some_and(|rename| rename.target == folder.path) {
        rename_row(ui, edits.rename);
        return;
    }
    let label = format!("{ICON_GAP}{}{}", folder.name, link_marker(folder));
    let response = tree_row(ui, is_revealed(context, &folder.path), label);
    files.record_focus(&response);
    scroll_if_revealed(ui, context, &folder.path, &response);
    if folder.kind == NodeKind::SymlinkLoop {
        response.clone().on_hover_text("Link to a parent folder; not expanded to avoid a loop");
    } else {
        paint_arrow(ui, &response, shows_contents(ui, folder, context));
    }
    click_on_folder(ui, &response, folder);
    if response.double_clicked() {
        *action = Some(ExplorerAction::RenameInTree(folder.path.clone()));
    }
    let mut fold = None;
    folder_menu(&response, &folder.path, context, &mut fold, action);
    if let Some(fold) = fold {
        fold_all(folder, fold);
    }
    if shows_contents(ui, folder, context) {
        let indent_id = folder.path.clone();
        ui.indent(indent_id, |ui| show_children(ui, folder, context, edits, files, action));
    }
}

/// Draw one file row; a click opens the file, a double click renames it
/// in place.
fn show_file(
    ui: &mut Ui,
    file: &FsNode,
    context: &TreeContext,
    edits: &mut ExplorerEdits,
    files: &mut FileRows,
    action: &mut Option<ExplorerAction>,
) {
    if edits.rename.as_ref().is_some_and(|rename| rename.target == file.path) {
        rename_row(ui, edits.rename);
        return;
    }
    if is_terminal_path(&file.path) {
        show_terminal_file(ui, file, context, files, action);
        return;
    }
    let is_open = context.open_file == Some(file.path.as_path()) || is_revealed(context, &file.path);
    let dirty = context.dirty_files.iter().any(|path| path == &file.path);
    let dirty_marker = if dirty { " •" } else { "" };
    let lock_gap = if file.encrypted { LOCK_GAP } else { "" };
    let label = format!("{}{}{dirty_marker}{lock_gap}", file.name, link_marker(file));
    // Unsaved is a state, so it gets a neutral mark (the bullet, in the strong
    // text color), like the dot on an unsaved tab. Hue is kept for telling
    // folders apart; see `paint_root_bar`.
    let label = if dirty {
        egui::RichText::new(label).color(ui.visuals().strong_text_color())
    } else {
        egui::RichText::new(label)
    };
    let row = file_row(ui, files.highlights(&file.path, is_open), label);
    let response = row.inner;
    scroll_if_revealed(ui, context, &file.path, &response);
    if file.encrypted {
        paint_file_lock(ui, &response);
    }
    if let Some(marks) = context.note_marks
        && is_markdown_path(&file.path)
        && marks.has_notes(&file.path)
    {
        paint_note_mark(ui, row.response.rect);
    }
    file_click(&response, &file.path, files, action);
    if response.hovered() {
        set_hovered(ui, HoveredFile { path: file.path.clone(), rect: response.rect });
    }
    file_menu(&response, &file.path, context, None, files, action);
}

/// Modified clicks change the selection. Plain clicks open or rename the file.
pub(super) fn file_click(
    response: &egui::Response,
    path: &Path,
    files: &mut FileRows,
    action: &mut Option<ExplorerAction>,
) {
    if files.record(path, response) {
        return;
    }
    if response.double_clicked() {
        *action = Some(ExplorerAction::RenameInTree(path.to_path_buf()));
    } else if response.clicked() {
        *action = Some(ExplorerAction::OpenFile(path.to_path_buf()));
    }
}

/// The row of an entry being renamed (a file, or a terminal in the
/// Terminals list): a name field, with the name before the extension
/// selected. Enter or leaving the field renames; Esc cancels.
pub(super) fn rename_row<T>(ui: &mut Ui, slot: &mut Option<InlineRename<T>>) {
    ui.horizontal(|ui| {
        // Line the field up with the names in the other rows.
        ui.add_space(ui.spacing().button_padding.x + NOTE_MARK_SIZE);
        rename_field(ui, slot, egui::Id::unique("explorer-rename-field"), RENAME_FIELD_WIDTH);
    });
    if let Some(error) = slot.as_ref().and_then(|rename| rename.error.as_ref()) {
        ui.label(RichText::new(error).small().color(ui.visuals().error_fg_color));
    }
}

/// The name field of something renamed in place (a tree row, a tab, a
/// terminal in the list).
/// On its first frame it takes focus with the name before the extension
/// selected. Enter or leaving the field marks it submitted; Esc clears `slot`.
pub fn rename_field<T>(ui: &mut Ui, slot: &mut Option<InlineRename<T>>, id: egui::Id, width: f32) {
    let Some(rename) = slot.as_mut() else {
        return;
    };
    let output = egui::TextEdit::singleline(&mut rename.name).id(id).desired_width(width).show(ui);
    if rename.needs_focus {
        rename.needs_focus = false;
        output.response.request_focus();
        let mut state = output.state;
        let stem_chars = name_stem_chars(&rename.name);
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(stem_chars))));
        state.store(ui.ctx(), id);
    }
    if output.response.lost_focus() {
        if ui.input(|input| input.key_pressed(Key::Escape)) {
            *slot = None;
        } else {
            rename.submitted = true;
        }
    }
}

/// How many characters of `name` come before its extension: all of them
/// when there is none, or when the name only starts with a dot (".env").
fn name_stem_chars(name: &str) -> usize {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[..dot].chars().count(),
        _ => name.chars().count(),
    }
}

/// True when `path` is the entry "Show in files" pointed at.
pub(super) fn is_revealed(context: &TreeContext, path: &Path) -> bool {
    context.revealed.is_some_and(|revealed| revealed.path == path)
}

/// Scroll the row of the revealed entry to the middle of the tree, on the
/// frame after "Show in files". Only up and down: a thin strip at the left
/// edge stands in for the row, so a long name does not scroll the tree sideways.
pub(super) fn scroll_if_revealed(ui: &Ui, context: &TreeContext, path: &Path, response: &egui::Response) {
    if context.revealed.is_some_and(|revealed| revealed.scroll && revealed.path == path) {
        let left = ui.clip_rect().left();
        let strip = response.rect.with_min_x(left).with_max_x(left + 1.0);
        ui.scroll_to_rect(strip, Some(egui::Align::Center));
    }
}

/// Fold or unfold a folder whose row was clicked. With Alt (Option on a
/// Mac) held, everything inside it folds or opens too.
fn click_on_folder(ui: &Ui, response: &egui::Response, folder: &mut FsNode) {
    if !toggles_fold(response) {
        return;
    }
    if !ui.input(|input| input.modifiers.alt) {
        folder.toggle_expanded(repaint_when_done(ui));
        return;
    }
    let fold = if folder.expanded { FoldAll::Collapse } else { FoldAll::Expand };
    fold_all(folder, fold);
}

/// A fold of a folder and everything inside it, asked for from its menu
/// or with an Alt-click.
#[derive(Clone, Copy, Debug)]
enum FoldAll {
    Expand,
    Collapse,
}

/// Fold or open `folder` and everything inside it.
fn fold_all(folder: &mut FsNode, fold: FoldAll) {
    match fold {
        FoldAll::Expand => folder.expand_all(),
        FoldAll::Collapse => folder.collapse_all(),
    }
}

/// "Expand all" and "Collapse all" at the top of a folder's menu.
fn fold_all_items(ui: &mut Ui, fold: &mut Option<FoldAll>) {
    let items = [
        (FoldAll::Expand, "Expand all", "Open this folder and every folder inside it (or Alt/Option-click it)"),
        (FoldAll::Collapse, "Collapse all", "Fold this folder and every folder inside it (or Alt/Option-click it)"),
    ];
    for (choice, label, tip) in items {
        if ui.button(label).on_hover_text(tip).clicked() {
            *fold = Some(choice);
            ui.close();
        }
    }
    ui.separator();
}

/// True when a click on a folder row should fold or unfold it. egui reports
/// both clicks of a double click as clicks, so the second one is skipped:
/// otherwise a double click opens the folder and closes it again at once.
fn toggles_fold(response: &egui::Response) -> bool {
    response.clicked() && !response.double_clicked()
}

/// One explorer row on a single line (the tree scrolls sideways for long names).
pub(super) fn tree_row(ui: &mut Ui, selected: bool, label: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(egui::Button::selectable(selected, label.into()))
}

/// A file's row: the gap where folders have their arrow, then the name as
/// a button. The gap is left outside the button, so the highlight of the
/// open file starts at its name rather than at a blank. The inner response
/// is the button's; the outer one spans the whole row, gap included.
fn file_row(ui: &mut Ui, selected: bool, label: impl Into<egui::WidgetText>) -> egui::InnerResponse<egui::Response> {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(icon_gap_width(ui));
        tree_row(ui, selected, label)
    })
}

/// Room at the start of each row for the folder arrow, so file names line
/// up with folder names.
pub(super) const ICON_GAP: &str = "     ";
/// Width of the folder arrow, in points (for placing it).
const ARROW_SIZE: f32 = 7.0;
/// Width of the color bar on the left edge of a root's row.
const ROOT_BAR_WIDTH: f32 = 3.0;
/// How far the bar stops short of the row's top and bottom.
const ROOT_BAR_INSET: f32 = 4.0;
/// Size of the buttons in a section header.
pub(super) const HEADER_BUTTON_SIZE: f32 = 22.0;
/// Width of the note marker's icon box, in points.
pub(super) const NOTE_MARK_SIZE: f32 = 14.0;
/// Room after an encrypted file's name for its padlock (wide enough for
/// [`LOCK_MARK_SIZE`] and a gap before it).
const LOCK_GAP: &str = "      ";
/// Width of the name field of a file being renamed in the tree.
const RENAME_FIELD_WIDTH: f32 = 220.0;

/// Paint the speech bubble in the gap before a file name, where folders
/// have their arrow.
/// `row` is the whole row, gap included; the bubble sits outside the
/// file's button, so it is always drawn over the panel.
fn paint_note_mark(ui: &Ui, row: egui::Rect) {
    let visuals = ui.visuals();
    let center = egui::pos2(row.left() + ui.spacing().button_padding.x + NOTE_MARK_SIZE / 2.0, row.center().y);
    let behind = visuals.panel_fill;
    paint_icon(ui.painter(), Icon::Bubble, center, visuals.weak_text_color(), visuals.hyperlink_color, behind);
}

/// Width of [`ICON_GAP`] in the row font: the room a file row leaves before
/// its button, so file names line up with the names of folders.
fn icon_gap_width(ui: &Ui) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.fonts_mut(|fonts| fonts.layout_no_wrap(ICON_GAP.to_string(), font, egui::Color32::WHITE).size().x)
}

/// Paint the padlock at the end of an encrypted file's row, in the room
/// [`LOCK_GAP`] left after its name.
fn paint_file_lock(ui: &Ui, response: &egui::Response) {
    let visuals = ui.visuals();
    let center = egui::pos2(
        response.rect.right() - ui.spacing().button_padding.x - LOCK_MARK_SIZE / 2.0,
        response.rect.center().y,
    );
    paint_lock_mark(ui.painter(), center, visuals.weak_text_color(), visuals.hyperlink_color);
}

/// Paint a bar of a root's color on the left edge of its row: the counterpart
/// of the colored line on top of its tabs. A bar rather than a dot after the
/// name, so the folder color is never mistaken for the unsaved bullet.
fn paint_root_bar(ui: &Ui, response: &egui::Response, color: egui::Color32) {
    let row = response.rect;
    let bar = egui::Rect::from_min_max(
        egui::pos2(row.left(), row.top() + ROOT_BAR_INSET),
        egui::pos2(row.left() + ROOT_BAR_WIDTH, row.bottom() - ROOT_BAR_INSET),
    );
    ui.painter().rect_filled(bar, ROOT_BAR_WIDTH / 2.0, color);
}

/// Paint the fold arrow in the gap before a folder name.
fn paint_arrow(ui: &Ui, response: &egui::Response, expanded: bool) {
    let center =
        egui::pos2(response.rect.left() + ui.spacing().button_padding.x + ARROW_SIZE / 2.0, response.rect.center().y);
    let color = ui.style().interact(response).fg_stroke.color;
    paint_fold_arrow(ui.painter(), center, expanded, color);
}

/// Marker after the name of a symlinked entry.
fn link_marker(node: &FsNode) -> &'static str {
    if node.is_symlink { " ↗" } else { "" }
}

/// Right-click menu of a root folder, including "Remove from explorer".
fn root_menu(
    response: &egui::Response,
    path: &Path,
    context: &TreeContext,
    fold: &mut Option<FoldAll>,
    action: &mut Option<ExplorerAction>,
) {
    response.context_menu(|ui| {
        fold_all_items(ui, fold);
        if new_menu(ui, path, context, action) {
            ui.separator();
        }
        open_in_file_manager_item(ui, path, action);
        copy_items(ui, path, context, action);
        ui.separator();
        let remove = ui
            .add_enabled(context.can_remove_roots, egui::Button::new("Remove from explorer"))
            .on_hover_text("Hide this folder here; nothing on disk changes")
            .on_disabled_hover_text("The explorer needs at least one folder");
        if remove.clicked() {
            *action = Some(ExplorerAction::RemoveRoot(path.to_path_buf()));
            ui.close();
        }
    });
}

/// Right-click menu for a file, with "Exit terminal" when it is a .tmux file.
pub(super) fn file_menu(
    response: &egui::Response,
    path: &Path,
    context: &TreeContext,
    exit: Option<(&str, bool)>,
    files: &FileRows,
    action: &mut Option<ExplorerAction>,
) {
    response.context_menu(|ui| {
        if let Some((session, running)) = exit {
            let exit_item = ui
                .add_enabled(running, egui::Button::new("Exit terminal"))
                .on_hover_text("End the session and everything running in it")
                .on_disabled_hover_text("Not running");
            if exit_item.clicked() {
                *action = Some(ExplorerAction::ExitTerminal(session.to_string()));
                ui.close();
            }
            ui.separator();
        }
        copy_items(ui, path, context, action);
        ui.separator();
        if encryption_item(ui, path, context, action) {
            ui.separator();
        }
        let count = files.target_count(path);
        if exit.is_none() && !crate::remote::is_mirror(path) {
            let label = if count > 1 {
                format!("Move {count} Selected Files to Folder…")
            } else {
                "Move to Folder…".into()
            };
            if ui.button(label).clicked() {
                *action = Some(ExplorerAction::MoveFiles(vec![path.to_path_buf()]));
                ui.close();
            }
        }
        let delete_label = if count > 1 { format!("Delete {count} selected files…") } else { "Delete…".into() };
        rename_delete_items(ui, path, &delete_label, action);
    });
}

/// "Encrypt" on a plain scratch, "Remove encryption" on an encrypted one;
/// nothing for files outside the scratch folder. Returns true when shown.
fn encryption_item(ui: &mut Ui, path: &Path, context: &TreeContext, action: &mut Option<ExplorerAction>) -> bool {
    let Some(folder) = context.scratch_folder else {
        return false;
    };
    if !is_scratch(path, folder) {
        return false;
    }
    // Only the first bytes are read, so this is cheap while the menu is open.
    let encrypted = crate::crypto::file_is_encrypted(path);
    let (label, tip) = if encrypted {
        ("Remove encryption", "Store this scratch as plain text; anyone who can read the file sees its text")
    } else {
        ("Encrypt", "Store this scratch encrypted with the scratch password. AI tools are then off for it")
    };
    if ui.button(label).on_hover_text(tip).clicked() {
        *action = Some(ExplorerAction::SetEncrypted { path: path.to_path_buf(), encrypt: !encrypted });
        ui.close();
    }
    true
}

/// "Rename…" (also a double click) and "Delete…" for a file or folder.
fn rename_delete_items(ui: &mut Ui, path: &Path, delete_label: &str, action: &mut Option<ExplorerAction>) {
    if ui.button("Rename…").on_hover_text("Or double-click it").clicked() {
        *action = Some(ExplorerAction::Rename(path.to_path_buf()));
        ui.close();
    }
    if ui.button(delete_label).clicked() {
        *action = Some(ExplorerAction::Delete(vec![path.to_path_buf()]));
        ui.close();
    }
}

/// Right-click menu of a nested folder, including "Rename…" and "Delete…".
fn folder_menu(
    response: &egui::Response,
    path: &Path,
    context: &TreeContext,
    fold: &mut Option<FoldAll>,
    action: &mut Option<ExplorerAction>,
) {
    response.context_menu(|ui| {
        fold_all_items(ui, fold);
        if new_menu(ui, path, context, action) {
            ui.separator();
        }
        open_in_file_manager_item(ui, path, action);
        copy_items(ui, path, context, action);
        ui.separator();
        rename_delete_items(ui, path, "Delete…", action);
    });
}

/// Offer to open a local folder in the system file manager.
fn open_in_file_manager_item(ui: &mut Ui, path: &Path, action: &mut Option<ExplorerAction>) {
    if crate::remote::is_mirror(path) {
        return;
    }
    let label = if cfg!(target_os = "macos") { "Open in Finder" } else { "Open in File Manager" };
    if ui.button(label).clicked() {
        *action = Some(ExplorerAction::OpenInFileManager(path.to_path_buf()));
        ui.close();
    }
}

/// The "New" submenu of a local folder: File, Folder, and Terminal when
/// tmux is installed. Remote folders get none (creating there is not
/// supported). Returns true when the menu was shown.
fn new_menu(ui: &mut Ui, folder: &Path, context: &TreeContext, action: &mut Option<ExplorerAction>) -> bool {
    if crate::remote::is_mirror(folder) {
        return false;
    }
    ui.menu_button("New", |ui| {
        let mut choices = vec![
            (NewKind::File, "File", "A new file here; type its name in the tree"),
            (NewKind::Folder, "Folder", "A new folder here; type its name in the tree"),
        ];
        if context.terminals.is_some() {
            choices.push((
                NewKind::Terminal,
                "Terminal",
                "A .tmux file here: opening it starts a terminal in this folder",
            ));
        }
        for (kind, label, tip) in choices {
            if ui.button(label).on_hover_text(tip).clicked() {
                *action = Some(ExplorerAction::New(folder.to_path_buf(), kind));
                ui.close();
            }
        }
    });
    true
}

/// "Copy absolute path", "Copy relative path" and "Copy name".
fn copy_items(ui: &mut Ui, path: &Path, context: &TreeContext, action: &mut Option<ExplorerAction>) {
    let choices = [
        ("Copy absolute path", absolute_path_text(path)),
        ("Copy relative path", relative_path_text(path, context.root)),
        ("Copy name", name_text(path)),
    ];
    for (title, text) in choices {
        if ui.button(title).on_hover_text(&text).clicked() {
            *action = Some(ExplorerAction::Copy(text));
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExplorerAction, FileRows, FileSelection, file_click, file_row, icon_gap_width, name_stem_chars};
    use eframe::egui::{self, Context, Event, Key, Modifiers, PointerButton, RawInput};
    use std::path::{Path, PathBuf};

    #[test]
    fn open_file_highlight_starts_at_its_name() {
        let ctx = Context::default();
        let mut offsets = None;
        let mut output = ctx.run_ui(RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let row = file_row(ui, true, "todo.md");
                offsets = Some((row.inner.rect.left() - row.response.rect.left(), icon_gap_width(ui)));
            });
        });
        output.textures_delta.clear();
        let (button_offset, gap) = offsets.expect("the row was drawn");
        assert!(gap > 0.0);
        assert_eq!(button_offset, gap);
    }

    #[test]
    fn selects_the_name_before_the_extension() {
        assert_eq!(name_stem_chars("notes.md"), 5);
        assert_eq!(name_stem_chars("archive.tar.gz"), 11);
        assert_eq!(name_stem_chars("README"), 6);
        assert_eq!(name_stem_chars(".env"), 4);
        assert_eq!(name_stem_chars("café.md"), 4);
    }

    const FILE_NAMES: [&str; 4] = ["one.md", "two.md", "three.md", "four.md"];

    /// Draw file rows with egui and dispatch the same clicks as the explorer.
    #[derive(Default)]
    struct FileTree {
        ctx: Context,
        selection: FileSelection,
        open_file: Option<PathBuf>,
        positions: [egui::Pos2; 4],
        folder_position: egui::Pos2,
        hide_middle: bool,
        highlighted: Vec<PathBuf>,
        editor: String,
        dialog_open: bool,
    }

    impl FileTree {
        /// Send input and record the action and highlights drawn by the rows.
        fn frame(&mut self, events: Vec<Event>) -> Option<ExplorerAction> {
            let mut action = None;
            self.highlighted.clear();
            let input = RawInput { events, ..Default::default() };
            let mut output = self.ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let open = self.open_file.as_deref();
                    let mut files = FileRows::new(ui, &mut self.selection, open);
                    let folder = super::tree_row(ui, false, "Folder");
                    self.folder_position = folder.rect.center();
                    files.record_focus(&folder);
                    for (index, name) in FILE_NAMES.iter().enumerate() {
                        if self.hide_middle && index == 1 {
                            continue;
                        }
                        let path = Path::new(name);
                        let highlighted = files.highlights(path, open == Some(path));
                        if highlighted {
                            self.highlighted.push(path.to_path_buf());
                        }
                        let row_id = ui.scope_id().with(path);
                        let row = ui
                            .scope_builder(egui::UiBuilder::new().scope_id(row_id), |ui| {
                                file_row(ui, highlighted, *name)
                            })
                            .inner;
                        self.positions[index] = row.inner.rect.center();
                        file_click(&row.inner, path, &mut files, &mut action);
                    }
                    files.update(ui);
                    action = action.take().or_else(|| files.keyboard_delete(ui).map(ExplorerAction::Delete));
                    ui.add(egui::TextEdit::multiline(&mut self.editor).id(egui::Id::unique("test-editor")));
                });
                if self.dialog_open {
                    let dialog = egui::Modal::new(egui::Id::unique("test-dialog")).show(ui.ctx(), |ui| {
                        ui.label("Confirm deletion");
                    });
                    self.dialog_open = !dialog.should_close();
                }
            });
            output.textures_delta.clear();
            if let Some(ExplorerAction::OpenFile(path)) = &action {
                self.open_file = Some(path.clone());
            }
            action
        }

        /// Move to the file row, press and release the primary mouse button.
        fn click(&mut self, index: usize, modifiers: Modifiers) -> Option<ExplorerAction> {
            self.click_at(self.positions[index], modifiers)
        }

        /// Click a position in the test UI.
        fn click_at(&mut self, pos: egui::Pos2, modifiers: Modifiers) -> Option<ExplorerAction> {
            self.frame(vec![Event::PointerMoved(pos)]);
            let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)])
        }

        /// Press Delete with no modifiers.
        fn press_delete(&mut self) -> Option<ExplorerAction> {
            self.press(Key::Delete, Modifiers::NONE)
        }

        /// Send a keyboard shortcut to the focused widget.
        fn press(&mut self, key: Key, modifiers: Modifiers) -> Option<ExplorerAction> {
            self.frame(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }])
        }
    }

    #[test]
    fn shift_click_selects_a_range_without_opening_files_and_delete_uses_visible_rows() {
        let mut tree = FileTree::default();
        tree.frame(Vec::new());
        let action = tree.click(0, Modifiers::NONE);
        assert!(matches!(action, Some(ExplorerAction::OpenFile(path)) if path == Path::new("one.md")));
        assert!(tree.click(2, Modifiers::SHIFT).is_none());
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, FILE_NAMES[..3].iter().map(PathBuf::from).collect::<Vec<_>>());

        tree.hide_middle = true;
        assert!(matches!(tree.press_delete(), Some(ExplorerAction::Delete(paths))
            if paths == [PathBuf::from("one.md"), PathBuf::from("three.md")]));
    }

    #[test]
    fn shift_arrows_extend_and_shrink_a_range_across_visible_files() {
        let mut tree = FileTree { hide_middle: true, ..FileTree::default() };
        tree.frame(Vec::new());
        tree.click(0, Modifiers::NONE);
        for _ in 0..3 {
            assert!(tree.press(Key::ArrowDown, Modifiers::SHIFT).is_none());
        }
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, ["one.md", "three.md", "four.md"].map(PathBuf::from));

        assert!(tree.press(Key::ArrowUp, Modifiers::SHIFT).is_none());
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, ["one.md", "three.md"].map(PathBuf::from));
        assert!(tree.press(Key::ArrowUp, Modifiers::SHIFT).is_none());
        assert!(tree.press(Key::ArrowUp, Modifiers::SHIFT).is_none());
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, [PathBuf::from("one.md")]);

        tree.press(Key::ArrowDown, Modifiers::SHIFT);
        assert!(matches!(tree.press_delete(), Some(ExplorerAction::Delete(paths))
            if paths == ["one.md", "three.md"].map(PathBuf::from)));
    }

    #[test]
    fn shift_arrows_move_the_shift_click_endpoint_across_its_anchor() {
        let mut tree = FileTree::default();
        tree.frame(Vec::new());
        tree.click(1, Modifiers::NONE);
        tree.click(0, Modifiers::SHIFT);
        tree.press(Key::ArrowDown, Modifiers::SHIFT);
        tree.press(Key::ArrowDown, Modifiers::SHIFT);
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, ["two.md", "three.md"].map(PathBuf::from));
    }

    #[test]
    fn shift_down_from_a_focused_folder_starts_with_the_next_file() {
        let mut tree = FileTree::default();
        tree.frame(Vec::new());
        tree.click_at(tree.folder_position, Modifiers::NONE);
        assert!(tree.press(Key::ArrowUp, Modifiers::SHIFT).is_none());
        assert!(tree.press(Key::ArrowDown, Modifiers::SHIFT).is_none());
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, [PathBuf::from("one.md")]);
        assert!(tree.open_file.is_none());
    }

    #[test]
    fn shift_arrows_in_the_editor_select_text() {
        let mut tree = FileTree { editor: "first line\nsecond line".into(), ..FileTree::default() };
        tree.frame(Vec::new());
        tree.click(0, Modifiers::NONE);
        let editor_id = egui::Id::unique("test-editor");
        tree.ctx.memory_mut(|memory| memory.request_focus(editor_id));
        tree.press(Key::ArrowUp, Modifiers::SHIFT);
        let editor = egui::TextEdit::load_state(&tree.ctx, editor_id).unwrap();
        let range = editor.cursor.char_range().unwrap().as_sorted_char_range();
        assert_ne!(range.start, range.end);
        assert_eq!(tree.highlighted, [PathBuf::from("one.md")]);
    }

    #[test]
    fn command_click_toggles_individual_files_and_deletes_the_remaining_selection() {
        let mut tree = FileTree::default();
        tree.frame(Vec::new());
        tree.click(0, Modifiers::NONE);
        assert!(tree.click(2, Modifiers::COMMAND).is_none());
        assert!(tree.click(3, Modifiers::COMMAND).is_none());
        assert!(tree.click(3, Modifiers::COMMAND).is_none());
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, [PathBuf::from("one.md"), PathBuf::from("three.md")]);
        assert!(matches!(tree.press_delete(), Some(ExplorerAction::Delete(paths))
            if paths == [PathBuf::from("one.md"), PathBuf::from("three.md")]));
    }

    #[test]
    fn command_click_can_deselect_the_open_file_and_start_a_new_selection() {
        let mut tree = FileTree::default();
        tree.frame(Vec::new());
        tree.click(0, Modifiers::NONE);
        assert!(tree.click(0, Modifiers::COMMAND).is_none());
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, Vec::<PathBuf>::new());
        assert!(tree.press_delete().is_none());

        assert!(tree.click(2, Modifiers::COMMAND).is_none());
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, [PathBuf::from("three.md")]);
    }

    #[test]
    fn select_all_in_a_focused_tree_targets_every_displayed_file() {
        for focus_folder in [false, true] {
            let mut tree = FileTree { hide_middle: true, ..FileTree::default() };
            tree.frame(Vec::new());
            if focus_folder {
                tree.click_at(tree.folder_position, Modifiers::NONE);
            } else {
                tree.click(0, Modifiers::NONE);
            }
            assert!(tree.press(Key::A, Modifiers::COMMAND).is_none());
            let displayed = [PathBuf::from("one.md"), PathBuf::from("three.md"), PathBuf::from("four.md")];
            tree.frame(Vec::new());
            assert_eq!(tree.highlighted, displayed);
            assert!(matches!(tree.press_delete(), Some(ExplorerAction::Delete(paths)) if paths == displayed));
        }
    }

    #[test]
    fn select_all_in_a_text_field_selects_its_text() {
        let mut tree = FileTree { editor: "hello".into(), ..FileTree::default() };
        tree.frame(Vec::new());
        tree.click(0, Modifiers::NONE);
        let editor_id = egui::Id::unique("test-editor");
        tree.ctx.memory_mut(|memory| memory.request_focus(editor_id));
        assert!(tree.press(Key::A, Modifiers::COMMAND).is_none());
        let editor = egui::TextEdit::load_state(&tree.ctx, editor_id).unwrap();
        let range = editor.cursor.char_range().unwrap().as_sorted_char_range();
        assert_eq!(range.start.0..range.end.0, 0..5);
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, [PathBuf::from("one.md")]);
    }

    #[test]
    fn escape_clears_the_selection_and_keeps_keyboard_focus_in_the_tree() {
        let mut tree = FileTree::default();
        tree.frame(Vec::new());
        tree.click(0, Modifiers::NONE);
        tree.press(Key::A, Modifiers::COMMAND);
        tree.press(Key::Escape, Modifiers::NONE);
        assert!(tree.press_delete().is_none());
        assert_eq!(tree.highlighted, Vec::<PathBuf>::new());

        tree.press(Key::A, Modifiers::COMMAND);
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, FILE_NAMES.map(PathBuf::from));
    }

    #[test]
    fn escape_leaves_the_tree_selection_when_a_text_field_has_focus() {
        let mut tree = FileTree::default();
        tree.frame(Vec::new());
        tree.click(0, Modifiers::NONE);
        tree.press(Key::A, Modifiers::COMMAND);
        let editor_id = egui::Id::unique("test-editor");
        tree.ctx.memory_mut(|memory| memory.request_focus(editor_id));
        tree.press(Key::Escape, Modifiers::NONE);
        assert_eq!(tree.highlighted, FILE_NAMES.map(PathBuf::from));
        assert!(tree.ctx.memory(|memory| memory.focused().is_none()));
    }

    #[test]
    fn escape_closes_the_dialog_before_it_can_clear_the_tree_selection() {
        let mut tree = FileTree::default();
        tree.frame(Vec::new());
        tree.click(0, Modifiers::NONE);
        tree.press(Key::A, Modifiers::COMMAND);
        tree.dialog_open = true;
        tree.frame(Vec::new());
        tree.press(Key::Escape, Modifiers::NONE);
        assert!(!tree.dialog_open);
        tree.frame(Vec::new());
        assert_eq!(tree.highlighted, FILE_NAMES.map(PathBuf::from));
    }
}
