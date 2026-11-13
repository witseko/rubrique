//! Draw parsed Markdown blocks with egui widgets (the read-only preview).

use eframe::egui::{self, Align, Color32, Layout, Margin, Rangef, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui::cache::{ComputerMut, FrameCache};

use super::blocks::{Block, ColumnAlign, List, Table, parse_blocks};
use super::inlines::{HtmlStyle, ImageSize, Inline, Script, is_openable_url, parse_inlines};
use super::table_edit::TableCell;
use crate::document::highlighting::{HighlightStyle, Syntax, highlighted};
use crate::media::{MAX_IMAGE_BYTES, MediaKind, image_uri, is_image_path, media_kind_of_url};
use crate::theme::palette::Rgb;
use crate::theme::style::page_fill;

/// Sizes and spacing for one place Markdown is drawn.
struct Look {
    /// Heading font sizes for levels 1 to 6.
    heading_sizes: [f32; 6],
    /// Space above a heading (levels 1 to 6), larger than below so it groups with its text.
    heading_gaps: [f32; 6],
    /// Headings up to this level get a thin line underneath.
    ruled_level: u8,
    /// Space between a heading and its line.
    rule_gap: f32,
    body_size: f32,
    body_line_height: f32,
    /// Vertical gap between blocks.
    block_gap: f32,
    /// Offer Prettify on code, and memo rows: both act on the open file.
    file_tools: bool,
}

/// The preview of the open file, at 100% zoom.
const PAGE: Look = Look {
    heading_sizes: [32.0, 26.0, 21.0, 18.0, 16.0, 16.0],
    heading_gaps: [28.0, 24.0, 18.0, 14.0, 14.0, 14.0],
    ruled_level: 2,
    rule_gap: 8.0,
    body_size: 16.0,
    body_line_height: 26.0,
    block_gap: 12.0,
    file_tools: true,
};

impl Look {
    /// The same look with every size and gap multiplied by `zoom`.
    fn scaled(&self, zoom: f32) -> Look {
        Look {
            heading_sizes: self.heading_sizes.map(|size| size * zoom),
            heading_gaps: self.heading_gaps.map(|gap| gap * zoom),
            rule_gap: self.rule_gap * zoom,
            body_size: self.body_size * zoom,
            body_line_height: self.body_line_height * zoom,
            block_gap: self.block_gap * zoom,
            ..*self
        }
    }
}

/// Zoom of the page's text in the preview (1.0 is 100%), kept as the bits of
/// an `f32`. The app sets it once per frame from the text area zoom setting.
static PAGE_ZOOM: AtomicU32 = AtomicU32::new(0x3f80_0000);

/// Zoom the preview's text by `zoom` (1.0 for none) from the next block on.
pub fn set_page_zoom(zoom: f32) {
    PAGE_ZOOM.store(zoom.to_bits(), Ordering::Relaxed);
}

fn page_zoom() -> f32 {
    f32::from_bits(PAGE_ZOOM.load(Ordering::Relaxed))
}

/// The preview of the open file, at the current zoom.
fn page_look() -> Look {
    PAGE.scaled(page_zoom())
}

/// The preview of an HTML file: the page's sizes, without Prettify and
/// memo rows, which rewrite the file as Markdown.
fn read_only_page_look() -> Look {
    Look { file_tools: false, ..page_look() }
}

/// Smaller and tighter, for AI answers.
const COMPACT: Look = Look {
    heading_sizes: [19.0, 17.0, 15.5, 14.5, 14.0, 14.0],
    heading_gaps: [14.0, 12.0, 10.0, 8.0, 8.0, 8.0],
    ruled_level: 1,
    rule_gap: 4.0,
    body_size: 14.0,
    body_line_height: 20.0,
    block_gap: 8.0,
    file_tools: false,
};

/// Sizes of the page's text, for the boxes that edit a block of it in place.
pub struct PageText {
    /// Font size of body text.
    pub body_size: f32,
    /// Height of a line of body text.
    pub line_height: f32,
    /// Font size of code within the text.
    pub code_size: f32,
}

/// The page's text sizes, so a block edited in place keeps the look and
/// about the height it had rendered.
pub fn page_text() -> PageText {
    let page = page_look();
    PageText {
        body_size: page.body_size,
        line_height: page.body_line_height,
        code_size: page.body_size * INLINE_CODE_SCALE,
    }
}

/// Line height as a multiple of the font size, for headings.
const HEADING_LINE_FACTOR: f32 = 1.3;
/// Vertical gap between list items.
const LIST_ITEM_GAP: f32 = 4.0;
/// Width of the column for bullets, numbers and checkboxes.
const MARKER_WIDTH: f32 = 22.0;
/// Space between a list marker and the item text.
const MARKER_GAP: f32 = 8.0;
const BULLET_RADIUS: f32 = 3.0;
const CHECKBOX_SIZE: f32 = 15.0;
const CHECKBOX_RADIUS: u8 = 4;
const CHECKBOX_STROKE: f32 = 1.5;
/// Check mark points, as fractions of the checkbox size.
const CHECK_MARK: [(f32, f32); 3] = [(0.24, 0.52), (0.43, 0.71), (0.77, 0.31)];
const CHECK_MARK_WIDTH: f32 = 1.7;
const QUOTE_BAR_WIDTH: f32 = 3.0;
/// Space between the quote bar and the quoted text.
const QUOTE_PADDING: i8 = 16;
/// Opacity of the accent color used for the quote bar.
const QUOTE_BAR_TINT: f32 = 0.5;
const CODE_SIZE: f32 = 14.0;
/// Inline code size relative to the text around it (monospace looks larger).
const INLINE_CODE_SCALE: f32 = 0.9;
const CODE_LINE_HEIGHT: f32 = 22.0;
const CODE_RADIUS: u8 = 6;
/// Inner margin (x, y) of a code block.
const CODE_MARGIN: (i8, i8) = (14, 12);
const CODE_LABEL_SIZE: f32 = 11.0;
/// Space between the code block's right edge and its buttons.
const CODE_BUTTON_INSET: f32 = 5.0;
/// Height of the row above the code with the language and the buttons.
const CODE_BUTTON_HEIGHT: f32 = 18.0;
/// Top margin above that row.
const CODE_HEADER_TOP: i8 = 5;
/// Opacity of the warning color behind text that has a note.
const NOTE_TINT: f32 = 0.35;
/// Tallest an image gets in the preview.
const PREVIEW_IMAGE_MAX_HEIGHT: f32 = 720.0;
const IMAGE_RADIUS: u8 = 4;
/// Total space a thematic break takes, its line in the middle.
const RULE_SPACING: f32 = 20.0;
/// Space between table columns, and between table rows.
const TABLE_COLUMN_GAP: f32 = 18.0;
const TABLE_ROW_GAP: f32 = 6.0;
/// Wide table columns are not narrowed below this, so text never wraps one
/// word at a time. A column being edited gets at least this much room too.
const TABLE_COLUMN_MIN_WIDTH: f32 = 120.0;
/// No column is narrower than this: an empty one (a column just added)
/// still shows and can be clicked.
const TABLE_COLUMN_LEAST_WIDTH: f32 = 40.0;
/// Opacity of the accent over the table cell under the mouse.
const CELL_HOVER_TINT: f32 = 0.08;
/// Least contrast an HTML page's text color keeps against the page (the
/// WCAG AA level for body text); a color below it is moved toward the
/// theme's text color until it reads.
const MIN_TEXT_CONTRAST: f32 = 4.5;
/// How many steps from an author color to the theme's text color are tried
/// when the color is too faint on the page.
const CONTRAST_STEPS: u8 = 10;
/// Opacity of an HTML page's background color (`<mark>`, `background-color`)
/// behind text, so the theme's text stays readable on it.
const HTML_BACKGROUND_TINT: f32 = 0.35;
/// Size of superscript and subscript text relative to the text around it.
const SCRIPT_SCALE: f32 = 0.75;
/// How far superscript rises above the text's top (and subscript drops
/// below its bottom), as a fraction of the text's height.
const SCRIPT_LIFT: f32 = 0.1;

/// Render blocks inside `depth` lists (bullets change shape with depth),
/// `gap` apart. List items use a smaller gap so nested lists stay tight.
fn render_blocks_at(ui: &mut Ui, look: &Look, blocks: &[Block], depth: usize, gap: f32) {
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            ui.add_space(gap_before(look, block, gap));
        }
        render_block(ui, look, block, depth);
    }
}

