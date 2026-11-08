//! Draw the editor and the preview, and apply editor keyboard shortcuts.

use std::ops::Range;
use std::path::{Path, PathBuf};

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::{TextEditOutput, TextEditState};
use eframe::egui::{self, Align, Align2, Color32, Context, FontId, Id, Key, Modifiers, TextBuffer, Ui, pos2};

use super::highlighting::{HighlightStyle, Syntax, highlighted};
use super::line_ops::{Direction, Selection, duplicate_lines, insert_text, move_lines};
use super::text_file::Document;
use super::text_stats::{TextStats, text_stats};
use crate::ai::mention_list::{self, Suggestion};
use crate::icons::{Icon, icon_button, small_icon_button_on};
use crate::markdown::blocks::{Block, SourceBlock};
use crate::markdown::render::page_text;
use crate::markdown::render::{
    CodeAction, render_top_block, render_top_table, set_image_folder, take_cell_click, take_code_action,
    take_play_request, top_gap,
};
use crate::markdown::table_edit::{TableAction, TableCell};
use crate::voice::view::{MemoAction, take_memo_action};

/// Widest the preview text gets, for comfortable reading.
const PREVIEW_MAX_WIDTH: f32 = 760.0;
/// Space (x, y) between the page edge and the editor text.
const EDITOR_MARGIN: (i8, i8) = (16, 12);
/// Space (x, y) between the page edge and the preview text.
const PREVIEW_MARGIN: (i8, i8) = (32, 24);
/// Space between the line numbers and the editor text.
const GUTTER_GAP: f32 = 12.0;
/// Widest the line-number gutter gets (an egui margin is at most 127).
const GUTTER_MAX_WIDTH: f32 = 100.0;
/// Opacity of the warning color behind a find match, and of the accent
/// behind the current one.
const MATCH_TINT: f32 = 0.35;
const CURRENT_MATCH_TINT: f32 = 0.55;

/// Find matches to show in the editor.
#[derive(Clone, Copy, Debug)]
pub struct FindHighlight<'a> {
    /// Byte ranges, in order.
    pub matches: &'a [Range<usize>],
    /// Index into `matches` of the one being looked at.
    pub current: Option<usize>,
    /// Scroll the current match into view this frame.
    pub scroll_to_current: bool,
}

/// The editor widget's id, shared by drawing, focus and the line shortcuts.
fn editor_id() -> Id {
    Id::unique("rubrique-editor")
}

/// The editor's cursor, selection and undo history for a tab that is not
/// shown. There is one editor widget, so each tab keeps its own copy.
#[derive(Clone)]
pub struct EditorState(Box<TextEditState>);

impl std::fmt::Debug for EditorState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EditorState")
    }
}

/// Take the editor's state out of egui, for the tab being left.
pub fn take_editor_state(ctx: &Context) -> Option<EditorState> {
    let state = TextEditState::load(ctx, editor_id())?;
    ctx.data_mut(|data| data.remove::<TextEditState>(editor_id()));
    Some(EditorState(Box::new(state)))
}

/// Give the editor the state of the tab being shown; `None` starts fresh
/// (so Undo never reaches into another file).
pub fn restore_editor_state(ctx: &Context, state: Option<EditorState>) {
    match state {
        Some(EditorState(state)) => state.store(ctx, editor_id()),
        None => ctx.data_mut(|data| data.remove::<TextEditState>(editor_id())),
    }
}

/// Draw the text editor, coloring `syntax` when there is one, numbering
/// the lines when `line_numbers` is set, and tinting the `find` matches.
/// Typing `@ai-` or `@tmux-` suggests `mentions`. Returns status-bar stats for the cursor.
pub fn show_editor(
    ui: &mut Ui,
    document: &mut Document,
    syntax: Option<Syntax>,
    line_numbers: bool,
    find: Option<FindHighlight<'_>>,
    mentions: &[Suggestion],
) -> Option<TextStats> {
    let available = ui.available_size();
    let style = HighlightStyle::from_ui(ui);
    let gutter = line_numbers.then(|| Gutter::measure(ui, &document.text));
    let margin = egui::Margin {
        left: EDITOR_MARGIN.0 + gutter.as_ref().map_or(0, |gutter| gutter.width),
        right: EDITOR_MARGIN.0,
        top: EDITOR_MARGIN.1,
        bottom: EDITOR_MARGIN.1,
    };
    let list_key = mention_list::take_key(ui.ctx(), editor_id());
    let output = egui::ScrollArea::both()
        // One scroll position per file, so each tab keeps its place.
        .id_salt(("editor-scroll", &document.path))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let editor = egui::TextEdit::multiline(&mut document.text)
                .id(editor_id())
                .code_editor()
                // No border: the editor is the page itself.
                .frame(egui::Frame::NONE.inner_margin(margin))
                .lock_focus(true)
                .desired_width(f32::INFINITY)
                .min_size(available);
            // Plain text goes through the layouter too, for the editor's line height.
            let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, wrap_width: f32| {
                let mut job = highlighted(ui.ctx(), buffer.as_str(), &style, syntax);
                job.wrap.max_width = wrap_width;
                ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
            };
            let output = editor.layouter(&mut layouter).show(ui);
            if let Some(gutter) = &gutter {
                gutter.paint(ui, &output);
            }
            if let Some(find) = find {
                paint_matches(ui, &output, &document.text, find);
            }
            // A jump from the Structure panel: its line at the top of the page.
            if let Some(index) = take_editor_scroll(ui.ctx()) {
                let line = output.galley.pos_from_cursor(CCursor::new(index)).translate(output.galley_pos.to_vec2());
                ui.scroll_to_rect(line, Some(Align::TOP));
            }
            output
        })
        .inner;
    let mention_added = mention_list::show(ui, editor_id(), &output, &mut document.text, mentions, list_key);
    if output.response.changed() || mention_added {
        document.on_text_changed();
    }
    // While a file is dragged over the editor, the cursor follows the
    // pointer, so the drop lands where it shows.
    let dragging_files = ui.ctx().input(|input| !input.raw.hovered_files.is_empty());
    if dragging_files
        && let Some(pointer) = ui.ctx().pointer_latest_pos()
        && output.response.response.rect.contains(pointer)
    {
        let cursor = output.galley.cursor_from_pos(pointer - output.galley_pos);
        set_editor_cursor(ui.ctx(), cursor.index.0);
    }
    let range = output.cursor_range?;
    let sorted = range.as_sorted_char_range();
    Some(text_stats(&document.text, range.primary.index.0, sorted.start.0..sorted.end.0))
}

