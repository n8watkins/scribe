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
    /// Whisper finished but produced no text (e.g. a tap with no speech).
    /// Benign: the app returns to Idle instead of Error.
    TranscriptionEmpty,
    TranscriptionFailed,
    /// Stopping the audio stream failed; the recording is gone, so the app
    /// returns to Idle rather than stranding in Stopping.
    StopFailed,
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
            (Stopping, StopFailed) => Idle,
            (Transcribing, TranscriptionSucceeded) => Ready,
            (Transcribing, TranscriptionEmpty) => Idle,
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

    #[test]
    fn empty_transcription_returns_to_idle_without_error() {
        let mut machine = AppStateMachine::default();

        machine.transition(AppEvent::StartRecording).unwrap();
        machine.transition(AppEvent::StopRecording).unwrap();
        machine.transition(AppEvent::ValidAudio).unwrap();

        let snapshot = machine.transition(AppEvent::TranscriptionEmpty).unwrap();
        assert_eq!(snapshot.status, AppStatus::Idle);
        assert!(snapshot.error.is_none());
    }

    #[test]
    fn stop_failure_returns_to_idle() {
        let mut machine = AppStateMachine::default();

        machine.transition(AppEvent::StartRecording).unwrap();
        machine.transition(AppEvent::StopRecording).unwrap();

        let snapshot = machine.transition(AppEvent::StopFailed).unwrap();
        assert_eq!(snapshot.status, AppStatus::Idle);
        assert!(snapshot.error.is_none());
    }

    #[test]
    fn error_state_recovers_to_idle_and_can_record_again() {
        let mut machine = AppStateMachine::default();

        machine.transition(AppEvent::StartRecording).unwrap();
        machine.transition(AppEvent::StopRecording).unwrap();
        machine.transition(AppEvent::ValidAudio).unwrap();
        let snapshot = machine.transition(AppEvent::TranscriptionFailed).unwrap();
        assert_eq!(snapshot.status, AppStatus::Error);
        assert!(snapshot.error.is_some());

        // The explicit recovery transition used by both the toggle-hotkey
        // restart and the 5-second Error self-heal timer.
        let snapshot = machine.transition(AppEvent::ResetError).unwrap();
        assert_eq!(snapshot.status, AppStatus::Idle);
        assert!(snapshot.error.is_none());

        // A fresh dictation can start immediately after recovery.
        assert_eq!(
            machine.transition(AppEvent::StartRecording).unwrap().status,
            AppStatus::Recording
        );
    }

    #[test]
    fn pasting_flow_round_trips_back_to_ready_then_idle() {
        let mut machine = AppStateMachine::default();

        machine.transition(AppEvent::StartRecording).unwrap();
        machine.transition(AppEvent::StopRecording).unwrap();
        machine.transition(AppEvent::ValidAudio).unwrap();
        machine
            .transition(AppEvent::TranscriptionSucceeded)
            .unwrap();
        assert_eq!(machine.status(), &AppStatus::Ready);

        assert_eq!(
            machine.transition(AppEvent::StartPasting).unwrap().status,
            AppStatus::Pasting
        );
        assert_eq!(
            machine.transition(AppEvent::PasteCompleted).unwrap().status,
            AppStatus::Ready
        );
        assert_eq!(
            machine.transition(AppEvent::ReadyTimeout).unwrap().status,
            AppStatus::Idle
        );
    }

    #[test]
    fn pause_and_resume_round_trip() {
        let mut machine = AppStateMachine::default();

        assert_eq!(
            machine.transition(AppEvent::Pause).unwrap().status,
            AppStatus::Paused
        );
        // A paused app rejects recording until it resumes.
        assert!(machine.transition(AppEvent::StartRecording).is_err());
        assert_eq!(machine.status(), &AppStatus::Paused);

        assert_eq!(
            machine.transition(AppEvent::Resume).unwrap().status,
            AppStatus::Idle
        );
    }

    #[test]
    fn disconnect_recovery_reuses_the_normal_stop_to_transcribe_sequence() {
        // A microphone unplugged mid-recording drives the SAME state events a
        // manual stop with valid audio does (audio::disconnect_recording_for_app
        // -> stop_recording_with_reason -> transcribe). This locks that the
        // recovery sequence is a legal path the FSM accepts, so a dropped mic
        // lands the user back on Ready (then Idle) rather than wedged.
        let mut machine = AppStateMachine::default();

        machine.transition(AppEvent::StartRecording).unwrap();
        assert_eq!(machine.status(), &AppStatus::Recording);
        assert_eq!(
            machine.transition(AppEvent::StopRecording).unwrap().status,
            AppStatus::Stopping
        );
        assert_eq!(
            machine.transition(AppEvent::ValidAudio).unwrap().status,
            AppStatus::Transcribing
        );
        let snapshot = machine
            .transition(AppEvent::TranscriptionSucceeded)
            .unwrap();
        assert_eq!(snapshot.status, AppStatus::Ready);
        assert!(snapshot.error.is_none());
    }

    #[test]
    fn illegal_transition_is_rejected_and_leaves_state_unchanged() {
        let mut machine = AppStateMachine::default();

        // Stopping from Idle is not a valid edge.
        let error = machine.transition(AppEvent::StopRecording).unwrap_err();
        assert_eq!(error.from, AppStatus::Idle);
        assert_eq!(machine.status(), &AppStatus::Idle);

        // Pasting requires Ready first.
        assert!(machine.transition(AppEvent::StartPasting).is_err());
        assert_eq!(machine.status(), &AppStatus::Idle);
    }
}
