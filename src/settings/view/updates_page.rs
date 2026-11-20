//! The Updates group: the daily check switch, "Check now" and its answer.

use eframe::egui::{self, RichText, Ui};

use super::SettingsAction;
use crate::settings::updates::{AutoCheck, UpdateSettings};
use crate::update::{CheckResult, Version};

/// The update settings to edit, and the state of the check.
#[derive(Debug)]
pub struct UpdateInfo<'a> {
    pub settings: &'a mut UpdateSettings,
    /// A check runs now.
    pub checking: bool,
    /// What the last check in this session found.
    pub result: Option<&'a CheckResult>,
}

/// The Updates group.
pub(super) fn updates_section(ui: &mut Ui, updates: UpdateInfo) -> Option<SettingsAction> {
    let UpdateInfo { settings, checking, result } = updates;
    let mut action = None;
    ui.label(RichText::new("Updates").strong());
    let mut daily = settings.auto_check == AutoCheck::On;
    if ui.checkbox(&mut daily, "Check for new versions once a day").changed() {
        settings.auto_check = if daily { AutoCheck::On } else { AutoCheck::Off };
    }
    ui.add(
        egui::Label::new(
            RichText::new(
                "Rubrique asks GitHub for the number of the newest release and tells you when there is a \
                 newer one. Nothing is downloaded or installed.",
            )
            .weak(),
        )
        .wrap(),
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(format!("You have Rubrique {}.", Version::current()));
        let button = ui.add_enabled(!checking, egui::Button::new("Check now"));
        if button.clicked() {
            action = Some(SettingsAction::CheckForUpdates);
        }
    });
    if checking {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("Checking…").weak());
        });
        return action;
    }
    match result {
        Some(CheckResult::UpToDate) => {
            ui.label(RichText::new("This is the newest version.").weak());
        }
        Some(CheckResult::NewVersion(version)) => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Rubrique {version} is available.")).strong());
                if ui.button("Open release page").clicked() {
                    action = Some(SettingsAction::OpenReleasePage(*version));
                }
            });
        }
        Some(CheckResult::Failed(reason)) => {
            let text = format!("Could not check: {reason}");
            ui.add(egui::Label::new(RichText::new(text).color(ui.visuals().warn_fg_color)).wrap());
        }
        None => {}
    }
    action
}
