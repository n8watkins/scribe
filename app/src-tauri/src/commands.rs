use std::sync::Mutex;

use serde::Serialize;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

use crate::{
    app_state::{AppEvent, AppStateMachine, AppStateSnapshot},
    audio::{self, MicrophoneInfo, RecordingResult, RecordingSessionInfo, StartRecordingRequest},
    db::Database,
    dictation::{self, DictationResult},
    error::CommandError,
    file_transcribe::{self, TranscribeFileResult},
    hotkeys::{self, HotkeyStatus},
    import::{ImportReport, PreparedImport, MAX_IMPORT_BYTES},
    model_manager::{self, DownloadRegistry},
    models::ModelInfo,
    output::{self, OutputResult},
    settings::AppSettings,
    stats::BasicStats,
    transcript::{Transcript, TranscriptSearchResult, TranscriptSort},
};

pub struct BackendState {
    app_state: Mutex<AppStateMachine>,
    audio: Mutex<audio::AudioService>,
    db: Mutex<Database>,
    model_downloads: DownloadRegistry,
    incremental: crate::incremental::Registry,
    github_device_flows: crate::github_oauth::DeviceFlowRegistry,
    /// Serializes GitHub note/transcript syncs so the debounce worker and a
    /// manual "Sync now" can never PUT the same daily file concurrently. The
    /// `()` payload is irrelevant — it's a plain mutual-exclusion latch.
    github_sync_lock: Mutex<()>,
    /// Selection captured when a voice Transform Selection recording starts,
    /// held until that recording's transcription consumes it.
    #[cfg(windows)]
    pending_transform: Mutex<Option<crate::selection_transform::CapturedSelection>>,
}

impl BackendState {
    pub fn new(db: Database, audio_temp_dir: std::path::PathBuf) -> Self {
        Self {
            app_state: Mutex::new(AppStateMachine::default()),
            audio: Mutex::new(audio::AudioService::new(audio_temp_dir)),
            db: Mutex::new(db),
            model_downloads: DownloadRegistry::default(),
            incremental: crate::incremental::Registry::default(),
            github_device_flows: crate::github_oauth::DeviceFlowRegistry::default(),
            github_sync_lock: Mutex::new(()),
            #[cfg(windows)]
            pending_transform: Mutex::new(None),
        }
    }

    /// Stashes the selection captured at the start of a voice transform. One-shot
    /// and overwritten on each transform trigger; a leftover entry (e.g. the
    /// recording was too short to transcribe) is only ever read by the next
    /// transform recording — ordinary dictation never consults it — so it can
    /// never be misapplied.
    #[cfg(windows)]
    pub fn set_pending_transform(&self, captured: crate::selection_transform::CapturedSelection) {
        if let Ok(mut slot) = self.pending_transform.lock() {
            *slot = Some(captured);
        }
    }

