use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    app_state::{AppEvent, AppStateSnapshot, AppStatus},
    audio::{RecordingResult, RecordingResultStatus},
    commands::BackendState,
    error::CommandError,
    incremental::{self, SessionHandle},
    model_manager, output,
    settings::Language,
    transcript::Transcript,
    tray,
    whisper::{WhisperRequest, WhisperTranscription},
    whisper_server::WarmTranscriber,
};

/// How long the app stays in Error before self-healing back to Idle.
const ERROR_RECOVERY_DELAY: Duration = Duration::from_secs(5);

/// Info-toast message for a dictation that produced no text.
pub(crate) const EMPTY_DICTATION_MESSAGE: &str = "Nothing heard — no text to insert.";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationStatus {
    Saved,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationResult {
    pub session_id: String,
    pub status: DictationStatus,
    pub transcript: Transcript,
    pub model_id: String,
    pub duration_ms: u64,
    pub transcription_latency_ms: u32,
}

/// Transcribes a finished recording. Returns Ok(None) when the recording was
/// valid but Whisper heard nothing (a benign outcome: the app returns to
/// Idle and "scribe:dictation-empty" is emitted, no transcript is
/// saved). On any error, the state machine is never left stranded in
/// Transcribing: it fails over to Error, which self-heals to Idle.
pub fn transcribe_recording_for_app(
    app: &AppHandle,
    recording: RecordingResult,
) -> Result<Option<DictationResult>, CommandError> {
    // Take ownership of the incremental session (if any) up front so it is
    // consumed exactly once, whatever happens below. `stopped` anchors the
    // stop-to-final-text latency measurement.
    let stopped = Instant::now();
    let incremental = app
        .state::<BackendState>()
        .incremental()
        .take(&recording.session_id);

    let result = transcribe_recording_checked(app, &recording, incremental, stopped);
    if result.is_err() {
        // Whatever failed (validation, model lookup, whisper, the database),
        // a Transcribing state must not be stranded: fail over to Error,
        // which self-heals back to Idle. No-op when already past
        // Transcribing.
        transition_after_failure(app);
    }
    result
}

fn transcribe_recording_checked(
    app: &AppHandle,
    recording: &RecordingResult,
    incremental: Option<SessionHandle>,
    stopped: Instant,
) -> Result<Option<DictationResult>, CommandError> {
    validate_recording_result(recording)?;
    let wav_path = PathBuf::from(recording.wav_path.as_deref().ok_or_else(|| {
        CommandError::new(
            "recording_wav_missing",
            "Recording completed but did not include a WAV path. Record again.",
        )
    })?);
    let cleanup = WavCleanup::new(wav_path.clone());

    let result = transcribe_recording_inner(app, recording, wav_path, incremental, stopped);
    cleanup.remove();
    result
}

fn transcribe_recording_inner(
    app: &AppHandle,
    recording: &RecordingResult,
    wav_path: PathBuf,
    incremental: Option<SessionHandle>,
    stopped: Instant,
) -> Result<Option<DictationResult>, CommandError> {
    let state = app.state::<BackendState>();
    let settings = state.db()?.get_settings()?;
    let (model_id, model_path) = {
        let db = state.db()?;
        model_manager::selected_model_path(app, &db)?
    };
    let language = whisper_language(&settings.language);

    let whisper_result = match incremental
        .and_then(|handle| collect_incremental_transcription(app, recording, handle, stopped))
    {
        Some(assembled) => Ok(assembled),
        None => {
            log::info!(
                "Transcription started for session {} (model {}, recording {} ms)",
                recording.session_id,
                model_id,
                recording.duration_ms
            );
            // The warm whisper-server transcriber falls back to whisper-cli
            // internally when the server path is unavailable.
            app.state::<WarmTranscriber>().transcribe(
                app,
                WhisperRequest {
                    model_path,
                    wav_path: wav_path.clone(),
                    language: language.clone(),
                    translate: settings.translate_to_english,
                    vocabulary_prompt: settings.vocabulary_prompt.clone(),
                },
            )
        }
    };

    let whisper_result = match whisper_result {
        Ok(result) => result,
        Err(error) => {
            log::error!(
                "Transcription failed for session {} (model {}): {}",
                recording.session_id,
                model_id,
                error.message
            );
            // The Err propagates to transcribe_recording_for_app, which
            // performs the Transcribing -> Error failover.
            return Err(error);
        }
    };

    log::info!(
        "Transcription finished for session {} (model {}, latency {} ms)",
        recording.session_id,
        model_id,
        whisper_result.latency_ms
    );

    // Apply the user's deterministic "say X -> get Y" replacements to the final
    // Whisper text. Both the incremental and full-clip paths land in
    // `whisper_result.text`, so doing it here covers both. (The file-transcribe
    // path is intentionally left untouched.) An empty replacements list returns
    // the text unchanged.
    let mut final_text =
        crate::text_replace::apply(&whisper_result.text, &settings.text_replacements);

    // A voice Transform Selection recording: the transcribed text is the user's
    // spoken *instruction*, not dictation. Route it to the transform engine —
    // rewrite the selection captured when the recording started, paste it back
    // in place — and skip the normal cleanup / save / paste path entirely.
    #[cfg(windows)]
    if recording.is_transform {
        return finish_selection_transform(app, &settings, &final_text);
    }

    // Optional local-LLM polish (filler removal, punctuation/casing, light
    // formatting) before the text is saved or pasted. Non-blocking with a
    // raw-text fallback: cleanup() returns the original text on any
    // error/timeout, so a slow or offline LLM never stalls or blanks the
    // dictation. Replacements run first so the LLM also sees the corrected
    // wording. (The file-transcribe path is intentionally left untouched.)
    if settings.dictation_cleanup_enabled {
        final_text = crate::dictation_cleanup::cleanup(
            &final_text,
            settings.dictation_cleanup_mode,
            &settings.dictation_cleanup_prompt,
            &settings.notes_analysis_endpoint,
            &settings.notes_analysis_model,
        );
    }

    // Empty or whitespace-only text is a benign outcome, not an error (e.g.
    // the user tapped the toggle hotkey on and immediately off). Both the
    // incremental path (whose assembled text lands in `whisper_result`, or
    // which fell back to the full clip) and the full-clip path funnel through
    // this single check. Nothing is saved; the previous Last Transcript
    // Buffer is preserved.
    let Some(mut transcript) = Transcript::new_last_buffer(
        final_text,
        Some(recording.duration_ms.min(u32::MAX as u64) as u32),
        Some(model_id.clone()),
        Some(language),
    ) else {
        log::info!(
            "Transcription empty for session {} (model {}): nothing heard, returning to Idle",
            recording.session_id,
            model_id
        );
        transition_after_empty(app);
        emit_dictation_empty(app);
        return Ok(None);
    };

    transcript.output_mode = Some(settings.output_mode.clone());
    transcript.paste_method = Some(settings.paste_method.clone());
    transcript.transcription_latency_ms = Some(whisper_result.latency_ms);
    transcript.is_note = recording.is_note;

    // Claim the recording WAV before WavCleanup runs: the move removes the
    // source, so the cleanup's remove becomes a no-op. A clip failure must
    // never fail the dictation itself.
    if settings.save_audio_clips {
        match save_audio_clip(app, &wav_path, &transcript.id) {
            Ok(clip_path) => transcript.audio_path = Some(clip_path.to_string_lossy().into_owned()),
            Err(error) => log::warn!(
                "Could not save audio clip for transcript {}: {}",
                transcript.id,
                error.message
            ),
        }
    }

    let save_result = state.db().and_then(|db| {
        if transcript.is_note {
            db.save_note_transcript(&transcript)
        } else {
            db.save_last_transcript_with_history(&transcript, settings.history_enabled)
        }
    });
    if let Err(error) = save_result {
        // A clip whose transcript was never saved would be orphaned forever.
        if let Some(path) = transcript.audio_path.as_deref() {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }

    // A saved transcript flows to Google Drive automatically when a relevant
    // Drive backup is on: notes go to the curated notes log (drive_sync_enabled),
    // and ordinary dictations go to the transcript backup
    // (drive_sync_all_transcripts). The worker debounces and uploads off this
    // thread, so it never delays the dictation result; it re-checks the settings
    // itself and runs only the backups that apply.
    let notify_drive = if transcript.is_note {
        settings.drive_sync_enabled
    } else {
        settings.drive_sync_all_transcripts
    };
    if notify_drive {
        if let Some(worker) = app.try_state::<crate::note_sync::DriveSyncWorker>() {
            worker.notify();
        }
    }

    transition_after_success(app);

    let result = DictationResult {
        session_id: recording.session_id.clone(),
        status: DictationStatus::Saved,
        transcript: transcript.clone(),
        model_id,
        duration_ms: recording.duration_ms,
        transcription_latency_ms: whisper_result.latency_ms,
    };

    let _ = app.emit("scribe:dictation-transcribed", &result);
    // Output fires exactly once here, on the final assembled transcript — never
    // per partial. Any "streaming" feel comes from the DirectInsert keystroke
    // injection, not from multiple output passes.
    // Notes are for the archive, not the cursor: never auto-paste them.
    if !transcript.is_note {
        if let Err(error) = output::handle_transcription_output(app, &transcript, &settings) {
            output::emit_output_failed(app, transcript.id.clone(), &error);
        }
    }
    Ok(Some(result))
}

/// Completes a voice Transform Selection: the `instruction` is the transcribed
/// spoken command; the selection was captured (and stashed on `BackendState`)
/// when the recording started. Runs the local-LLM transform and pastes the
/// result over the still-active selection. Nothing is saved as a transcript and
/// the normal dictation output never runs. The state machine always leaves
/// Transcribing (success or a toasted, recoverable failure — never a wedged
/// Error for a routine LLM miss).
#[cfg(windows)]
fn finish_selection_transform(
    app: &AppHandle,
    settings: &crate::settings::AppSettings,
    instruction: &str,
) -> Result<Option<DictationResult>, CommandError> {
    let state = app.state::<BackendState>();

    let Some(captured) = state.take_pending_transform() else {
        log::warn!("Transform recording finished but no selection was captured.");
        transition_after_empty(app);
        let _ = app.emit(
            "scribe:selection-transform-failed",
            "No selected text was captured. Highlight text, then trigger Transform Selection.",
        );
        return Ok(None);
    };

    let outcome = crate::selection_transform::transform(
        &captured.selection,
        instruction,
        &settings.notes_analysis_endpoint,
        &settings.notes_analysis_model,
    )
    .and_then(|text| {
        crate::selection_transform::apply_result(app, &text, &captured, true).map(|_| text)
    });

    // Done either way: return to Idle. A failure toasts rather than parking the
    // app in Error, since a missed LLM call is routine and recoverable.
    match outcome {
        Ok(text) => {
            transition_after_success(app);
            log::info!("Selection transform applied ({} chars).", text.len());
            let _ = app.emit("scribe:selection-transformed", &text);
        }
        Err(error) => {
            transition_after_empty(app);
            log::warn!("Selection transform failed: {}", error.message);
            let _ = app.emit("scribe:selection-transform-failed", &error.message);
        }
    }
    Ok(None)
}

/// Waits (bounded) for the incremental coordinator's assembled text and turns
/// it into the dictation transcription, with the latency measured from stop.
/// Returns None whenever the full-clip transcription path should run instead;
/// by the time None is returned the coordinator has definitively finished,
/// failed, or been cancelled, so the fallback never races a segment job for
/// the WarmTranscriber (which is serialized internally anyway).
fn collect_incremental_transcription(
    app: &AppHandle,
    recording: &RecordingResult,
    handle: SessionHandle,
    stopped: Instant,
) -> Option<WhisperTranscription> {
    match handle.wait(incremental::RESULT_TIMEOUT) {
        Ok(assembled) if !assembled.text.is_empty() => {
            let latency_ms = stopped.elapsed().as_millis().min(u32::MAX as u128) as u32;
            log::info!(
                "Incremental transcription assembled {} segment(s) for session {} ({} ms stop-to-text)",
                assembled.segments,
                recording.session_id,
                latency_ms
            );
            incremental::emit_partial_transcript(
                app,
                &recording.session_id,
                &assembled.text,
                assembled.segments,
                true,
            );
            Some(WhisperTranscription {
                text: assembled.text,
                latency_ms,
            })
        }
        Ok(_) => {
            // Zero segments produced any text (e.g. speech never crossed the
            // segmenter threshold): let the full clip decide.
            log::warn!(
                "Incremental transcription produced no text for session {}; falling back to full-clip transcription",
                recording.session_id
            );
            None
        }
        Err(reason) => {
            log::warn!(
                "Incremental transcription unavailable for session {} ({}); falling back to full-clip transcription",
                recording.session_id,
                reason
            );
            // On timeout the coordinator may still be working: tell it to
            // discard everything before the fallback transcription starts.
            handle.cancel();
            None
        }
    }
}

fn validate_recording_result(recording: &RecordingResult) -> Result<(), CommandError> {
    if !matches!(
        recording.status,
        RecordingResultStatus::Completed | RecordingResultStatus::TimedOut
    ) {
        return Err(CommandError::new(
            "recording_not_transcribable",
            "Only completed or timed-out recordings with a WAV file can be transcribed.",
        ));
    }

    if recording
        .wav_path
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Err(CommandError::new(
            "recording_wav_missing",
            "Recording completed but did not include a WAV path. Record again.",
        ));
    }

    Ok(())
}

/// The `--language` argument whisper.cpp expects for the selected language:
/// the stored ISO-639-1 code, or "auto" for auto-detection. An empty/blank
/// stored value falls back to "auto" so whisper never receives an empty arg.
pub(crate) fn whisper_language(language: &Language) -> String {
    let code = language.code().trim();
    if code.is_empty() {
        crate::settings::LANGUAGE_AUTO.to_string()
    } else {
        code.to_string()
    }
}

fn transition_after_success(app: &AppHandle) {
    transition_if_transcribing(app, AppEvent::TranscriptionSucceeded);
}

fn transition_after_failure(app: &AppHandle) {
    transition_if_transcribing(app, AppEvent::TranscriptionFailed);
}

fn transition_after_empty(app: &AppHandle) {
    transition_if_transcribing(app, AppEvent::TranscriptionEmpty);
}

fn transition_if_transcribing(app: &AppHandle, event: AppEvent) {
    let state = app.state::<BackendState>();
    let Ok(snapshot) = state.app_state().map(|state| state.snapshot()) else {
        return;
    };

    if snapshot.status != AppStatus::Transcribing {
        return;
    }

    let Ok(snapshot) = state.transition_app_state(event) else {
        return;
    };

    if snapshot.status == AppStatus::Error {
        // Error must never wedge the app: schedule the return to Idle.
        schedule_error_recovery(app, snapshot.updated_at);
    }

    emit_state_snapshot(app, &snapshot);
}

#[derive(Debug, Clone, Serialize)]
struct DictationEmptyPayload {
    message: String,
}

/// Tells the frontend that a dictation finished without producing any text,
/// so it can show a gentle info toast instead of an error.
pub(crate) fn emit_dictation_empty(app: &AppHandle) {
    let _ = app.emit(
        "scribe:dictation-empty",
        DictationEmptyPayload {
            message: EMPTY_DICTATION_MESSAGE.to_string(),
        },
    );
}

/// Self-heals the Error state: after ERROR_RECOVERY_DELAY the app returns to
/// Idle via the normal ResetError transition and "scribe:app-state"
/// event. `entered_at` is the timestamp of the Error snapshot being healed;
/// when any newer state (even a newer Error) has replaced it, the timer
/// expires without doing anything, so it can never clobber later activity.
fn schedule_error_recovery(app: &AppHandle, entered_at: DateTime<Utc>) {
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(ERROR_RECOVERY_DELAY);

        let Some(state) = app.try_state::<BackendState>() else {
            return;
        };
        // Check-and-transition under a single lock so the recovery can never
        // race a state change that happens between the check and the reset.
        let snapshot = {
            let Ok(mut machine) = state.app_state() else {
                return;
            };
            if machine.status() != &AppStatus::Error || machine.snapshot().updated_at != entered_at
            {
                return;
            }
            let Ok(snapshot) = machine.transition(AppEvent::ResetError) else {
                return;
            };
            snapshot
        };

        log::info!(
            "Error state self-healed to Idle after {:?}",
            ERROR_RECOVERY_DELAY
        );
        emit_state_snapshot(&app, &snapshot);
        tray::update_tray_status(&app, snapshot.status.clone());
    });
}

