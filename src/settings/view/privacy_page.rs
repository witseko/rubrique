//! Settings → Privacy: encrypted scratches and the app PIN.
//!
//! A card at the top says whether the scratches are locked and holds the
//! one action that matters now (choose a password, unlock, lock). Under it,
//! each option is one row of an outlined list: a short name, one line on
//! what it does, and its switch or button on the right. The App PIN part
//! uses the same rows.

use eframe::egui::{self, Align, Label, Layout, RichText, Sense, Ui, Vec2, vec2};

use super::highlight_rules::switch;
use super::{PrivacyInfo, ROW_MARGIN, SettingsAction, enabled_button, list_frame};
use crate::icons::paint_padlock;
use crate::settings::privacy::{LOCK_AFTER_CHOICES, PrivacySettings};

/// Width kept on the right of every row for its switch or buttons, so the
/// controls line up down the list.
const CONTROL_WIDTH: f32 = 180.0;
/// Gap between a row's text and its control.
const ROW_GAP: f32 = 16.0;
/// Size of the switches on the rows.
const SWITCH_SIZE: Vec2 = vec2(32.0, 18.0);
/// Width of the "Lock after" drop-down.
const LOCK_PICKER_WIDTH: f32 = 120.0;
/// Inner margin of the status card.
const CARD_MARGIN: i8 = 14;
/// Width of the padlock in the status card.
const CARD_ICON_SIZE: f32 = 22.0;
/// Corner radius and padding of the Locked / Unlocked chip.
const CHIP_RADIUS: u8 = 8;
const CHIP_SIZE: f32 = 11.0;
/// Space between the two parts of the page.
const PART_GAP: f32 = 18.0;

/// Where the scratch password stands, for the status card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VaultState {
    /// No password was chosen yet.
    NoPassword,
    Locked,
    Unlocked,
}

/// The Privacy group. Returns what the user clicked.
pub(super) fn privacy_section(ui: &mut Ui, privacy: PrivacyInfo) -> Option<SettingsAction> {
    let PrivacyInfo { settings, unlocked, touch_id_available } = privacy;
    let state = match (settings.vault.is_some(), unlocked) {
        (false, _) => VaultState::NoPassword,
        (true, false) => VaultState::Locked,
        (true, true) => VaultState::Unlocked,
    };
    let mut action = None;

    ui.label(RichText::new("Encrypted scratches").strong());
    action = action.or(status_card(ui, state));
    ui.add_space(8.0);
    action = action.or(scratch_rows(ui, settings, state, touch_id_available));
    ui.label(
        RichText::new("Any scratch can also be encrypted on its own: the lock in the status bar, or right-click it.")
            .weak()
            .small(),
    );

    ui.add_space(PART_GAP);
    ui.label(RichText::new("App PIN").strong());
    action.or(pin_rows(ui, settings))
}

/// The card at the top: a padlock, Locked or Unlocked, one line on what
/// that means, and the action that fits (Set password, Unlock, Lock now).
fn status_card(ui: &mut Ui, state: VaultState) -> Option<SettingsAction> {
    let (chip, line, button, action) = match state {
        VaultState::NoPassword => (
            None,
            "Choose a password to encrypt scratches. AI tools never see an encrypted scratch.",
            "Set password…",
            SettingsAction::SetScratchPassword,
        ),
        VaultState::Locked => (
            Some(("Locked", false)),
            "Encrypted scratches open after you enter the password.",
            "Unlock…",
            SettingsAction::UnlockScratches,
        ),
        VaultState::Unlocked => (
            Some(("Unlocked", true)),
            "Open for this session. They lock again when the app locks, or now.",
            "Lock now",
            SettingsAction::LockScratches,
        ),
    };
    let visuals = ui.visuals().clone();
    let mut clicked = false;
    egui::Frame::new()
        .fill(visuals.faint_bg_color)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(super::LIST_RADIUS)
        .inner_margin(CARD_MARGIN)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            row_layout(
                ui,
                |ui| {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(CARD_ICON_SIZE), Sense::hover());
                        let accent = if state == VaultState::Unlocked {
                            visuals.hyperlink_color
                        } else {
                            visuals.weak_text_color()
                        };
                        paint_padlock(ui.painter(), rect.center(), CARD_ICON_SIZE, visuals.text_color(), accent);
                        ui.label(RichText::new("Scratch password").strong());
                        if let Some((text, on)) = chip {
                            state_chip(ui, text, on);
                        }
                    });
                    ui.add(Label::new(RichText::new(line).weak()).wrap());
                },
                |ui| clicked = ui.add(primary_button(ui, button)).clicked(),
            );
        });
    clicked.then_some(action)
}

