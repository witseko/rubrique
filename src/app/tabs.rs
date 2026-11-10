//! Tabs: every file opened from the explorer gets a tab and stays open,
//! unsaved edits included, until its tab is closed.
//!
//! The active tab's file lives in `RubriqueApp::open_file`, so the editor, the
//! preview and the AI code work on it as before. The other tabs wait in
//! `RubriqueApp::tabs`; the active tab's own slot there holds `OpenFile::Nothing`.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, CornerRadius, Sense, Ui, pos2, vec2};

use super::tab_groups::ChipClick;
use super::{GuardedAction, OpenFile, RubriqueApp, side_panel};
use crate::document::preview_selection;
use crate::document::view::{focus_editor, restore_editor_state, take_editor_state};
use crate::explorer::paths::absolute_path_text;
use crate::explorer::view::{InlineRename, rename_field};
use crate::icons::{Icon, LOCK_MARK_SIZE, paint_icon, paint_lock_mark, small_icon_button};
use crate::markdown::html::is_html_path;
use crate::media::image_uri;
use crate::theme::style::page_fill;

/// Height of the tab strip.
pub(super) const TAB_HEIGHT: f32 = 30.0;
/// Space left and right of a tab's name.
const TAB_PADDING: f32 = 12.0;
/// Room for the close cross (or the unsaved dot) after the name.
const CLOSE_SIZE: f32 = 18.0;
/// Width of the name field while a tab is renamed.
const RENAME_FIELD_WIDTH: f32 = 180.0;
/// Widest a tab gets; longer names are cut with "…".
const TAB_MAX_WIDTH: f32 = 220.0;
/// Height of the accent line on top of the active tab.
const ACTIVE_LINE: f32 = 2.0;
/// How strongly an inactive tab shows its group color on top.
const GROUP_LINE_TINT: f32 = 0.55;
/// Radius of the unsaved-changes dot.
const DIRTY_DOT: f32 = 3.5;
/// Space between an encrypted tab's name and its padlock.
const LOCK_GAP: f32 = 6.0;
/// Width of each half of the Preview | Source switch: room for one icon.
const SWITCH_SEGMENT: f32 = 28.0;
const SWITCH_HEIGHT: f32 = 22.0;
/// Space inside the switch around its two halves.
const SWITCH_INSET: f32 = 2.0;
/// Corner radius of the switch's frame; a half is rounded a little less,
/// so the two curves stay parallel.
const SWITCH_RADIUS: u8 = 6;
const SWITCH_HALF_RADIUS: u8 = 4;
/// Space between the switch and the right edge of the tab strip.
const SWITCH_MARGIN: f32 = 10.0;

/// Something the user did on the tab strip.
#[derive(Clone, Copy, Debug)]
enum TabAction {
    /// The tab is being dragged (the strip shows where it would land).
    Dragging(usize),
    Switch(usize),
    Close(usize),
    CloseOthers(usize),
    CloseToTheRight(usize),
    CopyPath(usize),
    /// Edit the file's name right in its tab.
    Rename(usize),
    /// The tab was dragged and dropped with the pointer at screen `x`.
    Drop {
        index: usize,
        x: f32,
    },
    /// End the tmux session this tab shows, and close the tab.
    ExitTerminal(usize),
}

impl RubriqueApp {
    /// The file in tab `index`.
    pub(super) fn tab(&self, index: usize) -> &OpenFile {
        if index == self.active_tab { &self.open_file } else { &self.tabs[index] }
    }

    /// The tab showing `path`, if it is open.
    fn find_tab(&self, path: &Path) -> Option<usize> {
        (0..self.tabs.len()).find(|&index| self.tab(index).path() == Some(path))
    }

    /// Every open file, the active one first (the active tab's empty slot
    /// in `tabs` is included, which is harmless for lookups and renames).
    pub(super) fn all_tabs_mut(&mut self) -> impl Iterator<Item = &mut OpenFile> {
        std::iter::once(&mut self.open_file).chain(self.tabs.iter_mut())
    }

    /// Paths of the open tabs, left to right (for the settings file).
    pub(super) fn tab_paths(&self) -> Vec<PathBuf> {
        (0..self.tabs.len()).filter_map(|index| self.tab(index).path().map(Path::to_path_buf)).collect()
    }

    /// The first tab with unsaved changes whose path passes `filter`,
    /// trying the active tab first.
    pub(super) fn dirty_tab(&self, filter: impl Fn(&Path) -> bool) -> Option<usize> {
        let dirty = |index: usize| {
            let tab = self.tab(index);
            tab.is_dirty() && tab.path().is_some_and(&filter)
        };
        if !self.tabs.is_empty() && dirty(self.active_tab) {
            return Some(self.active_tab);
        }
        (0..self.tabs.len()).find(|&index| dirty(index))
    }