/// Draw one top-level block (for the clickable preview).
pub fn render_top_block(ui: &mut Ui, block: &Block) {
    render_block(ui, &page_look(), block, 0);
}

/// Draw one top-level block of a page that is not edited in place (the
/// HTML preview). Code blocks still offer Copy (see `take_code_action`).
pub fn render_read_only_block(ui: &mut Ui, block: &Block) {
    render_block(ui, &read_only_page_look(), block, 0);
}

/// Space above a top-level block that follows another one.
pub fn top_gap(block: &Block) -> f32 {
    {
        let page = page_look();
        gap_before(&page, block, page.block_gap)
    }
}

/// Draw Markdown from an AI tool, smaller than the page and without the
/// buttons that change the file. Code blocks still offer Copy (see
/// `take_code_action`). Partial text, still arriving, is fine.
pub fn render_answer(ui: &mut Ui, markdown: &str) {
    let blocks = cached_blocks(ui.ctx(), markdown);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        render_blocks_at(ui, &COMPACT, &blocks, 0, COMPACT.block_gap);
    });
}

/// The blocks of `markdown`, parsed again only when the text changes: an
/// answer is drawn every frame, and a long one would otherwise be parsed
/// every frame while the user reads it. Shared through an `Arc` so the
/// cache hands it out without copying the blocks.
fn cached_blocks(ctx: &egui::Context, markdown: &str) -> Arc<Vec<Block>> {
    ctx.memory_mut(|memory| Arc::clone(memory.caches.cache::<BlockCache>().get(markdown)))
}

type BlockCache = FrameCache<Arc<Vec<Block>>, BlockParser>;

/// Parses Markdown into blocks for the cache.
#[derive(Default)]
struct BlockParser;

impl ComputerMut<&str, Arc<Vec<Block>>> for BlockParser {
    fn compute(&mut self, markdown: &str) -> Arc<Vec<Block>> {
        Arc::new(parse_blocks(markdown))
    }
}

/// Draw inline Markdown (bold, italic, code, links) at the default text
/// size, wrapped.
pub fn render_inline_text(ui: &mut Ui, markdown: &str) {
    let size = egui::TextStyle::Body.resolve(ui.style()).size;
    let style = TextStyle { size, line_height: size * HEADING_LINE_FACTOR, ..TextStyle::body(&COMPACT) };
    render_inlines(ui, &parse_inlines(markdown), &style);
}

/// Space above a block: headings get more, so they sit close to their own text.
fn gap_before(look: &Look, block: &Block, gap: f32) -> f32 {
    match block {
        Block::Heading { level, .. } => look.heading_gaps[heading_index(*level)],
        // An aligned heading keeps the room above a heading.
        Block::Aligned { blocks, .. } | Block::Html(blocks) => {
            blocks.first().map_or(gap, |first| gap_before(look, first, gap))
        }
        _ => gap,
    }
}

/// Index 0 to 5 into the per-level heading tables.
fn heading_index(level: u8) -> usize {
    usize::from(level).clamp(1, 6) - 1
}

/// Render one block with the widgets for its kind.
fn render_block(ui: &mut Ui, look: &Look, block: &Block, depth: usize) {
    render_aligned_block(ui, look, block, depth, ColumnAlign::Left);
}

/// Render one block; paragraphs and headings sit as `align` asks.
fn render_aligned_block(ui: &mut Ui, look: &Look, block: &Block, depth: usize, align: ColumnAlign) {
    match block {
        Block::Heading { level, content } => render_heading(ui, look, *level, content, align),
        // A paragraph that is only hidden comments takes no room.
        Block::Paragraph(content) if content.iter().all(|inline| matches!(inline, Inline::Note { .. })) => {}
        Block::Paragraph(content) => match crate::voice::memo_link(content).filter(|_| look.file_tools) {
            Some(link) => crate::voice::view::memo_row(ui, &link),
            None => aligned_row(ui, align, |ui| render_inlines(ui, content, &TextStyle::body(look))),
        },
        Block::Code { info, code } => render_code(ui, look, info, code),
        Block::Quote(blocks) => render_quote(ui, look, blocks, depth),
        Block::List(list) => render_list(ui, look, list, depth),
        Block::Table(table) => render_table(ui, look, table, None),
        Block::ThematicBreak => {
            ui.add(egui::Separator::default().spacing(RULE_SPACING));
        }
        Block::Aligned { align, blocks } => {
            for (index, block) in blocks.iter().enumerate() {
                if index > 0 {
                    ui.add_space(gap_before(look, block, look.block_gap));
                }
                render_aligned_block(ui, look, block, depth, *align);
            }
        }
        Block::Details { summary, open, blocks } => render_details(ui, look, summary, *open, blocks, depth),
        // An HTML block in a Markdown file. Its parts offer no buttons that
        // change the file (Prettify, memo rows, table cells): those rewrite
        // Markdown, and this block's source is HTML.
        Block::Html(blocks) => {
            let read_only = Look { file_tools: false, ..*look };
            render_blocks_at(ui, &read_only, blocks, depth, look.block_gap);
        }
    }
}

/// Draw `content` (a paragraph or a heading's text) centered or at the
/// right, as `align` asks.
///
/// egui lays wrapped text out from the left, so the content is shifted by
/// the room it left free on the previous frame (the first frame is
/// measured and thrown away). Text that wraps fills the column and so
/// stays at the left, like a browser's last line would not; short lines,
/// such as a centered title, logo or row of badges, sit where they should.
fn aligned_row(ui: &mut Ui, align: ColumnAlign, content: impl FnOnce(&mut Ui)) {
    if align == ColumnAlign::Left {
        content(ui);
        return;
    }
    let id = ui.next_auto_id();
    ui.skip_ahead_auto_ids(1);
    let available = ui.available_width();
    let last_width = ui.data(|data| data.get_temp::<f32>(id));
    let free = last_width.map_or(0.0, |width| (available - width).max(0.0));
    let indent = if align == ColumnAlign::Center { free / 2.0 } else { free };
    let width = ui
        .horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(indent);
            ui.vertical(content).response.rect.width()
        })
        .inner;
    if last_width.is_none_or(|last| (last - width).abs() > 0.5) {
        ui.data_mut(|data| data.insert_temp(id, width));
        ui.ctx().request_discard("aligned HTML text measured");
    }
}

