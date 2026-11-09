use std::{fs, path::PathBuf, thread, time::Duration};

use chrono::{DateTime, Utc};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, SampleFormat, Stream, StreamConfig,
};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use crate::{
    app_state::{AppEvent, AppStatus},
    commands::BackendState,
    dictation_state::emit_state_snapshot,
    error::CommandError,
    settings::AppSettings,
};

pub(crate) const TARGET_SAMPLE_RATE: u32 = 16_000;
const TARGET_CHANNELS: u16 = 1;
const TARGET_BITS_PER_SAMPLE: u16 = 16;
const LEVEL_EVENT_INTERVAL: Duration = Duration::from_millis(40);
const DEFAULT_TEST_CLIP_MS: u64 = 3_000;
/// Per-chunk RMS at or above this counts as speech. Silence auto-stop and the
/// incremental segmenter only arm after at least one speech chunk, so a
/// recording that never picks up any voice is not cut short or segmented.
pub(crate) const AUTO_STOP_SPEECH_RMS: f32 = 0.03;
/// RMS below this counts as silence, for the auto-stop countdown, the
/// incremental segmenter, and trimming leading/trailing silence. Sits below
/// typical speech levels but above quiet room noise.
pub(crate) const SILENCE_RMS_THRESHOLD: f32 = 0.015;
/// Audio kept on each side of detected speech when trimming silence, so word
/// onsets/tails are not clipped.
const TRIM_PADDING_MS: u32 = 300;
/// How long capture keeps running after a user-initiated stop. People press
/// stop while the last word is still leaving their mouth (and in WASAPI
/// buffers); cutting the stream at the keypress clips it.
const STOP_GRACE_MS: u64 = 400;
/// RMS analysis window for silence trimming.
const TRIM_WINDOW_MS: u32 = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneInfo {
    pub id: String,
    pub name: String,
    pub endpoint_id: Option<String>,
    pub is_default: bool,
    pub is_selected: bool,
    pub is_available: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRecordingRequest {
    pub microphone_id: Option<String>,
    pub max_duration_ms: Option<u64>,
    /// Note-taking dictation (tilde+Q): saved to history flagged as a note,
    /// never auto-pasted; the pill renders blue while it records.
    #[serde(default)]
    pub is_note: bool,
    /// Selected-text transform: this recording captures a spoken *instruction*.
    /// Its transcribed text is routed to the transform engine (rewrite the
    /// previously-captured selection) instead of being saved/pasted as
    /// dictation. Set only by the Transform Selection hotkey path.
    #[serde(default)]
    pub is_transform: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSessionInfo {
    pub session_id: String,
    pub microphone_id: String,
    pub microphone_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub started_at: DateTime<Utc>,
    /// Hard stop for this recording: the safety ceiling, or a test clip's
    /// length.
    pub max_duration_ms: u64,
    pub is_test_clip: bool,
    pub is_note: bool,
    pub is_transform: bool,
    /// True when the microphone chosen in Settings isn't connected and this
    /// recording fell back to the system default, so the UI can say so.
    pub microphone_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingResultStatus {
    Completed,
    TooShort,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingResult {
    pub session_id: String,
    pub status: RecordingResultStatus,
    pub wav_path: Option<String>,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub bytes_written: Option<u64>,
    pub reason: Option<String>,
    pub started_at: DateTime<Utc>,
    pub stopped_at: DateTime<Utc>,
    #[serde(default)]
    pub is_note: bool,
    #[serde(default)]
    pub is_transform: bool,
    /// True when the recording ended because the mic was disconnected
    /// mid-capture. The salvaged audio is transcribed and saved to History so
    /// nothing is lost, but it is NOT auto-pasted — a mic dying on dead air
    /// otherwise pastes Whisper's silence hallucinations into the focused app.
    #[serde(default)]
    pub disconnected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioLevelEvent {
    session_id: String,
    level: f32,
    peak: f32,
    rms: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingErrorEvent {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    Completed,
    Cancelled,
    Timeout,
    /// The capture stream errored mid-recording (e.g. the mic was unplugged or
    /// disabled). Finalized like a normal completion — whatever was captured
    /// before the drop is still transcribed — but with no stop-grace sleep
    /// (the device is gone) so it tears down immediately.
    Disconnected,
}

#[derive(Debug)]
struct AudioChunk {
    samples: Vec<f32>,
}

#[derive(Debug)]
enum WorkerControl {
    Stop(StopReason),
}

pub struct AudioService {
    temp_dir: PathBuf,
    active: Option<RecordingSession>,
    last_result: Option<RecordingResult>,
    last_test_clip: Option<PathBuf>,
}

struct RecordingSession {
    info: RecordingSessionInfo,
    stream: CaptureStream,
    control_tx: Sender<WorkerControl>,
    worker: thread::JoinHandle<Result<RecordingResult, CommandError>>,
    timeout_active: Arc<AtomicBool>,
}

#[derive(Debug)]
struct StartOutcome {
    info: RecordingSessionInfo,
    started: bool,
}

struct DeviceCandidate {
    info: MicrophoneInfo,
    device: Device,
}

impl AudioService {
    pub fn new(temp_dir: PathBuf) -> Self {
        Self {
            temp_dir,
            active: None,
            last_result: None,
            last_test_clip: None,
        }
    }

    fn set_last_test_clip(&mut self, path: PathBuf) {
        self.last_test_clip = Some(path);
    }

    fn last_test_clip_path(&self) -> Option<PathBuf> {
        self.last_test_clip.clone()
    }

    fn list_microphones(
        &self,
        selected_mic_id: Option<&str>,
    ) -> Result<Vec<MicrophoneInfo>, CommandError> {
        Ok(device_candidates(selected_mic_id)?
            .into_iter()
            .map(|candidate| candidate.info)
            .collect())
    }

    fn start(
        &mut self,
        app: &AppHandle,
        settings: &AppSettings,
        request: StartRecordingRequest,
        is_test_clip: bool,
        allow_auto_stop: bool,
    ) -> Result<StartOutcome, CommandError> {
        if let Some(active) = &self.active {
            return Ok(StartOutcome {
                info: active.info.clone(),
                started: false,
            });
        }

        fs::create_dir_all(&self.temp_dir).map_err(|error| {
            CommandError::new(
                "recording_failed",
                format!("Could not create audio temp directory. {}", error),
            )
        })?;

        let selected_mic_id = request
            .microphone_id
            .as_deref()
            .or(settings.selected_mic_id.as_deref());
        let input = open_input(selected_mic_id)?;
        let microphone_fallback = input.fallback;
        if microphone_fallback {
            log::warn!(
                "Selected microphone {:?} is not connected; recording with {} instead",
                selected_mic_id,
                input.info.name
            );
        }
        let source_sample_rate = input.sample_rate;
        let source_channels = input.channels;
        let session_id = Uuid::new_v4().to_string();
        let started_at = Utc::now();
        let limits = StopLimits::for_recording(
            settings,
            request.max_duration_ms,
            is_test_clip,
            allow_auto_stop,
        );
        let max_duration_ms = limits.ceiling_ms;
        let info = RecordingSessionInfo {
            session_id: session_id.clone(),
            microphone_id: input.info.id.clone(),
            microphone_name: input.info.name.clone(),
            sample_rate: source_sample_rate,
            channels: source_channels,
            started_at,
            max_duration_ms,
            is_test_clip,
            is_note: request.is_note && !is_test_clip,
            is_transform: request.is_transform && !is_test_clip,
            microphone_fallback,
        };
        let (chunk_tx, chunk_rx) = unbounded::<AudioChunk>();
        let (control_tx, control_rx) = unbounded::<WorkerControl>();
        let app_for_worker = app.clone();
        let temp_dir = self.temp_dir.clone();
        let min_recording_ms = settings.min_recording_ms as u64;
        let silence_trim_enabled = settings.silence_trim_enabled;
        // Incremental transcription applies to dictation recordings only —
        // never test clips — and only when the setting is enabled. When the
        // coordinator cannot start this is None and behavior is unchanged.
        let incremental = if !is_test_clip && settings.incremental_transcription_enabled {
            // Pause-based cutting can be turned off (then segments end only at
            // the length cap); the cap is clamped to a Whisper-safe range by
            // settings validation.
            let segment_pause_ms = settings
                .segment_pause_enabled
                .then_some(settings.segment_pause_ms as u64);
            let segment_max_ms = settings.segment_max_ms as u64;
            crate::incremental::start_session(
                app,
                &session_id,
                self.temp_dir.clone(),
                source_sample_rate,
                segment_pause_ms,
                segment_max_ms,
            )
        } else {
            None
        };
        let worker_info = info.clone();
        let worker = thread::spawn(move || {
            recording_worker(
                app_for_worker,
                worker_info,
                chunk_rx,
                control_rx,
                temp_dir,
                source_sample_rate,
                min_recording_ms,
                silence_trim_enabled,
                limits,
                incremental,
            )
        });

        let stream = CaptureStream::start(input.source, chunk_tx, app.clone(), session_id.clone())?;

        let timeout_active = Arc::new(AtomicBool::new(true));
        spawn_timeout_thread(
            app.clone(),
            session_id.clone(),
            max_duration_ms,
            timeout_active.clone(),
        );

        let session = RecordingSession {
            info: info.clone(),
            stream,
            control_tx,
            worker,
            timeout_active,
        };
        self.active = Some(session);
        self.last_result = None;

        log::info!(
            "Recording started: session {} (mic '{}', {} Hz, test clip: {}, limits: {:?})",
            info.session_id,
            info.microphone_name,
            info.sample_rate,
            is_test_clip,
            limits
        );
        let _ = app.emit("audio://recording-started", &info);
        Ok(StartOutcome {
            info,
            started: true,
        })
    }

    fn stop(&mut self, reason: StopReason) -> Result<Option<RecordingResult>, CommandError> {
        let Some(session) = self.active.take() else {
            return Ok(self.last_result.clone());
        };

        session.timeout_active.store(false, Ordering::SeqCst);
        // Completed means the user chose to stop (toggle press / hold
        // release): keep capturing briefly so the tail of the final word is
        // not cut off at the keypress. Cancel and timeout tear down at once,
        // and test clips must stay exactly as long as requested.
        if reason == StopReason::Completed && !session.info.is_test_clip {
            thread::sleep(Duration::from_millis(STOP_GRACE_MS));
        }
        drop(session.stream);
        let _ = session.control_tx.send(WorkerControl::Stop(reason));
        let result = session.worker.join().map_err(|_| {
            CommandError::new("recording_failed", "Audio worker thread panicked.")
        })??;
        self.last_result = Some(result.clone());
        Ok(Some(result))
    }

    fn stop_session(
        &mut self,
        session_id: &str,
        reason: StopReason,
    ) -> Result<Option<RecordingResult>, CommandError> {
        if self
            .active
            .as_ref()
            .map(|session| session.info.session_id.as_str() != session_id)
            .unwrap_or(true)
        {
            return Ok(None);
        }

        self.stop(reason)
    }

    fn active_session_id(&self) -> Option<&str> {
        self.active
            .as_ref()
            .map(|session| session.info.session_id.as_str())
    }
}

pub fn list_microphones_for_app(app: &AppHandle) -> Result<Vec<MicrophoneInfo>, CommandError> {
    let state = app.state::<BackendState>();
    let settings = state.db()?.get_settings()?;
    let microphones = state
        .audio()?
        .list_microphones(settings.selected_mic_id.as_deref())?;
    Ok(microphones)
}

/// `allow_auto_stop` marks the recording as eligible for silence auto-stop
/// (toggle hotkey / tray menu / UI Start). Hold-to-talk passes false.
pub fn start_recording_for_app(
    app: &AppHandle,
    request: Option<StartRecordingRequest>,
    allow_auto_stop: bool,
) -> Result<RecordingSessionInfo, CommandError> {
    let state = app.state::<BackendState>();
    let settings = state.db()?.get_settings()?;
    let snapshot = state.app_state()?.snapshot();

    if matches!(
        snapshot.status,
        AppStatus::Idle | AppStatus::Ready | AppStatus::Error
    ) {
        if snapshot.status == AppStatus::Error {
            let snapshot = state.transition_app_state(AppEvent::ResetError)?;
            emit_state_snapshot(app, &snapshot);
        }

        let outcome = state.audio()?.start(
            app,
            &settings,
            request.unwrap_or_default(),
            false,
            allow_auto_stop,
        )?;

        if outcome.started {
            // Remember where the user is dictating, so the text goes back
            // there even if they switch apps while it transcribes.
            state.set_paste_target(crate::output::current_paste_target());
            let snapshot = state.transition_app_state(AppEvent::StartRecording)?;
            emit_state_snapshot(app, &snapshot);
            // Load the speech model while the person speaks, if it isn't
            // loaded already ("Keep the speech model loaded" off, or a crash).
            if let Some(warm) = app.try_state::<crate::whisper_server::WarmTranscriber>() {
                warm.prewarm();
            }
        }

        Ok(outcome.info)
    } else if snapshot.status == AppStatus::Recording {
        let outcome = state.audio()?.start(
            app,
            &settings,
            request.unwrap_or_default(),
            false,
            allow_auto_stop,
        )?;
        Ok(outcome.info)
    } else {
        Err(CommandError::new(
            "recording_unavailable",
            format!("Cannot start recording while app is {:?}.", snapshot.status),
        ))
    }
}

pub fn stop_recording_for_app(app: &AppHandle) -> Result<RecordingResult, CommandError> {
    stop_recording_with_reason(app, StopReason::Completed, None)
}

pub fn cancel_recording_for_app(app: &AppHandle) -> Result<(), CommandError> {
    let state = app.state::<BackendState>();
    let result = state.audio()?.stop(StopReason::Cancelled)?;

    if let Some(result) = result {
        log::info!(
            "Recording cancelled: session {} after {} ms",
            result.session_id,
            result.duration_ms
        );
        // No transcription will follow: drop any incremental session, and give
        // a transform recording's captured selection back to the clipboard.
        state.incremental().discard(&result.session_id);
        if result.is_transform {
            state.discard_pending_transform();
        }
        let _ = app.emit("audio://recording-stopped", &result);
    }

    let snapshot = state.app_state()?.snapshot();
    if snapshot.status == AppStatus::Recording {
        let snapshot = state.transition_app_state(AppEvent::CancelRecording)?;
        emit_state_snapshot(app, &snapshot);
    }

    Ok(())
}

pub fn record_test_clip_for_app(
    app: &AppHandle,
    duration_ms: Option<u64>,
) -> Result<RecordingResult, CommandError> {
    let state = app.state::<BackendState>();
    let settings = state.db()?.get_settings()?;
    let duration_ms = duration_ms.unwrap_or(DEFAULT_TEST_CLIP_MS).clamp(
        settings.min_recording_ms as u64,
        settings.max_recording_ms as u64,
    );
    let request = StartRecordingRequest {
        microphone_id: None,
        max_duration_ms: Some(duration_ms.saturating_add(1_000)),
        is_note: false,
        is_transform: false,
    };
    // Test clips are never silence auto-stopped: they run for a fixed
    // duration and must capture whatever the mic hears.
    let outcome = state.audio()?.start(app, &settings, request, true, false)?;

    if !outcome.started {
        return Err(CommandError::new(
            "recording_already_active",
            "A recording is already active. Stop or cancel it before recording a test clip.",
        ));
    }

    thread::sleep(Duration::from_millis(duration_ms));
    let result = state
        .audio()?
        .stop_session(&outcome.info.session_id, StopReason::Completed)?
        .ok_or_else(|| {
            CommandError::new("recording_failed", "Test recording ended unexpectedly.")
        })?;

    if let Some(wav_path) = result.wav_path.as_deref() {
        state.audio()?.set_last_test_clip(PathBuf::from(wav_path));
    }

    let _ = app.emit("audio://recording-stopped", &result);
    Ok(result)
}

/// Returns the most recent test clip WAV as a base64 string for in-app
/// playback. Errors with code "no_test_clip" when no clip is available.
pub fn get_test_clip_audio_for_app(app: &AppHandle) -> Result<String, CommandError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let state = app.state::<BackendState>();
    let path = state
        .audio()?
        .last_test_clip_path()
        .ok_or_else(no_test_clip)?;
    let bytes = fs::read(&path).map_err(|_| no_test_clip())?;
    Ok(STANDARD.encode(bytes))
}

fn no_test_clip() -> CommandError {
    CommandError::new(
        "no_test_clip",
        "No test clip is available yet. Record a test clip first.",
    )
}

/// Stops and transcribes a recording that ended on its own: the safety
/// ceiling, or the silence rule past the long-recording point (see
/// [`StopPolicy`]). Runs on the dictation worker; a no-op when `session_id` is
/// no longer the active recording (the user stopped it while this was
/// queued), so the transcript is never lost or duplicated.
pub(crate) fn finish_timed_out_recording(
    app: &AppHandle,
    session_id: &str,
) -> Result<(), CommandError> {
    let still_active = app
        .state::<BackendState>()
        .audio()?
        .active_session_id()
        .is_some_and(|active| active == session_id);
    if !still_active {
        return Ok(());
    }

    let result = stop_recording_with_reason(app, StopReason::Timeout, Some(session_id))?;
    let _ = app.emit("audio://recording-timeout", &result);

    // Stopping moved the state machine into Transcribing: a timed-out
    // recording must be transcribed exactly like a manual stop, or the app
    // stays stuck on Transcribing forever.
    if result.status == RecordingResultStatus::TooShort {
        crate::dictation::emit_dictation_empty(app);
    } else {
        crate::dictation::transcribe_recording_for_app(app, result)?;
    }
    Ok(())
}

/// Recovers from a capture-stream error (e.g. the mic was unplugged mid-record).
/// Runs on the dictation worker, queued by the cpal error callback. Stops the
/// in-flight session like a normal completion — so whatever was captured before
/// the drop is still transcribed — surfaces a `microphone_unavailable`
/// recording error, then finishes transcription exactly like the manual-stop
/// and timeout paths so the app never strands in Recording/Transcribing. If the
/// session is already gone (the user stopped first, or a duplicate error
/// callback slipped past the guard), it no-ops cleanly.
pub(crate) fn finish_disconnected_recording(
    app: &AppHandle,
    session_id: &str,
) -> Result<(), CommandError> {
    let result = match stop_recording_with_reason(app, StopReason::Disconnected, Some(session_id)) {
        Ok(result) => result,
        Err(error) => {
            log::info!(
                "Audio stream error for session {}, but it was no longer the active recording: {}",
                session_id,
                error.message
            );
            return Ok(());
        }
    };

    emit_recording_error(
        app,
        CommandError::new(
            "microphone_unavailable",
            "The microphone stopped sending audio (it may have been unplugged or disabled). \
             Anything captured was saved to your History (not pasted). Reconnect the mic and record again.",
        ),
    );

    // Stopping moved the state machine into Transcribing (valid audio) or Idle
    // (too short). Finish the same way the manual-stop / timeout paths do, or
    // the app stays stuck in Transcribing forever.
    if result.status == RecordingResultStatus::TooShort {
        crate::dictation::emit_dictation_empty(app);
    } else {
        crate::dictation::transcribe_recording_for_app(app, result)?;
    }
    Ok(())
}

fn stop_recording_with_reason(
    app: &AppHandle,
    reason: StopReason,
    session_id: Option<&str>,
) -> Result<RecordingResult, CommandError> {
    let state = app.state::<BackendState>();
    let should_transition = state.app_state()?.snapshot().status == AppStatus::Recording;

    if should_transition {
        // Three callers can stop a session — a manual/auto stop, the
        // max-duration timeout thread, and the mic-disconnect handler — and the
        // state lock is released between the snapshot read above and here. If a
        // racing stopper already advanced the FSM past Recording, StopRecording
        // is no longer a legal edge; treat that as a benign no-op instead of
        // propagating an "invalid state transition" error, which the timeout
        // path would otherwise surface to the user as a confusing toast. The
        // stopper that actually takes the session still drives the rest.
        match state.transition_app_state(AppEvent::StopRecording) {
            Ok(snapshot) => emit_state_snapshot(app, &snapshot),
            Err(error) => log::debug!(
                "StopRecording transition skipped; another stopper already advanced the state: {}",
                error.message
            ),
        }
    }

    let no_active_session =
        || CommandError::new("recording_not_active", "No active recording session.");
    let stop_result = if let Some(session_id) = session_id {
        state
            .audio()?
            .stop_session(session_id, reason)
            .and_then(|result| result.ok_or_else(no_active_session))
    } else {
        state
            .audio()?
            .stop(reason)
            .and_then(|result| result.ok_or_else(no_active_session))
    };
    let result = match stop_result {
        Ok(result) => result,
        Err(error) => {
            // The recording is gone either way: never strand the state
            // machine in Stopping (where the toggle hotkey would be a
            // no-op). Land back on Idle and surface the error to the caller.
            if state.app_state()?.snapshot().status == AppStatus::Stopping {
                let snapshot = state.transition_app_state(AppEvent::StopFailed)?;
                emit_state_snapshot(app, &snapshot);
            }
            return Err(error);
        }
    };

    log::info!(
        "Recording stopped: session {} ({:?}, {:?}, {} ms)",
        result.session_id,
        reason,
        result.status,
        result.duration_ms
    );
    if !matches!(
        result.status,
        RecordingResultStatus::Completed | RecordingResultStatus::TimedOut
    ) {
        // No transcription will follow: drop any incremental session, and give
        // a transform recording's captured selection back to the clipboard.
        state.incremental().discard(&result.session_id);
        if result.is_transform {
            state.discard_pending_transform();
        }
    }
    let _ = app.emit("audio://recording-stopped", &result);

    let snapshot = state.app_state()?.snapshot();
    if snapshot.status == AppStatus::Stopping {
        let event = if result.status == RecordingResultStatus::TooShort {
            AppEvent::AudioTooShort
        } else {
            AppEvent::ValidAudio
        };
        let snapshot = state.transition_app_state(event)?;
        emit_state_snapshot(app, &snapshot);
    }

    Ok(result)
}

/// Wall-clock backstop for the safety ceiling. [`StopPolicy`] stops at the
/// ceiling by counting captured audio; this timer still fires if the stream
/// stops delivering audio without reporting an error.
fn spawn_timeout_thread(
    app: AppHandle,
    session_id: String,
    max_duration_ms: u64,
    timeout_active: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(max_duration_ms));
        if !timeout_active.load(Ordering::SeqCst) {
            return;
        }

        // The worker re-checks that this session is still the active one
        // when the stop runs.
        crate::dictation_worker::dispatch(
            &app,
            crate::dictation_worker::DictationCommand::FinishTimedOut { session_id },
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn recording_worker(
    app: AppHandle,
    info: RecordingSessionInfo,
    chunk_rx: Receiver<AudioChunk>,
    control_rx: Receiver<WorkerControl>,
    temp_dir: PathBuf,
    source_sample_rate: u32,
    min_recording_ms: u64,
    silence_trim_enabled: bool,
    limits: StopLimits,
    mut incremental: Option<crate::incremental::WorkerLink>,
) -> Result<RecordingResult, CommandError> {
    let mut samples = Vec::<f32>::new();
    let mut last_level_emit = Instant::now()
        .checked_sub(LEVEL_EVENT_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut stop_policy = StopPolicy::new(limits, source_sample_rate);
    let stop_reason = loop {
        crossbeam_channel::select! {
            recv(control_rx) -> message => {
                match message {
                    Ok(WorkerControl::Stop(reason)) => break reason,
                    Err(_) => break StopReason::Completed,
                }
            }
            recv(chunk_rx) -> message => {
                match message {
                    Ok(chunk) => {
                        maybe_emit_level(&app, &info.session_id, &chunk.samples, &mut last_level_emit);
                        if let Some(signal) = stop_policy.observe(&chunk.samples) {
                            act_on_stop_signal(&app, &info.session_id, signal, limits);
                        }
                        if let Some(link) = incremental.as_mut() {
                            link.push_chunk(&chunk.samples);
                        }
                        samples.extend(chunk.samples);
                    }
                    Err(_) => break stop_reason_after_stream_end(&control_rx),
                }
            }
        }
    };

    while let Ok(chunk) = chunk_rx.try_recv() {
        maybe_emit_level(&app, &info.session_id, &chunk.samples, &mut last_level_emit);
        if let Some(link) = incremental.as_mut() {
            link.push_chunk(&chunk.samples);
        }
        samples.extend(chunk.samples);
    }

    if let Some(link) = incremental.take() {
        let transcribable = stop_reason != StopReason::Cancelled
            && duration_ms(samples.len(), source_sample_rate) >= min_recording_ms;
        if transcribable {
            // Flush the tail phrase and let the coordinator assemble the
            // final text while the full WAV is written below.
            link.finish();
        } else {
            link.cancel();
        }
    }

    finalize_recording(
        info,
        stop_reason,
        samples,
        temp_dir,
        source_sample_rate,
        min_recording_ms,
        silence_trim_enabled,
    )
}

/// Why the recording ended, once the audio stream is gone. `stop` drops the
/// stream before it sends the reason, so the chunk channel closes first; the
/// reason (Cancelled, Timeout, Disconnected) still decides how the clip is
/// finalized. Waits for it, and counts a stop that never comes (the session
/// was dropped) as Completed.
fn stop_reason_after_stream_end(control_rx: &Receiver<WorkerControl>) -> StopReason {
    match control_rx.recv() {
        Ok(WorkerControl::Stop(reason)) => reason,
        Err(_) => StopReason::Completed,
    }
}

fn maybe_emit_level(
    app: &AppHandle,
    session_id: &str,
    samples: &[f32],
    last_level_emit: &mut Instant,
) {
    if samples.is_empty() || last_level_emit.elapsed() < LEVEL_EVENT_INTERVAL {
        return;
    }

    let peak = samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    let rms = chunk_rms(samples);
    let event = AudioLevelEvent {
        session_id: session_id.to_string(),
        level: rms.clamp(0.0, 1.0),
        peak: peak.clamp(0.0, 1.0),
        rms: rms.clamp(0.0, 1.0),
    };
    let _ = app.emit("audio://level", event);
    *last_level_emit = Instant::now();
}

/// How a recording's length and silence end it. Built once per session from
/// the settings and the kind of start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StopLimits {
    /// Continuous silence that ends the recording (`silence_auto_stop_ms`),
    /// or `None` when silence auto-stop is off.
    silence_ms: Option<u64>,
    /// Whether the silence rule applies from the start (toggle-style starts:
    /// toggle hotkey, tray, UI Start). Hold-to-talk passes false: the user is
    /// still holding the key, so a pause must not cut them off.
    silence_from_start: bool,
    /// The long-recording point (`max_recording_ms`). It never cuts anyone
    /// off; past it the silence rule also covers hold-to-talk, and no longer
    /// needs speech first, so a stuck key or a forgotten open mic still ends.
    long_recording_ms: u64,
    /// Safety ceiling (`recording_ceiling_ms`): stops even mid-sentence.
    ceiling_ms: u64,
}

impl StopLimits {
    fn for_recording(
        settings: &AppSettings,
        request_max_ms: Option<u64>,
        is_test_clip: bool,
        allow_auto_stop: bool,
    ) -> Self {
        let min_ms = settings.min_recording_ms as u64;
        if is_test_clip {
            // Test clips run for a fixed length and must capture whatever the
            // mic hears: no silence rule, and the requested length is a hard
            // stop.
            let ceiling_ms = request_max_ms
                .unwrap_or(settings.recording_ceiling_ms as u64)
                .max(min_ms);
            return Self {
                silence_ms: None,
                silence_from_start: false,
                long_recording_ms: ceiling_ms,
                ceiling_ms,
            };
        }

        // A caller-chosen maximum is a hard stop, like the ceiling.
        let ceiling_ms = request_max_ms
            .unwrap_or(settings.recording_ceiling_ms as u64)
            .max(min_ms);
        Self {
            silence_ms: settings
                .silence_auto_stop_enabled
                .then_some(settings.silence_auto_stop_ms as u64),
            silence_from_start: allow_auto_stop,
            long_recording_ms: settings.max_recording_ms as u64,
            ceiling_ms,
        }
    }
}

/// Why [`StopPolicy`] wants the recording to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopSignal {
    /// The normal silence auto-stop of a toggle-style start: speech was heard,
    /// then `silence_ms` of silence. Stops exactly like a toggle press.
    Silence,
    /// Past the long-recording point, `silence_ms` of silence (any start, with
    /// or without speech). Stops through the timed-out salvage path.
    LongRecordingSilence,
    /// The safety ceiling. Stops through the timed-out salvage path, even
    /// mid-sentence.
    Ceiling,
}

/// Watches the captured audio and decides when a recording should end on its
/// own. A recording is never cut off at the long-recording point while the
/// user is talking: it ends when they stop it, after the silence rule, or at
/// the ceiling. Fires at most once per session. Pure (no app handle) so the
/// rules can be unit tested; the recording worker performs the stop.
struct StopPolicy {
    limits: StopLimits,
    sample_rate: u32,
    elapsed_samples: usize,
    speech_detected: bool,
    silence_samples: usize,
    fired: bool,
}

impl StopPolicy {
    fn new(limits: StopLimits, sample_rate: u32) -> Self {
        Self {
            limits,
            sample_rate,
            elapsed_samples: 0,
            speech_detected: false,
            silence_samples: 0,
            fired: false,
        }
    }

    fn observe(&mut self, samples: &[f32]) -> Option<StopSignal> {
        if self.fired || samples.is_empty() {
            return None;
        }

        self.elapsed_samples += samples.len();
        let rms = chunk_rms(samples);
        if rms >= AUTO_STOP_SPEECH_RMS {
            self.speech_detected = true;
            self.silence_samples = 0;
        } else if rms >= SILENCE_RMS_THRESHOLD {
            // Not loud enough to count as speech, but not silent either:
            // the continuous-silence run is broken.
            self.silence_samples = 0;
        } else {
            self.silence_samples += samples.len();
        }

        let signal = self.decide();
        self.fired = signal.is_some();
        signal
    }

    fn decide(&self) -> Option<StopSignal> {
        let elapsed_ms = duration_ms(self.elapsed_samples, self.sample_rate);
        if elapsed_ms >= self.limits.ceiling_ms {
            return Some(StopSignal::Ceiling);
        }

        let silence_ms = self.limits.silence_ms?;
        if duration_ms(self.silence_samples, self.sample_rate) < silence_ms {
            return None;
        }
        if self.limits.silence_from_start && self.speech_detected {
            return Some(StopSignal::Silence);
        }
        if elapsed_ms >= self.limits.long_recording_ms {
            return Some(StopSignal::LongRecordingSilence);
        }
        None
    }
}

/// Carries out a [`StopSignal`] from the recording worker. Every stop is
/// queued on the dictation worker: stopping joins this recording thread, so
/// it must not run on it.
fn act_on_stop_signal(app: &AppHandle, session_id: &str, signal: StopSignal, limits: StopLimits) {
    match signal {
        StopSignal::Silence => {
            log::info!(
                "Silence auto-stop: session {} silent for {:?} ms, stopping dictation",
                session_id,
                limits.silence_ms
            );
            // Stops exactly like a toggle press. Double stops are harmless,
            // since the worker skips a stop unless the app is still Recording.
            crate::dictation_worker::submit(app, crate::dictation_worker::Intent::Stop);
        }
        StopSignal::LongRecordingSilence | StopSignal::Ceiling => {
            if signal == StopSignal::Ceiling {
                log::warn!(
                    "Recording ceiling: session {} reached {} ms, stopping and transcribing",
                    session_id,
                    limits.ceiling_ms
                );
            } else {
                log::info!(
                    "Long recording: session {} past {} ms and silent for {:?} ms, stopping and transcribing",
                    session_id,
                    limits.long_recording_ms,
                    limits.silence_ms
                );
            }
            // The session-scoped salvage path: it no-ops when the user
            // already stopped this session, and transcribes what was captured.
            crate::dictation_worker::dispatch(
                app,
                crate::dictation_worker::DictationCommand::FinishTimedOut {
                    session_id: session_id.to_string(),
                },
            );
        }
    }
}

/// Root-mean-square level of one chunk of mono samples; 0.0 for an empty
/// chunk. Shared by the level meter, silence auto-stop, the incremental
/// segmenter, and silence trimming.
pub(crate) fn chunk_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    (samples
        .iter()
        .map(|sample| (*sample as f64) * (*sample as f64))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt() as f32
}

/// Whether a recorded WAV contains any speech — true if any short (30 ms) window
/// reaches [`AUTO_STOP_SPEECH_RMS`]. Used to skip transcription on effectively
/// silent clips (e.g. a record toggle on/off with nothing said) so Whisper can't
/// hallucinate text from silence. **Fails open:** an unreadable/unsamplable WAV
/// returns `true`, so real audio is never dropped on a read hiccup.
pub(crate) fn wav_has_speech(path: &std::path::Path) -> bool {
    const WINDOW: usize = 480; // 30 ms at 16 kHz

    let Ok(mut reader) = hound::WavReader::open(path) else {
        return true;
    };

    let mut window: Vec<f32> = Vec::with_capacity(WINDOW);
    for sample in reader.samples::<i16>() {
        let Ok(sample) = sample else {
            return true; // decode hiccup → fail open
        };
        window.push(sample as f32 / i16::MAX as f32);
        if window.len() == WINDOW {
            if chunk_rms(&window) >= AUTO_STOP_SPEECH_RMS {
                return true;
            }
            window.clear();
        }
    }
    !window.is_empty() && chunk_rms(&window) >= AUTO_STOP_SPEECH_RMS
}

fn finalize_recording(
    info: RecordingSessionInfo,
    stop_reason: StopReason,
    samples: Vec<f32>,
    temp_dir: PathBuf,
    source_sample_rate: u32,
    min_recording_ms: u64,
    silence_trim_enabled: bool,
) -> Result<RecordingResult, CommandError> {
    let stopped_at = Utc::now();
    let duration_ms = duration_ms(samples.len(), source_sample_rate);
    let disconnected = stop_reason == StopReason::Disconnected;

    if stop_reason == StopReason::Cancelled {
        return Ok(recording_result(
            info,
            RecordingResultStatus::Cancelled,
            None,
            duration_ms,
            None,
            Some("Recording was cancelled.".to_string()),
            stopped_at,
            disconnected,
        ));
    }

    if duration_ms < min_recording_ms {
        return Ok(recording_result(
            info,
            RecordingResultStatus::TooShort,
            None,
            duration_ms,
            None,
            Some(format!(
                "Recording was shorter than the {} ms minimum.",
                min_recording_ms
            )),
            stopped_at,
            disconnected,
        ));
    }

    let samples = trim_silence(samples, source_sample_rate, silence_trim_enabled);
    let normalized = normalize_to_whisper_wav_samples(samples, source_sample_rate);
    let wav_path = temp_dir.join(format!("{}.wav", info.session_id));
    write_wav(&wav_path, &normalized)?;
    let bytes_written = fs::metadata(&wav_path).ok().map(|metadata| metadata.len());
    let status = if stop_reason == StopReason::Timeout {
        RecordingResultStatus::TimedOut
    } else {
        RecordingResultStatus::Completed
    };

    Ok(recording_result(
        info,
        status,
        Some(wav_path.to_string_lossy().to_string()),
        duration_ms,
        bytes_written,
        None,
        stopped_at,
        disconnected,
    ))
}

#[allow(clippy::too_many_arguments)]
fn recording_result(
    info: RecordingSessionInfo,
    status: RecordingResultStatus,
    wav_path: Option<String>,
    duration_ms: u64,
    bytes_written: Option<u64>,
    reason: Option<String>,
    stopped_at: DateTime<Utc>,
    disconnected: bool,
) -> RecordingResult {
    RecordingResult {
        session_id: info.session_id,
        status,
        wav_path,
        duration_ms,
        sample_rate: TARGET_SAMPLE_RATE,
        channels: TARGET_CHANNELS,
        bits_per_sample: TARGET_BITS_PER_SAMPLE,
        bytes_written,
        reason,
        started_at: info.started_at,
        stopped_at,
        is_note: info.is_note,
        is_transform: info.is_transform,
        disconnected,
    }
}

/// The running microphone stream. cpal streams are not `Send` on every system
/// (the recording session lives in shared state), so the stream is built,
/// played and dropped on a capture thread of its own. Dropping the handle
/// stops the stream and waits until it is gone, so no audio arrives after.
struct CaptureStream {
    stop_tx: Option<Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl CaptureStream {
    fn start(
        source: InputSource,
        chunk_tx: Sender<AudioChunk>,
        app: AppHandle,
        session_id: String,
    ) -> Result<Self, CommandError> {
        match source {
            InputSource::Microphone {
                device,
                config,
                sample_format,
            } => Self::start_microphone(device, config, sample_format, chunk_tx, app, session_id),
            #[cfg(feature = "e2e-audio")]
            InputSource::TestClip(clip) => Self::start_test_clip(clip, chunk_tx),
        }
    }

    /// Plays a test clip in place of the microphone (see `test_audio`).
    #[cfg(feature = "e2e-audio")]
    fn start_test_clip(
        clip: crate::test_audio::Clip,
        chunk_tx: Sender<AudioChunk>,
    ) -> Result<Self, CommandError> {
        let (stop_tx, stop_rx) = bounded::<()>(1);
        let thread = thread::Builder::new()
            .name("scribe-e2e-audio".to_string())
            .spawn(move || {
                crate::test_audio::play(&clip, &stop_rx, |samples| {
                    chunk_tx.send(AudioChunk { samples }).is_ok()
                })
            })
            .map_err(|error| {
                CommandError::new(
                    "recording_failed",
                    format!("Could not start the test audio thread. {}", error),
                )
            })?;
        Ok(Self {
            stop_tx: Some(stop_tx),
            thread: Some(thread),
        })
    }

    fn start_microphone(
        device: Device,
        config: StreamConfig,
        sample_format: SampleFormat,
        chunk_tx: Sender<AudioChunk>,
        app: AppHandle,
        session_id: String,
    ) -> Result<Self, CommandError> {
        let (ready_tx, ready_rx) = bounded::<Result<(), CommandError>>(1);
        let (stop_tx, stop_rx) = bounded::<()>(1);
        let thread = thread::Builder::new()
            .name("scribe-audio-capture".to_string())
            .spawn(move || {
                let started =
                    build_input_stream(&device, &config, sample_format, chunk_tx, app, session_id)
                        .and_then(|stream| {
                            stream.play().map(|()| stream).map_err(|error| {
                                CommandError::new(
                                    "recording_failed",
                                    format!("Could not start recording stream. {}", error),
                                )
                            })
                        });
                let stream = match started {
                    Ok(stream) => stream,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                // Runs until stopped, or until the handle is gone.
                let _ = stop_rx.recv();
                drop(stream);
            })
            .map_err(|error| {
                CommandError::new(
                    "recording_failed",
                    format!("Could not start the audio capture thread. {}", error),
                )
            })?;

        let handle = Self {
            stop_tx: Some(stop_tx),
            thread: Some(thread),
        };
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(CommandError::new(
                "recording_failed",
                "The audio capture thread stopped before the recording started.",
            )),
        }
    }
}

impl Drop for CaptureStream {
    fn drop(&mut self) {
        // Closing the channel wakes the capture thread whether or not a
        // stop message fits.
        drop(self.stop_tx.take());
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                log::error!("The audio capture thread panicked");
            }
        }
    }
}

fn build_input_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    chunk_tx: Sender<AudioChunk>,
    app: AppHandle,
    session_id: String,
) -> Result<Stream, CommandError> {
    let channels = config.channels as usize;
    // cpal delivers stream errors (the mic being unplugged, the WASAPI endpoint
    // dying) on its own thread via this callback. Logging alone leaves the
    // worker blocked on the now-dead chunk channel and the app wedged in
    // Recording until the user stops or the safety ceiling fires. Instead, queue the same stop +
    // transcribe path the timeout uses, exactly once, on the dictation worker
    // (the callback must never block, and queueing never does), and surface a
    // recording error so the UI can tell the user the mic dropped. `handled`
    // collapses repeated error callbacks into a single recovery.
    let handled = Arc::new(AtomicBool::new(false));
    let err_fn = move |error: cpal::StreamError| {
        log::error!("Audio stream error: {}", error);
        if handled.swap(true, Ordering::SeqCst) {
            return;
        }
        crate::dictation_worker::dispatch(
            &app,
            crate::dictation_worker::DictationCommand::FinishDisconnected {
                session_id: session_id.clone(),
            },
        );
    };

    match sample_format {
        SampleFormat::F32 => {
            build_stream(device, config, channels, chunk_tx, err_fn, |sample: f32| {
                sample
            })
        }
        SampleFormat::F64 => {
            build_stream(device, config, channels, chunk_tx, err_fn, |sample: f64| {
                sample as f32
            })
        }
        SampleFormat::I8 => {
            build_stream(device, config, channels, chunk_tx, err_fn, |sample: i8| {
                sample as f32 / i8::MAX as f32
            })
        }
        SampleFormat::I16 => {
            build_stream(device, config, channels, chunk_tx, err_fn, |sample: i16| {
                sample as f32 / i16::MAX as f32
            })
        }
        SampleFormat::I32 => {
            build_stream(device, config, channels, chunk_tx, err_fn, |sample: i32| {
                sample as f32 / i32::MAX as f32
            })
        }
        SampleFormat::I64 => {
            build_stream(device, config, channels, chunk_tx, err_fn, |sample: i64| {
                sample as f32 / i64::MAX as f32
            })
        }
        SampleFormat::U8 => {
            build_stream(device, config, channels, chunk_tx, err_fn, |sample: u8| {
                unsigned_to_f32(sample as f32, u8::MAX as f32)
            })
        }
        SampleFormat::U16 => {
            build_stream(device, config, channels, chunk_tx, err_fn, |sample: u16| {
                unsigned_to_f32(sample as f32, u16::MAX as f32)
            })
        }
        SampleFormat::U32 => {
            build_stream(device, config, channels, chunk_tx, err_fn, |sample: u32| {
                unsigned_to_f32(sample as f32, u32::MAX as f32)
            })
        }
        SampleFormat::U64 => {
            build_stream(device, config, channels, chunk_tx, err_fn, |sample: u64| {
                unsigned_to_f32(sample as f32, u64::MAX as f32)
            })
        }
        _ => Err(CommandError::new(
            "recording_failed",
            format!("Unsupported microphone sample format: {:?}", sample_format),
        )),
    }
}

fn build_stream<T, F>(
    device: &Device,
    config: &StreamConfig,
    channels: usize,
    chunk_tx: Sender<AudioChunk>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
    convert: F,
) -> Result<Stream, CommandError>
where
    T: cpal::SizedSample + Copy,
    F: Fn(T) -> f32 + Send + Sync + 'static,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mut mono = Vec::with_capacity(data.len().saturating_div(channels.max(1)));
                for frame in data.chunks(channels.max(1)) {
                    let sum = frame.iter().fold(0.0_f32, |sum, sample| {
                        sum + convert(*sample).clamp(-1.0, 1.0)
                    });
                    mono.push(sum / frame.len().max(1) as f32);
                }

                let _ = chunk_tx.send(AudioChunk { samples: mono });
            },
            err_fn,
            None,
        )
        .map_err(|error| {
            CommandError::new(
                "recording_failed",
                format!("Could not build microphone input stream. {}", error),
            )
        })
}

