//! Draw a terminal tab: the tmux screen in a monospace grid with its
//! colors and cursor. A click gives it focus; while it has focus, what you
//! type goes to tmux (see [`super::keys`]).

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Color32, EventFilter, FontId, Sense, Stroke, Ui, vec2};

use super::ansi::{CellStyle, Color, Span};
use super::keys::{KeyInput, key_input};
use super::screen::Screen;
use super::session::{Latest, LiveTerminal};
use crate::theme::style::page_fill;

/// Space between the tab's edge and the text.
const PADDING: f32 = 10.0;
/// The smallest size a session is made, so a tiny window cannot break it.
const MIN_COLUMNS: u16 = 20;
const MIN_ROWS: u16 = 5;
/// How much dim text is faded.
const DIM: f32 = 0.6;
/// The 16 classic terminal colors: black, red, green, yellow, blue,
/// magenta, cyan, white, then their bright versions.
const ANSI_COLORS: [Color32; 16] = [
    Color32::from_rgb(0x1e, 0x1e, 0x1e),
    Color32::from_rgb(0xcd, 0x31, 0x31),
    Color32::from_rgb(0x0d, 0xbc, 0x79),
    Color32::from_rgb(0xe5, 0xa5, 0x10),
    Color32::from_rgb(0x24, 0x72, 0xc8),
    Color32::from_rgb(0xbc, 0x3f, 0xbc),
    Color32::from_rgb(0x11, 0xa8, 0xcd),
    Color32::from_rgb(0xe5, 0xe5, 0xe5),
    Color32::from_rgb(0x66, 0x66, 0x66),
    Color32::from_rgb(0xf1, 0x4c, 0x4c),
    Color32::from_rgb(0x23, 0xd1, 0x8b),
    Color32::from_rgb(0xf5, 0xf5, 0x43),
    Color32::from_rgb(0x3b, 0x8e, 0xea),
    Color32::from_rgb(0xd6, 0x70, 0xd6),
    Color32::from_rgb(0x29, 0xb8, 0xdb),
    Color32::from_rgb(0xff, 0xff, 0xff),
];

fn terminal_id() -> egui::Id {
    egui::Id::unique("rubrique-terminal")
}

/// Draw `terminal` in the rest of the page.
pub fn show_terminal(ui: &mut Ui, terminal: &mut LiveTerminal) {
    terminal.ensure_running(ui.ctx());
    let background = page_fill(ui.visuals());
    let rect = ui.available_rect_before_wrap();
    let response = ui.interact(rect, terminal_id(), Sense::click());
    ui.allocate_rect(rect, Sense::hover());
    ui.painter().rect_filled(rect, 0.0, background);
    let screen = match terminal.latest() {
        Latest::Waiting => {
            ui.put(rect, egui::Label::new(egui::RichText::new("Connecting to tmux…").weak()));
            return;
        }
        Latest::Gone(reason) => {
            if ended(ui, rect, &reason) {
                terminal.restart();
            }
            return;
        }
        Latest::Screen(screen) => screen,
    };
    // A new tab, or a click, takes the keyboard.
    if response.clicked() || ui.memory(|memory| memory.focused().is_none()) {
        response.request_focus();
    }
    let focused = response.has_focus();
    if focused {
        // Keep Tab, arrows and Esc for the terminal instead of moving focus.
        let filter = EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true };
        ui.memory_mut(|memory| memory.set_focus_lock_filter(terminal_id(), filter));
        terminal.send(take_typed_keys(ui));
    }
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let (cell_width, row_height) = ui.fonts_mut(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
    let inner = rect.shrink(PADDING);
    terminal.resize(
        cells_that_fit(inner.width(), cell_width, MIN_COLUMNS),
        cells_that_fit(inner.height(), row_height, MIN_ROWS),
    );
    let colors = DefaultColors { text: ui.visuals().text_color(), background };
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(screen_job(&screen, &font, colors)));
    let cursor_cell = screen.cursor.map(|(column, row)| {
        cursor_rect(&galley, usize::from(column), usize::from(row), vec2(cell_width, row_height))
            .translate(inner.min.to_vec2())
    });
    ui.painter().with_clip_rect(rect).galley(inner.min, galley, colors.text);
    if let Some(cursor) = cursor_cell {
        let color = ui.visuals().text_cursor.stroke.color;
        if focused {
            ui.painter().rect_filled(cursor, 0.0, color.gamma_multiply(0.7));
        } else {
            ui.painter().rect_stroke(cursor, 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
        }
    }
}

