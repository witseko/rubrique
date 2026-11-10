//! Settings → Highlights: the rules that color matches in plain text files,
//! one editable row each (switch, name, color, pattern), and a field that
//! asks the AI to write a rule from a description.

use eframe::egui::cache::{ComputerMut, FrameCache};
use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Ui, vec2};

use super::{ROW_MARGIN, SMALL_SIZE, SettingsAction, enabled_button, list_frame, section_title};
use crate::document::highlighting::HighlightStyle;
use crate::highlight::text_rules::{HighlightRule, RuleColor, pattern_error};
use crate::icons::{Icon, small_icon_button};
use crate::settings::text_highlight::TextHighlightSettings;
use crate::theme::palette::Rgb;

/// Width of a rule's name field.
const NAME_WIDTH: f32 = 150.0;
/// Width of the color drop-down.
const COLOR_PICKER_WIDTH: f32 = 110.0;
/// Side of the color square before a color's name.
const SWATCH_SIZE: f32 = 12.0;
const SWATCH_RADIUS: u8 = 3;
/// Side of the small remove button on a rule row.
const REMOVE_SIZE: f32 = 18.0;
/// Room kept beside the description field for its button.
const WRITE_BUTTON_WIDTH: f32 = 70.0;

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

/// What a click on a rule row's buttons asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowEdit {
    MoveUp,
    MoveDown,
    Remove,
}

/// The Highlights group. Returns what the user clicked.
pub fn highlights_section(ui: &mut Ui, info: HighlightInfo<'_>) -> Option<SettingsAction> {
    let HighlightInfo { settings, description, draft, writer, tool_name } = info;
    section_title(
        ui,
        "Text highlights",
        "Color matches in plain text files (.txt, .log and other files without code colors). Rules are \
         tried top to bottom, and the first one that matches a piece of text colors it.",
    );
    let style = HighlightStyle::from_ui(ui);
    rule_list(ui, &mut settings.rules, &style);
    if ui.button("Add rule").clicked() {
        settings.rules.push(HighlightRule::new("New rule", "", RuleColor::Keyword));
    }

    ui.add_space(10.0);
    section_title(
        ui,
        "Write a rule with AI",
        &format!("Describe what to highlight and {tool_name} writes the pattern. You check it before it is added."),
    );
    let action = writer_field(ui, description, writer);
    if let Some(rule) = draft {
        match draft_card(ui, rule, &style) {
            Some(true) => settings.rules.extend(draft.take()),
            Some(false) => *draft = None,
            None => {}
        }
    }
    action
}

/// Every rule in a box, one row each; applies the row buttons' moves and removals.
fn rule_list(ui: &mut Ui, rules: &mut Vec<HighlightRule>, style: &HighlightStyle) {
    if rules.is_empty() {
        ui.label(RichText::new("No rules: plain text is not colored.").weak());
        return;
    }
    let mut edit = None;
    let count = rules.len();
    list_frame(ui, |ui| {
        for (index, rule) in rules.iter_mut().enumerate() {
            if index > 0 {
                ui.add(egui::Separator::default().spacing(0.0));
            }
            ui.push_id(("highlight-rule", index), |ui| {
                if let Some(clicked) = rule_row(ui, rule, style, (index > 0, index + 1 < count)) {
                    edit = Some((index, clicked));
                }
            });
        }
    });
    match edit {
        Some((index, RowEdit::MoveUp)) => rules.swap(index - 1, index),
        Some((index, RowEdit::MoveDown)) => rules.swap(index, index + 1),
        Some((index, RowEdit::Remove)) => {
            rules.remove(index);
        }
        None => {}
    }
}

