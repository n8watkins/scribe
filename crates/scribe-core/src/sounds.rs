//! The short start and stop dictation cues, played natively so they work
//! with no webview open.
//!
//! The tones match the earlier Web Audio version exactly: two sine blips per
//! cue at a peak gain of 0.15, each with a 12 ms exponential attack and an
//! exponential release to silence by the end of the blip.
//!
//! How playback works:
//!
//! - [`play_with`] never waits on audio. It hands the cue to a player thread
//!   through a small bounded queue; when the queue is full the cue is dropped.
//! - The player keeps one output stream open and mixes each new cue into it
//!   the moment it arrives, so cues overlap the way the Web Audio ones did.
//! - The start cue opens the stream and it stays open for the whole
//!   recording, so the stop cue never lands on a freshly opened device (some
//!   Bluetooth, HDMI and USB outputs mute the first few hundred milliseconds
//!   of a new stream). It closes [`IDLE_CLOSE_AFTER`] after the stop cue has
//!   finished, so no output device stays open while Scribe is idle. A
//!   recording that never reports its end is let go after
//!   [`max_recording_hold`].
//! - Idle is measured by the frames the device consumed and, as a fallback,
//!   by the wall clock, so a stream whose callback silently stops is still
//!   closed. A stream that reports an error, or stops taking samples while a
//!   cue is waiting, is closed and the next cue opens a fresh one on the
//!   *current* default device (which is also how a changed default device is
//!   picked up).
//! - The output callback never allocates, frees or waits on a lock: cues are
//!   rendered before they are handed over, finished cues are freed by the
//!   player thread, and a contended lock means one buffer of silence.
//! - Device calls can hang or panic inside a driver. Panics are caught. A
//!   player stuck in one device call for [`STUCK_AFTER`] is abandoned and its
//!   cue handed to a fresh player. At most [`MAX_UNPROVEN_STARTS`] players
//!   are started in a row without one of them playing a cue, so a broken
//!   driver costs a few parked threads at worst. With that budget spent, a
//!   stuck player is kept (it plays the next cue if it recovers) and a dead
//!   one is replaced once per [`BUDGET_COOL_OFF`]; any played cue restores
//!   the full budget.

use std::{
    any::Any,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use crate::backend::BackendState;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, SampleFormat, SizedSample, StreamConfig,
};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender, TrySendError};

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
/// How long the output stream stays open after the last cue finished, when
/// no recording is in progress.
const IDLE_CLOSE_AFTER: Duration = Duration::from_secs(3);
/// Extra wall-clock time on top of [`IDLE_CLOSE_AFTER`] before the stream is
/// closed even though its callback has not reported the silence (a callback
/// that silently stopped running).
const IDLE_CLOCK_SLACK: Duration = Duration::from_secs(2);
/// A cue that should have finished this long ago but is still waiting in the
/// mixer means the device stopped taking samples.
const STALL_AFTER: Duration = Duration::from_secs(2);
/// How often the player checks an open stream.
const CHECK_EVERY: Duration = Duration::from_millis(250);
/// A cue still waiting this long after it was asked for is dropped: a late
/// blip would be more confusing than none.
const STALE_AFTER: Duration = Duration::from_secs(1);
/// Cues waiting for the player. Recording start and stop come one at a time,
/// so a full queue only happens when the player is stuck.
const QUEUE_CAPACITY: usize = 4;
/// A player inside one device call this long is treated as stuck. Opening an
/// output while Windows switches a Bluetooth headset between its headset and
/// stereo profiles (which a recording start can trigger) can take several
/// seconds; this sits well above that.
const STUCK_AFTER: Duration = Duration::from_secs(15);
/// Players started in a row without any of them playing a cue. A successful
/// cue resets the count.
const MAX_UNPROVEN_STARTS: usize = 4;
/// With the start budget spent, how long before one more player may start.
const BUDGET_COOL_OFF: Duration = Duration::from_secs(60);
/// After a stream error during a recording, how long before the one reopen,
/// so the stop cue still finds an open device.
const REOPEN_AFTER: Duration = Duration::from_millis(500);

/// The longest a recording can run (the largest allowed safety ceiling) plus
/// a minute: how long a start cue can hold the stream open without the
/// matching stop cue.
fn max_recording_hold() -> Duration {
    let ceiling_ms = *crate::settings::RECORDING_CEILING_RANGE_MS.end();
    Duration::from_millis(u64::from(ceiling_ms)) + Duration::from_secs(60)
}

impl Cue {
    fn tones(self) -> &'static [Tone] {
        match self {
            Cue::Start => &START_TONES,
            Cue::Stop => &STOP_TONES,
        }
    }
}

/// What the player is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Cue(Cue),
    /// A recording ended without a stop cue (it failed): stop holding the
    /// stream open for it. Plays nothing.
    RecordingEnded,
}

struct Request {
    event: Event,
    queued_at: Instant,
}

// ---------------------------------------------------------------------------
// Handing cues to the player
// ---------------------------------------------------------------------------

/// The running player thread, as seen from [`play_with`].
struct PlayerHandle {
    sender: Sender<Request>,
    /// When the player entered its current device call, in [`clock_ms`]
    /// units; 0 while it is not inside one.
    busy_since: Arc<AtomicU64>,
}

/// Owns the current player and decides when to replace it.
struct Dispatcher {
    current: Option<PlayerHandle>,
    /// Players started since a player last played a cue. Shared with the
    /// players, which reset it.
    unproven_starts: Arc<AtomicUsize>,
    /// When a start was first refused for a spent budget, in [`clock_ms`]
    /// units.
    budget_spent_at: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
enum Dispatched {
    Queued,
    QueueFull,
    NoPlayer,
}

impl Dispatcher {
    fn new() -> Self {
        Self {
            current: None,
            unproven_starts: Arc::new(AtomicUsize::new(0)),
            budget_spent_at: None,
        }
    }

