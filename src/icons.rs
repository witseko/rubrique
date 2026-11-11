//! Small line icons drawn with the painter (the built-in fonts have no icon
//! glyphs), and a flat button that shows one.

use std::f32::consts::TAU;

use eframe::egui::{self, Color32, Painter, Pos2, Response, Sense, Stroke, Ui, Vec2, WidgetInfo, pos2, vec2};

/// Size of an icon button.
const BUTTON_SIZE: Vec2 = vec2(28.0, 24.0);
/// Height of an icon button: the height of a row of them.
pub const BUTTON_HEIGHT: f32 = BUTTON_SIZE.y;
/// Line width of every icon: bold like the app icon's "R", but thin
/// enough to stay sharp at 14 px.
const LINE: f32 = 1.5;

/// The icons Rubrique draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Settings.
    Gear,
    /// Add a folder to the explorer.
    FolderPlus,
    /// New scratch file.
    NewScratch,
    /// Re-read from disk.
    Refresh,
    /// Ask AI.
    Sparkle,
    /// Stop a running AI tool.
    Stop,
    /// Play a video or a voice memo.
    Play,
    /// Close a tab. A plain cross with no accent: it sits on every tab, and
    /// a colored mark there would pull the eye from the file names.
    Close,
    /// Record a voice memo.
    Mic,
    /// Pause a voice memo.
    Pause,
    /// The rendered preview of a note.
    Eye,
    /// The Markdown source of a note.
    Code,
    /// A file has notes (the explorer's marker).
    Bubble,
    /// Add a folder from another computer (over SSH).
    ServerPlus,
    /// A terminal (a tmux session). The `>_` prompt is the accent, so it can
    /// carry the status color of the agent inside.
    Terminal,
    /// Add one more of what the section lists (a new terminal).
    Plus,
    /// Show or hide the right panel: a window whose right column is the accent.
    SidePanel,
    /// An encrypted file: a padlock whose keyhole is the accent.
    Lock,
    /// Add an audio file as a memo: a waveform with the plus badge.
    AudioPlus,
    /// Pretty-print the file: three text lines, the indented one the accent.
    Format,
    /// Turn the text highlights on or off: a marker whose stroke under it
    /// is the accent.
    Highlighter,
    /// Open the file in the web browser: a window with an arrow leaving it.
    Browser,
    /// Add a table row: a table split into two rows, with the plus badge.
    RowPlus,
    /// Remove a table row: the same table with the minus badge.
    RowMinus,
    /// Add a table column: a table split into two columns, with the plus badge.
    ColumnPlus,
    /// Remove a table column: the same table with the minus badge.
    ColumnMinus,
    /// Show only some files (the explorer's Markdown filter): a funnel
    /// whose spout is the accent.
    Filter,
}

/// A flat button with `icon` and a hover tip. `active` keeps it highlighted
/// (for a panel that is open). `label` is what screen readers announce.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, tip: &str, active: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(BUTTON_SIZE, Sense::click());
    response.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, active, label));
    let visuals = ui.visuals();
    let fill = if active {
        visuals.selection.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let color = if active { visuals.strong_text_color() } else { ui.style().interact(&response).fg_stroke.color };
    let painter = ui.painter();
    painter.rect_filled(rect, visuals.widgets.inactive.corner_radius, fill);
    // A badge cuts a gap into the shape in the color behind the icon.
    let behind = if fill == Color32::TRANSPARENT { visuals.panel_fill } else { fill };
    paint(painter, icon, rect.center(), Colors { line: color, accent: visuals.hyperlink_color, behind });
    response.on_hover_text(tip)
}

/// An icon on its own (not a button), in the text color.
pub fn icon_label(ui: &mut Ui, icon: Icon) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    let visuals = ui.visuals();
    let colors = Colors { line: visuals.text_color(), accent: visuals.hyperlink_color, behind: visuals.panel_fill };
    paint(ui.painter(), icon, rect.center(), colors);
}

