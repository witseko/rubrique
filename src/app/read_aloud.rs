//! Download the Phonon checkpoint on request and read selected text offline.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use eframe::egui::{Context, Ui};
use ptts::checkpoint::{Checkpoint, ResolveOptions};
use ptts::preprocess::Lang;
use ptts::synth::{DeviceKind, Quant, Synth};

use super::{OpenFile, RubriqueApp, char_to_byte};
use crate::document::{preview_selection, view::editor_selection};
use crate::phonon;
use crate::settings::phonon_model_folder;
use crate::settings::view::{ReadAloudInfo, SettingsGroup};
use crate::voice::{audio::Player, wav::Audio};

const MAX_SELECTION_CHARS: usize = 400;
const MAX_AUDIO_SECONDS: usize = 120;
const REPAINT_INTERVAL: Duration = Duration::from_millis(50);

/// Model cache and current speech in this window.
#[derive(Default)]
pub(super) struct ReadAloud {
    model: Option<Synth>,
    state: Speech,
    download: Option<Receiver<Result<(), String>>>,
    download_error: Option<String>,
}

impl std::fmt::Debug for ReadAloud {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ReadAloud").finish_non_exhaustive()
    }
}

/// Synthesis owns the model until the worker returns it.
#[derive(Default)]
enum Speech {
    #[default]
    Idle,
    Generating(Generation),
    Playing(Player),
}

/// A stopped job keeps its place until the worker has released its stream.
struct Generation {
    cancelled: Arc<AtomicBool>,
    result: Receiver<(Option<Synth>, Result<Audio, String>)>,
}

impl ReadAloud {
    /// Load the model if needed and synthesize off the UI thread.
    fn start(&mut self, ctx: &Context, text: String, model_dir: PathBuf) -> Result<(), String> {
        let model = self.model.take();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let (sender, result) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("read-aloud".into())
            .spawn(move || {
                let completed = match model.map_or_else(|| load_model(&model_dir), Ok) {
                    Ok(model) => {
                        let audio = generate(&model, &text, &worker_cancelled);
                        (Some(model), audio)
                    }
                    Err(error) => (None, Err(error)),
                };
                let _ = sender.send(completed);
                ctx.request_repaint();
            })
            .map_err(|error| format!("Could not start Read aloud: {error}"))?;
        self.state = Speech::Generating(Generation { cancelled, result });
        Ok(())
    }

    /// Stop audio immediately and ask the worker to stop generating.
    pub(super) fn stop(&mut self) {
        match &self.state {
            Speech::Idle => {}
            Speech::Generating(job) => job.cancelled.store(true, Ordering::Relaxed),
            Speech::Playing(_) => self.state = Speech::Idle,
        }
    }

    /// Play finished audio only if the job is still active.
    fn poll(&mut self, ctx: &Context) -> Result<(), String> {
        match &self.state {
            Speech::Idle => {}
            Speech::Generating(job) => match job.result.try_recv() {
                Ok((model, audio)) => {
                    let cancelled = job.cancelled.load(Ordering::Relaxed);
                    self.model = model;
                    self.state = Speech::Idle;
                    if !cancelled {
                        self.state = Speech::Playing(play(audio?)?);
                        ctx.request_repaint_after(REPAINT_INTERVAL);
                    }
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(REPAINT_INTERVAL),
                Err(TryRecvError::Disconnected) => {
                    self.state = Speech::Idle;
                    return Err("Read aloud stopped before it could generate speech".into());
                }
            },
            Speech::Playing(player) if player.is_playing() => ctx.request_repaint_after(REPAINT_INTERVAL),
            Speech::Playing(_) => self.state = Speech::Idle,
        }
        Ok(())
    }
}

impl Drop for ReadAloud {
    fn drop(&mut self) {
        self.stop();
    }
}

impl ReadAloud {
    /// Report model availability and the progress of the Settings download.
    pub(super) fn info(&self) -> ReadAloudInfo<'_> {
        if self.download.is_some() {
            ReadAloudInfo::Downloading
        } else if phonon_model_folder().is_some_and(|folder| phonon::is_installed(&folder)) {
            ReadAloudInfo::Installed
        } else if let Some(error) = self.download_error.as_deref() {
            ReadAloudInfo::Failed(error)
        } else {
            ReadAloudInfo::Missing
        }
    }
}

impl RubriqueApp {
    /// Start a download only from the explicit Settings action.
    pub(super) fn download_phonon(&mut self, ctx: &Context) {
        if self.read_aloud.download.is_some() {
            return;
        }
        match start_download(ctx) {
            Ok(download) => {
                self.read_aloud.download = Some(download);
                self.read_aloud.download_error = None;
            }
            Err(error) => self.read_aloud.download_error = Some(error),
        }
    }

