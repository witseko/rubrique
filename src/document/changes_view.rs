//! How unsaved block changes look: the bar in the margin beside a changed
//! block (with a before and after card on hover), the dashed line where a
//! block was deleted, and the card's contents, which the Changes panel
//! shows too. Colors come from the theme: the accent for the marks, the
//! selection color for added text and the error color for removed text.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Color32, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use super::block_changes::{BlockChange, ChangeKind, inline_diff};

/// Width of the bar beside a changed block.
const MARK_WIDTH: f32 = 3.0;
/// Space between the bar and the block's hover background.
const MARK_GAP: f32 = 8.0;
/// Extra width on each side of the bar that still shows its card.
const MARK_HIT_SLOP: f32 = 6.0;
/// Opacity of the accent behind a block that is new since the save.
const ADDED_TINT: f32 = 0.07;
/// Opacity of the accent of the dashed line where a block was deleted.
const DELETED_LINE_ALPHA: f32 = 0.6;
/// Length of a dash (and of the space after it) on that line.
const DASH_LENGTH: f32 = 4.0;
/// Height of the row that holds the deleted line and its label.
const DELETED_ROW_HEIGHT: f32 = 22.0;
/// Opacity of the error color behind removed text.
const REMOVED_TINT: f32 = 0.25;
/// Width of the before and after card.
const CARD_WIDTH: f32 = 480.0;
/// Characters of unchanged text kept on each side of a change.
const CONTEXT_CHARS: usize = 80;
/// Characters of changed text shown before it is cut short.
const CHANGED_CHARS: usize = 600;
/// Characters of a deleted block in its line's label.
const LABEL_CHARS: usize = 60;

/// "Edited", "New" or "Deleted", for labels.
pub fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Edited => "Edited",
        ChangeKind::Added => "New",
        ChangeKind::Deleted => "Deleted",
    }
}

/// What the button that undoes a change says.
pub fn revert_label(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Edited => "Revert block",
        ChangeKind::Added => "Remove block",
        ChangeKind::Deleted => "Restore block",
    }
}

/// "2 edited, 1 new, 1 deleted", leaving out the kinds there are none of.
pub fn summary(changes: &[BlockChange]) -> String {
    let count = |kind| changes.iter().filter(|change| change.kind == kind).count();
    let parts: Vec<String> =
        [(ChangeKind::Edited, "edited"), (ChangeKind::Added, "new"), (ChangeKind::Deleted, "deleted")]
            .into_iter()
            .map(|(kind, word)| (count(kind), word))
            .filter(|(number, _)| *number > 0)
            .map(|(number, word)| format!("{number} {word}"))
            .collect();
    parts.join(", ")
}

/// Paint the mark of an edited or new block whose drawn area is `block`
/// (its hover background), behind the block through `background`, a shape
/// slot taken before the block was drawn. Hovering the bar shows the card;
/// returns true when its button was clicked.
pub fn block_mark(ui: &mut Ui, block: Rect, background: egui::layers::ShapeIdx, change: &BlockChange) -> bool {
    let accent = ui.visuals().hyperlink_color;
    if change.kind == ChangeKind::Added {
        ui.painter().set(background, egui::Shape::rect_filled(block, 6, accent.gamma_multiply(ADDED_TINT)));
    }
    let bar = Rect::from_x_y_ranges(
        block.left() - MARK_GAP - MARK_WIDTH..=block.left() - MARK_GAP,
        block.y_range().shrink(2.0),
    );
    let id = ui.scope_id().with(("change-mark", change.block));
    let response = ui.interact(bar.expand2(vec2(MARK_HIT_SLOP, 0.0)), id, Sense::hover());
    let width = if response.hovered() { MARK_WIDTH + 2.0 } else { MARK_WIDTH };
    let bar = Rect::from_center_size(bar.center(), vec2(width, bar.height()));
    ui.painter().rect_filled(bar, 2, accent);
    let mut clicked = false;
    response.on_hover_ui(|ui| {
        ui.set_max_width(CARD_WIDTH);
        clicked = change_details(ui, change);
    });
    clicked
}

/// The row where a deleted block was: a dashed accent line from the margin
/// across the page, the start of the block, and Restore. Returns true when
/// Restore was clicked.
pub fn deleted_row(ui: &mut Ui, change: &BlockChange) -> bool {
    let accent = ui.visuals().hyperlink_color;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), DELETED_ROW_HEIGHT), Sense::hover());
    let line_y = row.center().y;
    let line_start = row.left() - MARK_GAP - MARK_WIDTH;
    dashed_line(ui, line_start..=row.right(), line_y, accent.gamma_multiply(DELETED_LINE_ALPHA));
    let mut clicked = false;
    let mut row_ui =
        ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let page = crate::theme::style::page_fill(row_ui.visuals());
    let label = format!("Deleted: \u{201c}{}\u{201d}", head_chars(first_line(&change.saved), LABEL_CHARS));
    // The page color behind the words hides the line under them.
    egui::Frame::new().fill(page).inner_margin(egui::Margin::symmetric(4, 0)).show(&mut row_ui, |ui| {
        ui.label(RichText::new(label).color(accent).small()).on_hover_ui(|ui| {
            ui.set_max_width(CARD_WIDTH);
            change_details(ui, change);
        });
    });
    row_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        egui::Frame::new().fill(page).inner_margin(egui::Margin::symmetric(4, 0)).show(ui, |ui| {
            let restore = egui::Button::new(RichText::new("Restore").color(accent).small()).frame(false);
            clicked = ui.add(restore).on_hover_text("Put the block back where it was").clicked();
        });
    });
    clicked
}

