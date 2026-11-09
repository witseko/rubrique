//! Find (Cmd+F) and replace (Cmd+Alt+F, Ctrl+H on Linux): a bar at the top
//! right of the page that searches the open file, or every text file under
//! the explorer's folders, and replaces matches in the open file.

use std::ops::Range;
use std::path::PathBuf;

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{self, Context, Id, Key, Modifiers, RichText, Ui, pos2};

use super::{OpenFile, RubriqueApp, char_to_byte};
use crate::document::view::{FindHighlight, apply_with_undo, editor_selection, focus_editor, select_editor_range};
use crate::explorer::paths::name_text;
use crate::find::{
    FileHit, HitKind, Query, SearchOptions, SearchResults, char_index, find_all, replace_all, search_files,
    search_remote_files,
};
use crate::remote::task::{Task, spawn_with_repaint};

const FIND_WIDTH: f32 = 440.0;
const FIND_RADIUS: u8 = 8;
/// Space between the bar and the page's top and right edges.
const FIND_MARGIN: f32 = 12.0;
/// Width of the match count ("12 of 40").
const COUNT_WIDTH: f32 = 72.0;
/// Width of the Aa and W toggles.
const TOGGLE_WIDTH: f32 = 30.0;
/// Width of the Previous and Next buttons, and of Replace.
const STEP_WIDTH: f32 = 64.0;
/// Width of the Replace all button.
const REPLACE_ALL_WIDTH: f32 = 86.0;
/// Width of the close cross.
const CLOSE_WIDTH: f32 = 24.0;
/// Tallest the all-files results list gets before it scrolls.
const RESULTS_HEIGHT: f32 = 320.0;
/// Hits an all-files search stops at.
const RESULT_LIMIT: usize = 300;
const SMALL_TEXT: f32 = 11.5;

/// The open find bar.
#[derive(Debug, Default)]
pub(super) struct FindBar {
    pub(super) query: String,
    options: SearchOptions,
    /// Search every file under the explorer's folders instead of the open one.
    all_files: bool,
    /// Show the replace row (hidden while searching all files).
    replacing: bool,
    replacement: String,
    /// Which field gets focus on the next frame.
    focus: FocusRequest,
    /// Matches in the open file, in order.
    matches: Vec<Range<usize>>,
    /// Index into `matches` of the one being looked at.
    current: Option<usize>,
    /// The query, options, file and text revision `matches` were computed for.
    computed_for: Option<(String, SearchOptions, PathBuf, u64)>,
    /// Scroll the editor to the current match this frame.
    scroll: bool,
    /// After opening a file from the results: make this match the current one.
    pending: Option<Range<usize>>,
    /// After a replacement: make the first match at or after this byte the
    /// current one (the one that followed the replaced match).
    resume_at: Option<usize>,
    /// All-files results, and the query they are for.
    results: Option<(String, SearchOptions, SearchResults)>,
    /// Results from remote folders (or why they failed), and their query.
    remote_results: Option<(String, SearchOptions, Result<SearchResults, String>)>,
    /// A remote search running in the background, and its query.
    remote_search: Option<(String, SearchOptions, Task<std::io::Result<SearchResults>>)>,
}

/// Which field of the bar gets focus on the next frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FocusRequest {
    #[default]
    Nowhere,
    Query,
    /// The query field, with the whole query selected so typing replaces it.
    QuerySelected,
    Replacement,
}

impl FindBar {
    /// The all-files search the bar asks for now.
    fn query(&self) -> Query<'_> {
        Query { text: &self.query, options: self.options }
    }

    /// True when `results` (or another cached search) is for the current query.
    fn is_current(&self, query: &str, options: SearchOptions) -> bool {
        *query == self.query && options == self.options
    }
}

