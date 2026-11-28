//! The menus in the macOS menu bar at the top of the screen: the app menu
//! (About, Settings, Hide, Quit), then File, View and Settings as described
//! in `menu_bar.rs`.
//!
//! The entries carry no key shortcuts. macOS hands a key that matches a menu
//! shortcut to the menu before the window sees it, so Cmd+S, Cmd+N and the
//! rest would skip the checks Rubrique makes before acting on them (an open
//! dialog, a field being renamed). The keys keep working as before; the
//! menus are for clicking. Only the system entries (Hide, Quit) keep their
//! usual shortcuts.
//!
//! Each Rubrique entry carries a tag (its index in `commands`) and calls one
//! small Objective-C object, `MenuTarget`, which sends the tag to the app.

use std::sync::mpsc::{Receiver, channel};

use eframe::egui::Context;
use objc2::rc::Retained;
use objc2::sel;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu, NSMenuItem,
};
use objc2_foundation::NSString;

use super::RubriqueApp;
use super::menu_bar::{MenuCommand, MenuEntry};
use cocoa::{MenuTarget, menu_item, set_target};

/// Tag of the app menu's "Settings..." entry.
const SETTINGS_TAG: isize = -1;
/// Tag of the app menu's "About Rubrique" entry.
const ABOUT_TAG: isize = -2;

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
    menu: Retained<NSMenu>,
    /// Receives every entry's clicks; kept alive because a menu item does
    /// not own its target.
    target: Retained<MenuTarget>,
    /// The command of each entry, by its tag.
    commands: Vec<MenuCommand>,
    /// Entries with a check mark, refreshed every frame from the settings.
    checks: Vec<(Retained<NSMenuItem>, MenuCommand)>,
    /// Tags of clicked entries, sent from the menu target.
    clicks: Receiver<isize>,
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
        let mtm = MainThreadMarker::new()?;
        let (sender, clicks) = channel();
        let target = MenuTarget::new(mtm, sender, ctx.clone());
        let mut native =
            NativeMenu { menu: NSMenu::new(mtm), target, commands: Vec::new(), checks: Vec::new(), clicks };
        let application = NSApplication::sharedApplication(mtm);
        let app_menu = native.app_menu(mtm, &application);
        native.menu.addItem(&submenu_item(mtm, "Rubrique", &app_menu));
        for (label, entries) in app.menus() {
            let submenu = new_menu(mtm, label);
            native.append_entries(app, mtm, &submenu, &entries);
            native.menu.addItem(&submenu_item(mtm, label, &submenu));
        }
        application.setMainMenu(Some(&native.menu));
        Some(native)
    }

    /// Add `entries` to `menu`, remembering what each one does.
    fn append_entries(&mut self, app: &RubriqueApp, mtm: MainThreadMarker, menu: &NSMenu, entries: &[MenuEntry]) {
        for entry in entries {
            match entry {
                MenuEntry::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
                MenuEntry::Submenu { label, entries } => {
                    let inner = new_menu(mtm, label);
                    self.append_entries(app, mtm, &inner, entries);
                    menu.addItem(&submenu_item(mtm, label, &inner));
                }
                MenuEntry::Item { label, command, .. } => {
                    let Ok(tag) = isize::try_from(self.commands.len()) else { continue };
                    let item = self.entry(mtm, label, tag);
                    if let Some(on) = app.menu_check(command) {
                        item.setState(if on { NSControlStateValueOn } else { NSControlStateValueOff });
                        self.checks.push((item.clone(), command.clone()));
                    }
                    menu.addItem(&item);
                    self.commands.push(command.clone());
                }
            }
        }
    }

    /// A Rubrique entry that sends `tag` to the app when clicked.
    fn entry(&self, mtm: MainThreadMarker, title: &str, tag: isize) -> Retained<NSMenuItem> {
        let item = menu_item(mtm, title, Some(sel!(entryClicked:)), "");
        item.setTag(tag);
        set_target(&item, &self.target);
        item
    }

    /// The app menu, named after the app by macOS: About, Settings, the
    /// system's Services, Hide and Quit.
    fn app_menu(&self, mtm: MainThreadMarker, application: &NSApplication) -> Retained<NSMenu> {
        let menu = new_menu(mtm, "Rubrique");
        let services = new_menu(mtm, "Services");
        application.setServicesMenu(Some(&services));
        let hide_others = menu_item(mtm, "Hide Others", Some(sel!(hideOtherApplications:)), "h");
        hide_others.setKeyEquivalentModifierMask(NSEventModifierFlags::Option | NSEventModifierFlags::Command);
        let items = [
            self.entry(mtm, "About Rubrique", ABOUT_TAG),
            NSMenuItem::separatorItem(mtm),
            self.entry(mtm, "Settings...", SETTINGS_TAG),
            NSMenuItem::separatorItem(mtm),
            submenu_item(mtm, "Services", &services),
            NSMenuItem::separatorItem(mtm),
            menu_item(mtm, "Hide Rubrique", Some(sel!(hide:)), "h"),
            hide_others,
            menu_item(mtm, "Show All", Some(sel!(unhideAllApplications:)), ""),
            NSMenuItem::separatorItem(mtm),
            menu_item(mtm, "Quit Rubrique", Some(sel!(terminate:)), "q"),
        ];
        for item in &items {
            menu.addItem(item);
        }
        menu
    }
}

