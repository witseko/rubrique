//! Unsaved changes of a Markdown file, block by block, until Save: the
//! list behind the marks in the preview, the "Unsaved: …" button in the
//! status bar, the Changes view of the side panel, and putting a block
//! back the way it was saved.

use std::ops::Range;

use eframe::egui::{self, RichText, Ui};

use super::{OpenFile, RubriqueApp, appended_block, deleted_block_range, inserted_block, replace_with_undo};
use crate::document::block_changes::{BlockChange, ChangeKind, compare_blocks};
use crate::document::changes_view::{change_details, summary};
use crate::document::view::scroll_preview_to_block;
use crate::explorer::tree::is_markdown_path;
use crate::markdown::blocks::{SourceBlock, parse_source_blocks};
use crate::settings::layout::SideView;

/// Opacity of the accent behind a change card in the panel.
const CARD_TINT: f32 = 0.06;
/// Characters of a block's first line in a panel card's title.
const CARD_TITLE_CHARS: usize = 48;

/// The open file's block changes, worked out again only when the text or
/// the saved text changed.
#[derive(Debug, Default)]
pub(super) struct ChangesCache {
    /// The document revision and saved text length the list is for.
    key: Option<(u64, usize)>,
    /// The changes, in block order.
    pub(super) list: Vec<BlockChange>,
}

impl RubriqueApp {
    /// Bring the change list up to date with the open file. Only Markdown
    /// files with unsaved changes have any.
    pub(super) fn refresh_block_changes(&mut self) {
        let OpenFile::Text { document, preview, .. } = &mut self.open_file else {
            self.block_changes = ChangesCache::default();
            return;
        };
        if !document.is_dirty() || !is_markdown_path(&document.path) {
            self.block_changes = ChangesCache::default();
            return;
        }
        let key = (document.revision(), document.base_text().len());
        if self.block_changes.key == Some(key) {
            return;
        }
        let blocks = preview.get_or_insert_with(|| parse_source_blocks(&document.text));
        self.block_changes =
            ChangesCache { key: Some(key), list: compare_blocks(document.base_text(), &document.text, blocks) };
    }

    /// Open the side panel on the Changes view.
    fn show_changes_panel(&mut self) {
        self.settings.layout.side_panel_open = true;
        self.settings.layout.side_view = SideView::Changes;
    }

    /// "Unsaved: 2 edited, 1 deleted" in the status bar while a Markdown
    /// file has block changes; a click opens the Changes view.
    pub(super) fn changes_status_button(&mut self, ui: &mut Ui) {
        self.refresh_block_changes();
        if self.block_changes.list.is_empty() {
            return;
        }
        let text = format!("Unsaved: {}", summary(&self.block_changes.list));
        let open = self.settings.layout.side_panel_open && self.settings.layout.side_view == SideView::Changes;
        let button = egui::Button::new(text).small().selected(open);
        if ui.add(button).on_hover_text("Show every block that will change on Save").clicked() {
            if open {
                self.settings.layout.side_panel_open = false;
            } else {
                self.show_changes_panel();
            }
        }
    }

    /// The Changes view: a card per changed block (a click scrolls the
    /// preview to it), then Discard all and Save.
    pub(super) fn changes_panel(&mut self, ui: &mut Ui) {
        self.refresh_block_changes();
        let mut revert = None;
        let mut jump = None;
        let mut discard = false;
        let mut save = false;
        let changes = &self.block_changes.list;
        egui::Frame::new().inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            if changes.is_empty() {
                let hint = "No unsaved changes. Blocks you edit, add or delete show here until you save.";
                ui.label(RichText::new(hint).weak().size(11.5));
                return;
            }
            ui.horizontal(|ui| {
                save = ui.button("Save").on_hover_text("Cmd+S").clicked();
                discard = ui.button("Discard all").on_hover_text("Back to the saved file (Cmd+Z undoes it)").clicked();
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (index, change) in changes.iter().enumerate() {
                    let card = change_card(ui, change);
                    if card.reverted {
                        revert = Some(index);
                    }
                    if card.clicked {
                        jump = Some(change.block);
                    }
                    ui.add_space(6.0);
                }
                let hint = "Compared with the file as last saved, so undoing an edit takes it off this list.";
                ui.label(RichText::new(hint).weak().size(11.5));
            });
        });
        if let Some(index) = revert {
            let change = self.block_changes.list[index].clone();
            self.revert_change(&change);
        }
        if let Some(block) = jump {
            if !self.showing_preview() {
                self.toggle_preview();
            }
            scroll_preview_to_block(ui.ctx(), block);
        }
        if discard {
            self.discard_all_changes();
        }
        if save {
            self.save();
        }
    }

    /// Put one block back the way it was saved: an edited block gets its
    /// saved text, a new one goes, a deleted one comes back in its place.
    /// One step of the undo history.
    pub(super) fn revert_change(&mut self, change: &BlockChange) {
        // An open block edit goes in first; the blocks may move, so the
        // change is looked up again by its text.
        self.finish_block_edit();
        self.refresh_block_changes();
        // Of two blocks with the same text, the one nearest the old place.
        let found = self
            .block_changes
            .list
            .iter()
            .filter(|candidate| same_change(candidate, change))
            .min_by_key(|candidate| candidate.block.abs_diff(change.block))
            .cloned();
        let Some(change) = found else {
            return;
        };
        let OpenFile::Text { document, preview, .. } = &mut self.open_file else {
            return;
        };
        let blocks = preview.get_or_insert_with(|| parse_source_blocks(&document.text));
        let Some((range, new_text)) = revert_edit(&document.text, blocks, &change) else {
            return;
        };
        replace_with_undo(&self.repaint, document, preview, range, &new_text);
        let done = match change.kind {
            ChangeKind::Edited => "Reverted the block",
            ChangeKind::Added => "Removed the new block",
            ChangeKind::Deleted => "Restored the block",
        };
        self.show_status(format!("{done} (Cmd+Z undoes it)"));
    }

    /// Back to the saved text in one step of the undo history, so Cmd+Z
    /// brings the changes back.
    fn discard_all_changes(&mut self) {
        self.block_edit = None;
        let OpenFile::Text { document, preview, .. } = &mut self.open_file else {
            return;
        };
        let saved = document.base_text().to_string();
        let whole = 0..document.text.len();
        replace_with_undo(&self.repaint, document, preview, whole, &saved);
        self.show_status("Discarded the unsaved changes (Cmd+Z brings them back)".to_string());
    }
}

