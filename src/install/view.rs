//! The install prompt ("whisper.cpp is not installed. [Install] [Copy
//! command]"), in Settings and in a small dialog where a missing tool was
//! needed.

use eframe::egui::{self, Context, RichText, Ui};

use super::{Progress, Recipe};
use crate::ai::format_elapsed;
use crate::alert::{self, ButtonKind};

const DIALOG_WIDTH: f32 = 420.0;
/// Size of the prompt's text in Settings.
const SMALL_SIZE: f32 = 11.5;

/// What the user clicked in an install prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallAction {
    /// Run the recipe's command.
    Install(Box<Recipe>),
    /// Put this command on the clipboard.
    Copy(String),
    OpenPage(&'static str),
}

/// A missing tool, offered in the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    /// Why it is needed: "whisper.cpp is not installed."
    pub message: String,
    pub recipe: Recipe,
}

/// The prompt under a missing tool: `message`, then Install (with the exact
/// command on hover), Copy command and Install page, and how the install went.
/// `busy` disables Install while another install runs.
pub fn install_prompt(
    ui: &mut Ui,
    message: &str,
    recipe: &Recipe,
    progress: Option<&Progress>,
    busy: bool,
) -> Option<InstallAction> {
    let visuals = ui.visuals().clone();
    let mut action = None;
    if let Some(Progress::Done(text)) = progress {
        ui.label(RichText::new(text).size(SMALL_SIZE).weak());
        return None;
    }
    ui.label(RichText::new(message).size(SMALL_SIZE).color(visuals.warn_fg_color));
    if let Some(Progress::Running { seconds }) = progress {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0));
            let text = format!("Installing {}… {}", recipe.name, format_elapsed(*seconds));
            ui.label(RichText::new(text).size(SMALL_SIZE).weak());
        });
        return None;
    }
    ui.horizontal_wrapped(|ui| {
        if let Some(command) = &recipe.run {
            let tip = format!("Runs: {}", command.display());
            if ui.add_enabled(!busy, egui::Button::new("Install")).on_hover_text(tip).clicked() {
                action = Some(InstallAction::Install(Box::new(recipe.clone())));
            }
        }
        if let Some(command) = recipe.shown_command() {
            let label = if recipe.run.is_some() { "Copy command" } else { "Copy install command" };
            if ui.button(label).on_hover_text(RichText::new(&command).monospace()).clicked() {
                action = Some(InstallAction::Copy(command));
            }
        }
        if let Some(page) = recipe.page
            && ui.button("Open install page").on_hover_text(page).clicked()
        {
            action = Some(InstallAction::OpenPage(page));
        }
    });
    if recipe.run.is_none()
        && let Some(command) = &recipe.copy
    {
        ui.label(RichText::new(format!("Run in a terminal: {command}")).size(SMALL_SIZE).monospace().weak());
    }
    if let Some(Progress::Failed(text)) = progress {
        ui.label(RichText::new(text).size(SMALL_SIZE).color(visuals.error_fg_color));
    }
    action
}

/// The user's answer in the install dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogChoice {
    Action(InstallAction),
    Close,
}

/// A small dialog with the prompt, for a tool that was needed just now.
pub fn install_dialog(ctx: &Context, offer: &Offer, progress: Option<&Progress>) -> Option<DialogChoice> {
    let modal = alert::show(ctx, "install-offer", DIALOG_WIDTH, |ui| {
        alert::title(ui, &format!("Install {}?", offer.recipe.name));
        let mut choice = install_prompt(ui, &offer.message, &offer.recipe, progress, false).map(DialogChoice::Action);
        let done = matches!(progress, Some(Progress::Done(_)));
        if let Some(command) = offer.recipe.run.as_ref().map(super::InstallCommand::display).filter(|_| !done) {
            alert::body(ui, "Install runs this command:");
            egui::Frame::new()
                .fill(ui.visuals().code_bg_color)
                .corner_radius(5)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(command).monospace());
                });
        }
        let ((), close) = alert::button_row(
            ui,
            |_| (),
            |ui| {
                // Once the install is done, Done is the only thing left to do.
                let close = if done {
                    alert::button(ui, "Done", ButtonKind::Primary, true)
                } else {
                    alert::button(ui, "Close", ButtonKind::Secondary, false)
                };
                let running = matches!(progress, Some(Progress::Running { .. }));
                if running { close.on_hover_text("The install goes on in the background") } else { close }
            },
        );
        if close.clicked() || (done && alert::enter_for_default(ui)) {
            choice = Some(DialogChoice::Close);
        }
        choice
    });
    if modal.should_close() { Some(DialogChoice::Close) } else { modal.inner }
}
