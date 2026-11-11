//! Settings → Highlights: the switch that turns text highlights on and off,
//! a sample to try the rules on, and the rules themselves, one short row
//! each that opens to edit. Under the rules, a field asks the AI to write a
//! rule from a description.

use eframe::egui::cache::{ComputerMut, FrameCache};
use eframe::egui::{
    self, Align, Color32, Id, Label, Layout, Response, RichText, Sense, Shape, Stroke, TextBuffer, Ui, UiBuilder, Vec2,
    pos2, vec2,
};

use super::{ROW_MARGIN, SMALL_SIZE, SettingsAction, enabled_button, list_frame};
use crate::document::highlighting::{HighlightStyle, highlighted};
use crate::highlight::text_rules::{HighlightRule, RuleColor, pattern_error};
use crate::icons::{Icon, icon_label};
use crate::settings::text_highlight::TextHighlightSettings;
use crate::theme::palette::Rgb;

/// What the sample shows until the user types their own lines. One of each
/// thing the default rules color, so every rule shows a count.
const SAMPLE: &str = "2026-11-11 09:14:05 INFO request 3f2a9c1e-7b4d-4e21-9a0c-5d8e1f6b2a90 from 10.0.4.17\n\
                      2026-11-11 09:14:09 WARN retry 3 of 5 for ops@rubrique.dev on fe80::1\n\
                      2026-11-11 09:14:12 INFO deployed a1b2c3d for PROJ-482 in 182 ms";
/// Where the sample text is kept while the app runs (it is not saved).
const SAMPLE_ID: &str = "rubrique-highlight-sample";
/// Where the place of the open rule row is kept.
const OPEN_RULE_ID: &str = "rubrique-open-highlight-rule";

/// Size of the switch that turns all highlights on and off.
const MAIN_SWITCH_SIZE: Vec2 = vec2(36.0, 20.0);
/// Size of the switch on each rule row.
const RULE_SWITCH_SIZE: Vec2 = vec2(28.0, 16.0);
/// Height of a closed rule row.
const ROW_HEIGHT: f32 = 34.0;
/// Width of a rule's name on its closed row.
const ROW_NAME_WIDTH: f32 = 130.0;
/// Width of the "2 in sample" text at the end of a closed row.
const COUNT_WIDTH: f32 = 80.0;
/// Side of the open and closed arrow at the end of a row.
const CHEVRON_SIZE: f32 = 10.0;
/// How far an open row's editor is indented, so it lines up with the color
/// square on the row above it (row margin, switch and the gap after it).
const EDITOR_INDENT: i8 = 48;
/// Opacity of a row whose rule does nothing now (it or highlights are off).
const DIM_OPACITY: f32 = 0.45;
/// Width of a rule's name field in the editor.
const NAME_FIELD_WIDTH: f32 = 170.0;
/// Side of the color square before a rule's name.
const SWATCH_SIZE: f32 = 12.0;
const SWATCH_RADIUS: u8 = 3;
/// Side of a color button in the editor.
const SWATCH_BUTTON_SIZE: f32 = 20.0;
/// Room kept beside the description field for its button.
const WRITE_BUTTON_WIDTH: f32 = 70.0;
/// Inner margin of the switch card at the top.
const CARD_MARGIN: i8 = 14;
/// Gap between the switch card's text and its switch.
const CARD_GAP: f32 = 12.0;

/// Where the AI rule writer stands, for the field under the list.
#[derive(Clone, Copy, Debug)]
pub enum RuleWriter<'a> {
    Idle,
    /// The AI has been writing a pattern for this many seconds.
    Writing(u64),
    /// The last attempt failed, and why.
    Failed(&'a str),
}

/// Everything the Highlights group shows and edits.
#[derive(Debug)]
pub struct HighlightInfo<'a> {
    pub settings: &'a mut TextHighlightSettings,
    /// What to highlight, in words, for the AI.
    pub description: &'a mut String,
    /// A rule the AI wrote, waiting to be added or discarded.
    pub draft: &'a mut Option<HighlightRule>,
    pub writer: RuleWriter<'a>,
    /// The Ask tool's name ("Claude"), which writes the rule.
    pub tool_name: &'static str,
}

