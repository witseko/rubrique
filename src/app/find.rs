//! Find (Cmd+F) and replace (Cmd+Alt+F, Ctrl+H on Linux): a bar at the top
//! right of the page that searches the open file, or every text file under
//! the explorer's folders, and replaces matches in the open file.

use std::ops::Range;
use std::path::{Path, PathBuf};

use eframe::egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat, TextWrapping};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Align, Color32, Context, FontId, Id, Key, Modifiers, RichText, Sense, Stroke, TextStyle, Ui, Vec2,
    WidgetInfo, pos2, vec2,
};

use super::{OpenFile, RubriqueApp, char_to_byte};
use crate::document::view::{
    FindHighlight, apply_with_undo, editor_selection, focus_editor, match_fill, select_editor_range,
};
use crate::explorer::paths::name_text;
use crate::find::{
    FileHit, HitKind, Query, SearchOptions, SearchResults, char_index, find_all, replace_all, search_files,
    search_remote_files,
};
use crate::remote::task::{Task, spawn_with_repaint};
use crate::theme::style::page_fill;

const FIND_RADIUS: u8 = 8;
/// Space between the bar and the page's top and right edges.
const FIND_MARGIN: f32 = 12.0;
/// Width of the bar as a multiple of its text size: room for a comfortable
/// field next to the count and the buttons.
const WIDTH_PER_TEXT_SIZE: f32 = 40.0;
/// Narrowest the bar gets when the page has room for it.
const MIN_WIDTH: f32 = 340.0;
/// Height of fields, buttons and result rows as a multiple of the text size.
const ROW_PER_TEXT_SIZE: f32 = 2.0;
/// Size of secondary text (the results summary, folder names and line
/// numbers) as a share of the bar's text size.
const SMALL_TEXT_SHARE: f32 = 0.86;
/// Space inside the bar's edge, as a share of the text size.
const PADDING_SHARE: f32 = 0.8;
/// Space between controls, as a share of the text size.
const GAP_SHARE: f32 = 0.45;
/// Space inside fields, buttons and result rows (left and right), as a
/// share of the text size.
const INNER_PADDING_SHARE: f32 = 0.6;
/// Tallest the results list gets before it scrolls, as a share of the page height.
const RESULTS_HEIGHT_SHARE: f32 = 0.6;
/// The match count is sized for this text, so the buttons after it do not
/// move as the numbers change.
const COUNT_SAMPLE: &str = "888 of 888";
/// The line numbers in the results are sized for this many digits.
const LINE_NUMBER_SAMPLE: &str = "88888";
/// Outline of a field with focus and of a switched on option.
const ACCENT_STROKE: f32 = 1.5;
/// Opacity of the weak text color used as the outline of fields and of
/// switched off options: plainly visible in every theme, softer than text.
const OUTLINE_OPACITY: f32 = 0.6;
/// Hits an all-files search stops at.
const RESULT_LIMIT: usize = 300;

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
        let style = ctx.global_style();
        let metrics =
            bar_metrics(TextStyle::Monospace.resolve(&style).size, TextStyle::Body.resolve(&style).size, page.size());
        let at = pos2(page.right() - metrics.width - FIND_MARGIN, page.top() + FIND_MARGIN);
        let mut event = None;
        egui::Area::new(Id::unique("find-bar")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
            scale_style(ui, &metrics);
            let padding = metrics.text * PADDING_SHARE;
            let frame = egui::Frame::popup(ui.style())
                .fill(ui.visuals().panel_fill)
                .corner_radius(FIND_RADIUS)
                .inner_margin(padding);
            frame.show(ui, |ui| {
                // The frame's padding and outline sit around this width.
                ui.set_width(metrics.width - 2.0 * padding - 2.0 * frame.stroke.width);
                let Some(bar) = &mut self.find else {
                    return;
                };
                event = find_row(ui, bar, &metrics);
                options_row(ui, bar, &metrics);
                if bar.replacing && !bar.all_files && event.is_none() {
                    event = replace_row(ui, bar, &metrics);
                }
                if bar.all_files {
                    let roots: Vec<PathBuf> = self.roots.iter().map(|root| root.path.clone()).collect();
                    if let Some(hit) = results_list(ui, bar, &roots, &metrics) {
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

fn replacement_field_id() -> Id {
    Id::unique("find-bar-replacement")
}

/// Sizes in the bar, worked out from the editor font size so the bar reads
/// as comfortably as the text it searches.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BarMetrics {
    /// Size of the bar's text, of the query and of the result lines.
    text: f32,
    /// Size of secondary text: the results summary, folder names and line numbers.
    small: f32,
    /// Height of fields, buttons and result rows.
    row: f32,
    /// Width of the whole bar, its padding included.
    width: f32,
    /// Tallest the results list gets before it scrolls.
    results_height: f32,
}

/// The bar's sizes for an editor font of `editor_size` points and interface
/// text of `body_size` points (the bar's text is never smaller than that),
/// on a page of `page` size. The bar never gets wider than the page.
fn bar_metrics(editor_size: f32, body_size: f32, page: Vec2) -> BarMetrics {
    let text = editor_size.max(body_size);
    let room = (page.x - 2.0 * FIND_MARGIN).max(0.0);
    BarMetrics {
        text,
        small: (text * SMALL_TEXT_SHARE).round(),
        row: (text * ROW_PER_TEXT_SIZE).round(),
        width: (text * WIDTH_PER_TEXT_SIZE).max(MIN_WIDTH).min(room),
        results_height: page.y * RESULTS_HEIGHT_SHARE,
    }
}

/// Size the bar's text, controls and spacing from `metrics`: egui's own
/// sizes are fixed and read small next to a large editor font.
fn scale_style(ui: &mut Ui, metrics: &BarMetrics) {
    let style = ui.style_mut();
    for text_style in [TextStyle::Body, TextStyle::Button] {
        style.text_styles.insert(text_style, FontId::proportional(metrics.text));
    }
    style.text_styles.insert(TextStyle::Small, FontId::proportional(metrics.small));
    let gap = metrics.text * GAP_SHARE;
    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(gap, gap);
    spacing.interact_size.y = metrics.row;
    spacing.button_padding = vec2(metrics.text * INNER_PADDING_SHARE, gap / 2.0);
    // The All files checkbox: a box the size of a letter, the tick half of it.
    spacing.icon_width = metrics.text;
    spacing.icon_width_inner = metrics.text / 2.0;
    spacing.icon_spacing = gap;
}

/// The width `text` takes in `font`.
fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER).size().x
}

/// The outline of a field and of a switched off option: the weak text
/// color, softened, which stands out from the bar in every theme (the
/// theme's border color is too faint on the dark ones).
fn outline_color(visuals: &egui::Visuals) -> Color32 {
    visuals.weak_text_color().gamma_multiply(OUTLINE_OPACITY)
}

/// A single-line field in the editor's font at the bar's text size, on the
/// page color, with an outline that turns to the accent while it has focus.
/// `width` includes the outline and the padding.
fn bar_field<'t>(
    ui: &Ui,
    text: &'t mut String,
    id: Id,
    hint: &str,
    width: f32,
    metrics: &BarMetrics,
) -> egui::TextEdit<'t> {
    let visuals = ui.visuals();
    let stroke = if ui.memory(|memory| memory.has_focus(id)) {
        Stroke::new(ACCENT_STROKE, visuals.hyperlink_color)
    } else {
        Stroke::new(1.0, outline_color(visuals))
    };
    let frame = egui::Frame::new()
        .fill(page_fill(visuals))
        .stroke(stroke)
        .corner_radius(visuals.widgets.inactive.corner_radius)
        .inner_margin(vec2(metrics.text * INNER_PADDING_SHARE, 0.0));
    egui::TextEdit::singleline(text)
        .id(id)
        .hint_text(hint)
        .font(FontId::monospace(metrics.text))
        .frame(frame)
        .desired_width(width)
        .min_size(vec2(0.0, metrics.row))
        .align(egui::Align2::LEFT_CENTER)
}

