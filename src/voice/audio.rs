//! Record from the microphone into a WAV file, and play WAV files, with
//! `cpal` (`CoreAudio` on macOS, ALSA on Linux).
//!
//! The audio callbacks run on the sound system's thread. They only copy
//! samples into shared buffers; the window writes the file and reads the
//! level each frame.

use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use super::wav::{self, Downsampler, count};

/// What the input callback hands to the window.
#[derive(Debug, Default)]
struct Captured {
    /// 16-bit samples at the memo rate, not yet written to the file.
    samples: Mutex<Vec<i16>>,
    /// Loudest sample since the window last looked, as `f32` bits.
    peak: AtomicU32,
    /// The last stream error, if any.
    error: Mutex<Option<String>>,
}

/// A recording in progress. Dropping it finishes the file, so quitting
/// keeps what was recorded.
pub struct Recorder {
    stream: cpal::Stream,
    captured: Arc<Captured>,
    file: Option<File>,
    path: PathBuf,
    sample_rate: u32,
    data_bytes: u32,
    started: Instant,
    /// Some sound above the noise floor has been heard.
    heard: bool,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Recorder").field("path", &self.path).finish_non_exhaustive()
    }
}

/// Peaks below this count as silence (about -50 dB).
const SILENCE: f32 = 0.003;

impl Recorder {
    /// Start recording from the default microphone into a new WAV file at `path`.
    ///
    /// # Errors
    ///
    /// Fails when there is no microphone, it cannot be opened, or the file
    /// cannot be created. The message is ready to show.
    pub fn start(path: &Path) -> Result<Recorder, String> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or("No microphone found")?;
        let config = device.default_input_config().map_err(|error| format!("Cannot open the microphone: {error}"))?;
        let format = config.sample_format();
        let stream_config = config.config();
        let downsampler = Downsampler::new(stream_config.sample_rate);
        let sample_rate = downsampler.out_rate();
        let captured = Arc::new(Captured::default());
        let stream = match format {
            SampleFormat::F32 => input_stream::<f32>(&device, &stream_config, downsampler, &captured),
            SampleFormat::I16 => input_stream::<i16>(&device, &stream_config, downsampler, &captured),
            SampleFormat::I32 => input_stream::<i32>(&device, &stream_config, downsampler, &captured),
            SampleFormat::U16 => input_stream::<u16>(&device, &stream_config, downsampler, &captured),
            SampleFormat::U8 => input_stream::<u8>(&device, &stream_config, downsampler, &captured),
            SampleFormat::I8 => input_stream::<i8>(&device, &stream_config, downsampler, &captured),
            other => return Err(format!("The microphone uses an unsupported sample format ({other})")),
        }
        .map_err(|error| format!("Cannot open the microphone: {error}"))?;
        stream.play().map_err(|error| format!("Cannot start recording: {error}"))?;
        let file = new_recording_file(path, sample_rate)?;
        Ok(Recorder {
            stream,
            captured,
            file: Some(file),
            path: path.to_path_buf(),
            sample_rate,
            data_bytes: 0,
            started: Instant::now(),
            heard: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn elapsed_seconds(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// The loudest sample since the last call, 0.0 to 1.0.
    pub fn take_level(&mut self) -> f32 {
        let level = f32::from_bits(self.captured.peak.swap(0, Ordering::AcqRel));
        self.heard |= level > SILENCE;
        level
    }

    /// Any sound was heard since the start (else the microphone may be off or blocked).
    pub fn heard_sound(&self) -> bool {
        self.heard
    }

    /// A problem the sound system reported while recording.
    pub fn error(&self) -> Option<String> {
        self.captured.error.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Write the samples captured so far to the file.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be written.
    pub fn write_pending(&mut self) -> io::Result<()> {
        let samples = std::mem::take(&mut *self.captured.samples.lock().unwrap_or_else(PoisonError::into_inner));
        let Some(file) = &mut self.file else {
            return Ok(());
        };
        if samples.is_empty() {
            return Ok(());
        }
        let bytes: Vec<u8> = samples.iter().flat_map(|sample| sample.to_le_bytes()).collect();
        file.write_all(&bytes)?;
        self.data_bytes = self.data_bytes.saturating_add(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
        Ok(())
    }

    /// Stop, write the rest and the final header. Returns the length in seconds.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be written.
    pub fn finish(mut self) -> io::Result<f64> {
        self.close()?;
        Ok(f64::from(self.data_bytes / 2) / f64::from(self.sample_rate))
    }

    fn close(&mut self) -> io::Result<()> {
        let _ = self.stream.pause();
        self.write_pending()?;
        let Some(mut file) = self.file.take() else {
            return Ok(());
        };
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&wav::header(self.sample_rate, 1, self.data_bytes))?;
        file.flush()
    }

    /// Stop and delete the file.
    pub fn cancel(mut self) {
        let _ = self.stream.pause();
        self.file = None;
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Create a private WAV file without overwriting another recording.
fn new_recording_file(path: &Path, sample_rate: u32) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| format!("Cannot create {}: {error}", path.display()))?;
    if let Err(error) = file.write_all(&wav::header(sample_rate, 1, 0)) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(format!("Cannot write the memo: {error}"));
    }
    Ok(file)
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.file.is_some() {
            let _ = self.close();
        }
    }
}

/// An input stream for samples of type `T` that mixes channels to mono,
/// brings them to the memo rate and hands them to `captured`.
fn input_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut downsampler: Downsampler,
    captured: &Arc<Captured>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let data_target = Arc::clone(captured);
    let error_target = Arc::clone(captured);
    let mut out = Vec::new();
    device.build_input_stream::<T, _, _>(
        *config,
        move |data: &[T], _| {
            let mut peak = 0.0_f32;
            out.clear();
            for frame in data.chunks(channels) {
                #[expect(clippy::cast_precision_loss, reason = "a frame has a handful of channels")]
                let mono = frame.iter().map(|sample| sample.to_sample::<f32>()).sum::<f32>() / frame.len() as f32;
                peak = peak.max(mono.abs());
                if let Some(sample) = downsampler.push(mono) {
                    out.push(wav::to_i16(sample));
                }
            }
            data_target.samples.lock().unwrap_or_else(PoisonError::into_inner).extend_from_slice(&out);
            // The peak is never negative, and the bits of positive floats
            // sort like the floats, so the largest bits are the loudest.
            data_target.peak.fetch_max(peak.to_bits(), Ordering::AcqRel);
        },
        move |error| {
            *error_target.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(error.to_string());
        },
        None,
    )
}

/// What the output callback plays.
#[derive(Debug)]
struct Playback {
    samples: Vec<f32>,
    /// Position in source samples (fractional, for resampling).
    position: f64,
    /// Source samples per output sample.
    step: f64,
    playing: bool,
}

/// A WAV file ready to play, with pause and seek.
pub struct Player {
    stream: cpal::Stream,
    state: Arc<Mutex<Playback>>,
    sample_rate: u32,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Player").field("sample_rate", &self.sample_rate).finish_non_exhaustive()
    }
}

