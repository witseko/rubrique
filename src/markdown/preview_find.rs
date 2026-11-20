//! Search and tint the text drawn during a Markdown or HTML preview pass.

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::text::{ByteIndex, CCursor, LayoutJob, LayoutSection};
use eframe::egui::text_selection::LabelSelectionState;
use eframe::egui::{
    Align, Color32, Context, FontSelection, Id, Label, Response, Role, Stroke, Ui, WidgetInfo, WidgetText,
};

use crate::document::view::{current_match_fill, match_fill};
use crate::find::{SearchOptions, find_all};

/// A find pass scoped to the document preview, in widget drawing order.
#[derive(Clone, Debug, Default)]
pub(crate) struct PreviewFind {
    query: Arc<str>,
    options: SearchOptions,
    current: usize,
    count: usize,
    scroll: bool,
}

impl PreviewFind {
    /// Start with no matches counted and optionally scroll to the current one.
    pub(crate) fn new(query: &str, options: SearchOptions, current: usize, scroll: bool) -> Self {
        Self { query: Arc::from(query), options, current, count: 0, scroll }
    }

    /// Make this search available to nested preview widgets for this draw only.
    pub(crate) fn draw<T>(self, ui: &mut Ui, draw: impl FnOnce(&mut Ui) -> T) -> (T, usize) {
        let id = search_id();
        ui.ctx().data_mut(|data| data.insert_temp(id, self));
        let output = draw(ui);
        let count = ui.ctx().data_mut(|data| data.remove_temp::<Self>(id)).map_or(0, |search| search.count);
        (output, count)
    }
}

/// One match within a logical run, which can cover several styled widgets.
#[derive(Debug)]
pub(super) struct Match {
    range: Range<usize>,
    current: bool,
}

fn search_id() -> Id {
    Id::unique("rubrique-preview-find")
}

/// True while the open document preview is drawing with a non-empty query.
pub(crate) fn active(ui: &Ui) -> bool {
    ui.ctx().data(|data| data.get_temp::<PreviewFind>(search_id()).is_some())
}

/// Find matches in a logical run and count them in document order.
pub(super) fn scan(ui: &Ui, text: &str) -> Vec<Match> {
    let id = search_id();
    ui.ctx().data_mut(|data| {
        if data.get_temp::<PreviewFind>(id).is_none() {
            return Vec::new();
        }
        let search = data.get_temp_mut_or_default::<PreviewFind>(id);
        find_all(text, &search.query, search.options)
            .into_iter()
            .filter(|range| !range.is_empty())
            .map(|range| {
                let current = search.count == search.current;
                search.count += 1;
                Match { range, current }
            })
            .collect()
    })
}

/// Apply the portion of each match that falls in this widget. Backgrounds
/// use layout sections so the text keeps its font, line breaks and hit area.
pub(super) fn tint(ui: &Ui, text: WidgetText, offset: usize, matches: &[Match]) -> (WidgetText, Option<usize>) {
    let end = offset + text.text().len();
    let first = matches.partition_point(|hit| hit.range.end <= offset);
    let last = matches.partition_point(|hit| hit.range.start < end);
    let Some(hits) = matches.get(first..last).filter(|hits| !hits.is_empty()) else {
        return (text, None);
    };
    let mut job = Arc::unwrap_or_clone(text.into_layout_job(ui.style(), FontSelection::Default, ui.text_valign()));
    let regular = match_fill(ui.visuals());
    let current = current_match_fill(ui.visuals());
    let scroll = hits.iter().find(|hit| hit.current).and_then(|hit| {
        let start = hit.range.start.saturating_sub(offset);
        take_scroll(ui.ctx()).then(|| job.text[..start].chars().count())
    });
    tint_sections(&mut job, hits, offset, regular, current);
    (job.into(), scroll)
}

/// Split only the formatting sections touched by a match.
fn tint_sections(job: &mut LayoutJob, hits: &[Match], offset: usize, regular: Color32, current: Color32) {
    for mut section in std::mem::take(&mut job.sections) {
        let first = hits.partition_point(|hit| hit.range.end <= offset + section.byte_range.start.0);
        let last = hits.partition_point(|hit| hit.range.start < offset + section.byte_range.end.0);
        for hit in &hits[first..last] {
            let start = ByteIndex(hit.range.start.saturating_sub(offset)).max(section.byte_range.start);
            let end = ByteIndex(hit.range.end - offset).min(section.byte_range.end);
            if section.byte_range.start < start {
                push_section(&mut job.sections, &mut section, start, None);
            }
            push_section(&mut job.sections, &mut section, end, Some(if hit.current { current } else { regular }));
        }
        if !section.byte_range.is_empty() {
            job.sections.push(section);
        }
    }
}

/// Emit the next part of a section and advance it without repeating its indent.
fn push_section(parts: &mut Vec<LayoutSection>, section: &mut LayoutSection, end: ByteIndex, tint: Option<Color32>) {
    let mut part = section.clone();
    part.byte_range.end = end;
    if let Some(tint) = tint {
        part.format.background = tint;
    }
    parts.push(part);
    section.byte_range.start = end;
    section.leading_space = 0.0;
}

/// Scroll once, even if the current match spans several widgets.
fn take_scroll(ctx: &Context) -> bool {
    ctx.data_mut(|data| std::mem::take(&mut data.get_temp_mut_or_default::<PreviewFind>(search_id()).scroll))
}

/// Draw a standalone preview label such as a code block or media caption.
pub(crate) fn label(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    let text = text.into();
    let matches = scan(ui, text.text());
    let (text, scroll) = tint(ui, text, 0, &matches);
    draw_label(ui, text, scroll)
}

/// Draw the label normally, or use its galley to scroll to the exact line.
pub(super) fn draw_label(ui: &mut Ui, text: WidgetText, scroll: Option<usize>) -> Response {
    let Some(index) = scroll else {
        return ui.label(text);
    };
    let (pos, galley, response) = Label::new(text).layout_in_ui(ui);
    scroll_to_match(ui, &galley, pos, index);
    response.widget_info(|| WidgetInfo::labeled(Role::Label, ui.is_enabled(), galley.text()));
    if ui.is_rect_visible(response.rect) {
        let color = ui.visuals().text_color();
        if ui.style().interaction.selectable_labels {
            LabelSelectionState::label_text_selection(ui, &response, pos, galley, color, Stroke::NONE);
        } else {
            ui.painter().galley(pos, galley, color);
        }
    }
    response
}

/// Scroll to the line that contains the active match.
pub(super) fn scroll_to_match(ui: &Ui, galley: &eframe::egui::Galley, pos: eframe::egui::Pos2, index: usize) {
    let mut cursor = CCursor::new(index);
    cursor.prefer_next_row = true;
    let target = galley.pos_from_cursor(cursor).translate(pos.to_vec2());
    ui.scroll_to_rect(target.expand(16.0), Some(Align::Center));
}