/// The card's contents: what kind of change, the saved and current
/// Markdown with the changed stretch highlighted, and the button that
/// undoes it. Returns true when that button was clicked.
pub fn change_details(ui: &mut Ui, change: &BlockChange) -> bool {
    ui.label(RichText::new(format!("{} since save", kind_name(change.kind))).weak().small());
    match change.kind {
        ChangeKind::Edited => {
            let diff = inline_diff(&change.saved, &change.now);
            source_box(
                ui,
                "SAVED",
                &[
                    (tail_chars(diff.before, CONTEXT_CHARS), Look::Plain),
                    (head_chars(diff.removed, CHANGED_CHARS), Look::Removed),
                    (head_chars(diff.after, CONTEXT_CHARS), Look::Plain),
                ],
            );
            source_box(
                ui,
                "NOW",
                &[
                    (tail_chars(diff.before, CONTEXT_CHARS), Look::Plain),
                    (head_chars(diff.added, CHANGED_CHARS), Look::Added),
                    (head_chars(diff.after, CONTEXT_CHARS), Look::Plain),
                ],
            );
        }
        ChangeKind::Added => source_box(ui, "NOW", &[(head_chars(&change.now, CHANGED_CHARS), Look::Added)]),
        ChangeKind::Deleted => source_box(ui, "SAVED", &[(head_chars(&change.saved, CHANGED_CHARS), Look::Removed)]),
    }
    ui.add_space(2.0);
    ui.button(revert_label(change.kind)).on_hover_text("Cmd+Z undoes this too").clicked()
}

/// How a stretch of text in a source box is drawn.
#[derive(Clone, Copy)]
enum Look {
    Plain,
    Added,
    Removed,
}

/// A small heading over a box of Markdown made of `parts`.
fn source_box(ui: &mut Ui, heading: &str, parts: &[(String, Look)]) {
    ui.label(RichText::new(heading).weak().small());
    let visuals = ui.visuals().clone();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let mut job = LayoutJob::default();
    for (text, look) in parts {
        let mut format = TextFormat::simple(font.clone(), visuals.text_color());
        match look {
            Look::Plain => {}
            Look::Added => {
                format.background = visuals.selection.bg_fill;
                format.color = visuals.strong_text_color();
            }
            Look::Removed => {
                format.background = visuals.error_fg_color.gamma_multiply(REMOVED_TINT);
                format.color = visuals.strong_text_color();
                format.strikethrough = Stroke::new(1.0, visuals.strong_text_color());
            }
        }
        job.append(text, 0.0, format);
    }
    egui::Frame::new().fill(visuals.code_bg_color).corner_radius(6).inner_margin(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        job.wrap.max_width = ui.available_width();
        ui.label(job);
    });
}

/// A horizontal dashed line across `x` at height `y`.
fn dashed_line(ui: &Ui, x: std::ops::RangeInclusive<f32>, y: f32, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    let mut start = *x.start();
    while start < *x.end() {
        let end = (start + DASH_LENGTH).min(*x.end());
        ui.painter().line_segment([pos2(start, y), pos2(end, y)], stroke);
        start += DASH_LENGTH * 2.0;
    }
}

/// The first line of `text`.
fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// The first `count` characters of `text`, with "…" when there were more.
fn head_chars(text: &str, count: usize) -> String {
    match text.char_indices().nth(count) {
        Some((cut, _)) => format!("{}\u{2026}", &text[..cut]),
        None => text.to_string(),
    }
}

/// The last `count` characters of `text`, with "…" before them when there were more.
fn tail_chars(text: &str, count: usize) -> String {
    let length = text.chars().count();
    if length <= count {
        return text.to_string();
    }
    let cut = text.char_indices().nth(length - count).map_or(0, |(index, _)| index);
    format!("\u{2026}{}", &text[cut..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(kind: ChangeKind) -> BlockChange {
        BlockChange { kind, block: 0, saved: String::new(), now: String::new() }
    }

    #[test]
    fn summary_names_only_the_kinds_there_are() {
        assert_eq!(summary(&[]), "");
        let changes = [change(ChangeKind::Edited), change(ChangeKind::Deleted), change(ChangeKind::Edited)];
        assert_eq!(summary(&changes), "2 edited, 1 deleted");
    }

    #[test]
    fn long_text_is_cut_at_whole_characters() {
        assert_eq!(head_chars("abc", 3), "abc");
        assert_eq!(head_chars("✔✔✔✔", 2), "✔✔\u{2026}");
        assert_eq!(tail_chars("abc", 3), "abc");
        assert_eq!(tail_chars("✔✔✔✔", 2), "\u{2026}✔✔");
    }
}
