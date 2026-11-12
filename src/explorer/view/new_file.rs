//! The row that names a new file in the tree: a name field, a dot and an
//! extension field, both empty, with the common extensions as buttons
//! under them. Enter in the name field moves on to the extension; Enter
//! there, or a click on an extension button, creates the file.

use eframe::egui::{self, Key, RichText, Ui};

use super::{InlineRename, NOTE_MARK_SIZE, NewEntry, NewKind, rename_row};

/// The extensions offered as buttons under a new file's name.
const FILE_EXTENSIONS: [&str; 7] = ["md", "txt", "sql", "py", "ts", "rs", "go"];

/// Width of the name field, in points.
const NAME_FIELD_WIDTH: f32 = 130.0;
/// Width of the extension field: room for about five letters.
const EXTENSION_FIELD_WIDTH: f32 = 48.0;
/// Gap between two extension buttons, in points.
const CHOICE_GAP: f32 = 3.0;

/// Which of a new file's two fields is being typed in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum NamePart {
    #[default]
    Name,
    Extension,
}

impl InlineRename<NewEntry> {
    /// The name to create: for a file, the name and the extension joined
    /// with one dot (just the name when no extension was given). Empty
    /// when no name was typed, so the caller reports that.
    pub fn full_name(&self) -> String {
        let name = self.name.trim();
        // ".md" typed into the extension field means "md".
        let extension = self.target.extension.trim().trim_start_matches('.');
        if self.target.kind != NewKind::File || name.is_empty() || extension.is_empty() {
            return name.to_string();
        }
        // "plan." and "md" make "plan.md", not "plan..md".
        format!("{}.{extension}", name.trim_end_matches('.'))
    }
}

/// The name row of the entry being created: two fields for a file, the
/// usual single field for a folder or a terminal.
pub(super) fn new_entry_row(ui: &mut Ui, slot: &mut Option<InlineRename<NewEntry>>) {
    match slot.as_ref().map(|entry| entry.target.kind) {
        Some(NewKind::File) => new_file_row(ui, slot),
        Some(NewKind::Folder | NewKind::Terminal) => rename_row(ui, slot),
        None => {}
    }
}

/// What the widgets of the new file row reported this frame.
struct RowOutput {
    name: egui::Response,
    extension: egui::Response,
    /// An extension button was clicked (the extension field now holds it).
    picked: bool,
    /// An extension button has the keyboard focus (after Tab).
    choice_focused: bool,
}

/// Draw the fields and buttons for a new file, and mark the entry submitted
/// when it should be created; Esc clears `slot`.
fn new_file_row(ui: &mut Ui, slot: &mut Option<InlineRename<NewEntry>>) {
    let Some(entry) = slot.as_mut() else {
        return;
    };
    let row = ui.scope(|ui| {
        let (name, extension) = name_fields(ui, entry);
        let (picked, choice_focused) = extension_choices(ui, &mut entry.target.extension);
        if let Some(error) = &entry.error {
            ui.label(RichText::new(error).small().color(ui.visuals().error_fg_color));
        }
        RowOutput { name, extension, picked, choice_focused }
    });
    let output = row.inner;

    if std::mem::take(&mut entry.needs_focus) {
        // A failed attempt with no name goes back to the name field.
        if entry.name.trim().is_empty() {
            entry.target.part = NamePart::Name;
        }
        match entry.target.part {
            NamePart::Name => output.name.request_focus(),
            NamePart::Extension => output.extension.request_focus(),
        }
        return;
    }
    if output.picked {
        entry.submitted = true;
        return;
    }

    let (enter, escape) = ui.input(|input| (input.key_pressed(Key::Enter), input.key_pressed(Key::Escape)));
    let left_field = output.name.lost_focus() || output.extension.lost_focus();
    if left_field && escape {
        *slot = None;
        return;
    }
    if output.name.lost_focus() && enter {
        // A name typed whole ("plan.md") needs no second step.
        if has_extension(&entry.name) {
            entry.submitted = true;
        } else {
            entry.target.part = NamePart::Extension;
            output.extension.request_focus();
        }
        return;
    }
    if output.extension.lost_focus() && enter {
        entry.submitted = true;
        return;
    }

    // Follow the focus when Tab or a click moved it between the fields.
    if output.name.has_focus() {
        entry.target.part = NamePart::Name;
    } else if output.extension.has_focus() {
        entry.target.part = NamePart::Extension;
    } else if !output.choice_focused && !click_in_progress(ui, row.response.rect) {
        // The row was left: create what was typed, like the other name
        // fields do, or drop the row when nothing was.
        if entry.name.trim().is_empty() {
            *slot = None;
        } else {
            entry.submitted = true;
        }
    }
}