/// An HTML `<details>`: a triangle and the summary, and under them the
/// content once opened (a click on the triangle or the summary).
fn render_details(ui: &mut Ui, look: &Look, summary: &[Inline], open: bool, blocks: &[Block], depth: usize) {
    let id = ui.next_auto_id();
    ui.skip_ahead_auto_ids(1);
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
    let (_, header, _) = state
        .show_header(ui, |ui| render_inlines(ui, summary, &TextStyle::body(look)))
        .body(|ui| render_blocks_at(ui, look, blocks, depth, look.block_gap));
    // The summary toggles too, as in a browser, not only the triangle.
    if ui.interact(header.response.rect, id.with("summary"), Sense::click()).clicked() {
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
        state.toggle(ui);
        state.store(ui.ctx());
    }
}

/// Heading text, placed as `align` asks; levels 1 and 2 get a thin line
/// underneath.
fn render_heading(ui: &mut Ui, look: &Look, level: u8, content: &[Inline], align: ColumnAlign) {
    let size = look.heading_sizes[heading_index(level)];
    let style = TextStyle { size, line_height: size * HEADING_LINE_FACTOR, strong: true, ..TextStyle::body(look) };
    aligned_row(ui, align, |ui| render_inlines(ui, content, &style));
    if level <= look.ruled_level {
        ui.add_space(look.rule_gap);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
        ui.painter().hline(rect.x_range(), rect.center().y, stroke);
    }
}

/// A button clicked on a code block in the preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeAction {
    /// Copy this code.
    Copy(String),
    /// Pretty-print this code as the language of its info string.
    Prettify { info: String, code: String },
}

/// The code block button clicked this frame, if any (taken once).
pub fn take_code_action(ctx: &egui::Context) -> Option<CodeAction> {
    ctx.data_mut(|data| {
        let action = data.get_temp::<CodeAction>(code_action_id());
        data.remove::<CodeAction>(code_action_id());
        action
    })
}

fn code_action_id() -> egui::Id {
    egui::Id::unique("rubrique-code-action")
}

/// Code block in an outlined box, colored when its info string names a
/// language the editor colors, with that info string as a small label at
/// the top right. Under the mouse, Copy (and Prettify for JSON, SQL and
/// XML) show there too.
fn render_code(ui: &mut Ui, look: &Look, info: &str, code: &str) {
    let visuals = ui.visuals();
    let frame = egui::Frame::new()
        .fill(visuals.code_bg_color)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(CODE_RADIUS)
        .inner_margin(Margin { top: CODE_HEADER_TOP, ..Margin::symmetric(CODE_MARGIN.0, CODE_MARGIN.1) });
    let mut header = egui::Rect::NOTHING;
    let response = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        // A slim row above the code for the language and the buttons, so
        // they never cover the code.
        header = ui.allocate_exact_size(vec2(ui.available_width(), CODE_BUTTON_HEIGHT), Sense::hover()).0;
        match Syntax::from_code_info(info) {
            Some(syntax) => {
                let style =
                    HighlightStyle::from_ui(ui).with_size(CODE_SIZE * page_zoom(), CODE_LINE_HEIGHT * page_zoom());
                ui.label(highlighted(ui.ctx(), code, &style, Some(syntax)));
            }
            None => {
                ui.label(
                    RichText::new(code)
                        .monospace()
                        .size(CODE_SIZE * page_zoom())
                        .line_height(Some(CODE_LINE_HEIGHT * page_zoom())),
                );
            }
        }
    });
    // The header spans the box's full width, minus a small inset on the right.
    let header = egui::Rect::from_min_max(
        egui::pos2(response.response.rect.left() + f32::from(CODE_MARGIN.0), header.top()),
        egui::pos2(response.response.rect.right() - CODE_BUTTON_INSET, header.bottom()),
    );
    let hovered = ui.rect_contains_pointer(response.response.rect);
    code_header(ui, header, info, code, hovered, look.file_tools);
}

/// The language at the right of the header; under the mouse, Copy (and,
/// with `prettify`, Prettify for JSON, SQL and XML) after it.
fn code_header(ui: &mut Ui, header: egui::Rect, info: &str, code: &str, hovered: bool, prettify: bool) {
    let layout = egui::Layout::right_to_left(egui::Align::Center);
    // A child that takes no room in `ui`, so the blocks after the code
    // still start below it (the header sits inside the code box).
    let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(header).layout(layout));
    ui.spacing_mut().item_spacing.x = 4.0;
    let mut action = None;
    if hovered {
        let fill = ui.visuals().widgets.inactive.weak_bg_fill;
        let button = |text: &str| egui::Button::new(RichText::new(text).size(CODE_LABEL_SIZE + 1.0)).fill(fill).small();
        if ui.add(button("Copy")).on_hover_text("Copy the code").clicked() {
            action = Some(CodeAction::Copy(code.to_string()));
        }
        if let Some(language) = crate::format::Language::from_code_info(info).filter(|_| prettify) {
            let tip = format!("Pretty-print this {} in the file", language.name());
            if ui.add(button("Prettify")).on_hover_text(tip).clicked() {
                action = Some(CodeAction::Prettify { info: info.to_string(), code: code.to_string() });
            }
        }
    }
    if !info.is_empty() {
        ui.label(RichText::new(info).size(CODE_LABEL_SIZE).weak());
    }
    if let Some(action) = action {
        ui.ctx().data_mut(|data| data.insert_temp(code_action_id(), action));
    }
}

/// Quoted blocks in softer text, with a bar in the accent color on the left.
fn render_quote(ui: &mut Ui, look: &Look, blocks: &[Block], depth: usize) {
    let weak_color = ui.visuals().weak_text_color();
    let bar_color = ui.visuals().hyperlink_color.gamma_multiply(QUOTE_BAR_TINT);
    let frame = egui::Frame::new().inner_margin(Margin { left: QUOTE_PADDING, ..Margin::ZERO });
    let response = frame.show(ui, |ui| {
        ui.visuals_mut().override_text_color = Some(weak_color);
        render_blocks_at(ui, look, blocks, depth, look.block_gap);
    });
    let rect = response.response.rect;
    let x = rect.left() + QUOTE_BAR_WIDTH / 2.0;
    ui.painter().vline(x, rect.y_range(), Stroke::new(QUOTE_BAR_WIDTH, bar_color));
}

/// List items with a bullet, number, or checkbox in a column before each.
/// Done tasks are dimmed.
fn render_list(ui: &mut Ui, look: &Look, list: &List, depth: usize) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = LIST_ITEM_GAP;
        for (index, item) in list.items.iter().enumerate() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = MARKER_GAP;
                match (item.task, list.start) {
                    (Some(checked), _) => paint_checkbox(ui, look, checked),
                    (None, Some(start)) => number_marker(ui, look, start + index as u64),
                    (None, None) => paint_bullet(ui, look, depth),
                }
                ui.vertical(|ui| {
                    if item.task == Some(true) {
                        ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
                    }
                    render_blocks_at(ui, look, &item.blocks, depth + 1, LIST_ITEM_GAP);
                });
            });
        }
    });
}

/// What the preview draws in place of a table cell being edited.
pub type CellEditor<'a> = &'a mut dyn FnMut(&mut Ui);

/// Draw a top-level table with `editing` replaced by what `editor` draws
/// (the preview's text box for the cell's Markdown).
pub fn render_top_table(ui: &mut Ui, table: &Table, editing: TableCell, editor: CellEditor<'_>) {
    render_table(ui, &page_look(), table, Some((editing, editor)));
}