fn unsigned_to_f32(sample: f32, max: f32) -> f32 {
    (sample / max) * 2.0 - 1.0
}

fn device_candidates(selected_mic_id: Option<&str>) -> Result<Vec<DeviceCandidate>, CommandError> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let endpoint_ids_by_name = crate::platform::audio_endpoint_ids_by_name();
    let mut endpoint_positions = HashMap::<String, usize>::new();
    let devices = host.input_devices().map_err(|error| {
        CommandError::new(
            "microphone_unavailable",
            format!("Could not enumerate microphones. {}", error),
        )
    })?;
    let mut candidates = Vec::new();

    for (index, device) in devices.enumerate() {
        let name = device
            .name()
            .unwrap_or_else(|_| format!("Microphone {}", index + 1));
        // Systems list plumbing devices too (ALSA resamplers, mixers and raw
        // hardware nodes on Linux); only real inputs are offered.
        let Some(label) = crate::platform::microphone_label(&name) else {
            continue;
        };
        let endpoint_id = endpoint_ids_by_name.get(&name).and_then(|ids| {
            let position = endpoint_positions.entry(name.clone()).or_default();
            let id = ids.get(*position).cloned();
            *position += 1;
            id
        });
        let id = endpoint_id
            .clone()
            .or_else(|| crate::platform::microphone_id(&name))
            .unwrap_or_else(|| fallback_microphone_id(index, &name));
        let is_selected = selected_mic_id
            .map(|selected| selected == id || selected == label)
            .unwrap_or(false);

        candidates.push(DeviceCandidate {
            info: MicrophoneInfo {
                id,
                name: label,
                endpoint_id,
                is_default: default_name.as_deref() == Some(name.as_str()),
                is_selected,
                is_available: true,
            },
            device,
        });
    }

    Ok(candidates)
}