    /// Show `path`: switch to its tab when it is open, otherwise open it in
    /// a new tab right after the active one, with the unsaved edits kept
    /// when its tab was last closed, if any.
    pub(super) fn open_in_tab(&mut self, path: PathBuf) {
        if let Some(index) = self.find_tab(&path) {
            self.switch_to_tab(index);
            return;
        }
        if self.reopen_kept_edits(&path) {
            return;
        }
        if let Some(file) = self.load_file(path) {
            self.add_tab(file);
        }
    }

    /// Show `file` in a new tab: right after the active one, or at the end
    /// of its group when tabs are grouped by folder.
    pub(super) fn add_tab(&mut self, file: OpenFile) {
        if self.tabs.is_empty() {
            self.tabs.push(OpenFile::Nothing);
            self.active_tab = 0;
            self.open_file = file;
            self.enter_tab();
        } else {
            let index = self.new_tab_index(&file);
            self.tabs.insert(index, file);
            // A group can start before the active tab: its slot moved right.
            if index <= self.active_tab {
                self.active_tab += 1;
            }
            self.switch_to_tab(index);
        }
    }

    /// Move tab `index` to place `to` (inside its group when tabs are
    /// grouped); the active tab stays active.
    fn move_tab(&mut self, index: usize, to: usize) {
        if index == to || index >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        if self.tabs_grouped() && self.group_of(self.tab(index)) != self.group_of(self.tab(to)) {
            return;
        }
        let tab = self.tabs.remove(index);
        self.tabs.insert(to, tab);
        self.active_tab = if self.active_tab == index {
            to
        } else if index < self.active_tab && to >= self.active_tab {
            self.active_tab - 1
        } else if index > self.active_tab && to <= self.active_tab {
            self.active_tab + 1
        } else {
            self.active_tab
        };
    }

    /// Make tab `index` the one on screen.
    pub(super) fn switch_to_tab(&mut self, index: usize) {
        if index >= self.tabs.len() || index == self.active_tab {
            return;
        }
        self.leave_tab();
        std::mem::swap(&mut self.open_file, &mut self.tabs[self.active_tab]);
        std::mem::swap(&mut self.open_file, &mut self.tabs[index]);
        self.active_tab = index;
        self.enter_tab();
    }

    /// The tab after (or, with `forward` false, before) the active one, wrapping around.
    fn cycle_tab(&mut self, forward: bool) {
        let count = self.tabs.len();
        if count > 1 {
            let next = if forward { (self.active_tab + 1) % count } else { (self.active_tab + count - 1) % count };
            self.switch_to_tab(next);
        }
    }

    /// Put away what belongs to the tab on screen before another one shows:
    /// a block being edited is applied, the editor's undo history is kept
    /// with the tab, and screens tied to this file (merge, note, Ask bar) close.
    fn leave_tab(&mut self) {
        self.finish_block_edit();
        self.tab_rename = None;
        self.merging = None;
        self.note_draft = None;
        self.stats = None;
        self.ask.close_bar();
        preview_selection::clear(&self.repaint);
        let state = take_editor_state(&self.repaint);
        match &mut self.open_file {
            OpenFile::Text { editor, .. } => *editor = state,
            // A hidden terminal stops reading its screen; the session runs on.
            OpenFile::Terminal { terminal } => terminal.pause(),
            _ => {}
        }
    }

    /// Set up the tab that just came on screen, and look for changes made
    /// to its file while it was hidden.
    pub(super) fn enter_tab(&mut self) {
        let state = match &mut self.open_file {
            OpenFile::Text { editor, .. } => editor.take(),
            _ => None,
        };
        restore_editor_state(&self.repaint, state);
        self.settings.last_file = self.open_file.path().map(Path::to_path_buf);
        self.check_disk();
    }

    /// Close the active tab without asking; the next one to the right (or
    /// the last one) takes its place.
    pub(super) fn close_active_tab_now(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.leave_tab();
        let closed = std::mem::replace(&mut self.open_file, OpenFile::Nothing);
        self.forget_media(&closed);
        self.tabs.remove(self.active_tab);
        if self.tabs.is_empty() {
            self.active_tab = 0;
            self.settings.last_file = None;
            return;
        }
        self.active_tab = self.active_tab.min(self.tabs.len() - 1);
        self.open_file = std::mem::replace(&mut self.tabs[self.active_tab], OpenFile::Nothing);
        self.enter_tab();
    }

