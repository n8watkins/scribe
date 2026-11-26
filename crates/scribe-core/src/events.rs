//! Every event the engine sends to the interface, as one type.
//!
//! Each variant carries the payload Scribe sends today. [`CoreEvent::name`]
//! is the event name the windows listen for and the event serializes as its
//! bare payload (`#[serde(untagged)]`), so a shell that emits
//! `(event.name(), &event)` sends byte for byte what the engine sends now.
//! The tests below pin every name and one JSON sample of every payload.
//!
//! The emit call sites are not converted yet: each still calls `app.emit`
//! with the same name and payload (see docs/CORE-SPLIT.md).

use serde::Serialize;

use crate::{
    app_state::AppStateSnapshot,
    audio::{AudioLevelEvent, RecordingErrorEvent, RecordingResult, RecordingSessionInfo},
    dictation::{DictationEmptyPayload, DictationResult, DictationSaveFailedPayload},
    hotkeys::HotkeyRegistrationFailure,
    incremental::PartialTranscriptEvent,
    model_manager::ModelDownloadProgress,
    output::{OutputFailedPayload, OutputResult, OutputStartedPayload},
    replacements_file::ReplacementsInfo,
    settings::AppSettings,
    speech_engine::EngineStatus,
};

/// The `scribe:hotkey-registration-failed` payload.
#[derive(Debug, Clone, Serialize)]
pub struct HotkeyRegistrationFailedPayload {
    pub failures: Vec<HotkeyRegistrationFailure>,
}

/// The `scribe:navigate` payload: the route the main window should show.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigatePayload {
    pub route: String,
}

/// An event from the engine to the interface.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum CoreEvent {
    /// `scribe:app-state`: the dictation state machine moved.
    AppState(AppStateSnapshot),
    /// `audio://level`: the microphone level while recording.
    AudioLevel(AudioLevelEvent),
    /// `audio://recording-started`.
    RecordingStarted(RecordingSessionInfo),
    /// `audio://recording-stopped`: every way a recording ends.
    RecordingStopped(RecordingResult),
    /// `audio://recording-timeout`: a recording stopped on its own.
    RecordingTimeout(RecordingResult),
    /// `audio://recording-error`.
    RecordingError(RecordingErrorEvent),
    /// `model://download-progress`.
    ModelDownloadProgress(ModelDownloadProgress),
    /// `scribe:hotkey-registration-failed`.
    HotkeyRegistrationFailed(HotkeyRegistrationFailedPayload),
    /// `scribe:hotkey-action`: a hotkey fired; the payload is the action's
    /// event name (`toggle_dictation`, `note_dictation`, ...).
    HotkeyAction(String),
    /// `scribe:selection-transform-failed`: the message to show.
    SelectionTransformFailed(String),
    /// `scribe:selection-transformed`: the text that replaced the selection.
    SelectionTransformed(String),
    /// `scribe:settings-changed`: the stored settings after a change.
    SettingsChanged(AppSettings),
    /// `scribe:engine-status`: the speech engine's status changed.
    EngineStatus(EngineStatus),
    /// `scribe:dictation-save-failed`.
    DictationSaveFailed(DictationSaveFailedPayload),
    /// `scribe:dictation-transcribed`.
    DictationTranscribed(DictationResult),
    /// `scribe:dictation-empty`: a dictation heard nothing.
    DictationEmpty(DictationEmptyPayload),
    /// `scribe:partial-transcript`: live text while recording.
    PartialTranscript(PartialTranscriptEvent),
    /// `scribe:output-started`.
    OutputStarted(OutputStartedPayload),
    /// `scribe:output-completed`.
    OutputCompleted(OutputResult),
    /// `scribe:output-failed`.
    OutputFailed(OutputFailedPayload),
    /// `scribe:navigate`: show the main window on a route (sent by the
    /// shell's open-dashboard hook).
    Navigate(NavigatePayload),
    /// `scribe:replacements-changed`: replacements.txt changed outside Scribe.
    ReplacementsChanged(ReplacementsInfo),
}

