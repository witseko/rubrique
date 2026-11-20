//! Draw voice memos: the player row in the preview and the bar shown while
//! recording. They only draw and report what was clicked.

use std::collections::HashMap;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Ui, vec2};

use super::MemoLink;
use crate::ai::format_elapsed;
use crate::icons::{Icon, icon_button, icon_text_button};

/// What the preview knows about each memo, by its link target. The app
/// fills it in before drawing.
#[derive(Clone, Debug, Default)]
pub struct MemoStates {
    pub by_url: HashMap<String, MemoState>,
}

/// One memo's state.
#[derive(Clone, Debug, Default, PartialEq)]
#[expect(clippy::struct_excessive_bools, reason = "independent facts about one memo")]
pub struct MemoState {
    /// The file is not there.
    pub missing: bool,
    /// Rubrique can play it (WAV); other files open in the system player.
    pub playable: bool,
    /// Length in seconds, when known.
    pub duration: Option<f64>,
    /// Where playback is, in seconds.
    pub position: f64,
    pub playing: bool,
    /// Seconds since a transcription of this memo started.
    pub transcribing: Option<u64>,
    /// Another memo is being transcribed.
    pub busy: bool,
}

/// A button clicked on a memo in the preview.
#[derive(Clone, Debug, PartialEq)]
pub enum MemoAction {
    Play(String),
    Pause,
    /// Go to this many seconds into the memo.
    Seek(String, f64),
    Transcribe(String),
    StopTranscribing,
    /// Open in the system's audio player.
    Open(String),
}

fn states_id() -> egui::Id {
    egui::Id::unique("rubrique-memo-states")
}

fn action_id() -> egui::Id {
    egui::Id::unique("rubrique-memo-action")
}

/// Hand the memo states to the preview for this frame.
pub fn set_memo_states(ctx: &egui::Context, states: MemoStates) {
    ctx.data_mut(|data| data.insert_temp(states_id(), states));
}

/// The memo button clicked this frame, if any (taken once).
pub fn take_memo_action(ctx: &egui::Context) -> Option<MemoAction> {
    ctx.data_mut(|data| {
        let action = data.get_temp::<MemoAction>(action_id());
        data.remove::<MemoAction>(action_id());
        action
    })
}

/// Height of the seek bar's track.
const TRACK_HEIGHT: f32 = 4.0;
/// Radius of the knob on the seek bar.
const KNOB_RADIUS: f32 = 5.5;
const ROW_RADIUS: u8 = 8;
const LABEL_SIZE: f32 = 14.0;
const SMALL_SIZE: f32 = 11.5;

/// A memo in the preview: play or open, its name and time, a seek bar, and
/// Transcribe.
pub fn memo_row(ui: &mut Ui, link: &MemoLink) {
    let state = ui
        .ctx()
        .data(|data| data.get_temp::<MemoStates>(states_id()))
        .and_then(|states| states.by_url.get(&link.url).cloned())
        .unwrap_or_default();
    let visuals = ui.visuals().clone();
    let mut action = None;
    egui::Frame::new()
        .fill(visuals.code_bg_color)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(ROW_RADIUS)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                action = play_button(ui, link, &state);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let label = if link.label.is_empty() { "Voice memo" } else { link.label.as_str() };
                    crate::markdown::preview_find::label(ui, RichText::new(label).size(LABEL_SIZE).strong());
                    ui.label(RichText::new(time_text(&link.url, &state)).size(SMALL_SIZE).weak());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(found) = transcribe_controls(ui, link, &state) {
                        action = Some(found);
                    }
                });
            });
            if state.playable
                && let Some(duration) = state.duration.filter(|duration| *duration > 0.0)
                && let Some(seconds) = seek_bar(ui, state.position, duration)
            {
                action = Some(MemoAction::Seek(link.url.clone(), seconds));
            }
        });
    if let Some(action) = action {
        ui.ctx().data_mut(|data| data.insert_temp(action_id(), action));
    }
}

/// Play / Pause, or Open for files Rubrique cannot play.
fn play_button(ui: &mut Ui, link: &MemoLink, state: &MemoState) -> Option<MemoAction> {
    if state.missing {
        ui.add_enabled_ui(false, |ui| icon_button(ui, Icon::Play, "Play", "The file is missing", false));
        return None;
    }
    if !state.playable {
        let open = icon_button(ui, Icon::Play, "Open", "Open in your audio player (Rubrique plays WAV files)", false);
        return open.clicked().then(|| MemoAction::Open(link.url.clone()));
    }
    if state.playing {
        icon_button(ui, Icon::Pause, "Pause", "Pause", true).clicked().then_some(MemoAction::Pause)
    } else {
        icon_button(ui, Icon::Play, "Play", "Play", false).clicked().then(|| MemoAction::Play(link.url.clone()))
    }
}

/// "0:12 / 1:30", or why the memo cannot play.
fn time_text(url: &str, state: &MemoState) -> String {
    let seconds = |value: f64| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "memo lengths are small and positive"
        )]
        let whole = value.max(0.0) as u64;
        format_elapsed(whole)
    };
    if state.missing {
        return format!("File not found: {}", super::decode_url(url));
    }
    let name = super::decode_url(url.rsplit('/').next().unwrap_or(url));
    match (state.playable, state.duration) {
        (true, Some(duration)) if state.playing || state.position > 0.0 => {
            format!("{} / {}", seconds(state.position), seconds(duration))
        }
        (true, Some(duration)) => format!("{} · {name}", seconds(duration)),
        _ => format!("{name} · opens in your audio player"),
    }
}