    /// Takes the pending transform selection, if any (consumed exactly once).
    #[cfg(windows)]
    pub fn take_pending_transform(&self) -> Option<crate::selection_transform::CapturedSelection> {
        self.pending_transform
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
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

    pub fn github_device_flows(&self) -> &crate::github_oauth::DeviceFlowRegistry {
        &self.github_device_flows
    }

    /// Acquires the GitHub-sync mutex, held for the duration of one sync entry
    /// point so syncs run one at a time. The guarded value is `()`, so a poisoned
    /// lock (a prior sync thread panicked while holding it) carries no corrupt
    /// state — recover with `into_inner` and carry on.
    pub fn github_sync_lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.github_sync_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Active incremental transcription sessions, keyed by recording session
    /// id.
    pub fn incremental(&self) -> &crate::incremental::Registry {
        &self.incremental
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

    // Apply launch-at-startup first: when the OS registration fails the new
    // value is never saved, so the UI toggle reverts.
    if previous.launch_at_startup != settings.launch_at_startup {
        apply_autostart(&app, settings.launch_at_startup)?;
    }

    let mut hotkey_failures = Vec::new();
    if previous.hotkeys != settings.hotkeys {
        hotkey_failures = hotkeys::replace_hotkeys(&app, &previous.hotkeys, &settings.hotkeys)?;
    }

    if let Err(error) = state.db()?.save_settings(&settings) {
        if previous.hotkeys != settings.hotkeys {
            let _ = hotkeys::replace_hotkeys(&app, &settings.hotkeys, &previous.hotkeys);
        }
        if previous.launch_at_startup != settings.launch_at_startup {
            let _ = apply_autostart(&app, previous.launch_at_startup);
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

    if previous.notes_retention_days != settings.notes_retention_days {
        state
            .db()?
            .enforce_notes_retention(settings.notes_retention_days)?;
    }

    Ok(settings)
}

/// Enables or disables the OS launch-at-startup registration. Shared by the
/// settings command and the setup-time reconciliation in `lib.rs`.
pub(crate) fn apply_autostart(app: &tauri::AppHandle, enabled: bool) -> Result<(), CommandError> {
    use tauri_plugin_autostart::ManagerExt;

    let autolaunch = app.autolaunch();
    let result = if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };

    result.map_err(|error| {
        CommandError::new(
            "autostart_failed",
            format!(
                "Could not {} launch at startup. {}",
                if enabled { "enable" } else { "disable" },
                error
            ),
        )
    })
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
// Tauri maps these flat parameters directly from the stable frontend command
// payload, so grouping them would be a breaking IPC contract change.
#[allow(clippy::too_many_arguments)]
pub fn search_transcripts(
    state: tauri::State<'_, BackendState>,
    query: Option<String>,
    notes_only: Option<bool>,
    from: Option<String>,
    to: Option<String>,
    sort: Option<TranscriptSort>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<TranscriptSearchResult, CommandError> {
    let limit = limit.unwrap_or(20).clamp(1, 100);
    let offset = offset.unwrap_or_default();
    state.db()?.search_transcripts(
        query.as_deref(),
        notes_only.unwrap_or(false),
        from.as_deref(),
        to.as_deref(),
        sort.unwrap_or_default(),
        limit,
        offset,
    )
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
pub fn clear_notes(state: tauri::State<'_, BackendState>) -> Result<(), CommandError> {
    state.db()?.clear_notes()
}

/// Loads the given transcripts, orders them oldest-first, and joins their text
/// with `separator` (default "\n\n"). Ids that don't resolve are skipped.
#[tauri::command]
pub fn combine_transcripts(
    state: tauri::State<'_, BackendState>,
    ids: Vec<String>,
    separator: Option<String>,
) -> Result<String, CommandError> {
    let separator = separator.unwrap_or_else(|| "\n\n".to_string());
    state.db()?.combine_transcripts(&ids, &separator)
}

/// Persists `text` as a new (non-note) history entry and makes it the Last
/// Transcript Buffer, mirroring how a dictation is saved. Returns the saved
/// transcript.
#[tauri::command]
pub fn save_combined_transcript(
    state: tauri::State<'_, BackendState>,
    text: String,
) -> Result<Transcript, CommandError> {
    let transcript = Transcript::new_last_buffer(text, None, None, None).ok_or_else(|| {
        CommandError::new(
            "empty_transcript",
            "Cannot save an empty combined transcript.",
        )
    })?;
    let db = state.db()?;
    let history_enabled = db.get_settings()?.history_enabled;
    db.save_last_transcript_with_history(&transcript, history_enabled)?;
    Ok(transcript)
}

/// Writes a transcript's text to a temp `.txt` file under the app cache dir and
/// opens it with the OS default text app.
#[tauri::command]
pub fn open_transcript_externally(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    id: String,
) -> Result<(), CommandError> {
    let transcript = state
        .db()?
        .get_transcript_by_id(&id)?
        .ok_or_else(|| CommandError::new("transcript_not_found", "Transcript was not found."))?;

    let export_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| {
            CommandError::new(
                "app_cache_dir_unavailable",
                format!("Could not locate Scribe cache directory. {}", error),
            )
        })?
        .join("scribe-export");
    std::fs::create_dir_all(&export_dir).map_err(|error| {
        CommandError::new(
            "transcript_export_failed",
            format!(
                "Could not create export folder {}. {}",
                export_dir.display(),
                error
            ),
        )
    })?;

    let path = export_dir.join(format!("{}.txt", id));
    std::fs::write(&path, transcript.text.as_bytes()).map_err(|error| {
        CommandError::new(
            "transcript_export_failed",
            format!(
                "Could not write transcript file {}. {}",
                path.display(),
                error
            ),
        )
    })?;

    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|error| {
            CommandError::new(
                "transcript_open_failed",
                format!(
                    "Could not open transcript file {}. {}",
                    path.display(),
                    error
                ),
            )
        })
}

/// Returns a transcript's saved audio clip as a base64 WAV string for
/// in-app playback. Errors with code "transcript_audio_missing" when the
/// transcript has no clip or the file is gone.
#[tauri::command]
pub fn get_transcript_audio(
    state: tauri::State<'_, BackendState>,
    id: String,
) -> Result<String, CommandError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let db = state.db()?;
    let transcript = match db.get_transcript_by_id(&id)? {
        Some(transcript) => Some(transcript),
        // With history disabled the dictation only lives in the Last
        // Transcript Buffer, which can still carry a clip.
        None => db.get_last_transcript()?.filter(|last| last.id == id),
    };

    let path = transcript
        .and_then(|transcript| transcript.audio_path)
        .ok_or_else(transcript_audio_missing)?;
    let bytes = std::fs::read(&path).map_err(|_| transcript_audio_missing())?;
    Ok(STANDARD.encode(bytes))
}

fn transcript_audio_missing() -> CommandError {
    CommandError::new(
        "transcript_audio_missing",
        "No audio clip is available for this transcript. It may have been deleted, or the dictation was recorded while clip saving was off.",
    )
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

/// Sets whether a single-shot bind (Toggle, Paste, Open Dashboard) acts on key
/// press or release. Rejected for Hold-to-Talk, which is push-to-talk. Re-runs
/// registration so the toggle watcher picks up the new edge, then persists.
#[tauri::command]
pub fn set_hotkey_trigger(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    action: String,
    trigger: crate::settings::TriggerEdge,
) -> Result<HotkeyStatus, CommandError> {
    let action = hotkeys::HotkeyAction::parse(&action)?;
    let mut settings = state.db()?.get_settings()?;
    let previous_hotkeys = settings.hotkeys.clone();
    let mut next_hotkeys = previous_hotkeys.clone();

    if !action.set_trigger(&mut next_hotkeys, trigger) {
        return Err(CommandError::new(
            "invalid_hotkey_trigger",
            "Hold-to-Talk is push-to-talk; it has no press/release option.",
        ));
    }

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
pub fn reset_hotkeys_to_defaults(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
) -> Result<HotkeyStatus, CommandError> {
    let mut settings = state.db()?.get_settings()?;
    let previous_hotkeys = settings.hotkeys.clone();
    // "Defaults" are flavor-specific: the Dev flavor resets to its
    // non-conflicting binds, stable resets to the production binds.
    let next_hotkeys = if crate::is_dev_flavor(&app) {
        crate::settings::HotkeySettings::dev_defaults()
    } else {
        AppSettings::default().hotkeys
    };

    hotkeys::validate_hotkeys(&next_hotkeys)?;
    let failures = hotkeys::replace_hotkeys(&app, &previous_hotkeys, &next_hotkeys)?;

    settings.hotkeys = next_hotkeys.clone();
    settings.dev_hotkeys_seeded = true;
    if let Err(error) = state.db()?.save_settings(&settings) {
        let _ = hotkeys::replace_hotkeys(&app, &next_hotkeys, &previous_hotkeys);
        return Err(error);
    }

    hotkeys::emit_registration_failures(&app, &failures);
    hotkeys::status(&app, &settings.hotkeys)
}

/// Loads the production (stable-flavor) hotkey defaults. Exposed for the Dev
/// flavor's Developer panel so you can switch Dev back to your real binds when
/// running it alone. Sets `dev_hotkeys_seeded` so the one-shot dev seeding
/// never re-applies the Dev binds on the next launch.
#[tauri::command]
pub fn load_production_hotkey_defaults(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
) -> Result<HotkeyStatus, CommandError> {
    let mut settings = state.db()?.get_settings()?;
    let previous_hotkeys = settings.hotkeys.clone();
    let next_hotkeys = AppSettings::default().hotkeys;

    hotkeys::validate_hotkeys(&next_hotkeys)?;
    let failures = hotkeys::replace_hotkeys(&app, &previous_hotkeys, &next_hotkeys)?;

    settings.hotkeys = next_hotkeys.clone();
    settings.dev_hotkeys_seeded = true;
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
    open_folder(&app, effective_data_dir(&app)?)
}

/// The directory Scribe treats as its data root: the user-chosen `data_dir`
/// when set, otherwise the OS app-data directory. Existing data is never moved
/// when this changes; only future data lands in the new location.
fn effective_data_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, CommandError> {
    if let Some(custom) = app
        .state::<BackendState>()
        .db()
        .ok()
        .and_then(|db| db.get_settings().ok())
        .and_then(|settings| settings.data_dir)
    {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return Ok(std::path::PathBuf::from(trimmed));
        }
    }

    app.path().app_data_dir().map_err(|error| {
        CommandError::new(
            "app_data_dir_unavailable",
            format!("Could not locate Scribe app data directory. {}", error),
        )
    })
}

/// Returns the current effective data directory as a display string, so the
/// Data & Privacy view can show the path prominently.
#[tauri::command]
pub fn get_data_dir(app: tauri::AppHandle) -> Result<String, CommandError> {
    Ok(effective_data_dir(&app)?.to_string_lossy().into_owned())
}

/// Opens a native folder picker so the user can choose a new data directory.
/// Returns the chosen absolute path, or None when the user cancels. Saving the
/// choice is the frontend's job (it writes `dataDir` via update_settings),
/// matching how the pill persists its position.
#[tauri::command]
pub fn pick_data_dir(app: tauri::AppHandle) -> Result<Option<String>, CommandError> {
    #[cfg(windows)]
    {
        let start = effective_data_dir(&app).unwrap_or_else(|_| std::path::PathBuf::from("."));
        let picked = rfd::FileDialog::new()
            .set_title("Choose Scribe data folder")
            .set_directory(&start)
            .pick_folder();
        Ok(picked.map(|path| path.to_string_lossy().into_owned()))
    }

    #[cfg(not(windows))]
    {
        let _ = app;
        Err(CommandError::new(
            "folder_picker_unsupported",
            "The folder picker is only available in the Windows build.",
        ))
    }
}

#[tauri::command]
pub fn open_models_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
    let dir = model_manager::models_dir(&app)?;
    open_folder(&app, dir)
}

/// The directory tauri-plugin-log writes rotating log files to (TargetKind::
/// LogDir resolves to app_log_dir()). Unlike the data/models folders this is
/// always the OS log dir and is not affected by the custom data_dir setting.
fn logs_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, CommandError> {
    app.path().app_log_dir().map_err(|error| {
        CommandError::new(
            "app_log_dir_unavailable",
            format!("Could not locate the Scribe logs directory. {}", error),
        )
    })
}

/// Opens the folder that holds Scribe's rotating local log files, so a user can
/// find and attach them to a bug report.
#[tauri::command]
pub fn open_logs_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
    let dir = logs_dir(&app)?;
    open_folder(&app, dir)
}

/// Returns the logs directory as a display string for the Data & Privacy view,
/// mirroring get_data_dir.
#[tauri::command]
pub fn get_logs_dir(app: tauri::AppHandle) -> Result<String, CommandError> {
    Ok(logs_dir(&app)?.to_string_lossy().into_owned())
}

/// The folder where audio from a dictation whose transcription failed is
/// quarantined so it isn't lost. This deliberately mirrors the location used by
/// dictation::quarantine_failed_recording — the OS app-data dir's `failed/`
/// subfolder, *not* the custom data_dir — so "Open" always lands on the real
/// quarantine folder rather than an empty one.
fn failed_recordings_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, CommandError> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("failed"))
        .map_err(|error| {
            CommandError::new(
                "app_data_dir_unavailable",
                format!("Could not locate the Scribe app data directory. {}", error),
            )
        })
}