impl RubriqueApp {
    /// Open the bar (or put the pointer back in its field), with the replace
    /// row when `replace` is set. A one-line selection in the editor becomes
    /// the query. Find works on the source, so a Markdown or HTML preview
    /// switches to it.
    pub(super) fn open_find(&mut self, ctx: &Context, replace: bool) {
        self.finish_block_edit();
        let selected = if self.showing_any_preview() { None } else { self.selected_line(ctx) };
        if self.showing_any_preview() {
            self.toggle_preview();
        }
        let bar = self.find.get_or_insert_with(FindBar::default);
        if let Some(selected) = selected {
            bar.query = selected;
        }
        bar.replacing |= replace;
        if replace {
            bar.all_files = false;
        }
        bar.focus = FocusRequest::QuerySelected;
    }

    /// The editor's selection when it is one non-empty line.
    fn selected_line(&self, ctx: &Context) -> Option<String> {
        let OpenFile::Text { document, .. } = &self.open_file else {
            return None;
        };
        let (first, last) = editor_selection(ctx)?;
        let selected = &document.text[char_to_byte(&document.text, first)..char_to_byte(&document.text, last)];
        (!selected.is_empty() && !selected.contains('\n')).then(|| selected.to_string())
    }

    pub(super) fn close_find(&mut self, ctx: &Context) {
        if self.find.take().is_some() && matches!(self.open_file, OpenFile::Text { .. }) {
            focus_editor(ctx);
        }
    }

    /// Cmd+G / Cmd+Shift+G while the bar is open.
    pub(super) fn find_shortcuts(&mut self, ctx: &Context) {
        if self.find.is_none() {
            return;
        }
        if ctx.input_mut(|input| input.consume_shortcut(&super::FIND_PREVIOUS)) {
            self.step_match(ctx, -1);
        } else if ctx.input_mut(|input| input.consume_shortcut(&super::FIND_NEXT)) {
            self.step_match(ctx, 1);
        }
    }

    /// Bring the open file's matches up to date with its text and the query.
    /// Runs before the editor is drawn, so the highlight is current.
    pub(super) fn prepare_find(&mut self, ctx: &Context) {
        let Some(bar) = &mut self.find else {
            return;
        };
        let OpenFile::Text { document, .. } = &self.open_file else {
            bar.matches.clear();
            bar.current = None;
            bar.computed_for = None;
            return;
        };
        let key = (bar.query.clone(), bar.options, document.path.clone(), document.revision());
        if bar.computed_for.as_ref() != Some(&key) {
            bar.matches = find_all(&document.text, &bar.query, bar.options);
            bar.computed_for = Some(key);
            bar.current = current_after_change(bar);
            if bar.current.is_some() {
                bar.scroll = true;
                self.select_current_match(ctx);
            }
        }
    }

    fn step_match(&mut self, ctx: &Context, step: isize) {
        let Some(bar) = &mut self.find else {
            return;
        };
        if bar.matches.is_empty() {
            return;
        }
        let count = bar.matches.len();
        let current = bar.current.unwrap_or(0);
        bar.current = Some((current + count).wrapping_add_signed(step) % count);
        bar.scroll = true;
        self.select_current_match(ctx);
    }

    /// Select the current match in the editor, so closing the bar leaves the
    /// cursor on it.
    fn select_current_match(&self, ctx: &Context) {
        let (Some(bar), OpenFile::Text { document, .. }) = (&self.find, &self.open_file) else {
            return;
        };
        let Some(range) = bar.current.and_then(|index| bar.matches.get(index)) else {
            return;
        };
        select_editor_range(ctx, char_index(&document.text, range.start), char_index(&document.text, range.end));
    }

    /// Replace the current match (one undo step) and move on to the match
    /// that followed it.
    fn replace_current(&mut self, ctx: &Context) {
        let (Some(bar), OpenFile::Text { document, preview, .. }) = (&mut self.find, &mut self.open_file) else {
            return;
        };
        let Some(range) = bar.current.and_then(|index| bar.matches.get(index).cloned()) else {
            return;
        };
        let replacement = bar.replacement.clone();
        let cursor = char_index(&document.text, range.start) + replacement.chars().count();
        apply_with_undo(ctx, &mut document.text, cursor, |text| text.replace_range(range.clone(), &replacement));
        document.on_text_changed();
        *preview = None;
        bar.resume_at = Some(range.start + replacement.len());
    }