/// The name field, a dot and the extension field on one line.
fn name_fields(ui: &mut Ui, entry: &mut InlineRename<NewEntry>) -> (egui::Response, egui::Response) {
    ui.horizontal(|ui| {
        // Line the fields up with the names in the other rows.
        ui.add_space(ui.spacing().button_padding.x + NOTE_MARK_SIZE);
        let name = ui.add(
            egui::TextEdit::singleline(&mut entry.name)
                .id(egui::Id::unique("explorer-new-file-name"))
                .hint_text("name")
                .desired_width(NAME_FIELD_WIDTH),
        );
        // Not selectable, so Tab goes from the name straight to the extension.
        ui.add(egui::Label::new(".").selectable(false));
        let extension = ui.add(
            egui::TextEdit::singleline(&mut entry.target.extension)
                .id(egui::Id::unique("explorer-new-file-extension"))
                .hint_text("ext")
                .desired_width(EXTENSION_FIELD_WIDTH),
        );
        (name, extension)
    })
    .inner
}

/// One small button per common extension. A click puts that extension in
/// `extension`. Returns whether one was clicked, and whether one has the
/// keyboard focus.
fn extension_choices(ui: &mut Ui, extension: &mut String) -> (bool, bool) {
    let mut picked = false;
    let mut focused = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = CHOICE_GAP;
        ui.add_space(ui.spacing().button_padding.x + NOTE_MARK_SIZE);
        for choice in FILE_EXTENSIONS {
            let response = ui.small_button(choice).on_hover_text(format!("Create it as a .{choice} file"));
            if response.clicked() {
                *extension = choice.to_string();
                picked = true;
            }
            focused |= response.has_focus();
        }
    });
    (picked, focused)
}

/// True while a mouse button is held or was just released over `row`: a
/// click on an extension button takes the focus from the fields a frame
/// before it counts as a click, and must not look like leaving the row.
fn click_in_progress(ui: &Ui, row: egui::Rect) -> bool {
    ui.rect_contains_pointer(row) && ui.input(|input| input.pointer.any_down() || input.pointer.any_released())
}