/// Which input device to record from. Prefers the microphone chosen in
/// Settings; when it isn't connected (a headset was unplugged), falls back to
/// the system default, then to the first device, instead of failing every
/// dictation. Returns the index and whether that fallback happened.
fn choose_input_index(
    candidates: &[&MicrophoneInfo],
    selected_mic_id: Option<&str>,
) -> Option<(usize, bool)> {
    if let Some(selected_mic_id) = selected_mic_id {
        if let Some(index) = candidates
            .iter()
            .position(|info| info.id == selected_mic_id || info.name == selected_mic_id)
        {
            return Some((index, false));
        }
    }

    let fell_back = selected_mic_id.is_some();
    let index = candidates
        .iter()
        .position(|info| info.is_default)
        .or_else(|| (!candidates.is_empty()).then_some(0))?;
    Some((index, fell_back))
}

/// Where a recording's audio comes from.
enum InputSource {
    Microphone {
        device: Device,
        config: StreamConfig,
        sample_format: SampleFormat,
    },
    /// A WAV file, in builds made for the end-to-end tests only.
    #[cfg(feature = "e2e-audio")]
    TestClip(crate::test_audio::Clip),
}

/// The input a recording is about to start from.
struct OpenedInput {
    info: MicrophoneInfo,
    /// The selected microphone isn't connected; this is the fallback.
    fallback: bool,
    sample_rate: u32,
    channels: u16,
    source: InputSource,
}