/// The field, the count, the Aa and W toggles, Previous / Next and the cross.
fn find_row(ui: &mut Ui, bar: &mut FindBar, metrics: &BarMetrics) -> Option<FindEvent> {
    let mut event = None;
    ui.horizontal(|ui| {
        let hint = if bar.all_files { "Find in all files" } else { "Find in this file" };
        let count_width = text_width(ui, COUNT_SAMPLE, FontId::proportional(metrics.text));
        // Five square buttons (two toggles, two arrows, the cross) and the
        // six gaps between the seven controls.
        let field_width = ui.available_width() - count_width - 5.0 * metrics.row - 6.0 * ui.spacing().item_spacing.x;
        let field = ui.add(bar_field(ui, &mut bar.query, query_field_id(), hint, field_width, metrics));
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
        let count = if bar.all_files { String::new() } else { count_text(bar) };
        let visuals = ui.visuals();
        let count_color = if count == NO_MATCHES { visuals.error_fg_color } else { visuals.text_color() };
        ui.add_sized([count_width, metrics.row], egui::Label::new(RichText::new(count).color(count_color)));
        option_toggles(ui, &mut bar.options, metrics);
        if let Some(step) = step_buttons(ui, !bar.all_files && !bar.matches.is_empty(), metrics) {
            event = Some(step);
        }
    });
    event
}

