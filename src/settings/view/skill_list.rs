//! Settings → Skills: the folders skills come from, each switched on or off,
//! and every loaded skill in one list to filter, read and open.

use std::path::Path;

use eframe::egui::{self, Align, Color32, CursorIcon, Layout, Margin, RichText, Sense, Ui};

use super::{SMALL_SIZE, SettingsAction, home_relative, path_box};
use crate::ai::AiSettings;
use crate::ai::skills::{Skill, SkillSource, SourceFolder};

/// Lines of a skill's instructions shown when its row is open.
const PREVIEW_LINES: usize = 6;
/// Corner radius of the boxes around the source list and skill list.
const LIST_RADIUS: u8 = 8;
/// Corner radius of the small source badges.
const BADGE_RADIUS: u8 = 4;

/// What the list shows, kept in egui's memory between frames.
#[derive(Clone, Debug, Default)]
struct ListState {
    /// Text typed in the filter field.
    query: String,
    /// Only skills from this source; `None` shows all.
    source: Option<SkillSource>,
    /// The name of the skill whose row is open.
    open: Option<String>,
}

/// Everything the Skills group shows.
#[derive(Debug)]
pub struct SkillsInfo<'a> {
    pub settings: &'a mut AiSettings,
    /// Loaded skills, by name.
    pub skills: &'a [Skill],
    /// Every source's folder in reading order, switched on or not.
    pub folders: &'a [SourceFolder],
}

/// The Skills group. Returns what the user clicked.
pub fn skills_section(ui: &mut Ui, info: SkillsInfo<'_>) -> Option<SettingsAction> {
    let SkillsInfo { settings, skills, folders } = info;
    let mut action = None;
    ui.label(RichText::new("Skills").strong());
    ui.label(
        RichText::new(
            "Reusable instructions for Ask (Cmd+K), one folder per skill with a SKILL.md file. \
             The same format works in Claude Code, Codex, Gemini CLI and Grok.",
        )
        .weak(),
    );
    ui.add_space(6.0);
    ui.label(RichText::new("Sources").strong());
    ui.label(
        RichText::new("Read top to bottom. When two folders have a skill with the same name, the first one wins.")
            .size(SMALL_SIZE)
            .weak(),
    );
    if let Some(picked) = sources_list(ui, settings, skills, folders) {
        action = Some(picked);
    }
    ui.add_space(10.0);
    let state_id = egui::Id::unique("settings-skills-list");
    let mut state: ListState = ui.data_mut(|data| data.get_temp(state_id)).unwrap_or_default();
    if let Some(picked) = list_header(ui, skills, &mut state, folders) {
        action = Some(picked);
    }
    if let Some(picked) = skill_list(ui, skills, &mut state) {
        action = Some(picked);
    }
    ui.data_mut(|data| data.insert_temp(state_id, state));
    action
}

/// One row per source: its switch, name, folder and how many skills it gave.
fn sources_list(
    ui: &mut Ui,
    settings: &mut AiSettings,
    skills: &[Skill],
    folders: &[SourceFolder],
) -> Option<SettingsAction> {
    let mut action = None;
    list_frame(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for (index, folder) in folders.iter().enumerate() {
            if index > 0 {
                ui.separator();
            }
            if let Some(picked) = source_row(ui, settings, skills, folder) {
                action = Some(picked);
            }
        }
    });
    action
}

