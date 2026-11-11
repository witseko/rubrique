//! The menu bar along the top of the window: File, View and Settings. It
//! repeats the common settings (theme, text size, text area zoom, line
//! numbers and the rest) and the main commands, so they are one click away
//! without opening the settings window. Both change the same values.

use eframe::egui::{self, Context, KeyboardShortcut, Ui};

use super::{
    ASK, NEW_ENCRYPTED_SCRATCH, NEW_SCRATCH, OPEN_PATH, PAGE_DEFAULT, PAGE_LARGER, PAGE_SMALLER, RubriqueApp, SAVE,
    TOGGLE_PREVIEW, TOGGLE_SETTINGS,
};
use crate::settings::accessibility::{DEFAULT_PAGE_ZOOM, TextSize, page_zoom_larger, page_zoom_smaller};
use crate::theme::catalog::ThemeChoice;
use crate::theme::code_colors::CodeColors;

/// A menu entry with its shortcut written at the right. True when clicked.
fn item(ui: &mut Ui, label: &str, shortcut: Option<&KeyboardShortcut>) -> bool {
    let mut button = egui::Button::new(label);
    if let Some(shortcut) = shortcut {
        button = button.shortcut_text(ui.ctx().format_shortcut(shortcut));
    }
    ui.add(button).clicked()
}

impl RubriqueApp {
    /// The bar itself. Look settings changed in a menu apply at once.
    pub(super) fn menu_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut look_changed = false;
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| self.file_menu(ui, &ctx));
            ui.menu_button("View", |ui| look_changed |= self.view_menu(ui, &ctx));
            ui.menu_button("Settings", |ui| look_changed |= self.settings_menu(ui));
        });
        if look_changed {
            self.apply_look(&ctx);
        }
    }

    /// New files and folders, opening by path, Save.
    fn file_menu(&mut self, ui: &mut Ui, ctx: &Context) {
        if item(ui, "New scratch", Some(&NEW_SCRATCH)) {
            self.new_scratch(ctx, self.encrypt_new_scratches());
        }
        if item(ui, "New encrypted scratch", Some(&NEW_ENCRYPTED_SCRATCH)) {
            self.new_scratch(ctx, true);
        }
        ui.separator();
        if item(ui, "Add folder...", None) {
            self.choose_folder_to_add();
        }
        if item(ui, "Open path...", Some(&OPEN_PATH)) {
            self.start_open_path();
        }
        ui.separator();
        if item(ui, "Save", Some(&SAVE)) {
            self.save();
        }
        if item(ui, "Ask AI...", Some(&ASK)) {
            self.check_ai_tools();
            self.load_skills_and_agents();
            self.open_ask(ctx);
        }
    }

    /// Preview or source, text size, text area zoom, line numbers, wrapping,
    /// theme, code colors and the explorer filter. True when a look setting
    /// changed.
    fn view_menu(&mut self, ui: &mut Ui, ctx: &Context) -> bool {
        let mut changed = false;
        if item(ui, "Preview / source", Some(&TOGGLE_PREVIEW)) {
            self.toggle_preview();
        }
        ui.separator();
        ui.menu_button("Text size", |ui| {
            for size in TextSize::ALL {
                let selected = self.settings.accessibility.text_size == size;
                if ui.selectable_label(selected, size.name()).clicked() {
                    self.settings.accessibility.text_size = size;
                    changed = true;
                }
            }
        });
        ui.menu_button("Text area zoom", |ui| {
            let percent = self.settings.accessibility.page_zoom;
            let mut zoom = percent;
            if item(ui, "Larger", PAGE_LARGER.first()) {
                zoom = page_zoom_larger(percent);
            }
            if item(ui, "Smaller", Some(&PAGE_SMALLER)) {
                zoom = page_zoom_smaller(percent);
            }
            if item(ui, &format!("Reset ({DEFAULT_PAGE_ZOOM}%)"), Some(&PAGE_DEFAULT)) {
                zoom = DEFAULT_PAGE_ZOOM;
            }
            if zoom != percent {
                self.change_page_zoom(ctx, zoom);
            }
        });
        changed |= ui.checkbox(&mut self.settings.accessibility.line_numbers, "Line numbers").changed();
        changed |= ui.checkbox(&mut self.settings.accessibility.wrap_lines, "Wrap lines").changed();
        ui.separator();
        ui.menu_button("Theme", |ui| {
            let system = self.settings.theme == ThemeChoice::System;
            if ui.selectable_label(system, "System").clicked() {
                self.settings.theme = ThemeChoice::System;
                changed = true;
            }
            for theme in &self.themes.list {
                let choice = ThemeChoice::Named(theme.id.clone());
                if ui.selectable_label(self.settings.theme == choice, &theme.name).clicked() {
                    self.settings.theme = choice;
                    changed = true;
                }
            }
        });
        ui.menu_button("Code colors", |ui| {
            for colors in CodeColors::ALL {
                if ui.selectable_label(self.settings.code_colors == colors, colors.name()).clicked() {
                    self.settings.code_colors = colors;
                    changed = true;
                }
            }
        });
        ui.menu_button("Filter files", |ui| {
            let filter = &mut self.settings.explorer_filter;
            ui.checkbox(&mut filter.markdown_only, "Markdown files only");
            ui.checkbox(&mut filter.unsaved_only, "Unsaved edits only");
            ui.checkbox(&mut filter.show_closed_terminals, "Show closed terminals");
        });
        changed
    }

    /// The accessibility switches and the explorer's note marker, then the
    /// settings window for the rest. True when a look setting changed.
    fn settings_menu(&mut self, ui: &mut Ui) -> bool {
        let accessibility = &mut self.settings.accessibility;
        let mut changed = ui.checkbox(&mut accessibility.reduce_motion, "Reduce motion").changed();
        changed |= ui.checkbox(&mut accessibility.strong_focus, "Stronger focus outline").changed();
        ui.checkbox(&mut self.settings.note_marks, "Mark files that have notes");
        ui.separator();
        if item(ui, "All settings...", Some(&TOGGLE_SETTINGS)) {
            self.toggle_settings();
        }
        changed
    }
}