/// Shown as the count when the query matches nothing.
const NO_MATCHES: &str = "No matches";

/// "3 of 12", the total while no match is current, or "No matches".
fn count_text(bar: &FindBar) -> String {
    match (bar.current, bar.matches.len()) {
        (_, 0) if bar.query.is_empty() => String::new(),
        (_, 0) => NO_MATCHES.to_string(),
        (Some(current), total) => format!("{} of {total}", current + 1),
        (None, total) => format!("{total}"),
    }
}

/// The Aa (match case) and W (whole word) toggles.
fn option_toggles(ui: &mut Ui, options: &mut SearchOptions, metrics: &BarMetrics) {
    let square = Vec2::splat(metrics.row);
    let match_case = option_toggle(ui, options.match_case, "Aa", square);
    if match_case.on_hover_text(option_tip("Match case", options.match_case)).clicked() {
        options.match_case = !options.match_case;
    }
    let whole_word = option_toggle(ui, options.whole_word, "W", square);
    if whole_word.on_hover_text(option_tip("Whole word", options.whole_word)).clicked() {
        options.whole_word = !options.whole_word;
    }
}

/// The hover tip of an option toggle: its name and whether it is on.
fn option_tip(name: &str, on: bool) -> String {
    format!("{name}: {}", if on { "on" } else { "off" })
}

/// An on / off button at least `min_size`: filled and outlined in the
/// accent while on, only outlined while off, so the state reads at a glance.
fn option_toggle(ui: &mut Ui, on: bool, label: &str, min_size: Vec2) -> egui::Response {
    let visuals = ui.visuals();
    let (fill, stroke, color) = if on {
        (visuals.selection.bg_fill, Stroke::new(ACCENT_STROKE, visuals.hyperlink_color), visuals.strong_text_color())
    } else {
        (Color32::TRANSPARENT, Stroke::new(1.0, outline_color(visuals)), visuals.text_color())
    };
    let button =
        egui::Button::new(RichText::new(label).color(color)).selected(on).fill(fill).stroke(stroke).min_size(min_size);
    add_steady(ui, button)
}

/// Add `widget` without egui's hover growth: a hovered or pressed button is
/// drawn one point larger on every side, which makes the outlined buttons
/// of the bar look like they jump. Only the hover fill changes here.
fn add_steady(ui: &mut Ui, widget: impl egui::Widget) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.hovered.expansion = 0.0;
        widgets.active.expansion = 0.0;
        ui.add(widget)
    })
    .inner
}

