#![cfg_attr(not(windows), allow(dead_code))]

use std::{fs, path::PathBuf, thread, time::Duration};

use chrono::{DateTime, Utc};
#[cfg(windows)]
use cpal::{
    platform::{WasapiDevice as Device, WasapiHost, WasapiStream as Stream},
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, StreamConfig,
};
#[cfg(windows)]
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
#[cfg(any(test, windows))]
use uuid::Uuid;

#[cfg(windows)]
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
#[cfg(windows)]
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
#[cfg(windows)]
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
    pub max_duration_ms: u64,
    pub is_test_clip: bool,
    pub is_note: bool,
    pub is_transform: bool,
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
#[cfg(windows)]
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
#[cfg(windows)]
struct AudioChunk {
    samples: Vec<f32>,
}

#[derive(Debug)]
#[cfg(windows)]
enum WorkerControl {
    Stop(StopReason),
}

pub struct AudioService {
    temp_dir: PathBuf,
    #[cfg(windows)]
    active: Option<RecordingSession>,
    last_result: Option<RecordingResult>,
    last_test_clip: Option<PathBuf>,
}

#[cfg(windows)]
struct RecordingSession {
    info: RecordingSessionInfo,
    stream: Stream,
    control_tx: Sender<WorkerControl>,
    worker: thread::JoinHandle<Result<RecordingResult, CommandError>>,
    timeout_active: Arc<AtomicBool>,
}

#[derive(Debug)]
struct StartOutcome {
    info: RecordingSessionInfo,
    started: bool,
}

#[derive(Debug)]
#[cfg(windows)]
struct DeviceCandidate {
    info: MicrophoneInfo,
    device: Device,
}