/// A small button with an icon before its text (like "Ask" in the status
/// bar). `active` gives it the selection fill.
pub fn icon_text_button(ui: &mut Ui, icon: Icon, text: &str, tip: &str, active: bool) -> Response {
    const GAP: f32 = 6.0;
    const ICON: f32 = 14.0;
    let padding = ui.spacing().button_padding;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
    let height = ui.spacing().interact_size.y.min(galley.size().y + 2.0 * padding.y).max(ICON + 4.0);
    let size = vec2(padding.x * 2.0 + ICON + GAP + galley.size().x, height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, text));
    let visuals = ui.visuals();
    let widget = ui.style().interact(&response);
    let fill = if active { visuals.selection.bg_fill } else { widget.weak_bg_fill };
    let color = if active { visuals.strong_text_color() } else { widget.fg_stroke.color };
    let painter = ui.painter();
    painter.rect_filled(rect, widget.corner_radius, fill);
    let icon_center = pos2(rect.left() + padding.x + ICON / 2.0, rect.center().y);
    paint(painter, icon, icon_center, Colors { line: color, accent: visuals.hyperlink_color, behind: fill });
    let text_pos = pos2(rect.left() + padding.x + ICON + GAP, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_pos, galley, color);
    response.on_hover_text(tip)
}

/// Draw `icon` centered on `center` in Rubrique's icon style: a 14 × 14 box, heavy strokes with
/// round ends and joins, softly rounded corners, and one detail in `accent`.
/// The colors an icon is drawn with.
#[derive(Clone, Copy)]
struct Colors {
    line: Color32,
    /// The one accent detail.
    accent: Color32,
    /// What is behind the icon, for the gap around a badge.
    behind: Color32,
}

/// Two stacked server units with a light each; the plus badge is the accent.
fn paint_server_plus(painter: &Painter, center: Pos2, colors: Colors) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let line = Stroke::new(LINE, colors.line);
    for top in [-5.5, 0.5] {
        let unit = [at(-6.5, top), at(6.5, top), at(6.5, top + 5.0), at(-6.5, top + 5.0)];
        stroke_path(painter, &round_corners(&unit, CORNER, true), true, line);
        painter.circle_filled(at(-3.6, top + 2.5), 0.9, colors.line);
    }
    plus_badge(painter, at(4.0, 3.5), colors.accent, colors.behind);
}

