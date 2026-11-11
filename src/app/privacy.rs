//! Encrypted scratches and the app PIN: the scratch key for this session,
//! the lock screen, and the password and PIN dialogs.
//!
//! Each scratch is encrypted or plain on its own: `Cmd+N` follows the
//! "Encrypt every new scratch" setting, `Cmd+Shift+N` always makes an encrypted
//! one, and encryption is turned on or off for one scratch at any time: the
//! lock button in the status bar for the open one, or "Encrypt" / "Remove
//! encryption" in its right-click menu in the explorer. Either way asks for
//! the password first when it was not entered yet.
//!
//! The PIN and the scratch password are separate. The PIN only covers the
//! window; the password is the key the scratches are encrypted with.
//! Locking the window also forgets the password.
//!
//! With Touch ID on (macOS), the key is also kept in the keychain behind the
//! fingerprint: a locked tab or the Unlock dialog asks the system for it as
//! soon as it shows, and the password field stays as the way back.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use eframe::egui::{self, Context, Key, RichText, Stroke, Ui, vec2};
use zeroize::Zeroizing;

use super::find::{ACCENT_STROKE, outline_color};
use super::{OpenFile, RubriqueApp};
use crate::crypto::{KEY_LEN, Vault, pin_hash, verify_pin};
use crate::document::view::restore_editor_state;
use crate::explorer::paths::name_text;
use crate::explorer::tree::FsNode;
use crate::icons::paint_padlock;
use crate::scratch::{decrypt_scratch, encrypt_scratch, encrypt_scratches, is_scratch, reencrypt_scratches};
use crate::settings::privacy::PrivacySettings;
use crate::settings::view::{PrivacyInfo, SettingsAction};
use crate::theme::style::page_fill;
use crate::touch_id::{self, TouchIdError};

/// Shortest scratch password: it is the only thing between a copied file
/// and its text, so it has to be longer than a PIN.
const MIN_PASSWORD_CHARS: usize = 8;
const MIN_PIN_CHARS: usize = 4;
/// Wrong PINs in a row before the lock screen makes you wait.
const PIN_TRIES: u32 = 5;
const PIN_WAIT_SECONDS: f64 = 30.0;
/// "Lock after" picked when a PIN is set for the first time.
const DEFAULT_LOCK_MINUTES: u32 = 15;
const DIALOG_WIDTH: f32 = 340.0;
const FIELD_WIDTH: f32 = 260.0;
/// Width of the password field and the buttons under it on a locked scratch.
const LOCKED_COLUMN_WIDTH: f32 = 280.0;
/// Width of the padlock above a locked scratch's name.
const LOCKED_ICON_SIZE: f32 = 36.0;
/// Height of the password field and the buttons on a locked scratch.
const LOCKED_ROW_HEIGHT: f32 = 32.0;
/// Space left and right of the text inside the locked scratch's password field.
const LOCKED_FIELD_PADDING: i8 = 8;
/// Shown instead of running any AI on an encrypted scratch.
pub(super) const AI_OFF_MESSAGE: &str = "AI is off for encrypted scratches";
/// Under the app name in the Touch ID sheet.
const TOUCH_ID_REASON: &str = "open your encrypted scratches";

/// What the Touch ID thread sends back.
type TouchIdResult = Result<Zeroizing<[u8; KEY_LEN]>, TouchIdError>;

/// What to do once the scratch password is chosen or entered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum AfterPassword {
    /// Make a new encrypted scratch.
    NewEncryptedScratch,
    /// Store the scratch at this path encrypted (`true`) or plain.
    SetEncrypted { path: PathBuf, encrypt: bool },
    /// Keep the key behind Touch ID.
    EnableTouchId,
}

impl AfterPassword {
    /// The encrypted file this waits for: its header holds the salt, so
    /// the password can be checked against it when the settings lost it.
    fn encrypted_file(&self) -> Option<&Path> {
        match self {
            AfterPassword::SetEncrypted { path, encrypt: false } => Some(path),
            _ => None,
        }
    }
}

/// What a secret dialog is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    /// Choose the scratch password.
    NewPassword,
    /// Choose a new scratch password and seal every scratch with it.
    ChangePassword,
    /// Enter the scratch password.
    Unlock,
    /// Choose a PIN (or a new one).
    NewPin,
    RemovePin,
}