/// Previous, Next and the close cross: square buttons as tall as the field.
fn step_buttons(ui: &mut Ui, has_matches: bool, metrics: &BarMetrics) -> Option<FindEvent> {
    let buttons = [
        (has_matches, "↑", "Previous match", "Previous match (Shift+Enter, Cmd+Shift+G)", FindEvent::Previous),
        (has_matches, "↓", "Next match", "Next match (Enter, Cmd+G)", FindEvent::Next),
        (true, "×", "Close", "Close (Esc)", FindEvent::Close),
    ];
    let mut event = None;
    for (enabled, glyph, name, tip, clicked) in buttons {
        let button = egui::Button::new(glyph).min_size(Vec2::splat(metrics.row));
        let response = ui
            .add_enabled_ui(enabled, |ui| add_steady(ui, button))
            .inner
            .on_hover_text(tip)
            .on_disabled_hover_text(tip);
        // Screen readers announce the name, not the arrow.
        response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, enabled, name));
        if response.clicked() {
            event = Some(clicked);
        }
    }
    event
}

/// Select every character of the single-line field `id`, whose text is `text`.
fn select_all(ctx: &Context, id: Id, text: &str) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    let end = CCursor::new(text.chars().count());
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), end)));
    state.store(ctx, id);
}

/// The All files switch and the Replace switch.
fn options_row(ui: &mut Ui, bar: &mut FindBar, metrics: &BarMetrics) {
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut bar.all_files, "All files")
            .on_hover_text("Search every file in the explorer's folders")
            .changed()
        {
            bar.focus = FocusRequest::Query;
        }
        let hover = if bar.all_files { "Replace works in the open file" } else { "Cmd+Alt+F (Ctrl+H on Linux)" };
        let replace = ui
            .add_enabled_ui(!bar.all_files, |ui| {
                option_toggle(ui, bar.replacing, "Replace", vec2(metrics.row, metrics.row))
            })
            .inner;
        if replace.on_hover_text(hover).on_disabled_hover_text(hover).clicked() {
            bar.replacing = !bar.replacing;
            bar.focus = if bar.replacing { FocusRequest::Replacement } else { FocusRequest::Query };
        }
    });
}

/// The replacement field, Replace and Replace all.
fn replace_row(ui: &mut Ui, bar: &mut FindBar, metrics: &BarMetrics) -> Option<FindEvent> {
    let mut event = None;
    ui.horizontal(|ui| {
        let button_font = TextStyle::Button.resolve(ui.style());
        let button_padding = 2.0 * ui.spacing().button_padding.x;
        let replace_width = text_width(ui, "Replace", button_font.clone()) + button_padding;
        let replace_all_width = text_width(ui, "Replace all", button_font) + button_padding;
        let field_width = ui.available_width() - replace_width - replace_all_width - 2.0 * ui.spacing().item_spacing.x;
        let hint = "Replace with";
        let field = ui.add(bar_field(ui, &mut bar.replacement, replacement_field_id(), hint, field_width, metrics));
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
        let size = vec2(replace_width, metrics.row);
        if ui
            .add_enabled(has_current, egui::Button::new("Replace").min_size(size))
            .on_hover_text("Replace this match and go to the next (Enter)")
            .clicked()
        {
            event = Some(FindEvent::Replace);
        }
        let size = vec2(replace_all_width, metrics.row);
        if ui
            .add_enabled(!bar.matches.is_empty(), egui::Button::new("Replace all").min_size(size))
            .on_hover_text("Replace every match, one Undo brings them back (Cmd+Enter)")
            .clicked()
        {
            event = Some(FindEvent::ReplaceAll);
        }
    });
    event
}

/// The all-files results under the field; a click opens the hit.
fn results_list(ui: &mut Ui, bar: &mut FindBar, roots: &[PathBuf], metrics: &BarMetrics) -> Option<FileHit> {
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
    ui.horizontal(|ui| {
        ui.label(RichText::new(results_summary(&results)).small().weak());
        if searching_remote {
            ui.add(egui::Spinner::new().size(metrics.small));
            ui.label(RichText::new("searching remote folders…").small().weak());
        }
    });
    if let Some(error) = remote_error {
        ui.label(RichText::new(format!("Remote folders: {error}")).small().color(ui.visuals().warn_fg_color));
    }
    let line_number_width = text_width(ui, LINE_NUMBER_SAMPLE, FontId::monospace(metrics.small));
    let mut clicked = None;
    egui::ScrollArea::vertical().id_salt("find-results").max_height(metrics.results_height).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, group) in group_by_file(&results.hits).iter().enumerate() {
            if index > 0 {
                ui.add_space(metrics.text * GAP_SHARE);
            }
            if file_header(ui, group, metrics).clicked() {
                let kind = HitKind::Name;
                clicked = Some(FileHit { path: group.path.to_path_buf(), root: group.root.to_path_buf(), kind });
            }
            for hit in &group.lines {
                let HitKind::Text { line, excerpt, highlight, .. } = &hit.kind else {
                    continue;
                };
                let found = LineMatch { line: *line, excerpt, highlight };
                if line_row(ui, &found, line_number_width, metrics).clicked() {
                    clicked = Some((*hit).clone());
                }
            }
        }
    });
    clicked
}

