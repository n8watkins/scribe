//! Test-only audio input for the Windows end-to-end job
//! (`.github/workflows/e2e-windows.yml`): hosted runners have no microphone, so
//! dictation records a WAV file instead, and everything after capture
//! (hotkeys, the recording pipeline, Whisper, output) runs for real.
//!
//! Three gates keep shipped builds out of reach:
//!   - This module only exists with the `e2e-audio` Cargo feature, which only
//!     the end-to-end job turns on. Release builds don't contain it at all
//!     (the release workflow checks the binary for [`ENV_VAR`]).
//!   - It only switches on in the Scribe Dev flavor.
//!   - It reads [`ENV_VAR`] from its own process environment once, at
//!     startup. Nothing another process sends can turn it on: a second launch
//!     hands over to the running copy through the single-instance plugin,
//!     which passes on neither its arguments nor its environment.
//!
//! When it is on, every recording reads the WAV at that path afresh (so a
//! test can swap the clip between dictations) and plays it in real time like
//! a microphone would, then keeps sending silence until the recording stops.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, RecvTimeoutError};
use tauri::AppHandle;

use crate::{audio::MicrophoneInfo, error::CommandError};

/// The environment variable that names the WAV file to record from.
pub const ENV_VAR: &str = "SCRIBE_E2E_AUDIO_WAV";

/// Logged once per recording when the whole clip has been sent, so a test
/// knows it can stop the dictation without losing the end of the clip.
pub const CLIP_FINISHED_LOG: &str = "E2E test audio: clip finished";

/// How much audio each chunk carries, about what WASAPI delivers per callback.
const CHUNK: Duration = Duration::from_millis(10);

static CLIP_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Switches the test input on when the process was started with [`ENV_VAR`]
/// and this is the Dev flavor. Called once during setup.
pub fn activate(app: &AppHandle) {
    let requested = std::env::var_os(ENV_VAR);
    let dev_flavor = crate::is_dev_flavor(app);
    if requested.is_some() && !dev_flavor {
        log::warn!("{} is ignored outside the Scribe Dev flavor", ENV_VAR);
    }
    if let Some(path) = configured_path(requested, dev_flavor) {
        log::warn!(
            "E2E test audio is on: recordings read {} instead of a microphone",
            path.display()
        );
        let _ = CLIP_PATH.set(path);
    }
}

/// The clip path to record from, given the variable's value and the flavor.
fn configured_path(requested: Option<OsString>, dev_flavor: bool) -> Option<PathBuf> {
    requested
        .filter(|value| dev_flavor && !value.is_empty())
        .map(PathBuf::from)
}

/// The clip for a recording that is starting, or `None` when the test input is
/// off and the microphone should be used.
pub fn clip_for_recording() -> Result<Option<Clip>, CommandError> {
    CLIP_PATH.get().map(|path| Clip::load(path)).transpose()
}

/// A mono clip at its own sample rate, as the recording worker expects it.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    name: String,
}

impl Clip {
    pub fn load(path: &Path) -> Result<Self, CommandError> {
        let failed = |error: hound::Error| {
            CommandError::new(
                "recording_failed",
                format!("Could not read test audio {}. {}", path.display(), error),
            )
        };
        let mut reader = hound::WavReader::open(path).map_err(failed)?;
        let spec = reader.spec();
        let interleaved: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>(),
            hound::SampleFormat::Int => {
                let scale = (1_i64 << (spec.bits_per_sample.max(1) - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|value| value as f32 / scale))
                    .collect::<Result<_, _>>()
            }
        }
        .map_err(failed)?;
        let channels = usize::from(spec.channels.max(1));
        let samples = interleaved
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect();
        Ok(Self {
            samples,
            sample_rate: spec.sample_rate,
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        })
    }

    /// How the clip shows up where a microphone would.
    pub fn microphone_info(&self) -> MicrophoneInfo {
        MicrophoneInfo {
            id: "e2e-test-audio".to_string(),
            name: format!("E2E test audio ({})", self.name),
            endpoint_id: None,
            is_default: true,
            is_selected: true,
            is_available: true,
        }
    }

    fn chunk_len(&self) -> usize {
        ((self.sample_rate as u128 * CHUNK.as_millis()) / 1000).max(1) as usize
    }
}