/// What a click on a rule row asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowEdit {
    /// Open the row's editor, or close it when it is open.
    OpenOrClose,
    MoveUp,
    MoveDown,
    Remove,
}

/// How one rule row looks, beyond the rule itself.
#[derive(Clone, Copy, Debug)]
struct RowLook {
    open: bool,
    /// Pieces of the sample this rule colors; `None` while the count is not
    /// known (highlights off, or the rules changed this frame).
    count: Option<usize>,
    /// Text highlights are on.
    highlights_on: bool,
    /// Whether the rule can move up and down.
    can_move: (bool, bool),
}

/// The Highlights group. Returns what the user clicked.
pub fn highlights_section(ui: &mut Ui, info: HighlightInfo<'_>) -> Option<SettingsAction> {
    let HighlightInfo { settings, description, draft, writer, tool_name } = info;
    let style = HighlightStyle::from_ui(ui);
    switch_card(ui, &mut settings.enabled);

    ui.add_space(14.0);
    heading(ui, "Sample", "Paste a few lines of your own text to try the rules");
    let counts = sample_box(ui, &style);

    ui.add_space(14.0);
    heading(ui, "Rules", "Tried top to bottom; the first match colors the text");
    let action = rule_list(ui, settings, &style, &counts, description, writer, tool_name);
    if let Some(rule) = draft {
        ui.add_space(6.0);
        match draft_card(ui, rule, &style, tool_name) {
            Some(true) => settings.rules.extend(draft.take()),
            Some(false) => *draft = None,
            None => {}
        }
    }
    action
}

/// The card at the top: what highlights do, and the switch for all of them.
fn switch_card(ui: &mut Ui, enabled: &mut bool) {
    let visuals = ui.visuals();
    egui::Frame::new()
        .fill(visuals.faint_bg_color)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(super::LIST_RADIUS)
        .inner_margin(CARD_MARGIN)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Text on the left in the room the switch leaves, switch on the right.
            ui.horizontal(|ui| {
                let text_width = ui.available_width() - MAIN_SWITCH_SIZE.x - CARD_GAP;
                ui.allocate_ui_with_layout(vec2(text_width, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(RichText::new("Color matches in plain text").strong());
                    ui.label(
                        RichText::new(
                            "IP addresses, ids, dates and your own patterns, in text files and the Markdown \
                             preview. Cmd/Ctrl+Shift+H or the highlighter in the status bar turns them off and on.",
                        )
                        .weak(),
                    );
                });
                ui.add_space(CARD_GAP);
                switch(ui, enabled, MAIN_SWITCH_SIZE, "Color matches in plain text");
            });
        });
}

/// A section's name with what it is for beside it, on one line.
fn heading(ui: &mut Ui, title: &str, hint: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(title).strong());
        ui.label(RichText::new(hint).weak());
    });
}

/// The sample lines, colored by the rules as they are now and editable.
/// Returns how many pieces of it each rule colors (empty while highlights
/// are off).
fn sample_box(ui: &mut Ui, style: &HighlightStyle) -> Vec<usize> {
    let id = Id::unique(SAMPLE_ID);
    let mut text = ui.data(|data| data.get_temp::<String>(id)).unwrap_or_else(|| SAMPLE.to_owned());
    let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, wrap_width: f32| {
        let mut job = highlighted(ui.ctx(), buffer.as_str(), style, None);
        job.wrap.max_width = wrap_width;
        ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
    };
    let field = egui::TextEdit::multiline(&mut text)
        .code_editor()
        .desired_rows(3)
        .desired_width(f32::INFINITY)
        .layouter(&mut layouter);
    ui.add(field);
    let counts = style.rule_match_counts(&text);
    ui.data_mut(|data| data.insert_temp(id, text));
    counts
}