    /// Report completion when the download worker releases its result.
    fn poll_phonon_download(&mut self) {
        let Some(download) = &self.read_aloud.download else {
            return;
        };
        let result = match download.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("The model download stopped. Try again.".into()),
        };
        self.read_aloud.download = None;
        match result {
            Ok(()) => self.show_status("Phonon is ready. Select text to read aloud.".into()),
            Err(error) => self.read_aloud.download_error = Some(error),
        }
    }

    /// Keep Stop available even while the status bar shows a message.
    pub(super) fn read_aloud_button(&mut self, ui: &mut Ui) {
        match &self.read_aloud.state {
            Speech::Generating(job) if job.cancelled.load(Ordering::Relaxed) => {
                ui.label("Stopping speech");
            }
            Speech::Generating(_) | Speech::Playing(_) => {
                if ui.button("Stop reading").clicked() {
                    self.read_aloud.stop();
                }
            }
            Speech::Idle => {
                let Some(text) = self.read_aloud_selection(ui.ctx()) else {
                    return;
                };
                let response = ui.button("Read aloud").on_hover_text("Speak the selection with local Phonon");
                if self.showing_any_preview() {
                    preview_selection::set_read_aloud_button_rect(ui.ctx(), response.rect);
                }
                if response.clicked() {
                    let selection = validate_selection(&text).map(str::to_owned);
                    if let Err(error) = selection.and_then(|text| self.start_read_aloud(ui.ctx(), text)) {
                        self.show_status(error);
                    }
                }
            }
        }
    }

    /// Get only the explicit selection in the current document.
    fn read_aloud_selection(&self, ctx: &Context) -> Option<Cow<'_, str>> {
        let OpenFile::Text { document, .. } = &self.open_file else {
            return None;
        };
        if document.is_encrypted() || self.merging.is_some() || self.block_edit.is_some() {
            return None;
        }
        let text = if self.showing_any_preview() {
            let selection = preview_selection::current(ctx)?;
            self.ask.page_rect.filter(|page| page.contains(selection.pointer))?;
            Cow::Owned(selection.text)
        } else {
            let (first, last) = editor_selection(ctx)?;
            let range = char_to_byte(&document.text, first)..char_to_byte(&document.text, last);
            Cow::Borrowed(document.text.get(range)?)
        };
        (!text.trim().is_empty()).then_some(text)
    }

    /// Open model setup if needed, or play the selection from the local model.
    fn start_read_aloud(&mut self, ctx: &Context, text: String) -> Result<(), String> {
        let model_dir = phonon_model_folder().ok_or("Could not find the settings folder for Phonon")?;
        if self.read_aloud.model.is_none() && !phonon::is_installed(&model_dir) {
            self.settings_group = SettingsGroup::ReadAloud;
            self.settings_open = true;
            return Ok(());
        }
        if self.voice.is_recording() {
            return Err("Stop recording before using Read aloud".into());
        }
        self.voice.stop_playback();
        self.read_aloud.start(ctx, text, model_dir)
    }

    /// Cancel speech on lock, or when the current document becomes encrypted.
    pub(super) fn poll_read_aloud(&mut self, ctx: &Context) {
        self.poll_phonon_download();
        if self.app_locked() || self.open_file.is_encrypted() {
            self.read_aloud.stop();
        }
        if let Err(error) = self.read_aloud.poll(ctx) {
            self.show_status(error);
        }
    }
}

