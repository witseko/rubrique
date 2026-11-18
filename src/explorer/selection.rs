//! File selection in the explorer, using the order of the displayed file rows.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Response, Ui};

/// The range anchor and any explicit selection in the file tree.
#[derive(Debug, Default)]
pub struct FileSelection {
    anchor: Option<egui::Id>,
    /// None follows the open-file highlight. An empty set shows no selection.
    selected: Option<HashSet<PathBuf>>,
}

/// How a modified click changes the selection.
#[derive(Clone, Copy, Debug)]
enum SelectionClick {
    Range,
    Toggle,
}

impl SelectionClick {
    /// Shift takes precedence when both selection modifiers are held.
    fn from_modifiers(modifiers: Modifiers) -> Option<Self> {
        if modifiers.shift {
            Some(Self::Range)
        } else if modifiers.command {
            Some(Self::Toggle)
        } else {
            None
        }
    }
}

/// One keyboard step along the visible file rows.
#[derive(Clone, Copy, Debug)]
enum RangeStep {
    Up,
    Down,
}

impl RangeStep {
    /// Match a bare Shift+Up or Shift+Down press, including key repeat.
    fn from_event(event: &Event) -> Option<Self> {
        match event {
            Event::Key { key: Key::ArrowUp, pressed: true, modifiers, .. } if modifiers.shift_only() => Some(Self::Up),
            Event::Key { key: Key::ArrowDown, pressed: true, modifiers, .. } if modifiers.shift_only() => {
                Some(Self::Down)
            }
            _ => None,
        }
    }

    /// Find the next file. For a folder, index is the first file below the folder.
    fn next(self, index: usize, is_file: bool, file_count: usize) -> Option<usize> {
        match self {
            Self::Up => index.checked_sub(1),
            Self::Down => Some(index + usize::from(is_file)).filter(|&next| next < file_count),
        }
    }
}

/// Collects the current frame's file rows before changing the selection.
#[derive(Debug)]
pub(super) struct FileRows<'a> {
    selection: &'a mut FileSelection,
    open_file: Option<&'a Path>,
    selection_click: Option<SelectionClick>,
    track_visible: bool,
    visible: Vec<(egui::Id, PathBuf)>,
    pending_click: Option<(SelectionClick, usize)>,
    /// The focused row and the number of file rows preceding it.
    focused: Option<(egui::Id, usize)>,
    escape_pressed: bool,
    escape_focus: Option<egui::Id>,
}