    /// Close tab `index`, asking first when it has unsaved changes.
    fn request_close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if index != self.active_tab && !self.tab(index).is_dirty() {
            self.close_hidden_tab(index);
            return;
        }
        self.switch_to_tab(index);
        self.request(GuardedAction::CloseTab);
    }

    /// Close a tab that is not on screen (and has nothing unsaved).
    pub(super) fn close_hidden_tab(&mut self, index: usize) {
        let closed = self.tabs.remove(index);
        self.forget_media(&closed);
        if index < self.active_tab {
            self.active_tab -= 1;
        }
    }

    /// Close every tab whose file is inside `folder`, without asking.
    pub(super) fn close_tabs_inside(&mut self, folder: &Path) {
        for index in (0..self.tabs.len()).rev() {
            if index != self.active_tab && self.tabs[index].path().is_some_and(|path| path.starts_with(folder)) {
                self.close_hidden_tab(index);
            }
        }
        if self.open_file.path().is_some_and(|path| path.starts_with(folder)) {
            self.close_active_tab_now();
        }
    }

    /// Close the tabs in `indexes` that have no unsaved changes; say how
    /// many were kept open because they do.
    fn close_clean_tabs(&mut self, indexes: Vec<usize>) {
        let mut kept = 0;
        for index in indexes.into_iter().rev() {
            if self.tab(index).is_dirty() {
                kept += 1;
            } else if index == self.active_tab {
                self.close_active_tab_now();
            } else {
                self.close_hidden_tab(index);
            }
        }
        if kept > 0 {
            let noun = if kept == 1 { "tab has" } else { "tabs have" };
            self.show_status(format!("{kept} {noun} unsaved changes and stayed open"));
        }
    }

    /// Free an image's decoded pixels when its tab closes.
    fn forget_media(&self, file: &OpenFile) {
        if let OpenFile::Media { path, .. } = file {
            self.repaint.forget_image(&image_uri(path));
        }
    }

    /// Tab shortcuts: Cmd+W closes, Ctrl+Tab and Ctrl+Shift+Tab move between tabs.
    pub(super) fn handle_tab_shortcuts(&mut self, ctx: &egui::Context) {
        // Shift first: egui lets Ctrl+Shift+Tab also match Ctrl+Tab.
        if ctx.input_mut(|input| input.consume_shortcut(&super::PREVIOUS_TAB)) {
            self.cycle_tab(false);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&super::NEXT_TAB)) {
            self.cycle_tab(true);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&super::CLOSE_TAB)) && !self.tabs.is_empty() {
            self.request_close_tab(self.active_tab);
        }
    }

    /// The Preview | Source switch of a Markdown or HTML file at the right
    /// end of the tab strip, and for Markdown the side panel toggle after
    /// it. Returns the part of `strip` left for the tabs.
    fn preview_controls(&mut self, ui: &mut Ui, strip: egui::Rect) -> egui::Rect {
        let has_toggle = self.can_show_side_panel();
        let has_switch = has_toggle || self.open_file.path().is_some_and(is_html_path);
        let switch_width = SWITCH_SEGMENT * 2.0 + SWITCH_INSET * 2.0;
        let toggle_width = if has_toggle { side_panel::TOGGLE_WIDTH + SWITCH_MARGIN } else { 0.0 };
        let controls_width = switch_width + SWITCH_MARGIN * 2.0 + toggle_width;
        let tabs_rect = if has_switch { strip.with_max_x(strip.right() - controls_width) } else { strip };
        if has_toggle {
            let toggle = egui::Rect::from_center_size(
                pos2(strip.right() - SWITCH_MARGIN - side_panel::TOGGLE_WIDTH / 2.0, strip.center().y),
                vec2(side_panel::TOGGLE_WIDTH, TAB_HEIGHT),
            );
            self.side_panel_toggle(ui, toggle);
        }
        if has_switch {
            let switch = egui::Rect::from_center_size(
                pos2(strip.right() - toggle_width - SWITCH_MARGIN - switch_width / 2.0, strip.center().y),
                vec2(switch_width, SWITCH_HEIGHT),
            );
            let showing = self.showing_any_preview();
            if let Some(preview) = preview_switch(ui, switch, showing)
                && preview != showing
            {
                self.toggle_preview();
                if !self.showing_any_preview() {
                    focus_editor(ui.ctx());
                }
            }
        }
        tabs_rect
    }

    /// The row of tabs above the page.
    pub(super) fn tab_bar(&mut self, ui: &mut Ui) {
        let mut action = None;
        let mut chip_click = None;
        // Where each drawn tab (and each group, chip included) is, to find
        // what a dragged tab or chip was dropped on.
        let mut tab_spans: Vec<(usize, f32, f32)> = Vec::new();
        let mut group_spans: Vec<(Option<usize>, f32, f32)> = Vec::new();
        let strip_fill = ui.visuals().panel_fill;
        let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), TAB_HEIGHT), Sense::hover());
        ui.painter().rect_filled(strip, 0, strip_fill);
        let line = ui.visuals().widgets.noninteractive.bg_stroke;
        ui.painter().hline(strip.x_range(), strip.bottom() - line.width / 2.0, line);
        let tabs_rect = self.preview_controls(ui, strip);
        ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_rect), |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("tab-strip")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let grouped = self.tabs_grouped();
                        let mut current_group = None;
                        for index in 0..self.tabs.len() {
                            let file = self.tab(index);
                            let group = self.group_of(file);
                            let folded = grouped && self.group_folded(group);
                            // Each group starts with its chip.
                            if grouped && (index == 0 || current_group != Some(group)) {
                                current_group = Some(group);
                                group_spans.push((group, ui.cursor().left(), ui.cursor().left()));
                                if let Some(click) = self.group_chip(ui, group, folded) {
                                    chip_click = Some(click);
                                }
                            }
                            if folded && index != self.active_tab {
                                continue;
                            }
                            let file = self.tab(index);
                            let tab = TabView {
                                file: file.path().map(Path::to_path_buf),
                                name: file.title(),
                                path: file.path().map(absolute_path_text).unwrap_or_default(),
                                dirty: file.is_dirty(),
                                active: index == self.active_tab,
                                group_color: self.group_color_of(group),
                                terminal: match file {
                                    OpenFile::Terminal { terminal } => Some(terminal.name.clone()),
                                    _ => None,
                                },
                                encrypted: file.is_encrypted(),
                            };
                            let left = ui.cursor().left();
                            if let Some(found) = show_tab(ui, &tab, index, self.tabs.len(), &mut self.tab_rename) {
                                action = Some(found);
                            }
                            tab_spans.push((index, left, ui.cursor().left()));
                            if let Some(span) = group_spans.last_mut() {
                                span.2 = ui.cursor().left();
                            }
                        }
                    });
                });
        });
        self.paint_insertion_marker(ui, strip, &tab_spans, &group_spans, (&action, &chip_click));
        match chip_click {
            Some(ChipClick::Toggle(group)) => self.toggle_group(group),
            Some(ChipClick::Dragging(_)) | None => {}
            Some(ChipClick::Drop { group, x }) => {
                // Dropped on a folder's group: take its place.
                if let Some((Some(to), ..)) = group_spans.iter().find(|(_, left, right)| (*left..=*right).contains(&x))
                {
                    self.move_group(group, *to);
                }
            }
        }
        if let Some(TabAction::Drop { index, x }) = action {
            if let Some(to) = drop_target(&tab_spans, x) {
                self.move_tab(index, to);
            }
        } else if let Some(action) = action {
            self.apply_tab_action(ui.ctx(), action);
        }
    }

    /// While a tab or a chip is dragged: a line in the strip where it would land.
    fn paint_insertion_marker(
        &self,
        ui: &Ui,
        strip: egui::Rect,
        tab_spans: &[(usize, f32, f32)],
        group_spans: &[(Option<usize>, f32, f32)],
        (action, chip_click): (&Option<TabAction>, &Option<ChipClick>),
    ) {
        let Some(x) = ui.input(|input| input.pointer.latest_pos()).map(|pos| pos.x) else { return };
        // The edge of the span the item would be put next to: past it when
        // it moves right, before it when it moves left.
        let edge = |from: usize, to: usize, left: f32, right: f32| {
            (from != to).then_some(if to > from { right } else { left })
        };
        let place = match (action, chip_click) {
            (Some(TabAction::Dragging(from)), _) => drop_target(tab_spans, x)
                .filter(|&to| !self.tabs_grouped() || self.group_of(self.tab(*from)) == self.group_of(self.tab(to)))
                .and_then(|to| {
                    let (_, left, right) = tab_spans.iter().find(|span| span.0 == to)?;
                    edge(*from, to, *left, *right)
                }),
            (_, Some(ChipClick::Dragging(from))) => group_spans
                .iter()
                .find(|(_, left, right)| (*left..=*right).contains(&x))
                .and_then(|(to, left, right)| edge(*from, (*to)?, *left, *right)),
            _ => None,
        };
        if let Some(place) = place {
            let stroke = egui::Stroke::new(2.0, ui.visuals().hyperlink_color);
            ui.painter().vline(place, strip.y_range().shrink(5.0), stroke);
        }
    }

    /// Do what the user asked for on the tab strip.
    fn apply_tab_action(&mut self, ctx: &egui::Context, action: TabAction) {
        match action {
            TabAction::Switch(index) => self.switch_to_tab(index),
            TabAction::Close(index) => self.request_close_tab(index),
            TabAction::CloseOthers(index) => {
                self.switch_to_tab(index);
                self.close_clean_tabs((0..self.tabs.len()).filter(|&other| other != index).collect());
            }
            TabAction::CloseToTheRight(index) => {
                self.close_clean_tabs((index + 1..self.tabs.len()).collect());
            }
            TabAction::Rename(index) => {
                if let Some(path) = self.tab(index).path() {
                    self.tab_rename = Some(InlineRename::new(path.to_path_buf()));
                }
            }
            TabAction::Drop { .. } | TabAction::Dragging(_) => {}
            TabAction::ExitTerminal(index) => {
                if let OpenFile::Terminal { terminal } = self.tab(index) {
                    let name = terminal.name.clone();
                    self.exit_terminal(&name);
                }
            }
            TabAction::CopyPath(index) => {
                if let Some(path) = self.tab(index).path() {
                    let text = absolute_path_text(path);
                    self.copy_to_clipboard(ctx, text);
                }
            }
        }
    }
}