fn emit_state_snapshot(app: &AppHandle, snapshot: &AppStateSnapshot) {
    let _ = app.emit("scribe:app-state", snapshot);
}

/// Permanent home of saved dictation clips: app_data_dir/clips/{id}.wav.
fn clips_dir(app: &AppHandle) -> Result<PathBuf, CommandError> {
    let dir = app.path().app_data_dir().map_err(|error| {
        CommandError::new(
            "app_data_dir_unavailable",
            format!(
                "Could not locate Scribe app data directory. {}",
                error
            ),
        )
    })?;
    Ok(dir.join("clips"))
}

/// Moves the finished recording WAV into the clips directory. fs::rename
/// cannot cross volumes (the recording temp dir may live on a different one
/// than app data), so it falls back to copy + delete.
fn save_audio_clip(
    app: &AppHandle,
    wav_path: &Path,
    transcript_id: &str,
) -> Result<PathBuf, CommandError> {
    let dir = clips_dir(app)?;
    fs::create_dir_all(&dir).map_err(|error| {
        CommandError::new(
            "audio_clip_save_failed",
            format!("Could not create clips folder {}. {}", dir.display(), error),
        )
    })?;

    let target = dir.join(format!("{}.wav", transcript_id));
    if fs::rename(wav_path, &target).is_err() {
        fs::copy(wav_path, &target).map_err(|error| {
            CommandError::new(
                "audio_clip_save_failed",
                format!("Could not save audio clip {}. {}", target.display(), error),
            )
        })?;
        // The source delete may fail (e.g. a lingering handle); WavCleanup
        // retries it right after.
        let _ = fs::remove_file(wav_path);
    }
    Ok(target)
}

/// Deletes the temp recording WAV once transcription is done. When the
/// success path saved the WAV as a clip, the move already consumed the
/// source and this remove is a harmless no-op.
struct WavCleanup {
    path: PathBuf,
}

impl WavCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn remove(&self) {
        let _ = fs::remove_file(&self.path);
    }
}