/// Plays `clip` into `send` in real time, then silence, until `stop` fires or
/// closes, or `send` reports the receiver is gone. Runs on the capture thread.
pub fn play(clip: &Clip, stop: &Receiver<()>, mut send: impl FnMut(Vec<f32>) -> bool) {
    let chunk_len = clip.chunk_len();
    let mut chunks = clip.samples.chunks(chunk_len);
    let mut finished = false;
    let started = Instant::now();
    for index in 1_u32.. {
        // Each chunk goes out once the audio it holds would have been heard,
        // paced from the start so the clip never drifts.
        let due = started + CHUNK * index;
        match stop.recv_timeout(due.saturating_duration_since(Instant::now())) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
        let samples = match chunks.next() {
            Some(chunk) => chunk.to_vec(),
            None => {
                if !finished {
                    finished = true;
                    log::info!("{} ({})", CLIP_FINISHED_LOG, clip.name);
                }
                vec![0.0; chunk_len]
            }
        };
        if !send(samples) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;

    fn write_wav(path: &Path, spec: hound::WavSpec, samples: &[i16]) {
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn spec(channels: u16, sample_rate: u32) -> hound::WavSpec {
        hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        }
    }

    #[test]
    fn only_the_dev_flavor_with_a_path_turns_it_on() {
        let path = || Some(OsString::from("C:\\clips\\hello.wav"));
        assert_eq!(
            configured_path(path(), true),
            Some(PathBuf::from("C:\\clips\\hello.wav"))
        );
        assert_eq!(configured_path(path(), false), None);
        assert_eq!(configured_path(None, true), None);
        assert_eq!(configured_path(Some(OsString::new()), true), None);
    }

    #[test]
    fn loads_mono_samples_at_the_file_rate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mono.wav");
        write_wav(&path, spec(1, 16_000), &[0, i16::MAX, i16::MIN]);

        let clip = Clip::load(&path).unwrap();

        assert_eq!(clip.sample_rate, 16_000);
        assert_eq!(clip.samples.len(), 3);
        assert_eq!(clip.samples[0], 0.0);
        assert!((clip.samples[1] - 1.0).abs() < 0.001);
        assert_eq!(clip.samples[2], -1.0);
        assert_eq!(clip.microphone_info().name, "E2E test audio (mono.wav)");
    }

    #[test]
    fn mixes_stereo_down_to_mono() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stereo.wav");
        write_wav(&path, spec(2, 48_000), &[16_384, 0, -16_384, -16_384]);

        let clip = Clip::load(&path).unwrap();

        assert_eq!(clip.sample_rate, 48_000);
        assert_eq!(clip.samples, vec![0.25, -0.5]);
    }

    #[test]
    fn a_missing_file_is_a_recording_error() {
        let error = Clip::load(Path::new("does-not-exist.wav")).unwrap_err();
        assert_eq!(error.code, "recording_failed");
        assert!(error.message.contains("does-not-exist.wav"));
    }

    #[test]
    fn plays_the_clip_then_silence_until_stopped() {
        // 25 ms at 1 kHz: chunks of 10, 10 and 5 samples, then silence.
        let clip = Clip {
            samples: (1..=25).map(|value| value as f32).collect(),
            sample_rate: 1_000,
            name: "count.wav".to_string(),
        };
        let (stop_tx, stop_rx) = bounded::<()>(1);
        let mut received = Vec::new();
        play(&clip, &stop_rx, |samples| {
            received.push(samples);
            if received.len() == 5 {
                let _ = stop_tx.try_send(());
            }
            true
        });

        assert_eq!(received.len(), 5);
        let played: Vec<f32> = received[..3].concat();
        assert_eq!(played, clip.samples);
        assert_eq!(received[3], vec![0.0; 10]);
        assert_eq!(received[4], vec![0.0; 10]);
    }

    #[test]
    fn stops_when_the_receiver_is_gone() {
        let clip = Clip {
            samples: vec![0.5; 100],
            sample_rate: 1_000,
            name: "gone.wav".to_string(),
        };
        let (_stop_tx, stop_rx) = bounded::<()>(1);
        let mut sends = 0;
        play(&clip, &stop_rx, |_| {
            sends += 1;
            false
        });
        assert_eq!(sends, 1);
    }
}
