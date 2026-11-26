//! The short start and stop dictation cues, played natively so they work
//! with no webview open.
//!
//! The tones match the earlier Web Audio version exactly: two sine blips per
//! cue at a peak gain of 0.15, each with a 12 ms exponential attack and an
//! exponential release to silence by the end of the blip.
//!
//! Playback runs on one long-lived thread that sleeps on a channel while
//! idle. For each cue it opens the *current* default output device, plays the
//! cue and closes the device again, so nothing holds an output device (or an
//! audio service process) open between cues, and a changed default device is
//! picked up on the next cue. Callers never wait: [`play`] only queues the
//! cue, so a slow or missing output device can never delay a recording.

use std::{
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, SampleFormat, SizedSample, StreamConfig,
};
use crossbeam_channel::{bounded, unbounded, Sender};
use tauri::{AppHandle, Manager};

use crate::commands::BackendState;

/// Which cue to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// Rising two-tone blip: a recording started.
    Start,
    /// Falling two-tone blip: a recording stopped.
    Stop,
}

/// One sine blip inside a cue.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tone {
    frequency_hz: f32,
    start_s: f32,
    duration_s: f32,
}

const START_TONES: [Tone; 2] = [
    Tone {
        frequency_hz: 660.0,
        start_s: 0.0,
        duration_s: 0.055,
    },
    Tone {
        frequency_hz: 880.0,
        start_s: 0.055,
        duration_s: 0.065,
    },
];

const STOP_TONES: [Tone; 2] = [
    Tone {
        frequency_hz: 880.0,
        start_s: 0.0,
        duration_s: 0.055,
    },
    Tone {
        frequency_hz: 440.0,
        start_s: 0.055,
        duration_s: 0.075,
    },
];

/// Peak gain of each blip (full scale is 1.0).
const PEAK_GAIN: f32 = 0.15;
/// Gain the exponential ramps start from and end at (about -80 dB).
const FLOOR_GAIN: f32 = 0.0001;
/// Rise time from the floor to the peak.
const ATTACK_S: f32 = 0.012;
/// Silence played after the cue before the device is closed, so the last
/// buffer the driver took reaches the speakers instead of being cut off.
const TAIL_SILENCE_S: f32 = 0.2;
/// A cue still waiting this long after it was asked for is dropped: a late
/// blip would be more confusing than none.
const STALE_AFTER: Duration = Duration::from_secs(1);
/// Extra time allowed for a device to take the whole cue before giving up.
const PLAYBACK_TIMEOUT_SLACK: Duration = Duration::from_secs(2);

impl Cue {
    fn tones(self) -> &'static [Tone] {
        match self {
            Cue::Start => &START_TONES,
            Cue::Stop => &STOP_TONES,
        }
    }
}

struct Request {
    app: AppHandle,
    cue: Cue,
    queued_at: Instant,
}

/// Queues `cue` for playback if the Sounds setting is on. Never blocks.
pub fn play(app: &AppHandle, cue: Cue) {
    let Some(sender) = player() else {
        return;
    };
    let _ = sender.send(Request {
        app: app.clone(),
        cue,
        queued_at: Instant::now(),
    });
}

/// The player thread's queue, started on first use. `None` when the thread
/// could not be started; cues are then silently skipped.
fn player() -> Option<&'static Sender<Request>> {
    static PLAYER: OnceLock<Option<Sender<Request>>> = OnceLock::new();
    PLAYER
        .get_or_init(|| {
            let (sender, receiver) = unbounded::<Request>();
            let spawned = thread::Builder::new()
                .name("scribe-sounds".into())
                .spawn(move || {
                    for request in receiver {
                        let enabled = sounds_enabled(&request.app);
                        if should_play(enabled, request.queued_at.elapsed()) {
                            play_cue(request.cue);
                        }
                    }
                });
            match spawned {
                Ok(_) => Some(sender),
                Err(error) => {
                    log::warn!("Could not start the sound cue thread: {}", error);
                    None
                }
            }
        })
        .as_ref()
}

/// Reads the Sounds setting at the moment the cue is about to play, so a
/// change takes effect at once.
fn sounds_enabled(app: &AppHandle) -> Option<bool> {
    let state = app.try_state::<BackendState>()?;
    let db = state.db().ok()?;
    db.get_settings()
        .ok()
        .map(|settings| settings.sounds_enabled)
}

/// Whether a queued cue should play: only when the Sounds setting is known to
/// be on, and only while the cue is still fresh.
fn should_play(sounds_enabled: Option<bool>, waited: Duration) -> bool {
    sounds_enabled == Some(true) && waited <= STALE_AFTER
}

/// Gain envelope of one blip at `t` seconds after it starts: an exponential
/// rise from the floor to the peak over the attack, then an exponential fall
/// back to the floor at the end of the blip, and nothing outside it. This is
/// what the Web Audio `exponentialRampToValueAtTime` pair produced.
fn envelope(t: f32, duration_s: f32) -> f32 {
    if !(0.0..duration_s).contains(&t) {
        return 0.0;
    }
    if t < ATTACK_S {
        FLOOR_GAIN * (PEAK_GAIN / FLOOR_GAIN).powf(t / ATTACK_S)
    } else {
        let progress = (t - ATTACK_S) / (duration_s - ATTACK_S);
        PEAK_GAIN * (FLOOR_GAIN / PEAK_GAIN).powf(progress)
    }
}