impl Purpose {
    fn title(self) -> &'static str {
        match self {
            Purpose::NewPassword => "Encrypt scratches",
            Purpose::ChangePassword => "Change scratch password",
            Purpose::Unlock => "Unlock encrypted scratches",
            Purpose::NewPin => "App PIN",
            Purpose::RemovePin => "Remove PIN",
        }
    }
}

/// A dialog that asks for a password or PIN. Typed text is wiped when it closes.
#[derive(Debug)]
struct SecretDialog {
    purpose: Purpose,
    /// The current password or PIN, when the purpose needs it.
    current: Zeroizing<String>,
    /// The new one, typed twice.
    first: Zeroizing<String>,
    second: Zeroizing<String>,
    error: Option<String>,
    /// The first field got focus when the dialog opened.
    focused: bool,
    /// Run after a password dialog succeeds.
    then: Option<AfterPassword>,
}

/// Privacy state for this run.
#[derive(Debug, Default)]
pub(super) struct PrivacyState {
    /// The scratch key, once the password was entered. Dropping the last
    /// copy wipes it.
    pub(super) vault: Option<Arc<Vault>>,
    /// The window shows only the PIN screen.
    locked: bool,
    pin_input: Zeroizing<String>,
    lock_error: Option<String>,
    wrong_pins: u32,
    /// egui time before which no PIN is checked (after too many wrong ones).
    wait_until: f64,
    /// egui time of the last key press, click or pointer move.
    last_activity: f64,
    dialog: Option<SecretDialog>,
    /// The password typed into a locked tab.
    tab_password: Zeroizing<String>,
    tab_error: Option<String>,
    /// A fingerprint is enrolled (macOS); asked once at start.
    pub(super) touch_id_available: bool,
    /// The keychain read in flight, while the system shows the Touch ID sheet.
    touch_id_request: Option<Receiver<TouchIdResult>>,
    /// The sheet was closed: no new one until the scratches lock again, unless
    /// asked for with the button.
    touch_id_declined: bool,
}

impl PrivacyState {
    /// Locked from the start when a PIN is set.
    pub(super) fn new(has_pin: bool) -> Self {
        PrivacyState { locked: has_pin, touch_id_available: touch_id::available(), ..PrivacyState::default() }
    }

    /// What the Privacy section of Settings shows, with `settings` to edit.
    pub(super) fn settings_info<'a>(&self, settings: &'a mut PrivacySettings) -> PrivacyInfo<'a> {
        PrivacyInfo { settings, unlocked: self.vault.is_some(), touch_id_available: self.touch_id_available }
    }

    fn open_dialog(&mut self, purpose: Purpose) {
        self.open_dialog_then(purpose, None);
    }

    fn open_dialog_then(&mut self, purpose: Purpose, then: Option<AfterPassword>) {
        self.dialog = Some(SecretDialog {
            purpose,
            current: Zeroizing::default(),
            first: Zeroizing::default(),
            second: Zeroizing::default(),
            error: None,
            focused: false,
            then,
        });
    }
}

impl RubriqueApp {
    /// The window is behind the PIN screen.
    pub(super) fn app_locked(&self) -> bool {
        self.privacy.locked
    }

    /// The open file is an encrypted scratch; says so in the status bar.
    /// Every AI entry point checks this first.
    pub(super) fn ai_blocked(&mut self) -> bool {
        let blocked = matches!(&self.open_file, OpenFile::Text { document, .. } if document.is_encrypted());
        if blocked {
            self.show_status(AI_OFF_MESSAGE.to_string());
        }
        blocked
    }

    /// New scratches are encrypted unless asked otherwise (the setting is
    /// on and a password exists).
    pub(super) fn encrypt_new_scratches(&self) -> bool {
        let privacy = &self.settings.privacy;
        privacy.encrypt_scratches && privacy.vault.is_some()
    }

    /// The scratch key, when it was entered in this session. Otherwise a
    /// dialog opens (to enter the password, or to choose one when there is
    /// none yet) and runs `then` once it succeeds.
    pub(super) fn scratch_vault_or_ask(&mut self, then: AfterPassword) -> Option<Arc<Vault>> {
        if let Some(vault) = &self.privacy.vault {
            return Some(Arc::clone(vault));
        }
        // An encrypted file has a password already, even when the settings
        // lost it: ask for that one rather than a new one.
        let has_password = self.settings.privacy.vault.is_some() || then.encrypted_file().is_some();
        let purpose = if has_password { Purpose::Unlock } else { Purpose::NewPassword };
        self.privacy.open_dialog_then(purpose, Some(then));
        None
    }

