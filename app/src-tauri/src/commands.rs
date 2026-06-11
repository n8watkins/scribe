use std::sync::Mutex;

use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

use crate::{
    app_state::{AppEvent, AppStateMachine, AppStateSnapshot},
    audio::{self, MicrophoneInfo, RecordingResult, RecordingSessionInfo, StartRecordingRequest},
    db::Database,
    dictation::{self, DictationResult},
    error::CommandError,
    hotkeys::{self, HotkeyStatus},
    model_manager::{self, DownloadRegistry},
    models::ModelInfo,
    output::{self, OutputResult},
    settings::AppSettings,
    stats::BasicStats,
    transcript::{Transcript, TranscriptSearchResult},
};

pub struct BackendState {
    app_state: Mutex<AppStateMachine>,
    audio: Mutex<audio::AudioService>,
    db: Mutex<Database>,
    model_downloads: DownloadRegistry,
}

impl BackendState {
    pub fn new(db: Database, audio_temp_dir: std::path::PathBuf) -> Self {
        Self {
            app_state: Mutex::new(AppStateMachine::default()),
            audio: Mutex::new(audio::AudioService::new(audio_temp_dir)),
            db: Mutex::new(db),
            model_downloads: DownloadRegistry::default(),
        }
    }

    pub fn app_state(&self) -> Result<std::sync::MutexGuard<'_, AppStateMachine>, CommandError> {
        self.app_state
            .lock()
            .map_err(|_| CommandError::new("state_lock_poisoned", "Could not access app state."))
    }

    pub fn db(&self) -> Result<std::sync::MutexGuard<'_, Database>, CommandError> {
        self.db
            .lock()
            .map_err(|_| CommandError::new("database_lock_poisoned", "Could not access database."))
    }

    pub fn audio(&self) -> Result<std::sync::MutexGuard<'_, audio::AudioService>, CommandError> {
        self.audio.lock().map_err(|_| {
            CommandError::new("audio_lock_poisoned", "Could not access audio service.")
        })
    }

    pub fn model_downloads(&self) -> &DownloadRegistry {
        &self.model_downloads
    }

    pub fn transition_app_state(&self, event: AppEvent) -> Result<AppStateSnapshot, CommandError> {
        self.app_state()?
            .transition(event)
            .map_err(|error| CommandError::new("invalid_state_transition", error.to_string()))
    }
}

#[tauri::command]
pub fn get_app_state(
    state: tauri::State<'_, BackendState>,
) -> Result<AppStateSnapshot, CommandError> {
    Ok(state.app_state()?.snapshot())
}

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, BackendState>) -> Result<AppSettings, CommandError> {
    state.db()?.get_settings()
}

#[tauri::command]
pub fn update_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    settings: AppSettings,
) -> Result<AppSettings, CommandError> {
    settings
        .validate()
        .map_err(CommandError::invalid_settings)?;

    let previous = state.db()?.get_settings()?;

    let mut hotkey_failures = Vec::new();
    if previous.hotkeys != settings.hotkeys {
        hotkey_failures = hotkeys::replace_hotkeys(&app, &previous.hotkeys, &settings.hotkeys)?;
    }

    if let Err(error) = state.db()?.save_settings(&settings) {
        if previous.hotkeys != settings.hotkeys {
            let _ = hotkeys::replace_hotkeys(&app, &settings.hotkeys, &previous.hotkeys);
        }

        return Err(error);
    }

    hotkeys::emit_registration_failures(&app, &hotkey_failures);

    if previous.history_retention_days != settings.history_retention_days
        || (!previous.history_enabled && settings.history_enabled)
    {
        state
            .db()?
            .enforce_history_retention(settings.history_retention_days)?;
    }

    Ok(settings)
}

#[tauri::command]
pub fn get_last_transcript(
    state: tauri::State<'_, BackendState>,
) -> Result<Option<Transcript>, CommandError> {
    state.db()?.get_last_transcript()
}

#[tauri::command]
pub fn clear_last_transcript(state: tauri::State<'_, BackendState>) -> Result<(), CommandError> {
    state.db()?.clear_last_transcript()
}

#[tauri::command]
pub fn list_recent_transcripts(
    state: tauri::State<'_, BackendState>,
    limit: Option<u32>,
) -> Result<Vec<Transcript>, CommandError> {
    let limit = limit.unwrap_or(20).clamp(1, 100);
    state.db()?.list_recent_transcripts(limit)
}

