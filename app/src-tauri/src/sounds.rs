//! The short start and stop dictation cues, played natively so they work
//! with no webview open.
//!
//! The tones match the earlier Web Audio version exactly: two sine blips per
//! cue at a peak gain of 0.15, each with a 12 ms exponential attack and an
//! exponential release to silence by the end of the blip.
//!
//! How playback works:
//!
//! - [`play`] never waits. It hands the cue to a player thread through a small
//!   bounded queue; when the queue is full the cue is dropped.
//! - The player keeps one output stream open while cues are playing and
//!   mixes each new cue into it the moment it arrives, so cues overlap the
//!   way the Web Audio ones did instead of waiting for each other.
//! - The stream is closed once it has been silent for [`IDLE_CLOSE_AFTER`],
//!   so no output device (or audio service process) stays open while Scribe
//!   is idle. The next cue opens the *current* default device, which is how a
//!   changed default device is picked up. A stream that reports an error is
//!   closed and reopened on the next cue.
//! - Device calls (open and close) can hang or panic inside a driver. Panics
//!   are caught. A player stuck in a device call for [`STUCK_AFTER`] is
//!   abandoned and a fresh one started, at most [`MAX_PLAYERS`] per session,
//!   so a broken driver costs a few parked threads at worst, never a growing
//!   queue.

use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, SampleFormat, SizedSample, StreamConfig,
};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender, TrySendError};
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
/// How long the output stream stays open after the last cue finished. Long
/// enough that a stop followed by a quick start reuses the open stream, short
/// enough that nothing stays open while Scribe is idle.
const IDLE_CLOSE_AFTER: Duration = Duration::from_secs(3);
/// How often the player checks whether an open stream has gone idle.
const IDLE_CHECK_EVERY: Duration = Duration::from_millis(250);
/// A cue still waiting this long after it was asked for is dropped: a late
/// blip would be more confusing than none.
const STALE_AFTER: Duration = Duration::from_secs(1);
/// Cues waiting for the player. Recording start and stop come one at a time,
/// so a full queue only happens when the player is stuck.
const QUEUE_CAPACITY: usize = 4;
/// A player inside one device call this long is treated as stuck.
const STUCK_AFTER: Duration = Duration::from_secs(5);
/// Player threads started per session, including replacements for stuck or
/// dead ones. Bounds what a broken driver can cost.
const MAX_PLAYERS: usize = 4;

impl Cue {
    fn tones(self) -> &'static [Tone] {
        match self {
            Cue::Start => &START_TONES,
            Cue::Stop => &STOP_TONES,
        }
    }
}

struct Request {
    cue: Cue,
    queued_at: Instant,
}

/// The running player thread, as seen from [`play`].
struct Player {
    sender: Sender<Request>,
    /// When the player entered its current device call, in [`clock_ms`]
    /// units; 0 while it is not inside one.
    busy_since: Arc<AtomicU64>,
}

static PLAYER: Mutex<Option<Player>> = Mutex::new(None);
static PLAYERS_STARTED: AtomicUsize = AtomicUsize::new(0);

/// Queues `cue` for playback if the Sounds setting is on. Never blocks on
/// audio: the only wait is a short lock around handing the cue over.
pub fn play(app: &AppHandle, cue: Cue) {
    let mut request = Request {
        cue,
        queued_at: Instant::now(),
    };
    let mut slot = lock(&PLAYER);

    if let Some(player) = slot.as_ref() {
        if is_stuck(player.busy_since.load(Ordering::SeqCst), clock_ms()) {
            log::warn!("The sound player is stuck in the audio driver; starting a new one");
            *slot = None;
        }
    }

    if let Some(player) = slot.as_ref() {
        match player.sender.try_send(request) {
            Ok(()) => return,
            Err(TrySendError::Full(_)) => {
                log::debug!("Sound queue full; dropping the {:?} sound", cue);
                return;
            }
            // The player thread is gone (it panicked): start a new one.
            Err(TrySendError::Disconnected(returned)) => {
                request = returned;
                *slot = None;
            }
        }
    }

    let Some(player) = start_player(app) else {
        return;
    };
    let _ = player.sender.try_send(request);
    *slot = Some(player);
}