/// Opens the selected microphone (or the fallback), or the test clip when the
/// end-to-end test input is on.
fn open_input(selected_mic_id: Option<&str>) -> Result<OpenedInput, CommandError> {
    #[cfg(feature = "e2e-audio")]
    if let Some(clip) = crate::test_audio::clip_for_recording()? {
        return Ok(OpenedInput {
            info: clip.microphone_info(),
            fallback: false,
            sample_rate: clip.sample_rate,
            channels: 1,
            source: InputSource::TestClip(clip),
        });
    }

    let (candidate, fallback) = select_input_device(selected_mic_id)?;
    let supported_config = candidate
        .device
        .default_input_config()
        .map_err(|error| microphone_unavailable(candidate.info.name.as_str(), error))?;
    let sample_format = supported_config.sample_format();
    let config: StreamConfig = supported_config.into();
    Ok(OpenedInput {
        info: candidate.info,
        fallback,
        sample_rate: config.sample_rate.0,
        channels: config.channels,
        source: InputSource::Microphone {
            device: candidate.device,
            config,
            sample_format,
        },
    })
}

fn select_input_device(
    selected_mic_id: Option<&str>,
) -> Result<(DeviceCandidate, bool), CommandError> {
    let mut candidates = device_candidates(selected_mic_id)?;
    let infos: Vec<&MicrophoneInfo> = candidates.iter().map(|candidate| &candidate.info).collect();

    let (index, fell_back) = choose_input_index(&infos, selected_mic_id).ok_or_else(|| {
        CommandError::new(
            "no_microphone_selected",
            "No microphone is available. Connect or enable a microphone, then try again.",
        )
    })?;
    Ok((candidates.remove(index), fell_back))
}