impl AudioService {
    pub fn new(temp_dir: PathBuf) -> Self {
        Self {
            temp_dir,
            #[cfg(windows)]
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

    #[cfg(windows)]
    fn list_microphones(
        &self,
        selected_mic_id: Option<&str>,
    ) -> Result<Vec<MicrophoneInfo>, CommandError> {
        Ok(device_candidates(selected_mic_id)?
            .into_iter()
            .map(|candidate| candidate.info)
            .collect())
    }

    #[cfg(not(windows))]
    fn list_microphones(
        &self,
        _selected_mic_id: Option<&str>,
    ) -> Result<Vec<MicrophoneInfo>, CommandError> {
        Err(unsupported_audio_platform())
    }

    #[cfg(windows)]
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
        let candidate = select_input_device(selected_mic_id)?;
        let supported_config = candidate
            .device
            .default_input_config()
            .map_err(|error| microphone_unavailable(candidate.info.name.as_str(), error))?;
        let sample_format = supported_config.sample_format();
        let stream_config: StreamConfig = supported_config.into();
        let source_sample_rate = stream_config.sample_rate.0;
        let source_channels = stream_config.channels;
        let session_id = Uuid::new_v4().to_string();
        let started_at = Utc::now();
        let max_duration_ms = request
            .max_duration_ms
            .unwrap_or(settings.max_recording_ms as u64)
            .max(settings.min_recording_ms as u64);
        let info = RecordingSessionInfo {
            session_id: session_id.clone(),
            microphone_id: candidate.info.id.clone(),
            microphone_name: candidate.info.name.clone(),
            sample_rate: source_sample_rate,
            channels: source_channels,
            started_at,
            max_duration_ms,
            is_test_clip,
            is_note: request.is_note && !is_test_clip,
            is_transform: request.is_transform && !is_test_clip,
        };
        let (chunk_tx, chunk_rx) = unbounded::<AudioChunk>();
        let (control_tx, control_rx) = unbounded::<WorkerControl>();
        let app_for_worker = app.clone();
        let temp_dir = self.temp_dir.clone();
        let min_recording_ms = settings.min_recording_ms as u64;
        let silence_trim_enabled = settings.silence_trim_enabled;
        // Silence auto-stop only applies to toggle-style starts (toggle
        // hotkey, tray menu, UI Start button) — never hold-to-talk or test
        // clips — and only when the setting is enabled.
        let auto_stop_after_ms =
            (allow_auto_stop && !is_test_clip && settings.silence_auto_stop_enabled)
                .then_some(settings.silence_auto_stop_ms as u64);
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
                auto_stop_after_ms,
                incremental,
            )
        });

        let stream = build_input_stream(
            &candidate.device,
            &stream_config,
            sample_format,
            chunk_tx,
            app.clone(),
            session_id.clone(),
        )?;
        stream.play().map_err(|error| {
            CommandError::new(
                "recording_failed",
                format!("Could not start recording stream. {}", error),
            )
        })?;

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
            "Recording started: session {} (mic '{}', {} Hz, test clip: {}, silence auto-stop: {:?} ms)",
            info.session_id,
            info.microphone_name,
            info.sample_rate,
            is_test_clip,
            auto_stop_after_ms
        );
        let _ = app.emit("audio://recording-started", &info);
        Ok(StartOutcome {
            info,
            started: true,
        })
    }

    #[cfg(not(windows))]
    fn start(
        &mut self,
        _app: &AppHandle,
        _settings: &AppSettings,
        _request: StartRecordingRequest,
        _is_test_clip: bool,
        _allow_auto_stop: bool,
    ) -> Result<StartOutcome, CommandError> {
        Err(unsupported_audio_platform())
    }

    #[cfg(windows)]
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

    #[cfg(not(windows))]
    fn stop(&mut self, _reason: StopReason) -> Result<Option<RecordingResult>, CommandError> {
        Ok(self.last_result.clone())
    }

    #[cfg(windows)]
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

    #[cfg(not(windows))]
    fn stop_session(
        &mut self,
        _session_id: &str,
        _reason: StopReason,
    ) -> Result<Option<RecordingResult>, CommandError> {
        Ok(None)
    }

    #[cfg(windows)]
    fn active_session_id(&self) -> Option<&str> {
        self.active
            .as_ref()
            .map(|session| session.info.session_id.as_str())
    }

    #[cfg(not(windows))]
    fn active_session_id(&self) -> Option<&str> {
        None
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
            let snapshot = state.transition_app_state(AppEvent::StartRecording)?;
            emit_state_snapshot(app, &snapshot);
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

#[cfg(windows)]
pub fn stop_recording_for_app(app: &AppHandle) -> Result<RecordingResult, CommandError> {
    stop_recording_with_reason(app, StopReason::Completed, None)
}

#[cfg(not(windows))]
pub fn stop_recording_for_app(_app: &AppHandle) -> Result<RecordingResult, CommandError> {
    Err(unsupported_audio_platform())
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
        // No transcription will follow: drop any incremental session.
        state.incremental().discard(&result.session_id);
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

#[cfg(windows)]
fn timeout_recording_for_app(app: &AppHandle, session_id: &str) -> Result<(), CommandError> {
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

    let state = app.state::<BackendState>();
    let status = state.app_state()?.status().clone();
    crate::tray::update_tray_status(app, status);
    Ok(())
}

/// Recovers from a capture-stream error (e.g. the mic was unplugged mid-record).
/// Runs on a background thread spawned by the cpal error callback. Stops the
/// in-flight session like a normal completion — so whatever was captured before
/// the drop is still transcribed — surfaces a `microphone_unavailable`
/// recording error, then finishes transcription exactly like the manual-stop
/// and timeout paths so the app never strands in Recording/Transcribing. If the
/// session is already gone (the user stopped first, or a duplicate error
/// callback slipped past the guard), it no-ops cleanly.
#[cfg(windows)]
fn disconnect_recording_for_app(app: &AppHandle, session_id: &str) -> Result<(), CommandError> {
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
             Anything captured was saved to your History (not pasted) — reconnect the mic and record again.",
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

    let state = app.state::<BackendState>();
    let status = state.app_state()?.status().clone();
    crate::tray::update_tray_status(app, status);
    Ok(())
}

#[cfg(windows)]
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
        // No transcription will follow: drop any incremental session.
        state.incremental().discard(&result.session_id);
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

#[cfg(windows)]
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

        let should_stop = app
            .try_state::<BackendState>()
            .and_then(|state| {
                state
                    .audio()
                    .ok()
                    .and_then(|audio| audio.active_session_id().map(str::to_owned))
            })
            .map(|active_session_id| active_session_id == session_id)
            .unwrap_or(false);

        if should_stop {
            if let Err(error) = timeout_recording_for_app(&app, &session_id) {
                emit_recording_error(&app, error);
            }
        }
    });
}