/// The table cell clicked this frame in the preview, if any (taken once).
pub fn take_cell_click(ctx: &egui::Context) -> Option<TableCell> {
    ctx.data_mut(|data| {
        let cell = data.get_temp::<TableCell>(cell_click_id());
        data.remove::<TableCell>(cell_click_id());
        cell
    })
}

fn cell_click_id() -> egui::Id {
    egui::Id::unique("rubrique-table-cell-click")
}

/// A table: bold header cells over striped rows, a line under the header.
/// In the preview (`look.file_tools`) a click on a cell is reported through
/// [`take_cell_click`], and the cell in `editor` shows the editor instead.
fn render_table(ui: &mut Ui, look: &Look, table: &Table, editor: Option<(TableCell, CellEditor<'_>)>) {
    let body = TextStyle::body(look);
    let header = TextStyle { strong: true, ..body.clone() };
    let mut widths = column_widths(ui, table, &header, &body);
    if let Some((editing, _)) = &editor
        && let Some(width) = widths.get_mut(editing.column)
    {
        *width = width.max(TABLE_COLUMN_MIN_WIDTH);
    }
    let mut drawing = TableDrawing {
        clickable: look.file_tools,
        table,
        widths,
        header,
        body,
        editor,
        rows: Vec::new(),
        columns: Vec::new(),
    };
    // A table wider than the page scrolls sideways instead of being cut off.
    let scroll = egui::ScrollArea::horizontal().id_salt(ui.next_auto_id());
    ui.skip_ahead_auto_ids(1);
    scroll.show(ui, |ui| {
        let mut header_bottom = 0.0_f32;
        let grid = egui::Grid::new(ui.next_auto_id()).striped(true).spacing(vec2(TABLE_COLUMN_GAP, TABLE_ROW_GAP));
        ui.skip_ahead_auto_ids(1);
        let response = grid.show(ui, |ui| {
            for (column, cell) in table.header.iter().enumerate() {
                drawing.cell(ui, TableCell { row: 0, column }, cell);
                header_bottom = header_bottom.max(ui.min_rect().bottom());
            }
            ui.end_row();
            for (index, row) in table.rows.iter().enumerate() {
                for (column, cell) in row.iter().enumerate() {
                    drawing.cell(ui, TableCell { row: index + 1, column }, cell);
                }
                ui.end_row();
            }
        });
        let rect = response.response.rect;
        let y = header_bottom + TABLE_ROW_GAP / 2.0;
        ui.painter().hline(rect.x_range(), y, ui.visuals().widgets.noninteractive.bg_stroke);
        if drawing.clickable
            && let Some((at, rect)) = drawing.cell_at_pointer(ui)
        {
            hover_cell(ui, at, rect);
        }
    });
}

/// Tint the cell under the mouse, show the text cursor over it (unless a
/// link in it already asked for the hand), and report a click on it.
fn hover_cell(ui: &Ui, at: TableCell, rect: egui::Rect) {
    let fill = ui.visuals().hyperlink_color.gamma_multiply(CELL_HOVER_TINT);
    ui.painter().rect_filled(rect, 4, fill);
    if ui.ctx().output(|output| output.cursor_icon) == egui::CursorIcon::Default {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if ui.input(|input| input.pointer.any_click()) {
        ui.ctx().data_mut(|data| data.insert_temp(cell_click_id(), at));
    }
}

/// What every cell of one table shares while the table is drawn.
struct TableDrawing<'table, 'editor> {
    /// Report clicks on cells: the preview edits them, AI answers do not.
    clickable: bool,
    table: &'table Table,
    /// One width per column.
    widths: Vec<f32>,
    header: TextStyle,
    body: TextStyle,
    /// The cell being edited and what to draw in its place.
    editor: Option<(TableCell, CellEditor<'editor>)>,
    /// Vertical extent of each row drawn so far, and horizontal extent of
    /// each column: a click anywhere in a cell's row and column hits it,
    /// not only its text.
    rows: Vec<Rangef>,
    columns: Vec<Rangef>,
}

impl TableDrawing<'_, '_> {
    /// Draw the cell `at` holding `content`, or the editor when that is the
    /// cell being edited.
    fn cell(&mut self, ui: &mut Ui, at: TableCell, content: &[Inline]) {
        let width = self.widths[at.column];
        let rect = if let Some((editing, editor)) = self.editor.as_mut()
            && *editing == at
        {
            ui.vertical(|ui| {
                ui.set_width(width);
                editor(ui);
            })
            .response
            .rect
        } else {
            let style = if at.row == 0 { &self.header } else { &self.body };
            table_cell(ui, content, style, self.table.align[at.column], width)
        };
        grow(&mut self.rows, at.row, rect.y_range());
        grow(&mut self.columns, at.column, rect.x_range());
    }

    /// The cell under the mouse and its whole area (the full row height and
    /// column width, half the gaps around it included), once the table is
    /// drawn. The cell being edited is not one.
    fn cell_at_pointer(&self, ui: &Ui) -> Option<(TableCell, egui::Rect)> {
        let pointer = ui.input(|input| input.pointer.interact_pos())?;
        let row_span = |span: &Rangef| span.expand(TABLE_ROW_GAP / 2.0);
        let column_span = |span: &Rangef| span.expand(TABLE_COLUMN_GAP / 2.0);
        let row = self.rows.iter().position(|span| row_span(span).contains(pointer.y))?;
        let column = self.columns.iter().position(|span| column_span(span).contains(pointer.x))?;
        let at = TableCell { row, column };
        if self.editor.as_ref().is_some_and(|(editing, _)| *editing == at) {
            return None;
        }
        let rect = egui::Rect::from_x_y_ranges(column_span(&self.columns[column]), row_span(&self.rows[row]));
        ui.rect_contains_pointer(rect).then_some((at, rect))
    }
}

/// Widen `spans[index]` to include `span`, adding entries as needed.
fn grow(spans: &mut Vec<Rangef>, index: usize, span: Rangef) {
    if spans.len() <= index {
        spans.resize(index + 1, Rangef::NOTHING);
    }
    spans[index] = Rangef::new(spans[index].min.min(span.min), spans[index].max.max(span.max));
}

/// The width of each table column for the room the preview has.
///
/// egui's `Grid` sizes a column from what was drawn in it last frame, and
/// starts narrow. Wrapped text then fits the narrow column and the column
/// never grows, so the widths are chosen here from the text instead.
fn column_widths(ui: &Ui, table: &Table, header: &TextStyle, body: &TextStyle) -> Vec<f32> {
    let mut natural: Vec<f32> =
        table.header.iter().map(|cell| cell_natural_width(ui, cell, header).max(TABLE_COLUMN_LEAST_WIDTH)).collect();
    for row in &table.rows {
        for (column, cell) in row.iter().enumerate() {
            natural[column] = natural[column].max(cell_natural_width(ui, cell, body));
        }
    }
    #[expect(clippy::cast_precision_loss, reason = "a table has a handful of columns")]
    let gaps = TABLE_COLUMN_GAP * natural.len().saturating_sub(1) as f32;
    fit_column_widths(&natural, ui.available_width() - gaps)
}

/// Shrink column widths to fit `budget` when their sum is larger.
///
/// Narrow columns keep their width; the widest columns are cut down to one
/// shared cap, so long text wraps and short labels stay on one line. No
/// column is cut below `TABLE_COLUMN_MIN_WIDTH`: a table that still does
/// not fit is drawn wider than the page.
fn fit_column_widths(natural: &[f32], budget: f32) -> Vec<f32> {
    if natural.iter().sum::<f32>() <= budget {
        return natural.to_vec();
    }
    let mut sorted = natural.to_vec();
    sorted.sort_by(f32::total_cmp);
    // Walk from the narrowest column up: a column narrower than an even share
    // of what is left keeps its width; the first wider one sets the cap.
    let mut remaining = budget;
    let mut cap = 0.0_f32;
    for (index, width) in sorted.iter().enumerate() {
        #[expect(clippy::cast_precision_loss, reason = "a table has a handful of columns")]
        let share = remaining / (sorted.len() - index) as f32;
        if *width > share {
            cap = share;
            break;
        }
        remaining -= width;
    }
    let cap = cap.max(TABLE_COLUMN_MIN_WIDTH);
    natural.iter().map(|width| width.min(cap)).collect()
}

/// How wide a cell's longest line is when nothing wraps.
fn cell_natural_width(ui: &Ui, cell: &[Inline], style: &TextStyle) -> f32 {
    let mut spans = Vec::new();
    collect_spans(cell, style, &mut spans);
    let mut widest = 0.0_f32;
    let mut line = 0.0_f32;
    for span in spans {
        match span {
            Span::Text(text, style) => {
                let galley = egui::WidgetText::from(span_rich_text(text, &style)).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::FontSelection::Default,
                );
                line += galley.size().x;
            }
            // Images size themselves to the cell, so ask only for a usable
            // width, or the width an HTML image asks for.
            Span::Image { size, .. } => line += size.width.map_or(TABLE_COLUMN_MIN_WIDTH, f32::from),
            Span::LineBreak => {
                widest = widest.max(line);
                line = 0.0;
            }
        }
    }
    widest.max(line)
}

/// One cell, `width` wide, aligned as its column asks; returns where it was drawn.
fn table_cell(ui: &mut Ui, cell: &[Inline], style: &TextStyle, align: ColumnAlign, width: f32) -> egui::Rect {
    let layout = match align {
        ColumnAlign::Left => Layout::left_to_right(Align::TOP),
        ColumnAlign::Center => Layout::top_down(Align::Center),
        ColumnAlign::Right => Layout::right_to_left(Align::TOP),
    };
    // Fix the width in a plain top-down scope first, so it grows to the
    // right even for right-aligned cells; the grid then remembers it. An
    // empty cell still takes one line, so it can be clicked to edit it.
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.set_min_height(style.line_height);
        ui.with_layout(layout, |ui| render_inlines(ui, cell, style));
    })
    .response
    .rect
}