    /// The open file is a local scratch (for the status bar's lock button).
    pub(super) fn open_scratch_encrypted(&self) -> Option<bool> {
        let OpenFile::Text { document, .. } = &self.open_file else {
            return None;
        };
        let folder = self.settings.scratch_folder_or_default()?;
        let scratch = is_scratch(&document.path, &folder) && !crate::remote::is_mirror(&document.path);
        scratch.then(|| document.is_encrypted())
    }

    /// Store the scratch at `path` encrypted (`true`) or plain, whether it
    /// is open or not. Asks for the password first when it was not entered
    /// in this session, then carries on. Unsaved edits stay in the tab and
    /// are saved the new way.
    pub(super) fn set_scratch_encrypted(&mut self, path: &Path, encrypt: bool) {
        let then = AfterPassword::SetEncrypted { path: path.to_path_buf(), encrypt };
        let Some(vault) = self.scratch_vault_or_ask(then) else {
            return;
        };
        let result = if encrypt { encrypt_scratch(path, &vault) } else { decrypt_scratch(path, &vault) };
        if let Err(error) = result {
            let verb = if encrypt { "encrypt" } else { "remove encryption from" };
            self.show_status(format!("Could not {verb} {}: {error}", name_text(path)));
            return;
        }
        let key = encrypt.then_some(vault);
        self.follow_new_key(&[path.to_path_buf()], key.as_ref());
        self.rewrite_draft(path);
        // The padlock in the explorer follows the file.
        self.roots.iter_mut().for_each(FsNode::refresh);
        let message = if encrypt {
            if self.open_file.path() == Some(path) {
                // AI never gets an encrypted scratch's text.
                self.ask.close_bar();
            }
            "This scratch is encrypted now. AI tools are off for it"
        } else {
            "This scratch is plain text now"
        };
        self.show_status(message.to_string());
    }

    /// Carry on with what waited for the scratch password.
    fn after_password(&mut self, ctx: &Context, then: AfterPassword) {
        match then {
            AfterPassword::NewEncryptedScratch => self.new_scratch(ctx, true),
            AfterPassword::SetEncrypted { path, encrypt } => self.set_scratch_encrypted(&path, encrypt),
            AfterPassword::EnableTouchId => self.enable_touch_id(),
        }
    }

    /// Note input and lock the window once it sat idle long enough.
    pub(super) fn track_activity(&mut self, ctx: &Context) {
        let (now, active) = ctx.input(|input| (input.time, !input.events.is_empty() || input.pointer.any_down()));
        if active || self.privacy.last_activity == 0.0 {
            self.privacy.last_activity = now;
        }
        let minutes = self.settings.privacy.lock_after_minutes;
        if self.privacy.locked || self.settings.privacy.pin_hash.is_none() || minutes == 0 {
            return;
        }
        let limit = f64::from(minutes) * 60.0;
        let idle = now - self.privacy.last_activity;
        if idle >= limit {
            self.lock_app();
        } else {
            ctx.request_repaint_after(Duration::from_secs_f64(limit - idle));
        }
    }

    /// Show the PIN screen, keeping unsaved edits, and forget the scratch password.
    pub(super) fn lock_app(&mut self) {
        if self.settings.privacy.pin_hash.is_none() {
            return;
        }
        self.lock_scratches();
        self.privacy.locked = true;
        self.privacy.dialog = None;
        self.privacy.lock_error = None;
        self.settings_open = false;
    }

    /// Forget the scratch password: unsaved edits of encrypted scratches go
    /// to encrypted drafts, and their tabs ask for the password again.
    pub(super) fn lock_scratches(&mut self) {
        if self.privacy.vault.is_none() {
            return;
        }
        self.finish_block_edit();
        if !self.keep_drafts() {
            // keep_drafts said why; never drop edits that are not kept.
            return;
        }
        if matches!(&self.open_file, OpenFile::Text { document, .. } if document.is_encrypted()) {
            self.merging = None;
            self.note_draft = None;
            self.stats = None;
            // The editor's undo history holds the text too.
            restore_editor_state(&self.repaint, None);
        }
        let files = std::iter::once(&mut self.open_file).chain(self.tabs.iter_mut());
        for file in files {
            if let OpenFile::Text { document, .. } = file
                && document.is_encrypted()
            {
                let path = document.path.clone();
                // The draft stays on disk, sealed, for the next unlock.
                self.drafts.release(&path);
                *file = OpenFile::Locked { path };
            }
        }
        self.privacy.vault = None;
        // The next locked tab may ask for the fingerprint again.
        self.privacy.touch_id_declined = false;
    }