/// Opens the folder that holds audio from dictations whose transcription failed,
/// so a user can recover the recording from disk (risk R2 follow-up to Fix E).
#[tauri::command]
pub fn open_failed_recordings_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
    let dir = failed_recordings_dir(&app)?;
    open_folder(&app, dir)
}

/// Returns the failed-recordings directory as a display string for the Data &
/// Privacy view, mirroring get_logs_dir.
#[tauri::command]
pub fn get_failed_recordings_dir(app: tauri::AppHandle) -> Result<String, CommandError> {
    Ok(failed_recordings_dir(&app)?.to_string_lossy().into_owned())
}

/// Reads the main window's current inner size and persists it as the default
/// window size, mirroring how the pill stores its position. The saved size is
/// restored on the next launch (see lib.rs setup).
#[tauri::command]
pub fn save_window_size(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
) -> Result<AppSettings, CommandError> {
    let window = app.get_webview_window("main").ok_or_else(|| {
        CommandError::new("window_unavailable", "The main window is not available.")
    })?;
    let size = window.inner_size().map_err(|error| {
        CommandError::new(
            "window_size_unavailable",
            format!("Could not read the window size. {}", error),
        )
    })?;

    let mut settings = state.db()?.get_settings()?;
    settings.window_width = Some(size.width as i32);
    settings.window_height = Some(size.height as i32);
    state.db()?.save_settings(&settings)?;
    log::info!("Saved default window size {}x{}", size.width, size.height);
    Ok(settings)
}