/// Room for a list marker, one text line tall.
fn marker_rect(ui: &mut Ui, look: &Look) -> egui::Rect {
    ui.allocate_exact_size(vec2(MARKER_WIDTH, look.body_line_height), Sense::hover()).0
}

/// Filled dot, then hollow dot, then square, repeating with nesting depth.
fn paint_bullet(ui: &mut Ui, look: &Look, depth: usize) {
    let center = marker_rect(ui, look).center();
    let color = ui.visuals().weak_text_color();
    let painter = ui.painter();
    match depth % 3 {
        0 => {
            painter.circle_filled(center, BULLET_RADIUS, color);
        }
        1 => {
            painter.circle_stroke(center, BULLET_RADIUS - 0.5, Stroke::new(1.3, color));
        }
        _ => {
            painter.rect_filled(egui::Rect::from_center_size(center, vec2(5.0, 5.0)), 1, color);
        }
    }
}

/// "3." in soft text, right-aligned in the marker column.
fn number_marker(ui: &mut Ui, look: &Look, number: u64) {
    let color = ui.visuals().weak_text_color();
    let size = vec2(MARKER_WIDTH, look.body_line_height);
    ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
        ui.label(RichText::new(format!("{number}.")).size(look.body_size).color(color));
    });
}

/// Read-only task checkbox: an outline, or filled with the accent and a check mark.
fn paint_checkbox(ui: &mut Ui, look: &Look, checked: bool) {
    let rect = marker_rect(ui, look);
    let center = pos2(rect.left() + CHECKBOX_SIZE / 2.0, rect.center().y);
    let square = egui::Rect::from_center_size(center, vec2(CHECKBOX_SIZE, CHECKBOX_SIZE));
    let visuals = ui.visuals();
    let painter = ui.painter();
    if checked {
        painter.rect_filled(square, CHECKBOX_RADIUS, visuals.hyperlink_color);
        let points: Vec<_> = CHECK_MARK.iter().map(|&(x, y)| square.lerp_inside(vec2(x, y))).collect();
        painter.line(points, Stroke::new(CHECK_MARK_WIDTH, check_mark_color(visuals)));
    } else {
        let stroke = Stroke::new(CHECKBOX_STROKE, visuals.weak_text_color());
        painter.rect_stroke(square, CHECKBOX_RADIUS, stroke, StrokeKind::Inside);
    }
}

/// The check mark uses the page color, so it reads on the accent in both themes.
fn check_mark_color(visuals: &egui::Visuals) -> Color32 {
    visuals.text_edit_bg_color()
}

/// Text formatting accumulated while walking nested inlines.
#[derive(Clone)]
#[expect(clippy::struct_excessive_bools, reason = "independent text styles that combine freely, e.g. bold + italic")]
struct TextStyle {
    /// Font size in points.
    size: f32,
    /// Height of one line of text in points.
    line_height: f32,
    strong: bool,
    italic: bool,
    strikethrough: bool,
    code: bool,
    /// Target URL when the text is part of a link.
    link: Option<String>,
    /// What a note says about this text (highlighted, shown on hover).
    note: Option<String>,
    /// Colors, underline and super/subscript from an HTML page.
    html: HtmlStyle,
}

impl TextStyle {
    /// Plain body text.
    fn body(look: &Look) -> Self {
        TextStyle {
            size: look.body_size,
            line_height: look.body_line_height,
            strong: false,
            italic: false,
            strikethrough: false,
            code: false,
            link: None,
            note: None,
            html: HtmlStyle::default(),
        }
    }
}

/// A run of text with one style, an image, or a hard line break.
enum Span {
    Text(String, TextStyle),
    /// `![alt](url)`, inside a link when `link` is set.
    Image {
        alt: String,
        url: String,
        size: ImageSize,
        link: Option<String>,
    },
    LineBreak,
}

/// Render a paragraph or heading as wrapped, styled text.
fn render_inlines(ui: &mut Ui, inlines: &[Inline], style: &TextStyle) {
    let mut spans = Vec::new();
    collect_spans(inlines, style, &mut spans);
    let highlight = HighlightStyle::from_ui(ui);
    ui.horizontal_wrapped(|ui| {
        // Spans touch; the line height alone sets the space between lines.
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let wrap_width = ui.max_rect().width();
        for span in spans {
            match span {
                Span::LineBreak => ui.end_row(),
                Span::Text(text, style) => {
                    for (part, color) in rule_parts(ui.ctx(), &highlight, &text, &style) {
                        // A match gets the rule's color; the rest keeps the style it came with.
                        let part_style = match color {
                            Some(color) => {
                                TextStyle { html: HtmlStyle { color: Some(color), ..style.html }, ..style.clone() }
                            }
                            None => style.clone(),
                        };
                        for piece in split_long_words(&part, style.size, wrap_width) {
                            render_span(ui, piece, &part_style);
                        }
                    }
                }
                Span::Image { alt, url, size, link } => render_image(ui, &alt, &url, size, link.as_deref()),
            }
        }
    });
}