#[tauri::command]
pub fn search_transcripts(
    state: tauri::State<'_, BackendState>,
    query: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<TranscriptSearchResult, CommandError> {
    let limit = limit.unwrap_or(20).clamp(1, 100);
    let offset = offset.unwrap_or_default();
    state
        .db()?
        .search_transcripts(query.as_deref(), limit, offset)
}

#[tauri::command]
pub fn get_transcript(
    state: tauri::State<'_, BackendState>,
    id: String,
) -> Result<Option<Transcript>, CommandError> {
    state.db()?.get_transcript_by_id(&id)
}

#[tauri::command]
pub fn update_transcript(
    state: tauri::State<'_, BackendState>,
    id: String,
    text: String,
) -> Result<Transcript, CommandError> {
    state.db()?.update_transcript(&id, &text)
}

#[tauri::command]
pub fn delete_transcript(
    state: tauri::State<'_, BackendState>,
    id: String,
) -> Result<(), CommandError> {
    state.db()?.delete_transcript(&id)
}

#[tauri::command]
pub fn clear_transcript_history(state: tauri::State<'_, BackendState>) -> Result<(), CommandError> {
    state.db()?.clear_transcript_history()
}

#[tauri::command]
pub fn get_basic_stats(state: tauri::State<'_, BackendState>) -> Result<BasicStats, CommandError> {
    state.db()?.get_basic_stats()
}

#[tauri::command]
pub fn refresh_basic_stats(
    state: tauri::State<'_, BackendState>,
) -> Result<BasicStats, CommandError> {
    state.db()?.get_basic_stats()
}

#[tauri::command]
pub fn get_hotkey_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
) -> Result<HotkeyStatus, CommandError> {
    let settings = state.db()?.get_settings()?;
    hotkeys::status(&app, &settings.hotkeys)
}

#[tauri::command]
pub fn rebind_hotkey(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    action: String,
    shortcut: String,
) -> Result<HotkeyStatus, CommandError> {
    let action = hotkeys::HotkeyAction::parse(&action)?;
    let mut settings = state.db()?.get_settings()?;
    let previous_hotkeys = settings.hotkeys.clone();
    let mut next_hotkeys = previous_hotkeys.clone();

    action.set_shortcut(&mut next_hotkeys, shortcut);
    hotkeys::validate_hotkeys(&next_hotkeys)?;
    let failures = hotkeys::replace_hotkeys(&app, &previous_hotkeys, &next_hotkeys)?;

    // When the binding being changed is the one that failed, restore the
    // previous set (other bindings stay registered either way) and surface
    // the error inline to the rebind UI.
    if let Some(failure) = failures.iter().find(|failure| failure.action == action) {
        let message = failure.message.clone();
        let _ = hotkeys::replace_hotkeys(&app, &next_hotkeys, &previous_hotkeys);
        return Err(CommandError::new("hotkey_registration_failed", message));
    }

    settings.hotkeys = next_hotkeys.clone();
    if let Err(error) = state.db()?.save_settings(&settings) {
        let _ = hotkeys::replace_hotkeys(&app, &next_hotkeys, &previous_hotkeys);
        return Err(error);
    }

    hotkeys::emit_registration_failures(&app, &failures);
    hotkeys::status(&app, &settings.hotkeys)
}

#[tauri::command]
pub fn reset_hotkeys_to_defaults(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
) -> Result<HotkeyStatus, CommandError> {
    let mut settings = state.db()?.get_settings()?;
    let previous_hotkeys = settings.hotkeys.clone();
    let next_hotkeys = AppSettings::default().hotkeys;

    hotkeys::validate_hotkeys(&next_hotkeys)?;
    let failures = hotkeys::replace_hotkeys(&app, &previous_hotkeys, &next_hotkeys)?;

    settings.hotkeys = next_hotkeys.clone();
    if let Err(error) = state.db()?.save_settings(&settings) {
        let _ = hotkeys::replace_hotkeys(&app, &next_hotkeys, &previous_hotkeys);
        return Err(error);
    }

    hotkeys::emit_registration_failures(&app, &failures);
    hotkeys::status(&app, &settings.hotkeys)
}

#[tauri::command]
pub fn open_dashboard(app: tauri::AppHandle) -> Result<(), CommandError> {
    crate::tray::open_dashboard(&app, None)
}