    /// Check the scratch password and keep the key for this session. Tabs
    /// waiting for it open, with their unsaved edits.
    fn unlock_scratches(&mut self, password: &str, file: Option<&Path>) -> Result<(), String> {
        let vault = if let Some(info) = &self.settings.privacy.vault {
            Vault::unlock(password, info).ok_or("Wrong password")?
        } else {
            // Settings lost: the salt is in the file as well.
            let bytes = file.and_then(|path| std::fs::read(path).ok()).ok_or("No scratch password is set")?;
            let (vault, info) = Vault::recover(password, &bytes).ok_or("Wrong password")?;
            self.settings.privacy.vault = Some(info);
            self.write_settings();
            vault
        };
        self.install_vault(Arc::new(vault));
        Ok(())
    }

    /// Keep the key for this session and open the tabs waiting for it.
    fn install_vault(&mut self, vault: Arc<Vault>) {
        self.privacy.vault = Some(vault);
        self.privacy.touch_id_declined = false;
        self.privacy.tab_error = None;
        self.reopen_locked_tabs();
    }

    /// Touch ID can open the scratches now: it is on, a fingerprint is
    /// enrolled, the settings know the vault and no sheet is up.
    fn touch_id_ready(&self) -> bool {
        self.settings.privacy.touch_id
            && self.privacy.touch_id_available
            && self.settings.privacy.vault.is_some()
            && self.privacy.touch_id_request.is_none()
    }

    /// Ask the system for the kept key on a thread: the keychain blocks
    /// while the Touch ID sheet is up. [`Self::poll_touch_id`] takes the answer.
    fn start_touch_id(&mut self, ctx: &Context) {
        if !self.touch_id_ready() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = touch_id::read_key(TOUCH_ID_REASON);
            // The receiver is gone only when the app closed meanwhile.
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.privacy.touch_id_request = Some(receiver);
    }

    /// Open the scratches with the key Touch ID gave, or say why not: in the
    /// Unlock dialog when it is open, otherwise under the locked tab.
    pub(super) fn poll_touch_id(&mut self, ctx: &Context) {
        let Some(receiver) = &self.privacy.touch_id_request else {
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };
        self.privacy.touch_id_request = None;
        let error = match result {
            Ok(key) => {
                let info = self.settings.privacy.vault.as_ref();
                if let Some(vault) = info.and_then(|info| Vault::from_key(key, info)) {
                    self.install_vault(Arc::new(vault));
                    // The dialog waited for this key; carry on with what it was for.
                    if let Some(dialog) = self.privacy.dialog.take_if(|dialog| dialog.purpose == Purpose::Unlock)
                        && let Some(then) = dialog.then
                    {
                        self.after_password(ctx, then);
                    }
                    return;
                }
                self.turn_touch_id_off();
                "The key kept for Touch ID is not the current password. Enter the password, then turn Touch ID \
                 on again in Settings"
                    .to_string()
            }
            Err(TouchIdError::Cancelled) => {
                self.privacy.touch_id_declined = true;
                return;
            }
            Err(TouchIdError::NotStored) => {
                self.turn_touch_id_off();
                "Touch ID is off: the kept key is gone (did the fingerprints change?). Set it up again in Settings"
                    .to_string()
            }
            Err(error) => error.to_string(),
        };
        // Said once: the button asks again, not the next frame.
        self.privacy.touch_id_declined = true;
        match &mut self.privacy.dialog {
            Some(dialog) if dialog.purpose == Purpose::Unlock => dialog.error = Some(error),
            _ => self.privacy.tab_error = Some(error),
        }
    }

    /// The "Use Touch ID" button under a locked scratch's Unlock button,
    /// when Touch ID is on. Returns `true` when clicked.
    fn touch_id_button(&self, ui: &mut Ui) -> bool {
        if !(self.settings.privacy.touch_id && self.privacy.touch_id_available) {
            return false;
        }
        let waiting = self.privacy.touch_id_request.is_some();
        let label = if waiting { "Waiting for Touch ID…" } else { "Use Touch ID" };
        let button = egui::Button::new(label).min_size(vec2(LOCKED_COLUMN_WIDTH, LOCKED_ROW_HEIGHT));
        ui.add_enabled(!waiting, button).clicked()
    }