impl<'a> FileRows<'a> {
    /// Start recording this frame's files in tree order.
    pub(super) fn new(ui: &Ui, selection: &'a mut FileSelection, open_file: Option<&'a Path>) -> Self {
        let selection_click = ui
            .input(|input| {
                input.events.iter().rev().find_map(|event| {
                    if let Event::PointerButton { button: PointerButton::Primary, pressed: false, modifiers, .. } =
                        event
                    {
                        Some(*modifiers)
                    } else {
                        None
                    }
                })
            })
            .and_then(SelectionClick::from_modifiers);
        let track_visible = selection_click.is_some()
            || selection.selected.as_ref().is_some_and(|paths| !paths.is_empty())
            || ui.input(|input| {
                input.key_pressed(Key::A) || input.events.iter().any(|event| RangeStep::from_event(event).is_some())
            });
        let escape_pressed = ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(event, Event::Key { key: Key::Escape, pressed: true, modifiers, .. } if !modifiers.any())
            })
        }) && !egui::Popup::is_any_open(ui.ctx());
        Self {
            selection,
            open_file,
            selection_click,
            track_visible,
            visible: Vec::new(),
            pending_click: None,
            focused: None,
            escape_pressed,
            escape_focus: None,
        }
    }

    /// Highlight the selection, or the open file before an explicit selection.
    pub(super) fn highlights(&self, path: &Path, open: bool) -> bool {
        self.selection.selected.as_ref().map_or(open, |paths| paths.contains(path))
    }

    /// Return the number of files a selection action for this row would affect.
    pub(super) fn target_count(&self, path: &Path) -> usize {
        self.selection.selected.as_ref().filter(|paths| paths.contains(path)).map_or(1, HashSet::len)
    }

    /// Record a row and its input. Return true when the click changes the selection.
    pub(super) fn record(&mut self, path: &Path, response: &Response) -> bool {
        self.record_focus(response);
        let selection_click = self.selection_click.filter(|_| response.clicked_by(PointerButton::Primary));
        if let Some(click) = selection_click {
            self.pending_click = Some((click, self.visible.len()));
        } else if response.clicked() {
            self.selection.anchor = Some(response.id);
            if self.selection.selected.take().is_some() {
                response.ctx.request_repaint();
            }
        }
        if self.track_visible {
            self.visible.push((response.id, path.to_path_buf()));
        }
        selection_click.is_some()
    }

    /// Track focus for file and folder rows when a tree shortcut can run.
    pub(super) fn record_focus(&mut self, response: &Response) {
        if response.clicked_by(PointerButton::Primary) {
            response.request_focus();
        }
        if self.track_visible && response.enabled() && response.has_focus() {
            self.focused = Some((response.id, self.visible.len()));
        }
        // egui drops keyboard focus before drawing the frame that receives Esc.
        if self.escape_pressed
            && response.enabled()
            && response.ctx.memory(|memory| memory.had_focus_last_frame(response.id))
        {
            self.escape_focus = Some(response.id);
        }
    }

    /// Apply selection input after all rows are known, dropping any hidden files.
    pub(super) fn update(&mut self, ui: &Ui) {
        let pending = self.pending_click.take();
        if let Some((click, index)) = pending {
            match click {
                SelectionClick::Range => self.select_range(index),
                SelectionClick::Toggle => self.toggle(index),
            }
        }
        if self.clear_from_keyboard(ui)
            || self.select_all_from_keyboard(ui)
            || self.extend_from_keyboard(ui)
            || self.prune_hidden()
            || pending.is_some()
        {
            ui.ctx().request_repaint();
        }
    }

    /// Clear selection and keep the tree focused when Esc belongs to a row.
    fn clear_from_keyboard(&mut self, ui: &Ui) -> bool {
        let Some(row) = self.escape_focus else {
            return false;
        };
        let can_clear = ui.memory(|memory| {
            memory.allows_interaction(ui.layer_id()) && memory.focused().is_none_or(|focused| focused == row)
        });
        if !can_clear || !ui.input_mut(|input| input.focused && input.consume_key(Modifiers::NONE, Key::Escape)) {
            return false;
        }
        self.selection.selected = Some(HashSet::new());
        self.selection.anchor = None;
        ui.memory_mut(|memory| memory.request_focus(row));
        true
    }

    /// Select every displayed file when the platform shortcut reaches a focused tree row.
    fn select_all_from_keyboard(&mut self, ui: &Ui) -> bool {
        let Some((focused, _)) = self.focused_row(ui) else {
            return false;
        };
        if !ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::A)) {
            return false;
        }
        self.selection.anchor = Some(focused);
        self.selection.selected = Some(self.visible.iter().map(|(_, path)| path.clone()).collect());
        true
    }

    /// Move the range endpoint and scroll just enough to keep that file visible.
    fn extend_from_keyboard(&mut self, ui: &Ui) -> bool {
        let Some((focused, index)) = self.focused_row(ui) else {
            return false;
        };
        if egui::Popup::is_any_open(ui.ctx()) {
            return false;
        }
        let mut steps = Vec::new();
        ui.input_mut(|input| {
            input.events.retain(|event| {
                let Some(step) = RangeStep::from_event(event) else {
                    return true;
                };
                steps.push(step);
                false
            });
        });
        let Some(end) = self.move_range_end(focused, index, steps) else {
            return false;
        };
        let id = self.visible[end].0;
        self.focused = Some((id, end));
        ui.memory_mut(|memory| memory.request_focus(id));
        if let Some(row) = ui.ctx().read_response(id) {
            let strip = row.rect.with_min_x(ui.clip_rect().left()).with_max_x(ui.clip_rect().left() + 1.0);
            ui.scroll_to_rect(strip, None);
        }
        true
    }

    /// Keep the anchor of the current range, or start one at the focused file.
    fn move_range_end(&mut self, focused: egui::Id, mut index: usize, steps: Vec<RangeStep>) -> Option<usize> {
        if steps.is_empty() {
            return None;
        }
        let mut is_file = self.visible.get(index).is_some_and(|(id, _)| *id == focused);
        let mut anchor = is_file.then(|| {
            self.selection
                .selected
                .as_ref()
                .filter(|paths| paths.contains(&self.visible[index].1))
                .and_then(|_| self.anchor_index())
                .map_or(focused, |anchor| self.visible[anchor].0)
        });
        let mut destination = None;
        for step in steps {
            if let Some(next) = step.next(index, is_file, self.visible.len()) {
                anchor.get_or_insert(self.visible[next].0);
                index = next;
                is_file = true;
                destination = Some(index);
            }
        }
        let end = destination?;
        self.selection.anchor = anchor;
        self.select_range(end);
        Some(end)
    }

    /// Return the focused tree row while the tree can accept input.
    fn focused_row(&self, ui: &Ui) -> Option<(egui::Id, usize)> {
        self.focused
            .filter(|&(id, _)| ui.memory(|memory| memory.has_focus(id) && memory.allows_interaction(ui.layer_id())))
    }

    /// Remove hidden files before they can be included in an action.
    fn prune_hidden(&mut self) -> bool {
        let Some(selected) = self.selection.selected.as_mut().filter(|paths| !paths.is_empty()) else {
            return false;
        };
        let visible = self.visible.iter().map(|(_, path)| path).collect::<HashSet<_>>();
        let previous_count = selected.len();
        selected.retain(|path| visible.contains(path));
        let changed = selected.len() != previous_count;
        if selected.is_empty() {
            self.selection.selected = None;
        }
        changed
    }

    /// Find the last clicked anchor in the displayed rows.
    fn anchor_index(&self) -> Option<usize> {
        let anchor = self.selection.anchor?;
        self.visible.iter().position(|(id, _)| *id == anchor)
    }

    /// Replace the selection with a range starting at the anchor, if still visible.
    fn select_range(&mut self, end_index: usize) {
        let start_index = self.anchor_index().unwrap_or_else(|| {
            self.selection.anchor = Some(self.visible[end_index].0);
            end_index
        });
        let range = start_index.min(end_index)..=start_index.max(end_index);
        self.selection.selected = Some(self.visible[range].iter().map(|(_, path)| path.clone()).collect());
    }

    /// Add or remove the clicked file, keeping a plain-click anchor if it is still highlighted.
    fn toggle(&mut self, index: usize) {
        let (id, path) = &self.visible[index];
        let mut selected = self.selection.selected.take().unwrap_or_else(|| {
            self.anchor_index()
                .map(|anchor| &self.visible[anchor].1)
                .filter(|anchor| self.open_file == Some(anchor.as_path()))
                .cloned()
                .into_iter()
                .collect()
        });
        if !selected.remove(path) {
            selected.insert(path.clone());
        }
        self.selection.anchor = Some(*id);
        self.selection.selected = Some(selected);
    }

    /// Expand an action on a selected file to all selected files in tree order.
    pub(super) fn action_targets(&self, path: &Path) -> Vec<PathBuf> {
        match &self.selection.selected {
            Some(selected) if selected.contains(path) => self.selected_paths(selected),
            _ => vec![path.to_path_buf()],
        }
    }

    /// Delete acts on the selection while a selected row or its anchor has focus.
    pub(super) fn keyboard_delete(&self, ui: &Ui) -> Option<Vec<PathBuf>> {
        let selected = self.selection.selected.as_ref()?;
        let (id, _) = self.focused_row(ui)?;
        let selection_has_focus = self.selection.anchor == Some(id)
            || self.visible.iter().any(|(row, path)| *row == id && selected.contains(path));
        if selected.is_empty() || !selection_has_focus {
            return None;
        }
        ui.input_mut(|input| {
            input.consume_key(Modifiers::NONE, Key::Delete)
                || (cfg!(target_os = "macos") && input.consume_key(Modifiers::COMMAND, Key::Backspace))
        })
        .then(|| self.selected_paths(selected))
    }

    /// Return the selected paths in the same order as their displayed rows.
    fn selected_paths(&self, selected: &HashSet<PathBuf>) -> Vec<PathBuf> {
        let mut seen = HashSet::new();
        self.visible
            .iter()
            .map(|(_, path)| path)
            .filter(|path| selected.contains(*path) && seen.insert(*path))
            .cloned()
            .collect()
    }
}