fn fallback_microphone_id(index: usize, name: &str) -> String {
    format!("cpal:{}:{}", index, name)
}

fn microphone_unavailable(name: &str, error: impl std::fmt::Display) -> CommandError {
    CommandError::new(
        "microphone_unavailable",
        format!("Microphone '{}' is unavailable. {}", name, error),
    )
}

/// Cuts leading and trailing silence (windowed RMS below
/// `SILENCE_RMS_THRESHOLD`), keeping `TRIM_PADDING_MS` of padding on each
/// side of the detected speech. When the whole recording is below the
/// threshold the input is returned unchanged rather than trimmed to nothing.
fn trim_silence(samples: Vec<f32>, sample_rate: u32, enabled: bool) -> Vec<f32> {
    if !enabled || samples.is_empty() || sample_rate == 0 {
        return samples;
    }

    let window_len = ((sample_rate as usize * TRIM_WINDOW_MS as usize) / 1_000).max(1);
    let mut first_loud_sample = None;
    let mut loud_end_sample = 0usize;

    for (index, window) in samples.chunks(window_len).enumerate() {
        if chunk_rms(window) >= SILENCE_RMS_THRESHOLD {
            let window_start = index * window_len;
            if first_loud_sample.is_none() {
                first_loud_sample = Some(window_start);
            }
            loud_end_sample = window_start + window.len();
        }
    }

    let Some(first_loud_sample) = first_loud_sample else {
        // All silence: keep the recording unchanged instead of emptying it.
        return samples;
    };

    let padding = (sample_rate as usize * TRIM_PADDING_MS as usize) / 1_000;
    let start = first_loud_sample.saturating_sub(padding);
    let end = (loud_end_sample + padding).min(samples.len());
    // In place: a copy would double the memory of a long recording.
    let mut samples = samples;
    samples.truncate(end);
    samples.drain(..start);
    samples
}