/// The tab a drop at screen `x` lands on: the one under it, else the first
/// or last tab when dropped past either end.
fn drop_target(spans: &[(usize, f32, f32)], x: f32) -> Option<usize> {
    let (first, last) = (spans.first()?, spans.last()?);
    if x < first.1 {
        return Some(first.0);
    }
    spans.iter().find(|(_, left, right)| (*left..=*right).contains(&x)).map_or(Some(last.0), |span| Some(span.0))
}

/// The Preview | Source switch: two icon halves (the eye, the code
/// brackets), the current mode filled like the page. The names and the
/// shortcut are in the hover tips. Returns the mode that was clicked.
fn preview_switch(ui: &mut Ui, rect: egui::Rect, preview: bool) -> Option<bool> {
    let visuals = ui.visuals().clone();
    let frame_fill = visuals.widgets.inactive.weak_bg_fill;
    ui.painter().rect_filled(rect, SWITCH_RADIUS, frame_fill);
    let mut clicked = None;
    let halves = [(true, "Preview", Icon::Eye, 0.0), (false, "Source", Icon::Code, SWITCH_SEGMENT)];
    for (mode, label, icon, offset) in halves {
        let left = rect.left() + SWITCH_INSET + offset;
        let half = egui::Rect::from_min_size(
            pos2(left, rect.top() + SWITCH_INSET),
            vec2(SWITCH_SEGMENT, rect.height() - SWITCH_INSET * 2.0),
        );
        let response = ui.interact(half, ui.scope_id().with(("preview-switch", label)), Sense::click());
        let selected = mode == preview;
        let fill = if selected {
            page_fill(&visuals)
        } else if response.hovered() {
            visuals.widgets.hovered.weak_bg_fill
        } else {
            frame_fill
        };
        ui.painter().rect_filled(half, SWITCH_HALF_RADIUS, fill);
        let color = if selected { visuals.strong_text_color() } else { visuals.weak_text_color() };
        let accent = if selected { visuals.hyperlink_color } else { color };
        paint_icon(ui.painter(), icon, half.center(), color, accent, fill);
        let tip = if mode { "Show the preview (Cmd+E)" } else { "Show the source (Cmd+E)" };
        let response = response.on_hover_text(tip);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
        if response.clicked() {
            clicked = Some(mode);
        }
    }
    clicked
}