/// The scratch options, one row each. Touch ID shows only on a Mac with a
/// fingerprint, and the password rows only once there is a password.
fn scratch_rows(
    ui: &mut Ui,
    settings: &mut PrivacySettings,
    state: VaultState,
    touch_id_available: bool,
) -> Option<SettingsAction> {
    let mut action = None;
    let has_password = state != VaultState::NoPassword;
    list_frame(ui, |ui| {
        let mut encrypt = settings.encrypt_scratches;
        let hint = RichText::new("What Cmd+N makes. Cmd+Shift+N always encrypts.").weak();
        row(ui, "Encrypt new scratches", hint, |ui| {
            if switch(ui, &mut encrypt, SWITCH_SIZE, "Encrypt new scratches").changed() {
                if encrypt && !has_password {
                    // The password comes first; the dialog turns this on.
                    action = Some(SettingsAction::SetScratchPassword);
                } else {
                    settings.encrypt_scratches = encrypt;
                }
            }
        });
        if !has_password {
            return;
        }
        if touch_id_available {
            separator(ui);
            // The app flips the setting itself, once the key is in the keychain.
            let mut touch_id = settings.touch_id;
            let hint = RichText::new(
                "Your fingerprint instead of the password on this Mac. Adding or removing a fingerprint turns it off.",
            )
            .weak();
            row(ui, "Open with Touch ID", hint, |ui| {
                if switch(ui, &mut touch_id, SWITCH_SIZE, "Open with Touch ID").changed() {
                    action = Some(SettingsAction::SetTouchId(touch_id));
                }
            });
        }
        separator(ui);
        let unlocked = state == VaultState::Unlocked;
        let hint = RichText::new("Seal the plain scratches already in the scratch folder.").weak();
        row(ui, "Encrypt existing scratches", hint, |ui| {
            let button = ui.add_enabled(unlocked, egui::Button::new("Encrypt all"));
            if button.on_disabled_hover_text("Unlock the scratches first").clicked() {
                action = Some(SettingsAction::EncryptExistingScratches);
            }
        });
        separator(ui);
        let hint =
            RichText::new("If you forget it, encrypted scratches cannot be read.").color(ui.visuals().warn_fg_color);
        row(ui, "Password", hint, |ui| {
            if ui.button("Change…").clicked() {
                action = Some(SettingsAction::ChangeScratchPassword);
            }
        });
    });
    action
}

