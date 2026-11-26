//! Commands of the `audio` domain (audio, dictation, dictation_state,
//! dictation_worker, incremental, sounds, file_transcribe, self_test,
//! test_audio).

use super::off_main;
use crate::{
    audio::{self, MicrophoneInfo, RecordingResult, RecordingSessionInfo, StartRecordingRequest},
    dictation_worker::{self, DictationCommand, Outcome},
    error::CommandError,
    file_transcribe::{self, TranscribeFileResult},
    output::OutputResult,
};

#[tauri::command]
pub async fn list_microphones(app: tauri::AppHandle) -> Result<Vec<MicrophoneInfo>, CommandError> {
    off_main(app, audio::list_microphones_for_app).await
}

/// Starts a dictation from the dashboard. Runs on the dictation worker, so it
/// is ordered with every hotkey, tray, and pill action.
#[tauri::command]
pub async fn start_recording(
    app: tauri::AppHandle,
    request: Option<StartRecordingRequest>,
) -> Result<RecordingSessionInfo, CommandError> {
    // UI-started recordings are toggle-style (no key is held), so silence
    // auto-stop applies.
    let command = DictationCommand::Start {
        request,
        allow_auto_stop: true,
    };
    match dictation_worker::request(app, command).await? {
        Outcome::Started(info) => Ok(info),
        _ => Err(CommandError::new(
            "recording_unavailable",
            "The recording could not be started.",
        )),
    }
}

/// Stops the dictation and transcribes it: the one stop path the dashboard and
/// the pill share with the hotkeys and the tray. Resolves once the text is
/// out; the result itself arrives through the usual dictation events. A stop
/// while nothing is recording does nothing.
#[tauri::command]
pub async fn stop_dictation(app: tauri::AppHandle) -> Result<(), CommandError> {
    dictation_worker::request(app, DictationCommand::Stop).await?;
    Ok(())
}

#[tauri::command]
pub async fn cancel_recording(app: tauri::AppHandle) -> Result<(), CommandError> {
    dictation_worker::request(app, DictationCommand::Cancel).await?;
    Ok(())
}

#[tauri::command]
pub async fn record_test_clip(
    app: tauri::AppHandle,
    duration_ms: Option<u64>,
) -> Result<RecordingResult, CommandError> {
    off_main(app, move |app| {
        audio::record_test_clip_for_app(app, duration_ms)
    })
    .await
}

#[tauri::command]
pub async fn get_test_clip_audio(app: tauri::AppHandle) -> Result<String, CommandError> {
    off_main(app, audio::get_test_clip_audio_for_app).await
}

/// Transcribes a user-picked audio/video file. Async so the (possibly
/// minutes-long) whisper-cli run happens on a blocking worker, never the
/// main thread.
#[tauri::command]
pub async fn transcribe_file(
    app: tauri::AppHandle,
    path: String,
) -> Result<TranscribeFileResult, CommandError> {
    tauri::async_runtime::spawn_blocking(move || file_transcribe::transcribe_file(&app, &path))
        .await
        .map_err(|error| {
            CommandError::new(
                "whisper_transcription_failed",
                format!("File transcription task failed. {}", error),
            )
        })?
}

/// Writes transcribed text to `<source>.txt` next to the source file and
/// returns the path written.
#[tauri::command]
pub async fn save_text_file(
    app: tauri::AppHandle,
    path: String,
    text: String,
) -> Result<String, CommandError> {
    off_main(app, move |_| file_transcribe::save_text_file(&path, &text)).await
}

/// Pastes the Last Transcript Buffer. Runs on the dictation worker so it can
/// never paste over a dictation that is being output.
#[tauri::command]
pub async fn paste_last_transcript(app: tauri::AppHandle) -> Result<OutputResult, CommandError> {
    match dictation_worker::request(app, DictationCommand::PasteLast).await? {
        Outcome::Output(result) => Ok(result),
        _ => Err(CommandError::new(
            "paste_failed",
            "The last transcript could not be pasted.",
        )),
    }
}