    /// Keep the key behind Touch ID and turn the setting on; asks for the
    /// password first when the scratches are locked.
    fn enable_touch_id(&mut self) {
        let Some(vault) = self.scratch_vault_or_ask(AfterPassword::EnableTouchId) else {
            return;
        };
        match touch_id::store_key(&vault.key_bytes()) {
            Ok(()) => {
                self.settings.privacy.touch_id = true;
                self.write_settings();
                self.show_status("Touch ID opens encrypted scratches from now on".to_string());
            }
            Err(error) => self.show_status(error.to_string()),
        }
    }

    /// Forget the kept key and turn the setting off.
    fn disable_touch_id(&mut self) {
        match touch_id::forget_key() {
            Ok(()) | Err(TouchIdError::NotStored) => {}
            Err(error) => self.show_status(error.to_string()),
        }
        self.turn_touch_id_off();
        self.show_status("Encrypted scratches ask for the password again".to_string());
    }

    fn turn_touch_id_off(&mut self) {
        if self.settings.privacy.touch_id {
            self.settings.privacy.touch_id = false;
            self.write_settings();
        }
    }

    /// After the password changed: keep the new key behind Touch ID, or turn
    /// Touch ID off when that fails.
    fn refresh_touch_id_key(&mut self) {
        if !self.settings.privacy.touch_id {
            return;
        }
        let Some(vault) = self.privacy.vault.clone() else {
            return;
        };
        if let Err(error) = touch_id::store_key(&vault.key_bytes()) {
            self.turn_touch_id_off();
            self.show_status(format!("Touch ID is off: {error}"));
        }
    }

    /// Read every tab waiting for the password again, then put back the
    /// sealed drafts.
    fn reopen_locked_tabs(&mut self) {
        for index in 0..self.tabs.len() {
            let path = match self.tab(index) {
                OpenFile::Locked { path } => path.clone(),
                _ => continue,
            };
            let Some(file) = self.load_file(path) else {
                continue;
            };
            if index == self.active_tab {
                self.open_file = file;
                self.enter_tab();
            } else {
                self.tabs[index] = file;
            }
        }
        self.restore_sealed_drafts();
    }