/// Tint the find matches behind the text, the current one in the accent,
/// and scroll to the current one when asked.
fn paint_matches(ui: &Ui, output: &TextEditOutput, text: &str, find: FindHighlight<'_>) {
    let visuals = ui.visuals();
    let tint = visuals.warn_fg_color.gamma_multiply(MATCH_TINT);
    let current_tint = visuals.hyperlink_color.gamma_multiply(CURRENT_MATCH_TINT);
    let clip = ui.clip_rect();
    // Byte to character conversion, walking forward through the sorted matches.
    let mut walked = (0_usize, 0_usize);
    let mut to_char = |byte: usize| {
        let (from_byte, from_char) = walked;
        let chars = from_char + text.get(from_byte..byte).map_or(0, |between| between.chars().count());
        walked = (byte, chars);
        chars
    };
    for (index, range) in find.matches.iter().enumerate() {
        let start = to_char(range.start);
        let end = to_char(range.end);
        let is_current = find.current == Some(index);
        let mut current_rect: Option<egui::Rect> = None;
        for rect in match_rects(&output.galley, start, end) {
            let rect = rect.translate(output.galley_pos.to_vec2());
            if is_current {
                current_rect = Some(current_rect.map_or(rect, |whole| whole.union(rect)));
            }
            if rect.intersects(clip) {
                ui.painter().rect_filled(rect, 2, if is_current { current_tint } else { tint });
            }
        }
        if is_current
            && find.scroll_to_current
            && let Some(rect) = current_rect
        {
            ui.scroll_to_rect(rect.expand(24.0), Some(Align::Center));
        }
    }
}

/// The rectangles a match covers, one per row it touches, in galley space.
fn match_rects(galley: &egui::Galley, start: usize, end: usize) -> Vec<egui::Rect> {
    let from = galley.pos_from_cursor(CCursor::new(start));
    let to = galley.pos_from_cursor(CCursor::new(end));
    if (from.min.y - to.min.y).abs() < 0.5 {
        return vec![egui::Rect::from_min_max(from.min, egui::pos2(to.max.x.max(from.min.x + 2.0), from.max.y))];
    }
    let mut rects = Vec::new();
    for row in &galley.rows {
        let rect = row.rect();
        if rect.bottom() <= from.min.y + 0.5 || rect.top() >= to.max.y - 0.5 {
            continue;
        }
        let left = if rect.top() <= from.min.y + 0.5 { from.min.x } else { rect.left() };
        let right = if rect.bottom() >= to.max.y - 0.5 { to.max.x } else { rect.right() };
        rects.push(egui::Rect::from_min_max(egui::pos2(left, rect.top()), egui::pos2(right, rect.bottom())));
    }
    rects
}

/// Line numbers to the left of the editor text, one per line of the file.
/// A wrapped line is numbered once, on its first row.
struct Gutter {
    font: FontId,
    color: Color32,
    /// Extra left margin for the numbers and the gap after them.
    width: i8,
}

impl Gutter {
    fn measure(ui: &Ui, text: &str) -> Gutter {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let digit = ui.ctx().fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
        let width = gutter_width(digit, line_count(text));
        // Clamped to 0..=100 first, so the cast cannot truncate.
        #[expect(clippy::cast_possible_truncation, reason = "clamped to the range")]
        let width = width.clamp(0.0, GUTTER_MAX_WIDTH) as i8;
        Gutter { font, color: ui.visuals().weak_text_color(), width }
    }

    /// Paint the numbers next to the rows that start a line.
    fn paint(&self, ui: &Ui, output: &TextEditOutput) {
        let right = output.galley_pos.x - GUTTER_GAP;
        let clip = ui.clip_rect();
        let painter = ui.painter();
        let mut number = 1_usize;
        let mut starts_line = true;
        for row in &output.galley.rows {
            let top = output.galley_pos.y + row.pos.y;
            let height = row.row.size.y;
            if starts_line && top <= clip.bottom() && top + height >= clip.top() {
                // Rows are taller than the font, with the text in the middle:
                // center the number on the row so it lines up with the text.
                let middle = pos2(right, top + height / 2.0);
                painter.text(middle, Align2::RIGHT_CENTER, number, self.font.clone(), self.color);
            }
            starts_line = row.ends_with_newline;
            if row.ends_with_newline {
                number += 1;
            }
        }
    }
}

/// Lines in `text`: one more than its line breaks.
fn line_count(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'\n').count() + 1
}

/// Width of the numbers (at least two digits) and the gap after them.
fn gutter_width(digit_width: f32, lines: usize) -> f32 {
    let digits = lines.to_string().len().max(2);
    #[expect(clippy::cast_precision_loss, reason = "a handful of digits")]
    let numbers = digit_width * digits as f32;
    (numbers + GUTTER_GAP).ceil()
}

