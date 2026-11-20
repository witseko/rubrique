//! The update check in the app: the daily check in the background, the
//! card at the bottom of the explorer that asks first and later tells
//! about a new version, and the Updates group's buttons.

use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Context, RichText, Ui};

use super::RubriqueApp;
use crate::remote::task::{Task, spawn_with_repaint};
use crate::settings::updates::{AutoCheck, UpdateSettings};
use crate::settings::view::UpdateInfo;
use crate::update::{CheckResult, Version, check_due, check_latest};

/// Space inside the explorer card.
const CARD_MARGIN: i8 = 10;
/// Corner rounding of the explorer card.
const CARD_RADIUS: u8 = 8;

/// The check that runs, and what the last one found.
#[derive(Debug, Default)]
pub(super) struct UpdateState {
    running: Option<Task<CheckResult>>,
    /// The last check's answer in this session.
    pub(super) result: Option<CheckResult>,
    /// The user opened the release page, so the card stays away until the
    /// next start.
    notice_closed: bool,
}

impl UpdateState {
    pub(super) fn is_checking(&self) -> bool {
        self.running.is_some()
    }

    /// What the Updates group in Settings shows, with `settings` to edit.
    pub(super) fn settings_info<'a>(&'a self, settings: &'a mut UpdateSettings) -> UpdateInfo<'a> {
        UpdateInfo { settings, checking: self.is_checking(), result: self.result.as_ref() }
    }
}

/// What the user clicked in the explorer card.
#[derive(Clone, Copy)]
enum CardChoice {
    TurnOn,
    TurnOff,
    Download(Version),
    Skip(Version),
}

impl RubriqueApp {
    /// Pick up a finished check, and start the daily one when it is due.
    pub(super) fn poll_update(&mut self, ctx: &Context) {
        if let Some(result) = self.update.running.as_ref().and_then(Task::take) {
            self.update.running = None;
            self.update.result = Some(result);
        }
        let now = seconds_since_1970();
        if self.settings.updates.auto_check == AutoCheck::On
            && !self.update.is_checking()
            && check_due(self.settings.updates.last_check, now)
        {
            // Recorded when the check starts, so a failed check waits a day
            // too instead of starting again on the next frame.
            self.settings.updates.last_check = Some(now);
            self.write_settings();
            self.start_update_check(ctx);
        }
    }

    /// Ask GitHub for the newest version in the background.
    pub(super) fn start_update_check(&mut self, ctx: &Context) {
        if self.update.is_checking() {
            return;
        }
        self.update.running = Some(spawn_with_repaint(ctx, || check_latest(Version::current())));
    }

    /// Whether the explorer card has something to show.
    pub(super) fn update_card_wanted(&self) -> bool {
        self.settings.updates.auto_check == AutoCheck::NotAsked || self.version_to_announce().is_some()
    }

    /// The card at the bottom of the explorer: the question before the
    /// first check, then the notice when a new version exists.
    pub(super) fn update_card(&mut self, ui: &mut Ui) {
        let frame = egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(CARD_RADIUS)
            .inner_margin(CARD_MARGIN);
        let choice = frame
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                match self.version_to_announce() {
                    Some(version) => new_version_card(ui, version),
                    None => question_card(ui),
                }
            })
            .inner;
        if let Some(choice) = choice {
            self.apply_card_choice(ui.ctx(), choice);
        }
    }

    /// Open the release page of `version` in the browser.
    pub(super) fn open_release_page(&mut self, ctx: &Context, version: Version) {
        ctx.open_url(egui::OpenUrl::new_tab(version.release_page()));
        self.update.notice_closed = true;
    }

    /// The new version the card tells about: found by the last check, not
    /// skipped, and the card not closed in this session.
    fn version_to_announce(&self) -> Option<Version> {
        if self.update.notice_closed {
            return None;
        }
        version_to_announce(self.update.result.as_ref(), self.settings.updates.skipped_version)
    }

    fn apply_card_choice(&mut self, ctx: &Context, choice: CardChoice) {
        match choice {
            // The check itself starts in `poll_update`, as it is due now.
            CardChoice::TurnOn => self.settings.updates.auto_check = AutoCheck::On,
            CardChoice::TurnOff => self.settings.updates.auto_check = AutoCheck::Off,
            CardChoice::Download(version) => self.open_release_page(ctx, version),
            CardChoice::Skip(version) => self.settings.updates.skipped_version = Some(version),
        }
        self.write_settings();
    }
}

/// `version` from a check result, unless the user skipped it.
fn version_to_announce(result: Option<&CheckResult>, skipped: Option<Version>) -> Option<Version> {
    match result {
        Some(CheckResult::NewVersion(version)) if skipped != Some(*version) => Some(*version),
        _ => None,
    }
}

/// "Check for new versions once a day?" with Yes and No.
fn question_card(ui: &mut Ui) -> Option<CardChoice> {
    ui.label(RichText::new("Check for new versions once a day?").strong());
    ui.label(RichText::new("Rubrique asks GitHub for the newest version number. Nothing is installed.").weak().small());
    let mut choice = None;
    ui.horizontal(|ui| {
        if ui.button("Yes").clicked() {
            choice = Some(CardChoice::TurnOn);
        }
        if ui.button("No").on_hover_text("You can check by hand in Settings, Updates").clicked() {
            choice = Some(CardChoice::TurnOff);
        }
    });
    choice
}

/// "Rubrique X.Y.Z is available" with Download and Skip.
fn new_version_card(ui: &mut Ui, version: Version) -> Option<CardChoice> {
    ui.label(RichText::new(format!("Rubrique {version} is available")).strong());
    ui.label(RichText::new(format!("You have {}.", Version::current())).weak().small());
    let mut choice = None;
    ui.horizontal(|ui| {
        if ui.button("Download").on_hover_text("Open the release page with the notes and downloads").clicked() {
            choice = Some(CardChoice::Download(version));
        }
        if ui.button("Skip").on_hover_text("No reminder for this version; a later one shows again").clicked() {
            choice = Some(CardChoice::Skip(version));
        }
    });
    choice
}

/// The clock in whole seconds; 0 if it is set before 1970.
fn seconds_since_1970() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announces_a_new_version_unless_skipped() {
        let new = Version::parse("9.0.0").unwrap();
        let found = CheckResult::NewVersion(new);
        assert_eq!(version_to_announce(Some(&found), None), Some(new));
        assert_eq!(version_to_announce(Some(&found), Version::parse("8.9.0")), Some(new));
        assert_eq!(version_to_announce(Some(&found), Some(new)), None);
        assert_eq!(version_to_announce(Some(&CheckResult::UpToDate), None), None);
        assert_eq!(version_to_announce(Some(&CheckResult::Failed("offline".to_string())), None), None);
        assert_eq!(version_to_announce(None, None), None);
    }
}