/// One rule: its switch, name, color and the move and remove buttons, then
/// its pattern, then why the pattern does not work (when it does not).
/// `can_move` says whether it can go up and down.
fn rule_row(ui: &mut Ui, rule: &mut HighlightRule, style: &HighlightStyle, can_move: (bool, bool)) -> Option<RowEdit> {
    let mut edit = None;
    egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal(|ui| {
            ui.checkbox(&mut rule.enabled, "").on_hover_text("Use this rule");
            ui.add(egui::TextEdit::singleline(&mut rule.name).desired_width(NAME_WIDTH).hint_text("Name"));
            color_picker(ui, &mut rule.color, style);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if small_icon_button(ui, Icon::Close, REMOVE_SIZE, "Remove rule", "Remove this rule").clicked() {
                    edit = Some(RowEdit::Remove);
                }
                if enabled_button(ui, "↓", can_move.1) {
                    edit = Some(RowEdit::MoveDown);
                }
                if enabled_button(ui, "↑", can_move.0) {
                    edit = Some(RowEdit::MoveUp);
                }
            });
        });
        pattern_field(ui, &mut rule.pattern);
    });
    edit
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

/// A drop-down of the code colors, each with its square, and Custom; a
/// custom color gets a color button beside it.
fn color_picker(ui: &mut Ui, color: &mut RuleColor, style: &HighlightStyle) {
    swatch(ui, style.rule_color(*color));
    egui::ComboBox::from_id_salt("rule-color").selected_text(color.name()).width(COLOR_PICKER_WIDTH).show_ui(
        ui,
        |ui| {
            for each in RuleColor::PALETTE {
                ui.horizontal(|ui| {
                    swatch(ui, style.rule_color(each));
                    ui.selectable_value(color, each, each.name());
                });
            }
            ui.separator();
            let custom = matches!(color, RuleColor::Custom(_));
            if ui.selectable_label(custom, "Custom").clicked() && !custom {
                // Start from the color shown now, so the pick is a small change.
                let [red, green, blue, _] = style.rule_color(*color).to_array();
                *color = RuleColor::Custom(Rgb(red, green, blue));
            }
        },
    );
    if let RuleColor::Custom(Rgb(red, green, blue)) = color {
        let mut picked = [*red, *green, *blue];
        if egui::color_picker::color_edit_button_srgb(ui, &mut picked).changed() {
            *color = RuleColor::Custom(Rgb(picked[0], picked[1], picked[2]));
        }
    }
}

/// A small filled square of `color`.
fn swatch(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(SWATCH_SIZE, SWATCH_SIZE), Sense::hover());
    ui.painter().rect_filled(rect, SWATCH_RADIUS, color);
}

/// The description field with its Write button (Stop while the AI runs),
/// and the status under it.
fn writer_field(ui: &mut Ui, description: &mut String, writer: RuleWriter) -> Option<SettingsAction> {
    let mut action = None;
    let writing = matches!(writer, RuleWriter::Writing(_));
    ui.horizontal(|ui| {
        let field = egui::TextEdit::singleline(description)
            .desired_width(ui.available_width() - WRITE_BUTTON_WIDTH)
            .hint_text("Describe what to highlight, like \"ticket ids such as PROJ-123\"");
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
    });
    match writer {
        RuleWriter::Idle => {}
        RuleWriter::Writing(seconds) => {
            ui.label(RichText::new(format!("Writing a pattern… {}", crate::ai::format_elapsed(seconds))).weak());
        }
        RuleWriter::Failed(error) => {
            ui.label(RichText::new(error).size(SMALL_SIZE).color(ui.visuals().warn_fg_color));
        }
    }
    action
}

/// The rule the AI wrote, to edit and check: `Some(true)` adds it,
/// `Some(false)` discards it.
fn draft_card(ui: &mut Ui, rule: &mut HighlightRule, style: &HighlightStyle) -> Option<bool> {
    let mut choice = None;
    let fill = ui.visuals().hyperlink_color.gamma_multiply(0.12);
    egui::Frame::new().fill(fill).corner_radius(6).inner_margin(ROW_MARGIN).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.push_id("highlight-draft", |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut rule.name).desired_width(NAME_WIDTH).hint_text("Name"));
                color_picker(ui, &mut rule.color, style);
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
