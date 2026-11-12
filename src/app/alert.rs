//! The look of Rubrique's alert dialogs: a padded frame with a title, a
//! line of weak text, and a row of buttons where the usual choice is
//! filled with the accent color and marked with a focus ring (Enter takes
//! it; Esc or a click outside cancels).

use eframe::egui::{
    self, Align, Color32, Context, CornerRadius, Frame, Id, Key, Layout, Modal, ModalResponse, Rgba, RichText, Role,
    Sense, Shadow, Stroke, StrokeKind, Style, Ui, Visuals, WidgetInfo, vec2,
};

/// Space between the dialog's edge and its content.
const PADDING: i8 = 20;
/// Corner rounding of the dialog.
const RADIUS: u8 = 12;
/// Space between the title, the text and the button row.
const LINE_GAP: f32 = 12.0;
/// Font size of the title (the body text keeps the interface size).
const TITLE_SIZE: f32 = 17.0;
/// Height of a dialog button.
const BUTTON_HEIGHT: f32 = 32.0;
/// Space left and right of a filled button's label.
const BUTTON_PADDING: f32 = 14.0;
/// Space left and right of an unfilled (Discard) button's label.
const TEXT_BUTTON_PADDING: f32 = 8.0;
/// Corner rounding of a dialog button.
const BUTTON_RADIUS: u8 = 6;
/// Space between buttons in a row.
const BUTTON_GAP: f32 = 8.0;
/// Width of the focus ring around the button Enter takes.
const RING_WIDTH: f32 = 3.0;
/// How strongly the ring shows the accent color.
const RING_OPACITY: f32 = 0.35;
/// How far a filled button moves toward the strong text color under the pointer.
const HOVER_SHIFT: f32 = 0.12;
/// Linear intensity above which a fill is light enough for dark text on it.
const LIGHT_FILL: f32 = 0.25;

/// What a dialog button does, which sets how it looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ButtonKind {
    /// The usual choice (Save, Rename): filled with the accent color.
    Primary,
    /// Any other choice, Cancel included: the surface fill.
    Secondary,
    /// Throws edits away (Discard): red text, filled only under the pointer.
    Discard,
    /// Deletes a file (Move to Trash): filled red.
    Delete,
}

/// Show an alert dialog `width` points wide with the shared frame and
/// spacing. Esc and a click outside close it ([`ModalResponse::should_close`]).
pub(super) fn show<T>(
    ctx: &Context,
    id: &'static str,
    width: f32,
    add_contents: impl FnOnce(&mut Ui) -> T,
) -> ModalResponse<T> {
    Modal::new(Id::unique(id)).frame(frame(&ctx.global_style())).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = LINE_GAP;
        add_contents(ui)
    })
}

/// The dialog frame: the page fill, a border that still shows on dark
/// themes, and a soft shadow.
fn frame(style: &Style) -> Frame {
    let visuals = &style.visuals;
    let shadow_alpha = if visuals.dark_mode { 160 } else { 70 };
    Frame::popup(style)
        .fill(visuals.window_fill)
        .inner_margin(PADDING)
        .corner_radius(RADIUS)
        .stroke(Stroke::new(1.0, edge_color(visuals)))
        .shadow(Shadow { offset: [0, 18], blur: 48, spread: 0, color: Color32::from_black_alpha(shadow_alpha) })
}

/// The dialog's border. Dark themes draw their borders close to the page
/// fill, which vanishes against the dimmed app behind a dialog, so lift it
/// a quarter of the way toward the weak text color there.
fn edge_color(visuals: &Visuals) -> Color32 {
    let border = visuals.window_stroke.color;
    if visuals.dark_mode { border.lerp_to_gamma(visuals.weak_text_color(), 0.25) } else { border }
}

/// The dialog's question, in the strong text color.
pub(super) fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(TITLE_SIZE).color(ui.visuals().strong_text_color()));
}

/// A line of explanation under the title, in the weak text color.
pub(super) fn body(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(ui.visuals().weak_text_color()));
}

/// A row of buttons: `left` packed from the left edge, `right` from the
/// right edge. `right` adds its buttons right to left, so the first one it
/// adds ends up rightmost: add the usual choice first.
pub(super) fn button_row<L, R>(
    ui: &mut Ui,
    left: impl FnOnce(&mut Ui) -> L,
    right: impl FnOnce(&mut Ui) -> R,
) -> (L, R) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = BUTTON_GAP;
        let left = left(ui);
        let right = ui.with_layout(Layout::right_to_left(Align::Center), right).inner;
        (left, right)
    })
    .inner
}

/// Whether Enter was pressed for the dialog's usual choice: no dialog
/// button has keyboard focus (Enter on a focused button clicks that one).
pub(super) fn enter_for_default(ui: &Ui, buttons: &[&egui::Response]) -> bool {
    !buttons.iter().any(|button| button.has_focus()) && ui.input(|input| input.key_pressed(Key::Enter))
}