/// A top-level block, or one table cell of it, being edited as Markdown
/// inside the preview.
#[derive(Debug)]
pub struct BlockEdit {
    /// Which top-level block.
    pub index: usize,
    /// The source range being replaced (the block, or the cell's text)
    /// when editing started.
    pub range: std::ops::Range<usize>,
    /// The Markdown being typed.
    pub text: String,
    /// Focus is given to the box once, when editing starts.
    pub focused: bool,
    /// `Some` when only this cell of a table is edited, not the whole block.
    pub cell: Option<TableCell>,
    /// A new block typed just before block `index` (at the end of the file
    /// when `index` is past the last block); `range` is empty there.
    pub inserting: bool,
}

/// What the user did in the preview.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewEvent {
    /// Clicked a block: edit it.
    StartEdit(usize),
    /// Clicked a cell of a table: edit that cell alone.
    StartCellEdit { index: usize, cell: TableCell },
    /// A +/- button under the table whose cell is being edited.
    Table { index: usize, action: TableAction },
    /// Clicked the empty space under the last block: type a new block there.
    StartNewBlock,
    /// Done with the block being edited (Esc, Cmd+Enter, Done, or a click elsewhere).
    FinishEdit,
    /// The cross beside a block, or Delete while editing it: remove the block.
    DeleteBlock(usize),
    /// Clicked the line in the gap above block `index`: type a new block there.
    InsertBlock(usize),
    /// Copy or Prettify on a code block inside top-level block `index`.
    Code { index: usize, action: CodeAction },
    /// Play on a video in the preview.
    PlayVideo(PathBuf),
    /// A button on a voice memo.
    Memo(MemoAction),
}

/// What the preview reports after drawing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewOutput {
    pub event: Option<PreviewEvent>,
    /// The top-level block under the pointer, if any.
    pub hovered: Option<usize>,
}

/// Opacity of the accent behind a block under the mouse.
const HOVER_TINT: f32 = 0.06;
/// Least height of the clickable space under the last block.
const NEW_BLOCK_SPACE: f32 = 48.0;
/// Width of the outline around the block being edited.
const EDIT_OUTLINE: f32 = 1.5;
/// Space between a table and the +/- buttons under it.
const TABLE_BUTTONS_GAP: f32 = 4.0;
/// Space between a block and its hover / edit background.
const BLOCK_PAD: i8 = 6;
/// Side of the delete cross beside a block under the mouse.
const DELETE_BUTTON_SIZE: f32 = 20.0;
/// Space between a block's hover background and its delete cross.
const DELETE_BUTTON_GAP: f32 = 4.0;
/// Width of the insert line drawn in a gap between blocks under the mouse.
const INSERT_LINE_WIDTH: f32 = 1.5;
/// Radius of the circled + in the middle of the insert line.
const INSERT_BUTTON_RADIUS: f32 = 9.0;

/// Draw the rendered Markdown, one clickable block at a time; the block in
/// `editing` shows as a Markdown text box instead, where typing `@ai-`
/// suggests `mentions`.
pub fn show_preview(
    ui: &mut Ui,
    path: &Path,
    blocks: &[SourceBlock],
    mut editing: Option<&mut BlockEdit>,
    mentions: &[Suggestion],
) -> PreviewOutput {
    let mut event = None;
    let mut hovered = None;
    let scroll_target = take_preview_scroll(ui.ctx());
    // Source offset of the first block still on screen, for the Structure panel.
    let mut top_offset = None;
    // Relative image paths are next to the file.
    set_image_folder(ui.ctx(), path.parent().map(Path::to_path_buf));
    egui::ScrollArea::vertical().id_salt(("preview-scroll", path)).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(PREVIEW_MARGIN.0, PREVIEW_MARGIN.1)).show(ui, |ui| {
            ui.set_max_width(PREVIEW_MAX_WIDTH.min(ui.available_width()));
            for (index, source) in blocks.iter().enumerate() {
                let gap = top_gap(&source.block) - f32::from(BLOCK_PAD) * 2.0;
                // Bottom of the block above (the first block has the page margin above it).
                let above = ui.cursor().top();
                if index > 0 {
                    ui.add_space(gap);
                }
                let found = match editing.as_deref_mut() {
                    // A new block typed in this gap: its box, then the gap again.
                    Some(edit) if edit.inserting && edit.index == index => {
                        let found = edit_box(ui, edit, mentions);
                        ui.add_space(gap.max(0.0));
                        found
                    }
                    _ => insert_line(ui, index, above, ui.cursor().top()),
                };
                if event.is_none() {
                    event = found;
                }
                let top = ui.cursor().top();
                let found = match editing.as_deref_mut() {
                    Some(edit) if edit.index == index && !edit.inserting => match edit.cell {
                        Some(cell) => cell_edit_block(ui, index, source, edit, cell),
                        None => edit_box(ui, edit, mentions),
                    },
                    _ => clickable_block(ui, index, source),
                };
                let span = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top());
                if ui.rect_contains_pointer(span) {
                    hovered = Some(index);
                }
                if scroll_target == Some(index) {
                    ui.scroll_to_rect(span, Some(Align::TOP));
                }
                if top_offset.is_none() && span.bottom() > ui.clip_rect().top() {
                    top_offset = Some(source.range.start);
                }
                if event.is_none() {
                    event = found;
                }
            }
            // Under the last block: the new block being typed, or empty
            // space that starts one when clicked.
            let found = match editing {
                Some(edit) if edit.index >= blocks.len() => edit_box(ui, edit, mentions),
                Some(_) => None,
                None => new_block_space(ui),
            };
            if event.is_none() {
                event = found;
            }
        });
    });
    ui.ctx().data_mut(|data| match top_offset {
        Some(offset) => {
            data.insert_temp(preview_top_id(path), offset);
        }
        None => data.remove::<usize>(preview_top_id(path)),
    });
    PreviewOutput { event, hovered }
}

/// Where a jump from the Structure panel waits for the next preview frame.
fn preview_scroll_id() -> Id {
    Id::unique("rubrique-preview-scroll-to-block")
}