/// Converts mono samples at the source rate into Whisper's 16 kHz input,
/// clamped to [-1, 1]. Takes the samples by value so a recording already at
/// 16 kHz is clamped in place instead of copied: a long recording's buffer is
/// the biggest allocation the app makes.
pub(crate) fn normalize_to_whisper_wav_samples(
    samples: Vec<f32>,
    source_sample_rate: u32,
) -> Vec<f32> {
    let mut resampled = if source_sample_rate == TARGET_SAMPLE_RATE {
        samples
    } else {
        let resampled = resample_to_target_rate(&samples, source_sample_rate, TARGET_SAMPLE_RATE);
        drop(samples);
        resampled
    };
    for sample in &mut resampled {
        *sample = sample.clamp(-1.0, 1.0);
    }
    resampled
}

fn resample_to_target_rate(
    samples: &[f32],
    source_sample_rate: u32,
    target_sample_rate: u32,
) -> Vec<f32> {
    if samples.is_empty() || source_sample_rate == 0 {
        return Vec::new();
    }

    if source_sample_rate == target_sample_rate {
        return samples.to_vec();
    }

    resample_with_rubato(samples, source_sample_rate, target_sample_rate)
        .unwrap_or_else(|| resample_linear(samples, source_sample_rate, target_sample_rate))
}

/// Input frames the sinc resampler takes per call. Streaming the recording
/// through in fixed chunks keeps the resampler's own buffers small; handing
/// it the whole recording as one chunk made it allocate several more copies
/// of a recording that can be half an hour long.
const RESAMPLE_CHUNK_FRAMES: usize = 4_096;