    /// Replace every match in one undo step.
    fn replace_all_matches(&mut self, ctx: &Context) {
        let (Some(bar), OpenFile::Text { document, preview, .. }) = (&mut self.find, &mut self.open_file) else {
            return;
        };
        let Some(first) = bar.matches.first() else {
            return;
        };
        let count = bar.matches.len();
        let matches = bar.matches.clone();
        let replacement = bar.replacement.clone();
        let cursor = char_index(&document.text, first.start);
        apply_with_undo(ctx, &mut document.text, cursor, |text| *text = replace_all(text, &matches, &replacement));
        document.on_text_changed();
        *preview = None;
        self.show_status(format!("Replaced {count}"));
    }

    /// Open a hit's file in a tab, on its source, and go to the match.
    fn open_hit(&mut self, hit: &FileHit) {
        self.open_in_tab(hit.path.clone());
        if let OpenFile::Text { source, .. } = &mut self.open_file {
            *source = true;
        }
        if let Some(bar) = &mut self.find {
            bar.all_files = false;
            bar.computed_for = None;
            bar.pending = match &hit.kind {
                HitKind::Text { range, .. } => Some(range.clone()),
                HitKind::Name => None,
            };
        }
    }

    /// Draw the bar over the top right of the page.
    pub(super) fn find_bar(&mut self, ctx: &Context) {
        let Some(page) = self.ask.page_rect else {
            return;
        };
        if self.find.is_none() {
            return;
        }
        let at = pos2(page.right() - FIND_WIDTH - FIND_MARGIN, page.top() + FIND_MARGIN);
        let mut event = None;
        egui::Area::new(Id::unique("find-bar")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
            let fill = ui.visuals().extreme_bg_color;
            egui::Frame::popup(ui.style()).fill(fill).corner_radius(FIND_RADIUS).inner_margin(10).show(ui, |ui| {
                ui.set_width(FIND_WIDTH);
                ui.spacing_mut().item_spacing.y = 6.0;
                let Some(bar) = &mut self.find else {
                    return;
                };
                event = find_row(ui, bar);
                options_row(ui, bar);
                if bar.replacing && !bar.all_files && event.is_none() {
                    event = replace_row(ui, bar);
                }
                if bar.all_files {
                    let roots: Vec<PathBuf> = self.roots.iter().map(|root| root.path.clone()).collect();
                    if let Some(hit) = results_list(ui, bar, &roots) {
                        event = Some(FindEvent::Open(hit));
                    }
                }
            });
        });
        match event {
            Some(FindEvent::Next) => self.step_match(ctx, 1),
            Some(FindEvent::Previous) => self.step_match(ctx, -1),
            Some(FindEvent::Replace) => self.replace_current(ctx),
            Some(FindEvent::ReplaceAll) => self.replace_all_matches(ctx),
            Some(FindEvent::Close) => self.close_find(ctx),
            Some(FindEvent::Open(hit)) => self.open_hit(&hit),
            None => {}
        }
    }
}

/// Which match to look at after the matches were computed again: the one
/// asked for (after opening a hit or replacing), else the same position,
/// else the first.
fn current_after_change(bar: &mut FindBar) -> Option<usize> {
    if bar.matches.is_empty() {
        bar.pending = None;
        bar.resume_at = None;
        return None;
    }
    let last = bar.matches.len() - 1;
    let current = match (bar.pending.take(), bar.resume_at.take(), bar.current) {
        (Some(range), _, _) => bar.matches.iter().position(|found| *found == range),
        // Past the last match: wrap around to the first, like Next does.
        (None, Some(at), _) => Some(bar.matches.iter().position(|found| found.start >= at).unwrap_or(0)),
        (None, None, Some(current)) => Some(current.min(last)),
        (None, None, None) => None,
    };
    Some(current.unwrap_or(0))
}