fn paint(painter: &Painter, icon: Icon, center: Pos2, colors: Colors) {
    // Whole pixels keep the thin lines sharp.
    let center = center.round();
    let Colors { line: color, accent, behind } = colors;
    let line = Stroke::new(LINE, color);
    let detail = Stroke::new(LINE, accent);
    let at = |x: f32, y: f32| center + vec2(x, y);
    match icon {
        Icon::Gear => {
            // Ring with six rounded teeth; the hub is the accent.
            stroke_path(painter, &circle_points(center, 4.6), true, line);
            for tooth in 0..6_u8 {
                let direction = Vec2::angled(f32::from(tooth) * TAU / 6.0 + TAU / 12.0);
                stroke_path(painter, &[center + direction * 4.6, center + direction * 6.6], false, line);
            }
            painter.circle_filled(center, 1.6, accent);
        }
        Icon::FolderPlus => {
            // Folder with a tab; the round plus badge at the bottom right is the accent.
            let outline = [at(-6.5, -5.5), at(-2.0, -5.5), at(-0.4, -3.9), at(6.5, -3.9), at(6.5, 5.5), at(-6.5, 5.5)];
            stroke_path(painter, &round_corners(&outline, CORNER, true), true, line);
            plus_badge(painter, at(4.0, 3.5), accent, behind);
        }
        Icon::ServerPlus => paint_server_plus(painter, center, colors),
        Icon::Plus => plus(painter, center, 5.0, line),
        Icon::SidePanel => paint_side_panel(painter, center, line, detail),
        Icon::NewScratch => paint_new_scratch(painter, center, line, detail),
        Icon::Lock => paint_lock(painter, center, 1.0, color, accent),
        Icon::AudioPlus => paint_audio_plus(painter, center, line, accent, behind),
        Icon::Browser => paint_browser(painter, center, line, detail),
        Icon::RowPlus | Icon::RowMinus | Icon::ColumnPlus | Icon::ColumnMinus => {
            paint_table_icon(painter, icon, center, colors);
        }
        Icon::Format => paint_format(painter, center, line, detail),
        Icon::Highlighter => paint_highlighter(painter, center, line, detail),
        Icon::Filter => paint_filter(painter, center, line, detail),
        Icon::Refresh => {
            // Most of a circle; the arrowhead at its end is the accent.
            let radius = 5.6;
            let points: Vec<Pos2> = (0..=24_u8)
                .map(|step| center + Vec2::angled(-TAU * 0.22 + f32::from(step) / 24.0 * TAU * 0.78) * radius)
                .collect();
            let end_angle = -TAU * 0.22 + TAU * 0.78;
            let end = points[points.len() - 1];
            stroke_path(painter, &points, false, line);
            // Arrowhead along the circle's direction at its end (y points down,
            // so a growing angle turns clockwise on screen).
            let ahead = Vec2::angled(end_angle + TAU / 4.0);
            let outward = Vec2::angled(end_angle);
            let tip = end + ahead * 1.2;
            let wings = [tip - ahead * 3.0 + outward * 2.6, tip, tip - ahead * 3.0 - outward * 2.6];
            stroke_path(painter, &wings, false, detail);
        }
        Icon::Sparkle => {
            // A four-point star with curved sides; a small plus is the accent.
            let tips = [at(-1.5, -4.5), at(3.5, 1.0), at(-1.5, 6.5), at(-6.5, 1.0)];
            let waist = [at(-0.9, 0.4), at(-0.9, 1.6), at(-2.1, 1.6), at(-2.1, 0.4)];
            let mut outline = Vec::new();
            for side in 0..4 {
                outline.extend(quadratic(tips[side], waist[side], tips[(side + 1) % 4]));
            }
            stroke_path(painter, &outline, true, line);
            plus(painter, at(4.6, -4.5), 1.9, detail);
        }
        Icon::Stop => {
            // A circle; the square inside is the accent.
            stroke_path(painter, &circle_points(center, 6.2), true, line);
            let square = egui::Rect::from_center_size(center, vec2(3.8, 3.8));
            painter.rect_filled(square, 1.0, accent);
        }
        Icon::Play => {
            // A circle; the triangle inside is the accent.
            stroke_path(painter, &circle_points(center, 6.2), true, line);
            let triangle = [at(-1.6, -2.6), at(2.8, 0.0), at(-1.6, 2.6)];
            stroke_path(painter, &round_corners(&triangle, 0.6, true), true, detail);
        }
        Icon::Eye => {
            // An almond outline; the pupil is the accent.
            let mut outline = quadratic(at(-6.5, 0.0), at(0.0, -6.0), at(6.5, 0.0));
            outline.extend(quadratic(at(6.5, 0.0), at(0.0, 6.0), at(-6.5, 0.0)));
            stroke_path(painter, &outline, true, line);
            painter.circle_filled(center, 2.1, accent);
        }
        Icon::Terminal => paint_terminal(painter, center, line, detail),
        Icon::Code => {
            // Angle brackets; the slash between them is the accent.
            stroke_path(painter, &[at(-2.5, -4.0), at(-6.5, 0.0), at(-2.5, 4.0)], false, line);
            stroke_path(painter, &[at(2.5, -4.0), at(6.5, 0.0), at(2.5, 4.0)], false, line);
            stroke_path(painter, &[at(1.0, -4.5), at(-1.0, 4.5)], false, detail);
        }
        Icon::Bubble => paint_bubble(painter, center, line, accent),
        Icon::Close => {
            // Two crossing strokes, smaller than the box so they read as a control.
            stroke_path(painter, &[at(-3.5, -3.5), at(3.5, 3.5)], false, line);
            stroke_path(painter, &[at(3.5, -3.5), at(-3.5, 3.5)], false, line);
        }
        Icon::Mic => {
            // A capsule in a cradle; the stand under it is the accent.
            let capsule = [at(-2.6, -6.5), at(2.6, -6.5), at(2.6, 1.0), at(-2.6, 1.0)];
            stroke_path(painter, &round_corners(&capsule, 2.6, true), true, line);
            let cradle: Vec<Pos2> = (0..=16_u8)
                .map(|step| at(0.0, -1.5) + Vec2::angled(f32::from(step) / 16.0 * TAU / 2.0) * 5.0)
                .collect();
            stroke_path(painter, &cradle, false, line);
            stroke_path(painter, &[at(0.0, 3.5), at(0.0, 6.5)], false, detail);
            stroke_path(painter, &[at(-2.5, 6.5), at(2.5, 6.5)], false, detail);
        }
        Icon::Pause => {
            // A circle; the two bars inside are the accent.
            stroke_path(painter, &circle_points(center, 6.2), true, line);
            stroke_path(painter, &[at(-1.6, -2.6), at(-1.6, 2.6)], false, detail);
            stroke_path(painter, &[at(1.6, -2.6), at(1.6, 2.6)], false, detail);
        }
    }
}