/// True when `name` already ends in an extension ("plan.md"). A name that
/// only starts with a dot (".env") or ends with one ("plan.") has none.
fn has_extension(name: &str) -> bool {
    let name = name.trim();
    name.rfind('.').is_some_and(|dot| dot > 0 && dot + 1 < name.len())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry(kind: NewKind, name: &str, extension: &str) -> InlineRename<NewEntry> {
        let mut entry = InlineRename::with_name(NewEntry::new(PathBuf::from("/notes"), kind), name.to_string());
        entry.target.extension = extension.to_string();
        entry
    }

    #[test]
    fn joins_a_file_name_and_its_extension() {
        assert_eq!(entry(NewKind::File, " plan ", " md ").full_name(), "plan.md");
        assert_eq!(entry(NewKind::File, "plan.", ".md").full_name(), "plan.md");
        assert_eq!(entry(NewKind::File, "Makefile", "").full_name(), "Makefile");
        assert_eq!(entry(NewKind::File, ".env", "").full_name(), ".env");
        assert_eq!(entry(NewKind::File, "plan.md", "").full_name(), "plan.md");
    }

    #[test]
    fn a_file_with_no_name_has_no_full_name() {
        assert_eq!(entry(NewKind::File, "  ", "md").full_name(), "");
    }

    #[test]
    fn folders_and_terminals_ignore_the_extension() {
        assert_eq!(entry(NewKind::Folder, "docs", "md").full_name(), "docs");
        assert_eq!(entry(NewKind::Terminal, "build.tmux", "md").full_name(), "build.tmux");
    }

    /// Draw the new file row once per step, feeding it that step's events,
    /// with an idle frame after each so focus changes settle. Returns the
    /// entry (or `None` after Esc) and the row's area in the last frame.
    fn row_frames(steps: Vec<Vec<egui::Event>>) -> (Option<InlineRename<NewEntry>>, egui::Rect) {
        let mut slot = Some(entry(NewKind::File, "", ""));
        let ctx = egui::Context::default();
        let mut area = egui::Rect::NOTHING;
        let frames = steps.into_iter().flat_map(|events| [events, Vec::new()]);
        for events in std::iter::once(Vec::new()).chain(frames) {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 600.0));
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let top = ui.cursor().min;
                    new_file_row(ui, &mut slot);
                    area = egui::Rect::from_min_max(top, egui::pos2(ui.max_rect().right(), ui.cursor().top()));
                });
            });
            output.textures_delta.clear();
        }
        (slot, area)
    }

    fn typed(text: &str) -> egui::Event {
        egui::Event::Text(text.to_string())
    }

    fn pressed(key: Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    fn mouse(pos: egui::Pos2, pressed: bool) -> egui::Event {
        let (button, modifiers) = (egui::PointerButton::Primary, egui::Modifiers::NONE);
        egui::Event::PointerButton { pos, button, pressed, modifiers }
    }

    #[test]
    fn enter_moves_from_the_name_to_the_extension_then_creates() {
        let (slot, _) = row_frames(vec![vec![typed("plan")], vec![pressed(Key::Enter)]]);
        let waiting = slot.expect("the row stays open");
        assert!(!waiting.submitted);
        assert_eq!(waiting.target.part, NamePart::Extension);

        let steps = vec![vec![typed("plan")], vec![pressed(Key::Enter)], vec![typed("sql")], vec![pressed(Key::Enter)]];
        let (slot, _) = row_frames(steps);
        let done = slot.expect("the row stays until the app creates the file");
        assert!(done.submitted);
        assert_eq!(done.full_name(), "plan.sql");
    }

    #[test]
    fn a_name_typed_whole_is_created_on_the_first_enter() {
        let (slot, _) = row_frames(vec![vec![typed("plan.md")], vec![pressed(Key::Enter)]]);
        let done = slot.expect("the row stays until the app creates the file");
        assert!(done.submitted);
        assert_eq!(done.full_name(), "plan.md");
    }

    #[test]
    fn escape_closes_the_row() {
        let (slot, _) = row_frames(vec![vec![typed("plan")], vec![pressed(Key::Escape)]]);
        assert!(slot.is_none());
    }

    #[test]
    fn a_click_on_an_extension_button_creates_the_file() {
        let (_, area) = row_frames(Vec::new());
        // The buttons are the last line of the row; "md" is the first,
        // right after the indent.
        let indent = egui::Context::default().global_style().spacing.button_padding.x + NOTE_MARK_SIZE;
        let md = egui::pos2(area.left() + indent + 8.0, area.bottom() - 14.0);
        let steps = vec![
            vec![typed("plan")],
            vec![egui::Event::PointerMoved(md)],
            vec![mouse(md, true)],
            vec![mouse(md, false)],
        ];
        let (slot, _) = row_frames(steps);
        let done = slot.expect("the row stays until the app creates the file");
        assert!(done.submitted);
        assert_eq!(done.full_name(), "plan.md");
    }

    #[test]
    fn a_click_outside_creates_what_was_typed_or_drops_an_empty_row() {
        let outside = egui::pos2(400.0, 400.0);
        let click =
            || vec![vec![egui::Event::PointerMoved(outside)], vec![mouse(outside, true)], vec![mouse(outside, false)]];
        let (slot, _) = row_frames(click());
        assert!(slot.is_none());

        let mut steps = vec![vec![typed("Makefile")]];
        steps.extend(click());
        let (slot, _) = row_frames(steps);
        let done = slot.expect("the row stays until the app creates the file");
        assert!(done.submitted);
        assert_eq!(done.full_name(), "Makefile");
    }

    #[test]
    fn knows_a_name_typed_with_its_extension() {
        assert!(has_extension("plan.md"));
        assert!(has_extension(" a.b.rs "));
        assert!(!has_extension("plan"));
        assert!(!has_extension(".env"));
        assert!(!has_extension("plan."));
    }
}