/// What was clicked on a change card.
struct CardClicks {
    /// The card itself: scroll to the block.
    clicked: bool,
    /// Its Revert, Remove or Restore button.
    reverted: bool,
}

/// One change in the panel: the block's first line, then its kind, the
/// saved and current Markdown and the button that undoes it.
fn change_card(ui: &mut Ui, change: &BlockChange) -> CardClicks {
    let tint = ui.visuals().hyperlink_color.gamma_multiply(CARD_TINT);
    let frame = egui::Frame::new().fill(tint).corner_radius(8).inner_margin(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let source = if change.kind == ChangeKind::Deleted { &change.saved } else { &change.now };
        let first_line: String = source.lines().next().unwrap_or_default().chars().take(CARD_TITLE_CHARS).collect();
        ui.label(RichText::new(first_line).strong());
        change_details(ui, change)
    });
    let response = frame.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    CardClicks { clicked: response.clicked() && !frame.inner, reverted: frame.inner }
}

/// The text edit that undoes `change` in `text` (split into `blocks`):
/// the range to replace and what goes there. `None` when its block is gone.
fn revert_edit(text: &str, blocks: &[SourceBlock], change: &BlockChange) -> Option<(Range<usize>, String)> {
    match change.kind {
        ChangeKind::Edited => Some((blocks.get(change.block)?.range.clone(), change.saved.clone())),
        ChangeKind::Added => Some((deleted_block_range(text, blocks.get(change.block)?.range.clone()), String::new())),
        ChangeKind::Deleted => Some(match blocks.get(change.block) {
            Some(next) => {
                let at = next.range.start;
                (at..at, inserted_block(&text[..at], &change.saved))
            }
            None => (text.len()..text.len(), appended_block(text, &change.saved)),
        }),
    }
}

/// The same block change, though blocks before it may have moved.
fn same_change(a: &BlockChange, b: &BlockChange) -> bool {
    a.kind == b.kind && a.saved == b.saved && a.now == b.now
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `saved` edited into `current`, then every change reverted one at a
    /// time (last first, so earlier block numbers stay right).
    fn revert_all(saved: &str, current: &str) -> String {
        let mut text = current.to_string();
        let changes = compare_blocks(saved, current, &parse_source_blocks(current));
        assert!(!changes.is_empty(), "the texts differ");
        for change in changes.iter().rev() {
            let blocks = parse_source_blocks(&text);
            let (range, new_text) = revert_edit(&text, &blocks, change).expect("its block is there");
            text.replace_range(range, &new_text);
        }
        text
    }

    #[test]
    fn reverting_every_change_gives_back_the_saved_text() {
        let saved = "# Notes\n\nKeep\n\n- a\n- b\n\nGone\n\nEnd\n";
        assert_eq!(revert_all(saved, "# Notes\n\nKeep\n\n- a\n- b\n- c\n\nEnd\n\nNew\n"), saved);
    }

    #[test]
    fn a_deleted_first_or_last_block_comes_back_in_place() {
        assert_eq!(revert_all("One\n\nTwo\n", "Two\n"), "One\n\nTwo\n");
        assert_eq!(revert_all("One\n\nTwo\n", "One\n"), "One\n\nTwo\n");
    }

    #[test]
    fn a_new_last_block_goes_with_its_blank_line() {
        assert_eq!(revert_all("One\n", "One\n\nTwo\n"), "One\n");
    }
}