/// A small flat button with `icon` (like a tab's close cross), `size` wide
/// and tall. `label` is what screen readers announce.
/// Draw `icon` alone (no button) centered on `center`.
pub fn paint_icon(painter: &Painter, icon: Icon, center: Pos2, line: Color32, accent: Color32, behind: Color32) {
    paint(painter, icon, center, Colors { line, accent, behind });
}

pub fn small_icon_button(ui: &mut Ui, icon: Icon, size: f32, label: &str, tip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    paint_small_button(ui, icon, rect, &response, false);
    response.on_hover_text(tip)
}

/// A small flat button like [`small_icon_button`] drawn on `rect` without
/// taking layout space, so it can sit beside a widget (like the cross next
/// to a preview block). `id` must be unique among the widgets on screen.
pub fn small_icon_button_on(ui: &Ui, rect: egui::Rect, id: egui::Id, icon: Icon, label: &str, tip: &str) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    paint_small_button(ui, icon, rect, &response, false);
    response.on_hover_text(tip)
}

/// A small flat button like [`small_icon_button`] that turns `on` and off
/// when clicked, and keeps the selection fill while it is on.
pub fn small_toggle_button(ui: &mut Ui, icon: Icon, size: f32, label: &str, tip: &str, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let selected = *on;
    response.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, label));
    paint_small_button(ui, icon, rect, &response, selected);
    response.on_hover_text(tip)
}

/// The fill and icon of a small button; `active` gives it the selection fill.
fn paint_small_button(ui: &Ui, icon: Icon, rect: egui::Rect, response: &Response, active: bool) {
    let visuals = ui.visuals();
    let fill = if active {
        visuals.selection.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let color = if active || response.hovered() { visuals.strong_text_color() } else { visuals.weak_text_color() };
    let painter = ui.painter();
    painter.rect_filled(rect, visuals.widgets.inactive.corner_radius, fill);
    let behind = if fill == Color32::TRANSPARENT { visuals.panel_fill } else { fill };
    paint(painter, icon, rect.center(), Colors { line: color, accent: visuals.hyperlink_color, behind });
}

/// A funnel: the cone narrows to a neck; the spout under it is the accent.
fn paint_filter(painter: &Painter, center: Pos2, line: Stroke, detail: Stroke) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let cone = [at(-6.5, -5.5), at(6.5, -5.5), at(1.6, 0.5), at(-1.6, 0.5)];
    stroke_path(painter, &round_corners(&cone, CORNER, true), true, line);
    stroke_path(painter, &[at(-1.6, 0.5), at(-1.6, 6.5), at(1.6, 4.5), at(1.6, 0.5)], false, detail);
}