/// Where the preview of `path` keeps the source offset of its first block on screen.
fn preview_top_id(path: &Path) -> Id {
    Id::unique(("rubrique-preview-top", path))
}

/// Scroll the preview so top-level block `index` is at the top, on the next frame.
pub fn scroll_preview_to_block(ctx: &Context, index: usize) {
    ctx.data_mut(|data| data.insert_temp(preview_scroll_id(), index));
}

/// The block the preview was asked to scroll to, if any (taken once).
fn take_preview_scroll(ctx: &Context) -> Option<usize> {
    ctx.data_mut(|data| {
        let index = data.get_temp::<usize>(preview_scroll_id());
        data.remove::<usize>(preview_scroll_id());
        index
    })
}

/// Byte offset in the source of the first block on screen in the last
/// drawn preview of `path`, so the Structure panel can mark that section.
pub fn preview_top_offset(ctx: &Context, path: &Path) -> Option<usize> {
    ctx.data(|data| data.get_temp::<usize>(preview_top_id(path)))
}

/// The rest of the page under the last block. A click there starts a new
/// block at the end of the file.
fn new_block_space(ui: &mut Ui) -> Option<PreviewEvent> {
    let size = egui::vec2(ui.available_width(), ui.available_height().max(NEW_BLOCK_SPACE));
    let (_, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::Text);
    response.clicked().then_some(PreviewEvent::StartNewBlock)
}

/// The gap above block `index`, from `above` (the bottom of the block over
/// it) to `below` (its top). Under the mouse it shows an accent line with
/// a plus in the middle; a click there types a new block in that gap. The gap
/// between two paragraphs is only a few pixels, so the line also takes the
/// padding of the blocks on both sides (which leave that padding to it).
fn insert_line(ui: &Ui, index: usize, above: f32, below: f32) -> Option<PreviewEvent> {
    let pad = f32::from(BLOCK_PAD);
    // Above the first block there is no block padding, only the page margin.
    let top = if index == 0 { below - f32::from(PREVIEW_MARGIN.1) / 2.0 } else { above - pad };
    let zone = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=below + pad);
    let response = ui.interact(zone, ui.scope_id().with(("insert-block", index)), egui::Sense::click());
    if !response.hovered() {
        return None;
    }
    let accent = ui.visuals().hyperlink_color;
    let painter = ui.painter();
    let center = zone.center();
    let line = egui::Stroke::new(INSERT_LINE_WIDTH, accent);
    painter.hline(zone.left()..=center.x - INSERT_BUTTON_RADIUS, center.y, line);
    painter.hline(center.x + INSERT_BUTTON_RADIUS..=zone.right(), center.y, line);
    painter.circle(center, INSERT_BUTTON_RADIUS, ui.visuals().panel_fill, line);
    let arm = INSERT_BUTTON_RADIUS * 0.5;
    painter.hline(center.x - arm..=center.x + arm, center.y, line);
    painter.vline(center.x, center.y - arm..=center.y + arm, line);
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Insert a block here");
    response.clicked().then_some(PreviewEvent::InsertBlock(index))
}

/// A rendered block: tinted under the mouse, with a delete cross beside its
/// top right corner; a click (not a text selection or a link) asks to edit it.
fn clickable_block(ui: &mut Ui, index: usize, source: &SourceBlock) -> Option<PreviewEvent> {
    let background = ui.painter().add(egui::Shape::Noop);
    let response = egui::Frame::new().inner_margin(BLOCK_PAD).show(ui, |ui| {
        ui.set_width(ui.available_width());
        render_top_block(ui, &source.block);
    });
    let rect = response.response.rect;
    // The padding above and below belongs to the insert lines in the gaps.
    let own = rect.shrink2(egui::vec2(0.0, f32::from(BLOCK_PAD)));
    let hovered = ui.rect_contains_pointer(own);
    // The cross sits in the page margin, just right of the block, so it never
    // covers the block's own buttons (a code block's Copy is at its top right).
    let delete_rect = egui::Rect::from_min_size(
        pos2(rect.right() + DELETE_BUTTON_GAP, rect.top()),
        egui::vec2(DELETE_BUTTON_SIZE, DELETE_BUTTON_SIZE),
    );
    // Moving from the block to its cross keeps both shown.
    if ui.rect_contains_pointer(own.union(delete_rect)) {
        let fill = ui.visuals().hyperlink_color.gamma_multiply(HOVER_TINT);
        ui.painter().set(background, egui::Shape::rect_filled(rect, 6, fill));
        let id = ui.scope_id().with(("delete-block", index));
        if small_icon_button_on(ui, delete_rect, id, Icon::Close, "Delete block", "Delete this block").clicked() {
            return Some(PreviewEvent::DeleteBlock(index));
        }
    }
    // A code block button was clicked: that, not editing the block.
    if let Some(action) = take_code_action(ui.ctx()) {
        return Some(PreviewEvent::Code { index, action });
    }
    if let Some(action) = take_memo_action(ui.ctx()) {
        return Some(PreviewEvent::Memo(action));
    }
    if let Some(path) = take_play_request(ui.ctx()) {
        return Some(PreviewEvent::PlayVideo(path));
    }
    // A click (pointer released without dragging) that did not open a link;
    // drags select text for notes instead. On a table cell, that cell alone.
    let opened_link = opened_link(ui.ctx());
    let cell = take_cell_click(ui.ctx());
    if opened_link {
        return None;
    }
    if let Some(cell) = cell {
        clear_text_selection(ui);
        return Some(PreviewEvent::StartCellEdit { index, cell });
    }
    if hovered && ui.input(|input| input.pointer.any_click()) {
        clear_text_selection(ui);
        if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
            let at = ((pointer.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            ui.ctx().data_mut(|data| data.insert_temp(block_click_id(), at));
        }
        return Some(PreviewEvent::StartEdit(index));
    }
    None
}

/// Where the click that opened a block for editing leaves how far down the
/// block it was (0 at the top, 1 at the bottom), for the box's first frame.
fn block_click_id() -> Id {
    Id::unique("rubrique-block-click-at")
}

/// How far down the block the opening click was, taken once.
fn take_block_click(ctx: &Context) -> Option<f32> {
    ctx.data_mut(|data| {
        let at = data.get_temp::<f32>(block_click_id());
        data.remove::<f32>(block_click_id());
        at
    })
}

/// Character index of the start of the line `at` of the way down `text`
/// (0 the first line, 1 the last).
fn line_start_at(text: &str, at: f32) -> usize {
    let lines: Vec<&str> = text.split('\n').collect();
    #[expect(clippy::cast_precision_loss, reason = "a line count")]
    let count = lines.len() as f32;
    // `at` is within 0..=1, so the product is within 0..=count.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "floored and within the line count")]
    let line = ((at.clamp(0.0, 1.0) * count).floor() as usize).min(lines.len() - 1);
    lines[..line].iter().map(|line| line.chars().count() + 1).sum()
}

/// Put the block box's cursor at the start of the line the opening click
/// was on, so the box opens where the user looked (egui would put the
/// cursor at the end of the text and scroll there, far down a long list).
fn place_block_cursor(ctx: &Context, text: &str, at: f32) {
    let mut state = TextEditState::load(ctx, block_editor_id()).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(line_start_at(text, at)))));
    state.store(ctx, block_editor_id());
}

