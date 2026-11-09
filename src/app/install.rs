//! Installing a missing tool from the app: which recipes apply, the dialog,
//! and the install run in the background.

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, Context};

use super::RubriqueApp;
use crate::ai::run::{AiRun, RunEnd};
use crate::ai::{self, AiTool};
use crate::install::view::{DialogChoice, InstallAction, Offer, install_dialog};
use crate::install::{
    InstallStatus, Platform, Progress, Recipe, find_installer, installed_message, last_line, model_recipe,
    ollama_model, ollama_model_present, ollama_pull_recipe, recipe_for_command, tmux_recipe,
};
use crate::terminal::monitor::TerminalMonitor;
use crate::terminal::tmux::Tmux;
use crate::voice::MODEL_URL;

/// Recipes for what is missing, the dialog, and the install that runs.
#[derive(Debug, Default)]
pub(super) struct InstallState {
    /// The dialog's offer, while it is open.
    offer: Option<Offer>,
    running: Option<(AiRun, Recipe)>,
    /// How the last install went, with the recipe's name.
    last: Option<(String, Progress)>,
    /// For each AI tool that was not found, in [`AiTool::ALL`] order.
    pub(super) ai: [Option<Recipe>; 4],
    /// The local model's download, when Ollama is there but the model is not.
    pub(super) local_model: Option<Recipe>,
    /// The transcription program, when it was not found.
    pub(super) voice: Option<Recipe>,
    /// The whisper model, when it is missing.
    pub(super) voice_model: Option<Recipe>,
    /// tmux, when it was not found (Terminals are off without it).
    pub(super) tmux: Option<Recipe>,
}

impl RubriqueApp {
    /// Recipes for the AI tools that were not found, and the local model.
    pub(super) fn find_ai_recipes(&mut self) {
        let platform = Platform::current();
        for (index, tool) in AiTool::ALL.into_iter().enumerate() {
            let command = self.settings.ai.command(tool);
            self.install.ai[index] = self.ai_found[index]
                .is_none()
                .then(|| recipe_for_command(command, platform, &find_installer))
                .flatten();
        }
        let local = AiTool::ALL.iter().position(|tool| *tool == AiTool::Local).unwrap_or_default();
        self.install.local_model = self.ai_found[local].as_deref().and_then(|ollama| {
            let model = ollama_model(self.settings.ai.command(AiTool::Local))?;
            (!ollama_model_present(&model)).then(|| ollama_pull_recipe(ollama, &model))
        });
    }

    /// Look for the transcription program and its model, and their recipes.
    pub(super) fn check_voice_tool(&mut self) {
        let voice = &self.settings.voice;
        self.voice.program_found = voice.find_program();
        self.install.voice = self
            .voice
            .program_found
            .is_none()
            .then(|| recipe_for_command(&voice.command, Platform::current(), &find_installer))
            .flatten();
        self.install.voice_model = voice
            .model_download_command()
            .and(voice.model_path())
            .filter(|_| voice.model_missing())
            .map(|model| model_recipe(&model, MODEL_URL, &find_installer));
    }

    /// Look for tmux when Terminals are off, and turn them on once it is
    /// there, so installing it needs no restart. Its recipe while missing.
    pub(super) fn check_terminals(&mut self) {
        if self.terminals.is_none() {
            self.terminals = Tmux::find().map(|tmux| TerminalMonitor::start(tmux, &self.repaint));
        }
        self.install.tmux = self.terminals.is_none().then(|| tmux_recipe(Platform::current(), &find_installer));
    }

    /// Settings' Check again: look for tmux and say what was found.
    pub(super) fn check_terminals_again(&mut self) {
        self.check_terminals();
        let status = if self.terminals.is_some() { "Found tmux: Terminals are on" } else { "tmux is still not found" };
        self.show_status(status.to_string());
    }

    /// The install that runs or ran last.
    pub(super) fn install_status(&self) -> InstallStatus {
        let progress = match &self.install.running {
            Some((run, recipe)) => Some((recipe.name.clone(), Progress::Running { seconds: run.elapsed_seconds() })),
            None => self.install.last.clone(),
        };
        InstallStatus { progress, busy: self.install.running.is_some() }
    }

    /// Open the dialog for something that was needed just now.
    pub(super) fn offer_install(&mut self, message: String, recipe: Recipe) {
        if self.install.last.as_ref().is_some_and(|(name, _)| *name == recipe.name) {
            self.install.last = None;
        }
        self.install.offer = Some(Offer { message, recipe });
    }