/// A square open at the top right with a pencil writing into it; the
/// pencil tip is the accent, like the marker tip of the app icon.
fn paint_new_scratch(painter: &Painter, center: Pos2, line: Stroke, detail: Stroke) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let square = [at(0.0, -5.5), at(-6.5, -5.5), at(-6.5, 6.5), at(5.5, 6.5), at(5.5, 0.0)];
    stroke_path(painter, &round_corners(&square, CORNER, false), false, line);
    let along = vec2(1.0, -1.0).normalized();
    let across = vec2(1.0, 1.0).normalized();
    let tip = at(-1.6, 1.6);
    let cone = tip + along * 2.2;
    let back = tip + along * 10.4;
    let body = [cone + across, back + across, back - across, cone - across];
    stroke_path(painter, &body, true, line);
    stroke_path(painter, &[cone + across, tip, cone - across], false, detail);
}

/// A padlock: a body with a rounded shackle on top; the keyhole is the accent.
/// The padlock, drawn at `scale` times the icon size (1.0 in buttons;
/// smaller as the mark after a file name).
fn paint_lock(painter: &Painter, center: Pos2, scale: f32, line: Color32, accent: Color32) {
    let at = |x: f32, y: f32| center + vec2(x, y) * scale;
    let stroke = Stroke::new(LINE * scale, line);
    let body = [at(-5.5, -0.5), at(5.5, -0.5), at(5.5, 6.5), at(-5.5, 6.5)];
    stroke_path(painter, &round_corners(&body, CORNER * scale, true), true, stroke);
    // Half a circle from left to right over the top, with short legs down.
    let radius = 3.4 * scale;
    let mut shackle = vec![at(-3.4, -0.5)];
    shackle.extend(
        (0..=12_u8).map(|step| at(0.0, -3.4) + Vec2::angled(TAU / 2.0 + f32::from(step) / 12.0 * TAU / 2.0) * radius),
    );
    shackle.push(at(3.4, -0.5));
    stroke_path(painter, &shackle, false, stroke);
    painter.circle_filled(at(0.0, 2.4), 1.3 * scale, accent);
    stroke_path(painter, &[at(0.0, 2.4), at(0.0, 4.4)], false, Stroke::new(LINE * scale, accent));
}

/// Width of the padlock drawn after an encrypted file's name, in a tab and
/// in the explorer: smaller than the icons (14) so it does not crowd the text.
pub const LOCK_MARK_SIZE: f32 = 10.0;

/// Paint the padlock mark at [`LOCK_MARK_SIZE`], centered on `center`.
pub fn paint_lock_mark(painter: &Painter, center: Pos2, line: Color32, accent: Color32) {
    paint_lock(painter, center.round(), LOCK_MARK_SIZE / 14.0, line, accent);
}

/// Four waveform bars; the plus badge at the bottom right is the accent.
fn paint_audio_plus(painter: &Painter, center: Pos2, line: Stroke, accent: Color32, behind: Color32) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let bars = [(-6.0, -2.0, 2.0), (-3.0, -5.0, 5.0), (0.0, -3.0, 3.0), (3.0, -5.5, -1.0)];
    for (x, top, bottom) in bars {
        stroke_path(painter, &[at(x, top), at(x, bottom)], false, line);
    }
    plus_badge(painter, at(4.0, 3.5), accent, behind);
}

/// Three lines of text; the indented middle one is the accent.
fn paint_format(painter: &Painter, center: Pos2, line: Stroke, detail: Stroke) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    stroke_path(painter, &[at(-6.0, -4.5), at(6.0, -4.5)], false, line);
    stroke_path(painter, &[at(-2.5, 0.0), at(6.0, 0.0)], false, detail);
    stroke_path(painter, &[at(-6.0, 4.5), at(2.0, 4.5)], false, line);
}

