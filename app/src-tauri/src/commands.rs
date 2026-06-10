use std::sync::Mutex;

use crate::{
    app_state::{AppEvent, AppStateMachine, AppStateSnapshot},
    audio::{self, MicrophoneInfo, RecordingResult, RecordingSessionInfo, StartRecordingRequest},
    db::Database,
    error::CommandError,
    hotkeys::{self, HotkeyStatus},
    settings::AppSettings,
    stats::BasicStats,
    transcript::Transcript,
};

pub struct BackendState {
    app_state: Mutex<AppStateMachine>,
    audio: Mutex<audio::AudioService>,
    db: Mutex<Database>,
}

impl BackendState {
    pub fn new(db: Database, audio_temp_dir: std::path::PathBuf) -> Self {
        Self {
            app_state: Mutex::new(AppStateMachine::default()),
            audio: Mutex::new(audio::AudioService::new(audio_temp_dir)),
            db: Mutex::new(db),
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

    if previous.hotkeys != settings.hotkeys {
        hotkeys::replace_hotkeys(&app, &previous.hotkeys, &settings.hotkeys)?;
    }

    if let Err(error) = state.db()?.save_settings(&settings) {
        if previous.hotkeys != settings.hotkeys {
            let _ = hotkeys::replace_hotkeys(&app, &settings.hotkeys, &previous.hotkeys);
        }

        return Err(error);
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
pub fn get_basic_stats(state: tauri::State<'_, BackendState>) -> Result<BasicStats, CommandError> {
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
    hotkeys::replace_hotkeys(&app, &previous_hotkeys, &next_hotkeys)?;

    settings.hotkeys = next_hotkeys.clone();
    if let Err(error) = state.db()?.save_settings(&settings) {
        let _ = hotkeys::replace_hotkeys(&app, &next_hotkeys, &previous_hotkeys);
        return Err(error);
    }

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
    hotkeys::replace_hotkeys(&app, &previous_hotkeys, &next_hotkeys)?;

    settings.hotkeys = next_hotkeys.clone();
    if let Err(error) = state.db()?.save_settings(&settings) {
        let _ = hotkeys::replace_hotkeys(&app, &next_hotkeys, &previous_hotkeys);
        return Err(error);
    }

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
    audio::start_recording_for_app(&app, request)
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