/// Whether a link was opened this frame (then its click edits nothing).
fn opened_link(ctx: &Context) -> bool {
    ctx.output(|output| output.commands.iter().any(|command| matches!(command, egui::OutputCommand::OpenUrl(_))))
}

/// Drop the preview's text selection, which a click leaves behind.
fn clear_text_selection(ui: &Ui) {
    ui.ctx().plugin::<egui::text_selection::LabelSelectionState>().lock().clear_selection();
}

/// A table with one cell being edited: a box with the cell's Markdown in
/// that cell (wrapping like the cell), and +/- buttons for its row and
/// column under the table. Enter, Esc or a click outside the box finishes;
/// a click on another cell moves there.
fn cell_edit_block(
    ui: &mut Ui,
    index: usize,
    source: &SourceBlock,
    edit: &mut BlockEdit,
    cell: TableCell,
) -> Option<PreviewEvent> {
    let Block::Table(table) = &source.block else {
        return Some(PreviewEvent::FinishEdit);
    };
    let mut finished = false;
    let mut box_rect = egui::Rect::NOTHING;
    let mut action = None;
    egui::Frame::new().inner_margin(BLOCK_PAD).show(ui, |ui| {
        ui.set_width(ui.available_width());
        render_top_table(ui, table, cell, &mut |ui| {
            // A cell is one line of Markdown, but a long one wraps like the
            // rendered cell does, so the box is a growing multi-line one
            // where Enter finishes instead of adding a line.
            let entered = ui.memory(|memory| memory.has_focus(block_editor_id()))
                && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
            let output = egui::TextEdit::multiline(&mut edit.text)
                .id(block_editor_id())
                .font(FontId::proportional(page_text().body_size))
                .desired_width(f32::INFINITY)
                .desired_rows(1)
                .show(ui);
            if !edit.focused {
                output.response.request_focus();
                // A cell at the far right of a wide table may be scrolled away.
                output.response.scroll_to_me(None);
                edit.focused = true;
            }
            // Esc and Tab take the focus away.
            finished = entered || output.response.lost_focus();
            box_rect = output.response.rect;
        });
        ui.add_space(TABLE_BUTTONS_GAP);
        action = table_buttons(ui, cell, table.align.len());
    });
    if let Some(action) = action {
        return Some(PreviewEvent::Table { index, action });
    }
    let opened_link = opened_link(ui.ctx());
    let next_cell = take_cell_click(ui.ctx());
    if opened_link {
        return None;
    }
    if let Some(cell) = next_cell {
        clear_text_selection(ui);
        return Some(PreviewEvent::StartCellEdit { index, cell });
    }
    let clicked_outside = ui.input(|input| input.pointer.any_click()) && !ui.rect_contains_pointer(box_rect);
    (finished || clicked_outside).then_some(PreviewEvent::FinishEdit)
}

/// The +/- buttons for the row and column of `cell` in a table with
/// `columns` columns: add a row under it, remove its row (not the header),
/// add a column after it, remove its column (not the last one).
fn table_buttons(ui: &mut Ui, cell: TableCell, columns: usize) -> Option<TableAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Enter, Esc or a click outside to finish").small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Right to left: the last button sits at the right edge.
            if columns > 1 && icon_button(ui, Icon::ColumnMinus, "Remove column", "Remove this column", false).clicked()
            {
                action = Some(TableAction::RemoveColumn);
            }
            if icon_button(ui, Icon::ColumnPlus, "Add column", "Add a column after this one", false).clicked() {
                action = Some(TableAction::AddColumn);
            }
            if cell.row > 0 && icon_button(ui, Icon::RowMinus, "Remove row", "Remove this row", false).clicked() {
                action = Some(TableAction::RemoveRow);
            }
            if icon_button(ui, Icon::RowPlus, "Add row", "Add a row under this one", false).clicked() {
                action = Some(TableAction::AddRow);
            }
        });
    });
    action
}