    /// Draw the PIN screen over the whole window.
    pub(super) fn lock_screen(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        let waiting = now < self.privacy.wait_until;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 3.0).max(20.0));
                ui.heading("Rubrique is locked");
                ui.add_space(8.0);
                ui.label("Enter your PIN");
                let field = ui.add_enabled(
                    !waiting,
                    egui::TextEdit::singleline(&mut *self.privacy.pin_input).password(true).desired_width(FIELD_WIDTH),
                );
                // The field takes focus back every frame, so Enter is read
                // while it has focus too, not only when it lets go.
                let entered =
                    (field.lost_focus() || field.has_focus()) && ui.input(|input| input.key_pressed(Key::Enter));
                if !waiting && !entered && !field.has_focus() {
                    field.request_focus();
                }
                let clicked = ui.add_enabled(!waiting, egui::Button::new("Unlock")).clicked();
                if waiting {
                    let seconds = (self.privacy.wait_until - now).ceil();
                    ui.label(RichText::new(format!("Too many wrong PINs. Try again in {seconds} s")).weak());
                    ctx.request_repaint_after(Duration::from_secs(1));
                } else if let Some(error) = &self.privacy.lock_error {
                    ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
                }
                if (entered || clicked) && !waiting {
                    self.try_pin(now);
                }
            });
        });
    }

    fn try_pin(&mut self, now: f64) {
        let hash = self.settings.privacy.pin_hash.clone().unwrap_or_default();
        if verify_pin(&self.privacy.pin_input, &hash) {
            self.privacy.locked = false;
            self.privacy.wrong_pins = 0;
            self.privacy.lock_error = None;
            self.privacy.last_activity = now;
        } else {
            self.privacy.wrong_pins += 1;
            self.privacy.lock_error = Some("Wrong PIN".to_string());
            if self.privacy.wrong_pins >= PIN_TRIES {
                self.privacy.wrong_pins = 0;
                self.privacy.wait_until = now + PIN_WAIT_SECONDS;
            }
        }
        self.privacy.pin_input = Zeroizing::default();
    }

    /// The page of a tab whose encrypted scratch waits for the password: a
    /// centered column like the PIN screen, with a padlock, the name, an
    /// outlined password field that has focus, Unlock under it, and Use
    /// Touch ID under that when Touch ID is on.
    pub(super) fn locked_tab(&mut self, ui: &mut Ui, path: &Path) {
        if !self.privacy.touch_id_declined {
            self.start_touch_id(ui.ctx());
        }
        let mut unlock = false;
        let mut use_touch_id = false;
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 4.0).max(24.0));
            let (rect, _) = ui.allocate_exact_size(vec2(LOCKED_ICON_SIZE, LOCKED_ICON_SIZE), egui::Sense::hover());
            let visuals = ui.visuals();
            paint_padlock(ui.painter(), rect.center(), LOCKED_ICON_SIZE, visuals.text_color(), visuals.hyperlink_color);
            ui.add_space(12.0);
            ui.heading(name_text(path));
            ui.label(RichText::new("This scratch is encrypted. Enter the scratch password to open it.").weak());
            ui.add_space(16.0);
            unlock = self.locked_password_field(ui);
            if let Some(error) = &self.privacy.tab_error {
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            }
            ui.add_space(8.0);
            unlock |= ui.add(primary_button(ui, "Unlock")).clicked();
            use_touch_id = self.touch_id_button(ui);
        });
        if unlock {
            let password = std::mem::take(&mut self.privacy.tab_password);
            self.privacy.tab_error = self.unlock_scratches(&password, Some(path)).err();
        }
        if use_touch_id {
            self.privacy.touch_id_declined = false;
            self.start_touch_id(ui.ctx());
        }
    }

    /// The locked scratch's password field: outlined (the page and the field
    /// share a color in most themes), accent outline while typing, focused
    /// when nothing else is. Returns `true` when Enter was pressed in it.
    fn locked_password_field(&mut self, ui: &mut Ui) -> bool {
        let id = ui.make_persistent_id("locked-scratch-password");
        let visuals = ui.visuals();
        let stroke = if ui.memory(|memory| memory.has_focus(id)) {
            Stroke::new(ACCENT_STROKE, visuals.hyperlink_color)
        } else {
            Stroke::new(1.0, outline_color(visuals))
        };
        let frame = egui::Frame::new()
            .fill(page_fill(visuals))
            .stroke(stroke)
            .corner_radius(visuals.widgets.inactive.corner_radius)
            .inner_margin(egui::Margin::symmetric(LOCKED_FIELD_PADDING, 0));
        // As in the find bar, the width covers the outline and the padding.
        let field = ui.add(
            egui::TextEdit::singleline(&mut *self.privacy.tab_password)
                .id(id)
                .password(true)
                .hint_text("Scratch password")
                .frame(frame)
                .desired_width(LOCKED_COLUMN_WIDTH)
                .min_size(vec2(0.0, LOCKED_ROW_HEIGHT))
                .align(egui::Align2::LEFT_CENTER),
        );
        // Ready to type when the tab opens, but never taken from another
        // field, and not while the Touch ID sheet is up.
        if self.privacy.touch_id_request.is_none() && ui.memory(|memory| memory.focused().is_none()) {
            field.request_focus();
        }
        field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
    }

    /// Run a Privacy button from Settings.
    pub(super) fn privacy_action(&mut self, action: &SettingsAction) {
        let purpose = match action {
            SettingsAction::SetScratchPassword => Purpose::NewPassword,
            SettingsAction::ChangeScratchPassword => Purpose::ChangePassword,
            SettingsAction::UnlockScratches => Purpose::Unlock,
            SettingsAction::SetPin => Purpose::NewPin,
            SettingsAction::RemovePin => Purpose::RemovePin,
            SettingsAction::LockScratches => {
                self.lock_scratches();
                self.show_status("Encrypted scratches are locked".to_string());
                return;
            }
            SettingsAction::SetTouchId(true) => {
                self.enable_touch_id();
                return;
            }
            SettingsAction::SetTouchId(false) => {
                self.disable_touch_id();
                return;
            }
            SettingsAction::EncryptExistingScratches => {
                self.encrypt_existing();
                return;
            }
            SettingsAction::LockNow => {
                self.lock_app();
                return;
            }
            _ => return,
        };
        self.privacy.open_dialog(purpose);
    }

    /// Encrypt the plain scratches already in the folder, open ones included.
    fn encrypt_existing(&mut self) {
        let (Some(vault), Some(folder)) = (self.privacy.vault.clone(), self.settings.scratch_folder_or_default())
        else {
            return;
        };
        match encrypt_scratches(&folder, &vault) {
            Ok(changed) => {
                self.follow_new_key(&changed, Some(&vault));
                self.roots.iter_mut().for_each(FsNode::refresh);
                let count = changed.len();
                self.show_status(format!("Encrypted {count} scratch{}", if count == 1 { "" } else { "es" }));
            }
            Err(error) => self.show_status(format!("Could not encrypt the scratches: {error}")),
        }
    }

    /// Open tabs of `changed` files are now stored with `vault` (`None`: plain).
    fn follow_new_key(&mut self, changed: &[PathBuf], vault: Option<&Arc<Vault>>) {
        for file in self.all_tabs_mut() {
            if let OpenFile::Text { document, .. } = file
                && changed.contains(&document.path)
            {
                document.set_vault(vault.cloned());
            }
        }
    }

    /// Draw the password or PIN dialog, if one is open.
    pub(super) fn secret_dialog(&mut self, ctx: &Context) {
        let unlocking = self.privacy.dialog.as_ref().is_some_and(|dialog| dialog.purpose == Purpose::Unlock);
        if unlocking && !self.privacy.touch_id_declined {
            self.start_touch_id(ctx);
        }
        let touch_id_on = unlocking && self.settings.privacy.touch_id && self.privacy.touch_id_available;
        let touch_id_waiting = self.privacy.touch_id_request.is_some();
        let Some(dialog) = &mut self.privacy.dialog else {
            return;
        };
        let has_pin = self.settings.privacy.pin_hash.is_some();
        let (asks_current, asks_new, is_pin) = match dialog.purpose {
            Purpose::NewPassword => (false, true, false),
            Purpose::ChangePassword => (true, true, false),
            Purpose::Unlock => (true, false, false),
            Purpose::NewPin => (has_pin, true, true),
            Purpose::RemovePin => (true, false, true),
        };
        let word = if is_pin { "PIN" } else { "password" };
        let mut submit = false;
        let mut cancel = false;
        let mut use_touch_id = false;
        let modal = egui::Modal::new(egui::Id::unique("secret-dialog")).show(ctx, |ui| {
            ui.set_width(DIALOG_WIDTH);
            ui.heading(dialog.purpose.title());
            if dialog.purpose == Purpose::NewPassword {
                ui.label(
                    RichText::new("If you forget this password, encrypted scratches cannot be read.")
                        .color(ui.visuals().warn_fg_color),
                );
            }
            let mut first_field = None;
            let mut field = |ui: &mut Ui, label: &str, text: &mut Zeroizing<String>| {
                ui.label(label);
                let response =
                    ui.add(egui::TextEdit::singleline(&mut **text).password(true).desired_width(FIELD_WIDTH));
                if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                    submit = true;
                }
                first_field.get_or_insert(response.id);
            };
            if asks_current {
                let label = if asks_new { format!("Current {word}") } else { capitalized(word) };
                field(ui, &label, &mut dialog.current);
            }
            if asks_new {
                field(ui, &format!("New {word}"), &mut dialog.first);
                field(ui, &format!("New {word} again"), &mut dialog.second);
            }
            // Once, on opening: the button that opened it still has focus.
            if let Some(id) = first_field
                && !dialog.focused
            {
                dialog.focused = true;
                ui.memory_mut(|memory| memory.request_focus(id));
            }
            if let Some(error) = &dialog.error {
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                submit |= ui.button("OK").clicked();
                cancel |= ui.button("Cancel").clicked();
                if touch_id_on {
                    let label = if touch_id_waiting { "Waiting for Touch ID…" } else { "Use Touch ID" };
                    use_touch_id |= ui.add_enabled(!touch_id_waiting, egui::Button::new(label)).clicked();
                }
            });
        });
        if cancel || modal.should_close() {
            self.privacy.dialog = None;
        } else if submit {
            self.submit_dialog(ctx);
        } else if use_touch_id {
            self.privacy.touch_id_declined = false;
            self.start_touch_id(ctx);
        }
    }

    /// Act on the dialog's fields; closes it on success, shows why not otherwise.
    fn submit_dialog(&mut self, ctx: &Context) {
        let Some(dialog) = self.privacy.dialog.take() else {
            return;
        };
        let current = dialog.current.as_str();
        // From Settings, a first password also turns the setting on; from a
        // single scratch it only gives that scratch a key.
        let from_settings = dialog.then.is_none();
        let result = match dialog.purpose {
            Purpose::NewPassword => new_secret(&dialog, MIN_PASSWORD_CHARS, "password")
                .map(|()| self.set_scratch_password(&dialog.first, from_settings)),
            Purpose::ChangePassword => self.change_scratch_password(&dialog),
            Purpose::Unlock => {
                let file = dialog.then.as_ref().and_then(AfterPassword::encrypted_file);
                self.unlock_scratches(current, file)
            }
            Purpose::NewPin => self.set_pin(&dialog),
            Purpose::RemovePin => self.check_pin(current).map(|()| {
                self.settings.privacy.pin_hash = None;
                self.write_settings();
                self.show_status("The PIN is off".to_string());
            }),
        };
        match result {
            Ok(()) => {
                if let Some(then) = dialog.then {
                    self.after_password(ctx, then);
                }
            }
            Err(error) => self.privacy.dialog = Some(SecretDialog { error: Some(error), ..dialog }),
        }
    }

    /// Choose the first scratch password; `make_default` also encrypts
    /// every new scratch from now on.
    fn set_scratch_password(&mut self, password: &str, make_default: bool) {
        let (vault, info) = Vault::create(password);
        self.settings.privacy.vault = Some(info);
        if make_default {
            self.settings.privacy.encrypt_scratches = true;
        }
        self.privacy.vault = Some(Arc::new(vault));
        // Written at once: without it the scratches could not be opened.
        self.write_settings();
        self.refresh_touch_id_key();
        if make_default {
            self.show_status("New scratches are encrypted from now on".to_string());
        }
    }

    /// Seal every scratch and sealed draft with a new password.
    fn change_scratch_password(&mut self, dialog: &SecretDialog) -> Result<(), String> {
        let info = self.settings.privacy.vault.clone().ok_or("No scratch password is set")?;
        let old = Vault::unlock(&dialog.current, &info).ok_or("Wrong password")?;
        new_secret(dialog, MIN_PASSWORD_CHARS, "password")?;
        let folder = self.settings.scratch_folder_or_default().ok_or("No scratch folder")?;
        let (new, new_info) = Vault::create(&dialog.first);
        let new = Arc::new(new);
        let changed = reencrypt_scratches(&folder, &old, &new)
            .map_err(|error| format!("Could not change the password: {error}. Some scratches may use the new one."))?;
        self.reseal_drafts(&old, &new);
        self.settings.privacy.vault = Some(new_info);
        self.write_settings();
        self.follow_new_key(&changed, Some(&new));
        self.install_vault(new);
        self.refresh_touch_id_key();
        self.show_status("The scratch password is changed".to_string());
        Ok(())
    }

    /// Choose a PIN (the current one is asked for first, when there is one).
    fn set_pin(&mut self, dialog: &SecretDialog) -> Result<(), String> {
        if self.settings.privacy.pin_hash.is_some() {
            self.check_pin(&dialog.current)?;
        }
        new_secret(dialog, MIN_PIN_CHARS, "PIN")?;
        let first_time = self.settings.privacy.pin_hash.is_none();
        self.settings.privacy.pin_hash = Some(pin_hash(&dialog.first));
        if first_time && self.settings.privacy.lock_after_minutes == 0 {
            self.settings.privacy.lock_after_minutes = DEFAULT_LOCK_MINUTES;
        }
        self.write_settings();
        self.show_status("PIN set. Lock now with Cmd+L".to_string());
        Ok(())
    }

    fn check_pin(&self, pin: &str) -> Result<(), String> {
        let hash = self.settings.privacy.pin_hash.as_deref().unwrap_or_default();
        verify_pin(pin, hash).then_some(()).ok_or_else(|| "Wrong PIN".to_string())
    }
}

/// "password" → "Password".
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The new secret is long enough and typed the same twice.
fn new_secret(dialog: &SecretDialog, min_chars: usize, word: &str) -> Result<(), String> {
    if dialog.first.chars().count() < min_chars {
        return Err(format!("Use at least {min_chars} characters for the {word}"));
    }
    if *dialog.first != *dialog.second {
        return Err(format!("The two {word}s are not the same"));
    }
    Ok(())
}

/// The main button of a locked scratch, as wide as its password field and
/// filled with the selection color so it reads as the thing to press.
fn primary_button(ui: &Ui, label: &str) -> egui::Button<'static> {
    let selection = ui.visuals().selection;
    egui::Button::new(RichText::new(label).color(selection.stroke.color))
        .fill(selection.bg_fill)
        .min_size(vec2(LOCKED_COLUMN_WIDTH, LOCKED_ROW_HEIGHT))
}