    /// Whether another player may start at `now_ms`: while the budget lasts,
    /// and once per [`BUDGET_COOL_OFF`] after it is spent.
    fn may_start(&mut self, now_ms: u64) -> bool {
        if self.unproven_starts.load(Ordering::SeqCst) < MAX_UNPROVEN_STARTS {
            return true;
        }
        match self.budget_spent_at {
            None => {
                self.budget_spent_at = Some(now_ms);
                false
            }
            Some(at) if now_ms.saturating_sub(at) >= BUDGET_COOL_OFF.as_millis() as u64 => {
                self.budget_spent_at = None;
                self.unproven_starts
                    .store(MAX_UNPROVEN_STARTS - 1, Ordering::SeqCst);
                true
            }
            Some(_) => false,
        }
    }

    /// Hands `request` to the current player, replacing a player that is
    /// stuck or gone. `start` starts a new player. A recording-ended notice
    /// never starts one: with no player there is nothing to let go of.
    fn dispatch(
        &mut self,
        mut request: Request,
        now_ms: u64,
        start: &mut dyn FnMut(Arc<AtomicUsize>) -> Option<PlayerHandle>,
    ) -> Dispatched {
        let is_cue = matches!(request.event, Event::Cue(_));
        let stuck = self
            .current
            .as_ref()
            .is_some_and(|player| is_stuck(player.busy_since.load(Ordering::SeqCst), now_ms));
        // A stuck player is only given up when a replacement may start;
        // otherwise it keeps the cue and plays it if it recovers.
        if stuck && is_cue && self.may_start(now_ms) {
            log::warn!("The sound player is stuck in the audio driver; starting a new one");
            self.current = None;
            return self.start_with(request, start);
        }

        if let Some(player) = &self.current {
            match player.sender.try_send(request) {
                Ok(()) => return Dispatched::Queued,
                Err(TrySendError::Full(_)) => return Dispatched::QueueFull,
                // The player thread is gone (it panicked): start a new one.
                Err(TrySendError::Disconnected(returned)) => {
                    request = returned;
                    self.current = None;
                }
            }
        }

        if !is_cue || !self.may_start(now_ms) {
            return Dispatched::NoPlayer;
        }
        self.start_with(request, start)
    }

    fn start_with(
        &mut self,
        request: Request,
        start: &mut dyn FnMut(Arc<AtomicUsize>) -> Option<PlayerHandle>,
    ) -> Dispatched {
        self.unproven_starts.fetch_add(1, Ordering::SeqCst);
        let Some(player) = start(self.unproven_starts.clone()) else {
            return Dispatched::NoPlayer;
        };
        let outcome = match player.sender.try_send(request) {
            Ok(()) => Dispatched::Queued,
            Err(_) => Dispatched::QueueFull,
        };
        self.current = Some(player);
        outcome
    }
}

/// Queues `cue` for playback if the Sounds setting is on. Never blocks on
/// audio: the only wait is a short lock around handing the cue over. `state`
/// is where the player reads the Sounds setting from.
pub fn play_with(state: &BackendState, cue: Cue) {
    let request = Request {
        event: Event::Cue(cue),
        queued_at: Instant::now(),
    };
    let outcome = lock(dispatcher()).dispatch(request, clock_ms(), &mut |unproven| {
        start_player(state, unproven)
    });
    if outcome != Dispatched::Queued {
        log::debug!("Skipping the {:?} sound: {:?}", cue, outcome);
    }
}

/// Tells the player a recording ended without a stop cue (it failed), so it
/// stops holding the output stream open for it. Plays nothing.
pub fn recording_ended() {
    #[cfg(test)]
    tests::RECORDING_ENDED_NOTICES.fetch_add(1, Ordering::SeqCst);
    let request = Request {
        event: Event::RecordingEnded,
        queued_at: Instant::now(),
    };
    lock(dispatcher()).dispatch(request, clock_ms(), &mut |_| None);
}

fn dispatcher() -> &'static Mutex<Dispatcher> {
    static DISPATCHER: OnceLock<Mutex<Dispatcher>> = OnceLock::new();
    DISPATCHER.get_or_init(|| Mutex::new(Dispatcher::new()))
}

/// Starts the real player thread.
fn start_player(state: &BackendState, unproven_starts: Arc<AtomicUsize>) -> Option<PlayerHandle> {
    let (sender, receiver) = bounded::<Request>(QUEUE_CAPACITY);
    let busy_since = Arc::new(AtomicU64::new(0));
    let thread_busy = busy_since.clone();
    let state = state.clone();
    let spawned = thread::Builder::new()
        .name("scribe-sounds".into())
        .spawn(move || {
            let player = Player::new(
                open_default_output,
                move || sounds_enabled(&state),
                thread_busy,
                unproven_starts,
            );
            run_player(player, &receiver);
        });
    match spawned {
        Ok(_) => Some(PlayerHandle { sender, busy_since }),
        Err(error) => {
            log::warn!("Could not start the sound player: {}", error);
            None
        }
    }
}

