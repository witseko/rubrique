//! Smart scratch names: after a scratch with its random name is saved, the
//! "Summaries and names" tool suggests a name from its content and shows it
//! in a bar. The Rename dialog can ask for a name too.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText, Ui};

use super::{OpenFile, RubriqueApp};
use crate::ai::naming::{name_prompt, name_slug, suggested_file_name};
use crate::ai::run::{AiRun, RunEnd};
use crate::ai::{self, AiTool};
use crate::explorer::paths::name_text;
use crate::icons::{Icon, icon_label};
use crate::scratch::is_random_scratch_name;

/// Scratches shorter than this keep their random name for now.
const MIN_NAMEABLE_CHARS: usize = 40;

/// A suggestion in progress or waiting for an answer.
#[derive(Debug, Default)]
pub(super) struct NamingState {
    /// The run, the file it names, and whether the Rename dialog asked.
    running: Option<(PathBuf, AiRun, bool)>,
    /// A suggested name for a file, shown in the bar.
    pub(super) offer: Option<(PathBuf, String)>,
    /// Files whose offer was turned down this session.
    declined: HashSet<PathBuf>,
}

impl NamingState {
    /// The Rename dialog is waiting for a suggestion.
    pub(super) fn dialog_waiting(&self) -> bool {
        self.running.as_ref().is_some_and(|(_, _, for_dialog)| *for_dialog)
    }
}

/// What the user chose in the name bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NameChoice {
    Rename,
    NotNow,
}

impl RubriqueApp {
    /// After a save: suggest a name when the open file is a scratch that
    /// still has its random name and enough text to name.
    pub(super) fn maybe_suggest_scratch_name(&mut self) {
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        if document.is_encrypted() {
            return;
        }
        let path = document.path.clone();
        let in_scratches = self.settings.scratch_folder_or_default().as_deref() == path.parent();
        let nameable = in_scratches
            && is_random_scratch_name(&name_text(&path))
            && document.text.trim().chars().count() >= MIN_NAMEABLE_CHARS
            && !self.naming.declined.contains(&path)
            && self.naming.running.is_none()
            && self.naming.offer.as_ref().is_none_or(|(offered, _)| *offered != path);
        if nameable {
            let text = document.text.clone();
            // Quietly skipped when no tool can run (the bar is a bonus).
            let _ = self.start_naming(path, &text, false);
        }
    }

    /// Ask the summaries tool for a name for `path`, from `text`.
    pub(super) fn start_naming(&mut self, path: PathBuf, text: &str, for_dialog: bool) -> Result<(), String> {
        if crate::crypto::file_is_encrypted(&path) {
            return Err(super::privacy::AI_OFF_MESSAGE.to_string());
        }
        let Some(tool) = self.settings.ai.summary_tool else {
            return Err("Pick a tool under Summaries use in Settings → AI".to_string());
        };
        let folder = path.parent().map_or_else(|| PathBuf::from("/"), Path::to_path_buf);
        if tool != AiTool::Local && !self.settings.ai.is_allowed(&folder) {
            return Err(format!("{} may not run in this folder yet; use Ask here once", tool.name()));
        }
        let command = self.settings.ai.command(tool).to_string();
        if ai::find_program(&command).is_none() {
            return Err(format!("{} is not installed; install it in Settings → AI", tool.name()));
        }
        let (program, args) = ai::command_args(&command, &name_prompt(text)).ok_or("The command is empty")?;
        let repaint = self.repaint.clone();
        let run = AiRun::start(&program, &args, &folder, move || repaint.request_repaint())
            .map_err(|error| format!("Cannot start {program}: {error}"))?;
        self.naming.running = Some((path, run, for_dialog));
        Ok(())
    }

    /// Collect a finished suggestion: fill the Rename dialog, or offer it in the bar.
    pub(super) fn poll_naming(&mut self) {
        let Some((path, run, for_dialog)) = &mut self.naming.running else {
            return;
        };
        let Some(end) = run.poll().cloned() else {
            return;
        };
        let suggestion = match end {
            RunEnd::Exited { success: true, .. } => name_slug(&run.output_text()),
            _ => None,
        };
        let (path, for_dialog) = (path.clone(), *for_dialog);
        self.naming.running = None;
        let name = suggestion.map(|slug| suggested_file_name(&name_text(&path), &slug));
        match (name, for_dialog) {
            (Some(name), true) => {
                if let Some(renaming) = &mut self.renaming
                    && renaming.path == path
                {
                    renaming.name = name;
                    renaming.error = None;
                }
            }
            (Some(name), false) => self.naming.offer = Some((path, name)),
            (None, true) => {
                if let Some(renaming) = &mut self.renaming {
                    renaming.error = Some("No name came back; try again or type one".to_string());
                }
            }
            (None, false) => {}
        }
    }

    /// Act on the name bar.
    pub(super) fn apply_name_choice(&mut self, choice: NameChoice) {
        let Some((path, name)) = self.naming.offer.take() else {
            return;
        };
        match choice {
            NameChoice::Rename => {
                if let Err(error) = self.finish_rename(&path, &name) {
                    self.show_status(error);
                }
            }
            NameChoice::NotNow => {
                self.naming.declined.insert(path);
            }
        }
    }
}

/// "Suggested name: …" with Rename and Not now.
pub(super) fn name_offer_bar(ui: &mut Ui, name: &str) -> Option<NameChoice> {
    let mut choice = None;
    let fill = ui.visuals().hyperlink_color.gamma_multiply(0.12);
    egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            icon_label(ui, Icon::Sparkle);
            ui.label("Suggested name:");
            ui.label(RichText::new(name).monospace().strong());
            if ui.button("Rename").clicked() {
                choice = Some(NameChoice::Rename);
            }
            if ui.button("Not now").clicked() {
                choice = Some(NameChoice::NotNow);
            }
        });
    });
    choice
}