/// The block being edited: its Markdown in an outlined box.
fn edit_box(ui: &mut Ui, edit: &mut BlockEdit, mentions: &[Suggestion]) -> Option<PreviewEvent> {
    let visuals = ui.visuals().clone();
    let mut event = None;
    let frame = egui::Frame::new()
        .fill(visuals.text_edit_bg_color())
        .stroke(egui::Stroke::new(EDIT_OUTLINE, visuals.hyperlink_color))
        .corner_radius(8)
        .inner_margin(BLOCK_PAD + 2);
    let response = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        // An existing block is outlined where it was; only a new one needs a name.
        if edit.range.is_empty() {
            ui.label(egui::RichText::new("New block (Markdown)").small().weak());
        }
        let rows = edit.text.lines().count().max(1);
        let list_key = mention_list::take_key(ui.ctx(), block_editor_id());
        // The page's code size on the page's lines, so the text keeps its size
        // and the block about its height when it opens; egui adds the extra
        // line spacing to the font's own rows.
        let page = page_text();
        let font = FontId::monospace(page.code_size);
        let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
        ui.spacing_mut().extra_text_line_spacing = page.line_height - row_height;
        if !edit.focused
            && let Some(at) = take_block_click(ui.ctx())
        {
            place_block_cursor(ui.ctx(), &edit.text, at);
        }
        let output = egui::TextEdit::multiline(&mut edit.text)
            .id(block_editor_id())
            .code_editor()
            .font(font)
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .show(ui);
        mention_list::show(ui, block_editor_id(), &output, &mut edit.text, mentions, list_key);
        if !edit.focused {
            output.response.request_focus();
            edit.focused = true;
        }
        // egui's text box drops focus on Esc by itself, so look for that too.
        let escaped = output.response.lost_focus() && ui.input(|input| input.key_pressed(Key::Escape));
        let finish_keys = escaped
            || (output.response.has_focus() && ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Enter)));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Esc, Cmd+Enter or a click outside to finish").small().weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Done").clicked() || finish_keys {
                    event = Some(PreviewEvent::FinishEdit);
                }
                // A new block is not in the file yet: emptying it is enough.
                if !edit.range.is_empty()
                    && ui.small_button("Delete").on_hover_text("Delete this block from the file").clicked()
                {
                    event = Some(PreviewEvent::DeleteBlock(edit.index));
                }
            });
        });
    });
    let clicked_outside =
        ui.input(|input| input.pointer.any_click()) && !ui.rect_contains_pointer(response.response.rect);
    if clicked_outside {
        event = Some(PreviewEvent::FinishEdit);
    }
    event
}

fn block_editor_id() -> Id {
    Id::unique("rubrique-block-editor")
}

/// The editor's selection as sorted character indices, if any text is selected.
pub fn editor_selection(ctx: &Context) -> Option<(usize, usize)> {
    let state = TextEditState::load(ctx, editor_id())?;
    let range = state.cursor.char_range()?.as_sorted_char_range();
    (range.start.0 < range.end.0).then_some((range.start.0, range.end.0))
}

/// The editor's cursor as a character index, when it has one.
pub fn editor_cursor(ctx: &Context) -> Option<usize> {
    let state = TextEditState::load(ctx, editor_id())?;
    Some(state.cursor.char_range()?.primary.index.0)
}

/// Put the editor's cursor at character `index` (no selection).
pub fn set_editor_cursor(ctx: &Context, index: usize) {
    if let Some(mut state) = TextEditState::load(ctx, editor_id()) {
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(index))));
        state.store(ctx, editor_id());
    }
}

/// Put the editor's cursor at character `index`, focus the editor, and
/// scroll that line to the top of the page on the next frame. Works before
/// the editor has drawn this file, unlike [`set_editor_cursor`].
pub fn jump_editor_to(ctx: &Context, index: usize) {
    let mut state = TextEditState::load(ctx, editor_id()).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(index))));
    state.store(ctx, editor_id());
    ctx.data_mut(|data| data.insert_temp(editor_scroll_id(), index));
    focus_editor(ctx);
}

/// Where a jump waits for the editor's next frame.
fn editor_scroll_id() -> Id {
    Id::unique("rubrique-editor-scroll-to-char")
}

/// The character the editor was asked to scroll to, if any (taken once).
fn take_editor_scroll(ctx: &Context) -> Option<usize> {
    ctx.data_mut(|data| {
        let index = data.get_temp::<usize>(editor_scroll_id());
        data.remove::<usize>(editor_scroll_id());
        index
    })
}

/// Select characters `start..end` in the editor, the cursor at the end.
pub fn select_editor_range(ctx: &Context, start: usize, end: usize) {
    if let Some(mut state) = TextEditState::load(ctx, editor_id()) {
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(start), CCursor::new(end))));
        state.store(ctx, editor_id());
    }
}

/// Change the editor's text as one undo step, then put its cursor at
/// character `cursor`: Cmd+Z brings back exactly the text from before
/// `edit`. The editor's own undo only follows typing while it has focus,
/// so an edit made from a bar needs its points added by hand.
pub fn apply_with_undo(ctx: &Context, text: &mut String, cursor: usize, edit: impl FnOnce(&mut String)) {
    let mut state = TextEditState::load(ctx, editor_id()).unwrap_or_default();
    let before = state.cursor.char_range().unwrap_or_default();
    let mut undoer = state.undoer();
    // Closes any typing still being gathered into a point.
    undoer.add_undo(&(before, text.clone()));
    edit(text);
    let after = CCursorRange::one(CCursor::new(cursor));
    // The new text is a point too, so the editor sees no change to gather.
    undoer.add_undo(&(after, text.clone()));
    state.set_undoer(undoer);
    state.cursor.set_char_range(Some(after));
    state.store(ctx, editor_id());
}

/// Give keyboard focus to the editor (e.g. after opening a file).
pub fn focus_editor(ctx: &Context) {
    ctx.memory_mut(|memory| memory.request_focus(editor_id()));
}

/// Line-editing commands bound to keys while the editor has focus.
enum EditorCommand {
    MoveLines(Direction),
    DuplicateLines,
    InsertDate,
}