/// Every rule in a box, one row each, then the Add rule button and the AI
/// field. Applies the rows' moves and removals and remembers the open row.
fn rule_list(
    ui: &mut Ui,
    settings: &mut TextHighlightSettings,
    style: &HighlightStyle,
    counts: &[usize],
    description: &mut String,
    writer: RuleWriter,
    tool_name: &str,
) -> Option<SettingsAction> {
    let open_id = Id::unique(OPEN_RULE_ID);
    let mut open: Option<usize> = ui.data(|data| data.get_temp(open_id)).flatten();
    let rules = &mut settings.rules;
    let count = rules.len();
    let mut edit = None;
    let mut add = false;
    let mut action = None;
    list_frame(ui, |ui| {
        if rules.is_empty() {
            egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
                ui.label(RichText::new("No rules yet: plain text stays uncolored.").weak());
            });
        }
        for (index, rule) in rules.iter_mut().enumerate() {
            if index > 0 {
                ui.add(egui::Separator::default().spacing(0.0));
            }
            let look = RowLook {
                open: open == Some(index),
                count: counts.get(index).copied(),
                highlights_on: settings.enabled,
                can_move: (index > 0, index + 1 < count),
            };
            ui.push_id(("highlight-rule", index), |ui| {
                if let Some(clicked) = rule_row(ui, rule, style, look) {
                    edit = Some((index, clicked));
                }
            });
        }
        ui.add(egui::Separator::default().spacing(0.0));
        (add, action) = list_footer(ui, description, writer, tool_name);
    });
    match edit {
        Some((index, RowEdit::OpenOrClose)) => open = (open != Some(index)).then_some(index),
        // Only the open row has move buttons, so the editor moves with its rule.
        Some((index, RowEdit::MoveUp)) => {
            rules.swap(index - 1, index);
            open = Some(index - 1);
        }
        Some((index, RowEdit::MoveDown)) => {
            rules.swap(index, index + 1);
            open = Some(index + 1);
        }
        Some((index, RowEdit::Remove)) => {
            rules.remove(index);
            open = None;
        }
        None => {}
    }
    if add {
        rules.push(HighlightRule::new("New rule", "", RuleColor::Keyword));
        open = Some(rules.len() - 1);
    }
    ui.data_mut(|data| data.insert_temp(open_id, open));
    action
}

/// One rule: a short row (switch, color, name, pattern, sample count) that
/// opens to the rule's editor when clicked.
fn rule_row(ui: &mut Ui, rule: &mut HighlightRule, style: &HighlightStyle, look: RowLook) -> Option<RowEdit> {
    let fill = if look.open { ui.visuals().faint_bg_color } else { Color32::TRANSPARENT };
    egui::Frame::new()
        .fill(fill)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let clicked = summary_line(ui, rule, style, look);
            let edit = if look.open { rule_editor(ui, rule, style, look) } else { None };
            clicked.then_some(RowEdit::OpenOrClose).or(edit)
        })
        .inner
}

/// The closed row's line. Returns true when it was clicked (anywhere but
/// the switch).
fn summary_line(ui: &mut Ui, rule: &mut HighlightRule, style: &HighlightStyle, look: RowLook) -> bool {
    // A place for the hover fill, set once we know the row is hovered, so it
    // is drawn under the row's text.
    let hover_fill = ui.painter().add(Shape::Noop);
    let row = ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        ui.add_space(f32::from(ROW_MARGIN.left));
        switch(ui, &mut rule.enabled, RULE_SWITCH_SIZE, "Use this rule");
        // The click sense comes before the contents; the labels inside only
        // sense hover, so the whole area takes the click.
        let builder = UiBuilder::new().sense(Sense::click()).layout(Layout::left_to_right(Align::Center));
        ui.scope_builder(builder, |ui| {
            if !(rule.enabled && look.highlights_on) {
                ui.multiply_opacity(DIM_OPACITY);
            }
            ui.set_min_size(vec2(ui.available_width(), ROW_HEIGHT));
            swatch(ui, style.rule_color(rule.color));
            ui.allocate_ui_with_layout(vec2(ROW_NAME_WIDTH, ROW_HEIGHT), Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_width(ROW_NAME_WIDTH);
                let name = if rule.name.trim().is_empty() { "Untitled" } else { rule.name.as_str() };
                ui.add(Label::new(RichText::new(name).strong()).selectable(false).truncate());
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(f32::from(ROW_MARGIN.right));
                chevron(ui, look.open);
                ui.allocate_ui_with_layout(vec2(COUNT_WIDTH, ROW_HEIGHT), Layout::right_to_left(Align::Center), |ui| {
                    ui.add(Label::new(count_text(ui, rule, look)).selectable(false))
                });
                let pattern = if rule.pattern.is_empty() { "no pattern yet" } else { rule.pattern.as_str() };
                // The rest of the row, read left to right, so a short pattern
                // starts right after the name.
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), ROW_HEIGHT),
                    Layout::left_to_right(Align::Center),
                    |ui| ui.add(Label::new(RichText::new(pattern).monospace().weak()).selectable(false).truncate()),
                );
            });
        })
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
    });
    let response = row.inner;
    if response.hovered() {
        let full_width = row.response.rect.with_min_x(ui.max_rect().left()).with_max_x(ui.max_rect().right());
        ui.painter().set(hover_fill, Shape::rect_filled(full_width, 0, ui.visuals().widgets.hovered.weak_bg_fill));
    }
    response.clicked()
}

