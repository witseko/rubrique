//! The menus: File, View and Settings. They repeat the common settings
//! (theme, text size, text area zoom, line numbers and the rest) and the main
//! commands, so they are one click away without opening the settings window.
//! Both change the same values.
//!
//! The menus are described once ([`RubriqueApp::menus`]) and drawn in two
//! ways: on macOS in the system menu bar at the top of the screen
//! (`native_menu.rs`), elsewhere as a bar inside the window that Settings can
//! hide.

use eframe::egui::{Context, KeyboardShortcut};

use super::{
    ASK, NEW_ENCRYPTED_SCRATCH, NEW_SCRATCH, OPEN_PATH, PAGE_DEFAULT, PAGE_LARGER, PAGE_SMALLER, RubriqueApp, SAVE,
    TOGGLE_PREVIEW, TOGGLE_SETTINGS,
};
use crate::settings::accessibility::{DEFAULT_PAGE_ZOOM, TextSize, page_zoom_larger, page_zoom_smaller};
use crate::theme::catalog::ThemeChoice;
use crate::theme::code_colors::CodeColors;

/// Something a menu entry does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MenuCommand {
    NewScratch,
    NewEncryptedScratch,
    AddFolder,
    OpenPath,
    Save,
    Ask,
    TogglePreview,
    TextSize(TextSize),
    PageZoomLarger,
    PageZoomSmaller,
    PageZoomReset,
    LineNumbers,
    WrapLines,
    Theme(ThemeChoice),
    CodeColors(CodeColors),
    MarkdownOnly,
    UnsavedOnly,
    HideClosedTerminals,
    ReduceMotion,
    StrongFocus,
    NoteMarks,
    OpenSettings,
}

impl MenuCommand {
    /// True for one choice out of several (a text size, a theme): drawn as a
    /// pick rather than a check box in the window's bar.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn is_choice(&self) -> bool {
        matches!(self, MenuCommand::TextSize(_) | MenuCommand::Theme(_) | MenuCommand::CodeColors(_))
    }
}

/// One entry of a menu.
#[derive(Clone, Debug)]
pub(super) enum MenuEntry {
    /// A command, with the shortcut that also runs it (shown in the window's
    /// bar; the macOS menu leaves keys to the app, see `native_menu.rs`).
    Item {
        label: String,
        command: MenuCommand,
        #[cfg_attr(target_os = "macos", expect(dead_code, reason = "the macOS menu leaves keys to the app"))]
        shortcut: Option<KeyboardShortcut>,
    },
    Separator,
    Submenu {
        label: &'static str,
        entries: Vec<MenuEntry>,
    },
}

/// A plain entry.
fn item(label: &str, command: MenuCommand, shortcut: Option<&KeyboardShortcut>) -> MenuEntry {
    MenuEntry::Item { label: label.to_string(), command, shortcut: shortcut.copied() }
}

impl RubriqueApp {
    /// The menus, left to right, with their entries. Themes are read from
    /// the current list, so a reloaded themes folder shows up.
    pub(super) fn menus(&self) -> Vec<(&'static str, Vec<MenuEntry>)> {
        let file = vec![
            item("New scratch", MenuCommand::NewScratch, Some(&NEW_SCRATCH)),
            item("New encrypted scratch", MenuCommand::NewEncryptedScratch, Some(&NEW_ENCRYPTED_SCRATCH)),
            MenuEntry::Separator,
            item("Add folder...", MenuCommand::AddFolder, None),
            item("Open path...", MenuCommand::OpenPath, Some(&OPEN_PATH)),
            MenuEntry::Separator,
            item("Save", MenuCommand::Save, Some(&SAVE)),
            item("Ask AI...", MenuCommand::Ask, Some(&ASK)),
        ];
        let text_sizes = TextSize::ALL.into_iter().map(|size| item(size.name(), MenuCommand::TextSize(size), None));
        let zoom = vec![
            item("Larger", MenuCommand::PageZoomLarger, PAGE_LARGER.first()),
            item("Smaller", MenuCommand::PageZoomSmaller, Some(&PAGE_SMALLER)),
            item(&format!("Reset ({DEFAULT_PAGE_ZOOM}%)"), MenuCommand::PageZoomReset, Some(&PAGE_DEFAULT)),
        ];
        let mut themes = vec![item("System", MenuCommand::Theme(ThemeChoice::System), None)];
        themes.extend(
            self.themes
                .list
                .iter()
                .map(|theme| item(&theme.name, MenuCommand::Theme(ThemeChoice::Named(theme.id.clone())), None)),
        );
        let code_colors =
            CodeColors::ALL.into_iter().map(|colors| item(colors.name(), MenuCommand::CodeColors(colors), None));
        let filter = vec![
            item("Markdown files only", MenuCommand::MarkdownOnly, None),
            item("Unsaved edits only", MenuCommand::UnsavedOnly, None),
            item("Hide closed terminals", MenuCommand::HideClosedTerminals, None),
        ];
        let view = vec![
            item("Preview / source", MenuCommand::TogglePreview, Some(&TOGGLE_PREVIEW)),
            MenuEntry::Separator,
            MenuEntry::Submenu { label: "Text size", entries: text_sizes.collect() },
            MenuEntry::Submenu { label: "Text area zoom", entries: zoom },
            item("Line numbers", MenuCommand::LineNumbers, None),
            item("Wrap lines", MenuCommand::WrapLines, None),
            MenuEntry::Separator,
            MenuEntry::Submenu { label: "Theme", entries: themes },
            MenuEntry::Submenu { label: "Code colors", entries: code_colors.collect() },
            MenuEntry::Submenu { label: "Filter files", entries: filter },
        ];
        let settings = vec![
            item("Reduce motion", MenuCommand::ReduceMotion, None),
            item("Stronger focus outline", MenuCommand::StrongFocus, None),
            item("Mark files that have notes", MenuCommand::NoteMarks, None),
            MenuEntry::Separator,
            item("All settings...", MenuCommand::OpenSettings, Some(&TOGGLE_SETTINGS)),
        ];
        vec![("File", file), ("View", view), ("Settings", settings)]
    }