/// What the editor should tint this frame. A free function over the field,
/// so the app can borrow its open file at the same time.
pub(super) fn highlight(find: &mut Option<FindBar>) -> Option<FindHighlight<'_>> {
    let bar = find.as_mut()?;
    if bar.all_files || bar.matches.is_empty() {
        return None;
    }
    let scroll_to_current = std::mem::take(&mut bar.scroll);
    Some(FindHighlight { matches: &bar.matches, current: bar.current, scroll_to_current })
}

/// What the bar asked for this frame.
enum FindEvent {
    Next,
    Previous,
    Replace,
    ReplaceAll,
    Close,
    Open(FileHit),
}

fn query_field_id() -> Id {
    Id::unique("find-bar-query")
}

/// The field, the Aa and W toggles, the count, Previous / Next and the cross.
fn find_row(ui: &mut Ui, bar: &mut FindBar) -> Option<FindEvent> {
    let mut event = None;
    ui.horizontal(|ui| {
        let hint = if bar.all_files { "Find in all files" } else { "Find in this file" };
        let field_width = ui.available_width()
            - 2.0 * TOGGLE_WIDTH
            - COUNT_WIDTH
            - 2.0 * STEP_WIDTH
            - CLOSE_WIDTH
            - 7.0 * ui.spacing().item_spacing.x;
        let field = ui.add(
            egui::TextEdit::singleline(&mut bar.query).id(query_field_id()).hint_text(hint).desired_width(field_width),
        );
        if matches!(bar.focus, FocusRequest::Query | FocusRequest::QuerySelected) {
            field.request_focus();
            if bar.focus == FocusRequest::QuerySelected {
                select_all(ui.ctx(), query_field_id(), &bar.query);
            }
            bar.focus = FocusRequest::Nowhere;
        }
        if field.lost_focus() && ui.input(|input| input.key_pressed(Key::Escape)) {
            event = Some(FindEvent::Close);
        } else if field.has_focus() && ui.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::Enter)) {
            event = Some(FindEvent::Previous);
        } else if field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
            event = Some(FindEvent::Next);
            bar.focus = FocusRequest::Query;
        }
        if option_toggle(ui, bar.options.match_case, "Aa", "Match case").clicked() {
            bar.options.match_case = !bar.options.match_case;
        }
        if option_toggle(ui, bar.options.whole_word, "W", "Whole word").clicked() {
            bar.options.whole_word = !bar.options.whole_word;
        }
        let count = if bar.all_files { String::new() } else { count_text(bar) };
        ui.add_sized([COUNT_WIDTH, ui.spacing().interact_size.y], egui::Label::new(RichText::new(count).weak()));
        let has_matches = !bar.all_files && !bar.matches.is_empty();
        let size = [STEP_WIDTH, ui.spacing().interact_size.y];
        if ui
            .add_enabled(has_matches, egui::Button::new("Previous").min_size(size.into()))
            .on_hover_text("Shift+Enter, Cmd+Shift+G")
            .clicked()
        {
            event = Some(FindEvent::Previous);
        }
        if ui
            .add_enabled(has_matches, egui::Button::new("Next").min_size(size.into()))
            .on_hover_text("Enter, Cmd+G")
            .clicked()
        {
            event = Some(FindEvent::Next);
        }
        if ui.small_button("×").on_hover_text("Close (Esc)").clicked() {
            event = Some(FindEvent::Close);
        }
    });
    event
}

/// "3 of 12", the total while no match is current, or "No matches".
fn count_text(bar: &FindBar) -> String {
    match (bar.current, bar.matches.len()) {
        (_, 0) if bar.query.is_empty() => String::new(),
        (_, 0) => "No matches".to_string(),
        (Some(current), total) => format!("{} of {total}", current + 1),
        (None, total) => format!("{total}"),
    }
}