/// A marker leaning right, like the pencil of New scratch but wider, with
/// a chisel nib; the stroke it left under it is the accent.
fn paint_highlighter(painter: &Painter, center: Pos2, line: Stroke, detail: Stroke) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let along = vec2(1.0, -1.0).normalized();
    let across = vec2(1.0, 1.0).normalized();
    let nib = at(-4.0, 2.5);
    let collar = nib + along * 2.6;
    let back = nib + along * 11.0;
    let half_width = across * 1.8;
    let body = [collar + half_width, back + half_width, back - half_width, collar - half_width];
    stroke_path(painter, &round_corners(&body, 1.0, true), true, line);
    let chisel = [collar + half_width, nib + across * 0.7, nib - across * 0.7, collar - half_width];
    stroke_path(painter, &chisel, false, line);
    stroke_path(painter, &[at(-6.5, 6.5), at(1.5, 6.5)], false, detail);
}

/// A table as a box split in two (rows, or columns for the column icons),
/// with a plus or minus badge at the bottom right as the accent.
fn paint_table_icon(painter: &Painter, icon: Icon, center: Pos2, colors: Colors) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let line = Stroke::new(LINE, colors.line);
    let outline = [at(-6.5, -5.5), at(6.5, -5.5), at(6.5, 5.5), at(-6.5, 5.5)];
    stroke_path(painter, &round_corners(&outline, CORNER, true), true, line);
    let divider = match icon {
        Icon::ColumnPlus | Icon::ColumnMinus => [at(0.0, -5.5), at(0.0, 5.5)],
        _ => [at(-6.5, 0.0), at(6.5, 0.0)],
    };
    stroke_path(painter, &divider, false, line);
    let badge_at = at(4.0, 3.5);
    match icon {
        Icon::RowMinus | Icon::ColumnMinus => minus_badge(painter, badge_at, colors.accent, colors.behind),
        _ => plus_badge(painter, badge_at, colors.accent, colors.behind),
    }
}

/// A window open at its top right corner, where an arrow (the accent)
/// points out of it.
fn paint_browser(painter: &Painter, center: Pos2, line: Stroke, detail: Stroke) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let frame = [at(1.5, -6.0), at(-6.5, -6.0), at(-6.5, 6.5), at(6.5, 6.5), at(6.5, -1.5)];
    stroke_path(painter, &round_corners(&frame, CORNER, false), false, line);
    stroke_path(painter, &[at(1.5, -6.5), at(6.5, -6.5), at(6.5, -1.5)], false, detail);
    stroke_path(painter, &[at(6.5, -6.5), at(0.5, -0.5)], false, detail);
}

/// A window split in two; the lines in the right column are the accent.
fn paint_side_panel(painter: &Painter, center: Pos2, line: Stroke, detail: Stroke) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let window = [at(-6.5, -5.5), at(6.5, -5.5), at(6.5, 5.5), at(-6.5, 5.5)];
    stroke_path(painter, &round_corners(&window, CORNER, true), true, line);
    stroke_path(painter, &[at(1.5, -5.5), at(1.5, 5.5)], false, line);
    stroke_path(painter, &[at(3.6, -2.5), at(4.4, -2.5)], false, detail);
    stroke_path(painter, &[at(3.6, 0.0), at(4.4, 0.0)], false, detail);
}

/// A window; the `>_` prompt inside is the accent.
fn paint_terminal(painter: &Painter, center: Pos2, line: Stroke, detail: Stroke) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let window = [at(-6.5, -5.0), at(6.5, -5.0), at(6.5, 5.0), at(-6.5, 5.0)];
    stroke_path(painter, &round_corners(&window, CORNER, true), true, line);
    stroke_path(painter, &[at(-4.0, -2.2), at(-1.6, 0.0), at(-4.0, 2.2)], false, detail);
    stroke_path(painter, &[at(0.2, 2.4), at(3.8, 2.4)], false, detail);
}

/// Points along a quadratic curve from `start` to `end` bent toward `control`.
fn quadratic(start: Pos2, control: Pos2, end: Pos2) -> Vec<Pos2> {
    (0..=8_u8)
        .map(|step| {
            let t = f32::from(step) / 8.0;
            let a = start + (control - start) * t;
            let b = control + (end - control) * t;
            a + (b - a) * t
        })
        .collect()
}