/// Run the download off the UI thread and wake the window when it finishes.
fn start_download(ctx: &Context) -> Result<Receiver<Result<(), String>>, String> {
    let folder = phonon_model_folder().ok_or("Could not find the settings folder for Phonon")?;
    let curl = crate::install::find_installer("curl").ok_or("Install curl to download Phonon, then try again.")?;
    let (sender, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("phonon-download".into())
        .spawn(move || {
            let result = phonon::download(&curl, &folder).map_err(|error| format!("Phonon download failed: {error}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        })
        .map_err(|error| format!("Could not start the Phonon download: {error}"))?;
    Ok(receiver)
}

/// Bound CPU work to a short selection and measure length in characters.
fn validate_selection(text: &str) -> Result<&str, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Select text to read aloud".into());
    }
    if text.chars().nth(MAX_SELECTION_CHARS).is_some() {
        return Err(format!("Select {MAX_SELECTION_CHARS} characters or fewer to read aloud"));
    }
    Ok(text)
}

/// Read the English q8 checkpoint and its default preset voice from disk.
fn load_model(path: &Path) -> Result<Synth, String> {
    phonon::verify(path).map_err(|error| {
        format!("Could not verify Phonon: {error}. Move {} and download the model again.", path.display())
    })?;
    let options = ResolveOptions { quant: Quant::Q80, weights: Some("model.q8.gguf") };
    let checkpoint = Checkpoint::resolve(path, options).map_err(|_| "Could not read the local Phonon checkpoint")?;
    let mut model = checkpoint
        .builder(Lang::En)
        .device(DeviceKind::Cpu)
        .build()
        .map_err(|_| "Could not load the local Phonon model")?;
    model
        .add_voice_file("default", &path.join("default-voice.safetensors"))
        .map_err(|_| "Could not load the local Phonon voice")?;
    Ok(model)
}

/// Buffer one short selection. Dropping the stream cancels its worker threads.
fn generate(model: &Synth, text: &str, cancelled: &AtomicBool) -> Result<Audio, String> {
    let sample_rate = model.sample_rate();
    let sample_limit = sample_rate as usize * MAX_AUDIO_SECONDS;
    let mut samples = Vec::new();
    if !cancelled.load(Ordering::Relaxed) {
        let stream = model.stream(text).map_err(|_| "Could not start Phonon speech synthesis")?;
        for chunk in stream {
            if cancelled.load(Ordering::Relaxed) {
                break;
            }
            let chunk = chunk.map_err(|_| "Phonon could not finish generating speech")?;
            if chunk.len() > sample_limit.saturating_sub(samples.len()) {
                return Err("Phonon speech exceeded the playback limit".into());
            }
            samples.extend(chunk);
        }
    }
    if samples.is_empty() {
        return Err("Phonon returned no speech".into());
    }
    Ok(Audio { sample_rate, samples })
}

/// Send the generated samples to the same output player as voice memos.
fn play(audio: Audio) -> Result<Player, String> {
    let player = Player::new(audio)?;
    player.try_play()?;
    Ok(player)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn validates_selected_characters() {
        assert_eq!(validate_selection("  hello  "), Ok("hello"));
        assert!(validate_selection("  ").is_err());
        assert!(validate_selection(&"é".repeat(MAX_SELECTION_CHARS)).is_ok());
        assert!(validate_selection(&"é".repeat(MAX_SELECTION_CHARS + 1)).is_err());
    }

    #[test]
    fn reports_a_missing_checkpoint() {
        assert!(load_model(Path::new("/dev/null")).is_err());
    }

    #[test]
    fn stop_ignores_a_late_worker_result() -> Result<(), String> {
        let (sender, result) = mpsc::channel();
        let mut speech = ReadAloud {
            state: Speech::Generating(Generation { cancelled: Arc::new(AtomicBool::new(false)), result }),
            model: None,
            download: None,
            download_error: None,
        };
        speech.stop();
        sender.send((None, Err("discard this result".into()))).map_err(|error| error.to_string())?;
        speech.poll(&Context::default())?;
        assert!(matches!(speech.state, Speech::Idle));
        Ok(())
    }

    #[test]
    #[ignore = "requires a local Phonon checkpoint and a working playback device"]
    fn offline_phonon_playback() -> Result<(), String> {
        let model_dir =
            PathBuf::from(std::env::var_os("RUBRIQUE_PHONON_MODEL_DIR").ok_or("Set the test model directory")?);
        let text = "This is Rubrique reading the selected text with a local model.";
        let cold_started = Instant::now();
        let model = load_model(&model_dir)?;
        let loaded = cold_started.elapsed();
        eprintln!("model load: {} ms", loaded.as_millis());
        for label in ["cold", "warm", "warm", "warm"] {
            let started = Instant::now();
            let audio = generate(&model, text, &AtomicBool::new(false))?;
            assert!(audio.samples.iter().all(|sample| sample.is_finite()));
            assert!(audio.samples.iter().any(|sample| sample.abs() > 0.01));
            let duration = crate::voice::wav::count(audio.samples.len()) / f64::from(audio.sample_rate);
            let synthesized = started.elapsed();
            let player = play(audio)?;
            while player.position_seconds() == 0.0 && started.elapsed() < synthesized + Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(player.position_seconds() > 0.0, "audio callback did not consume samples");
            let startup = started.elapsed() + if label == "cold" { loaded } else { Duration::ZERO };
            eprintln!(
                "{label}: synthesis {} ms, output started {} ms, {:.2} s audio",
                synthesized.as_millis(),
                startup.as_millis(),
                duration
            );
            let deadline = Instant::now() + Duration::from_secs_f64(duration + 2.0);
            while player.is_playing() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
            }
            assert!(!player.is_playing(), "audio did not reach the end");
        }
        Ok(())
    }
}