/// How visible the copy that follows the pointer is, and how much of the
/// dragged tab or chip is left in its place.
pub(super) const GHOST_OPACITY: f32 = 0.5;

/// While `response` is being dragged: a painter on top of everything and
/// how far the pointer has moved sideways since the press. The caller draws
/// a half visible copy of what is dragged with it, so the item seems to
/// follow the pointer. The strip never moves up or down, and the real item
/// stays in its place (dimmed, see `dim_dragged`) until the drop.
pub(super) fn drag_ghost(ui: &Ui, response: &egui::Response) -> Option<(egui::Painter, egui::Vec2)> {
    if !response.dragged() {
        return None;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    let (now, start) = ui.input(|input| (input.pointer.latest_pos(), input.pointer.press_origin()));
    let offset = vec2(now?.x - start?.x, 0.0);
    let layer = egui::LayerId::new(egui::Order::Tooltip, response.id.with("drag-ghost"));
    Some((ui.ctx().layer_painter(layer), offset))
}

/// Fade the dragged item in its place, so the copy under the pointer is
/// the one that reads as held.
pub(super) fn dim_dragged(ui: &Ui, rect: egui::Rect) {
    ui.painter().rect_filled(rect, 0, ui.visuals().panel_fill.gamma_multiply(1.0 - GHOST_OPACITY));
}

/// While the tab is dragged: dim it in place and draw its copy at the pointer.
fn paint_dragged_tab(
    ui: &Ui,
    response: &egui::Response,
    fill: Color32,
    line: Option<Color32>,
    name: (egui::Pos2, std::sync::Arc<egui::Galley>, Color32),
) {
    let Some((ghost, offset)) = drag_ghost(ui, response) else { return };
    dim_dragged(ui, response.rect);
    // A tab with no fill (not hovered, not active) still needs one to be seen.
    let fill = if fill == Color32::TRANSPARENT { ui.visuals().widgets.hovered.weak_bg_fill } else { fill };
    let (position, galley, color) = name;
    paint_tab_ghost(&ghost, response.rect.translate(offset), fill, line, (position + offset, galley, color));
}

/// The half visible copy of a dragged tab at `rect`: its fill, the line on
/// top, and its name (position, text, color).
fn paint_tab_ghost(
    painter: &egui::Painter,
    rect: egui::Rect,
    fill: Color32,
    line: Option<Color32>,
    name: (egui::Pos2, std::sync::Arc<egui::Galley>, Color32),
) {
    let corners = CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 };
    let half = |color: Color32| color.gamma_multiply(GHOST_OPACITY);
    painter.rect_filled(rect, corners, half(fill));
    if let Some(color) = line {
        let top = egui::Rect::from_min_size(rect.min, vec2(rect.width(), ACTIVE_LINE));
        painter.rect_filled(top, corners, half(color));
    }
    let (position, galley, color) = name;
    painter.galley(position, galley, half(color));
}