/// Runs the user's notes-analysis prompt over a saved transcript via the
/// configured local LLM server and stores the result on the row. Async so
/// the (possibly slow) local inference happens on a blocking worker. The
/// Notes view only offers this for notes, but any transcript id works.
#[tauri::command]
pub async fn analyze_note(
    app: tauri::AppHandle,
    transcript_id: String,
) -> Result<Transcript, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<BackendState>();
        // Read everything up front and drop the DB lock before the
        // long-running HTTP call.
        let (settings, transcript) = {
            let db = state.db()?;
            let settings = db.get_settings()?;
            let transcript = db.get_transcript_by_id(&transcript_id)?.ok_or_else(|| {
                CommandError::new(
                    "transcript_not_found",
                    format!(
                        "Transcript {} was not found in local history.",
                        transcript_id
                    ),
                )
            })?;
            (settings, transcript)
        };

        if !settings.notes_analysis_enabled {
            return Err(CommandError::new(
                "note_analysis_disabled",
                "Notes analysis is turned off in Settings.",
            ));
        }

        let outcome = crate::note_analysis::analyze_text(
            &settings.notes_analysis_endpoint,
            &settings.notes_analysis_model,
            &settings.notes_analysis_prompt,
            &transcript.text,
        )?;

        let saved =
            state
                .db()?
                .save_note_analysis(&transcript_id, &outcome.analysis, &outcome.model);
        saved
    })
    .await
    .map_err(|error| CommandError::new("note_analysis_failed", error.to_string()))?
}