/// Where the cursor cell at `column`, `row` sits, relative to the galley.
///
/// Placed from the laid-out rows rather than `row * cell height`: a row
/// with a glyph from a fallback font (such as `❯` or box drawing) is
/// taller than the monospace font's row, so multiplying would drift the
/// cursor further down with every such row above it. tmux trims trailing
/// spaces, so a cursor past the end of a row's text (or below the last
/// row) is placed by whole cells after the last character (or row).
fn cursor_rect(galley: &egui::Galley, column: usize, row: usize, cell: egui::Vec2) -> egui::Rect {
    let Some(last) = galley.rows.last() else {
        return egui::Rect::from_min_size(egui::pos2(cells(column, cell.x), cells(row, cell.y)), cell);
    };
    let (placed, rows_below) = match galley.rows.get(row) {
        Some(placed) => (placed, 0),
        None => (last, row + 1 - galley.rows.len()),
    };
    let glyphs = &placed.glyphs;
    let x = match glyphs.get(column) {
        Some(glyph) => glyph.pos.x,
        None => placed.size.x + cells(column - glyphs.len(), cell.x),
    };
    let (top, height) = if rows_below == 0 {
        (placed.pos.y, placed.size.y)
    } else {
        (placed.pos.y + placed.size.y + cells(rows_below - 1, cell.y), cell.y)
    };
    let width = glyphs.get(column).map_or(cell.x, |glyph| glyph.advance_width);
    egui::Rect::from_min_size(egui::pos2(placed.pos.x + x, top), vec2(width, height))
}

/// The length of `count` cells of `size`; terminal sizes fit in a `u16`.
fn cells(count: usize, size: f32) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX)) * size
}

/// The "session ended" message with a button to start it again. True
/// when the button was clicked.
fn ended(ui: &mut Ui, rect: egui::Rect, reason: &str) -> bool {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(PADDING * 3.0)), |ui| {
        ui.heading("This terminal has ended");
        ui.label(egui::RichText::new(reason).weak());
        ui.button("Start again").clicked()
    })
    .inner
}

/// Take the typing events out of this frame's input, so no other widget
/// reacts to them, and return them as keys for tmux.
fn take_typed_keys(ui: &Ui) -> Vec<KeyInput> {
    let mut keys = Vec::new();
    ui.ctx().input_mut(|input| {
        input.events.retain(|event| match key_input(event) {
            Some(key) => {
                keys.push(key);
                false
            }
            None => true,
        });
    });
    keys
}

/// How many cells of `cell` size fit in `room`, at least `minimum`.
fn cells_that_fit(room: f32, cell: f32, minimum: u16) -> u16 {
    if cell <= 0.0 {
        return minimum;
    }
    // Rounded down, and the float-to-int `as` saturates, so huge rooms are safe.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded and clamped to u16 just below"
    )]
    let count = (room / cell).floor().clamp(0.0, f32::from(u16::MAX)) as u16;
    count.max(minimum)
}

/// The theme's text and page colors, used for "default" terminal colors.
#[derive(Clone, Copy, Debug)]
struct DefaultColors {
    text: Color32,
    background: Color32,
}

/// The whole screen as one layout: each row on its own line.
fn screen_job(screen: &Screen, font: &FontId, colors: DefaultColors) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    for (index, line) in screen.lines.iter().enumerate() {
        for Span { text, style } in line {
            job.append(text, 0.0, text_format(*style, font, colors));
        }
        if index + 1 < screen.lines.len() {
            job.append("\n", 0.0, text_format(CellStyle::default(), font, colors));
        }
    }
    job
}