/// What the end of a closed row says about the rule and the sample.
fn count_text(ui: &Ui, rule: &HighlightRule, look: RowLook) -> RichText {
    let weak = |text: &str| RichText::new(text).size(SMALL_SIZE).weak();
    if !rule.enabled {
        return weak("off");
    }
    if cached_pattern_error(ui.ctx(), &rule.pattern).is_some() {
        return RichText::new("broken").size(SMALL_SIZE).color(ui.visuals().warn_fg_color);
    }
    match (look.highlights_on, look.count) {
        (true, Some(0)) => weak("no match"),
        (true, Some(count)) => weak(&format!("{count} in sample")),
        _ => weak(""),
    }
}

/// The open row's fields: name and color, pattern, then what the rule does
/// to the sample beside the move and delete buttons.
fn rule_editor(ui: &mut Ui, rule: &mut HighlightRule, style: &HighlightStyle, look: RowLook) -> Option<RowEdit> {
    let mut edit = None;
    let margin = egui::Margin { left: EDITOR_INDENT, right: ROW_MARGIN.right, top: 2, bottom: 12 };
    egui::Frame::new().inner_margin(margin).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut rule.name).desired_width(NAME_FIELD_WIDTH).hint_text("Name"));
            ui.add_space(12.0);
            color_buttons(ui, &mut rule.color, style);
        });
        pattern_field(ui, &mut rule.pattern);
        ui.horizontal(|ui| {
            if let Some(text) = sample_text(ui, rule, look) {
                ui.label(RichText::new(text).weak());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let delete = RichText::new("Delete rule").color(ui.visuals().error_fg_color);
                if ui.add(egui::Button::new(delete).frame(false)).clicked() {
                    edit = Some(RowEdit::Remove);
                }
                if enabled_button(ui, "↓", look.can_move.1) {
                    edit = Some(RowEdit::MoveDown);
                }
                if enabled_button(ui, "↑", look.can_move.0) {
                    edit = Some(RowEdit::MoveUp);
                }
            });
        });
    });
    edit
}

/// What an open rule does to the sample, in words. `None` when the pattern
/// is broken: the reason is already under the pattern.
fn sample_text(ui: &Ui, rule: &HighlightRule, look: RowLook) -> Option<String> {
    if !look.highlights_on {
        return Some("Highlights are off, so nothing is colored.".to_owned());
    }
    if !rule.enabled {
        return Some("This rule is off.".to_owned());
    }
    if cached_pattern_error(ui.ctx(), &rule.pattern).is_some() {
        return None;
    }
    Some(match look.count {
        Some(0) | None => "Nothing in the sample matches this rule.".to_owned(),
        Some(1) => "Colors 1 piece of the sample.".to_owned(),
        Some(count) => format!("Colors {count} pieces of the sample."),
    })
}

