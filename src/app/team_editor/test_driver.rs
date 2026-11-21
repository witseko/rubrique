//! Drive a team editor view headless: run frames, find text on screen,
//! click it and type into it.

use eframe::egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, Ui, Vec2};

use super::common::TeamEditor;
use crate::document::text_file::{Document, Opened};
use crate::test_support::TempFolder;

/// One view of one team editor, with what the last frame drew.
pub(in crate::app::team_editor) struct Driver {
    pub ctx: Context,
    pub editor: TeamEditor,
    view: fn(&mut Ui, &mut TeamEditor),
    /// Each text drawn in the last frame and where.
    texts: Vec<(String, Rect)>,
    _temp: TempFolder,
}

/// Each text in `shape`, in drawing order, with where it is.
fn collect_texts(shape: Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Vec(inner) => {
            for shape in inner {
                collect_texts(shape, texts);
            }
        }
        Shape::Text(text) => {
            texts.push((text.galley.text().to_owned(), text.galley.rect.translate(text.pos.to_vec2())));
        }
        _ => {}
    }
}

impl Driver {
    /// A driver for `view` over the spec `text`; a built-in (read-only) team when `read_only`.
    pub(in crate::app::team_editor) fn new(text: &str, read_only: bool, view: fn(&mut Ui, &mut TeamEditor)) -> Self {
        let temp = TempFolder::new();
        let place = if read_only { "openrig/packages/daemon/specs/rigs/team/rig.yaml" } else { "my-team/rig.yaml" };
        let path = temp.write(place, text);
        let Ok(Opened::Text(document)) = Document::open(&path, None) else { panic!("a text file") };
        let editor = TeamEditor::new(&document);
        assert_eq!(editor.read_only, read_only);
        let mut driver = Self { ctx: Context::default(), editor, view, texts: Vec::new(), _temp: temp };
        driver.frame(Vec::new());
        driver.frame(Vec::new());
        driver
    }

    /// Run one frame with `events`.
    pub(in crate::app::team_editor) fn frame(&mut self, events: Vec<Event>) {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 2400.0));
        let input = RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let (view, editor) = (self.view, &mut self.editor);
        let mut output = self.ctx.run_ui(input, |ui| view(ui, editor));
        // No screen here: the font texture is never uploaded.
        output.textures_delta.clear();
        self.texts.clear();
        for clipped in output.shapes {
            collect_texts(clipped.shape, &mut self.texts);
        }
    }

    /// Where each drawn text starting with `start` is, in drawing order.
    pub(in crate::app::team_editor) fn find(&self, start: &str) -> Vec<Rect> {
        self.texts.iter().filter(|(text, _)| text.starts_with(start)).map(|(_, rect)| *rect).collect()
    }

    /// Click at `pos`: move there, press, release, then one more frame.
    pub(in crate::app::team_editor) fn click_at(&mut self, pos: Pos2) {
        let button =
            |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.frame(Vec::new());
    }

    /// Click the last drawn text starting with `start` (a popup's item
    /// is drawn after the page under it).
    pub(in crate::app::team_editor) fn click_text(&mut self, start: &str) {
        let rect = *self.find(start).last().unwrap_or_else(|| panic!("no text starting with {start:?}"));
        self.click_at(rect.center());
    }

    /// Click the `nth` (from 0) drawn text starting with `start`.
    pub(in crate::app::team_editor) fn click_nth_text(&mut self, start: &str, nth: usize) {
        let rect = self.find(start)[nth];
        self.click_at(rect.center());
    }

    /// Replace the text of the focused field with `text`.
    pub(in crate::app::team_editor) fn replace_text(&mut self, text: &str) {
        let select_all =
            Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
        self.frame(vec![select_all, Event::Text(text.to_owned())]);
        self.frame(Vec::new());
    }
}
