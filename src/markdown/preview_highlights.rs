//! Fit preview highlight backgrounds to the font's height.

use std::sync::Arc;

use eframe::egui::epaint::{TextShape, text::Row};
use eframe::egui::layers::ShapeIdx;
use eframe::egui::{Color32, Rangef, Shape, Ui};

use super::preview_find;
use crate::document::view::{current_match_fill, match_fill};

/// Keep egui's text layout and selection behavior, trimming only the painted highlights.
pub(super) fn fit<T>(ui: &mut Ui, draw: impl FnOnce(&mut Ui) -> T) -> T {
    let find = preview_find::active(ui).then(|| [match_fill(ui.visuals()), current_match_fill(ui.visuals())]);
    let selection = ui.style().interaction.selectable_labels.then_some(ui.visuals().selection.bg_fill);
    if find.is_none() && selection.is_none() {
        return draw(ui);
    }
    let layer = ui.layer_id();
    let first = ui.ctx().graphics(|layers| layers.get(layer).map_or(0, |list| list.next_idx().0));
    let output = draw(ui);
    ui.ctx().graphics_mut(|layers| {
        if let Some(list) = layers.get_mut(layer) {
            for index in first..list.next_idx().0 {
                list.mutate_shape(ShapeIdx(index), |clipped| {
                    if let Shape::Text(text) = &mut clipped.shape {
                        fit_text(text, find.as_ref(), selection);
                    }
                });
            }
        }
    });
    output
}

/// Trim each painted row independently so wrapped matches keep their line spacing.
fn fit_text(text: &mut TextShape, find: Option<&[Color32; 2]>, selection: Option<Color32>) {
    for index in 0..text.galley.rows.len() {
        let row = &text.galley.rows[index];
        let find = find.filter(|colors| {
            row.visuals.mesh.vertices[..row.visuals.glyph_vertex_range.start]
                .iter()
                .any(|vertex| colors.contains(&vertex.color))
        });
        let selected = selection.and_then(|color| selection_start(row, color));
        if (find.is_none() && selected.is_none()) || row.glyphs.is_empty() {
            continue;
        }
        let bounds = font_bounds(row);
        let row = Arc::make_mut(&mut Arc::make_mut(&mut text.galley).rows[index].row);
        fit_row(row, bounds, find, selected);
    }
}

/// egui appends the selection quad after the other vertices and inserts its triangles before the glyph triangles.
fn selection_start(row: &Row, color: Color32) -> Option<usize> {
    let mesh = &row.visuals.mesh;
    let first = mesh.vertices.len().checked_sub(4)?;
    if first < row.visuals.glyph_vertex_range.end {
        return None;
    }
    let start = row.visuals.glyph_index_start;
    let indices = mesh.indices.get(start..start + 6)?;
    (indices.iter().all(|&index| index as usize >= first)
        && mesh.vertices[first..].iter().all(|vertex| vertex.color == color))
    .then_some(first)
}

/// Find the vertical bounds of the fonts on this row at their painted baselines.
fn font_bounds(row: &Row) -> Rangef {
    let mut bounds = Rangef::NOTHING;
    for glyph in &row.glyphs {
        let top = glyph.pos.y - glyph.font_face_ascent;
        bounds.min = bounds.min.min(top);
        bounds.max = bounds.max.max(top + glyph.font_face_height);
    }
    bounds
}

/// Keep each rectangle's horizontal extent and leave glyph vertices untouched.
fn fit_row(row: &mut Row, bounds: Rangef, find: Option<&[Color32; 2]>, selected: Option<usize>) {
    // egui keeps the selection's vertex indices until the end of the pass.
    let vertices = &mut row.visuals.mesh.vertices;
    if let Some(colors) = find {
        for vertex in &mut vertices[..row.visuals.glyph_vertex_range.start] {
            if colors.contains(&vertex.color) {
                vertex.pos.y = vertex.pos.y.clamp(bounds.min, bounds.max);
            }
        }
    }
    if let Some(first) = selected {
        for vertex in &mut vertices[first..] {
            vertex.pos.y = vertex.pos.y.clamp(bounds.min, bounds.max);
        }
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Context, Event, FullOutput, Modifiers, PointerButton, RawInput, Rect, RichText, pos2};

    use super::*;
    use crate::find::SearchOptions;

    /// Draw a tall line with the regular preview label and a live Find match.
    fn preview_frame(ctx: &Context, events: Vec<Event>) -> FullOutput {
        let mut output = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
            ui.style_mut().interaction.selectable_labels = true;
            preview_find::PreviewFind::new("README", SearchOptions::default(), 0, false).draw(ui, |ui| {
                fit(ui, |ui| {
                    preview_find::label(ui, RichText::new("README screenshots").size(16.0).line_height(Some(40.0)));
                });
            });
        });
        output.textures_delta.clear();
        output
    }

    /// Read the visible bounds of a background by its tint.
    fn painted_bounds(row: &Row, color: Color32) -> Rect {
        row.visuals.mesh.vertices.iter().filter(|vertex| vertex.color == color).fold(
            Rect::NOTHING,
            |mut rect, vertex| {
                rect.extend_with(vertex.pos);
                rect
            },
        )
    }

    /// Locate the preview label's painted text.
    fn text_shape(output: &FullOutput) -> &TextShape {
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                Shape::Text(text) if text.galley.text() == "README screenshots" => Some(text),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn find_and_mouse_selection_fit_the_font_without_changing_line_spacing() {
        let ctx = Context::default();
        let output = preview_frame(&ctx, vec![]);
        let text = text_shape(&output);
        let row = &text.galley.rows[0];
        assert!((row.size.y - 40.0).abs() < 0.01);
        let style = ctx.global_style();
        let find = painted_bounds(row, current_match_fill(&style.visuals));
        assert!(find.height() > 0.0 && find.bottom() < row.size.y - 5.0);
        let expected_width =
            row.glyphs[5].max_x() - row.glyphs[0].pos.x + 2.0 * text.galley.job.sections[0].format.expand_bg;
        assert!((find.width() - expected_width).abs() <= 1.0);

        let start = text.pos + row.pos.to_vec2() + pos2(row.x_offset(7.into()), find.center().y).to_vec2();
        let end = text.pos + row.pos.to_vec2() + pos2(row.x_offset(14.into()), find.center().y).to_vec2();
        let click = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        preview_frame(&ctx, vec![Event::PointerMoved(start), click(start, true)]);
        preview_frame(&ctx, vec![Event::PointerMoved(end)]);
        let output = preview_frame(&ctx, vec![click(end, false)]);

        let selected_row = &text_shape(&output).galley.rows[0];
        let selection = painted_bounds(selected_row, style.visuals.selection.bg_fill);
        assert!(selection.height() > 0.0 && selection.bottom() < selected_row.size.y - 5.0);
        assert!((selection.width() - (end.x - start.x)).abs() < 1.0);
        assert_eq!(selected_row.glyphs, row.glyphs);
    }
}
