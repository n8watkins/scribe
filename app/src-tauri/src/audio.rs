#![cfg_attr(not(windows), allow(dead_code))]

use std::{fs, path::PathBuf, thread, time::Duration};

use chrono::{DateTime, Utc};
#[cfg(windows)]
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, SampleFormat, Stream, StreamConfig,
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
    app_state::{AppEvent, AppStateSnapshot, AppStatus},
    commands::BackendState,
    error::CommandError,
    settings::AppSettings,
};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const TARGET_CHANNELS: u16 = 1;
const TARGET_BITS_PER_SAMPLE: u16 = 16;
#[cfg(windows)]
const LEVEL_EVENT_INTERVAL: Duration = Duration::from_millis(40);
const DEFAULT_TEST_CLIP_MS: u64 = 3_000;

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingResultStatus {
    Completed,
    TooShort,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize)]
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
struct RecordingErrorEvent {
    code: String,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    Completed,
    Cancelled,
    Timeout,
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

#[derive(Debug)]
pub struct AudioService {
    temp_dir: PathBuf,
    #[cfg(windows)]
    active: Option<RecordingSession>,
    last_result: Option<RecordingResult>,
}

#[derive(Debug)]
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
        }
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
        };
        let (chunk_tx, chunk_rx) = unbounded::<AudioChunk>();
        let (control_tx, control_rx) = unbounded::<WorkerControl>();
        let app_for_worker = app.clone();
        let temp_dir = self.temp_dir.clone();
        let min_recording_ms = settings.min_recording_ms as u64;
        let silence_trim_enabled = settings.silence_trim_enabled;
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
            )
        });

        let stream =
            build_input_stream(&candidate.device, &stream_config, sample_format, chunk_tx)?;
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
    ) -> Result<StartOutcome, CommandError> {
        Err(unsupported_audio_platform())
    }

    #[cfg(windows)]
    fn stop(&mut self, reason: StopReason) -> Result<Option<RecordingResult>, CommandError> {
        let Some(session) = self.active.take() else {
            return Ok(self.last_result.clone());
        };

        session.timeout_active.store(false, Ordering::SeqCst);
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

pub fn start_recording_for_app(
    app: &AppHandle,
    request: Option<StartRecordingRequest>,
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

        let outcome = state
            .audio()?
            .start(app, &settings, request.unwrap_or_default(), false)?;

        if outcome.started {
            let snapshot = state.transition_app_state(AppEvent::StartRecording)?;
            emit_state_snapshot(app, &snapshot);
        }

        Ok(outcome.info)
    } else if snapshot.status == AppStatus::Recording {
        let outcome = state
            .audio()?
            .start(app, &settings, request.unwrap_or_default(), false)?;
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
    };
    let outcome = state.audio()?.start(app, &settings, request, true)?;

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
    let _ = app.emit("audio://recording-stopped", &result);
    Ok(result)
}

#[cfg(windows)]
fn timeout_recording_for_app(app: &AppHandle, session_id: &str) -> Result<(), CommandError> {
    let result = stop_recording_with_reason(app, StopReason::Timeout, Some(session_id))?;
    let _ = app.emit("audio://recording-timeout", &result);
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
        let snapshot = state.transition_app_state(AppEvent::StopRecording)?;
        emit_state_snapshot(app, &snapshot);
    }

    let result = if let Some(session_id) = session_id {
        state
            .audio()?
            .stop_session(session_id, reason)?
            .ok_or_else(|| {
                CommandError::new("recording_not_active", "No active recording session.")
            })?
    } else {
        state.audio()?.stop(reason)?.ok_or_else(|| {
            CommandError::new("recording_not_active", "No active recording session.")
        })?
    };

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

fn emit_state_snapshot(app: &AppHandle, snapshot: &AppStateSnapshot) {
    let _ = app.emit("localdictate:app-state", snapshot);
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
fn recording_worker(
    app: AppHandle,
    info: RecordingSessionInfo,
    chunk_rx: Receiver<AudioChunk>,
    control_rx: Receiver<WorkerControl>,
    temp_dir: PathBuf,
    source_sample_rate: u32,
    min_recording_ms: u64,
    silence_trim_enabled: bool,
) -> Result<RecordingResult, CommandError> {
    let mut samples = Vec::<f32>::new();
    let mut last_level_emit = Instant::now()
        .checked_sub(LEVEL_EVENT_INTERVAL)
        .unwrap_or_else(Instant::now);
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
                        samples.extend(chunk.samples);
                    }
                    Err(_) => break StopReason::Completed,
                }
            }
        }
    };

    while let Ok(chunk) = chunk_rx.try_recv() {
        maybe_emit_level(&app, &info.session_id, &chunk.samples, &mut last_level_emit);
        samples.extend(chunk.samples);
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
    let rms = (samples
        .iter()
        .map(|sample| (*sample as f64) * (*sample as f64))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt() as f32;
    let event = AudioLevelEvent {
        session_id: session_id.to_string(),
        level: rms.clamp(0.0, 1.0),
        peak: peak.clamp(0.0, 1.0),
        rms: rms.clamp(0.0, 1.0),
    };
    let _ = app.emit("audio://level", event);
    *last_level_emit = Instant::now();
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

    if stop_reason == StopReason::Cancelled {
        return Ok(recording_result(
            info,
            RecordingResultStatus::Cancelled,
            None,
            duration_ms,
            None,
            Some("Recording was cancelled.".to_string()),
            stopped_at,
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
        ));
    }

    let samples = trim_silence_placeholder(samples, source_sample_rate, silence_trim_enabled);
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
    ))
}

fn recording_result(
    info: RecordingSessionInfo,
    status: RecordingResultStatus,
    wav_path: Option<String>,
    duration_ms: u64,
    bytes_written: Option<u64>,
    reason: Option<String>,
    stopped_at: DateTime<Utc>,
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
    }
}

#[cfg(windows)]
fn build_input_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    chunk_tx: Sender<AudioChunk>,
) -> Result<Stream, CommandError> {
    let channels = config.channels as usize;
    let err_fn = |error| {
        eprintln!("LocalDictate audio stream error: {}", error);
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
    let host = cpal::default_host();
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

fn trim_silence_placeholder(samples: Vec<f32>, _sample_rate: u32, _enabled: bool) -> Vec<f32> {
    samples
}

fn normalize_to_whisper_wav_samples(samples: &[f32], source_sample_rate: u32) -> Vec<f32> {
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

fn write_wav(path: &PathBuf, samples: &[f32]) -> Result<(), CommandError> {
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

fn duration_ms(sample_count: usize, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }

    ((sample_count as u128 * 1_000) / sample_rate as u128) as u64
}

#[cfg(windows)]
fn emit_recording_error(app: &AppHandle, error: CommandError) {
    let _ = app.emit(
        "audio://recording-error",
        RecordingErrorEvent {
            code: error.code,
            message: error.message,
        },
    );
}

fn unsupported_audio_platform() -> CommandError {
    CommandError::new(
        "audio_platform_unsupported",
        "Audio capture is implemented for Windows in LocalDictate V1.",
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

    #[test]
    fn writes_pcm_16_mono_wav() {
        let path = std::env::temp_dir().join(format!("localdictate-test-{}.wav", Uuid::new_v4()));
        write_wav(&path, &[0.0, 0.5, -0.5]).unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);

        let _ = fs::remove_file(path);
    }
}
