//! Remember the text selected in the preview for selection actions.
//!
//! egui reports a finished label selection only at the end of a pass, as an
//! output command; this plugin, registered after egui's own, catches it and
//! keeps it (with where the pointer was) for the next frame.

use eframe::egui::{self, Context, Id, OutputCommand, Pos2, Ui};

/// The selected preview text and the pointer position when it was made.
#[derive(Clone, Debug)]
pub struct PreviewSelection {
    pub text: String,
    pub pointer: Pos2,
}

/// Install the plugin once, when the app starts.
pub fn install(ctx: &Context) {
    ctx.add_plugin(SelectionTracker::default());
}

/// Pointer movement (points) between press and release that counts as a
/// drag; a plain click on a label also reports its text, which is not a
/// selection the user made.
const MIN_DRAG: f32 = 4.0;

/// The current preview selection, if any.
pub fn current(ctx: &Context) -> Option<PreviewSelection> {
    ctx.data(|data| data.get_temp::<PreviewSelection>(selection_id()))
}

/// Forget the selection (after a note was added).
pub fn clear(ctx: &Context) {
    ctx.data_mut(|data| {
        data.remove::<PreviewSelection>(selection_id());
        data.remove::<egui::Rect>(button_id());
        data.remove::<egui::Rect>(read_aloud_button_id());
    });
    ctx.plugin::<egui::text_selection::LabelSelectionState>().lock().clear_selection();
}

/// Where the "Add note" button was drawn this frame.
pub fn set_button_rect(ctx: &Context, rect: egui::Rect) {
    ctx.data_mut(|data| data.insert_temp(button_id(), rect));
}

/// Where the Read aloud button was drawn this frame.
pub fn set_read_aloud_button_rect(ctx: &Context, rect: egui::Rect) {
    ctx.data_mut(|data| data.insert_temp(read_aloud_button_id(), rect));
}

fn read_aloud_button_id() -> Id {
    Id::unique("rubrique-read-aloud-button")
}

fn selection_id() -> Id {
    Id::unique("rubrique-preview-selection")
}

fn button_id() -> Id {
    Id::unique("rubrique-add-note-button")
}

/// Keep the selection until a click that started on a selection action ends.
fn clicking_button(ctx: &Context, press_origin: Option<Pos2>) -> bool {
    let Some(origin) = press_origin else {
        return false;
    };
    let pressing = ctx.input(|input| input.pointer.any_down() || input.pointer.any_released());
    pressing
        && ctx.data(|data| {
            [button_id(), read_aloud_button_id()]
                .into_iter()
                .any(|id| data.get_temp::<egui::Rect>(id).is_some_and(|rect| rect.contains(origin)))
        })
}

#[derive(Default)]
struct SelectionTracker {
    /// Where the primary button went down.
    press_origin: Option<Pos2>,
}

impl egui::Plugin for SelectionTracker {
    fn debug_name(&self) -> &'static str {
        "rubrique selection tracker"
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        if let Some(origin) = ui.ctx().input(|input| input.pointer.press_origin()) {
            self.press_origin = Some(origin);
        }
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let settled = ctx.output(|output| {
            output.commands.iter().rev().find_map(|command| match command {
                OutputCommand::TextSelectionSettled(text) => Some(text.clone()),
                _ => None,
            })
        });
        let has_selection = ctx.plugin::<egui::text_selection::LabelSelectionState>().lock().has_selection();
        if !has_selection {
            if !clicking_button(&ctx, self.press_origin) {
                ctx.data_mut(|data| {
                    data.remove::<PreviewSelection>(selection_id());
                    data.remove::<egui::Rect>(button_id());
                    data.remove::<egui::Rect>(read_aloud_button_id());
                });
            }
            return;
        }
        if let Some(text) = settled.filter(|text| !text.trim().is_empty()) {
            let pointer = ctx.input(|input| input.pointer.latest_pos()).unwrap_or_default();
            let dragged = self.press_origin.is_some_and(|origin| origin.distance(pointer) >= MIN_DRAG);
            if !dragged {
                return;
            }
            ctx.data_mut(|data| data.insert_temp(selection_id(), PreviewSelection { text, pointer }));
        }
    }
}
