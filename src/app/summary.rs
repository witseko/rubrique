//! Hover summaries: rest the pointer on a file in the explorer and a small
//! card shows a two-sentence summary made by the "Summaries and names" tool.
//! Summaries are cached by file content.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use eframe::egui::{self, Context, RichText, pos2};

use super::RubriqueApp;
use crate::ai::run::{AiRun, RunEnd};
use crate::ai::summary::{
    cache_key, clean_summary, edited_ago, model_label, read_cached, summary_prompt, write_cached,
};
use crate::ai::{self, AiTool};
use crate::explorer::view::{HoveredFile, hovered_file};
use crate::icons::{Icon, icon_label};
use crate::markdown::render::render_inline_text;
use crate::settings::summary_cache_folder;

/// Seconds the pointer rests on a file before its summary shows.
const HOVER_DELAY: f64 = 0.5;
const CARD_WIDTH: f32 = 290.0;
/// Where the card sits relative to the row's bottom left corner.
const CARD_OFFSET: egui::Vec2 = egui::vec2(36.0, 4.0);
/// Larger files are not summarized.
const MAX_FILE_BYTES: u64 = 2_000_000;
const FOOTER_SIZE: f32 = 11.5;

/// Summaries made, failed and running.
#[derive(Debug, Default)]
pub(super) struct SummaryState {
    /// The hovered file and when the pointer arrived on it.
    hover: Option<(PathBuf, f64)>,
    /// Cache key per file, valid while the file and the command stay the same.
    keys: HashMap<PathBuf, (SystemTime, String, String)>,
    /// Summary per cache key.
    done: HashMap<String, String>,
    /// Why a summary could not be made, per cache key (not retried).
    failed: HashMap<String, String>,
    /// The one summary being made, with its cache key.
    running: Option<(String, AiRun)>,
}

/// What the card says.
enum CardBody {
    Summary(String),
    Working(String),
    Message(String),
}

impl RubriqueApp {
    /// Show the summary card for the hovered explorer file, starting a run
    /// when needed. Call once per frame, after the explorer is drawn.
    pub(super) fn hover_summary(&mut self, ctx: &Context) {
        self.poll_summary();
        let hovered = hovered_file(ctx);
        let (Some(tool), Some(file)) = (self.settings.ai.summary_tool, hovered) else {
            self.summaries.hover = None;
            return;
        };
        let now = ctx.input(|input| input.time);
        let since = match &self.summaries.hover {
            Some((path, since)) if *path == file.path => *since,
            _ => {
                self.summaries.hover = Some((file.path.clone(), now));
                now
            }
        };
        if now - since < HOVER_DELAY {
            ctx.request_repaint_after(Duration::from_secs_f64(HOVER_DELAY - (now - since)));
            return;
        }
        let command = self.settings.ai.command(tool).to_string();
        let Some((key, modified)) = self.summary_key(&file.path, &command) else {
            return;
        };
        let body = self.summary_body(tool, &command, &key, &file.path);
        let footer = matches!(body, CardBody::Summary(_)).then(|| {
            let edited = edited_ago(modified, SystemTime::now());
            (model_label(tool, &command), edited)
        });
        show_card(ctx, &file, &body, footer);
    }

    /// The cache key for `path`, reading the file only when it changed.
    fn summary_key(&mut self, path: &Path, command: &str) -> Option<(String, SystemTime)> {
        let metadata = std::fs::metadata(path).ok()?;
        // Encrypted scratches are never summarized (nor cached).
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES || crate::crypto::file_is_encrypted(path) {
            return None;
        }
        let modified = metadata.modified().ok()?;
        if let Some((known, known_command, key)) = self.summaries.keys.get(path)
            && *known == modified
            && known_command == command
        {
            return Some((key.clone(), modified));
        }
        let text = std::fs::read_to_string(path).ok().filter(|text| !text.trim().is_empty())?;
        let key = cache_key(command, &text);
        self.summaries.keys.insert(path.to_path_buf(), (modified, command.to_string(), key.clone()));
        Some((key, modified))
    }

