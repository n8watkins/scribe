use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppStatus {
    Idle,
    Recording,
    Stopping,
    Transcribing,
    Pasting,
    Ready,
    Error,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppEvent {
    StartRecording,
    StopRecording,
    ValidAudio,
    AudioTooShort,
    TranscriptionSucceeded,
    TranscriptionFailed,
    StartPasting,
    PasteCompleted,
    ReadyTimeout,
    ResetError,
    Pause,
    Resume,
    CancelRecording,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppErrorInfo {
    pub code: AppErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppErrorCode {
    NoMicrophoneSelected,
    MicrophonePermissionDenied,
    MicrophoneUnavailable,
    RecordingFailed,
    AudioTooShort,
    WhisperModelMissing,
    WhisperTranscriptionFailed,
    ModelDownloadFailed,
    HotkeyRegistrationFailed,
    PasteFailed,
    ClipboardRestoreFailed,
    AppDatabaseError,
    InvalidStateTransition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateSnapshot {
    pub status: AppStatus,
    pub error: Option<AppErrorInfo>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AppStateMachine {
    status: AppStatus,
    error: Option<AppErrorInfo>,
    updated_at: DateTime<Utc>,
}

impl Default for AppStateMachine {
    fn default() -> Self {
        Self {
            status: AppStatus::Idle,
            error: None,
            updated_at: Utc::now(),
        }
    }
}

impl AppStateMachine {
    pub fn snapshot(&self) -> AppStateSnapshot {
        AppStateSnapshot {
            status: self.status.clone(),
            error: self.error.clone(),
            updated_at: self.updated_at,
        }
    }

    pub fn status(&self) -> &AppStatus {
        &self.status
    }

    pub fn transition(
        &mut self,
        event: AppEvent,
    ) -> Result<AppStateSnapshot, StateTransitionError> {
        use AppEvent::*;
        use AppStatus::*;

        let next = match (&self.status, event) {
            (Idle, StartRecording) | (Ready, StartRecording) => Recording,
            (Recording, StopRecording) => Stopping,
            (Recording, CancelRecording) => Idle,
            (Stopping, ValidAudio) => Transcribing,
            (Stopping, AudioTooShort) => Idle,
            (Transcribing, TranscriptionSucceeded) => Ready,
            (Transcribing, TranscriptionFailed) => Error,
            (Ready, StartPasting) => Pasting,
            (Pasting, PasteCompleted) => Ready,
            (Ready, ReadyTimeout) => Idle,
            (Error, ResetError) => Idle,
            (Idle, Pause) | (Ready, Pause) => Paused,
            (Paused, Resume) => Idle,
            _ => {
                return Err(StateTransitionError {
                    from: self.status.clone(),
                })
            }
        };

        self.status = next;
        self.error = if self.status == AppStatus::Error {
            Some(AppErrorInfo {
                code: AppErrorCode::WhisperTranscriptionFailed,
                message: "Whisper transcription failed. Try again or choose another model."
                    .to_string(),
            })
        } else {
            None
        };
        self.updated_at = Utc::now();

        Ok(self.snapshot())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTransitionError {
    pub from: AppStatus,
}

impl std::fmt::Display for StateTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid transition from {:?}", self.from)
    }
}

impl std::error::Error for StateTransitionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_main_recording_flow() {
        let mut machine = AppStateMachine::default();

        assert_eq!(machine.status(), &AppStatus::Idle);
        assert_eq!(
            machine.transition(AppEvent::StartRecording).unwrap().status,
            AppStatus::Recording
        );
        assert_eq!(
            machine.transition(AppEvent::StopRecording).unwrap().status,
            AppStatus::Stopping
        );
        assert_eq!(
            machine.transition(AppEvent::ValidAudio).unwrap().status,
            AppStatus::Transcribing
        );
        assert_eq!(
            machine
                .transition(AppEvent::TranscriptionSucceeded)
                .unwrap()
                .status,
            AppStatus::Ready
        );
        assert_eq!(
            machine.transition(AppEvent::ReadyTimeout).unwrap().status,
            AppStatus::Idle
        );
    }

    #[test]
    fn ignores_new_recording_while_transcribing() {
        let mut machine = AppStateMachine::default();

        machine.transition(AppEvent::StartRecording).unwrap();
        machine.transition(AppEvent::StopRecording).unwrap();
        machine.transition(AppEvent::ValidAudio).unwrap();

        let error = machine.transition(AppEvent::StartRecording).unwrap_err();
        assert_eq!(error.from, AppStatus::Transcribing);
        assert_eq!(machine.status(), &AppStatus::Transcribing);
    }

    #[test]
    fn audio_too_short_returns_to_idle() {
        let mut machine = AppStateMachine::default();

        machine.transition(AppEvent::StartRecording).unwrap();
        machine.transition(AppEvent::StopRecording).unwrap();

        assert_eq!(
            machine.transition(AppEvent::AudioTooShort).unwrap().status,
            AppStatus::Idle
        );
    }
}