/// A small on / off button for a search option.
fn option_toggle(ui: &mut Ui, on: bool, label: &str, hover: &str) -> egui::Response {
    let size = [TOGGLE_WIDTH, ui.spacing().interact_size.y];
    ui.add_sized(size, egui::Button::selectable(on, label)).on_hover_text(hover)
}

/// Select every character of the single-line field `id`, whose text is `text`.
fn select_all(ctx: &Context, id: Id, text: &str) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    let end = CCursor::new(text.chars().count());
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), end)));
    state.store(ctx, id);
}

/// The All files switch and the Replace switch.
fn options_row(ui: &mut Ui, bar: &mut FindBar) {
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut bar.all_files, "All files")
            .on_hover_text("Search every file in the explorer's folders")
            .changed()
        {
            bar.focus = FocusRequest::Query;
        }
        let hover = if bar.all_files { "Replace works in the open file" } else { "Cmd+Alt+F (Ctrl+H on Linux)" };
        if ui
            .add_enabled(!bar.all_files, egui::Button::selectable(bar.replacing, "Replace"))
            .on_hover_text(hover)
            .clicked()
        {
            bar.replacing = !bar.replacing;
            bar.focus = if bar.replacing { FocusRequest::Replacement } else { FocusRequest::Query };
        }
    });
}

/// The replacement field, Replace and Replace all.
fn replace_row(ui: &mut Ui, bar: &mut FindBar) -> Option<FindEvent> {
    let mut event = None;
    ui.horizontal(|ui| {
        let field_width = ui.available_width() - STEP_WIDTH - REPLACE_ALL_WIDTH - 2.0 * ui.spacing().item_spacing.x;
        let field = ui
            .add(egui::TextEdit::singleline(&mut bar.replacement).hint_text("Replace with").desired_width(field_width));
        if bar.focus == FocusRequest::Replacement {
            field.request_focus();
            bar.focus = FocusRequest::Nowhere;
        }
        let has_current = bar.current.is_some();
        if field.lost_focus() && ui.input(|input| input.key_pressed(Key::Escape)) {
            event = Some(FindEvent::Close);
        } else if (field.has_focus() || field.lost_focus())
            && ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Enter))
        {
            event = Some(FindEvent::ReplaceAll);
            bar.focus = FocusRequest::Replacement;
        } else if field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
            event = has_current.then_some(FindEvent::Replace);
            bar.focus = FocusRequest::Replacement;
        }
        let height = ui.spacing().interact_size.y;
        if ui
            .add_enabled(has_current, egui::Button::new("Replace").min_size([STEP_WIDTH, height].into()))
            .on_hover_text("Replace this match and go to the next (Enter)")
            .clicked()
        {
            event = Some(FindEvent::Replace);
        }
        if ui
            .add_enabled(
                !bar.matches.is_empty(),
                egui::Button::new("Replace all").min_size([REPLACE_ALL_WIDTH, height].into()),
            )
            .on_hover_text("Replace every match, one Undo brings them back (Cmd+Enter)")
            .clicked()
        {
            event = Some(FindEvent::ReplaceAll);
        }
    });
    event
}