fn source_row(
    ui: &mut Ui,
    settings: &mut AiSettings,
    skills: &[Skill],
    folder: &SourceFolder,
) -> Option<SettingsAction> {
    let mut action = None;
    let is_on = settings.uses_skill_source(folder.source);
    ui.horizontal(|ui| {
        if folder.source == SkillSource::Rubrique {
            ui.label(RichText::new(folder.source.name()).strong());
            ui.label(RichText::new("your folder, always on").size(SMALL_SIZE).weak());
        } else {
            let mut on = is_on;
            if ui.checkbox(&mut on, RichText::new(folder.source.name()).strong()).changed() {
                settings.set_skill_source(folder.source, on);
                action = Some(SettingsAction::ReloadSkills);
            }
        }
        if folder.source == SkillSource::Agents {
            ui.label(RichText::new("shared, read by Codex too").size(SMALL_SIZE).weak());
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(source_count(skills, folder, is_on));
        });
    });
    let path = home_relative(&folder.path);
    ui.add(egui::Label::new(RichText::new(&path).monospace().size(12.0).weak()).truncate())
        .on_hover_text(folder.path.display().to_string());
    if folder.source == SkillSource::Rubrique {
        ui.horizontal(|ui| {
            if ui.button("Change…").clicked() {
                action = Some(SettingsAction::ChooseSkillsFolder);
            }
            if ui.add_enabled(settings.skills_folder.is_some(), egui::Button::new("Use default")).clicked() {
                action = Some(SettingsAction::ResetSkillsFolder);
            }
        });
    }
    action
}

/// "12 skills", "Off" or "Not found" for a source.
fn source_count(skills: &[Skill], folder: &SourceFolder, is_on: bool) -> RichText {
    if !folder.exists {
        return RichText::new("Not found").size(SMALL_SIZE).weak();
    }
    if !is_on {
        return RichText::new("Off").size(SMALL_SIZE).weak();
    }
    let count = skills.iter().filter(|skill| skill.sources().any(|source| source == folder.source)).count();
    let noun = if count == 1 { "skill" } else { "skills" };
    RichText::new(format!("{count} {noun}")).size(SMALL_SIZE)
}

/// The "Loaded skills" title with Reload and Create example, then the filter
/// field and one button per source.
fn list_header(
    ui: &mut Ui,
    skills: &[Skill],
    state: &mut ListState,
    folders: &[SourceFolder],
) -> Option<SettingsAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Loaded skills · {}", skills.len())).strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Create example").on_hover_text("Adds an \"action-items\" skill to your folder").clicked() {
                action = Some(SettingsAction::CreateExampleSkill);
            }
            if ui.button("Reload").clicked() {
                action = Some(SettingsAction::ReloadSkills);
            }
        });
    });
    ui.add(
        egui::TextEdit::singleline(&mut state.query)
            .hint_text("Filter by name or description")
            .desired_width(f32::INFINITY),
    );
    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(state.source.is_none(), "All").clicked() {
            state.source = None;
        }
        // Only sources that gave at least one skill are worth a filter.
        for folder in folders {
            let source = folder.source;
            if !skills.iter().any(|skill| skill.sources().any(|each| each == source)) {
                continue;
            }
            if ui.selectable_label(state.source == Some(source), source.name()).clicked() {
                state.source = Some(source);
            }
        }
    });
    action
}

/// The skills that pass the filter, one row each.
fn skill_list(ui: &mut Ui, skills: &[Skill], state: &mut ListState) -> Option<SettingsAction> {
    let mut action = None;
    let shown: Vec<&Skill> = skills
        .iter()
        .filter(|skill| state.source.is_none_or(|source| skill.sources().any(|each| each == source)))
        .filter(|skill| skill.matches(&state.query))
        .collect();
    if skills.is_empty() {
        ui.label(RichText::new("No skills found yet. Create the example, or add a folder with a SKILL.md.").weak());
        return None;
    }
    if shown.is_empty() {
        ui.label(RichText::new("No skills match.").weak());
        return None;
    }
    list_frame(ui).inner_margin(Margin::same(0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, skill) in shown.iter().enumerate() {
            if index > 0 {
                ui.separator();
            }
            if let Some(picked) = skill_row(ui, skill, &mut state.open) {
                action = Some(picked);
            }
        }
    });
    action
}