/// Cut a run of plain text at the pieces the text highlight rules match
/// (IP addresses, ticket ids, numbers: Settings → Highlights), the way the
/// editor colors a plain text file. Each part comes with its color; the text
/// between matches has none. Code, links and text an HTML page colored keep
/// their own look, and are returned whole.
fn rule_parts(
    ctx: &egui::Context,
    highlight: &HighlightStyle,
    text: &str,
    style: &TextStyle,
) -> Vec<(String, Option<Rgb>)> {
    if style.code || style.link.is_some() || style.html.color.is_some() {
        return vec![(text.to_string(), None)];
    }
    let mut parts = Vec::new();
    let mut done = 0;
    for (range, color) in highlight.rule_spans(ctx, text) {
        if done < range.start {
            parts.push((text[done..range.start].to_string(), None));
        }
        parts.push((text[range.clone()].to_string(), Some(Rgb(color.r(), color.g(), color.b()))));
        done = range.end;
    }
    if done < text.len() || parts.is_empty() {
        parts.push((text[done..].to_string(), None));
    }
    parts
}

/// Cut `text` so that every word that may be wider than `wrap_width` is a
/// piece of its own (with the whitespace after it); the text around such
/// words stays whole. Joining the pieces gives back `text`.
///
/// This works around an egui 0.37 wrapping bug. A label that starts part way
/// along a line keeps that start as an indent past its first line break, and
/// when a later line has no space to break at (a long path or URL), egui
/// applies the indent again: that line is drawn to the left of the text and
/// a table column grows by the indent, leaving empty room on its right. A
/// label holding a single word breaks correctly.
///
/// No glyph is wider than about one font size, so a word of `n` characters is
/// at most `n * font_size` wide; only words past that bound are cut out, and
/// nothing has to be measured.
fn split_long_words(text: &str, font_size: f32, wrap_width: f32) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        // One word and the whitespace before it.
        let word_start = rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len());
        let word_end = rest[word_start..].find(char::is_whitespace).map_or(rest.len(), |offset| word_start + offset);
        let (leading, word) = rest[..word_end].split_at(word_start);
        #[expect(clippy::cast_precision_loss, reason = "a word has a handful of characters")]
        let may_overflow = word.chars().count() as f32 * font_size > wrap_width;
        current.push_str(leading);
        if may_overflow && !current.is_empty() {
            pieces.push(std::mem::take(&mut current));
        }
        current.push_str(word);
        if may_overflow {
            // The whitespace after the word stays with it, so the next piece
            // starts with a word, and copied text keeps its spaces.
            let after =
                rest[word_end..].find(|c: char| !c.is_whitespace()).map_or(rest.len(), |offset| word_end + offset);
            current.push_str(&rest[word_end..after]);
            pieces.push(std::mem::take(&mut current));
            rest = &rest[after..];
        } else {
            rest = &rest[word_end..];
        }
    }
    if !current.is_empty() || pieces.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// One styled run of text; links become clickable.
fn render_span(ui: &mut Ui, text: String, style: &TextStyle) {
    let mut rich = span_rich_text(text, style);
    if let Some(color) = style.html.color {
        rich = rich.color(readable_color(color32(color), page_fill(ui.visuals()), ui.visuals().text_color()));
    }
    if style.note.is_some() {
        rich = rich.background_color(ui.visuals().warn_fg_color.gamma_multiply(NOTE_TINT));
    } else if let Some(background) = style.html.background {
        rich = rich.background_color(color32(background).gamma_multiply(HTML_BACKGROUND_TINT));
    }
    let Some(script) = style.html.script else {
        span_widget(ui, rich, style);
        return;
    };
    // egui centers every label in its row, so superscript and subscript
    // take a full line's room and draw their smaller text near its top or
    // bottom edge, in a child that does not move the row along.
    let size = egui::WidgetText::from(rich.clone())
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::FontSelection::Default)
        .size();
    let (row, _) = ui.allocate_exact_size(vec2(size.x, style.line_height), Sense::hover());
    let normal_height = size.y / SCRIPT_SCALE;
    let lift = normal_height * SCRIPT_LIFT;
    let top = match script {
        Script::Superscript => row.center().y - normal_height / 2.0 - lift,
        Script::Subscript => row.center().y + normal_height / 2.0 - size.y + lift,
    };
    let rect = egui::Rect::from_min_size(pos2(row.left(), top), size);
    span_widget(&mut ui.new_child(egui::UiBuilder::new().max_rect(rect)), rich, style);
}

/// The widget for one run of text: a link, a local link shown but not
/// opened, or a label; with the note on hover when it has one.
fn span_widget(ui: &mut Ui, rich: RichText, style: &TextStyle) {
    let note_tip = |response: egui::Response| match &style.note {
        Some(note) => response.on_hover_text(format!("Note: {note}")),
        None => response,
    };
    match &style.link {
        Some(url) if is_openable_url(url) => {
            note_tip(ui.hyperlink_to(rich, url).on_hover_text(url));
        }
        // Local or unknown targets are shown, not opened.
        Some(url) => {
            note_tip(
                ui.label(rich.underline()).on_hover_text(format!("{url} (not opened: only web and mail links open)")),
            );
        }
        None => {
            note_tip(ui.label(rich));
        }
    }
}

/// The font, size and emphasis of one span, as drawn and as measured for tables.
fn span_rich_text(text: String, style: &TextStyle) -> RichText {
    // egui draws strike and underline lines from the line box, so a tall
    // line height pushes them too low: those spans keep the font's own height.
    // Code spans too: egui paints their background from the top of the text
    // down by the full line height, which leaves empty space under the code.
    let own_height = style.code
        || style.strikethrough
        || style.html.underline
        || style.link.as_ref().is_some_and(|url| !is_openable_url(url));
    // Superscript and subscript are placed by `render_span`, at their
    // font's own height.
    let line_height = (!own_height && style.html.script.is_none()).then_some(style.line_height);
    let mut size = if style.code { style.size * INLINE_CODE_SCALE } else { style.size };
    if style.html.script.is_some() {
        size *= SCRIPT_SCALE;
    }
    let mut rich = RichText::new(text).size(size).line_height(line_height);
    if style.html.underline {
        rich = rich.underline();
    }
    if style.strong {
        rich = rich.strong();
    }
    if style.italic {
        rich = rich.italics();
    }
    if style.strikethrough {
        rich = rich.strikethrough();
    }
    if style.code {
        rich = rich.code();
    }
    rich
}

/// Set the folder that relative image paths in the preview resolve against
/// (the open file's folder). Call before drawing the blocks.
pub fn set_image_folder(ctx: &egui::Context, folder: Option<PathBuf>) {
    ctx.data_mut(|data| match folder {
        Some(folder) => {
            data.insert_temp(image_folder_id(), folder);
        }
        None => data.remove::<PathBuf>(image_folder_id()),
    });
}

fn image_folder_id() -> egui::Id {
    egui::Id::unique("rubrique-image-folder")
}

/// The local file an image url points at: relative paths resolve against
/// the open file's folder, `file://` and absolute paths are used as they
/// are. Web and other addresses give `None`: the preview never loads
/// anything from the network.
fn local_media_path(ctx: &egui::Context, url: &str) -> Option<PathBuf> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let path = path.strip_prefix("file://").unwrap_or(path);
    if path.is_empty() || (path.contains(':') && !path.starts_with('/')) {
        return None;
    }
    let path = Path::new(path);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    let folder = ctx.data(|data| data.get_temp::<PathBuf>(image_folder_id()))?;
    Some(folder.join(path))
}