    /// Whether a switch or a choice is on; `None` for plain commands.
    pub(super) fn menu_check(&self, command: &MenuCommand) -> Option<bool> {
        let accessibility = &self.settings.accessibility;
        let filter = &self.settings.explorer_filter;
        let checked = match command {
            MenuCommand::TextSize(size) => accessibility.text_size == *size,
            MenuCommand::LineNumbers => accessibility.line_numbers,
            MenuCommand::WrapLines => accessibility.wrap_lines,
            MenuCommand::Theme(choice) => self.settings.theme == *choice,
            MenuCommand::CodeColors(colors) => self.settings.code_colors == *colors,
            MenuCommand::MarkdownOnly => filter.markdown_only,
            MenuCommand::UnsavedOnly => filter.unsaved_only,
            MenuCommand::HideClosedTerminals => filter.hide_closed_terminals,
            MenuCommand::ReduceMotion => accessibility.reduce_motion,
            MenuCommand::StrongFocus => accessibility.strong_focus,
            MenuCommand::NoteMarks => self.settings.note_marks,
            _ => return None,
        };
        Some(checked)
    }

    /// Do what a menu entry says. Look settings apply at once.
    pub(super) fn run_menu_command(&mut self, ctx: &Context, command: MenuCommand) {
        let accessibility = &mut self.settings.accessibility;
        let filter = &mut self.settings.explorer_filter;
        let page_zoom = accessibility.page_zoom;
        match command {
            MenuCommand::NewScratch => self.new_scratch(ctx, self.encrypt_new_scratches()),
            MenuCommand::NewEncryptedScratch => self.new_scratch(ctx, true),
            MenuCommand::AddFolder => self.choose_folder_to_add(),
            MenuCommand::OpenPath => self.start_open_path(),
            MenuCommand::Save => {
                self.save();
            }
            MenuCommand::Ask => {
                self.check_ai_tools();
                self.load_skills_and_agents();
                self.open_ask(ctx);
            }
            MenuCommand::TogglePreview => self.toggle_preview(),
            MenuCommand::PageZoomLarger => self.change_page_zoom(ctx, page_zoom_larger(page_zoom)),
            MenuCommand::PageZoomSmaller => self.change_page_zoom(ctx, page_zoom_smaller(page_zoom)),
            MenuCommand::PageZoomReset => self.change_page_zoom(ctx, DEFAULT_PAGE_ZOOM),
            MenuCommand::MarkdownOnly => filter.markdown_only = !filter.markdown_only,
            MenuCommand::UnsavedOnly => filter.unsaved_only = !filter.unsaved_only,
            MenuCommand::HideClosedTerminals => filter.hide_closed_terminals = !filter.hide_closed_terminals,
            MenuCommand::NoteMarks => self.settings.note_marks = !self.settings.note_marks,
            MenuCommand::OpenSettings => {
                if !self.settings_open {
                    self.toggle_settings();
                }
            }
            // The rest change how things look.
            MenuCommand::TextSize(size) => accessibility.text_size = size,
            MenuCommand::LineNumbers => accessibility.line_numbers = !accessibility.line_numbers,
            MenuCommand::WrapLines => accessibility.wrap_lines = !accessibility.wrap_lines,
            MenuCommand::ReduceMotion => accessibility.reduce_motion = !accessibility.reduce_motion,
            MenuCommand::StrongFocus => accessibility.strong_focus = !accessibility.strong_focus,
            MenuCommand::Theme(choice) => self.settings.theme = choice,
            MenuCommand::CodeColors(colors) => self.settings.code_colors = colors,
        }
        self.apply_look(ctx);
    }

    /// The bar inside the window (systems other than macOS).
    #[cfg(not(target_os = "macos"))]
    pub(super) fn menu_bar(&mut self, ui: &mut eframe::egui::Ui) {
        let ctx = ui.ctx().clone();
        let menus = self.menus();
        let mut clicked = None;
        eframe::egui::MenuBar::new().ui(ui, |ui| {
            for (label, entries) in &menus {
                ui.menu_button(*label, |ui| self.draw_entries(ui, entries, &mut clicked));
            }
        });
        if let Some(command) = clicked {
            self.run_menu_command(&ctx, command);
        }
    }

    /// Draw `entries` in an open menu; the clicked command goes to `clicked`.
    #[cfg(not(target_os = "macos"))]
    fn draw_entries(&self, ui: &mut eframe::egui::Ui, entries: &[MenuEntry], clicked: &mut Option<MenuCommand>) {
        use eframe::egui;
        for entry in entries {
            match entry {
                MenuEntry::Separator => {
                    ui.separator();
                }
                MenuEntry::Submenu { label, entries } => {
                    ui.menu_button(*label, |ui| self.draw_entries(ui, entries, clicked));
                }
                MenuEntry::Item { label, command, shortcut } => {
                    let response = match self.menu_check(command) {
                        Some(on) if command.is_choice() => ui.selectable_label(on, label.as_str()),
                        Some(mut on) => ui.checkbox(&mut on, label.as_str()),
                        None => {
                            let mut button = egui::Button::new(label.as_str());
                            if let Some(shortcut) = shortcut {
                                button = button.shortcut_text(ui.ctx().format_shortcut(shortcut));
                            }
                            ui.add(button)
                        }
                    };
                    if response.clicked() {
                        *clicked = Some(command.clone());
                    }
                }
            }
        }
    }
}