/// One skill: a header that opens and closes the row, and the details when open.
fn skill_row(ui: &mut Ui, skill: &Skill, open: &mut Option<String>) -> Option<SettingsAction> {
    let is_open = open.as_deref() == Some(skill.name.as_str());
    let fill = if is_open { ui.visuals().panel_fill } else { Color32::TRANSPARENT };
    let mut action = None;
    egui::Frame::new().fill(fill).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        let header = ui.vertical(|ui| skill_header(ui, skill, is_open)).response;
        let click = ui.interact(header.rect, egui::Id::unique(("settings-skill-row", &skill.name)), Sense::click());
        if click.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            *open = if is_open { None } else { Some(skill.name.clone()) };
        }
        if is_open {
            action = skill_details(ui, skill);
        }
    });
    action
}

/// Name, source badges, "answer" when it gives one, and the description.
fn skill_header(ui: &mut Ui, skill: &Skill, is_open: bool) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(&skill.name).monospace().strong());
        for source in skill.sources() {
            badge(ui, source.name());
        }
        if skill.kind == crate::ai::AskKind::Answer {
            badge(ui, "answer");
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(if is_open { "▾" } else { "▸" }).weak());
        });
    });
    let description = egui::Label::new(RichText::new(&skill.description).size(SMALL_SIZE).weak());
    if is_open {
        ui.add(description.wrap());
    } else {
        ui.add(description.truncate());
    }
}

fn badge(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.weak_bg_fill)
        .corner_radius(BADGE_RADIUS)
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(SMALL_SIZE));
        });
}

/// The open row: where the file is, other copies, the start of the
/// instructions, and the Open and Show folder buttons.
fn skill_details(ui: &mut Ui, skill: &Skill) -> Option<SettingsAction> {
    let mut action = None;
    ui.add_space(2.0);
    path_box(ui, &skill.path);
    for (source, path) in &skill.linked {
        detail_line(ui, &format!("Also in {}", source.name()), path);
    }
    for (source, path) in &skill.skipped {
        detail_line(ui, &format!("Skipped, same name in {}", source.name()), path);
    }
    let what = match skill.kind {
        crate::ai::AskKind::Answer => "Gives an answer to read (output: answer).",
        crate::ai::AskKind::Rewrite => "Gives new text for the selection, reviewed in the merge screen.",
    };
    ui.label(RichText::new(what).size(SMALL_SIZE).weak());
    egui::Frame::new()
        .fill(ui.visuals().code_bg_color)
        .corner_radius(BADGE_RADIUS)
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(instructions_preview(&skill.instructions)).monospace().size(12.0));
        });
    ui.horizontal(|ui| {
        if ui.button("Open SKILL.md").clicked() {
            action = Some(SettingsAction::OpenFile(skill.path.clone()));
        }
        if let Some(folder) = skill.path.parent()
            && ui.button("Show folder").clicked()
        {
            action = Some(SettingsAction::ShowFolder(folder.to_path_buf()));
        }
    });
    ui.add_space(4.0);
    action
}

/// "Also in Claude Code: ~/.claude/skills/x/SKILL.md", in small text.
fn detail_line(ui: &mut Ui, label: &str, path: &Path) {
    let text = format!("{label}: {}", home_relative(path));
    ui.add(egui::Label::new(RichText::new(text).size(SMALL_SIZE).weak()).wrap());
}

/// The first lines of the instructions, with "…" when there are more.
fn instructions_preview(instructions: &str) -> String {
    let mut lines = instructions.lines();
    let mut preview: Vec<&str> = lines.by_ref().take(PREVIEW_LINES).collect();
    if lines.next().is_some() {
        preview.push("…");
    }
    preview.join("\n")
}

/// The thin rounded box around the source list and the skill list.
fn list_frame(ui: &Ui) -> egui::Frame {
    egui::Frame::new()
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(LIST_RADIUS)
        .inner_margin(Margin::symmetric(10, 8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_keeps_the_first_lines() {
        assert_eq!(instructions_preview("one\ntwo"), "one\ntwo");
        let long = (1..=8).map(|line| line.to_string()).collect::<Vec<_>>().join("\n");
        assert_eq!(instructions_preview(&long), "1\n2\n3\n4\n5\n6\n…");
    }
}