/// What one tab shows.
struct TabView {
    /// The tab's file; `None` for a terminal opened from the Terminals list.
    file: Option<PathBuf>,
    name: String,
    /// Full path, for the hover tip.
    path: String,
    dirty: bool,
    active: bool,
    /// The color of the tab's folder, when tabs are grouped.
    group_color: Option<Color32>,
    /// The tmux session the tab shows, if it is a terminal.
    terminal: Option<String>,
    /// The file is stored encrypted: a padlock after the name.
    encrypted: bool,
}

/// One tab: the file name, then a close cross (a dot instead while the file
/// has unsaved changes and the pointer is elsewhere). A click switches, a
/// middle click or the cross closes, a right click opens a menu.
fn show_tab(
    ui: &mut Ui,
    tab: &TabView,
    index: usize,
    count: usize,
    rename: &mut Option<InlineRename>,
) -> Option<TabAction> {
    if tab.file.is_some() && rename.as_ref().map(|rename| &rename.target) == tab.file.as_ref() {
        renaming_tab(ui, tab, rename);
        return None;
    }
    let visuals = ui.visuals().clone();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_color = if tab.active { visuals.strong_text_color() } else { visuals.weak_text_color() };
    let lock_width = if tab.encrypted { LOCK_GAP + LOCK_MARK_SIZE } else { 0.0 };
    let max_text = TAB_MAX_WIDTH - TAB_PADDING * 2.0 - lock_width - CLOSE_SIZE;
    let mut job = egui::text::LayoutJob::simple_singleline(tab.name.clone(), font, text_color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_text);
    let galley = ui.painter().layout_job(job);
    let text_width = galley.size().x;
    let width = TAB_PADDING * 2.0 + text_width + lock_width + CLOSE_SIZE;
    // Own id: a hovered tab's cross takes an automatic id, renumbering the rest.
    let (rect, _) = ui.allocate_exact_size(vec2(width, TAB_HEIGHT), Sense::hover());
    let response = ui.interact(rect, ui.scope_id().with(("tab", index)), Sense::click_and_drag());
    // Pointer anywhere on the tab, the close cross included. `hovered()` turns
    // false over the cross (a widget on top), which swapped the cross back to
    // the unsaved dot and lost the click.
    let hovered = response.contains_pointer();

    let fill = if tab.active {
        page_fill(&visuals)
    } else if hovered {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, fill);
    // The line on top: the accent (or the group color) on the active tab,
    // the group color, softer, on the others.
    let line = match (tab.active, tab.group_color) {
        (true, Some(color)) => Some(color),
        (true, None) => Some(visuals.hyperlink_color),
        (false, Some(color)) => Some(color.gamma_multiply(GROUP_LINE_TINT)),
        (false, None) => None,
    };
    if let Some(color) = line {
        let top = egui::Rect::from_min_size(rect.min, vec2(rect.width(), ACTIVE_LINE));
        painter.rect_filled(top, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, color);
    }
    // Thin line between tabs that are not active.
    if !tab.active {
        let x = rect.right() - 0.5;
        painter.vline(x, rect.y_range().shrink(8.0), visuals.widgets.noninteractive.bg_stroke);
    }
    let text_pos = pos2(rect.left() + TAB_PADDING, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_pos, galley.clone(), text_color);
    if tab.encrypted {
        let center = pos2(text_pos.x + text_width + LOCK_GAP + LOCK_MARK_SIZE / 2.0, rect.center().y);
        paint_lock_mark(painter, center, text_color, visuals.hyperlink_color);
    }
    paint_dragged_tab(ui, &response, fill, line, (text_pos, galley, text_color));
    let close_rect = egui::Rect::from_center_size(
        pos2(rect.right() - TAB_PADDING / 2.0 - CLOSE_SIZE / 2.0, rect.center().y),
        vec2(CLOSE_SIZE, CLOSE_SIZE),
    );
    let mut action = None;
    if tab.dirty && !hovered {
        ui.painter().circle_filled(close_rect.center(), DIRTY_DOT, visuals.text_color());
    } else if hovered || tab.active {
        // A child ui at the cross's rect, not `ui.put`: `put` moves the row's
        // cursor back to the cross's right edge, which shifted the tabs after
        // this one 6 points left whenever the cross appeared.
        let layout = egui::Layout::centered_and_justified(egui::Direction::TopDown);
        let mut cross =
            ui.new_child(egui::UiBuilder::new().id_salt(("close", index)).max_rect(close_rect).layout(layout));
        let close = small_icon_button(&mut cross, Icon::Close, CLOSE_SIZE, "Close tab", "Close (Cmd+W)");
        if close.clicked() {
            action = Some(TabAction::Close(index));
        }
    }

    let mut tip = tab.path.clone();
    if tab.encrypted {
        tip.push_str("\nEncrypted");
    }
    if tab.dirty {
        tip.push_str("\nUnsaved changes");
    }
    let response = response.on_hover_text(tip);
    if response.double_clicked() && tab.file.is_some() {
        action = Some(TabAction::Rename(index));
    } else if action.is_none() && response.clicked() {
        action = Some(TabAction::Switch(index));
    }
    // Drag a tab sideways and drop it where it should go.
    if response.drag_stopped()
        && let Some(pos) = response.interact_pointer_pos()
    {
        action = Some(TabAction::Drop { index, x: pos.x });
    } else if response.dragged() {
        action = Some(TabAction::Dragging(index));
    }
    if response.middle_clicked() {
        action = Some(TabAction::Close(index));
    }
    tab_menu(&response, tab, index, count, &mut action);
    action
}