/// A dialog button. `is_default` marks the one Enter takes with the focus
/// ring while no other button has keyboard focus; a focused button always
/// gets the ring.
pub(super) fn button(ui: &mut Ui, label: &str, kind: ButtonKind, is_default: bool) -> egui::Response {
    let padding = if kind == ButtonKind::Discard { TEXT_BUTTON_PADDING } else { BUTTON_PADDING };
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let size = vec2(galley.size().x + 2.0 * padding, BUTTON_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let visuals = ui.visuals();
    let something_focused = ui.memory(|memory| memory.focused().is_some());
    let ringed = response.has_focus() || (is_default && !something_focused);
    let (fill, text_color) = button_colors(visuals, kind, response.hovered() || response.is_pointer_button_down_on());
    let painter = ui.painter();
    let radius = CornerRadius::same(BUTTON_RADIUS);
    painter.rect_filled(rect, radius, fill);
    if ringed {
        let ring = Stroke::new(RING_WIDTH, visuals.hyperlink_color.gamma_multiply(RING_OPACITY));
        painter.rect_stroke(rect, radius, ring, StrokeKind::Outside);
        painter.rect_stroke(rect, radius, Stroke::new(1.0, visuals.hyperlink_color), StrokeKind::Inside);
    }
    let text_pos = rect.center() - galley.size() / 2.0;
    painter.galley(text_pos, galley, text_color);
    response
}

/// Fill and label color of a button of `kind`, `active` while under the
/// pointer or pressed.
fn button_colors(visuals: &Visuals, kind: ButtonKind, active: bool) -> (Color32, Color32) {
    match kind {
        ButtonKind::Primary => filled(visuals, visuals.hyperlink_color, active),
        ButtonKind::Delete => filled(visuals, visuals.error_fg_color, active),
        ButtonKind::Secondary => {
            let state = if active { &visuals.widgets.hovered } else { &visuals.widgets.inactive };
            (state.weak_bg_fill, visuals.strong_text_color())
        }
        ButtonKind::Discard => {
            let fill = if active { visuals.error_fg_color.gamma_multiply(0.14) } else { Color32::TRANSPARENT };
            (fill, visuals.error_fg_color)
        }
    }
}

/// A button filled with `color`, nudged toward the strong text color under
/// the pointer, with a label that reads on it.
fn filled(visuals: &Visuals, color: Color32, active: bool) -> (Color32, Color32) {
    let fill = if active { color.lerp_to_gamma(visuals.strong_text_color(), HOVER_SHIFT) } else { color };
    (fill, text_on(fill))
}

/// White on a dark fill, near black on a light one: the accent is a deep
/// blue on light themes and a pale violet on dark ones.
fn text_on(fill: Color32) -> Color32 {
    if Rgba::from(fill).intensity() > LIGHT_FILL { Color32::from_rgb(0x15, 0x13, 0x1f) } else { Color32::WHITE }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Modifiers, RawInput};

    /// What one frame of a Delete / Cancel dialog saw.
    struct Frame {
        delete_id: Id,
        delete_clicked: bool,
        enter_for_cancel: bool,
    }

    /// Run one frame of a dialog laid out like "Delete", with `keys` pressed.
    fn delete_dialog_frame(ctx: &Context, keys: &[Key]) -> Frame {
        let events = keys
            .iter()
            .map(|&key| Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            })
            .collect();
        let mut seen = None;
        let mut output = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
            let modal = show(ui.ctx(), "test-dialog", 300.0, |ui| {
                button_row(
                    ui,
                    |_| (),
                    |ui| {
                        let delete = button(ui, "Move to Trash", ButtonKind::Delete, false);
                        let cancel = button(ui, "Cancel", ButtonKind::Secondary, true);
                        let enter = enter_for_default(ui, &[&delete, &cancel]);
                        Frame { delete_id: delete.id, delete_clicked: delete.clicked(), enter_for_cancel: enter }
                    },
                )
                .1
            });
            seen = Some(modal.inner);
        });
        // Fonts are uploaded as textures; a test has no painter to take them.
        output.textures_delta.clear();
        seen.expect("the dialog ran")
    }

    #[test]
    fn enter_takes_the_default_until_another_button_has_focus() {
        let ctx = Context::default();
        let first = delete_dialog_frame(&ctx, &[]);
        let pressed = delete_dialog_frame(&ctx, &[Key::Enter]);
        assert!(pressed.enter_for_cancel);
        assert!(!pressed.delete_clicked);
        // Tab to the delete button: Enter clicks it and leaves the default alone.
        ctx.memory_mut(|memory| memory.request_focus(first.delete_id));
        delete_dialog_frame(&ctx, &[]);
        let focused = delete_dialog_frame(&ctx, &[Key::Enter]);
        assert!(focused.delete_clicked);
        assert!(!focused.enter_for_cancel);
    }

    #[test]
    fn labels_read_on_the_builtin_accents_and_errors() {
        // Light theme accent and error: dark fills, white labels.
        assert_eq!(text_on(Color32::from_rgb(0x2f, 0x6f, 0xde)), Color32::WHITE);
        assert_eq!(text_on(Color32::from_rgb(0xc6, 0x2f, 0x2f)), Color32::WHITE);
        // Indigo accent and error: light fills, dark labels.
        assert_ne!(text_on(Color32::from_rgb(0xa7, 0x9b, 0xff)), Color32::WHITE);
        assert_ne!(text_on(Color32::from_rgb(0xf0, 0x72, 0x72)), Color32::WHITE);
    }

    #[test]
    fn dark_themes_lift_the_dialog_border() {
        let dark = Visuals::dark();
        assert_ne!(edge_color(&dark), dark.window_stroke.color);
        let light = Visuals::light();
        assert_eq!(edge_color(&light), light.window_stroke.color);
    }
}
