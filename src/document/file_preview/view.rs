//! Draw a preview and its page, sheet, and system-open controls.

use std::path::Path;
use std::time::Duration;

use eframe::egui::{self, Ui};

use super::{Content, FilePreview, open_source};

impl FilePreview {
    /// Show the selected page, sheet, or text in the current tab.
    pub fn show(&mut self, ui: &mut Ui, path: &Path) -> Option<String> {
        if self.loaded.is_none() && self.pending.is_none() && self.error.is_none() {
            self.start(ui.ctx(), path);
        }
        self.poll();

        let (selected, status) = self.toolbar(ui, path);
        if selected != self.selected {
            self.selected = selected;
            self.start(ui.ctx(), path);
        }
        ui.separator();
        if self.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading preview…");
            });
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        } else if let Some(error) = &self.error {
            ui.label(error);
        } else {
            self.show_content(ui, path);
        }
        status
    }

    /// Draw controls for the displayed format and return the requested page or sheet.
    fn toolbar(&mut self, ui: &mut Ui, path: &Path) -> (usize, Option<String>) {
        let mut selected = self.selected;
        let mut status = None;
        ui.horizontal(|ui| {
            if ui.button("Open in default app").clicked()
                && let Err(error) = open_source(path)
            {
                status = Some(error);
            }
            if self.error.is_some() && ui.button("Reload preview").clicked() {
                *self = Self::new(self.kind);
                self.start(ui.ctx(), path);
            }
            let Some(loaded) = &self.loaded else {
                return;
            };
            ui.add_enabled_ui(self.pending.is_none() && self.error.is_none(), |ui| match &loaded.content {
                Content::Page { page_count, .. } => {
                    selected = page_selector(ui, selected, *page_count);
                }
                Content::Sheet { names, .. } => {
                    egui::ComboBox::from_id_salt(("workbook-sheet", path)).selected_text(&names[selected]).show_ui(
                        ui,
                        |ui| {
                            for (index, name) in names.iter().enumerate() {
                                ui.selectable_value(&mut selected, index, name);
                            }
                        },
                    );
                }
                Content::Text(_) => {}
            });
        });
        (if self.loaded.is_some() { selected } else { self.selected }, status)
    }

    /// Draw the current content with a separate scroll position per page or sheet.
    fn show_content(&mut self, ui: &mut Ui, path: &Path) {
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        ui.push_id((path, self.selected), |ui| match &mut loaded.content {
            Content::Text(text) => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if text.trim().is_empty() {
                        ui.weak("No text to display.");
                    } else {
                        ui.add(egui::Label::new(text.as_str()).wrap().selectable(true));
                    }
                });
            }
            Content::Sheet { table, .. } => table.show(ui),
            Content::Page { pixels, .. } => {
                if let Some(image) = pixels.take() {
                    self.texture = Some(ui.ctx().load_texture("PDF page", image, egui::TextureOptions::LINEAR));
                }
                if let Some(texture) = &self.texture {
                    show_page(ui, texture);
                }
            }
        });
    }
}

/// Move one page backward or forward without going outside the document.
fn page_selector(ui: &mut Ui, mut page: usize, pages: usize) -> usize {
    if ui.add_enabled(page > 0, egui::Button::new("Previous")).clicked() {
        page -= 1;
    }
    ui.label(format!("Page {} of {pages}", page + 1));
    if ui.add_enabled(page + 1 < pages, egui::Button::new("Next")).clicked() {
        page += 1;
    }
    page
}

/// Fit the page to the reader width and let the user scroll down the page.
fn show_page(ui: &mut Ui, texture: &egui::TextureHandle) {
    let width = ui.available_width().max(1.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add(egui::Image::from_texture(texture).fit_to_original_size(1.0).max_width(width));
        });
    });
}