/// Corner radius of outlines: soft, like SF Symbols.
const CORNER: f32 = 2.0;
/// Radius of a badge's accent disc.
const BADGE: f32 = 4.25;
/// Radius of the gap cut around a badge, so it never touches the shape.
const BADGE_GAP: f32 = 5.5;
/// Half the length of the arms of the plus cut out of a badge.
const PLUS_HALF: f32 = 1.75;

/// A plus badge: a gap cut into the shape behind it, a disc in `accent`,
/// and a plus cut out of the disc in the color behind the icon.
fn plus_badge(painter: &Painter, center: Pos2, accent: Color32, behind: Color32) {
    painter.circle_filled(center, BADGE_GAP, behind);
    painter.circle_filled(center, BADGE, accent);
    plus(painter, center, PLUS_HALF, Stroke::new(LINE, behind));
}

/// Like [`plus_badge`], with a minus sign.
fn minus_badge(painter: &Painter, center: Pos2, accent: Color32, behind: Color32) {
    painter.circle_filled(center, BADGE_GAP, behind);
    painter.circle_filled(center, BADGE, accent);
    let stroke = Stroke::new(LINE, behind);
    stroke_path(painter, &[pos2(center.x - PLUS_HALF, center.y), pos2(center.x + PLUS_HALF, center.y)], false, stroke);
}

/// A path with round joins and round ends: the line, plus a dot of the
/// line's width at every point (egui draws plain lines with square ends).
/// A speech bubble with a short tail at the bottom left; the dot inside is
/// the accent.
fn paint_bubble(painter: &Painter, center: Pos2, line: Stroke, accent: Color32) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let outline =
        [at(-6.0, -5.5), at(6.0, -5.5), at(6.0, 2.5), at(-1.5, 2.5), at(-4.5, 5.5), at(-4.5, 2.5), at(-6.0, 2.5)];
    stroke_path(painter, &round_corners(&outline, 1.8, true), true, line);
    painter.circle_filled(at(0.0, -1.5), 1.4, accent);
}

fn stroke_path(painter: &Painter, points: &[Pos2], closed: bool, stroke: Stroke) {
    let shape = if closed {
        egui::Shape::closed_line(points.to_vec(), stroke)
    } else {
        egui::Shape::line(points.to_vec(), stroke)
    };
    painter.add(shape);
    for point in points {
        painter.circle_filled(*point, stroke.width / 2.0, stroke.color);
    }
}

/// `outline` with every corner replaced by a short curve of about `radius`.
fn round_corners(outline: &[Pos2], radius: f32, closed: bool) -> Vec<Pos2> {
    let count = outline.len();
    let mut points = Vec::with_capacity(count * 5);
    for index in 0..count {
        let corner = outline[index];
        let is_end = !closed && (index == 0 || index == count - 1);
        if is_end {
            points.push(corner);
            continue;
        }
        let previous = outline[(index + count - 1) % count];
        let next = outline[(index + 1) % count];
        let into = (corner - previous).normalized();
        let out_of = (next - corner).normalized();
        let reach = radius.min((corner - previous).length() / 2.0).min((next - corner).length() / 2.0);
        let start = corner - into * reach;
        let end = corner + out_of * reach;
        // Quadratic curve from `start` to `end` with the corner as control point.
        for step in 0..=4_u8 {
            let t = f32::from(step) / 4.0;
            let a = start + (corner - start) * t;
            let b = corner + (end - corner) * t;
            points.push(a + (b - a) * t);
        }
    }
    points
}

/// Points around a circle, for paths that need round joins.
fn circle_points(center: Pos2, radius: f32) -> Vec<Pos2> {
    (0..32_u8).map(|step| center + Vec2::angled(f32::from(step) / 32.0 * TAU) * radius).collect()
}

/// A plus sign with round ends.
fn plus(painter: &Painter, center: Pos2, half: f32, stroke: Stroke) {
    stroke_path(painter, &[pos2(center.x - half, center.y), pos2(center.x + half, center.y)], false, stroke);
    stroke_path(painter, &[pos2(center.x, center.y - half), pos2(center.x, center.y + half)], false, stroke);
}