/// Health check for the local LLM (notes analysis) server. Uses the supplied
/// `endpoint` when given — so the Settings card can test a typed-but-unsaved
/// value — otherwise the saved `notes_analysis_endpoint`. Returns
/// `reachable: false` for a down server rather than erroring; only an
/// inaccessible settings store produces an `Err`.
#[tauri::command]
pub fn llm_status(
    state: tauri::State<'_, BackendState>,
    endpoint: Option<String>,
) -> Result<crate::note_analysis::LlmStatus, CommandError> {
    let endpoint = match endpoint {
        Some(endpoint) => endpoint,
        None => state.db()?.get_settings()?.notes_analysis_endpoint,
    };
    Ok(crate::note_analysis::check_status(&endpoint))
}

#[tauri::command]
pub async fn check_for_update() -> Result<crate::update_check::UpdateCheckResult, CommandError> {
    tauri::async_runtime::spawn_blocking(crate::update_check::check_for_update)
        .await
        .map_err(|error| CommandError::new("update_check_failed", error.to_string()))?
}

#[tauri::command]
pub fn open_release_page(app: tauri::AppHandle, url: Option<String>) -> Result<(), CommandError> {
    let url = url.unwrap_or_else(|| crate::update_check::RELEASES_PAGE_URL.to_string());
    if !url.starts_with("https://github.com/n8watkins/scribe/") {
        return Err(CommandError::new(
            "invalid_url",
            "Refusing to open a non-release URL.",
        ));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| CommandError::new("open_url_failed", error.to_string()))
}

/// Connection / configuration status for the Settings → Sync panel.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubStatus {
    /// This build ships a valid GitHub App client id (device flow available).
    pub configured: bool,
    /// An access token is present in the keyring.
    pub connected: bool,
    /// The connected GitHub login (empty when not connected).
    pub username: String,
    /// The configured target repo ("owner/name", empty when unset).
    pub repo: String,
}

#[tauri::command]
pub fn github_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
) -> Result<GithubStatus, CommandError> {
    let settings = state.db()?.get_settings()?;
    let service = app.config().identifier.clone();
    Ok(GithubStatus {
        configured: crate::github_oauth::is_configured(),
        // A stored access token is the source of truth for "connected"; the
        // login is just for display.
        connected: crate::github_oauth::has_stored_token(&service),
        username: settings.github_account_login,
        repo: settings.github_repo,
    })
}

/// The device-flow payload the UI shows the user to authorize. The frontend
/// passes `deviceCode` + `intervalSecs` back into `github_device_poll`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubDeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in_secs: u64,
    pub interval_secs: u64,
}

/// Step 1 of the GitHub device flow: requests a device + user code, opens the
/// verification URL in the browser, and returns the code for the UI to show.
/// Does NOT block on polling — the frontend then calls `github_device_poll`.
#[tauri::command]
pub async fn github_device_start(app: tauri::AppHandle) -> Result<GithubDeviceStart, CommandError> {
    let opener_app = app.clone();
    let device = tauri::async_runtime::spawn_blocking(crate::github_oauth::request_device_code)
        .await
        .map_err(|error| CommandError::new("github_auth_failed", error.to_string()))??;

    // Best-effort: open the verification page so the user just types the code.
    let _ = opener_app
        .opener()
        .open_url(device.verification_uri.clone(), None::<&str>);

    app.state::<BackendState>()
        .github_device_flows()
        .start(&device.device_code);

    Ok(GithubDeviceStart {
        device_code: device.device_code,
        user_code: device.user_code,
        verification_uri: device.verification_uri,
        expires_in_secs: device.expires_in,
        interval_secs: device.interval,
    })
}