/// The PIN rows. Lock after and Lock now wait for a PIN.
fn pin_rows(ui: &mut Ui, settings: &mut PrivacySettings) -> Option<SettingsAction> {
    let mut action = None;
    let has_pin = settings.pin_hash.is_some();
    list_frame(ui, |ui| {
        let hint =
            RichText::new("Asked when Rubrique opens and after it sits idle. It locks the window, not your files.")
                .weak();
        row(ui, "PIN", hint, |ui| {
            // Right to left: the last button added sits leftmost.
            if has_pin {
                if ui.button("Remove…").clicked() {
                    action = Some(SettingsAction::RemovePin);
                }
                if ui.button("Change…").clicked() {
                    action = Some(SettingsAction::SetPin);
                }
            } else if ui.button("Set PIN…").clicked() {
                action = Some(SettingsAction::SetPin);
            }
        });
        separator(ui);
        ui.add_enabled_ui(has_pin, |ui| {
            row(ui, "Lock after", RichText::new("How long Rubrique sits unused before it asks again.").weak(), |ui| {
                egui::ComboBox::from_id_salt("privacy-lock-after")
                    .selected_text(lock_after_label(settings.lock_after_minutes))
                    .width(LOCK_PICKER_WIDTH)
                    .show_ui(ui, |ui| {
                        for minutes in LOCK_AFTER_CHOICES {
                            ui.selectable_value(&mut settings.lock_after_minutes, minutes, lock_after_label(minutes));
                        }
                    });
            });
        });
        separator(ui);
        ui.add_enabled_ui(has_pin, |ui| {
            row(ui, "Lock now", RichText::new("Or press Cmd+L.").weak(), |ui| {
                if enabled_button(ui, "Lock", has_pin) {
                    action = Some(SettingsAction::LockNow);
                }
            });
        });
    });
    action
}

/// One row of a list: the name over its hint on the left, the control on
/// the right, inside the list's row margin.
fn row(ui: &mut Ui, title: &str, hint: RichText, control: impl FnOnce(&mut Ui)) {
    egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
        ui.set_width(ui.available_width());
        row_layout(
            ui,
            |ui| {
                ui.label(RichText::new(title).strong());
                ui.add(Label::new(hint).wrap());
            },
            control,
        );
    });
}

/// Text on the left in the room the control column leaves, the control
/// right-aligned in that column and centered on the text's height.
///
/// The three pieces add up to exactly the width there is: egui's gap between
/// items is turned off for them (and back on inside the control column, for
/// two buttons side by side). Otherwise each row comes out a little wider
/// than the room it got, and the rows under it grow with it.
fn row_layout(ui: &mut Ui, text: impl FnOnce(&mut Ui), control: impl FnOnce(&mut Ui)) {
    let item_spacing = ui.spacing().item_spacing;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let text_width = (ui.available_width() - CONTROL_WIDTH - ROW_GAP).max(0.0);
        let text_rect = ui
            .allocate_ui_with_layout(vec2(text_width, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(text_width);
                ui.spacing_mut().item_spacing = vec2(item_spacing.x, 2.0);
                text(ui);
            })
            .response
            .rect;
        ui.add_space(ROW_GAP);
        let height = text_rect.height().max(ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(vec2(CONTROL_WIDTH, height), Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing = item_spacing;
            control(ui);
        });
    });
}

/// The line between two rows.
fn separator(ui: &mut Ui) {
    ui.add(egui::Separator::default().spacing(0.0));
}

/// "Unlocked" filled with the accent, "Locked" as a quiet outline (a fill
/// would match the card behind it), as a small chip.
fn state_chip(ui: &mut Ui, text: &str, on: bool) {
    let visuals = ui.visuals();
    let (fill, stroke, color) = if on {
        (visuals.selection.bg_fill, egui::Stroke::NONE, visuals.strong_text_color())
    } else {
        (egui::Color32::TRANSPARENT, egui::Stroke::new(1.0, visuals.weak_text_color()), visuals.text_color())
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CHIP_RADIUS)
        .inner_margin(egui::Margin::symmetric(7, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(CHIP_SIZE).strong().color(color));
        });
}

/// "Off", "1 minute", "15 minutes".
fn lock_after_label(minutes: u32) -> String {
    match minutes {
        0 => "Off".to_string(),
        1 => "1 minute".to_string(),
        _ => format!("{minutes} minutes"),
    }
}

/// The card's action, filled with the selection color: the card's own fill
/// matches a plain button's.
fn primary_button(ui: &Ui, label: &str) -> egui::Button<'static> {
    let selection = ui.visuals().selection;
    egui::Button::new(RichText::new(label).color(selection.stroke.color)).fill(selection.bg_fill)
}