impl Player {
    /// Get the speakers ready for `audio` (paused at the start).
    ///
    /// # Errors
    ///
    /// Fails when there is no output device or it cannot be opened.
    pub fn new(audio: wav::Audio) -> Result<Player, String> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("No speakers or headphones found")?;
        let config = device.default_output_config().map_err(|error| format!("Cannot open the speakers: {error}"))?;
        let format = config.sample_format();
        let stream_config = config.config();
        let sample_rate = audio.sample_rate;
        let step = f64::from(audio.sample_rate) / f64::from(stream_config.sample_rate.max(1));
        let state = Arc::new(Mutex::new(Playback { samples: audio.samples, position: 0.0, step, playing: false }));
        let stream = match format {
            SampleFormat::F32 => output_stream::<f32>(&device, &stream_config, &state),
            SampleFormat::I16 => output_stream::<i16>(&device, &stream_config, &state),
            SampleFormat::I32 => output_stream::<i32>(&device, &stream_config, &state),
            SampleFormat::U16 => output_stream::<u16>(&device, &stream_config, &state),
            SampleFormat::U8 => output_stream::<u8>(&device, &stream_config, &state),
            other => return Err(format!("The speakers use an unsupported sample format ({other})")),
        }
        .map_err(|error| format!("Cannot open the speakers: {error}"))?;
        Ok(Player { stream, state, sample_rate })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, Playback> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Play from the current position (from the start when at the end).
    pub fn play(&self) {
        {
            let mut state = self.state();
            if state.position >= count(state.samples.len()) {
                state.position = 0.0;
            }
            state.playing = true;
        }
        let _ = self.stream.play();
    }

    pub fn pause(&self) {
        self.state().playing = false;
        let _ = self.stream.pause();
    }

    pub fn is_playing(&self) -> bool {
        self.state().playing
    }

    /// Go to `seconds` from the start.
    pub fn seek(&self, seconds: f64) {
        let mut state = self.state();
        let end = count(state.samples.len());
        state.position = (seconds * f64::from(self.sample_rate)).clamp(0.0, end);
    }

    pub fn position_seconds(&self) -> f64 {
        self.state().position / f64::from(self.sample_rate)
    }
}

/// An output stream that plays `state` on every channel, resampling by
/// straight-line interpolation.
fn output_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    state: &Arc<Mutex<Playback>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels.max(1));
    let state = Arc::clone(state);
    device.build_output_stream::<T, _, _>(
        *config,
        move |data: &mut [T], _| {
            // Never wait on the window: a busy lock means one buffer of silence.
            let Ok(mut playing) = state.try_lock() else {
                return;
            };
            let length = playing.samples.len();
            for frame in data.chunks_mut(channels) {
                if !playing.playing {
                    break;
                }
                #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "position is in 0..length")]
                let index = playing.position as usize;
                if index + 1 >= length {
                    playing.playing = false;
                    playing.position = count(length);
                    break;
                }
                #[expect(clippy::cast_possible_truncation, reason = "the fraction is between 0 and 1")]
                let fraction = (playing.position - count(index)) as f32;
                let sample = playing.samples[index] + (playing.samples[index + 1] - playing.samples[index]) * fraction;
                frame.fill(T::from_sample(sample));
                playing.position += playing.step;
            }
        },
        |error| eprintln!("rubrique: audio output: {error}"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    #[test]
    fn opening_a_recording_keeps_an_existing_file() {
        let folder = TempFolder::new();
        let path = folder.write("memo.wav", "an earlier recording");
        assert!(new_recording_file(&path, 16_000).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"an earlier recording");
    }
}