    /// The summary, or what is happening instead; starts a run when needed.
    fn summary_body(&mut self, tool: AiTool, command: &str, key: &str, path: &Path) -> CardBody {
        let state = &mut self.summaries;
        if let Some(summary) = state.done.get(key) {
            return CardBody::Summary(summary.clone());
        }
        if let Some(summary) = summary_cache_folder().and_then(|folder| read_cached(&folder, key)) {
            state.done.insert(key.to_string(), summary.clone());
            return CardBody::Summary(summary);
        }
        if let Some(message) = state.failed.get(key) {
            return CardBody::Message(message.clone());
        }
        let working = if tool == AiTool::Local {
            "Summarizing on this computer…".to_string()
        } else {
            format!("Summarizing with {}…", tool.name())
        };
        if state.running.is_some() {
            // One run at a time; this file's turn comes when the pointer stays.
            return CardBody::Working(working);
        }
        let folder = path.parent().map_or_else(|| PathBuf::from("/"), Path::to_path_buf);
        if tool != AiTool::Local && !self.settings.ai.is_allowed(&folder) {
            return CardBody::Message(format!(
                "Summaries with {} run only in folders where you allowed AI. Use Ask here once, \
                 or pick Local model in Settings → AI.",
                tool.name()
            ));
        }
        match self.start_summary(tool, command, key, path, &folder) {
            Ok(()) => CardBody::Working(working),
            Err(message) => {
                self.summaries.failed.insert(key.to_string(), message.clone());
                CardBody::Message(message)
            }
        }
    }

    fn start_summary(
        &mut self,
        tool: AiTool,
        command: &str,
        key: &str,
        path: &Path,
        folder: &Path,
    ) -> Result<(), String> {
        if ai::find_program(command).is_none() {
            return Err(format!("{} is not installed. Install it in Settings → AI.", tool.name()));
        }
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let prompt = summary_prompt(&name, &text);
        let (program, args) = ai::command_args(command, &prompt).ok_or("The command is empty.")?;
        let repaint = self.repaint.clone();
        let run = AiRun::start(&program, &args, folder, move || repaint.request_repaint())
            .map_err(|error| format!("Cannot start {program}: {error}"))?;
        self.summaries.running = Some((key.to_string(), run));
        Ok(())
    }

    /// Collect a finished summary.
    fn poll_summary(&mut self) {
        let Some((key, run)) = &mut self.summaries.running else {
            return;
        };
        let Some(end) = run.poll().cloned() else {
            return;
        };
        let summary = clean_summary(&run.output_text());
        let key = key.clone();
        match end {
            RunEnd::Exited { success: true, .. } if !summary.is_empty() => {
                if let Some(folder) = summary_cache_folder() {
                    write_cached(&folder, &key, &summary);
                }
                self.summaries.done.insert(key, summary);
            }
            _ => {
                let errors = run.error_text();
                let reason = errors.lines().find(|line| !line.trim().is_empty()).unwrap_or("The tool gave no summary");
                self.summaries.failed.insert(key, format!("Could not summarize: {}", reason.trim()));
            }
        }
        self.summaries.running = None;
    }
}

/// The card next to the hovered row.
fn show_card(ctx: &Context, file: &HoveredFile, body: &CardBody, footer: Option<(String, String)>) {
    let at = pos2(file.rect.left(), file.rect.bottom()) + CARD_OFFSET;
    egui::Area::new(egui::Id::unique("hover-summary")).order(egui::Order::Tooltip).fixed_pos(at).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
            ui.set_width(CARD_WIDTH);
            ui.spacing_mut().item_spacing.y = 6.0;
            let name = file.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            ui.label(RichText::new(name).strong());
            match body {
                CardBody::Summary(summary) => {
                    render_inline_text(ui, summary);
                }
                CardBody::Working(text) => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(11.0));
                        ui.label(RichText::new(text).weak());
                    });
                }
                CardBody::Message(text) => {
                    ui.label(RichText::new(text).weak());
                }
            }
            if let Some((model, edited)) = footer {
                ui.separator();
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Sparkle);
                    ui.label(RichText::new(model).size(FOOTER_SIZE).weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(edited).size(FOOTER_SIZE).weak());
                    });
                });
            }
        });
    });
}