    /// Offer to install `tool`'s program when it is missing; false when it is there.
    pub(super) fn offer_missing_ai_tool(&mut self, tool: AiTool) -> bool {
        let command = self.settings.ai.command(tool).to_string();
        if ai::find_program(&command).is_some() {
            return false;
        }
        match recipe_for_command(&command, Platform::current(), &find_installer) {
            Some(recipe) => {
                let message = format!("{} is not installed, or not on PATH.", recipe.name);
                self.offer_install(message, recipe);
            }
            None => self.show_status(format!("{} was not found. Check its command in Settings → AI.", tool.name())),
        }
        true
    }

    /// A click in an install prompt.
    pub(super) fn install_action(&mut self, ctx: &Context, action: InstallAction) {
        match action {
            InstallAction::Install(recipe) => self.start_install(*recipe),
            InstallAction::Copy(command) => {
                ctx.copy_text(command);
                self.show_status("Copied the command; paste it into a terminal".to_string());
            }
            InstallAction::OpenPage(page) => ctx.open_url(egui::OpenUrl::new_tab(page)),
        }
    }

    /// Run the recipe's command in the background (only after the user's click).
    fn start_install(&mut self, recipe: Recipe) {
        if self.install.running.is_some() {
            self.show_status("Wait for the install to finish".to_string());
            return;
        }
        let Some(command) = &recipe.run else {
            return;
        };
        let folder = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
        let repaint = self.repaint.clone();
        match AiRun::start(&command.program, &command.args, &folder, move || repaint.request_repaint()) {
            Ok(run) => {
                self.install.last = None;
                self.show_status(format!("Installing {}…", recipe.name));
                self.install.running = Some((run, recipe));
            }
            Err(error) => {
                let message = format!("Cannot start {}: {error}", command.program);
                self.install.last = Some((recipe.name.clone(), Progress::Failed(message.clone())));
                self.show_status(message);
            }
        }
    }

    /// Each frame: follow the install; when it ends, look for the tools again.
    pub(super) fn poll_install(&mut self, ctx: &Context) {
        let Some((run, _)) = &mut self.install.running else {
            return;
        };
        let Some(end) = run.poll().cloned() else {
            ctx.request_repaint_after(Duration::from_secs(1));
            return;
        };
        let Some((run, recipe)) = self.install.running.take() else {
            return;
        };
        let result = match end {
            RunEnd::Exited { success: true, .. } => finish_download(&recipe),
            RunEnd::Exited { code, .. } => {
                discard_download(&recipe);
                let code = code.map(|code| format!(" (exit code {code})")).unwrap_or_default();
                Err(format!("Install failed{code}: {}", last_line(&run.error_text(), &run.output_text())))
            }
            RunEnd::Stopped => {
                discard_download(&recipe);
                Err("Install stopped".to_string())
            }
        };
        self.check_ai_tools();
        self.check_voice_tool();
        self.check_terminals();
        let progress = match result {
            Ok(()) => {
                let found = recipe.provides.as_deref().is_none_or(|program| ai::find_on_path(program).is_some());
                Progress::Done(installed_message(&recipe, found))
            }
            Err(message) => Progress::Failed(message),
        };
        if let Progress::Done(message) | Progress::Failed(message) = &progress {
            self.show_status(message.clone());
        }
        self.install.last = Some((recipe.name, progress));
    }

    /// The install dialog, while it is open.
    pub(super) fn install_dialog(&mut self, ctx: &Context) {
        let Some(offer) = &self.install.offer else {
            return;
        };
        let status = self.install_status();
        match install_dialog(ctx, offer, status.of(&offer.recipe)) {
            Some(DialogChoice::Action(action)) => self.install_action(ctx, action),
            Some(DialogChoice::Close) => self.install.offer = None,
            None => {}
        }
    }
}

/// Put a finished download in place.
fn finish_download(recipe: &Recipe) -> Result<(), String> {
    match &recipe.download {
        Some((partial, target)) => {
            std::fs::rename(partial, target).map_err(|error| format!("Cannot move {}: {error}", partial.display()))
        }
        None => Ok(()),
    }
}

/// Remove what a broken download left.
fn discard_download(recipe: &Recipe) {
    if let Some((partial, _)) = &recipe.download
        && Path::new(partial).is_file()
    {
        let _ = std::fs::remove_file(partial);
    }
}