/// The pattern's field, in monospace, and the reason under it when the
/// pattern cannot be used (that rule is then skipped).
fn pattern_field(ui: &mut Ui, pattern: &mut String) {
    let field = egui::TextEdit::singleline(pattern)
        .font(egui::TextStyle::Monospace)
        .desired_width(f32::INFINITY)
        .hint_text("Regular expression, like \\bPROJ-[0-9]+\\b");
    ui.add(field);
    if let Some(error) = cached_pattern_error(ui.ctx(), pattern) {
        ui.label(RichText::new(error).size(SMALL_SIZE).color(ui.visuals().warn_fg_color));
    }
}

/// A button per code color, then Custom (a "+" that becomes a color button
/// once picked), then the chosen color's name.
fn color_buttons(ui: &mut Ui, color: &mut RuleColor, style: &HighlightStyle) {
    ui.spacing_mut().item_spacing.x = 6.0;
    for each in RuleColor::PALETTE {
        if swatch_button(ui, style.rule_color(each), *color == each, each.name()).clicked() {
            *color = each;
        }
    }
    if let RuleColor::Custom(Rgb(red, green, blue)) = color {
        let mut picked = [*red, *green, *blue];
        if egui::color_picker::color_edit_button_srgb(ui, &mut picked).changed() {
            *color = RuleColor::Custom(Rgb(picked[0], picked[1], picked[2]));
        }
    } else if ui.small_button("+").on_hover_text("Custom color").clicked() {
        // Start from the color shown now, so the pick is a small change.
        let [red, green, blue, _] = style.rule_color(*color).to_array();
        *color = RuleColor::Custom(Rgb(red, green, blue));
    }
    ui.add_space(4.0);
    ui.label(color.name());
}

/// A clickable square of `color`, ringed when it is the chosen one.
fn swatch_button(ui: &mut Ui, color: Color32, chosen: bool, name: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SWATCH_BUTTON_SIZE), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, chosen, name));
    let painter = ui.painter();
    painter.rect_filled(rect.shrink(3.0), SWATCH_RADIUS, color);
    if chosen || response.hovered() || response.has_focus() {
        let ring = if chosen { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
        painter.rect_stroke(rect, 5, Stroke::new(1.5, ring), egui::StrokeKind::Inside);
    }
    response.on_hover_text(name)
}

/// A small filled square of `color`.
fn swatch(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(SWATCH_SIZE, SWATCH_SIZE), Sense::hover());
    ui.painter().rect_filled(rect, SWATCH_RADIUS, color);
}

/// An arrow that points right on a closed row and down on an open one.
fn chevron(ui: &mut Ui, open: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(CHEVRON_SIZE), Sense::hover());
    let center = rect.center();
    let half = CHEVRON_SIZE / 4.0;
    let points = if open {
        vec![
            pos2(center.x - 2.0 * half, center.y - half),
            center + vec2(0.0, half),
            pos2(center.x + 2.0 * half, center.y - half),
        ]
    } else {
        vec![
            pos2(center.x - half, center.y - 2.0 * half),
            center + vec2(half, 0.0),
            pos2(center.x - half, center.y + 2.0 * half),
        ]
    };
    ui.painter().add(Shape::line(points, Stroke::new(1.5, ui.visuals().weak_text_color())));
}

/// An on and off switch: a rounded track with a knob that slides to the
/// right when on. Flips `on` when clicked.
pub(super) fn switch(ui: &mut Ui, on: &mut bool, size: Vec2, label: &str) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, true, *on, label));
    let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
    let visuals = ui.visuals();
    // The off track uses the weak text color: widget fills match the card
    // behind the main switch and would hide it.
    let track = if *on { visuals.hyperlink_color } else { visuals.weak_text_color().gamma_multiply(0.6) };
    let radius = rect.height() / 2.0;
    let painter = ui.painter();
    painter.rect_filled(rect, radius, track);
    if response.has_focus() {
        painter.rect_stroke(rect.expand(2.0), radius + 2.0, visuals.selection.stroke, egui::StrokeKind::Outside);
    }
    let knob_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
    painter.circle_filled(pos2(knob_x, rect.center().y), radius - 2.5, Color32::WHITE);
    response.on_hover_text(label)
}