/// Transcribe, or the time and Stop while it runs.
fn transcribe_controls(ui: &mut Ui, link: &MemoLink, state: &MemoState) -> Option<MemoAction> {
    if let Some(seconds) = state.transcribing {
        let stop = icon_text_button(ui, Icon::Stop, "Stop", "Stop transcribing", false).clicked();
        ui.label(RichText::new(format!("Transcribing · {}", format_elapsed(seconds))).size(SMALL_SIZE).weak());
        ui.add(egui::Spinner::new().size(12.0));
        return stop.then_some(MemoAction::StopTranscribing);
    }
    let mut action = None;
    ui.add_enabled_ui(!state.missing && !state.busy, |ui| {
        let tip = "Turn the speech into text under the memo (Settings → Voice memos); you review it first";
        if icon_text_button(ui, Icon::Sparkle, "Transcribe", tip, false).clicked() {
            action = Some(MemoAction::Transcribe(link.url.clone()));
        }
    });
    action
}

/// A thin track with a knob; returns the seconds clicked or dragged to.
fn seek_bar(ui: &mut Ui, position: f64, duration: f64) -> Option<f64> {
    ui.add_space(4.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), KNOB_RADIUS * 2.0 + 2.0), Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let track = egui::Rect::from_center_size(rect.center(), vec2(rect.width() - KNOB_RADIUS * 2.0, TRACK_HEIGHT));
    #[expect(clippy::cast_possible_truncation, reason = "a fraction between 0 and 1")]
    let fraction = (position / duration).clamp(0.0, 1.0) as f32;
    let visuals = ui.visuals();
    let painter = ui.painter();
    painter.rect_filled(track, TRACK_HEIGHT / 2.0, visuals.widgets.noninteractive.bg_stroke.color);
    let knob_x = track.left() + track.width() * fraction;
    let played = egui::Rect::from_min_max(track.min, egui::pos2(knob_x, track.max.y));
    painter.rect_filled(played, TRACK_HEIGHT / 2.0, visuals.hyperlink_color);
    let knob = egui::pos2(knob_x, track.center().y);
    let knob_radius = if response.hovered() || response.dragged() { KNOB_RADIUS } else { KNOB_RADIUS - 1.5 };
    painter.circle(knob, knob_radius, visuals.hyperlink_color, Stroke::new(1.5, visuals.code_bg_color));
    let pressed = response.interact_pointer_pos()?;
    if !(response.clicked() || response.dragged() || response.drag_stopped()) {
        return None;
    }
    let picked = ((pressed.x - track.left()) / track.width()).clamp(0.0, 1.0);
    Some(f64::from(picked) * duration)
}

/// Button on the recording bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingChoice {
    /// Stop and add the memo.
    Stop,
    /// Stop and delete what was recorded.
    Cancel,
}

/// What the recording bar shows.
#[derive(Clone, Debug)]
pub struct RecordingInfo {
    /// The recording will be attached to a selected passage.
    pub voice_note: bool,
    pub seconds: u64,
    /// Smoothed loudness, 0.0 to 1.0.
    pub level: f32,
    /// After a few seconds of silence: a hint about the microphone.
    pub silent_hint: bool,
    /// A problem the sound system reported.
    pub error: Option<String>,
}

/// Opacity of the error color behind the recording bar.
const BAR_TINT: f32 = 0.12;
const METER_SIZE: egui::Vec2 = vec2(120.0, 6.0);

/// The bar above the page while recording.
pub fn recording_bar(ui: &mut Ui, info: &RecordingInfo) -> Option<RecordingChoice> {
    let red = ui.visuals().error_fg_color;
    egui::Frame::new()
        .fill(red.gamma_multiply(BAR_TINT))
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut choice = None;
            ui.horizontal_wrapped(|ui| {
                let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 5.0, red);
                let title = if info.voice_note { "Recording voice note" } else { "Recording" };
                ui.label(RichText::new(format!("{title} {}", format_elapsed(info.seconds))).strong());
                level_meter(ui, info.level, red);
                if let Some(error) = &info.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                } else if info.silent_hint {
                    let hint = if cfg!(target_os = "macos") {
                        "No sound yet. Check the microphone in System Settings → Privacy & Security → Microphone."
                    } else {
                        "No sound yet. Check that the microphone is on and not muted."
                    };
                    ui.colored_label(ui.visuals().warn_fg_color, hint);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Cancel").on_hover_text("Stop and delete this recording").clicked() {
                        choice = Some(RecordingChoice::Cancel);
                    }
                    let stop_tip = if info.voice_note {
                        "Stop and attach the voice note (Cmd+Shift+R)"
                    } else {
                        "Stop and add the memo here (Cmd+Shift+R)"
                    };
                    if icon_text_button(ui, Icon::Stop, "Stop", stop_tip, false).clicked() {
                        choice = Some(RecordingChoice::Stop);
                    }
                });
            });
            choice
        })
        .inner
}

/// A small bar that fills with the loudness.
fn level_meter(ui: &mut Ui, level: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(METER_SIZE, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
    // Square root: quiet speech still moves the meter.
    let filled = rect.width() * level.clamp(0.0, 1.0).sqrt();
    painter.rect_filled(egui::Rect::from_min_size(rect.min, vec2(filled, rect.height())), 3.0, color);
}