/// Length of a cue in seconds.
fn cue_length_s(cue: Cue) -> f32 {
    cue.tones()
        .iter()
        .map(|tone| tone.start_s + tone.duration_s)
        .fold(0.0, f32::max)
}

/// Renders `cue` as mono samples at `sample_rate`.
fn render(cue: Cue, sample_rate: u32) -> Vec<f32> {
    let rate = sample_rate as f32;
    let length = (cue_length_s(cue) * rate).round() as usize;
    (0..length)
        .map(|index| {
            let t = index as f32 / rate;
            let sum: f32 = cue
                .tones()
                .iter()
                .map(|tone| {
                    let local = t - tone.start_s;
                    envelope(local, tone.duration_s)
                        * (std::f32::consts::TAU * tone.frequency_hz * local).sin()
                })
                .sum();
            sum.clamp(-1.0, 1.0)
        })
        .collect()
}

/// Plays `cue` on the current default output device and closes it again.
/// Any failure (no device, unsupported format, a stream error) is logged and
/// skipped: a missing blip must never get in the way of dictation.
fn play_cue(cue: Cue) {
    let Some(device) = cpal::default_host().default_output_device() else {
        log::debug!("No output device; skipping the {:?} sound", cue);
        return;
    };
    match play_cue_on(&device, cue) {
        Ok(true) => {}
        Ok(false) => log::debug!("The output device stalled during the {:?} sound", cue),
        Err(error) => log::debug!("Could not play the {:?} sound: {}", cue, error),
    }
}

/// Plays `cue` on `device` in its default format. `Ok(true)` once the device
/// has taken the whole cue, `Ok(false)` if it stalled and was given up on.
fn play_cue_on(device: &Device, cue: Cue) -> Result<bool, String> {
    let supported = device
        .default_output_config()
        .map_err(|error| format!("no usable output config: {}", error))?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let mut samples = render(cue, config.sample_rate.0);
    let length = samples.len();
    samples.resize(
        length + (TAIL_SILENCE_S * config.sample_rate.0 as f32) as usize,
        0.0,
    );
    let timeout =
        Duration::from_secs_f32(cue_length_s(cue) + TAIL_SILENCE_S) + PLAYBACK_TIMEOUT_SLACK;

    match sample_format {
        SampleFormat::F32 => play_samples::<f32>(device, &config, samples, timeout),
        SampleFormat::F64 => play_samples::<f64>(device, &config, samples, timeout),
        SampleFormat::I16 => play_samples::<i16>(device, &config, samples, timeout),
        SampleFormat::I32 => play_samples::<i32>(device, &config, samples, timeout),
        SampleFormat::U16 => play_samples::<u16>(device, &config, samples, timeout),
        SampleFormat::U8 => play_samples::<u8>(device, &config, samples, timeout),
        SampleFormat::I8 => play_samples::<i8>(device, &config, samples, timeout),
        other => Err(format!("unsupported sample format {:?}", other)),
    }
}