/// Add rule, then the field that asks the AI for a rule, with its status
/// under them. Returns whether Add rule was clicked, and what else was.
fn list_footer(
    ui: &mut Ui,
    description: &mut String,
    writer: RuleWriter,
    tool_name: &str,
) -> (bool, Option<SettingsAction>) {
    egui::Frame::new()
        .inner_margin(ROW_MARGIN)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let mut add = false;
            let mut action = None;
            ui.horizontal(|ui| {
                add = ui.button("+ Add rule").clicked();
                ui.separator();
                action = writer_field(ui, description, writer, tool_name);
            });
            writer_status(ui, writer);
            (add, action)
        })
        .inner
}

/// The description field with its Write button (Stop while the AI runs).
fn writer_field(ui: &mut Ui, description: &mut String, writer: RuleWriter, tool_name: &str) -> Option<SettingsAction> {
    let mut action = None;
    let writing = matches!(writer, RuleWriter::Writing(_));
    icon_label(ui, Icon::Sparkle);
    let field = egui::TextEdit::singleline(description)
        .desired_width(ui.available_width() - WRITE_BUTTON_WIDTH)
        .hint_text(format!("Or describe one for {tool_name}, like ticket ids such as PROJ-123"));
    let response = ui.add_enabled(!writing, field);
    let submitted = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    let has_text = !description.trim().is_empty();
    if writing {
        if ui.button("Stop").clicked() {
            action = Some(SettingsAction::StopHighlightRule);
        }
    } else if enabled_button(ui, "Write", has_text) || (submitted && has_text) {
        action = Some(SettingsAction::WriteHighlightRule);
    }
    action
}

/// How the AI is doing with a rule: nothing, a timer, or why it failed.
fn writer_status(ui: &mut Ui, writer: RuleWriter) {
    match writer {
        RuleWriter::Idle => {}
        RuleWriter::Writing(seconds) => {
            ui.label(RichText::new(format!("Writing a pattern… {}", crate::ai::format_elapsed(seconds))).weak());
        }
        RuleWriter::Failed(error) => {
            ui.label(RichText::new(error).size(SMALL_SIZE).color(ui.visuals().warn_fg_color));
        }
    }
}

/// The rule the AI wrote, to edit and check: `Some(true)` adds it,
/// `Some(false)` discards it.
fn draft_card(ui: &mut Ui, rule: &mut HighlightRule, style: &HighlightStyle, tool_name: &str) -> Option<bool> {
    let mut choice = None;
    let accent = ui.visuals().hyperlink_color;
    egui::Frame::new()
        .fill(accent.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, accent.gamma_multiply(0.35)))
        .corner_radius(super::LIST_RADIUS)
        .inner_margin(ROW_MARGIN)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.push_id("highlight-draft", |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Sparkle);
                    ui.label(RichText::new(format!("{tool_name} wrote a rule. Check it before adding.")).strong());
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut rule.name).desired_width(NAME_FIELD_WIDTH).hint_text("Name"),
                    );
                    ui.add_space(12.0);
                    color_buttons(ui, &mut rule.color, style);
                });
                pattern_field(ui, &mut rule.pattern);
                ui.horizontal(|ui| {
                    let usable = cached_pattern_error(ui.ctx(), &rule.pattern).is_none();
                    if enabled_button(ui, "Add rule", usable) {
                        choice = Some(true);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(false);
                    }
                });
            });
        });
    choice
}

/// [`pattern_error`], kept between frames so a pattern is compiled once
/// while it stays the same, not on every frame.
fn cached_pattern_error(ctx: &egui::Context, pattern: &str) -> Option<String> {
    ctx.memory_mut(|memory| memory.caches.cache::<PatternErrorCache>().get(pattern).clone())
}

type PatternErrorCache = FrameCache<Option<String>, PatternChecker>;

/// Checks patterns for the cache.
#[derive(Default)]
struct PatternChecker;

impl ComputerMut<&str, Option<String>> for PatternChecker {
    fn compute(&mut self, pattern: &str) -> Option<String> {
        pattern_error(pattern)
    }
}