/// An empty menu with `title`.
fn new_menu(mtm: MainThreadMarker, title: &str) -> Retained<NSMenu> {
    NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title))
}

/// A menu bar or menu entry that opens `submenu`.
fn submenu_item(mtm: MainThreadMarker, title: &str, submenu: &NSMenu) -> Retained<NSMenuItem> {
    let item = menu_item(mtm, title, None, "");
    item.setSubmenu(Some(submenu));
    item
}

/// The Objective-C side: the click target class, menu item creation and the
/// About panel. These are the `unsafe` calls of this module.
#[expect(unsafe_code, reason = "Objective-C class definition and calls from the objc2 bindings")]
mod cocoa {
    use std::sync::mpsc::Sender;

    use eframe::egui::Context;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, Sel};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{
        NSAboutPanelOptionApplicationName, NSAboutPanelOptionApplicationVersion, NSApplication, NSMenuItem,
    };
    use objc2_foundation::{NSDictionary, NSString};

    use super::ABOUT_TAG;

    /// Where a click goes: the channel the app reads, and the context to
    /// wake it (it may be idle and not drawing frames).
    pub(super) struct TargetState {
        clicks: Sender<isize>,
        repaint: Context,
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and this class
        // implements no `Drop` or `dealloc`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = TargetState]
        pub(super) struct MenuTarget;

        impl MenuTarget {
            /// The action of every Rubrique entry: send its tag to the app.
            #[unsafe(method(entryClicked:))]
            fn entry_clicked(&self, item: &NSMenuItem) {
                if item.tag() == ABOUT_TAG {
                    show_about(MainThreadMarker::from(self));
                    return;
                }
                let _ = self.ivars().clicks.send(item.tag());
                self.ivars().repaint.request_repaint();
            }
        }
    );

    impl MenuTarget {
        /// A target that sends clicked tags to `clicks`.
        pub(super) fn new(mtm: MainThreadMarker, clicks: Sender<isize>, repaint: Context) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TargetState { clicks, repaint });
            // SAFETY: `init` is NSObject's designated initializer, called once
            // on a freshly allocated object.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// A menu item with `title`, an `action` sent up the responder chain
    /// (or to its target), and a key equivalent (`""` for none).
    pub(super) fn menu_item(
        mtm: MainThreadMarker,
        title: &str,
        action: Option<Sel>,
        key: &str,
    ) -> Retained<NSMenuItem> {
        // SAFETY: every selector passed in is a method of NSApplication or
        // of `MenuTarget`, taking the sender as its one argument.
        unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                action,
                &NSString::from_str(key),
            )
        }
    }

    /// Send the item's action to `target`.
    pub(super) fn set_target(item: &NSMenuItem, target: &MenuTarget) {
        let target: &AnyObject = target;
        // SAFETY: the item keeps a weak reference; `NativeMenu` owns the
        // target as long as the menu exists, and the target implements the
        // item's action.
        unsafe { item.setTarget(Some(target)) };
    }

    /// The standard About panel with the app's name and full version
    /// (including a release candidate suffix the bundle cannot hold).
    fn show_about(mtm: MainThreadMarker) {
        let name = NSString::from_str("Rubrique");
        let version = NSString::from_str(env!("CARGO_PKG_VERSION"));
        // SAFETY: AppKit defines these constant keys for the app's lifetime.
        let keys = unsafe { [NSAboutPanelOptionApplicationName, NSAboutPanelOptionApplicationVersion] };
        let values: [&AnyObject; 2] = [&name, &version];
        let options = NSDictionary::from_slices(&keys, &values);
        // SAFETY: both values are strings, as these two keys require.
        unsafe { NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanelWithOptions(&options) };
    }
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
            .filter_map(|tag| {
                if tag == SETTINGS_TAG {
                    return Some(MenuCommand::OpenSettings);
                }
                native.commands.get(usize::try_from(tag).ok()?).cloned()
            })
            .collect();
        for command in clicked {
            self.run_menu_command(ctx, command);
        }
        let NativeMenuState::Installed(native) = &self.native_menu else {
            return;
        };
        for (item, command) in &native.checks {
            let state =
                if self.menu_check(command).unwrap_or(false) { NSControlStateValueOn } else { NSControlStateValueOff };
            if item.state() != state {
                item.setState(state);
            }
        }
    }
}