#[tauri::command]
pub fn list_microphones(app: tauri::AppHandle) -> Result<Vec<MicrophoneInfo>, CommandError> {
    audio::list_microphones_for_app(&app)
}

#[tauri::command]
pub fn start_recording(
    app: tauri::AppHandle,
    request: Option<StartRecordingRequest>,
) -> Result<RecordingSessionInfo, CommandError> {
    // UI-started recordings are toggle-style (no key is held), so silence
    // auto-stop applies.
    audio::start_recording_for_app(&app, request, true)
}

#[tauri::command]
pub fn stop_recording(app: tauri::AppHandle) -> Result<RecordingResult, CommandError> {
    audio::stop_recording_for_app(&app)
}

#[tauri::command]
pub fn cancel_recording(app: tauri::AppHandle) -> Result<(), CommandError> {
    audio::cancel_recording_for_app(&app)
}

#[tauri::command]
pub fn record_test_clip(
    app: tauri::AppHandle,
    duration_ms: Option<u64>,
) -> Result<RecordingResult, CommandError> {
    audio::record_test_clip_for_app(&app, duration_ms)
}

#[tauri::command]
pub fn get_test_clip_audio(app: tauri::AppHandle) -> Result<String, CommandError> {
    audio::get_test_clip_audio_for_app(&app)
}

#[tauri::command]
pub fn open_data_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
    let dir = app.path().app_data_dir().map_err(|error| {
        CommandError::new(
            "app_data_dir_unavailable",
            format!(
                "Could not locate LocalDictate app data directory. {}",
                error
            ),
        )
    })?;
    open_folder(&app, dir)
}

#[tauri::command]
pub fn open_models_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
    let dir = model_manager::models_dir(&app)?;
    open_folder(&app, dir)
}

fn open_folder(app: &tauri::AppHandle, dir: std::path::PathBuf) -> Result<(), CommandError> {
    std::fs::create_dir_all(&dir).map_err(|error| {
        CommandError::new(
            "open_folder_failed",
            format!("Could not create folder {}. {}", dir.display(), error),
        )
    })?;

    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|error| {
            CommandError::new(
                "open_folder_failed",
                format!("Could not open folder {}. {}", dir.display(), error),
            )
        })
}

#[tauri::command]
pub fn transcribe_recording(
    app: tauri::AppHandle,
    recording: RecordingResult,
) -> Result<DictationResult, CommandError> {
    dictation::transcribe_recording_for_app(&app, recording)
}

#[tauri::command]
pub fn paste_last_transcript(app: tauri::AppHandle) -> Result<OutputResult, CommandError> {
    output::paste_last_transcript(&app)
}

#[tauri::command]
pub fn copy_last_transcript(app: tauri::AppHandle) -> Result<OutputResult, CommandError> {
    output::copy_last_transcript(&app)
}

#[tauri::command]
pub fn paste_transcript(app: tauri::AppHandle, id: String) -> Result<OutputResult, CommandError> {
    output::paste_transcript(&app, &id)
}

#[tauri::command]
pub fn copy_transcript(app: tauri::AppHandle, id: String) -> Result<OutputResult, CommandError> {
    output::copy_transcript(&app, &id)
}

#[tauri::command]
pub fn list_models(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
) -> Result<Vec<ModelInfo>, CommandError> {
    let db = state.db()?;
    model_manager::list_models(&app, &db, state.model_downloads())
}

#[tauri::command]
pub fn download_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    let db = state.db()?;
    model_manager::download_model(&app, &db, state.model_downloads(), &model_id)
}

#[tauri::command]
pub fn cancel_model_download(
    state: tauri::State<'_, BackendState>,
    model_id: String,
) -> Result<(), CommandError> {
    if model_manager::request_cancel_download(state.model_downloads(), &model_id)? {
        return Ok(());
    }

    let db = state.db()?;
    model_manager::cancel_model_download(&db, state.model_downloads(), &model_id)
}

#[tauri::command]
pub fn retry_model_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    let db = state.db()?;
    model_manager::retry_model_download(&app, &db, state.model_downloads(), &model_id)
}

#[tauri::command]
pub fn delete_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    let db = state.db()?;
    model_manager::delete_model(&app, &db, &model_id)
}

#[tauri::command]
pub fn select_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    let db = state.db()?;
    model_manager::select_model(&app, &db, &model_id)
}
