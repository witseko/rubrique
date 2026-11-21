//! Full screen for a terminal tab: the window fills the screen and only the
//! terminal shows, with no explorer, tabs, side panel, menu or status bar.
//! Cmd+Shift+Enter (Ctrl+Shift+Enter on Linux), the View menu or the status
//! bar button turns it on and off. It ends when another kind of tab becomes
//! active, and the window goes back to the size it had.

use eframe::egui::{self, Context, Key, KeyboardShortcut, Modifiers, RichText, Ui, ViewportCommand};

use super::{OpenFile, RubriqueApp};
use crate::icons::{Icon, icon_button};

/// Turn terminal full screen on or off. Enter alone goes to the terminal;
/// with Cmd and Shift it never reaches it, as the app takes it first.
pub(super) const TOGGLE_TERMINAL_FULL_SCREEN: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Enter);
/// How long the "how to leave" hint stays after full screen starts.
const HINT_SECONDS: f64 = 3.0;

/// Whether terminal full screen is on, and what to restore when it ends.
#[derive(Debug, Default)]
pub(super) struct TerminalFullScreen {
    /// The user asked for it (it shows only while a terminal tab is active).
    on: bool,
    /// While it shows: whether the window was already full screen before,
    /// so leaving keeps a window the user made full screen themselves.
    window_was_full_screen: Option<bool>,
    /// When it started (egui time), for the hint; NaN until the next frame reads the clock.
    started_at: f64,
}

impl TerminalFullScreen {
    /// True while the terminal fills the window.
    pub(super) fn is_showing(&self) -> bool {
        self.window_was_full_screen.is_some()
    }

    /// Bring the state in step with the active tab. Returns the full screen
    /// state to give the window, when it has to change.
    fn update(&mut self, terminal_active: bool, window_full_screen: bool) -> Option<bool> {
        if !terminal_active {
            self.on = false;
        }
        match (self.on, self.window_was_full_screen) {
            (true, None) => {
                self.window_was_full_screen = Some(window_full_screen);
                self.started_at = f64::NAN;
                (!window_full_screen).then_some(true)
            }
            (false, Some(was)) => {
                self.window_was_full_screen = None;
                (!was).then_some(false)
            }
            _ => None,
        }
    }
}

impl RubriqueApp {
    /// The shortcut, the View menu entry and the status bar button.
    pub(super) fn toggle_terminal_full_screen(&mut self) {
        if !matches!(self.open_file, OpenFile::Terminal { .. }) {
            self.show_status("Full screen works in a terminal tab".to_string());
            return;
        }
        self.terminal_full_screen.on = !self.terminal_full_screen.on;
    }

    /// Whether the menus show terminal full screen as on.
    pub(super) fn terminal_full_screen_on(&self) -> bool {
        self.terminal_full_screen.on
    }

    /// Run the shortcut, end full screen when the active tab is no longer a
    /// terminal, and make the window full screen or put it back. Called
    /// every frame, before the panels are drawn.
    pub(super) fn sync_terminal_full_screen(&mut self, ctx: &Context) {
        let terminal_active = matches!(self.open_file, OpenFile::Terminal { .. });
        // Only in a terminal tab: elsewhere Cmd+Enter fields (a note, a block
        // being edited) read these keys too.
        if terminal_active && ctx.input_mut(|input| input.consume_shortcut(&TOGGLE_TERMINAL_FULL_SCREEN)) {
            self.toggle_terminal_full_screen();
        }
        let window_full_screen = ctx.input(|input| input.viewport().fullscreen.unwrap_or(false));
        if let Some(full_screen) = self.terminal_full_screen.update(terminal_active, window_full_screen) {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(full_screen));
        }
    }

    /// The status bar of a terminal tab, right to left: the full screen
    /// button in the corner, then the `.tmux` file the session came from.
    pub(super) fn terminal_status(&mut self, ui: &mut Ui) {
        let keys = ui.ctx().format_shortcut(&TOGGLE_TERMINAL_FULL_SCREEN);
        let tip = format!("Full screen ({keys})");
        if icon_button(ui, Icon::FullScreen, "Full screen", &tip, false).clicked() {
            self.toggle_terminal_full_screen();
        }
        if let Some(path) = self.open_file.path() {
            let path_text = crate::remote::display(path);
            let left = egui::Layout::left_to_right(egui::Align::Center);
            ui.allocate_ui_with_layout(ui.available_size(), left, |ui| {
                ui.add(egui::Label::new(path_text.as_str()).truncate()).on_hover_text(path_text.as_str());
            });
        }
    }

    /// For a few seconds after full screen starts, say how to leave it.
    pub(super) fn terminal_full_screen_hint(&mut self, ctx: &Context) {
        if !self.terminal_full_screen.is_showing() {
            return;
        }
        let now = ctx.input(|input| input.time);
        let started_at = &mut self.terminal_full_screen.started_at;
        if started_at.is_nan() {
            *started_at = now;
        }
        let remaining_seconds = HINT_SECONDS - (now - *started_at);
        if remaining_seconds <= 0.0 {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining_seconds));
        let keys = ctx.format_shortcut(&TOGGLE_TERMINAL_FULL_SCREEN);
        egui::Area::new(egui::Id::unique("terminal-full-screen-hint"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 16.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(RichText::new(format!("Press {keys} to leave full screen")));
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalFullScreen;

    fn turned_on() -> TerminalFullScreen {
        TerminalFullScreen { on: true, ..TerminalFullScreen::default() }
    }

    #[test]
    fn makes_the_window_full_screen_and_puts_it_back() {
        let mut full_screen = turned_on();
        assert_eq!(full_screen.update(true, false), Some(true));
        assert!(full_screen.is_showing());
        assert_eq!(full_screen.update(true, true), None);
        full_screen.on = false;
        assert_eq!(full_screen.update(true, true), Some(false));
        assert!(!full_screen.is_showing());
    }

    #[test]
    fn keeps_a_window_that_was_already_full_screen() {
        let mut full_screen = turned_on();
        assert_eq!(full_screen.update(true, true), None);
        assert!(full_screen.is_showing());
        full_screen.on = false;
        assert_eq!(full_screen.update(true, true), None);
        assert!(!full_screen.is_showing());
    }

    #[test]
    fn ends_when_another_kind_of_tab_becomes_active() {
        let mut full_screen = turned_on();
        full_screen.update(true, false);
        assert_eq!(full_screen.update(false, true), Some(false));
        assert!(!full_screen.on);
        // Coming back to a terminal does not turn it on again.
        assert_eq!(full_screen.update(true, false), None);
    }

    #[test]
    fn stays_off_until_asked() {
        let mut full_screen = TerminalFullScreen::default();
        assert_eq!(full_screen.update(true, false), None);
        assert!(!full_screen.is_showing());
    }
}