fn resample_with_rubato(
    samples: &[f32],
    source_sample_rate: u32,
    target_sample_rate: u32,
) -> Option<Vec<f32>> {
    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };

    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 128,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = SincFixedIn::<f32>::new(
        target_sample_rate as f64 / source_sample_rate as f64,
        1.0,
        params,
        RESAMPLE_CHUNK_FRAMES,
        1,
    )
    .ok()?;
    let expected_len = ((samples.len() as f64) * (target_sample_rate as f64)
        / (source_sample_rate as f64))
        .round()
        .max(1.0) as usize;
    // SincFixedIn starts its output aligned with the input (measured: under
    // a sample of shift), so nothing is dropped from the front; the end is
    // flushed below until the whole length is out.
    let mut output = Vec::with_capacity(expected_len + resampler.output_frames_max());
    let mut buffer = resampler.output_buffer_allocate(true);

    let (chunks, remainder) = samples.as_chunks::<RESAMPLE_CHUNK_FRAMES>();
    for chunk in chunks {
        let (_, written) = resampler
            .process_into_buffer(&[chunk], &mut buffer, None)
            .ok()?;
        output.extend_from_slice(&buffer[0][..written]);
    }
    if !remainder.is_empty() {
        let (_, written) = resampler
            .process_partial_into_buffer(Some(&[remainder]), &mut buffer, None)
            .ok()?;
        output.extend_from_slice(&buffer[0][..written]);
    }
    while output.len() < expected_len {
        let (_, written) = resampler
            .process_partial_into_buffer(None::<&[&[f32]]>, &mut buffer, None)
            .ok()?;
        if written == 0 {
            break;
        }
        output.extend_from_slice(&buffer[0][..written]);
    }

    output.resize(expected_len, 0.0);
    Some(output)
}

fn resample_linear(samples: &[f32], source_sample_rate: u32, target_sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() || source_sample_rate == 0 {
        return Vec::new();
    }

    if source_sample_rate == target_sample_rate {
        return samples.to_vec();
    }

    let output_len = ((samples.len() as f64) * (target_sample_rate as f64)
        / (source_sample_rate as f64))
        .round()
        .max(1.0) as usize;
    let source_step = source_sample_rate as f64 / target_sample_rate as f64;
    let mut output = Vec::with_capacity(output_len);

    for index in 0..output_len {
        let source_position = index as f64 * source_step;
        let left = source_position.floor() as usize;
        let right = (left + 1).min(samples.len() - 1);
        let fraction = (source_position - left as f64) as f32;
        output.push(samples[left] + (samples[right] - samples[left]) * fraction);
    }

    output
}

pub(crate) fn write_wav(path: &PathBuf, samples: &[f32]) -> Result<(), CommandError> {
    let spec = hound::WavSpec {
        channels: TARGET_CHANNELS,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: TARGET_BITS_PER_SAMPLE,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|error| {
        CommandError::new(
            "recording_failed",
            format!("Could not create normalized WAV file. {}", error),
        )
    })?;

    for sample in samples {
        let sample = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        writer.write_sample(sample).map_err(|error| {
            CommandError::new(
                "recording_failed",
                format!("Could not write normalized WAV sample. {}", error),
            )
        })?;
    }

    writer.finalize().map_err(|error| {
        CommandError::new(
            "recording_failed",
            format!("Could not finalize normalized WAV file. {}", error),
        )
    })
}

pub(crate) fn duration_ms(sample_count: usize, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }

    ((sample_count as u128 * 1_000) / sample_rate as u128) as u64
}

pub fn emit_recording_error(app: &AppHandle, error: CommandError) {
    log::error!("Recording error {}: {}", error.code, error.message);
    let _ = app.emit(
        "audio://recording-error",
        RecordingErrorEvent {
            code: error.code,
            message: error.message,
        },
    );
}

#[cfg(test)]
mod tests {
    fn mic(id: &str, is_default: bool) -> MicrophoneInfo {
        MicrophoneInfo {
            id: id.to_string(),
            name: format!("{id} name"),
            endpoint_id: None,
            is_default,
            is_selected: false,
            is_available: true,
        }
    }

    #[test]
    fn a_stream_that_ends_first_keeps_the_stop_reason() {
        for reason in [
            StopReason::Cancelled,
            StopReason::Timeout,
            StopReason::Disconnected,
            StopReason::Completed,
        ] {
            let (control_tx, control_rx) = unbounded();
            control_tx.send(WorkerControl::Stop(reason)).unwrap();
            assert_eq!(stop_reason_after_stream_end(&control_rx), reason);
        }

        // The session went away without a stop: finalize what was captured.
        let (control_tx, control_rx) = unbounded::<WorkerControl>();
        drop(control_tx);
        assert_eq!(
            stop_reason_after_stream_end(&control_rx),
            StopReason::Completed
        );
    }

    #[test]
    fn uses_the_selected_microphone_when_connected() {
        let (a, b) = (mic("a", true), mic("b", false));
        assert_eq!(choose_input_index(&[&a, &b], Some("b")), Some((1, false)));
        // Matching by display name also counts as the selected mic.
        assert_eq!(
            choose_input_index(&[&a, &b], Some("b name")),
            Some((1, false))
        );
    }

    #[test]
    fn falls_back_to_the_default_when_the_selected_mic_is_missing() {
        let (a, b) = (mic("a", false), mic("b", true));
        assert_eq!(
            choose_input_index(&[&a, &b], Some("headset")),
            Some((1, true))
        );
    }

    #[test]
    fn falls_back_to_the_first_device_without_a_default() {
        let (a, b) = (mic("a", false), mic("b", false));
        assert_eq!(
            choose_input_index(&[&a, &b], Some("headset")),
            Some((0, true))
        );
        assert_eq!(choose_input_index(&[&a, &b], None), Some((0, false)));
    }

    #[test]
    fn no_devices_means_no_choice() {
        assert_eq!(choose_input_index(&[], Some("headset")), None);
        assert_eq!(choose_input_index(&[], None), None);
    }

    use super::*;

    #[test]
    fn resamples_to_target_rate() {
        let samples = vec![0.0; 48_000];
        let resampled = normalize_to_whisper_wav_samples(samples, 48_000);

        assert_eq!(resampled.len(), 16_000);
    }

    #[test]
    fn chunked_resampling_matches_one_shot_resampling() {
        use rubato::{
            Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType,
            WindowFunction,
        };

        // A 220 Hz tone at 48 kHz, 2.5 s long: not a whole number of chunks.
        let tone = |i: usize, rate: f32| {
            (2.0 * std::f32::consts::PI * 220.0 * i as f32 / rate).sin() * 0.5
        };
        let source: Vec<f32> = (0..120_000).map(|i| tone(i, 48_000.0)).collect();

        // What the resampler gives when handed the whole recording at once
        // (how it was used before chunking).
        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };
        let mut one_shot = SincFixedIn::<f32>::new(1.0 / 3.0, 1.0, params, source.len(), 1)
            .unwrap()
            .process(std::slice::from_ref(&source), None)
            .unwrap()
            .remove(0);
        one_shot.truncate(39_900);

