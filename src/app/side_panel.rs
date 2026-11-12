//! The right panel beside the page, for Markdown files: Structure (the
//! headings), Notes, Tasks or Changes (unsaved block changes). It stays
//! closed until asked for: the toggle at the right end of the tab bar,
//! Cmd+Shift+O (Structure), adding a note (Notes), or the "Unsaved: …"
//! button in the status bar (Changes). Its header switches between the
//! views and closes it, and the choice is remembered between runs.

use eframe::egui::{self, Context, Key, KeyboardShortcut, Modifiers, Rect, Sense, Ui, vec2};

use super::{NOTES_PANEL_WIDTH, OpenFile, RubriqueApp};
use crate::explorer::tree::is_markdown_path;
use crate::icons::{Icon, icon_button, paint_icon, small_icon_button};
use crate::markdown::notes::find_notes;
use crate::settings::layout::SideView;
use crate::terminal::mentions::find_delegations;
use crate::theme::style::page_fill;

const TOGGLE_STRUCTURE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O);
/// Width of the panel toggle in the tab bar.
pub(super) const TOGGLE_WIDTH: f32 = 28.0;
/// Height of the panel toggle in the tab bar.
const TOGGLE_HEIGHT: f32 = 24.0;
/// Radius of the dot on the toggle that says the closed panel has notes.
const NOTES_BADGE_RADIUS: f32 = 3.0;
/// Height of the Structure | Notes | Tasks | Changes switch.
const SWITCH_HEIGHT: f32 = 24.0;
/// Gap between the switch's rounded frame and its segments.
const SWITCH_INSET: f32 = 2.0;
/// Space left and right of a segment's icon and count.
const SEGMENT_PADDING: f32 = 12.0;
/// Width of the icon in a segment of the switch.
const SEGMENT_ICON: f32 = 14.0;
/// Gap between a segment's icon and its count.
const COUNT_GAP: f32 = 4.0;
/// Least space left and right of a segment's icon and count, in a narrow panel.
const MIN_SEGMENT_PADDING: f32 = 3.0;
/// Size of the panel's close button.
const CLOSE_SIZE: f32 = 22.0;
/// Margin around the panel's header.
const HEADER_MARGIN: i8 = 10;

impl RubriqueApp {
    /// Cmd+Shift+O shows the Structure, or closes the panel when it shows it.
    pub(super) fn side_panel_shortcut(&mut self, ctx: &Context) {
        if ctx.input_mut(|input| input.consume_shortcut(&TOGGLE_STRUCTURE)) {
            let layout = &mut self.settings.layout;
            if layout.side_panel_open && layout.side_view == SideView::Structure {
                layout.side_panel_open = false;
            } else {
                layout.side_panel_open = true;
                layout.side_view = SideView::Structure;
            }
        }
    }

    /// Open the panel on the notes, so a note just added is in sight.
    pub(super) fn show_notes_panel(&mut self) {
        self.settings.layout.side_panel_open = true;
        self.settings.layout.side_view = SideView::Notes;
    }

    /// True when the open tab can have the panel: a Markdown text file.
    pub(super) fn can_show_side_panel(&self) -> bool {
        matches!(&self.open_file, OpenFile::Text { document, .. } if is_markdown_path(&document.path))
    }

    /// The panel toggle at `rect` in the tab bar, highlighted while the
    /// panel is open. While it is closed, a dot on it says the file has notes.
    pub(super) fn side_panel_toggle(&mut self, ui: &mut Ui, rect: Rect) {
        let open = self.settings.layout.side_panel_open;
        // A quick look for a note's opening; this runs every frame.
        let has_notes =
            matches!(&self.open_file, OpenFile::Text { document, .. } if document.text.contains("<!-- note on \""));
        let tip = if open { "Hide the side panel" } else { "Show the structure and notes (Cmd+Shift+O)" };
        let toggle_rect = Rect::from_center_size(rect.center(), vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT));
        let response = ui.scope_builder(egui::UiBuilder::new().max_rect(toggle_rect), |ui| {
            icon_button(ui, Icon::SidePanel, "Side panel", tip, open)
        });
        if !open && has_notes {
            let badge = toggle_rect.right_top() + vec2(-6.0, 5.0);
            ui.painter().circle_filled(badge, NOTES_BADGE_RADIUS, ui.visuals().warn_fg_color);
        }
        if response.inner.clicked() {
            self.settings.layout.side_panel_open = !open;
        }
    }

    /// The panel, when it is open and the tab shows a Markdown file (not
    /// while merging, which needs the whole width).
    pub(super) fn right_panel(&mut self, ui: &mut Ui) {
        if !self.settings.layout.side_panel_open || self.merging.is_some() || !self.can_show_side_panel() {
            return;
        }
        let frame = egui::Frame::side_top_panel(&ui.ctx().global_style()).inner_margin(0);
        egui::Panel::right("side-panel").resizable(true).default_size(NOTES_PANEL_WIDTH).frame(frame).show(ui, |ui| {
            self.side_panel_header(ui);
            match self.settings.layout.side_view {
                SideView::Structure => self.structure_panel(ui),
                SideView::Notes => self.notes_panel(ui),
                SideView::Tasks => self.tasks_panel(ui),
                SideView::Changes => self.changes_panel(ui),
            }
        });
    }

    /// The Structure | Notes | Tasks | Changes switch (icons with counts) and the close cross.
    fn side_panel_header(&mut self, ui: &mut Ui) {
        self.refresh_block_changes();
        let change_count = self.block_changes.list.len();
        let (note_count, task_count) = match &self.open_file {
            OpenFile::Text { document, .. } => {
                (find_notes(&document.text).len(), find_delegations(&document.text).len())
            }
            _ => (0, 0),
        };
        let layout = &mut self.settings.layout;
        let margin = egui::Margin { left: HEADER_MARGIN, right: HEADER_MARGIN, top: HEADER_MARGIN, bottom: 0 };
        egui::Frame::new().inner_margin(margin).show(ui, |ui| {
            ui.horizontal(|ui| {
                let choices = [
                    ViewChoice { view: SideView::Structure, icon: Icon::Outline, name: "Structure", count: 0 },
                    ViewChoice { view: SideView::Notes, icon: Icon::Bubble, name: "Notes", count: note_count },
                    ViewChoice { view: SideView::Tasks, icon: Icon::Checklist, name: "Tasks", count: task_count },
                    ViewChoice { view: SideView::Changes, icon: Icon::Changes, name: "Changes", count: change_count },
                ];
                // Room for the close cross, so the switch never runs under it.
                let reserved = CLOSE_SIZE + ui.spacing().item_spacing.x;
                if let Some(view) = view_switch(ui, &choices, layout.side_view, reserved) {
                    layout.side_view = view;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_icon_button(ui, Icon::Close, CLOSE_SIZE, "Close panel", "Hide the side panel").clicked() {
                        layout.side_panel_open = false;
                    }
                });
            });
        });
    }
}