/// Apply editor shortcuts before the editor is drawn, so the editor itself
/// never sees these keys. Returns true when the text changed.
pub fn handle_editor_shortcuts(ctx: &Context, document: &mut Document) -> bool {
    if !ctx.memory(|memory| memory.has_focus(editor_id())) {
        return false;
    }
    let Some(command) = ctx.input_mut(|input| {
        // Check Shift variants first: egui lets Cmd+Shift+D also match Cmd+D.
        if input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::D) {
            Some(EditorCommand::InsertDate)
        } else if input.consume_key(Modifiers::COMMAND, Key::D) {
            Some(EditorCommand::DuplicateLines)
        } else if input.consume_key(Modifiers::ALT, Key::ArrowUp) {
            Some(EditorCommand::MoveLines(Direction::Up))
        } else if input.consume_key(Modifiers::ALT, Key::ArrowDown) {
            Some(EditorCommand::MoveLines(Direction::Down))
        } else {
            None
        }
    }) else {
        return false;
    };

    let Some(mut state) = TextEditState::load(ctx, editor_id()) else {
        return false;
    };
    let Some(range) = state.cursor.char_range() else {
        return false;
    };
    let sorted = range.as_sorted_char_range();
    let selection = Selection { start: sorted.start.0, end: sorted.end.0 };
    let new_selection = match command {
        EditorCommand::MoveLines(direction) => move_lines(&mut document.text, selection, direction),
        EditorCommand::DuplicateLines => Some(duplicate_lines(&mut document.text, selection)),
        EditorCommand::InsertDate => Some(insert_text(&mut document.text, selection, &crate::date::today_iso())),
    };
    let Some(new_selection) = new_selection else {
        return false;
    };
    let new_range = CCursorRange::two(CCursor::new(new_selection.start), CCursor::new(new_selection.end));
    state.cursor.set_char_range(Some(new_range));
    state.store(ctx, editor_id());
    document.on_text_changed();
    true
}

#[cfg(test)]
mod preview_tests {
    use std::collections::BTreeSet;

    use eframe::egui::{Event, PointerButton, Pos2, RawInput, Rect, Vec2};

    use super::*;
    use crate::markdown::blocks::parse_source_blocks;

    /// Two body rows, then a row with an empty cell (as a new row has), and
    /// an empty last column (as a column just added is).
    const TABLE: &str = "| File | Size |  |\n| --- | --- | --- |\n| a.md | 1 |  |\n| b.md | 2 |  |\n|  | 3 |  |\n";

    /// A headless preview of `TABLE`, drawn one frame at a time.
    struct Preview {
        ctx: Context,
        blocks: Vec<SourceBlock>,
        path: PathBuf,
        time: f64,
    }

    impl Preview {
        fn new() -> Self {
            Preview {
                ctx: Context::default(),
                blocks: parse_source_blocks(TABLE),
                path: PathBuf::from("t.md"),
                time: 0.0,
            }
        }

