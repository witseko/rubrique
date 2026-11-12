//! Tabs listed in the explorer (Settings > Appearance > Tabs: In the
//! explorer). A Files | Tabs switch at the top of the explorer picks what
//! it shows; Tabs lists the open tabs one per row, so long names show in
//! full, grouped by folder like the tab strip. The row above the page then
//! lists no tabs (see `RubriqueApp::page_bar`).

use eframe::egui::{self, Color32, Sense, Ui, pos2, vec2};

use super::RubriqueApp;
use super::side_panel::{ViewChoice, view_switch};
use super::tab_groups::ChipClick;
use super::tabs::{TabAction, TabView, close_or_dirty_mark, tab_click_action};
use crate::explorer::view::{InlineRename, rename_field};
use crate::icons::{Icon, LOCK_MARK_SIZE, paint_lock_mark};
use crate::settings::layout::ExplorerView;
use crate::theme::style::page_fill;

/// Height of a tab's row.
const ROW_HEIGHT: f32 = 28.0;
/// Space before a tab's name (after the line on its left edge).
const ROW_PADDING: f32 = 16.0;
/// Space right of the close cross.
const ROW_END_PADDING: f32 = 6.0;
/// Room for the close cross (or the unsaved dot) at the end of a row.
const CLOSE_SIZE: f32 = 18.0;
/// Width of the line on a tab's left edge (the accent on the active tab).
const EDGE_LINE: f32 = 2.0;
/// Space between an encrypted tab's name and its padlock.
const LOCK_GAP: f32 = 6.0;
/// Space above and below the Files | Tabs switch.
const SWITCH_MARGIN: f32 = 6.0;

impl RubriqueApp {
    /// The Files | Tabs switch at the top of the explorer. The Tabs half
    /// shows how many tabs are open.
    pub(super) fn explorer_view_switch(&mut self, ui: &mut Ui) {
        let choices = [
            ViewChoice { view: ExplorerView::Files, icon: Icon::Files, name: "Files", count: 0 },
            ViewChoice { view: ExplorerView::Tabs, icon: Icon::TabList, name: "Tabs", count: self.tabs.len() },
        ];
        ui.add_space(SWITCH_MARGIN);
        ui.horizontal(|ui| {
            if let Some(view) = view_switch(ui, &choices, self.settings.layout.explorer_view, 0.0) {
                self.settings.layout.explorer_view = view;
            }
        });
        ui.add_space(SWITCH_MARGIN);
    }

    /// The open tabs, one per row, in tab order. With more than one folder
    /// each group starts with its chip, which folds the group like in the
    /// tab strip.
    pub(super) fn tab_list(&mut self, ui: &mut Ui) {
        if self.tabs.is_empty() {
            ui.add_space(SWITCH_MARGIN);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new("No open tabs").weak()));
            return;
        }
        let mut action = None;
        let mut fold = None;
        egui::ScrollArea::vertical().id_salt("tab-list").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let grouped = self.tabs_grouped();
            let mut current_group = None;
            for index in 0..self.tabs.len() {
                let group = self.group_of(self.tab(index));
                let folded = grouped && self.group_folded(group);
                if grouped && current_group != Some(group) {
                    current_group = Some(group);
                    // Only a click counts here: the groups are reordered by
                    // dragging chips in the tab strip or folders in Files.
                    if let Some(ChipClick::Toggle(group)) = self.group_chip(ui, group, folded) {
                        fold = Some(group);
                    }
                }
                if folded && index != self.active_tab {
                    continue;
                }
                let tab = self.tab_view(index, group);
                if let Some(found) = tab_row(ui, &tab, index, self.tabs.len(), &mut self.tab_rename) {
                    action = Some(found);
                }
            }
        });
        if let Some(group) = fold {
            self.toggle_group(group);
        }
        if let Some(action) = action {
            self.apply_tab_action(ui.ctx(), action);
        }
    }
}