/// How one styled run is drawn.
fn text_format(style: CellStyle, font: &FontId, colors: DefaultColors) -> TextFormat {
    let mut text = resolve(style.foreground, colors.text);
    let mut background = match style.background {
        Color::Default => Color32::TRANSPARENT,
        color => resolve(color, colors.background),
    };
    if style.reverse {
        let behind = if background == Color32::TRANSPARENT { colors.background } else { background };
        (text, background) = (behind, text);
    }
    if style.dim {
        text = text.gamma_multiply(DIM);
    }
    let line = Stroke::new(1.0, text);
    TextFormat {
        font_id: font.clone(),
        color: text,
        background,
        italics: style.italic,
        underline: if style.underline { line } else { Stroke::NONE },
        strikethrough: if style.strikethrough { line } else { Stroke::NONE },
        ..TextFormat::default()
    }
}

/// The screen color for a terminal color; `default` for [`Color::Default`].
fn resolve(color: Color, default: Color32) -> Color32 {
    match color {
        Color::Default => default,
        Color::Rgb(red, green, blue) => Color32::from_rgb(red, green, blue),
        Color::Indexed(index) => indexed_color(index),
    }
}

/// One of the 256 numbered colors: the 16 classic ones, a 6×6×6 color
/// cube, then 24 grays.
fn indexed_color(index: u8) -> Color32 {
    match index {
        0..=15 => ANSI_COLORS[usize::from(index)],
        16..=231 => {
            let cube = index - 16;
            let level = |step: u8| if step == 0 { 0 } else { 55 + step * 40 };
            Color32::from_rgb(level(cube / 36), level((cube / 6) % 6), level(cube % 6))
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            Color32::from_rgb(gray, gray, gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_whole_cells_with_a_minimum() {
        assert_eq!(cells_that_fit(805.0, 8.0, 20), 100);
        assert_eq!(cells_that_fit(50.0, 8.0, 20), 20);
        assert_eq!(cells_that_fit(100.0, 0.0, 5), 5);
    }

    #[test]
    fn numbered_colors() {
        assert_eq!(indexed_color(1), ANSI_COLORS[1]);
        assert_eq!(indexed_color(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(indexed_color(231), Color32::from_rgb(255, 255, 255));
        assert_eq!(indexed_color(232), Color32::from_rgb(8, 8, 8));
        assert_eq!(indexed_color(255), Color32::from_rgb(238, 238, 238));
    }

    #[test]
    fn cursor_follows_the_laid_out_rows() {
        // A first row in a bigger font stands in for a row made taller by
        // a fallback glyph.
        let mut job = LayoutJob::default();
        let small = TextFormat { font_id: FontId::monospace(12.0), ..TextFormat::default() };
        let big = TextFormat { font_id: FontId::monospace(24.0), ..TextFormat::default() };
        job.append("big\n", 0.0, big);
        job.append("ab", 0.0, small);
        let ctx = egui::Context::default();
        let mut galley = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            galley = Some(ui.fonts_mut(|fonts| fonts.layout_job(job.clone())));
        });
        output.textures_delta.clear();
        let galley = galley.expect("laid out in the frame");
        let cell = vec2(7.0, 14.0);
        let first_height = galley.rows[0].size.y;
        assert!(first_height > cell.y);

        let on_text = cursor_rect(&galley, 1, 1, cell);
        assert_eq!(on_text.min, egui::pos2(galley.rows[1].glyphs[1].pos.x, first_height));
        let past_text = cursor_rect(&galley, 4, 1, cell);
        assert_eq!(past_text.min.x, galley.rows[1].size.x + 2.0 * cell.x);
        let below = cursor_rect(&galley, 0, 3, cell);
        assert_eq!(below.min.y, galley.rows[1].pos.y + galley.rows[1].size.y + cell.y);
    }

    #[test]
    fn reverse_swaps_text_and_page_colors() {
        let colors = DefaultColors { text: Color32::WHITE, background: Color32::BLACK };
        let style = CellStyle { reverse: true, ..CellStyle::default() };
        let format = text_format(style, &FontId::monospace(12.0), colors);
        assert_eq!((format.color, format.background), (Color32::BLACK, Color32::WHITE));
    }
}