/// Step 2 of the GitHub device flow: polls until the user authorizes (or the
/// code expires / is denied), stores the token in the keyring, records the
/// connected login in settings, and returns the updated settings. Long-running.
#[tauri::command]
pub async fn github_device_poll(
    app: tauri::AppHandle,
    device_code: String,
    interval: u64,
) -> Result<AppSettings, CommandError> {
    let cancelled = app
        .state::<BackendState>()
        .github_device_flows()
        .get(&device_code)
        .ok_or_else(|| {
            CommandError::new(
                "github_auth_cancelled",
                "This GitHub sign-in attempt is no longer active.",
            )
        })?;
    let service = app.config().identifier.clone();
    let poll_service = service.clone();
    let poll_device_code = device_code.clone();
    let poll_result = tauri::async_runtime::spawn_blocking(move || {
        let credential =
            crate::github_oauth::poll_for_token(&poll_device_code, interval, cancelled.as_ref())?;
        crate::github_oauth::ensure_attempt_active(cancelled.as_ref())?;
        crate::github_oauth::store_credential(&poll_service, &credential)
    })
    .await
    .map_err(|error| CommandError::new("github_auth_failed", error.to_string()))?;
    app.state::<BackendState>()
        .github_device_flows()
        .finish(&device_code);
    poll_result?;

    // Fetch the login for display (best-effort: an empty login is fine).
    let login_service = service.clone();
    let login = tauri::async_runtime::spawn_blocking(move || {
        crate::github_oauth::fetch_login(&login_service)
    })
    .await
    .map_err(|error| CommandError::new("github_auth_failed", error.to_string()))?
    .unwrap_or_default();

    let state = app.state::<BackendState>();
    let mut settings = state.db()?.get_settings()?;
    settings.github_account_login = login;
    state.db()?.save_settings(&settings)?;
    Ok(settings)
}

/// Cancels an active GitHub device flow and prevents its token from being
/// persisted if authorization completes after the user cancels.
#[tauri::command]
pub fn github_device_cancel(
    state: tauri::State<'_, BackendState>,
    device_code: String,
) -> Result<(), CommandError> {
    state.github_device_flows().cancel(&device_code);
    Ok(())
}

/// Clears the stored GitHub token and fully resets the GitHub backup settings:
/// both backup toggles off, the connected login and target repo cleared. A later
/// reconnect then starts clean rather than silently resuming pushes to a stale
/// repo. Returns the updated settings.
#[tauri::command]
pub async fn github_disconnect(app: tauri::AppHandle) -> Result<AppSettings, CommandError> {
    let service = app.config().identifier.clone();
    tauri::async_runtime::spawn_blocking(move || crate::github_oauth::sign_out(&service))
        .await
        .map_err(|error| CommandError::new("github_auth_failed", error.to_string()))??;

    let state = app.state::<BackendState>();
    let mut settings = state.db()?.get_settings()?;
    settings.github_account_login = String::new();
    settings.github_sync_enabled = false;
    settings.github_sync_all_transcripts = false;
    settings.github_repo = String::new();
    state.db()?.save_settings(&settings)?;
    Ok(settings)
}

/// Pushes the current notes (is_note=1 only) to the configured GitHub repo as
/// dated Markdown, AND — when "Back up all transcripts" is on — the full-history
/// transcript dump, so a user who enabled only that backup isn't met with a
/// no-op. The daily files are regenerated from the DB, so this is safe to run
/// repeatedly. Both `collect_*` helpers self-gate on their setting and return an
/// empty report when off; the two reports are summed into one.
#[tauri::command]
pub async fn github_sync_now(
    app: tauri::AppHandle,
) -> Result<crate::github_backup::SyncReport, CommandError> {
    let service = app.config().identifier.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Serialize against the debounce worker so the same daily file is never
        // PUT concurrently. Held for the whole sync; recovers from a poisoned
        // lock (the guarded value is `()`).
        let state = app.state::<BackendState>();
        let _guard = state.github_sync_lock();
        let notes = crate::note_sync::collect_and_sync(&app, &service)?;
        let transcripts = crate::note_sync::collect_and_sync_all_transcripts(&app, &service)?;
        Ok(crate::github_backup::SyncReport {
            synced_notes: notes.synced_notes + transcripts.synced_notes,
            files_written: notes.files_written + transcripts.files_written,
        })
    })
    .await
    .map_err(|error| CommandError::new("github_sync_failed", error.to_string()))?
}