/// "12 matches in 3 files", "First 300 matches in 40 files" when the search
/// stopped at its limit, or "No matches in 25 files".
fn results_summary(results: &SearchResults) -> String {
    if results.hits.is_empty() {
        return format!("No matches in {}", plural(results.files_searched, "file", "files"));
    }
    let first = if results.truncated { "First " } else { "" };
    let matches = plural(results.hits.len(), "match", "matches");
    format!("{first}{matches} in {}", plural(results.file_count(), "file", "files"))
}

/// "1 match", "2 matches".
fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// One file's hits in the results list.
#[derive(Debug, PartialEq, Eq)]
struct FileGroup<'a> {
    path: &'a Path,
    /// The explorer folder the file was found under.
    root: &'a Path,
    /// The file's name contains the query.
    name_matches: bool,
    /// Hits on lines of the file, in order.
    lines: Vec<&'a FileHit>,
}

/// The hits grouped by file, files in the order of their first hit.
fn group_by_file(hits: &[FileHit]) -> Vec<FileGroup<'_>> {
    let mut groups: Vec<FileGroup<'_>> = Vec::new();
    for hit in hits {
        let index = if let Some(index) = groups.iter().position(|group| group.path == hit.path) {
            index
        } else {
            groups.push(FileGroup { path: &hit.path, root: &hit.root, name_matches: false, lines: Vec::new() });
            groups.len() - 1
        };
        let group = &mut groups[index];
        match hit.kind {
            HitKind::Name => group.name_matches = true,
            HitKind::Text { .. } => group.lines.push(hit),
        }
    }
    groups
}

/// What a file's header says about its hits: "3 matches", "file name", or
/// "file name, 2 matches".
fn group_note(group: &FileGroup<'_>) -> String {
    let lines = plural(group.lines.len(), "match", "matches");
    match (group.name_matches, group.lines.is_empty()) {
        (true, true) => "file name".to_string(),
        (true, false) => format!("file name, {lines}"),
        (false, _) => lines,
    }
}

/// Where a file is: its explorer folder's name and the folders below it
/// that hold the file, like `notes/2024/march`.
fn folder_text(path: &Path, root: &Path) -> String {
    let root_name = name_text(root);
    match path.strip_prefix(root).ok().and_then(Path::parent) {
        Some(parent) if !parent.as_os_str().is_empty() => Path::new(&root_name).join(parent).display().to_string(),
        _ => root_name,
    }
}

/// A file's header in the results: its name, the folder it is in, and what
/// matched. A click opens the file.
fn file_header(ui: &mut Ui, group: &FileGroup<'_>, metrics: &BarMetrics) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), metrics.row), Sense::click());
    let name = name_text(group.path);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, format!("Open {name}")));
    let visuals = ui.visuals();
    let painter = ui.painter();
    let fill = if response.hovered() { visuals.widgets.hovered.weak_bg_fill } else { visuals.faint_bg_color };
    painter.rect_filled(rect, visuals.widgets.inactive.corner_radius, fill);

    let padding = metrics.text * INNER_PADDING_SHARE;
    let note =
        painter.layout_no_wrap(group_note(group), FontId::proportional(metrics.small), visuals.weak_text_color());
    let note_left = rect.right() - padding - note.size().x;
    let format = |size: f32, color: Color32| TextFormat {
        font_id: FontId::proportional(size),
        color,
        valign: Align::Center,
        ..TextFormat::default()
    };
    let mut job = LayoutJob::default();
    job.append(&name, 0.0, format(metrics.text, visuals.strong_text_color()));
    job.append(&folder_text(group.path, group.root), padding, format(metrics.small, visuals.weak_text_color()));
    job.wrap = TextWrapping::truncate_at_width(note_left - rect.left() - 2.0 * padding);
    let title = painter.layout_job(job);
    let title_top = rect.center().y - title.size().y / 2.0;
    painter.galley(pos2(rect.left() + padding, title_top), title, Color32::PLACEHOLDER);
    let note_top = rect.center().y - note.size().y / 2.0;
    painter.galley(pos2(note_left, note_top), note, Color32::PLACEHOLDER);
    response.on_hover_text(crate::remote::display(group.path))
}