        let resampled = normalize_to_whisper_wav_samples(source, 48_000);
        assert_eq!(resampled.len(), 40_000);
        assert_eq!(&resampled[..one_shot.len()], one_shot.as_slice());
        // The end is flushed through the filter instead of padded with zeros.
        let tail = &resampled[39_950..];
        for (offset, sample) in tail.iter().enumerate() {
            let expected = tone(39_950 + offset, 16_000.0);
            assert!(
                (sample - expected).abs() < 0.05,
                "tail {offset}: {sample} vs {expected}"
            );
        }
    }

    #[test]
    fn normalizing_16_khz_audio_only_clamps() {
        let resampled = normalize_to_whisper_wav_samples(vec![0.25, 1.5, -2.0], 16_000);
        assert_eq!(resampled, vec![0.25, 1.0, -1.0]);
    }

    /// 1 kHz keeps the policy tests readable: one sample is one millisecond.
    const POLICY_RATE: u32 = 1_000;

    fn limits(silence_from_start: bool) -> StopLimits {
        StopLimits {
            silence_ms: Some(60_000),
            silence_from_start,
            long_recording_ms: 600_000,
            ceiling_ms: 1_800_000,
        }
    }

    /// Feeds `ms` of audio in 100 ms chunks and returns the first signal with
    /// the elapsed time (ms since the start of the recording) it fired at.
    fn run(policy: &mut StopPolicy, level: f32, ms: usize) -> Option<(StopSignal, u64)> {
        for _ in 0..ms / 100 {
            if let Some(signal) = policy.observe(&vec![level; 100]) {
                return Some((signal, duration_ms(policy.elapsed_samples, POLICY_RATE)));
            }
        }
        None
    }

    const TALK: f32 = 0.5;
    const QUIET: f32 = 0.0;

    #[test]
    fn talking_through_the_long_recording_point_keeps_recording() {
        for silence_from_start in [true, false] {
            let mut policy = StopPolicy::new(limits(silence_from_start), POLICY_RATE);
            // 20 minutes of talking with 30 s pauses: never stops.
            for _ in 0..20 {
                assert_eq!(run(&mut policy, TALK, 30_000), None);
                assert_eq!(run(&mut policy, QUIET, 30_000), None);
            }
        }
    }

    #[test]
    fn toggle_silence_auto_stop_is_unchanged_before_the_point() {
        let mut policy = StopPolicy::new(limits(true), POLICY_RATE);
        // Silence before any speech never stops a toggle recording early.
        assert_eq!(run(&mut policy, QUIET, 120_000), None);
        assert_eq!(run(&mut policy, TALK, 5_000), None);
        assert_eq!(
            run(&mut policy, QUIET, 61_000),
            Some((StopSignal::Silence, 185_000))
        );
    }

    #[test]
    fn hold_to_talk_never_stops_on_silence_before_the_point() {
        let mut policy = StopPolicy::new(limits(false), POLICY_RATE);
        assert_eq!(run(&mut policy, TALK, 5_000), None);
        assert_eq!(run(&mut policy, QUIET, 500_000), None);
    }

    #[test]
    fn a_pause_past_the_point_stops_after_the_silence_window() {
        for silence_from_start in [true, false] {
            let mut policy = StopPolicy::new(limits(silence_from_start), POLICY_RATE);
            // Talking up to 11 minutes, then a pause: stops 60 s into it.
            assert_eq!(run(&mut policy, TALK, 660_000), None);
            let expected = if silence_from_start {
                StopSignal::Silence
            } else {
                StopSignal::LongRecordingSilence
            };
            assert_eq!(run(&mut policy, QUIET, 61_000), Some((expected, 720_000)));
        }
    }

    #[test]
    fn a_stuck_hold_key_on_silence_ends_at_the_point() {
        // Nothing was ever said: the silence run already covers the window
        // when the point arrives, so it stops right there.
        let mut policy = StopPolicy::new(limits(false), POLICY_RATE);
        assert_eq!(
            run(&mut policy, QUIET, 700_000),
            Some((StopSignal::LongRecordingSilence, 600_000))
        );
        // Same for a forgotten toggle recording that never heard speech.
        let mut policy = StopPolicy::new(limits(true), POLICY_RATE);
        assert_eq!(
            run(&mut policy, QUIET, 700_000),
            Some((StopSignal::LongRecordingSilence, 600_000))
        );
    }

    #[test]
    fn the_ceiling_stops_even_mid_sentence() {
        for silence_from_start in [true, false] {
            let mut policy = StopPolicy::new(limits(silence_from_start), POLICY_RATE);
            assert_eq!(
                run(&mut policy, TALK, 2_000_000),
                Some((StopSignal::Ceiling, 1_800_000))
            );
        }
    }

    #[test]
    fn with_silence_auto_stop_off_only_the_ceiling_stops() {
        let mut limits = limits(false);
        limits.silence_ms = None;
        let mut policy = StopPolicy::new(limits, POLICY_RATE);
        assert_eq!(run(&mut policy, QUIET, 1_700_000), None);
        assert_eq!(
            run(&mut policy, QUIET, 200_000),
            Some((StopSignal::Ceiling, 1_800_000))
        );
    }

    #[test]
    fn room_noise_breaks_the_silence_run() {
        // Between the silence and speech thresholds: not silent, so the
        // silence rule never fires, and the ceiling still does.
        let mut policy = StopPolicy::new(limits(false), POLICY_RATE);
        assert_eq!(
            run(&mut policy, 0.02, 2_000_000),
            Some((StopSignal::Ceiling, 1_800_000))
        );
    }

    #[test]
    fn the_policy_fires_once() {
        let mut policy = StopPolicy::new(limits(false), POLICY_RATE);
        assert!(run(&mut policy, QUIET, 700_000).is_some());
        assert_eq!(run(&mut policy, QUIET, 3_000_000), None);
    }

    #[test]
    fn limits_follow_the_settings_and_the_start_kind() {
        let settings = AppSettings::default();
        let toggle = StopLimits::for_recording(&settings, None, false, true);
        assert_eq!(
            toggle,
            StopLimits {
                silence_ms: Some(60_000),
                silence_from_start: true,
                long_recording_ms: 600_000,
                ceiling_ms: 1_800_000,
            }
        );
        let hold = StopLimits::for_recording(&settings, None, false, false);
        assert!(!hold.silence_from_start);
        assert_eq!(hold.silence_ms, Some(60_000));

        let off = AppSettings {
            silence_auto_stop_enabled: false,
            ..AppSettings::default()
        };
        assert_eq!(
            StopLimits::for_recording(&off, None, false, true).silence_ms,
            None
        );

        // A test clip: fixed length, no silence rule.
        let clip = StopLimits::for_recording(&settings, Some(4_000), true, false);
        assert_eq!(clip.silence_ms, None);
        assert_eq!(clip.ceiling_ms, 4_000);
        assert_eq!(clip.long_recording_ms, 4_000);
    }

    #[test]
    fn duration_uses_source_sample_rate() {
        assert_eq!(duration_ms(4_800, 48_000), 100);
        assert_eq!(duration_ms(16_000, 16_000), 1_000);
    }

    fn silence(samples: usize) -> Vec<f32> {
        vec![0.0; samples]
    }

    fn speech(samples: usize) -> Vec<f32> {
        vec![0.5; samples]
    }

    const TEST_SAMPLE_RATE: u32 = 16_000;
    /// 300 ms of padding at 16 kHz.
    const PADDING_SAMPLES: usize = 4_800;

    #[test]
    fn trims_leading_and_trailing_silence_keeping_padding() {
        // 1 s silence + 0.5 s speech + 1 s silence at 16 kHz.
        let mut samples = silence(16_000);
        samples.extend(speech(8_000));
        samples.extend(silence(16_000));

        let trimmed = trim_silence(samples.clone(), TEST_SAMPLE_RATE, true);

        let expected_start = 16_000 - PADDING_SAMPLES;
        let expected_end = 24_000 + PADDING_SAMPLES;
        assert_eq!(trimmed, samples[expected_start..expected_end].to_vec());
        assert_eq!(trimmed.len(), 8_000 + 2 * PADDING_SAMPLES);
    }

    #[test]
    fn trim_keeps_short_edges_without_underflow() {
        // Speech starts immediately and runs to the end: nothing to trim and
        // padding must not extend past the buffer.
        let samples = speech(8_000);

        let trimmed = trim_silence(samples.clone(), TEST_SAMPLE_RATE, true);

        assert_eq!(trimmed, samples);
    }

    #[test]
    fn trim_returns_all_silence_unchanged() {
        let samples = silence(32_000);

        let trimmed = trim_silence(samples.clone(), TEST_SAMPLE_RATE, true);

        assert_eq!(trimmed, samples);
    }

    #[test]
    fn trim_disabled_returns_input_unchanged() {
        let mut samples = silence(16_000);
        samples.extend(speech(8_000));
        samples.extend(silence(16_000));

        let trimmed = trim_silence(samples.clone(), TEST_SAMPLE_RATE, false);

        assert_eq!(trimmed, samples);
    }

    #[test]
    fn chunk_rms_handles_empty_and_constant_signals() {
        assert_eq!(chunk_rms(&[]), 0.0);
        assert!((chunk_rms(&[0.5; 64]) - 0.5).abs() < 1e-6);
        assert!(chunk_rms(&[0.0; 64]) < SILENCE_RMS_THRESHOLD);
    }

    #[test]
    fn writes_pcm_16_mono_wav() {
        let path = std::env::temp_dir().join(format!("scribe-test-{}.wav", Uuid::new_v4()));
        write_wav(&path, &[0.0, 0.5, -0.5]).unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn wav_has_speech_distinguishes_silence_from_speech() {
        // A silent clip → no speech (so transcription is skipped).
        let silent = std::env::temp_dir().join(format!("scribe-silent-{}.wav", Uuid::new_v4()));
        write_wav(&silent, &vec![0.0_f32; 16_000]).unwrap();
        assert!(
            !wav_has_speech(&silent),
            "all-silence WAV must report no speech"
        );
        let _ = fs::remove_file(&silent);

        // A loud clip (well above AUTO_STOP_SPEECH_RMS) → speech present.
        let loud = std::env::temp_dir().join(format!("scribe-loud-{}.wav", Uuid::new_v4()));
        write_wav(&loud, &vec![0.5_f32; 16_000]).unwrap();
        assert!(wav_has_speech(&loud), "loud WAV must report speech");
        let _ = fs::remove_file(&loud);

        // A missing file fails open (true) so real audio is never dropped.
        assert!(wav_has_speech(std::path::Path::new("/no/such/scribe.wav")));
    }
}