/// Streams mono `samples` to every channel of `device` and returns once the
/// device has taken all of them (`true`) or `timeout` passes (`false`).
/// Dropping the stream on return closes the device.
fn play_samples<T>(
    device: &Device,
    config: &StreamConfig,
    samples: Vec<f32>,
    timeout: Duration,
) -> Result<bool, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let (done_tx, done_rx) = bounded::<()>(1);
    let mut position = 0usize;
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                for frame in data.chunks_mut(channels) {
                    let value = samples.get(position).copied().unwrap_or(0.0);
                    position = position.saturating_add(1);
                    frame.fill(T::from_sample(value));
                }
                if position >= samples.len() {
                    let _ = done_tx.try_send(());
                }
            },
            |error| log::debug!("Sound output stream error: {}", error),
            None,
        )
        .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    let finished = done_rx.recv_timeout(timeout).is_ok();
    drop(stream);
    Ok(finished)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// Sign changes in `samples[from..to]`, ignoring exact zeros.
    fn zero_crossings(samples: &[f32], from: usize, to: usize) -> usize {
        samples[from..to]
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count()
    }

    /// The dominant frequency of a slice estimated from its zero crossings.
    fn estimated_frequency(samples: &[f32], from_s: f32, to_s: f32) -> f32 {
        let from = (from_s * RATE as f32) as usize;
        let to = (to_s * RATE as f32) as usize;
        let crossings = zero_crossings(samples, from, to) as f32;
        crossings / 2.0 / (to_s - from_s)
    }

    #[test]
    fn cues_have_the_web_audio_lengths() {
        assert_eq!(render(Cue::Start, RATE).len(), 5_760, "start is 120 ms");
        assert_eq!(render(Cue::Stop, RATE).len(), 6_240, "stop is 130 ms");
        assert_eq!(render(Cue::Start, 44_100).len(), 5_292);
    }

    #[test]
    fn start_cue_rises_from_660_to_880_hz() {
        let samples = render(Cue::Start, RATE);
        let first = estimated_frequency(&samples, 0.005, 0.050);
        let second = estimated_frequency(&samples, 0.060, 0.115);
        assert!((first - 660.0).abs() < 25.0, "first blip was {first} Hz");
        assert!((second - 880.0).abs() < 25.0, "second blip was {second} Hz");
    }

    #[test]
    fn stop_cue_falls_from_880_to_440_hz() {
        let samples = render(Cue::Stop, RATE);
        let first = estimated_frequency(&samples, 0.005, 0.050);
        let second = estimated_frequency(&samples, 0.060, 0.125);
        assert!((first - 880.0).abs() < 25.0, "first blip was {first} Hz");
        assert!((second - 440.0).abs() < 25.0, "second blip was {second} Hz");
    }

    #[test]
    fn envelope_rises_over_12_ms_and_fades_to_the_floor() {
        assert!((envelope(0.0, 0.055) - FLOOR_GAIN).abs() < 1e-6);
        assert!((envelope(ATTACK_S, 0.055) - PEAK_GAIN).abs() < 1e-4);
        // Monotonic rise during the attack, monotonic fall after it.
        let mut previous = 0.0;
        for step in 0..12 {
            let gain = envelope(step as f32 / 1_000.0, 0.055);
            assert!(gain > previous, "attack must rise at {step} ms");
            previous = gain;
        }
        let mut previous = f32::MAX;
        for step in 12..55 {
            let gain = envelope(step as f32 / 1_000.0, 0.055);
            assert!(gain < previous, "release must fall at {step} ms");
            previous = gain;
        }
        assert!(envelope(0.054_99, 0.055) < 0.000_11);
        assert_eq!(envelope(0.055, 0.055), 0.0);
        assert_eq!(envelope(-0.001, 0.055), 0.0);
    }

    #[test]
    fn cues_never_clip_and_start_and_end_quietly() {
        for cue in [Cue::Start, Cue::Stop] {
            for rate in [22_050, 44_100, 48_000, 96_000] {
                let samples = render(cue, rate);
                let peak = samples.iter().fold(0.0f32, |max, s| max.max(s.abs()));
                assert!(
                    peak <= PEAK_GAIN + 1e-4,
                    "{cue:?} at {rate} peaked at {peak}"
                );
                assert!(peak > PEAK_GAIN * 0.9, "{cue:?} at {rate} is too quiet");
                assert!(samples.iter().all(|s| s.is_finite()));
                // No click at either edge.
                assert!(samples[0].abs() <= FLOOR_GAIN);
                assert!(samples.last().unwrap().abs() <= 0.001);
            }
        }
    }

    #[test]
    fn blips_meet_without_a_click() {
        // At the hand-over (55 ms) both blips are near the floor.
        for cue in [Cue::Start, Cue::Stop] {
            let samples = render(cue, RATE);
            let handover = (0.055 * RATE as f32) as usize;
            for sample in &samples[handover - 2..handover + 2] {
                assert!(sample.abs() < 0.001, "{cue:?} clicks at the hand-over");
            }
        }
    }

    /// Drives both cues through a real output stream and checks the device
    /// took every sample and was closed again. Needs an output device, and on
    /// the default device it is audible, so run it by hand:
    /// `cargo test --lib sounds -- --ignored`. On Linux it uses ALSA's silent
    /// "null" device when there is one.
    #[test]
    #[ignore = "needs an output device"]
    fn cues_play_through_a_real_output_stream() {
        let host = cpal::default_host();
        let null_device = cfg!(target_os = "linux")
            .then(|| host.output_devices().ok())
            .flatten()
            .and_then(|mut devices| {
                devices.find(|device| device.name().is_ok_and(|name| name == "null"))
            });
        let device = null_device
            .or_else(|| host.default_output_device())
            .expect("an output device");
        for cue in [Cue::Start, Cue::Stop, Cue::Start] {
            let started = Instant::now();
            let finished = play_cue_on(&device, cue).expect("the cue plays");
            assert!(finished, "{cue:?}: the device took every sample");
            let took = started.elapsed();
            assert!(
                took < Duration::from_secs(2),
                "{cue:?} held the device for {took:?}"
            );
        }
    }

    #[test]
    fn plays_only_when_sounds_are_on_and_the_cue_is_fresh() {
        let fresh = Duration::from_millis(5);
        assert!(should_play(Some(true), fresh));
        assert!(!should_play(Some(false), fresh), "Sounds setting off");
        assert!(!should_play(None, fresh), "settings unreadable");
        assert!(should_play(Some(true), STALE_AFTER));
        assert!(
            !should_play(Some(true), STALE_AFTER + Duration::from_millis(1)),
            "a cue that waited too long is dropped"
        );
    }
}