/// The all-files results under the field; a click opens the hit.
fn results_list(ui: &mut Ui, bar: &mut FindBar, roots: &[PathBuf]) -> Option<FileHit> {
    let (remote_roots, local_roots): (Vec<PathBuf>, Vec<PathBuf>) =
        roots.iter().cloned().partition(|root| crate::remote::is_mirror(root));
    if bar.results.as_ref().is_none_or(|(query, options, _)| !bar.is_current(query, *options)) {
        let results = if bar.query.trim().is_empty() {
            SearchResults::default()
        } else {
            search_files(&local_roots, bar.query(), RESULT_LIMIT)
        };
        bar.results = Some((bar.query.clone(), bar.options, results));
    }
    let searching_remote =
        !remote_roots.is_empty() && !bar.query.trim().is_empty() && poll_remote_search(ui.ctx(), bar, remote_roots);
    let Some((_, _, local)) = &bar.results else {
        return None;
    };
    if bar.query.trim().is_empty() {
        return None;
    }
    let mut results = local.clone();
    let mut remote_error = None;
    match &bar.remote_results {
        Some((query, options, Ok(remote))) if bar.is_current(query, *options) => {
            results.files_searched += remote.files_searched;
            results.truncated |= remote.truncated;
            let room = RESULT_LIMIT.saturating_sub(results.hits.len());
            results.truncated |= remote.hits.len() > room;
            results.hits.extend(remote.hits.iter().take(room).cloned());
        }
        Some((query, options, Err(error))) if bar.is_current(query, *options) => remote_error = Some(error.clone()),
        _ => {}
    }
    ui.separator();
    let summary = if results.hits.is_empty() {
        format!("No matches in {} files", results.files_searched)
    } else {
        format!(
            "{}{} matches in {} files",
            if results.truncated { "First " } else { "" },
            results.hits.len(),
            results.file_count()
        )
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(summary).size(SMALL_TEXT).weak());
        if searching_remote {
            ui.add(egui::Spinner::new().size(10.0));
            ui.label(RichText::new("searching remote folders…").size(SMALL_TEXT).weak());
        }
    });
    if let Some(error) = remote_error {
        ui.label(RichText::new(format!("Remote folders: {error}")).size(SMALL_TEXT).color(ui.visuals().warn_fg_color));
    }
    let mut clicked = None;
    egui::ScrollArea::vertical().id_salt("find-results").max_height(RESULTS_HEIGHT).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for hit in &results.hits {
            if result_row(ui, hit).clicked() {
                clicked = Some(hit.clone());
            }
        }
    });
    clicked
}

/// Keep the remote search in step with the query: pick up a finished one,
/// and start a new one when none is running. One search at a time, so fast
/// typing does not start a search per key. True while results for the
/// current query are still coming.
fn poll_remote_search(ctx: &Context, bar: &mut FindBar, remote_roots: Vec<PathBuf>) -> bool {
    let finished =
        bar.remote_search.as_ref().and_then(|(query, options, task)| Some((query.clone(), *options, task.take()?)));
    if let Some((query, options, result)) = finished {
        bar.remote_search = None;
        bar.remote_results = Some((query, options, result.map_err(|error| error.to_string())));
    }
    let current = bar.remote_results.as_ref().is_some_and(|(query, options, _)| bar.is_current(query, *options));
    if current {
        return false;
    }
    if bar.remote_search.is_none() {
        let query = bar.query.clone();
        let options = bar.options;
        let task_query = query.clone();
        let task = spawn_with_repaint(ctx, move || {
            search_remote_files(&remote_roots, Query { text: &task_query, options }, RESULT_LIMIT)
        });
        bar.remote_search = Some((query, options, task));
    }
    true
}

/// One hit: the file's path under its folder, then the line and its text.
fn result_row(ui: &mut Ui, hit: &FileHit) -> egui::Response {
    let relative = hit.path.strip_prefix(&hit.root).unwrap_or(&hit.path);
    let place = match &hit.kind {
        HitKind::Name => format!("{}  (file name)", relative.display()),
        HitKind::Text { line, .. } => format!("{}:{line}", relative.display()),
    };
    let text = match &hit.kind {
        HitKind::Name => name_text(&hit.path),
        HitKind::Text { excerpt, .. } => excerpt.clone(),
    };
    let label = format!("{place}\n{text}");
    ui.add(egui::Button::new(RichText::new(label).size(SMALL_TEXT)).frame(false).wrap())
        .on_hover_text(crate::remote::display(&hit.path))
}