/// A matching line in a file, as the results list shows it.
struct LineMatch<'a> {
    /// 1-based line number.
    line: usize,
    excerpt: &'a str,
    /// Byte range of the match in `excerpt`.
    highlight: &'a Range<usize>,
}

/// One matching line under its file's header: the line number, then the
/// line with the match highlighted. A click goes to the match.
fn line_row(ui: &mut Ui, found: &LineMatch<'_>, line_number_width: f32, metrics: &BarMetrics) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), metrics.row), Sense::click());
    let label = format!("Line {}: {}", found.line, found.excerpt);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, &label));
    let visuals = ui.visuals();
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, visuals.widgets.inactive.corner_radius, visuals.widgets.hovered.weak_bg_fill);
    }
    let padding = metrics.text * INNER_PADDING_SHARE;
    let number_font = FontId::monospace(metrics.small);
    let number = painter.layout_no_wrap(found.line.to_string(), number_font, visuals.weak_text_color());
    // Right-aligned, so the lines of a file start in one column.
    let number_right = rect.left() + padding + line_number_width;
    let number_top = rect.center().y - number.size().y / 2.0;
    painter.galley(pos2(number_right - number.size().x, number_top), number, Color32::PLACEHOLDER);

    let text_left = number_right + padding;
    let mut job = excerpt_job(found.excerpt, found.highlight, FontId::monospace(metrics.text), visuals);
    job.wrap = TextWrapping::truncate_at_width(rect.right() - padding - text_left);
    let text = painter.layout_job(job);
    let text_top = rect.center().y - text.size().y / 2.0;
    painter.galley(pos2(text_left, text_top), text, Color32::PLACEHOLDER);
    response
}