/// Exports transcripts to a local file the user picks via a native save dialog.
/// `scope` selects which rows ("all" | "notes" | "dictation"); `format` selects
/// the renderer ("markdown" | "csv" | "json"). Returns the saved absolute path,
/// or None when the user cancels the dialog. Unlike the GitHub backup this needs
/// no connected account — it is a purely local save.
#[tauri::command]
pub fn export_transcripts(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    scope: String,
    format: String,
) -> Result<Option<String>, CommandError> {
    // Fetch the rows for the scope. A large cap so a normal history exports in
    // one pass; notes/dictation reuse the same filtered queries the rest of the
    // app uses.
    let db = state.db()?;
    let transcripts = match scope.as_str() {
        "all" => {
            db.search_transcripts(
                None,
                false,
                None,
                None,
                TranscriptSort::OldestFirst,
                1_000_000,
                0,
            )?
            .transcripts
        }
        "notes" => {
            db.search_transcripts(
                None,
                true,
                None,
                None,
                TranscriptSort::OldestFirst,
                1_000_000,
                0,
            )?
            .transcripts
        }
        "dictation" => db.search_dictation_transcripts(1_000_000, 0)?.transcripts,
        other => {
            return Err(CommandError::new(
                "invalid_export_scope",
                format!(
                    "Unknown export scope \"{}\". Expected all, notes, or dictation.",
                    other
                ),
            ));
        }
    };
    drop(db);

    let (contents, extension) = match format.as_str() {
        "markdown" => (crate::export::to_markdown(&transcripts), "md"),
        "csv" => (crate::export::to_csv(&transcripts), "csv"),
        "json" => (crate::export::to_json(&transcripts), "json"),
        other => {
            return Err(CommandError::new(
                "invalid_export_format",
                format!(
                    "Unknown export format \"{}\". Expected markdown, csv, or json.",
                    other
                ),
            ));
        }
    };

    let default_name = format!(
        "scribe-export-{}.{}",
        chrono::Local::now().format("%Y-%m-%d"),
        extension
    );

    #[cfg(windows)]
    {
        let start = effective_data_dir(&app).unwrap_or_else(|_| std::path::PathBuf::from("."));
        let picked = rfd::FileDialog::new()
            .set_title("Export transcripts")
            .set_directory(&start)
            .set_file_name(&default_name)
            .save_file();
        let Some(path) = picked else {
            return Ok(None);
        };
        std::fs::write(&path, contents.as_bytes()).map_err(|error| {
            CommandError::new(
                "export_failed",
                format!("Could not write {}. {}", path.display(), error),
            )
        })?;
        Ok(Some(path.to_string_lossy().into_owned()))
    }

    #[cfg(not(windows))]
    {
        let _ = (app, contents, default_name);
        Err(CommandError::new(
            "save_dialog_unsupported",
            "The export save dialog is only available in the Windows build.",
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptImportPreview {
    pub path: String,
    pub file_name: String,
    pub total: u32,
    pub notes: u32,
    pub dictations: u32,
    pub conflicts: u32,
    pub audio_paths_removed: u32,
    pub metadata_corrected: u32,
    pub fingerprint: String,
}

struct LoadedTranscriptImport {
    prepared: PreparedImport,
    fingerprint: String,
}

/// Opens a native picker and validates a Scribe JSON export without changing
/// the database. The returned counts drive the explicit confirmation UI.
#[tauri::command]
pub fn preview_transcript_import(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
) -> Result<Option<TranscriptImportPreview>, CommandError> {
    #[cfg(windows)]
    {
        let start = effective_data_dir(&app).unwrap_or_else(|_| std::path::PathBuf::from("."));
        let picked = rfd::FileDialog::new()
            .set_title("Restore Scribe backup")
            .set_directory(&start)
            .add_filter("Scribe JSON export", &["json"])
            .pick_file();
        let Some(path) = picked else {
            return Ok(None);
        };
        let loaded = load_transcript_import(&path)?;
        let conflicts = state
            .db()?
            .count_existing_transcripts(&loaded.prepared.transcripts)?;
        Ok(Some(TranscriptImportPreview {
            path: path.to_string_lossy().into_owned(),
            file_name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Scribe backup.json")
                .to_string(),
            total: loaded.prepared.total,
            notes: loaded.prepared.notes,
            dictations: loaded.prepared.dictations,
            conflicts,
            audio_paths_removed: loaded.prepared.audio_paths_removed,
            metadata_corrected: loaded.prepared.metadata_corrected,
            fingerprint: loaded.fingerprint,
        }))
    }

    #[cfg(not(windows))]
    {
        let _ = (app, state);
        Err(CommandError::new(
            "open_dialog_unsupported",
            "The restore file picker is only available in the Windows build.",
        ))
    }
}

/// Revalidates the selected export, then imports every row in one database
/// transaction. Re-reading at confirmation time prevents a stale preview from
/// bypassing validation if the source file changed while the dialog was open.
#[tauri::command]
pub fn restore_transcript_import(
    state: tauri::State<'_, BackendState>,
    path: String,
    replace_existing: bool,
    expected_fingerprint: String,
) -> Result<ImportReport, CommandError> {
    let loaded = load_transcript_import(std::path::Path::new(&path))?;
    crate::import::verify_fingerprint(&loaded.fingerprint, &expected_fingerprint)?;
    state
        .db()?
        .restore_transcripts(&loaded.prepared.transcripts, replace_existing)
}

fn load_transcript_import(path: &std::path::Path) -> Result<LoadedTranscriptImport, CommandError> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return Err(CommandError::new(
            "invalid_import_file",
            "Choose a .json file exported by Scribe.",
        ));
    }
    let metadata = std::fs::metadata(path).map_err(|error| {
        CommandError::new(
            "import_read_failed",
            format!("Could not read {}. {error}", path.display()),
        )
    })?;
    if metadata.len() > MAX_IMPORT_BYTES {
        return Err(CommandError::new(
            "import_too_large",
            "The backup is larger than the 100 MB restore limit.",
        ));
    }
    let contents = std::fs::read(path).map_err(|error| {
        CommandError::new(
            "import_read_failed",
            format!("Could not read {}. {error}", path.display()),
        )
    })?;
    let fingerprint = crate::import::sha256_fingerprint(&contents);
    let prepared = crate::import::prepare_json(&contents)?;
    Ok(LoadedTranscriptImport {
        prepared,
        fingerprint,
    })
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

/// Returns None (null) when the recording was valid but contained no speech;
/// the frontend hears about it via "scribe:dictation-empty" and the
/// app-state event returning to Idle.
#[tauri::command]
pub fn transcribe_recording(
    app: tauri::AppHandle,
    recording: RecordingResult,
) -> Result<Option<DictationResult>, CommandError> {
    dictation::transcribe_recording_for_app(&app, recording)
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
pub fn save_text_file(path: String, text: String) -> Result<String, CommandError> {
    file_transcribe::save_text_file(&path, &text)
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

/// Selected-text transform: copy the text the user highlighted in the focused
/// app, rewrite it with the local LLM per `instruction`, and paste the result
/// back over the selection. The instruction is typed here (the v1 path); the
/// voice path will route a spoken instruction into the same engine.
///
/// Runs on a blocking worker because the local inference (and the capture
/// settle) can be slow; settings are read up front so no lock is held across
/// the HTTP call. Reuses `notes_analysis_endpoint` / `notes_analysis_model` as
/// the LLM server — no new settings.
#[tauri::command]
pub async fn transform_selection(
    app: tauri::AppHandle,
    instruction: String,
) -> Result<OutputResult, CommandError> {
    tauri::async_runtime::spawn_blocking(move || transform_selection_blocking(&app, &instruction))
        .await
        .map_err(|error| CommandError::new("selection_transform_failed", error.to_string()))?
}

#[cfg(windows)]
fn transform_selection_blocking(
    app: &tauri::AppHandle,
    instruction: &str,
) -> Result<OutputResult, CommandError> {
    use crate::selection_transform;

    if instruction.trim().is_empty() {
        return Err(CommandError::new(
            "instruction_empty",
            "No instruction was given. Type what to do with the selected text.",
        ));
    }

    let (endpoint, model) = {
        let state = app.state::<BackendState>();
        let settings = state.db()?.get_settings()?;
        (
            settings.notes_analysis_endpoint,
            settings.notes_analysis_model,
        )
    };

    // 1. Copy the current selection out of the focused app, remembering the
    //    user's prior clipboard so it can be restored.
    let captured = selection_transform::capture_selection()?;

    // 2. Rewrite it with the local LLM.
    let transformed =
        selection_transform::transform(&captured.selection, instruction, &endpoint, &model)?;

    // 3. Paste the result back over the still-selected original, then restore
    //    the pre-capture clipboard.
    selection_transform::apply_result(app, &transformed, &captured, true)
}

#[cfg(not(windows))]
fn transform_selection_blocking(
    _app: &tauri::AppHandle,
    _instruction: &str,
) -> Result<OutputResult, CommandError> {
    Err(CommandError::new(
        "selection_transform_unsupported",
        "Selected-text transform is currently implemented for Windows only.",
    ))
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