/// One tab's row: the line on its left edge, the name (cut with "…" when
/// the explorer is narrow), the padlock for an encrypted file, and the
/// close cross or the unsaved dot. Clicks work like on a tab in the strip.
fn tab_row(
    ui: &mut Ui,
    tab: &TabView,
    index: usize,
    count: usize,
    rename: &mut Option<InlineRename>,
) -> Option<TabAction> {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let visuals = ui.visuals().clone();
    if tab.is_renamed(rename.as_ref()) {
        let field_rect = rect.shrink2(vec2(ROW_PADDING / 2.0, 2.0));
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        let mut field = ui.new_child(egui::UiBuilder::new().max_rect(field_rect).layout(layout));
        rename_field(&mut field, rename, egui::Id::unique("tab-list-rename-field"), field_rect.width());
        return None;
    }
    // Own id, like a tab in the strip: the cross takes an automatic one.
    let response = ui.interact(rect, ui.scope_id().with(("tab-row", index)), Sense::click());
    // Pointer anywhere on the row, the close cross included (see `show_tab`).
    let hovered = response.contains_pointer();
    let fill = if tab.active {
        page_fill(&visuals)
    } else if hovered {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 0, fill);
    if let Some(color) = tab.line_color(&visuals) {
        painter.rect_filled(egui::Rect::from_min_size(rect.min, vec2(EDGE_LINE, rect.height())), 0, color);
    }
    let text_color = if tab.active { visuals.strong_text_color() } else { visuals.text_color() };
    let lock_width = if tab.encrypted { LOCK_GAP + LOCK_MARK_SIZE } else { 0.0 };
    let max_text = (rect.width() - ROW_PADDING - lock_width - CLOSE_SIZE - ROW_END_PADDING * 2.0).max(0.0);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut job = egui::text::LayoutJob::simple_singleline(tab.name.clone(), font, text_color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_text);
    let galley = painter.layout_job(job);
    let text_pos = pos2(rect.left() + ROW_PADDING, rect.center().y - galley.size().y / 2.0);
    let text_width = galley.size().x;
    painter.galley(text_pos, galley, text_color);
    if tab.encrypted {
        let center = pos2(text_pos.x + text_width + LOCK_GAP + LOCK_MARK_SIZE / 2.0, rect.center().y);
        paint_lock_mark(painter, center, text_color, visuals.hyperlink_color);
    }
    let close_rect = egui::Rect::from_center_size(
        pos2(rect.right() - ROW_END_PADDING - CLOSE_SIZE / 2.0, rect.center().y),
        vec2(CLOSE_SIZE, CLOSE_SIZE),
    );
    let close = close_or_dirty_mark(ui, tab, index, close_rect, hovered);
    tab_click_action(response, tab, index, count, close)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::OpenFile;
    use eframe::egui::{Context, Event, PointerButton, Pos2, RawInput, Rect, Vec2};

    /// Space the central panel leaves around the list (egui's default margin).
    const PANEL_MARGIN: f32 = 8.0;

    #[test]
    fn clicking_a_row_switches_to_its_tab() {
        let dir = std::env::temp_dir().join(format!("rubrique-tab-list-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = Context::default();
        let mut app = RubriqueApp::new(&ctx, crate::settings::Settings::default(), vec![dir.clone()], None);
        // `new` reopens this machine's drafts as tabs; only the three below count.
        app.tabs.clear();
        app.active_tab = 0;
        app.open_file = OpenFile::Nothing;
        for index in 0..3 {
            let path = dir.join(format!("file-{index}.md"));
            std::fs::write(&path, "x").unwrap();
            app.open_in_tab(path);
        }
        app.switch_to_tab(0);
        // The middle of the third row; one folder, so no group chips above it.
        let pointer = egui::pos2(60.0, PANEL_MARGIN + ROW_HEIGHT * 2.5);
        let presses = [None, None, Some(true), Some(false), None];
        for (frame, pressed) in presses.into_iter().enumerate() {
            let mut input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 400.0))),
                time: Some(f64::from(u8::try_from(frame).unwrap_or(0)) * 0.016),
                ..Default::default()
            };
            input.events.push(Event::PointerMoved(pointer));
            if let Some(pressed) = pressed {
                input.events.push(Event::PointerButton {
                    pos: pointer,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
            }
            let mut output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| app.tab_list(ui));
            });
            output.textures_delta.clear();
        }
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(app.active_tab, 2);
        assert_eq!(app.open_file.title(), "file-2.md");
    }
}