/// `excerpt` in `font`, with the `highlight` part on the same tint as a
/// match in the editor, in the strong text color.
fn excerpt_job(excerpt: &str, highlight: &Range<usize>, font: FontId, visuals: &egui::Visuals) -> LayoutJob {
    let plain = TextFormat::simple(font.clone(), visuals.text_color());
    let marked =
        TextFormat { background: match_fill(visuals), ..TextFormat::simple(font, visuals.strong_text_color()) };
    let mut job = LayoutJob::default();
    let parts = (excerpt.get(..highlight.start), excerpt.get(highlight.clone()), excerpt.get(highlight.end..));
    if let (Some(before), Some(found), Some(after)) = parts {
        job.append(before, 0.0, plain.clone());
        job.append(found, 0.0, marked);
        job.append(after, 0.0, plain);
    } else {
        // Not expected (the search gives a range inside the excerpt), but
        // the line still shows, without a highlight.
        job.append(excerpt, 0.0, plain);
    }
    job
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

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: Vec2 = vec2(1200.0, 800.0);

    #[test]
    fn bar_grows_with_the_editor_font_and_fits_the_page() {
        let default = bar_metrics(14.0, 14.0, PAGE);
        let large = bar_metrics(20.0, 14.0, PAGE);
        assert_eq!(default.text, 14.0);
        assert_eq!(large.text, 20.0);
        assert!(large.row > default.row && large.width > default.width && large.small > default.small);
        assert_eq!(default.results_height, PAGE.y * RESULTS_HEIGHT_SHARE);
        // A small editor font does not shrink the bar below interface text.
        assert_eq!(bar_metrics(10.0, 14.0, PAGE), default);
        // A narrow page caps the width: the bar stays on the page.
        let narrow = bar_metrics(28.0, 14.0, vec2(500.0, 800.0));
        assert_eq!(narrow.width, 500.0 - 2.0 * FIND_MARGIN);
    }

    #[test]
    fn count_shows_the_current_match_of_the_total() {
        let mut bar = FindBar::default();
        assert_eq!(count_text(&bar), "");
        bar.query = "milk".into();
        assert_eq!(count_text(&bar), NO_MATCHES);
        bar.matches = vec![0..4, 10..14, 20..24];
        assert_eq!(count_text(&bar), "3");
        bar.current = Some(1);
        assert_eq!(count_text(&bar), "2 of 3");
    }

    #[test]
    fn option_tips_say_whether_the_option_is_on() {
        assert_eq!(option_tip("Match case", true), "Match case: on");
        assert_eq!(option_tip("Whole word", false), "Whole word: off");
    }

    fn text_hit(path: &str, line: usize) -> FileHit {
        let kind = HitKind::Text { line, excerpt: "milk".into(), highlight: 0..4, range: 0..4 };
        FileHit { path: PathBuf::from(path), root: PathBuf::from("/notes"), kind }
    }

    fn name_hit(path: &str) -> FileHit {
        FileHit { path: PathBuf::from(path), root: PathBuf::from("/notes"), kind: HitKind::Name }
    }

    #[test]
    fn results_are_grouped_by_file_in_order() {
        let hits = vec![
            name_hit("/notes/a.md"),
            text_hit("/notes/a.md", 2),
            text_hit("/notes/a.md", 9),
            text_hit("/notes/sub/b.md", 1),
            name_hit("/notes/c.md"),
        ];
        let groups = group_by_file(&hits);
        let summary: Vec<(&Path, bool, Vec<usize>)> = groups
            .iter()
            .map(|group| {
                let lines = group
                    .lines
                    .iter()
                    .filter_map(|hit| match hit.kind {
                        HitKind::Text { line, .. } => Some(line),
                        HitKind::Name => None,
                    })
                    .collect();
                (group.path, group.name_matches, lines)
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (Path::new("/notes/a.md"), true, vec![2, 9]),
                (Path::new("/notes/sub/b.md"), false, vec![1]),
                (Path::new("/notes/c.md"), true, vec![]),
            ]
        );
        let notes: Vec<String> = groups.iter().map(group_note).collect();
        assert_eq!(notes, ["file name, 2 matches", "1 match", "file name"]);
    }

    #[test]
    fn folder_text_names_the_explorer_folder_and_the_folders_below() {
        let root = Path::new("/home/me/notes");
        assert_eq!(folder_text(&root.join("a.md"), root), "notes");
        assert_eq!(
            folder_text(&root.join("2024/march/a.md"), root),
            Path::new("notes/2024/march").display().to_string()
        );
    }

    #[test]
    fn summary_counts_matches_and_files() {
        let mut results = SearchResults { hits: Vec::new(), files_searched: 1, truncated: false };
        assert_eq!(results_summary(&results), "No matches in 1 file");
        results.hits = vec![text_hit("/notes/a.md", 1)];
        assert_eq!(results_summary(&results), "1 match in 1 file");
        results.hits.push(text_hit("/notes/b.md", 1));
        results.truncated = true;
        assert_eq!(results_summary(&results), "First 2 matches in 2 files");
    }

    #[test]
    fn excerpt_job_marks_the_match() {
        let visuals = egui::Visuals::dark();
        let job = excerpt_job("buy milk today", &(4..8), FontId::monospace(14.0), &visuals);
        assert_eq!(job.text, "buy milk today");
        let ranges: Vec<Range<usize>> =
            job.sections.iter().map(|section| section.byte_range.start.0..section.byte_range.end.0).collect();
        assert_eq!(ranges, vec![0..4, 4..8, 8..14]);
        assert_eq!(job.sections[1].format.background, match_fill(&visuals));
        assert_eq!(job.sections[0].format.background, Color32::TRANSPARENT);
        // A range outside the excerpt shows the line without a highlight.
        let job = excerpt_job("milk", &(2..9), FontId::monospace(14.0), &visuals);
        assert_eq!(job.sections.len(), 1);
    }
}
