//! Section headers of the explorer ("FILES", "TERMINALS"): a fold arrow and
//! a small uppercase title on the left, the section's buttons on the right.
//! Both sections share this look so the panel reads as one list of parts.

use eframe::egui::{self, Color32, Painter, Pos2, Sense, Ui, vec2};

/// Height of a section header row.
pub const SECTION_HEADER_HEIGHT: f32 = 28.0;
/// Size of the title text: smaller than the file names, like a label, but
/// not so small next to 14 point names that it drops out.
const TITLE_SIZE: f32 = 12.0;
/// Room before the title for the fold arrow.
const ARROW_ROOM: f32 = 18.0;
/// Width and height of a fold arrow, in points.
const ARROW_SIZE: f32 = 7.0;

/// How much room a section header has for its buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonRoom {
    /// The whole row of buttons fits beside the title.
    Row,
    /// The row would run over the title. A section with several buttons
    /// shows one menu button in their place.
    Menu,
}

/// One section header. `title` is drawn as given (callers pass it in
/// capitals). `after_title` adds small details next to the title (a count,
/// status dots); `buttons` adds the section's buttons, right-aligned, and
/// is told whether the whole row fits. Returns true when the title was
/// clicked (the caller folds or unfolds the section).
pub fn section_header(
    ui: &mut Ui,
    title: &str,
    expanded: bool,
    after_title: impl FnOnce(&mut Ui),
    buttons: impl FnOnce(&mut Ui, ButtonRoom),
) -> bool {
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    let size = vec2(ui.available_width(), SECTION_HEADER_HEIGHT);
    ui.allocate_ui_with_layout(size, layout, |ui| {
        let clicked = section_title(ui, title, expanded);
        after_title(ui);
        section_buttons(ui, title, buttons);
        clicked
    })
    .inner
}

/// The section's buttons, right-aligned in the room left after the title.
/// The width of the whole row is remembered from the last frame that drew
/// it, to tell `buttons` when the panel is too narrow for it.
fn section_buttons(ui: &mut Ui, title: &str, buttons: impl FnOnce(&mut Ui, ButtonRoom)) {
    let width_id = ui.scope_id().with(("section-buttons-width", title));
    let row_width: Option<f32> = ui.data(|data| data.get_temp(width_id));
    let fits = row_width.is_none_or(|width| width <= ui.available_width());
    let room = if fits { ButtonRoom::Row } else { ButtonRoom::Menu };
    let size = vec2(ui.available_width(), SECTION_HEADER_HEIGHT);
    let layout = egui::Layout::right_to_left(egui::Align::Center);
    let drawn = ui.allocate_ui_with_layout(size, layout, |ui| {
        if row_width.is_none() {
            // The first frame only measures the row, so a narrow panel
            // never shows it over the title.
            ui.set_invisible();
            ui.ctx().request_repaint();
        }
        buttons(ui, room);
    });
    if room == ButtonRoom::Row {
        ui.data_mut(|data| data.insert_temp(width_id, drawn.response.rect.width()));
    }
}

/// The fold arrow and the title, as one click target.
fn section_title(ui: &mut Ui, title: &str, expanded: bool) -> bool {
    let font = egui::FontId::proportional(TITLE_SIZE);
    let galley = ui.painter().layout_no_wrap(title.to_string(), font, Color32::PLACEHOLDER);
    let size = vec2(ARROW_ROOM + galley.size().x + 6.0, SECTION_HEADER_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let visuals = ui.visuals();
    let color = if response.hovered() { visuals.strong_text_color() } else { visuals.weak_text_color() };
    let arrow_center = egui::pos2(rect.left() + ARROW_ROOM / 2.0, rect.center().y);
    paint_fold_arrow(ui.painter(), arrow_center, expanded, color);
    let text_top = rect.center().y - galley.size().y / 2.0;
    ui.painter().galley_with_override_text_color(egui::pos2(rect.left() + ARROW_ROOM, text_top), galley, color);
    let state = if expanded { "Collapse" } else { "Expand" };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    response.on_hover_text(format!("{state} {}", title.to_lowercase())).clicked()
}

/// A small triangle: pointing right when folded, down when open. Painted by
/// hand because the default egui fonts have no arrow glyphs.
pub fn paint_fold_arrow(painter: &Painter, center: Pos2, expanded: bool, color: Color32) {
    let half = ARROW_SIZE / 2.0;
    let points = if expanded {
        vec![center + vec2(-half, -half / 2.0), center + vec2(half, -half / 2.0), center + vec2(0.0, half)]
    } else {
        vec![center + vec2(-half / 2.0, -half), center + vec2(half, 0.0), center + vec2(-half / 2.0, half)]
    };
    painter.add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Context, RawInput, Rect};

    /// The room a header with five buttons reports in a panel this wide,
    /// once it has measured the row.
    fn room_in_panel(panel_width: f32) -> ButtonRoom {
        let ctx = Context::default();
        let mut reported = ButtonRoom::Row;
        for _ in 0..2 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(panel_width, 200.0))),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    section_header(
                        ui,
                        "FILES",
                        true,
                        |_| {},
                        |ui, room| {
                            reported = room;
                            for _ in 0..5 {
                                ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                            }
                        },
                    );
                });
            });
            output.textures_delta.clear();
        }
        reported
    }

    #[test]
    fn a_narrow_panel_asks_for_the_menu() {
        assert_eq!(room_in_panel(80.0), ButtonRoom::Menu);
    }

    #[test]
    fn a_wide_panel_keeps_the_row_of_buttons() {
        assert_eq!(room_in_panel(400.0), ButtonRoom::Row);
    }
}