/// The player thread's loop. Exits when its queue is dropped (the player was
/// replaced).
fn run_player<O, S>(mut player: Player<O, S>, receiver: &Receiver<Request>)
where
    O: FnMut() -> Result<OpenStream, String>,
    S: FnMut() -> Option<bool>,
{
    loop {
        let request = match player.wait() {
            Some(wait) => match receiver.recv_timeout(wait) {
                Ok(request) => Some(request),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match receiver.recv() {
                Ok(request) => Some(request),
                Err(_) => break,
            },
        };
        player.step(request, Instant::now());
    }
    player.close();
}

/// Milliseconds on a process-wide monotonic clock, starting at 1 so that 0
/// can mean "not busy".
fn clock_ms() -> u64 {
    static BASE: OnceLock<Instant> = OnceLock::new();
    BASE.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// Whether a player that entered a device call at `busy_since` (0: it is not
/// in one) counts as stuck at `now`.
///
/// On platforms whose monotonic clock keeps running through system sleep, a
/// device call that spans a sleep can look stuck once after wake. That costs
/// at most one extra player start, and the budget recovers with the next
/// played cue, so it is not worth tracking sleep separately.
fn is_stuck(busy_since: u64, now: u64) -> bool {
    busy_since != 0 && now.saturating_sub(busy_since) >= STUCK_AFTER.as_millis() as u64
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

/// Reads the Sounds setting at the moment the cue is about to play, so a
/// change takes effect at once.
fn sounds_enabled(state: &BackendState) -> Option<bool> {
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

// ---------------------------------------------------------------------------
// The player: one output stream, opened and closed by rule
// ---------------------------------------------------------------------------

/// The player thread's state. The stream opener and the settings source are
/// injected so the open and close rules can be tested without a device.
struct Player<O, S> {
    open: O,
    sounds_enabled: S,
    output: Option<OpenStream>,
    /// When the last cue handed to the open stream should have finished.
    last_cue_end: Option<Instant>,
    /// Set by a played start cue, cleared by any stop cue or a
    /// recording-ended notice.
    recording_since: Option<Instant>,
    /// When to reopen a stream that failed during a recording.
    reopen_at: Option<Instant>,
    /// The stream was already reopened once during this recording.
    reopened: bool,
    /// Swapped with the mixer's cue list to take finished cues out without
    /// allocating under the lock.
    spare_voices: Vec<Voice>,
    busy: Arc<AtomicU64>,
    unproven_starts: Arc<AtomicUsize>,
}

impl<O, S> Player<O, S>
where
    O: FnMut() -> Result<OpenStream, String>,
    S: FnMut() -> Option<bool>,
{
    fn new(
        open: O,
        sounds_enabled: S,
        busy: Arc<AtomicU64>,
        unproven_starts: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            open,
            sounds_enabled,
            output: None,
            last_cue_end: None,
            recording_since: None,
            reopen_at: None,
            reopened: false,
            spare_voices: Vec::with_capacity(VOICE_ROOM),
            busy,
            unproven_starts,
        }
    }

    /// How long to wait for the next cue before checking again; `None` (wait
    /// for ever) when no stream is open and no reopen is due.
    fn wait(&self) -> Option<Duration> {
        (self.output.is_some() || self.reopen_at.is_some()).then_some(CHECK_EVERY)
    }

    /// Handles one wake-up at `now`: frees finished cues, applies the close
    /// rules, reopens a stream that failed mid-recording, then handles
    /// `request` if there is one.
    fn step(&mut self, request: Option<Request>, now: Instant) {
        // While a cue is sounding, leave the mixer lock to the output
        // callback, so housekeeping can never cost it a buffer.
        let cue_sounding = self.output.is_some() && self.last_cue_end.is_some_and(|end| now < end);
        if !cue_sounding {
            self.free_finished_cues();
            if self.should_close(now) {
                let failed = self
                    .output
                    .as_ref()
                    .is_some_and(|output| output.failed.load(Ordering::SeqCst));
                self.close();
                if failed && self.recording_since.is_some() && !self.reopened {
                    self.reopen_at = Some(now + REOPEN_AFTER);
                }
            }
        }
        if self.reopen_at.is_some_and(|at| now >= at) {
            self.reopen_at = None;
            self.reopened = true;
            if self.recording_since.is_some() && self.output.is_none() {
                self.open_output(now);
            }
        }

        let Some(request) = request else {
            return;
        };
        let cue = match request.event {
            Event::Cue(cue) => cue,
            Event::RecordingEnded => {
                self.recording_since = None;
                self.reopen_at = None;
                return;
            }
        };
        if cue == Cue::Stop {
            self.recording_since = None;
            self.reopen_at = None;
        }
        let waited = now.saturating_duration_since(request.queued_at);
        if !should_play((self.sounds_enabled)(), waited) {
            return;
        }
        // Timers start once the device is open, not before a slow open.
        let now = match &self.output {
            Some(_) => now,
            None => match self.open_output(now) {
                Some(opened_at) => opened_at,
                None => return,
            },
        };
        let Some(output) = &self.output else {
            return;
        };
        // Opening can be slow; a cue that went stale meanwhile is skipped.
        if now.saturating_duration_since(request.queued_at) > STALE_AFTER {
            return;
        }

        // Render before taking the lock the output callback needs.
        let samples = render(cue, output.sample_rate);
        lock(&output.mixer).add(samples);
        self.last_cue_end = Some(now + Duration::from_secs_f32(cue_length_s(cue)));
        if cue == Cue::Start {
            self.recording_since = Some(now);
            self.reopened = false;
        }
        self.unproven_starts.store(0, Ordering::SeqCst);
    }

    /// Opens a stream and returns when it finished opening.
    fn open_output(&mut self, now: Instant) -> Option<Instant> {
        let open = &mut self.open;
        let opened = device_call(&self.busy, open).and_then(|opened| {
            opened
                .map_err(|error| log::debug!("Could not open sound output: {}", error))
                .ok()
        })?;
        let opened_at = opened.opened_at.max(now);
        self.output = Some(opened);
        self.last_cue_end = Some(opened_at);
        Some(opened_at)
    }

    /// Whether the open stream should be closed at `now`.
    fn should_close(&self, now: Instant) -> bool {
        let Some(output) = &self.output else {
            return false;
        };
        if output.failed.load(Ordering::SeqCst) {
            return true;
        }
        let Some(last_cue_end) = self.last_cue_end else {
            return true;
        };
        let (pending, idle_frames) = {
            let mixer = lock(&output.mixer);
            (mixer.has_pending(), mixer.idle_frames)
        };
        if pending {
            // The device stopped taking samples: a fresh stream may work.
            return now >= last_cue_end + STALL_AFTER;
        }
        if let Some(since) = self.recording_since {
            if now < since + max_recording_hold() {
                return false;
            }
        }
        let idle_by_frames =
            Duration::from_secs_f64(idle_frames as f64 / output.sample_rate as f64)
                >= IDLE_CLOSE_AFTER;
        let idle_by_clock = now >= last_cue_end + IDLE_CLOSE_AFTER + IDLE_CLOCK_SLACK;
        idle_by_frames || idle_by_clock
    }

    /// Once every cue has finished, takes them out of the mixer (a swap with
    /// an empty list, no allocation under the lock) and frees them here, not
    /// in the output callback.
    fn free_finished_cues(&mut self) {
        let Some(output) = &self.output else {
            return;
        };
        {
            let mut mixer = lock(&output.mixer);
            if mixer.voices.is_empty() || mixer.has_pending() {
                return;
            }
            std::mem::swap(&mut mixer.voices, &mut self.spare_voices);
        }
        self.spare_voices.clear();
    }

    /// Closes the open stream, surviving a panic in the driver's close.
    fn close(&mut self) {
        if let Some(output) = self.output.take() {
            device_call(&self.busy, move || drop(output));
        }
        self.last_cue_end = None;
    }
}

// ---------------------------------------------------------------------------
// Tones
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Mixing and the output stream
// ---------------------------------------------------------------------------

/// Room for cues in the mixer, reserved up front so adding a cue under the
/// lock does not allocate in normal use.
const VOICE_ROOM: usize = 8;

/// Cues currently sounding, mixed together by the output callback.
struct Mixer {
    /// Cues being played. Finished ones stay here until the player thread
    /// frees them, so the callback never frees memory.
    voices: Vec<Voice>,
    /// Frames written since the last cue finished (0 while one is playing).
    idle_frames: u64,
}

struct Voice {
    samples: Vec<f32>,
    position: usize,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            voices: Vec::with_capacity(VOICE_ROOM),
            idle_frames: 0,
        }
    }
}

impl Voice {
    fn finished(&self) -> bool {
        self.position >= self.samples.len()
    }
}

impl Mixer {
    fn add(&mut self, samples: Vec<f32>) {
        self.voices.push(Voice {
            samples,
            position: 0,
        });
        self.idle_frames = 0;
    }

    fn has_pending(&self) -> bool {
        self.voices.iter().any(|voice| !voice.finished())
    }

    /// Fills an interleaved buffer of `channels` channels with the mix of all
    /// sounding cues. Never allocates or frees.
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
        if !self.has_pending() {
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

/// The output callback: mixes into `data`, or writes one buffer of silence
/// when the player thread holds the lock at that instant, so the callback
/// never waits.
fn output_callback<T>(mixer: &Mutex<Mixer>, data: &mut [T], channels: usize)
where
    T: SizedSample + FromSample<f32>,
{
    match mixer.try_lock() {
        Ok(mut mixer) => mixer.fill(data, channels),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => {
            poisoned.into_inner().fill(data, channels)
        }
        Err(std::sync::TryLockError::WouldBlock) => data.fill(T::EQUILIBRIUM),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// An open output stream and the mixer feeding it. Dropping it closes the
/// device.
struct OpenStream {
    /// The platform stream; dropping it closes the device.
    _stream: Box<dyn Any>,
    mixer: Arc<Mutex<Mixer>>,
    sample_rate: u32,
    /// Set by the stream's error callback.
    failed: Arc<AtomicBool>,
    /// When the open finished; the player's timers start here.
    opened_at: Instant,
}

fn open_default_output() -> Result<OpenStream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| "no output device".to_string())?;
    open_output(&device)
}

fn open_output(device: &Device) -> Result<OpenStream, String> {
    let supported = device
        .default_output_config()
        .map_err(|error| format!("no usable output config: {}", error))?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    match sample_format {
        SampleFormat::F32 => open_output_as::<f32>(device, config),
        SampleFormat::F64 => open_output_as::<f64>(device, config),
        SampleFormat::I16 => open_output_as::<i16>(device, config),
        SampleFormat::I32 => open_output_as::<i32>(device, config),
        SampleFormat::U16 => open_output_as::<u16>(device, config),
        SampleFormat::U8 => open_output_as::<u8>(device, config),
        SampleFormat::I8 => open_output_as::<i8>(device, config),
        other => Err(format!("unsupported sample format {:?}", other)),
    }
}

fn open_output_as<T>(device: &Device, config: StreamConfig) -> Result<OpenStream, String>
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
            move |data: &mut [T], _| output_callback(&callback_mixer, data, channels),
            move |error| {
                log::debug!("Sound output stream error: {}", error);
                callback_failed.store(true, Ordering::SeqCst);
            },
            None,
        )
        .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    Ok(OpenStream {
        _stream: Box::new(stream),
        mixer,
        sample_rate: config.sample_rate.0.max(1),
        failed,
        opened_at: Instant::now(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use cpal::Sample;
    use std::{cell::Cell, rc::Rc};

    const RATE: u32 = 48_000;

    /// Counts [`recording_ended`] calls, for the audio.rs test.
    pub(crate) static RECORDING_ENDED_NOTICES: AtomicUsize = AtomicUsize::new(0);

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
        assert!(!mixer.has_pending(), "both cues finished");
        assert_eq!(
            mixer.voices.len(),
            2,
            "the callback leaves finished cues for the player to free"
        );
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
        assert!(!mixer.has_pending());
        assert_eq!(mixer.idle_frames, 6_000);
        assert!(
            rest[5_500..].iter().all(|s| *s == 0.0),
            "silence after the cue"
        );
    }

    #[test]
    fn a_busy_mixer_lock_means_one_buffer_of_silence() {
        let mixer = Mutex::new(Mixer::default());
        lock(&mixer).add(vec![0.5; 4]);
        let held = lock(&mixer);
        let mut out = [1.0f32; 4];
        output_callback(&mixer, &mut out, 2);
        assert_eq!(out, [0.0; 4], "silence instead of waiting");
        drop(held);
        output_callback(&mixer, &mut out, 2);
        assert_eq!(out, [0.5; 4], "the cue plays on the next buffer");
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
        assert!(stuck_ms >= 15_000, "well above a slow Bluetooth open");
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

    // --- The player's open and close rules, on a fake stream -------------

    /// A fake output: counts opens and closes and lets the test play the
    /// device's part by consuming frames from the mixer.
    struct Fake {
        opens: Rc<Cell<usize>>,
        closes: Rc<Cell<usize>>,
        mixers: Rc<std::cell::RefCell<Vec<Arc<Mutex<Mixer>>>>>,
        failed: Rc<std::cell::RefCell<Vec<Arc<AtomicBool>>>>,
        enabled: Rc<Cell<Option<bool>>>,
        open_fails: Rc<Cell<bool>>,
        /// When the next open finishes (a slow open); `None`: at once.
        open_finishes_at: Rc<Cell<Option<Instant>>>,
        created: Instant,
    }

    struct CloseCounter(Rc<Cell<usize>>);

    impl Drop for CloseCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    impl Fake {
        fn new() -> Self {
            Self {
                opens: Rc::default(),
                closes: Rc::default(),
                mixers: Rc::default(),
                failed: Rc::default(),
                enabled: Rc::new(Cell::new(Some(true))),
                open_fails: Rc::default(),
                open_finishes_at: Rc::default(),
                created: Instant::now(),
            }
        }

        fn player(
            &self,
        ) -> Player<impl FnMut() -> Result<OpenStream, String>, impl FnMut() -> Option<bool>>
        {
            self.player_sharing(Arc::new(AtomicUsize::new(0)))
        }

        /// A player sharing the dispatcher's start budget.
        fn player_sharing(
            &self,
            unproven_starts: Arc<AtomicUsize>,
        ) -> Player<impl FnMut() -> Result<OpenStream, String>, impl FnMut() -> Option<bool>>
        {
            let (opens, closes, mixers, failed, fails) = (
                self.opens.clone(),
                self.closes.clone(),
                self.mixers.clone(),
                self.failed.clone(),
                self.open_fails.clone(),
            );
            let (finishes_at, created) = (self.open_finishes_at.clone(), self.created);
            let enabled = self.enabled.clone();
            Player::new(
                move || {
                    if fails.get() {
                        return Err("no device".into());
                    }
                    opens.set(opens.get() + 1);
                    let mixer = Arc::new(Mutex::new(Mixer::default()));
                    let flag = Arc::new(AtomicBool::new(false));
                    mixers.borrow_mut().push(mixer.clone());
                    failed.borrow_mut().push(flag.clone());
                    Ok(OpenStream {
                        _stream: Box::new(CloseCounter(closes.clone())),
                        mixer,
                        sample_rate: RATE,
                        failed: flag,
                        // Before any test time, so "at once" means at `now`.
                        opened_at: finishes_at.take().unwrap_or(created),
                    })
                },
                move || enabled.get(),
                Arc::new(AtomicU64::new(0)),
                unproven_starts,
            )
        }

        /// The device consumes `ms` of audio from the newest stream, in 10 ms
        /// buffers like a real callback.
        fn consume(&self, ms: u64) {
            let mixer = self.mixers.borrow().last().cloned().expect("a stream");
            let mut buffer = vec![0.0f32; RATE as usize / 100 * 2];
            for _ in 0..ms / 10 {
                lock(&mixer).fill(&mut buffer, 2);
            }
        }
    }

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn cue(cue: Cue, queued_at: Instant) -> Option<Request> {
        Some(Request {
            event: Event::Cue(cue),
            queued_at,
        })
    }

    #[test]
    fn a_stop_cue_closes_the_stream_three_seconds_after_it_ends() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        assert_eq!(player.wait(), None, "nothing open: sleep until a cue");

        player.step(cue(Cue::Stop, base), base);
        assert_eq!(fake.opens.get(), 1);
        assert_eq!(player.wait(), Some(CHECK_EVERY));

        fake.consume(130 + 2_900);
        player.step(None, at(base, 3_030));
        assert_eq!(fake.closes.get(), 0, "not yet 3 s of silence");

        fake.consume(100);
        player.step(None, at(base, 3_130));
        assert_eq!(fake.closes.get(), 1, "closed after 3 s of silence");
        assert_eq!(player.wait(), None);
    }

    #[test]
    fn the_next_cue_after_a_close_opens_a_fresh_stream() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Stop, base), base);
        fake.consume(3_200);
        // The close and the next cue land on the same wake-up.
        player.step(cue(Cue::Start, at(base, 3_200)), at(base, 3_200));
        assert_eq!(fake.closes.get(), 1, "the idle stream was closed");
        assert_eq!(fake.opens.get(), 2, "and a fresh one opened for the cue");
        let mixer = fake.mixers.borrow()[1].clone();
        assert!(lock(&mixer).has_pending(), "the cue went to the new stream");
    }

    #[test]
    fn the_stream_stays_open_for_the_whole_recording() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Start, base), base);
        fake.consume(60_000);
        player.step(None, at(base, 60_000));
        assert_eq!(fake.closes.get(), 0, "a minute in, still open");

        player.step(cue(Cue::Stop, at(base, 60_000)), at(base, 60_000));
        assert_eq!(fake.opens.get(), 1, "the stop cue reuses the open stream");
        fake.consume(130 + 3_000);
        player.step(None, at(base, 63_130));
        assert_eq!(fake.closes.get(), 1, "closed 3 s after the stop cue");
    }

    #[test]
    fn a_recording_that_never_ends_lets_the_device_go() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Start, base), base);
        let hold = max_recording_hold().as_millis() as u64;
        fake.consume(5_000);
        player.step(None, at(base, hold - 1));
        assert_eq!(fake.closes.get(), 0);
        player.step(None, at(base, hold + 1));
        assert_eq!(
            fake.closes.get(),
            1,
            "held at most for the longest recording"
        );
    }

    #[test]
    fn a_callback_that_stops_counting_still_closes_by_the_clock() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Stop, base), base);
        // The device took the cue, then its callback stopped running.
        fake.consume(130);
        player.step(None, at(base, 4_000));
        assert_eq!(fake.closes.get(), 0, "within the slack");
        player.step(None, at(base, 130 + 5_000));
        assert_eq!(fake.closes.get(), 1, "closed by the wall clock");
    }

    #[test]
    fn a_stream_that_stops_taking_samples_is_replaced() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Start, base), base);
        // The callback never runs, even during the recording.
        player.step(None, at(base, 1_000));
        assert_eq!(fake.closes.get(), 0);
        player.step(None, at(base, 120 + 2_000));
        assert_eq!(fake.closes.get(), 1, "a stalled stream is closed");

        let now = at(base, 5_000);
        player.step(cue(Cue::Stop, now), now);
        assert_eq!(fake.opens.get(), 2, "the stop cue gets a fresh stream");
    }

    #[test]
    fn a_stream_error_when_idle_closes_it_and_the_next_cue_reopens() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Stop, base), base);
        fake.failed.borrow()[0].store(true, Ordering::SeqCst);
        player.step(None, at(base, 250));
        assert_eq!(fake.closes.get(), 1);
        assert_eq!(player.wait(), None, "no recording: no reopen");
        player.step(cue(Cue::Start, at(base, 400)), at(base, 400));
        assert_eq!(fake.opens.get(), 2);
    }

    #[test]
    fn a_stream_error_during_a_recording_reopens_once() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Start, base), base);
        fake.failed.borrow()[0].store(true, Ordering::SeqCst);
        player.step(None, at(base, 250));
        assert_eq!(fake.closes.get(), 1);
        player.step(None, at(base, 250 + 400));
        assert_eq!(fake.opens.get(), 1, "waits out the back-off");
        player.step(None, at(base, 250 + 500));
        assert_eq!(fake.opens.get(), 2, "reopened for the stop cue");

        // The reopened stream fails too: no second reopen.
        fake.failed.borrow()[1].store(true, Ordering::SeqCst);
        player.step(None, at(base, 1_000));
        assert_eq!(fake.closes.get(), 2);
        player.step(None, at(base, 5_000));
        assert_eq!(fake.opens.get(), 2, "once, not in a loop");
        assert_eq!(player.wait(), None);
    }

    #[test]
    fn the_reopened_stream_carries_the_stop_cue() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Start, base), base);
        fake.failed.borrow()[0].store(true, Ordering::SeqCst);
        player.step(None, at(base, 250));
        player.step(None, at(base, 750));
        player.step(cue(Cue::Stop, at(base, 20_000)), at(base, 20_000));
        assert_eq!(fake.opens.get(), 2, "no fresh open for the stop cue");
        assert!(lock(&fake.mixers.borrow()[1]).has_pending());
    }

    #[test]
    fn a_failed_recording_stops_holding_the_stream_open() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Start, base), base);
        fake.consume(10_000);
        player.step(None, at(base, 10_000));
        assert_eq!(fake.closes.get(), 0, "held open for the recording");

        let ended = Request {
            event: Event::RecordingEnded,
            queued_at: at(base, 10_000),
        };
        player.step(Some(ended), at(base, 10_000));
        let mixer = fake.mixers.borrow()[0].clone();
        assert!(!lock(&mixer).has_pending(), "the notice plays nothing");
        player.step(None, at(base, 10_250));
        assert_eq!(fake.closes.get(), 1, "let go like after a stop cue");
    }

    #[test]
    fn timers_start_after_a_slow_open() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        // Opening takes 900 ms (a Bluetooth profile switch, say).
        fake.open_finishes_at.set(Some(at(base, 900)));
        player.step(cue(Cue::Stop, base), base);
        let end = player.last_cue_end.expect("a cue is playing");
        let expected = at(base, 900 + 130);
        assert!(
            end.max(expected) - end.min(expected) < Duration::from_millis(1),
            "the cue ends 130 ms after the open finished"
        );
        // The callback has not started yet: the stall window runs from the
        // end of the cue as played, not from before the open.
        player.step(None, at(base, 900 + 130 + 1_999));
        assert_eq!(fake.closes.get(), 0);
        player.step(None, at(base, 900 + 130 + 2_000));
        assert_eq!(fake.closes.get(), 1);

        fake.open_finishes_at.set(Some(at(base, 6_000)));
        player.step(cue(Cue::Start, at(base, 5_500)), at(base, 5_500));
        assert_eq!(player.recording_since, Some(at(base, 6_000)));
    }

    #[test]
    fn housekeeping_leaves_the_lock_alone_while_a_cue_sounds() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Stop, base), base);
        let mixer = fake.mixers.borrow()[0].clone();
        // Hold the mixer lock, as the output callback would mid-buffer: a
        // wake-up during the cue must not wait for it.
        let held = lock(&mixer);
        fake.failed.borrow()[0].store(true, Ordering::SeqCst);
        player.step(None, at(base, 100));
        assert_eq!(fake.closes.get(), 0, "no checks until the cue has ended");
        drop(held);
        player.step(None, at(base, 130));
        assert_eq!(fake.closes.get(), 1, "checked once it ended");
    }

    #[test]
    fn finished_cues_are_freed_by_the_player_not_the_callback() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        player.step(cue(Cue::Stop, base), base);
        fake.consume(200);
        let mixer = fake.mixers.borrow()[0].clone();
        assert_eq!(lock(&mixer).voices.len(), 1, "left for the player");
        player.step(None, at(base, 250));
        assert!(lock(&mixer).voices.is_empty(), "freed on the player thread");
    }

    #[test]
    fn nothing_opens_when_sounds_are_off_stale_or_there_is_no_device() {
        let fake = Fake::new();
        let mut player = fake.player();
        let base = Instant::now();
        fake.enabled.set(Some(false));
        player.step(cue(Cue::Start, base), base);
        fake.enabled.set(None);
        player.step(cue(Cue::Start, base), base);
        fake.enabled.set(Some(true));
        player.step(cue(Cue::Start, base), at(base, 1_001));
        assert_eq!(fake.opens.get(), 0);

        fake.open_fails.set(true);
        player.step(cue(Cue::Start, base), base);
        assert_eq!(player.wait(), None, "no device: nothing open");
    }

    #[test]
    fn a_played_cue_resets_the_player_start_budget() {
        let fake = Fake::new();
        let mut player = fake.player();
        player.unproven_starts.store(3, Ordering::SeqCst);
        let base = Instant::now();
        player.step(cue(Cue::Start, base), base);
        assert_eq!(player.unproven_starts.load(Ordering::SeqCst), 0);
    }

    // --- Handing cues over and replacing players -------------------------

    /// Starts fake players: a channel and a busy clock, no thread.
    #[derive(Default)]
    struct FakeStarts {
        receivers: Vec<Receiver<Request>>,
        busy: Vec<Arc<AtomicU64>>,
        unproven: Vec<Arc<AtomicUsize>>,
    }

    impl FakeStarts {
        fn start(&mut self, unproven: Arc<AtomicUsize>) -> Option<PlayerHandle> {
            let (sender, receiver) = bounded(QUEUE_CAPACITY);
            let busy_since = Arc::new(AtomicU64::new(0));
            self.receivers.push(receiver);
            self.busy.push(busy_since.clone());
            self.unproven.push(unproven);
            Some(PlayerHandle { sender, busy_since })
        }

        /// Kills every started player (their threads "panicked").
        fn kill_all(&mut self) {
            self.receivers.clear();
        }
    }

    fn request(cue: Cue) -> Request {
        Request {
            event: Event::Cue(cue),
            queued_at: Instant::now(),
        }
    }

    /// Spends the start budget: every player dies before playing a cue.
    fn spend_the_budget(dispatcher: &mut Dispatcher, starts: &mut FakeStarts) {
        for round in 0..MAX_UNPROVEN_STARTS {
            assert_eq!(
                send(dispatcher, starts, Cue::Start, 1),
                Dispatched::Queued,
                "start {round}"
            );
            starts.kill_all();
        }
    }

    fn send(
        dispatcher: &mut Dispatcher,
        starts: &mut FakeStarts,
        cue: Cue,
        now_ms: u64,
    ) -> Dispatched {
        dispatcher.dispatch(request(cue), now_ms, &mut |unproven| starts.start(unproven))
    }

    #[test]
    fn a_stuck_player_is_abandoned_and_its_cue_handed_over() {
        let mut dispatcher = Dispatcher::new();
        let mut starts = FakeStarts::default();
        assert_eq!(
            send(&mut dispatcher, &mut starts, Cue::Start, 1),
            Dispatched::Queued
        );
        assert_eq!(starts.receivers.len(), 1);

        // The first player has been inside a device call for 15 s.
        starts.busy[0].store(1_000, Ordering::SeqCst);
        let now = 1_000 + STUCK_AFTER.as_millis() as u64;
        assert_eq!(
            send(&mut dispatcher, &mut starts, Cue::Stop, now),
            Dispatched::Queued
        );
        assert_eq!(starts.receivers.len(), 2, "a new player was started");
        let handed = starts.receivers[1]
            .try_recv()
            .expect("the cue was handed over");
        assert_eq!(handed.event, Event::Cue(Cue::Stop));
        assert!(starts.receivers[0].try_recv().is_ok(), "only the first cue");
        assert!(starts.receivers[0].try_recv().is_err());
    }

    #[test]
    fn a_slow_device_call_is_not_mistaken_for_a_stuck_player() {
        let mut dispatcher = Dispatcher::new();
        let mut starts = FakeStarts::default();
        send(&mut dispatcher, &mut starts, Cue::Start, 1);
        // A Bluetooth profile switch: 8 s inside the open.
        starts.busy[0].store(1_000, Ordering::SeqCst);
        send(&mut dispatcher, &mut starts, Cue::Stop, 9_000);
        assert_eq!(starts.receivers.len(), 1, "the same player keeps the cue");
    }

    #[test]
    fn a_full_queue_drops_the_cue_instead_of_growing() {
        let mut dispatcher = Dispatcher::new();
        let mut starts = FakeStarts::default();
        for _ in 0..QUEUE_CAPACITY {
            assert_eq!(
                send(&mut dispatcher, &mut starts, Cue::Start, 1),
                Dispatched::Queued
            );
        }
        assert_eq!(
            send(&mut dispatcher, &mut starts, Cue::Start, 1),
            Dispatched::QueueFull
        );
        assert_eq!(
            starts.receivers.len(),
            1,
            "no extra player for a full queue"
        );
    }

    #[test]
    fn the_player_start_budget_runs_out_and_recovers_after_a_cue_plays() {
        let mut dispatcher = Dispatcher::new();
        let mut starts = FakeStarts::default();
        spend_the_budget(&mut dispatcher, &mut starts);
        assert_eq!(
            send(&mut dispatcher, &mut starts, Cue::Start, 1),
            Dispatched::NoPlayer,
            "out of budget"
        );
        let cool_off = BUDGET_COOL_OFF.as_millis() as u64;
        assert_eq!(
            send(&mut dispatcher, &mut starts, Cue::Start, cool_off),
            Dispatched::NoPlayer,
            "still cooling off"
        );

        // After the cool-off one more player starts, and it really plays the
        // cue it was handed (on a fake stream).
        assert_eq!(
            send(&mut dispatcher, &mut starts, Cue::Start, 1 + cool_off),
            Dispatched::Queued
        );
        let fake = Fake::new();
        let unproven = starts.unproven.last().unwrap().clone();
        let mut player = fake.player_sharing(unproven);
        let handed = starts.receivers.last().unwrap().try_recv().unwrap();
        player.step(Some(handed), Instant::now());
        assert!(lock(&fake.mixers.borrow()[0]).has_pending(), "it played");

        // The played cue restored the whole budget: four more starts.
        for round in 0..MAX_UNPROVEN_STARTS {
            starts.kill_all();
            assert_eq!(
                send(&mut dispatcher, &mut starts, Cue::Start, 2 + cool_off),
                Dispatched::Queued,
                "start {round} after the recovery"
            );
        }
    }

    #[test]
    fn a_stuck_player_is_kept_when_no_replacement_may_start() {
        let mut dispatcher = Dispatcher::new();
        let mut starts = FakeStarts::default();
        // Three players die; the fourth, the last the budget allows, lives
        // but gets stuck in the driver.
        for _ in 1..MAX_UNPROVEN_STARTS {
            send(&mut dispatcher, &mut starts, Cue::Start, 1);
            starts.kill_all();
        }
        send(&mut dispatcher, &mut starts, Cue::Start, 1);
        let stuck = starts.receivers.len() - 1;
        starts.busy.last().unwrap().store(1, Ordering::SeqCst);
        let late = 1 + STUCK_AFTER.as_millis() as u64;
        assert_eq!(
            send(&mut dispatcher, &mut starts, Cue::Stop, late),
            Dispatched::Queued
        );
        assert_eq!(starts.receivers.len(), stuck + 1, "no new player");
        let queued: Vec<_> = starts.receivers[stuck].try_iter().collect();
        assert_eq!(queued.len(), 2, "the stuck player keeps both cues");
        assert_eq!(queued[1].event, Event::Cue(Cue::Stop));
    }

    #[test]
    fn a_recording_ended_notice_never_starts_a_player() {
        let mut dispatcher = Dispatcher::new();
        let mut starts = FakeStarts::default();
        let notice = Request {
            event: Event::RecordingEnded,
            queued_at: Instant::now(),
        };
        let outcome = dispatcher.dispatch(notice, 1, &mut |unproven| starts.start(unproven));
        assert_eq!(outcome, Dispatched::NoPlayer);
        assert!(starts.receivers.is_empty());

        send(&mut dispatcher, &mut starts, Cue::Start, 1);
        let notice = Request {
            event: Event::RecordingEnded,
            queued_at: Instant::now(),
        };
        let outcome = dispatcher.dispatch(notice, 1, &mut |unproven| starts.start(unproven));
        assert_eq!(outcome, Dispatched::Queued, "a running player gets it");
    }

    /// Without a device this cannot run the app end to end, so it checks
    /// the source instead: each recording event is emitted from exactly one
    /// helper in audio.rs, that helper plays the matching cue, and code (not
    /// only a comment) calls the helper.
    #[test]
    fn recording_events_play_their_cues() {
        // A Windows checkout can have CRLF line endings.
        let audio = include_str!("audio.rs").replace("\r\n", "\n");
        let code: String = audio
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // `audio://recording-started` and `audio://recording-stopped`.
        for (event, helper, cue) in [
            (
                "CoreEvent::RecordingStarted(",
                "emit_recording_started",
                "Cue::Start",
            ),
            (
                "CoreEvent::RecordingStopped(",
                "emit_recording_stopped",
                "Cue::Stop",
            ),
        ] {
            assert_eq!(
                code.matches(event).count(),
                1,
                "{event} is emitted only from {helper}"
            );
            let definition = format!("fn {helper}(");
            let body_start = code.find(&definition).expect("the helper exists");
            let body = &code[body_start..];
            let body = &body[..body.find("\n}").expect("the helper ends")];
            assert!(body.contains(event), "{helper} emits {event}");
            assert!(
                body.contains(&format!(
                    "crate::sounds::play_with(parts.backend, crate::sounds::{cue})"
                )),
                "{helper} plays {cue}"
            );
            let calls = code.matches(&format!("{helper}(")).count() - 1;
            assert!(calls >= 1, "{helper} is called from code");
        }
    }

    /// Plays two overlapping cues through a real output stream, checks the
    /// device consumed every sample of both and then counted silence (what
    /// the close rule reads), then closes the stream. Needs an output device,
    /// and on a real one it is audible, so run it by hand:
    /// `cargo test --lib sounds -- --ignored`.
    #[test]
    #[ignore = "needs an output device"]
    fn cues_play_through_a_real_output_stream() {
        let output = open_default_output().expect("an output stream opens");
        lock(&output.mixer).add(render(Cue::Stop, output.sample_rate));
        thread::sleep(Duration::from_millis(40));
        lock(&output.mixer).add(render(Cue::Start, output.sample_rate));

        let deadline = Instant::now() + Duration::from_secs(3);
        while lock(&output.mixer).has_pending() {
            assert!(
                Instant::now() < deadline,
                "the device stopped taking samples"
            );
            thread::sleep(Duration::from_millis(10));
        }
        while lock(&output.mixer).idle_frames < u64::from(output.sample_rate / 10) {
            assert!(Instant::now() < deadline, "the stream never went idle");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!output.failed.load(Ordering::SeqCst), "no stream error");
        let busy = AtomicU64::new(0);
        assert!(device_call(&busy, move || drop(output)).is_some());
    }
}