/// The tab's right-click menu: the close actions, then Exit terminal (for
/// a terminal), Rename (for a file) and Copy path.
fn tab_menu(response: &egui::Response, tab: &TabView, index: usize, count: usize, action: &mut Option<TabAction>) {
    response.context_menu(|ui| {
        if ui.button("Close").clicked() {
            *action = Some(TabAction::Close(index));
        }
        if ui.add_enabled(count > 1, egui::Button::new("Close others")).clicked() {
            *action = Some(TabAction::CloseOthers(index));
        }
        if ui.add_enabled(index + 1 < count, egui::Button::new("Close to the right")).clicked() {
            *action = Some(TabAction::CloseToTheRight(index));
        }
        ui.separator();
        if tab.terminal.is_some() && ui.button("Exit terminal").clicked() {
            *action = Some(TabAction::ExitTerminal(index));
        }
        if ui
            .add_enabled(tab.file.is_some(), egui::Button::new("Rename"))
            .on_hover_text("Or double-click the tab")
            .clicked()
        {
            *action = Some(TabAction::Rename(index));
        }
        if ui.button("Copy path").clicked() {
            *action = Some(TabAction::CopyPath(index));
        }
    });
}

/// A tab whose file is being renamed: the name field in place of its label.
fn renaming_tab(ui: &mut Ui, tab: &TabView, rename: &mut Option<InlineRename>) {
    let visuals = ui.visuals().clone();
    let width = TAB_PADDING * 2.0 + RENAME_FIELD_WIDTH + CLOSE_SIZE;
    let (rect, _) = ui.allocate_exact_size(vec2(width, TAB_HEIGHT), Sense::hover());
    let fill = if tab.active { page_fill(&visuals) } else { visuals.widgets.hovered.weak_bg_fill };
    ui.painter().rect_filled(rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, fill);
    let field_rect = rect.shrink2(vec2(TAB_PADDING, 0.0)).with_max_x(rect.right() - TAB_PADDING - CLOSE_SIZE);
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    let mut field = ui.new_child(egui::UiBuilder::new().max_rect(field_rect).layout(layout));
    rename_field(&mut field, rename, egui::Id::unique("tab-rename-field"), RENAME_FIELD_WIDTH);
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use eframe::egui::{Context, Event, Pos2, RawInput, Rect, Vec2};

    /// Lay out three tabs (the middle one active) and return where each one
    /// starts and the top of the row, after a few frames so hover settles.
    fn tab_positions(pointer: Option<Pos2>) -> Vec<(f32, f32)> {
        let ctx = Context::default();
        let mut positions = Vec::new();
        for _ in 0..3 {
            let mut input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                ..Default::default()
            };
            if let Some(pos) = pointer {
                input.events.push(Event::PointerMoved(pos));
            }
            positions.clear();
            let mut output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for index in 0..3 {
                            let tab = TabView {
                                file: None,
                                name: format!("file-{index}.md"),
                                path: String::new(),
                                dirty: index == 2,
                                active: index == 1,
                                group_color: None,
                                terminal: None,
                                encrypted: false,
                            };
                            let left = ui.cursor().left();
                            show_tab(ui, &tab, index, 3, &mut None);
                            positions.push((left, ui.min_rect().top()));
                        }
                    });
                });
            });
            // No screen here: the font texture is never uploaded.
            output.textures_delta.clear();
        }
        positions
    }

    #[test]
    fn hovering_a_tab_moves_no_tab() {
        let still = tab_positions(None);
        // Over the first tab, then over the dirty third one.
        let first = tab_positions(Some(pos2(20.0, 20.0)));
        let third = tab_positions(Some(pos2(still[2].0 + 20.0, 20.0)));
        assert_eq!(still, first);
        assert_eq!(still, third);
    }

    /// Open `file-0.md` to `file-3.md` in a temp folder (the last one ends up
    /// active) and drag tab `from` onto tab `to` over many small steps, like
    /// a hand. Returns the tab names before and after.
    fn drag_in_the_tab_bar(from: u8, to: u8) -> (Vec<String>, Vec<String>) {
        use eframe::egui::PointerButton;
        let dir = std::env::temp_dir().join(format!("rubrique-drag-{}-{from}-{to}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = Context::default();
        let mut app = RubriqueApp::new(&ctx, crate::settings::Settings::default(), vec![dir.clone()], None);
        // `new` reopens the drafts in this machine's config folder as tabs
        // (none on CI, some on a developer's Mac). Only the four below count.
        app.tabs.clear();
        app.active_tab = 0;
        app.open_file = OpenFile::Nothing;
        for index in 0..4 {
            let path = dir.join(format!("file-{index}.md"));
            std::fs::write(&path, "x").unwrap();
            app.open_in_tab(path);
        }
        let names = |app: &RubriqueApp| (0..app.tabs.len()).map(|i| app.tab(i).title()).collect::<Vec<_>>();
        let before = names(&app);
        // Where the tabs are: the same names laid out by `tab_positions`, so
        // the first tab's left edge and the width of one tab are known.
        let positions = tab_positions(None);
        let (left, width) = (positions[0].0, positions[1].0 - positions[0].0);
        let center = |index: u8| left + width * (f32::from(index) + 0.5);
        let (from, to) = (center(from), center(to));
        // Settle on the tab, press, move in 40 steps, release.
        let mut steps: Vec<(f32, Option<bool>)> = vec![(from, None), (from, None), (from, Some(true))];
        steps.extend((1..=40_u8).map(|n| (from + (to - from) * f32::from(n) / 40.0, None)));
        steps.push((to, Some(false)));
        for (n, (x, button)) in steps.into_iter().enumerate() {
            let mut input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                time: Some(f64::from(u16::try_from(n).unwrap_or(0)) * 0.016),
                ..Default::default()
            };
            input.events.push(Event::PointerMoved(pos2(x, 20.0)));
            if let Some(pressed) = button {
                input.events.push(Event::PointerButton {
                    pos: pos2(x, 20.0),
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
            }
            let mut output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| app.tab_bar(ui));
            });
            output.textures_delta.clear();
        }
        std::fs::remove_dir_all(&dir).ok();
        (before, names(&app))
    }

    #[test]
    fn dragging_a_tab_right_moves_it() {
        let (before, after) = drag_in_the_tab_bar(0, 2);
        assert_eq!(after, [before[1].clone(), before[2].clone(), before[0].clone(), before[3].clone()]);
    }

    #[test]
    fn dragging_a_tab_left_moves_it() {
        // From the last tab (the active one) to the first.
        let (before, after) = drag_in_the_tab_bar(3, 0);
        assert_eq!(after, [before[3].clone(), before[0].clone(), before[1].clone(), before[2].clone()]);
    }
}
