//! The menus in the macOS menu bar at the top of the screen: the app menu
//! (About, Settings, Hide, Quit), then File, View and Settings as described
//! in `menu_bar.rs`.
//!
//! The entries carry no key shortcuts. macOS hands a key that matches a menu
//! shortcut to the menu before the window sees it, so Cmd+S, Cmd+N and the
//! rest would skip the checks Rubrique makes before acting on them (an open
//! dialog, a field being renamed). The keys keep working as before; the
//! menus are for clicking.

use std::sync::mpsc::{Receiver, channel};

use eframe::egui::Context;
use muda::{AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

use super::RubriqueApp;
use super::menu_bar::{MenuCommand, MenuEntry};

/// Id of the app menu's "Settings..." entry.
const SETTINGS_ID: &str = "app-settings";

/// Whether the menus are in the menu bar yet.
#[derive(Debug)]
pub(super) enum NativeMenuState {
    /// Before the first frame.
    NotYet,
    Installed(NativeMenu),
    /// Installing failed; the system's default menu stays.
    Failed,
}

/// The installed menu bar and what its entries do.
pub(super) struct NativeMenu {
    /// Kept alive: the menu bar goes away with it.
    menu: Menu,
    /// The command of each entry, by its id.
    commands: Vec<(MenuId, MenuCommand)>,
    /// Entries with a check mark, refreshed every frame from the settings.
    checks: Vec<(CheckMenuItem, MenuCommand)>,
    /// Ids of clicked entries, sent from the menu's event handler.
    clicks: Receiver<MenuId>,
}

impl std::fmt::Debug for NativeMenu {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("NativeMenu").field("entries", &self.commands.len()).finish_non_exhaustive()
    }
}

impl NativeMenu {
    /// Build the menus from `app` and install them as the app's menu bar.
    /// Call on the main thread once the app is running (the first frame).
    /// A failure leaves the system's default menu.
    fn install(app: &RubriqueApp, ctx: &Context) -> Option<Self> {
        let (sender, clicks) = channel();
        let repaint = ctx.clone();
        // A click wakes the app, which may be idle and not drawing frames.
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = sender.send(event.id);
            repaint.request_repaint();
        }));
        let mut native = NativeMenu { menu: Menu::new(), commands: Vec::new(), checks: Vec::new(), clicks };
        native.menu.append(&app_menu().ok()?).ok()?;
        for (label, entries) in app.menus() {
            let submenu = Submenu::new(label, true);
            native.append_entries(app, &submenu, &entries).ok()?;
            native.menu.append(&submenu).ok()?;
        }
        native.menu.init_for_nsapp();
        Some(native)
    }

    /// Add `entries` to `submenu`, remembering what each one does.
    fn append_entries(&mut self, app: &RubriqueApp, submenu: &Submenu, entries: &[MenuEntry]) -> muda::Result<()> {
        for entry in entries {
            match entry {
                MenuEntry::Separator => submenu.append(&PredefinedMenuItem::separator())?,
                MenuEntry::Submenu { label, entries } => {
                    let inner = Submenu::new(*label, true);
                    self.append_entries(app, &inner, entries)?;
                    submenu.append(&inner)?;
                }
                MenuEntry::Item { label, command, .. } => {
                    let id = MenuId::new(format!("command-{}", self.commands.len()));
                    match app.menu_check(command) {
                        Some(on) => {
                            let check = CheckMenuItem::with_id(id.clone(), label, true, on, None);
                            submenu.append(&check)?;
                            self.checks.push((check, command.clone()));
                        }
                        None => submenu.append(&MenuItem::with_id(id.clone(), label, true, None))?,
                    }
                    self.commands.push((id, command.clone()));
                }
            }
        }
        Ok(())
    }
}

/// The app menu, named after the app by macOS: About, Settings, the
/// system's Services, Hide and Quit.
fn app_menu() -> muda::Result<Submenu> {
    let about = AboutMetadata {
        name: Some("Rubrique".to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        ..AboutMetadata::default()
    };
    Submenu::with_items(
        "Rubrique",
        true,
        &[
            &PredefinedMenuItem::about(None, Some(about)),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(SETTINGS_ID, "Settings...", true, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ],
    )
}

impl RubriqueApp {
    /// Install the menu bar on the first frame, then run the entries clicked
    /// since the last frame and keep the check marks in step with the
    /// settings (Settings or a shortcut may have changed them).
    pub(super) fn native_menu(&mut self, ctx: &Context) {
        if matches!(self.native_menu, NativeMenuState::NotYet) {
            self.native_menu = match NativeMenu::install(self, ctx) {
                Some(native) => NativeMenuState::Installed(native),
                None => NativeMenuState::Failed,
            };
        }
        let NativeMenuState::Installed(native) = &self.native_menu else {
            return;
        };
        let clicked: Vec<MenuCommand> = native
            .clicks
            .try_iter()
            .filter_map(|id| {
                if id.0 == SETTINGS_ID {
                    return Some(MenuCommand::OpenSettings);
                }
                native.commands.iter().find(|(known, _)| *known == id).map(|(_, command)| command.clone())
            })
            .collect();
        for command in clicked {
            self.run_menu_command(ctx, command);
        }
        let NativeMenuState::Installed(native) = &self.native_menu else {
            return;
        };
        for (check, command) in &native.checks {
            let on = self.menu_check(command).unwrap_or(false);
            if check.is_checked() != on {
                check.set_checked(on);
            }
        }
    }
}