impl CoreEvent {
    /// The event name the windows listen for.
    pub fn name(&self) -> &'static str {
        match self {
            Self::AppState(_) => "scribe:app-state",
            Self::AudioLevel(_) => "audio://level",
            Self::RecordingStarted(_) => "audio://recording-started",
            Self::RecordingStopped(_) => "audio://recording-stopped",
            Self::RecordingTimeout(_) => "audio://recording-timeout",
            Self::RecordingError(_) => "audio://recording-error",
            Self::ModelDownloadProgress(_) => "model://download-progress",
            Self::HotkeyRegistrationFailed(_) => "scribe:hotkey-registration-failed",
            Self::HotkeyAction(_) => "scribe:hotkey-action",
            Self::SelectionTransformFailed(_) => "scribe:selection-transform-failed",
            Self::SelectionTransformed(_) => "scribe:selection-transformed",
            Self::SettingsChanged(_) => crate::settings_store::SETTINGS_CHANGED_EVENT,
            Self::EngineStatus(_) => crate::whisper_server::ENGINE_STATUS_EVENT,
            Self::DictationSaveFailed(_) => "scribe:dictation-save-failed",
            Self::DictationTranscribed(_) => "scribe:dictation-transcribed",
            Self::DictationEmpty(_) => "scribe:dictation-empty",
            Self::PartialTranscript(_) => crate::incremental::PARTIAL_TRANSCRIPT_EVENT,
            Self::OutputStarted(_) => "scribe:output-started",
            Self::OutputCompleted(_) => "scribe:output-completed",
            Self::OutputFailed(_) => "scribe:output-failed",
            Self::Navigate(_) => "scribe:navigate",
            Self::ReplacementsChanged(_) => crate::replacements_file::REPLACEMENTS_CHANGED_EVENT,
        }
    }

    /// The payload as JSON, exactly as it goes to the windows.
    pub fn payload(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app_state::{AppErrorCode, AppErrorInfo, AppStatus},
        audio::RecordingResultStatus,
        dictation::DictationStatus,
        hotkeys::HotkeyAction,
        models::{EngineKind, ModelStatus},
        output::{ClipboardPreservation, OutputAction, OutputStatus},
        settings::{OutputMode, PasteMethod},
        speech_engine::{LoadState, Measurement},
        text_replace::TextReplacement,
        transcript::Transcript,
    };
    use chrono::{DateTime, Utc};
    use serde_json::json;

    fn at() -> DateTime<Utc> {
        "2026-01-02T03:04:05Z".parse().unwrap()
    }

    fn transcript() -> Transcript {
        Transcript {
            id: "t1".into(),
            text: "hello there".into(),
            created_at: at(),
            duration_ms: Some(1200),
            word_count: 2,
            character_count: 11,
            model_id: Some("base.en".into()),
            language: Some("en".into()),
            output_mode: Some(OutputMode::AutoPaste),
            paste_method: Some(PasteMethod::ClipboardPaste),
            transcription_latency_ms: Some(300),
            audio_path: None,
            is_note: false,
            analysis: None,
            analysis_model: None,
            analysis_created_at: None,
        }
    }

    fn recording_result() -> RecordingResult {
        RecordingResult {
            session_id: "s1".into(),
            status: RecordingResultStatus::Completed,
            wav_path: Some("/tmp/s1.wav".into()),
            duration_ms: 1200,
            sample_rate: 16000,
            channels: 1,
            bits_per_sample: 16,
            bytes_written: Some(38400),
            reason: None,
            started_at: at(),
            stopped_at: at(),
            is_note: false,
            is_transform: false,
            disconnected: false,
        }
    }

    /// The position of each variant, so adding one fails to compile until it
    /// gets a sample below.
    fn index(event: &CoreEvent) -> usize {
        match event {
            CoreEvent::AppState(_) => 0,
            CoreEvent::AudioLevel(_) => 1,
            CoreEvent::RecordingStarted(_) => 2,
            CoreEvent::RecordingStopped(_) => 3,
            CoreEvent::RecordingTimeout(_) => 4,
            CoreEvent::RecordingError(_) => 5,
            CoreEvent::ModelDownloadProgress(_) => 6,
            CoreEvent::HotkeyRegistrationFailed(_) => 7,
            CoreEvent::HotkeyAction(_) => 8,
            CoreEvent::SelectionTransformFailed(_) => 9,
            CoreEvent::SelectionTransformed(_) => 10,
            CoreEvent::SettingsChanged(_) => 11,
            CoreEvent::EngineStatus(_) => 12,
            CoreEvent::DictationSaveFailed(_) => 13,
            CoreEvent::DictationTranscribed(_) => 14,
            CoreEvent::DictationEmpty(_) => 15,
            CoreEvent::PartialTranscript(_) => 16,
            CoreEvent::OutputStarted(_) => 17,
            CoreEvent::OutputCompleted(_) => 18,
            CoreEvent::OutputFailed(_) => 19,
            CoreEvent::Navigate(_) => 20,
            CoreEvent::ReplacementsChanged(_) => 21,
        }
    }

    /// One sample of every event: its name and its exact JSON today.
    fn samples() -> Vec<(CoreEvent, &'static str, serde_json::Value)> {
        let transcript_json = json!({
            "id": "t1",
            "text": "hello there",
            "createdAt": "2026-01-02T03:04:05Z",
            "durationMs": 1200,
            "wordCount": 2,
            "characterCount": 11,
            "modelId": "base.en",
            "language": "en",
            "outputMode": "auto_paste",
            "pasteMethod": "clipboard_paste",
            "transcriptionLatencyMs": 300,
            "audioPath": null,
            "isNote": false,
            "analysis": null,
            "analysisModel": null,
            "analysisCreatedAt": null
        });
        let recording_json = json!({
            "sessionId": "s1",
            "status": "completed",
            "wavPath": "/tmp/s1.wav",
            "durationMs": 1200,
            "sampleRate": 16000,
            "channels": 1,
            "bitsPerSample": 16,
            "bytesWritten": 38400,
            "reason": null,
            "startedAt": "2026-01-02T03:04:05Z",
            "stoppedAt": "2026-01-02T03:04:05Z",
            "isNote": false,
            "isTransform": false,
            "disconnected": false
        });
        let output_result = OutputResult {
            transcript_id: "t1".into(),
            action: OutputAction::Paste,
            status: OutputStatus::Completed,
            output_mode: OutputMode::AutoPaste,
            paste_method: Some(PasteMethod::ClipboardPaste),
            copied: false,
            pasted: true,
            clipboard_restored: Some(true),
            clipboard_preservation: ClipboardPreservation::RestoredAfterPaste,
            clipboard_restore_error: None,
            message: "Pasted.".into(),
        };
        let settings = AppSettings::default();
        let settings_json = serde_json::to_value(&settings).unwrap();
        vec![
            (
                CoreEvent::AppState(AppStateSnapshot {
                    status: AppStatus::Error,
                    error: Some(AppErrorInfo {
                        code: AppErrorCode::InvalidStateTransition,
                        message: "Nope.".into(),
                    }),
                    updated_at: at(),
                }),
                "scribe:app-state",
                json!({
                    "status": "Error",
                    "error": {"code": "invalid_state_transition", "message": "Nope."},
                    "updatedAt": "2026-01-02T03:04:05Z"
                }),
            ),
            (
                CoreEvent::AudioLevel(AudioLevelEvent {
                    session_id: "s1".into(),
                    level: 0.25,
                    peak: 0.5,
                    rms: 0.25,
                }),
                "audio://level",
                json!({"sessionId": "s1", "level": 0.25, "peak": 0.5, "rms": 0.25}),
            ),
            (
                CoreEvent::RecordingStarted(RecordingSessionInfo {
                    session_id: "s1".into(),
                    microphone_id: "mic".into(),
                    microphone_name: "Mic".into(),
                    sample_rate: 48000,
                    channels: 2,
                    started_at: at(),
                    max_duration_ms: 600000,
                    is_test_clip: false,
                    is_note: true,
                    is_transform: false,
                    microphone_fallback: false,
                }),
                "audio://recording-started",
                json!({
                    "sessionId": "s1",
                    "microphoneId": "mic",
                    "microphoneName": "Mic",
                    "sampleRate": 48000,
                    "channels": 2,
                    "startedAt": "2026-01-02T03:04:05Z",
                    "maxDurationMs": 600000,
                    "isTestClip": false,
                    "isNote": true,
                    "isTransform": false,
                    "microphoneFallback": false
                }),
            ),
            (
                CoreEvent::RecordingStopped(recording_result()),
                "audio://recording-stopped",
                recording_json.clone(),
            ),
            (
                CoreEvent::RecordingTimeout(recording_result()),
                "audio://recording-timeout",
                recording_json,
            ),
            (
                CoreEvent::RecordingError(RecordingErrorEvent {
                    code: "microphone_unavailable".into(),
                    message: "No mic.".into(),
                }),
                "audio://recording-error",
                json!({"code": "microphone_unavailable", "message": "No mic."}),
            ),
            (
                CoreEvent::ModelDownloadProgress(ModelDownloadProgress {
                    model_id: "base.en".into(),
                    bytes_downloaded: 50,
                    total_bytes: Some(100),
                    percent: Some(50.0),
                    status: ModelStatus::Downloading,
                }),
                "model://download-progress",
                json!({
                    "modelId": "base.en",
                    "bytesDownloaded": 50,
                    "totalBytes": 100,
                    "percent": 50.0,
                    "status": "downloading"
                }),
            ),
            (
                CoreEvent::HotkeyRegistrationFailed(HotkeyRegistrationFailedPayload {
                    failures: vec![HotkeyRegistrationFailure {
                        action: HotkeyAction::HoldToTalk,
                        shortcut: "Ctrl+Space".into(),
                        message: "Taken.".into(),
                    }],
                }),
                "scribe:hotkey-registration-failed",
                json!({"failures": [
                    {"action": "holdToTalk", "shortcut": "Ctrl+Space", "message": "Taken."}
                ]}),
            ),
            (
                CoreEvent::HotkeyAction(HotkeyAction::ToggleDictation.event_name().into()),
                "scribe:hotkey-action",
                json!("toggle_dictation"),
            ),
            (
                CoreEvent::SelectionTransformFailed("Highlight some text first.".into()),
                "scribe:selection-transform-failed",
                json!("Highlight some text first."),
            ),
            (
                CoreEvent::SelectionTransformed("Rewritten.".into()),
                "scribe:selection-transformed",
                json!("Rewritten."),
            ),
            (
                CoreEvent::SettingsChanged(settings),
                "scribe:settings-changed",
                settings_json,
            ),
            (
                CoreEvent::EngineStatus(EngineStatus {
                    engine: EngineKind::Parakeet,
                    model_id: Some("parakeet-v3".into()),
                    model_name: Some("Parakeet v3".into()),
                    state: LoadState::Loaded,
                    memory_bytes: Some(900),
                    keep_loaded: true,
                    measured: Some(Measurement {
                        load_ms: Some(1500),
                        memory_bytes: Some(800),
                    }),
                }),
                "scribe:engine-status",
                json!({
                    "engine": "parakeet",
                    "modelId": "parakeet-v3",
                    "modelName": "Parakeet v3",
                    "state": "loaded",
                    "memoryBytes": 900,
                    "keepLoaded": true,
                    "measured": {"loadMs": 1500, "memoryBytes": 800}
                }),
            ),
            (
                CoreEvent::DictationSaveFailed(DictationSaveFailedPayload {
                    transcript_id: "t1".into(),
                    message: "Not saved.".into(),
                }),
                "scribe:dictation-save-failed",
                json!({"transcriptId": "t1", "message": "Not saved."}),
            ),
            (
                CoreEvent::DictationTranscribed(DictationResult {
                    session_id: "s1".into(),
                    status: DictationStatus::NotSaved,
                    transcript: transcript(),
                    model_id: "base.en".into(),
                    duration_ms: 1200,
                    transcription_latency_ms: 300,
                }),
                "scribe:dictation-transcribed",
                json!({
                    "sessionId": "s1",
                    "status": "not_saved",
                    "transcript": transcript_json,
                    "modelId": "base.en",
                    "durationMs": 1200,
                    "transcriptionLatencyMs": 300
                }),
            ),
            (
                CoreEvent::DictationEmpty(DictationEmptyPayload {
                    message: "Nothing heard.".into(),
                }),
                "scribe:dictation-empty",
                json!({"message": "Nothing heard."}),
            ),
            (
                CoreEvent::PartialTranscript(PartialTranscriptEvent {
                    session_id: "s1".into(),
                    text: "hello".into(),
                    segments: 2,
                    finalized: false,
                }),
                "scribe:partial-transcript",
                json!({"sessionId": "s1", "text": "hello", "segments": 2, "finalized": false}),
            ),
            (
                CoreEvent::OutputStarted(OutputStartedPayload {
                    transcript_id: "t1".into(),
                    action: OutputAction::CopyAndPaste,
                    output_mode: OutputMode::CopyAndPaste,
                    paste_method: None,
                }),
                "scribe:output-started",
                json!({
                    "transcriptId": "t1",
                    "action": "copy_and_paste",
                    "outputMode": "copy_and_paste",
                    "pasteMethod": null
                }),
            ),
            (
                CoreEvent::OutputCompleted(output_result),
                "scribe:output-completed",
                json!({
                    "transcriptId": "t1",
                    "action": "paste",
                    "status": "completed",
                    "outputMode": "auto_paste",
                    "pasteMethod": "clipboard_paste",
                    "copied": false,
                    "pasted": true,
                    "clipboardRestored": true,
                    "clipboardPreservation": "restored_after_paste",
                    "clipboardRestoreError": null,
                    "message": "Pasted."
                }),
            ),
            (
                CoreEvent::OutputFailed(OutputFailedPayload {
                    transcript_id: "t1".into(),
                    code: "paste_failed".into(),
                    message: "No.".into(),
                }),
                "scribe:output-failed",
                json!({"transcriptId": "t1", "code": "paste_failed", "message": "No."}),
            ),
            (
                CoreEvent::Navigate(NavigatePayload {
                    route: "settings".into(),
                }),
                "scribe:navigate",
                json!({"route": "settings"}),
            ),
            (
                CoreEvent::ReplacementsChanged(ReplacementsInfo {
                    path: "/data/replacements.txt".into(),
                    rows: vec![TextReplacement {
                        from: "scribe".into(),
                        to: "Scribe".into(),
                    }],
                    skipped_lines: vec![3],
                }),
                "scribe:replacements-changed",
                json!({
                    "path": "/data/replacements.txt",
                    "rows": [{"from": "scribe", "to": "Scribe"}],
                    "skippedLines": [3]
                }),
            ),
        ]
    }

    #[test]
    fn every_event_keeps_its_name_and_payload() {
        let samples = samples();
        assert_eq!(samples.len(), 22);
        for (position, (event, name, json)) in samples.iter().enumerate() {
            assert_eq!(index(event), position, "{name} is out of order");
            assert_eq!(event.name(), *name);
            assert_eq!(&event.payload(), json, "{name}");
            // What a shell emits (the event itself) serializes the same.
            assert_eq!(&serde_json::to_value(event).unwrap(), json, "{name}");
        }
    }

    #[test]
    fn event_names_are_unique() {
        let mut names: Vec<_> = samples().iter().map(|(event, _, _)| event.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 22);
    }

    /// The settings event carries the settings the windows read, key for key.
    #[test]
    fn the_settings_event_sends_camel_case_settings() {
        let event = CoreEvent::SettingsChanged(AppSettings::default());
        let payload = event.payload();
        assert!(payload.get("outputMode").is_some());
        assert!(payload.get("hotkeys").is_some());
        assert!(payload.get("output_mode").is_none());
    }

    /// The hotkey failure payload is the one `hotkeys` sends today.
    #[test]
    fn the_hotkey_failure_payload_matches_the_hotkeys_module() {
        let failures = vec![HotkeyRegistrationFailure {
            action: HotkeyAction::TransformSelection,
            shortcut: "Ctrl+Alt+T".into(),
            message: "Taken.".into(),
        }];
        let today = serde_json::to_value(crate::hotkeys::RegistrationFailedEvent {
            failures: &failures,
        })
        .unwrap();
        let event =
            CoreEvent::HotkeyRegistrationFailed(HotkeyRegistrationFailedPayload { failures });
        assert_eq!(event.payload(), today);
    }
}