/// One segment of the side panel's switch (or the explorer's Files | Tabs
/// switch): an icon, with the number of items after it when there are any.
/// The name shows on hover and is what screen readers announce; the panel
/// is too narrow for the words.
pub(super) struct ViewChoice<View> {
    pub(super) view: View,
    pub(super) icon: Icon,
    pub(super) name: &'static str,
    pub(super) count: usize,
}

/// A segmented switch in the style of the Preview | Source switch: the
/// chosen segment filled like the page. It leaves `reserved` points of the
/// row free, with less space around the icons when the panel is narrow.
/// Returns the segment clicked.
pub(super) fn view_switch<View: Copy + PartialEq>(
    ui: &mut Ui,
    choices: &[ViewChoice<View>],
    current: View,
    reserved: f32,
) -> Option<View> {
    let visuals = ui.visuals().clone();
    let font = egui::TextStyle::Body.resolve(ui.style());
    // The count after an icon, or nothing when there are no items.
    let counts: Vec<_> = choices
        .iter()
        .map(|choice| {
            (choice.count > 0).then(|| {
                ui.painter().layout_no_wrap(choice.count.to_string(), font.clone(), egui::Color32::PLACEHOLDER)
            })
        })
        .collect();
    let contents: Vec<f32> = counts
        .iter()
        .map(|count| SEGMENT_ICON + count.as_ref().map_or(0.0, |galley| COUNT_GAP + galley.size().x))
        .collect();
    let contents_width: f32 = contents.iter().sum();
    let room = ui.available_width() - reserved - SWITCH_INSET * 2.0 - contents_width;
    #[expect(clippy::cast_precision_loss, reason = "a handful of segments")]
    let padding = (room / (choices.len() as f32 * 2.0)).clamp(MIN_SEGMENT_PADDING, SEGMENT_PADDING);
    let widths: Vec<f32> = contents.iter().map(|content| content + padding * 2.0).collect();
    let total = widths.iter().sum::<f32>() + SWITCH_INSET * 2.0;
    // Centered in the header, but never so far right that it runs under
    // the close cross.
    let available = ui.available_width();
    let indent = ((available - total) / 2.0).min(available - reserved - total).max(0.0);
    // add_space is followed by the usual gap between items, so take it off.
    let gap = ui.spacing().item_spacing.x;
    if indent > gap {
        ui.add_space(indent - gap);
    }
    let (rect, _) = ui.allocate_exact_size(vec2(total, SWITCH_HEIGHT), Sense::hover());
    let switch_fill = visuals.widgets.inactive.weak_bg_fill;
    ui.painter().rect_filled(rect, 7, switch_fill);
    let mut left = rect.left() + SWITCH_INSET;
    let mut clicked = None;
    for (((choice, count), content), width) in choices.iter().zip(counts).zip(contents).zip(widths) {
        let segment = Rect::from_min_size(
            egui::pos2(left, rect.top() + SWITCH_INSET),
            vec2(width, SWITCH_HEIGHT - SWITCH_INSET * 2.0),
        );
        left += width;
        let response = ui.interact(segment, ui.scope_id().with(("side-view", choice.name)), Sense::click());
        let selected = choice.view == current;
        let fill = if selected {
            page_fill(&visuals)
        } else if response.hovered() {
            visuals.widgets.hovered.weak_bg_fill
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(segment, 5, fill);
        let color = if selected { visuals.strong_text_color() } else { visuals.weak_text_color() };
        let behind = if fill == egui::Color32::TRANSPARENT { switch_fill } else { fill };
        let content_left = segment.center().x - content / 2.0;
        let icon_center = egui::pos2(content_left + SEGMENT_ICON / 2.0, segment.center().y);
        paint_icon(ui.painter(), choice.icon, icon_center, color, visuals.hyperlink_color, behind);
        if let Some(galley) = count {
            let text_pos =
                egui::pos2(content_left + SEGMENT_ICON + COUNT_GAP, segment.center().y - galley.size().y / 2.0);
            ui.painter().galley_with_override_text_color(text_pos, galley, color);
        }
        let label =
            if choice.count == 0 { choice.name.to_string() } else { format!("{} {}", choice.name, choice.count) };
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, &label));
        if response.on_hover_text(&label).clicked() {
            clicked = Some(choice.view);
        }
    }
    clicked
}