#[cfg(windows)]
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
    auto_stop_after_ms: Option<u64>,
    mut incremental: Option<crate::incremental::WorkerLink>,
) -> Result<RecordingResult, CommandError> {
    let mut samples = Vec::<f32>::new();
    let mut last_level_emit = Instant::now()
        .checked_sub(LEVEL_EVENT_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut auto_stop = SilenceAutoStop::new(auto_stop_after_ms, source_sample_rate);
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
                        auto_stop.observe(&app, &info.session_id, &chunk.samples);
                        if let Some(link) = incremental.as_mut() {
                            link.push_chunk(&chunk.samples);
                        }
                        samples.extend(chunk.samples);
                    }
                    Err(_) => break StopReason::Completed,
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

#[cfg(windows)]
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

/// Watches per-chunk RMS during a recording and stops the dictation after
/// `auto_stop_after_ms` of continuous silence — but only once speech has been
/// heard, and only once per session. Stopping goes through the same
/// `tray::stop_dictation` path as a toggle-hotkey stop so transcription and
/// output run exactly as if the user stopped manually.
#[cfg(windows)]
struct SilenceAutoStop {
    auto_stop_after_ms: Option<u64>,
    sample_rate: u32,
    speech_detected: bool,
    silence_samples: usize,
    triggered: bool,
}

#[cfg(windows)]
impl SilenceAutoStop {
    fn new(auto_stop_after_ms: Option<u64>, sample_rate: u32) -> Self {
        Self {
            auto_stop_after_ms,
            sample_rate,
            speech_detected: false,
            silence_samples: 0,
            triggered: false,
        }
    }

    fn observe(&mut self, app: &AppHandle, session_id: &str, samples: &[f32]) {
        let Some(limit_ms) = self.auto_stop_after_ms else {
            return;
        };
        if self.triggered || samples.is_empty() {
            return;
        }

        let rms = chunk_rms(samples);
        if rms >= AUTO_STOP_SPEECH_RMS {
            self.speech_detected = true;
            self.silence_samples = 0;
            return;
        }
        if rms >= SILENCE_RMS_THRESHOLD {
            // Not loud enough to count as speech, but not silent either:
            // the continuous-silence run is broken.
            self.silence_samples = 0;
            return;
        }
        if !self.speech_detected {
            return;
        }

        self.silence_samples += samples.len();
        if duration_ms(self.silence_samples, self.sample_rate) < limit_ms {
            return;
        }

        self.triggered = true;
        log::info!(
            "Silence auto-stop: session {} silent for {} ms, stopping dictation",
            session_id,
            limit_ms
        );
        // Stop from a separate thread: stop_dictation joins this worker
        // thread, so it must not run on it. Double stops are harmless —
        // stop_dictation no-ops unless the app is still Recording.
        let app = app.clone();
        thread::spawn(move || {
            if let Err(error) = crate::tray::stop_dictation(&app) {
                log::warn!("Silence auto-stop could not stop dictation: {}", error);
            }
        });
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
    let normalized = normalize_to_whisper_wav_samples(&samples, source_sample_rate);
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

#[cfg(windows)]
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
    // Recording until the user stops or the max-duration timeout fires — up to
    // ten minutes of captured silence. Instead, drive the same stop +
    // transcribe path the timeout uses, exactly once, on a fresh thread (the
    // callback must never block), and surface a recording error so the UI can
    // tell the user the mic dropped. `handled` collapses repeated error
    // callbacks into a single recovery.
    let handled = Arc::new(AtomicBool::new(false));
    let err_fn = move |error: cpal::StreamError| {
        log::error!("Audio stream error: {}", error);
        if handled.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = app.clone();
        let session_id = session_id.clone();
        thread::spawn(move || {
            if let Err(error) = disconnect_recording_for_app(&app, &session_id) {
                log::warn!(
                    "Could not recover from a mid-recording audio stream error: {}",
                    error.message
                );
            }
        });
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

#[cfg(windows)]
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

#[cfg(windows)]
fn unsigned_to_f32(sample: f32, max: f32) -> f32 {
    (sample / max) * 2.0 - 1.0
}

#[cfg(windows)]
fn device_candidates(selected_mic_id: Option<&str>) -> Result<Vec<DeviceCandidate>, CommandError> {
    let host = WasapiHost::new().map_err(|error| {
        CommandError::new(
            "microphone_unavailable",
            format!("Could not initialize Windows audio host. {}", error),
        )
    })?;
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let endpoint_ids_by_name = endpoint_ids_by_name();
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
        let endpoint_id = endpoint_ids_by_name.get(&name).and_then(|ids| {
            let position = endpoint_positions.entry(name.clone()).or_default();
            let id = ids.get(*position).cloned();
            *position += 1;
            id
        });
        let id = endpoint_id
            .clone()
            .unwrap_or_else(|| fallback_microphone_id(index, &name));
        let is_selected = selected_mic_id
            .map(|selected| selected == id || selected == name)
            .unwrap_or(false);

        candidates.push(DeviceCandidate {
            info: MicrophoneInfo {
                id,
                name: name.clone(),
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

#[cfg(windows)]
fn select_input_device(selected_mic_id: Option<&str>) -> Result<DeviceCandidate, CommandError> {
    let mut candidates = device_candidates(selected_mic_id)?;

    if let Some(selected_mic_id) = selected_mic_id {
        if let Some(index) = candidates.iter().position(|candidate| {
            candidate.info.id == selected_mic_id || candidate.info.name == selected_mic_id
        }) {
            return Ok(candidates.remove(index));
        }

        return Err(CommandError::new(
            "microphone_unavailable",
            format!(
                "Selected microphone is not available: {}. Choose another microphone.",
                selected_mic_id
            ),
        ));
    }

    if let Some(index) = candidates
        .iter()
        .position(|candidate| candidate.info.is_default)
    {
        return Ok(candidates.remove(index));
    }

    candidates.into_iter().next().ok_or_else(|| {
        CommandError::new(
            "no_microphone_selected",
            "No microphone is available. Connect or enable a microphone, then try again.",
        )
    })
}

#[cfg(windows)]
fn fallback_microphone_id(index: usize, name: &str) -> String {
    format!("cpal:{}:{}", index, name)
}

#[cfg(windows)]
fn microphone_unavailable(name: &str, error: impl std::fmt::Display) -> CommandError {
    CommandError::new(
        "microphone_unavailable",
        format!("Microphone '{}' is unavailable. {}", name, error),
    )
}

#[cfg(windows)]
fn endpoint_ids_by_name() -> HashMap<String, Vec<String>> {
    use windows::Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Media::Audio::{
            eCapture, eConsole, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
        },
        System::Com::{
            CoCreateInstance, CoInitializeEx, StructuredStorage::PropVariantToStringAlloc,
            CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
        },
    };

    let mut endpoints = HashMap::<String, Vec<String>>::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return endpoints;
        };
        let Ok(collection) = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) else {
            return endpoints;
        };
        let Ok(count) = collection.GetCount() else {
            return endpoints;
        };

        for index in 0..count {
            let Ok(device) = collection.Item(index) else {
                continue;
            };
            let Ok(id) = device.GetId() else {
                continue;
            };
            let Ok(store) = device.OpenPropertyStore(STGM_READ) else {
                continue;
            };
            let Ok(value) = store.GetValue(&PKEY_Device_FriendlyName) else {
                continue;
            };
            let Ok(name) = PropVariantToStringAlloc(&value) else {
                continue;
            };

            endpoints
                .entry(name.to_string().unwrap_or_default())
                .or_default()
                .push(id.to_string().unwrap_or_default());
        }

        let _ = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole);
    }

    endpoints
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
    samples[start..end].to_vec()
}

pub(crate) fn normalize_to_whisper_wav_samples(
    samples: &[f32],
    source_sample_rate: u32,
) -> Vec<f32> {
    let resampled = resample_to_target_rate(samples, source_sample_rate, TARGET_SAMPLE_RATE);
    resampled
        .into_iter()
        .map(|sample| sample.clamp(-1.0, 1.0))
        .collect()
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
        2.0,
        params,
        samples.len(),
        1,
    )
    .ok()?;
    let input = vec![samples.to_vec()];
    let mut output = resampler.process(&input, None).ok()?.into_iter().next()?;
    let expected_len = ((samples.len() as f64) * (target_sample_rate as f64)
        / (source_sample_rate as f64))
        .round()
        .max(1.0) as usize;

    match output.len().cmp(&expected_len) {
        std::cmp::Ordering::Greater => output.truncate(expected_len),
        std::cmp::Ordering::Less => output.resize(expected_len, 0.0),
        std::cmp::Ordering::Equal => {}
    }

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

#[cfg(not(windows))]
fn unsupported_audio_platform() -> CommandError {
    CommandError::new(
        "audio_platform_unsupported",
        "Audio capture is implemented for Windows in Scribe V1.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_to_target_rate() {
        let samples = vec![0.0; 48_000];
        let resampled = normalize_to_whisper_wav_samples(&samples, 48_000);

        assert_eq!(resampled.len(), 16_000);
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
        assert!(!wav_has_speech(&silent), "all-silence WAV must report no speech");
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