/// Seconds a verdict of [`image_refusal`] is trusted before the file is
/// looked at again, so the preview does not ask the disk every frame yet
/// notices an image that changes.
const IMAGE_CHECK_SECONDS: f64 = 1.0;

/// [`image_refusal`] for a file that is not there.
const IMAGE_MISSING: &str = "The file is missing";

/// Why the preview will not load the file at `path` as an image, or `None`
/// when it may: it must be a regular file (not a folder, a device such as
/// `/dev/zero` or a pipe that never ends), have an image extension, and be
/// at most [`MAX_IMAGE_BYTES`] long, since egui reads all of it at once.
fn image_refusal(path: &Path) -> Option<&'static str> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Some(IMAGE_MISSING);
    };
    if !metadata.is_file() {
        Some("Not a file")
    } else if !is_image_path(path) {
        Some("Not an image file the preview shows")
    } else if metadata.len() > MAX_IMAGE_BYTES {
        Some("The image is too large to show in the preview")
    } else {
        None
    }
}

/// [`image_refusal`] for `path`, remembered for [`IMAGE_CHECK_SECONDS`].
/// A missing file is not remembered: a remote image shows the frame after
/// its download lands, which may be the last frame for a while.
fn cached_image_refusal(ctx: &egui::Context, path: &Path) -> Option<&'static str> {
    let id = egui::Id::unique(("rubrique-image-check", path));
    let now = ctx.input(|input| input.time);
    let known = ctx.data(|data| data.get_temp::<(Option<&'static str>, f64)>(id));
    if let Some((refusal, checked)) = known
        && now - checked < IMAGE_CHECK_SECONDS
    {
        return refusal;
    }
    let refusal = image_refusal(path);
    if refusal != Some(IMAGE_MISSING) {
        ctx.data_mut(|data| data.insert_temp(id, (refusal, now)));
    }
    refusal
}

/// The video a preview card asked to play this frame, if any (taken once).
pub fn take_play_request(ctx: &egui::Context) -> Option<PathBuf> {
    ctx.data_mut(|data| {
        let path = data.get_temp::<PathBuf>(play_request_id());
        data.remove::<PathBuf>(play_request_id());
        path
    })
}

fn play_request_id() -> egui::Id {
    egui::Id::unique("rubrique-play-video")
}

/// An image from a local file, as wide as the text column at most; a video
/// as a card with Play. Web images and missing files show their alt text.
fn render_image(ui: &mut Ui, alt: &str, url: &str, size: ImageSize, link: Option<&str>) {
    let Some(path) = local_media_path(ui.ctx(), url) else {
        image_fallback(
            ui,
            alt,
            &format!("{url}\n(web images are not loaded in the preview; Open in browser shows them)"),
        );
        return;
    };
    if media_kind_of_url(url) == Some(MediaKind::Video) {
        video_card(ui, alt, &path);
        return;
    }
    if let Some(refusal) = cached_image_refusal(ui.ctx(), &path) {
        image_fallback(ui, alt, &format!("{}\n{refusal}", path.display()));
        return;
    }
    let max_width = ui.max_rect().width();
    let image = egui::Image::new(image_uri(&path)).corner_radius(IMAGE_RADIUS);
    let image = if size == ImageSize::default() {
        image.fit_to_original_size(1.0).max_width(max_width).max_height(PREVIEW_IMAGE_MAX_HEIGHT)
    } else {
        // The size an HTML image asks for, within the column; with only one
        // side given the picture keeps its proportions.
        let width = size.width.map_or(max_width, |width| f32::from(width).min(max_width));
        let height =
            size.height.map_or(PREVIEW_IMAGE_MAX_HEIGHT, |height| f32::from(height).min(PREVIEW_IMAGE_MAX_HEIGHT));
        image
            .fit_to_exact_size(vec2(width, height))
            .maintain_aspect_ratio(size.width.is_none() || size.height.is_none())
    };
    if let Err(error) = image.load_for_size(ui.ctx(), vec2(max_width, PREVIEW_IMAGE_MAX_HEIGHT)) {
        image_fallback(ui, alt, &format!("{}\n{error}", path.display()));
        return;
    }
    let tip = if alt.is_empty() { path.display().to_string() } else { alt.to_string() };
    match link {
        Some(url) if is_openable_url(url) => {
            let response = ui.add(image.sense(Sense::click())).on_hover_text(url);
            if response.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
        _ => {
            ui.add(image).on_hover_text(tip);
        }
    }
}

/// "[image: alt]" in soft text, for an image that cannot be shown.
fn image_fallback(ui: &mut Ui, alt: &str, why: &str) {
    let name = if alt.is_empty() { "image" } else { alt };
    let page = page_look();
    let text = RichText::new(format!("[{name}]")).size(page.body_size).line_height(Some(page.body_line_height)).weak();
    ui.label(text).on_hover_text(why);
}

/// A video in the preview: its name and a Play button that opens it in the
/// system's video player (Rubrique cannot play video itself).
fn video_card(ui: &mut Ui, alt: &str, path: &Path) {
    let visuals = ui.visuals();
    let frame = egui::Frame::new()
        .fill(visuals.code_bg_color)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(CODE_RADIUS)
        .inner_margin(Margin::symmetric(12, 8));
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.horizontal(|ui| {
            crate::icons::icon_label(ui, crate::icons::Icon::Play);
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let title = if alt.is_empty() { name } else { format!("{alt} ({name})") };
            ui.label(RichText::new(title).size(page_look().body_size));
            if ui.button("Play").on_hover_text("Open in your video player").clicked() {
                ui.ctx().data_mut(|data| data.insert_temp(play_request_id(), path.to_path_buf()));
            }
        });
    });
}

/// Highlight the end of the spans so far that matches `quote` (the text a
/// note is about) and attach the note to it, splitting a span if needed.
fn mark_note(spans: &mut Vec<Span>, quote: &str, note: &str) {
    let shown: String = spans
        .iter()
        .filter_map(|span| if let Span::Text(text, _) = span { Some(text.as_str()) } else { None })
        .collect();
    let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    if quote.is_empty() || !normalize(&shown).ends_with(&normalize(quote)) {
        return;
    }
    let mut remaining = quote.trim_end().chars().count();
    let mut index = spans.len();
    while remaining > 0 && index > 0 {
        index -= 1;
        let Span::Text(text, style) = &mut spans[index] else {
            continue;
        };
        let length = text.chars().count();
        if length <= remaining {
            style.note = Some(note.to_string());
            remaining -= length;
        } else {
            let split_at = text.char_indices().nth(length - remaining).map_or(text.len(), |(offset, _)| offset);
            let tail = text.split_off(split_at);
            let marked = TextStyle { note: Some(note.to_string()), ..style.clone() };
            spans.insert(index + 1, Span::Text(tail, marked));
            remaining = 0;
        }
    }
}