        /// One frame with `events`, editing `edit`; what the preview reported.
        fn frame(&mut self, events: Vec<Event>, mut edit: Option<&mut BlockEdit>) -> PreviewOutput {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut output = None;
            let mut full = self.ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    output = Some(show_preview(ui, &self.path, &self.blocks, edit.as_deref_mut(), &[]));
                });
            });
            // No screen here: the font texture is never uploaded.
            full.textures_delta.clear();
            output.expect("the panel draws the preview")
        }

        /// Move to `at`, press and release; the event of the release frame.
        fn click(&mut self, at: Pos2, mut edit: Option<&mut BlockEdit>) -> Option<PreviewEvent> {
            self.frame(vec![Event::PointerMoved(at)], edit.as_deref_mut());
            let press = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![press(true)], edit.as_deref_mut());
            self.frame(vec![press(false)], edit).event
        }

        /// Every distinct event a click somewhere in the area gives.
        fn scan(
            &mut self,
            xs: std::ops::Range<i32>,
            ys: std::ops::Range<i32>,
            mut edit: Option<&mut BlockEdit>,
        ) -> Vec<PreviewEvent> {
            let mut events = Vec::new();
            for y in ys.step_by(6) {
                for x in xs.clone().step_by(8) {
                    #[expect(clippy::cast_precision_loss, reason = "screen positions")]
                    let at = pos2(x as f32, y as f32);
                    if let Some(event) = self.click(at, edit.as_deref_mut())
                        && !events.contains(&event)
                    {
                        events.push(event);
                    }
                }
            }
            events
        }
    }

    fn cell(row: usize, column: usize) -> TableCell {
        TableCell { row, column }
    }

    fn cells_clicked(events: &[PreviewEvent]) -> BTreeSet<(usize, usize)> {
        events
            .iter()
            .filter_map(|event| match event {
                PreviewEvent::StartCellEdit { index: 0, cell } => Some((cell.row, cell.column)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_opening_click_picks_the_line_under_it() {
        let text = "- one\n- two\n- three";
        assert_eq!(line_start_at(text, 0.0), 0);
        assert_eq!(line_start_at(text, 0.5), 6);
        assert_eq!(line_start_at(text, 0.99), 12);
        // The bottom edge still lands on the last line, not past it.
        assert_eq!(line_start_at(text, 1.0), 12);
        assert_eq!(line_start_at("", 0.7), 0);
        // Characters, not bytes: egui cursors count characters.
        assert_eq!(line_start_at("éé\nx", 0.6), 3);
    }

    #[test]
    fn the_box_opens_with_the_cursor_on_the_clicked_line() {
        let text = "- one\n- two\n- three\n- four";
        let mut preview = Preview::new();
        preview.blocks = parse_source_blocks(text);
        // The rows of the page that open the list (the only block).
        let opens: Vec<i32> = (0..300)
            .step_by(4)
            .filter(|y| {
                #[expect(clippy::cast_precision_loss, reason = "screen positions")]
                let at = pos2(60.0, *y as f32);
                preview.click(at, None) == Some(PreviewEvent::StartEdit(0))
            })
            .collect();
        let (top, bottom) = (*opens.first().unwrap(), *opens.last().unwrap());
        assert!(bottom - top > 60, "the list spans more than one line: {top}..{bottom}");
        let cursor_after_click = |preview: &mut Preview, y: i32| {
            #[expect(clippy::cast_precision_loss, reason = "screen positions")]
            let at = pos2(60.0, y as f32);
            assert_eq!(preview.click(at, None), Some(PreviewEvent::StartEdit(0)));
            let mut edit = BlockEdit {
                index: 0,
                range: 0..text.len(),
                text: text.to_string(),
                focused: false,
                cell: None,
                inserting: false,
            };
            preview.frame(vec![], Some(&mut edit));
            let state = TextEditState::load(&preview.ctx, block_editor_id()).expect("the box stored its state");
            state.cursor.char_range().expect("the cursor was placed").primary.index.0
        };
        // Near the top: the first line. Near the bottom: the last one.
        assert_eq!(cursor_after_click(&mut preview, top + 4), 0);
        assert_eq!(cursor_after_click(&mut preview, bottom - 4), 20);
    }

    #[test]
    fn a_click_on_a_cell_asks_to_edit_that_cell() {
        let mut preview = Preview::new();
        let events = preview.scan(36..300, 26..140, None);
        let cells = cells_clicked(&events);
        let expected = BTreeSet::from([(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (2, 0), (3, 0), (3, 2)]);
        assert!(cells.is_superset(&expected), "{events:?}");
        // Next to the cells the table itself is still clickable.
        assert!(events.contains(&PreviewEvent::StartEdit(0)), "{events:?}");
    }

    #[test]
    fn a_short_cell_is_clickable_over_the_whole_height_of_its_row() {
        let mut preview = Preview::new();
        let long = "a long cell that wraps over several lines in its column ".repeat(6);
        preview.blocks = parse_source_blocks(&format!("| A | B |\n| - | - |\n| x | {long} |\n"));
        // Down the first column: the short cell `x` should answer along the
        // whole height of the row its tall neighbour sets.
        let mut hits = 0;
        for y in (26..200).step_by(6) {
            #[expect(clippy::cast_precision_loss, reason = "screen positions")]
            let at = pos2(50.0, y as f32);
            if preview.click(at, None) == Some(PreviewEvent::StartCellEdit { index: 0, cell: cell(1, 0) }) {
                hits += 1;
            }
        }
        assert!(hits >= 6, "{hits} rows of 6 px");
    }

    #[test]
    fn the_cell_box_takes_focus_typing_and_enter() {
        let mut preview = Preview::new();
        let mut edit = BlockEdit {
            index: 0,
            range: 2..6,
            text: "File".to_string(),
            focused: false,
            cell: Some(cell(0, 0)),
            inserting: false,
        };
        preview.frame(Vec::new(), Some(&mut edit));
        preview.frame(Vec::new(), Some(&mut edit));
        assert!(edit.focused);
        assert_eq!(preview.ctx.memory(egui::Memory::focused), Some(block_editor_id()));
        let typed = preview.frame(vec![Event::Text("s".to_string())], Some(&mut edit));
        assert_eq!(typed.event, None);
        assert!(edit.text.contains('s') && edit.text != "File", "{}", edit.text);
        let enter = Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let finished = preview.frame(vec![enter], Some(&mut edit));
        assert_eq!(finished.event, Some(PreviewEvent::FinishEdit));
        // Enter finished the edit instead of starting a new line.
        assert!(!edit.text.contains('\n'), "{}", edit.text);
    }

    #[test]
    fn while_editing_a_cell_other_cells_and_the_buttons_are_clickable() {
        let mut preview = Preview::new();
        let mut edit = BlockEdit {
            index: 0,
            range: 42..46,
            text: "a.md".to_string(),
            focused: false,
            cell: Some(cell(1, 0)),
            inserting: false,
        };
        preview.frame(Vec::new(), Some(&mut edit));
        let events = preview.scan(36..780, 26..200, Some(&mut edit));
        let cells = cells_clicked(&events);
        assert!(cells.is_superset(&BTreeSet::from([(0, 0), (0, 1), (1, 1), (2, 0)])), "{events:?}");
        // The box replaces the edited cell: no click on it is reported.
        assert!(!cells.contains(&(1, 0)), "{events:?}");
        let actions: BTreeSet<String> = events
            .iter()
            .filter_map(|event| match event {
                PreviewEvent::Table { index: 0, action } => Some(format!("{action:?}")),
                _ => None,
            })
            .collect();
        assert_eq!(
            actions,
            BTreeSet::from_iter(["AddColumn", "AddRow", "RemoveColumn", "RemoveRow"].map(String::from))
        );
    }

    #[test]
    fn the_header_row_has_no_remove_row_button() {
        let mut preview = Preview::new();
        let mut edit = BlockEdit {
            index: 0,
            range: 2..6,
            text: "File".to_string(),
            focused: false,
            cell: Some(cell(0, 0)),
            inserting: false,
        };
        preview.frame(Vec::new(), Some(&mut edit));
        let events = preview.scan(600..780, 50..200, Some(&mut edit));
        let actions: Vec<TableAction> = events
            .iter()
            .filter_map(|event| match event {
                PreviewEvent::Table { action, .. } => Some(*action),
                _ => None,
            })
            .collect();
        assert!(!actions.contains(&TableAction::RemoveRow), "{actions:?}");
        assert!(actions.contains(&TableAction::AddRow), "{actions:?}");
    }
}