/// Starts a player thread, unless [`MAX_PLAYERS`] were already started this
/// session.
fn start_player(app: &AppHandle) -> Option<Player> {
    if PLAYERS_STARTED.fetch_add(1, Ordering::SeqCst) >= MAX_PLAYERS {
        log::debug!("Sound player limit reached; sounds are off for this session");
        return None;
    }
    let (sender, receiver) = bounded::<Request>(QUEUE_CAPACITY);
    let busy_since = Arc::new(AtomicU64::new(0));
    let thread_busy = busy_since.clone();
    let app = app.clone();
    let spawned = thread::Builder::new()
        .name("scribe-sounds".into())
        .spawn(move || run_player(&app, &receiver, &thread_busy));
    match spawned {
        Ok(_) => Some(Player { sender, busy_since }),
        Err(error) => {
            log::warn!("Could not start the sound player: {}", error);
            None
        }
    }
}

/// The player thread. Exits when its queue is dropped (it was replaced).
fn run_player(app: &AppHandle, receiver: &Receiver<Request>, busy: &AtomicU64) {
    let mut output: Option<Output> = None;
    loop {
        let request = if output.is_some() {
            match receiver.recv_timeout(IDLE_CHECK_EVERY) {
                Ok(request) => Some(request),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match receiver.recv() {
                Ok(request) => Some(request),
                Err(_) => break,
            }
        };

        if output
            .as_ref()
            .is_some_and(|open| open.failed() || open.idle_for() >= IDLE_CLOSE_AFTER)
        {
            close(output.take(), busy);
        }

        let Some(request) = request else {
            continue;
        };
        if !should_play(sounds_enabled(app), request.queued_at.elapsed()) {
            continue;
        }
        if output.is_none() {
            output = device_call(busy, open_default_output).and_then(|opened| {
                opened
                    .map_err(|error| log::debug!("Could not open sound output: {}", error))
                    .ok()
            });
        }
        // Opening can be slow; a cue that went stale meanwhile is skipped.
        if let Some(open) = &output {
            if request.queued_at.elapsed() <= STALE_AFTER {
                open.add(request.cue);
            }
        }
    }
    close(output.take(), busy);
}

/// Closes an output stream, surviving a panic in the driver's close.
fn close(output: Option<Output>, busy: &AtomicU64) {
    if let Some(output) = output {
        device_call(busy, move || drop(output));
    }
}

/// Runs one device call with the stuck-detection clock running and any panic
/// caught, so neither a hang nor a panic silences cues for the session.
fn device_call<R>(busy: &AtomicU64, call: impl FnOnce() -> R) -> Option<R> {
    busy.store(clock_ms(), Ordering::SeqCst);
    let result = catch_unwind(AssertUnwindSafe(call));
    busy.store(0, Ordering::SeqCst);
    match result {
        Ok(value) => Some(value),
        Err(_) => {
            log::warn!("The audio driver panicked while handling a sound");
            None
        }
    }
}

/// Milliseconds on a process-wide monotonic clock, starting at 1 so that 0
/// can mean "not busy".
fn clock_ms() -> u64 {
    static BASE: OnceLock<Instant> = OnceLock::new();
    BASE.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// Whether a player that entered a device call at `busy_since` (0: it is not
/// in one) counts as stuck at `now`.
fn is_stuck(busy_since: u64, now: u64) -> bool {
    busy_since != 0 && now.saturating_sub(busy_since) >= STUCK_AFTER.as_millis() as u64
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

/// Cues currently sounding, mixed together by the output callback.
#[derive(Default)]
struct Mixer {
    voices: Vec<Voice>,
    /// Frames written since the last cue finished (0 while one is playing).
    idle_frames: u64,
}

struct Voice {
    samples: Vec<f32>,
    position: usize,
}

impl Mixer {
    fn add(&mut self, samples: Vec<f32>) {
        self.voices.push(Voice {
            samples,
            position: 0,
        });
        self.idle_frames = 0;
    }

    /// Fills an interleaved buffer of `channels` channels with the mix of all
    /// sounding cues, then forgets the cues that have finished.
    fn fill<T>(&mut self, out: &mut [T], channels: usize)
    where
        T: SizedSample + FromSample<f32>,
    {
        let channels = channels.max(1);
        for frame in out.chunks_mut(channels) {
            let mut value = 0.0;
            for voice in &mut self.voices {
                if let Some(sample) = voice.samples.get(voice.position) {
                    value += sample;
                    voice.position += 1;
                }
            }
            write_frame(frame, value.clamp(-1.0, 1.0));
        }
        self.voices
            .retain(|voice| voice.position < voice.samples.len());
        if self.voices.is_empty() {
            self.idle_frames += (out.len() / channels) as u64;
        }
    }
}

/// Writes one mono value into an interleaved frame: the front left and right
/// channels (or the only channel of a mono device) carry it and every other
/// channel (centre, subwoofer, surrounds) stays silent. Channels 0 and 1 are
/// front left and right on WASAPI, CoreAudio and ALSA alike.
fn write_frame<T>(frame: &mut [T], value: f32)
where
    T: SizedSample + FromSample<f32>,
{
    let sample = T::from_sample(value);
    for (index, slot) in frame.iter_mut().enumerate() {
        *slot = if index < 2 { sample } else { T::EQUILIBRIUM };
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// An open output stream and the mixer feeding it. Dropping it closes the
/// device.
struct Output {
    _stream: cpal::Stream,
    mixer: Arc<Mutex<Mixer>>,
    sample_rate: u32,
    failed: Arc<AtomicBool>,
}

impl Output {
    fn open(device: &Device) -> Result<Output, String> {
        let supported = device
            .default_output_config()
            .map_err(|error| format!("no usable output config: {}", error))?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        match sample_format {
            SampleFormat::F32 => Self::open_as::<f32>(device, config),
            SampleFormat::F64 => Self::open_as::<f64>(device, config),
            SampleFormat::I16 => Self::open_as::<i16>(device, config),
            SampleFormat::I32 => Self::open_as::<i32>(device, config),
            SampleFormat::U16 => Self::open_as::<u16>(device, config),
            SampleFormat::U8 => Self::open_as::<u8>(device, config),
            SampleFormat::I8 => Self::open_as::<i8>(device, config),
            other => Err(format!("unsupported sample format {:?}", other)),
        }
    }

    fn open_as<T>(device: &Device, config: StreamConfig) -> Result<Output, String>
    where
        T: SizedSample + FromSample<f32>,
    {
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        let failed = Arc::new(AtomicBool::new(false));
        let channels = config.channels as usize;
        let callback_mixer = mixer.clone();
        let callback_failed = failed.clone();
        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [T], _| lock(&callback_mixer).fill(data, channels),
                move |error| {
                    log::debug!("Sound output stream error: {}", error);
                    callback_failed.store(true, Ordering::SeqCst);
                },
                None,
            )
            .map_err(|error| error.to_string())?;
        stream.play().map_err(|error| error.to_string())?;
        Ok(Output {
            _stream: stream,
            mixer,
            sample_rate: config.sample_rate.0.max(1),
            failed,
        })
    }

    fn add(&self, cue: Cue) {
        lock(&self.mixer).add(render(cue, self.sample_rate));
    }

    fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    /// How long the stream has played only silence.
    fn idle_for(&self) -> Duration {
        let frames = lock(&self.mixer).idle_frames;
        Duration::from_secs_f64(frames as f64 / self.sample_rate as f64)
    }
}

fn open_default_output() -> Result<Output, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| "no output device".to_string())?;
    Output::open(&device)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::Sample;

    const RATE: u32 = 48_000;

    /// Sign changes in `samples[from..to]`.
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

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |max, s| max.max(s.abs()))
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
    fn cues_never_clip() {
        for cue in [Cue::Start, Cue::Stop] {
            for rate in [22_050, 44_100, 48_000, 96_000] {
                let samples = render(cue, rate);
                let peak = peak(&samples);
                assert!(
                    peak <= PEAK_GAIN + 1e-4,
                    "{cue:?} at {rate} peaked at {peak}"
                );
                assert!(peak > PEAK_GAIN * 0.9, "{cue:?} at {rate} is too quiet");
                assert!(samples.iter().all(|s| s.is_finite()));
            }
        }
    }

    #[test]
    fn cues_fade_in_and_out_instead_of_clicking() {
        // A tone switched on or off abruptly reaches near its peak within the
        // first or last millisecond (most of a cycle at 440-880 Hz). The
        // envelope keeps both edges about 40 dB under the peak.
        for cue in [Cue::Start, Cue::Stop] {
            for rate in [44_100, 48_000] {
                let samples = render(cue, rate);
                let one_ms = rate as usize / 1_000;
                let head = peak(&samples[..one_ms]);
                let tail = peak(&samples[samples.len() - one_ms..]);
                assert!(head < PEAK_GAIN / 100.0, "{cue:?} starts at {head}");
                assert!(tail < PEAK_GAIN / 100.0, "{cue:?} ends at {tail}");
            }
        }
    }

    #[test]
    fn blips_meet_without_a_click() {
        // Around the hand-over (55 ms) both blips are near the floor.
        for cue in [Cue::Start, Cue::Stop] {
            let samples = render(cue, RATE);
            let handover = (0.055 * RATE as f32) as usize;
            let one_ms = RATE as usize / 1_000;
            let around = peak(&samples[handover - one_ms / 2..handover + one_ms / 2]);
            assert!(
                around < PEAK_GAIN / 100.0,
                "{cue:?} clicks at the hand-over"
            );
        }
    }

    #[test]
    fn overlapping_cues_are_mixed_not_queued() {
        let start = render(Cue::Start, RATE);
        let stop = render(Cue::Stop, RATE);
        let offset = 2_000; // the start cue arrives 42 ms into the stop cue
        let total = offset + start.len() + 64;
        let mut mixer = Mixer::default();
        let mut out = vec![0.0f32; total * 2];

        mixer.add(stop.clone());
        mixer.fill(&mut out[..offset * 2], 2);
        mixer.add(start.clone());
        mixer.fill(&mut out[offset * 2..], 2);

        for frame in 0..total {
            let expected = stop.get(frame).copied().unwrap_or(0.0)
                + frame
                    .checked_sub(offset)
                    .and_then(|index| start.get(index).copied())
                    .unwrap_or(0.0);
            assert!(
                (out[frame * 2] - expected).abs() < 1e-6,
                "frame {frame}: {} != {expected}",
                out[frame * 2]
            );
            assert_eq!(out[frame * 2], out[frame * 2 + 1], "left and right match");
        }
        assert!(mixer.voices.is_empty(), "finished cues are forgotten");
    }

    #[test]
    fn mixer_counts_silence_only_after_the_last_cue() {
        let mut mixer = Mixer::default();
        let mut out = vec![0.0f32; 480];
        mixer.fill(&mut out, 1);
        assert_eq!(mixer.idle_frames, 480);

        mixer.add(render(Cue::Start, RATE));
        assert_eq!(mixer.idle_frames, 0, "a new cue resets the idle count");
        mixer.fill(&mut out, 1);
        assert_eq!(mixer.idle_frames, 0, "still playing");

        let mut rest = vec![0.0f32; 6_000];
        mixer.fill(&mut rest, 1);
        assert!(mixer.voices.is_empty());
        assert_eq!(mixer.idle_frames, 6_000);
        assert!(
            rest[5_500..].iter().all(|s| *s == 0.0),
            "silence after the cue"
        );
    }

    #[test]
    fn only_front_left_and_right_carry_the_cue() {
        let value = 0.5;
        let mut mono = [0.0f32; 1];
        write_frame(&mut mono, value);
        assert_eq!(mono, [0.5]);

        let mut stereo = [0.0f32; 2];
        write_frame(&mut stereo, value);
        assert_eq!(stereo, [0.5, 0.5]);

        // 5.1: FL FR C LFE SL SR. Centre, subwoofer and surrounds stay silent.
        let mut surround = [9.0f32; 6];
        write_frame(&mut surround, value);
        assert_eq!(surround, [0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);

        // Silence is the format's equilibrium, not a raw 0 (unsigned formats).
        let mut unsigned = [0u16; 8];
        write_frame(&mut unsigned, 0.0);
        assert!(unsigned.iter().all(|s| *s == u16::EQUILIBRIUM));
        let mut signed = [7i16; 8];
        write_frame(&mut signed, value);
        assert!(signed[0] > 16_000 && signed[0] == signed[1]);
        assert!(signed[2..].iter().all(|s| *s == 0));
    }

    #[test]
    fn mixer_fills_surround_buffers_frame_by_frame() {
        let mut mixer = Mixer::default();
        mixer.add(vec![0.25, -0.25]);
        let mut out = vec![1.0f32; 3 * 6];
        mixer.fill(&mut out, 6);
        assert_eq!(&out[..6], &[0.25, 0.25, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(&out[6..12], &[-0.25, -0.25, 0.0, 0.0, 0.0, 0.0]);
        assert!(out[12..].iter().all(|s| *s == 0.0));
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

    #[test]
    fn a_player_is_stuck_only_inside_a_long_device_call() {
        let stuck_ms = STUCK_AFTER.as_millis() as u64;
        assert!(!is_stuck(0, 1_000_000), "not in a device call");
        assert!(!is_stuck(1_000, 1_000 + stuck_ms - 1));
        assert!(is_stuck(1_000, 1_000 + stuck_ms));
    }

    #[test]
    fn device_calls_survive_a_driver_panic() {
        let busy = AtomicU64::new(0);
        let result: Option<()> = device_call(&busy, || panic!("driver bug"));
        assert!(result.is_none());
        assert_eq!(busy.load(Ordering::SeqCst), 0, "busy clock is cleared");
        assert_eq!(device_call(&busy, || 7), Some(7));
    }

    /// Without a device this cannot run the player end to end, so it checks
    /// the source instead: each recording event is emitted from exactly one
    /// helper in audio.rs, and that helper plays the matching cue.
    #[test]
    fn recording_events_play_their_cues() {
        let audio = include_str!("audio.rs");
        for (event, helper, cue) in [
            (
                "\"audio://recording-started\"",
                "fn emit_recording_started(",
                "Cue::Start",
            ),
            (
                "\"audio://recording-stopped\"",
                "fn emit_recording_stopped(",
                "Cue::Stop",
            ),
        ] {
            assert_eq!(
                audio.matches(event).count(),
                1,
                "{event} is emitted only from {helper}"
            );
            let body_start = audio.find(helper).expect("the helper exists");
            let body = &audio[body_start..];
            let body = &body[..body.find("\n}\n").expect("the helper ends")];
            assert!(body.contains(event), "{helper} emits {event}");
            assert!(
                body.contains(&format!("crate::sounds::play(app, crate::sounds::{cue})")),
                "{helper} plays {cue}"
            );
            let calls = audio.matches(&helper[3..]).count() - 1;
            assert!(calls >= 1, "{helper} is called");
        }
    }

    /// Plays two overlapping cues through a real output stream, checks the
    /// device consumed every sample of both and that the stream then reports
    /// itself idle (which is when the player closes it), then closes it.
    /// Needs an output device, and on a real one it is audible, so run it by
    /// hand: `cargo test --lib sounds -- --ignored`.
    #[test]
    #[ignore = "needs an output device"]
    fn cues_play_through_a_real_output_stream() {
        let output = open_default_output().expect("an output stream opens");
        output.add(Cue::Stop);
        thread::sleep(Duration::from_millis(40));
        output.add(Cue::Start);

        let deadline = Instant::now() + Duration::from_secs(3);
        while !lock(&output.mixer).voices.is_empty() {
            assert!(
                Instant::now() < deadline,
                "the device stopped taking samples"
            );
            thread::sleep(Duration::from_millis(10));
        }
        while output.idle_for() < Duration::from_millis(100) {
            assert!(Instant::now() < deadline, "the stream never went idle");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!output.failed(), "the stream reported no error");
        let busy = AtomicU64::new(0);
        close(Some(output), &busy);
        assert_eq!(busy.load(Ordering::SeqCst), 0);
    }
}