/// Flatten nested inlines into styled spans.
fn collect_spans(inlines: &[Inline], style: &TextStyle, spans: &mut Vec<Span>) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => spans.push(Span::Text(text.clone(), style.clone())),
            Inline::Code(code) => spans.push(Span::Text(code.clone(), TextStyle { code: true, ..style.clone() })),
            Inline::Emphasis(children) => collect_spans(children, &TextStyle { italic: true, ..style.clone() }, spans),
            Inline::Strong(children) => collect_spans(children, &TextStyle { strong: true, ..style.clone() }, spans),
            Inline::Strikethrough(children) => {
                collect_spans(children, &TextStyle { strikethrough: true, ..style.clone() }, spans);
            }
            Inline::Link { content, url } => {
                // Links keep the link color, as a browser draws them, even
                // inside text an HTML page colors.
                let html = HtmlStyle { color: None, ..style.html };
                collect_spans(content, &TextStyle { link: Some(url.clone()), html, ..style.clone() }, spans);
            }
            Inline::Image { alt, url, size } => {
                spans.push(Span::Image { alt: alt.clone(), url: url.clone(), size: *size, link: style.link.clone() });
            }
            Inline::LineBreak => spans.push(Span::LineBreak),
            Inline::Note { quote, text } => mark_note(spans, quote, text),
            Inline::Styled { style: html, content } => {
                collect_spans(content, &TextStyle { html: html.inside(style.html), ..style.clone() }, spans);
            }
        }
    }
}

/// A theme color as egui's.
fn color32(Rgb(red, green, blue): Rgb) -> Color32 {
    Color32::from_rgb(red, green, blue)
}

/// An HTML page's text `color`, kept readable on the theme's `page`: a
/// color with too little contrast (dark blue on a dark theme, yellow on a
/// light one) is moved step by step toward the theme's `text` color, which
/// keeps its hue as far as it can.
fn readable_color(color: Color32, page: Color32, text: Color32) -> Color32 {
    for step in 0..=CONTRAST_STEPS {
        let mixed = color.lerp_to_gamma(text, f32::from(step) / f32::from(CONTRAST_STEPS));
        if contrast_ratio(mixed, page) >= MIN_TEXT_CONTRAST {
            return mixed;
        }
    }
    text
}

/// The WCAG contrast ratio of two colors, from 1 (the same) to 21 (black
/// on white).
fn contrast_ratio(first: Color32, second: Color32) -> f32 {
    let luminance = |color: Color32| {
        let linear = egui::Rgba::from(color);
        0.2126 * linear.r() + 0.7152 * linear.g() + 0.0722 * linear.b()
    };
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use eframe::egui;

    use super::{
        Color32, IMAGE_MISSING, MAX_IMAGE_BYTES, MIN_TEXT_CONTRAST, TABLE_COLUMN_MIN_WIDTH, cached_blocks,
        contrast_ratio, fit_column_widths, image_refusal, parse_blocks, readable_color, split_long_words,
    };
    use crate::test_support::TempFolder;

    #[test]
    fn only_regular_image_files_up_to_the_limit_are_loaded() {
        let temp = TempFolder::new();
        assert_eq!(image_refusal(&temp.path().join("gone.png")), Some(IMAGE_MISSING));
        assert_eq!(image_refusal(&temp.make_folder("folder.png")), Some("Not a file"));
        #[cfg(unix)]
        assert_eq!(image_refusal(std::path::Path::new("/dev/zero")), Some("Not a file"));
        assert_eq!(image_refusal(&temp.write("notes.txt", "text")), Some("Not an image file the preview shows"));
        let large = temp.write("large.png", "");
        // A sparse file: as long as the limit plus one byte, without writing it.
        std::fs::File::options().write(true).open(&large).unwrap().set_len(MAX_IMAGE_BYTES + 1).unwrap();
        assert_eq!(image_refusal(&large), Some("The image is too large to show in the preview"));
        assert_eq!(image_refusal(&temp.write("photo.png", "png bytes")), None);
    }

    const LIGHT_PAGE: Color32 = Color32::from_rgb(0xfd, 0xfc, 0xfa);
    const LIGHT_TEXT: Color32 = Color32::from_rgb(0x2b, 0x2a, 0x28);
    const DARK_PAGE: Color32 = Color32::from_rgb(0x1d, 0x1f, 0x23);
    const DARK_TEXT: Color32 = Color32::from_rgb(0xd4, 0xd4, 0xd4);

    #[test]
    fn readable_author_colors_stay_as_written() {
        let dark_red = Color32::from_rgb(0xb0, 0x00, 0x00);
        assert_eq!(readable_color(dark_red, LIGHT_PAGE, LIGHT_TEXT), dark_red);
    }

    #[test]
    fn faint_author_colors_move_toward_the_theme_text() {
        for (color, page, text) in [
            (Color32::from_rgb(0xff, 0xff, 0x00), LIGHT_PAGE, LIGHT_TEXT),
            (Color32::from_rgb(0x00, 0x00, 0x80), DARK_PAGE, DARK_TEXT),
            (Color32::WHITE, LIGHT_PAGE, LIGHT_TEXT),
        ] {
            let shown = readable_color(color, page, text);
            assert_ne!(shown, color);
            assert!(contrast_ratio(shown, page) >= MIN_TEXT_CONTRAST, "{color:?} became {shown:?}");
        }
    }

    #[test]
    fn contrast_runs_from_one_to_twenty_one() {
        assert!((contrast_ratio(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.01);
        assert!((contrast_ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.1);
    }

    #[test]
    fn answer_blocks_are_parsed_once_per_text() {
        let ctx = egui::Context::default();
        // Still streaming: the code fence is not closed yet.
        let partial = "# Plan\n\n- **one**\n\n```rust\nfn main() {";
        let first = cached_blocks(&ctx, partial);
        assert_eq!(*first, parse_blocks(partial));
        assert!(Arc::ptr_eq(&first, &cached_blocks(&ctx, partial)));
        let longer = format!("{partial}\n}}\n```\n");
        let second = cached_blocks(&ctx, &longer);
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(*second, parse_blocks(&longer));
    }

    #[test]
    fn columns_that_fit_keep_their_width() {
        assert_eq!(fit_column_widths(&[80.0, 300.0], 500.0), vec![80.0, 300.0]);
    }

    #[test]
    fn a_long_column_wraps_while_a_short_one_keeps_its_width() {
        // The file-name and description table from the screenshot: the short
        // column stays whole, the long one takes the rest of the page.
        assert_eq!(fit_column_widths(&[180.0, 2000.0], 900.0), vec![180.0, 720.0]);
    }

    #[test]
    fn several_wide_columns_share_the_room_evenly() {
        assert_eq!(fit_column_widths(&[100.0, 900.0, 600.0], 700.0), vec![100.0, 300.0, 300.0]);
    }

    #[test]
    fn wide_columns_are_not_cut_below_the_minimum() {
        let widths = fit_column_widths(&[50.0, 1000.0, 1000.0], 200.0);
        assert_eq!(widths, vec![50.0, TABLE_COLUMN_MIN_WIDTH, TABLE_COLUMN_MIN_WIDTH]);
    }

    #[test]
    fn text_of_short_words_stays_one_piece() {
        assert_eq!(split_long_words("a superuser still creates it", 10.0, 200.0), vec!["a superuser still creates it"]);
    }

    #[test]
    fn a_word_that_may_overflow_gets_a_piece_of_its_own() {
        // 39 characters at 10 points may be 390 wide, past the 200 of the line.
        let pieces = split_long_words("POST /api_internal/v2/virtual-penalty/rounds still", 10.0, 200.0);
        assert_eq!(pieces, vec!["POST ", "/api_internal/v2/virtual-penalty/rounds ", "still"]);
    }

    #[test]
    fn the_pieces_join_back_into_the_text() {
        let text = "  lead /a/very/long/path/name/here   and   /another/long/path/name/x  ";
        assert_eq!(split_long_words(text, 10.0, 100.0).concat(), text);
    }

    #[test]
    fn empty_text_is_one_empty_piece() {
        assert_eq!(split_long_words("", 10.0, 100.0), vec![String::new()]);
    }
}
