use std::sync::Mutex;

use serde::Serialize;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

use crate::{
    app_state::{AppEvent, AppStateMachine, AppStateSnapshot},
    audio::{self, MicrophoneInfo, RecordingResult, RecordingSessionInfo, StartRecordingRequest},
    db::Database,
    dictation_worker::{self, DictationCommand, Outcome},
    error::CommandError,
    file_transcribe::{self, TranscribeFileResult},
    hotkeys::{self, HotkeyStatus},
    import::{ImportReport, PreparedImport, MAX_IMPORT_BYTES},
    model_manager::{self, DownloadRegistry},
    models::ModelInfo,
    output::{self, OutputResult},
    settings::AppSettings,
    settings_store,
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
    /// Serializes every settings write from read to save, so concurrent
    /// writers never lose each other's fields (see `settings_store`).
    settings_writes: Mutex<()>,
    /// Selection captured when a voice Transform Selection recording starts,
    /// held until that recording's transcription consumes it.
    pending_transform: Mutex<Option<crate::selection_transform::CapturedSelection>>,
    /// The folders Scribe writes to, for cleanup that touches files.
    dirs: crate::data_hygiene::DataDirs,
    /// The window that had focus when the current dictation started, so its
    /// text goes back there and nowhere else. Taken once by the output step.
    paste_target: Mutex<Option<output::PasteTarget>>,
    /// The latest dictation when saving it to the database failed. It was
    /// still pasted; this keeps it reachable for Paste last until the next
    /// successful save.
    unsaved_last_transcript: Mutex<Option<Transcript>>,
}

impl BackendState {
    pub fn new(
        db: Database,
        audio_temp_dir: std::path::PathBuf,
        dirs: crate::data_hygiene::DataDirs,
    ) -> Self {
        Self {
            dirs,
            app_state: Mutex::new(AppStateMachine::default()),
            audio: Mutex::new(audio::AudioService::new(audio_temp_dir)),
            db: Mutex::new(db),
            model_downloads: DownloadRegistry::default(),
            incremental: crate::incremental::Registry::default(),
            github_device_flows: crate::github_oauth::DeviceFlowRegistry::default(),
            github_sync_lock: Mutex::new(()),
            settings_writes: Mutex::new(()),
            pending_transform: Mutex::new(None),
            paste_target: Mutex::new(None),
            unsaved_last_transcript: Mutex::new(None),
        }
    }

    /// Records where the dictation that is starting should paste. `None`
    /// means "no window to hold it to" (Scribe itself had focus).
    pub fn set_paste_target(&self, target: Option<output::PasteTarget>) {
        if let Ok(mut slot) = self.paste_target.lock() {
            *slot = target;
        }
    }

    /// Takes the paste target recorded at dictation start (consumed once).
    pub fn take_paste_target(&self) -> Option<output::PasteTarget> {
        self.paste_target
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    /// Remembers (or, with `None`, forgets) a dictation that could not be
    /// saved.
    pub fn set_unsaved_last_transcript(&self, transcript: Option<Transcript>) {
        if let Ok(mut slot) = self.unsaved_last_transcript.lock() {
            *slot = transcript;
        }
    }

    /// The Last Transcript Buffer: an unsaved dictation when the last save
    /// failed (it is newer than anything in the database), else the saved one.
    pub fn last_transcript(&self) -> Result<Option<Transcript>, CommandError> {
        let unsaved = self
            .unsaved_last_transcript
            .lock()
            .ok()
            .and_then(|slot| slot.clone());
        match unsaved {
            Some(transcript) => Ok(Some(transcript)),
            None => self.db()?.get_last_transcript(),
        }
    }

    /// Stashes the selection captured at the start of a voice transform. Every
    /// way a transform recording ends takes it back out (`take_pending_transform`
    /// on transcription, `discard_pending_transform` on cancel or a too-short
    /// recording). A leftover found here is stale: it is dropped without
    /// restoring, because its snapshot is older than the clipboard the new
    /// capture just saved.
    pub fn set_pending_transform(&self, captured: crate::selection_transform::CapturedSelection) {
        if let Ok(mut slot) = self.pending_transform.lock() {
            if let Some(stale) = slot.replace(captured) {
                stale.forget();
            }
        }
    }

    /// Drops the pending transform, which puts the user's pre-capture clipboard
    /// back. Called when a transform recording ends without a transcription.
    pub fn discard_pending_transform(&self) {
        drop(self.take_pending_transform());
    }

    /// Takes the pending transform selection, if any (consumed exactly once).
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

    pub fn dirs(&self) -> &crate::data_hygiene::DataDirs {
        &self.dirs
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

    /// The settings write lock. Take it (via `settings_store::lock`) before
    /// the database lock whenever settings are read in order to be written.
    pub fn settings_writes(&self) -> &Mutex<()> {
        &self.settings_writes
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

/// Runs blocking command work on Tauri's blocking thread pool. Tauri runs every
/// non-async command on the main thread, where anything slow (the database,
/// disk, the network, the clipboard, a file dialog) freezes every window, so
/// commands that touch any of those wrap their body in this.
async fn off_main<T, F>(app: tauri::AppHandle, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&tauri::AppHandle) -> Result<T, CommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || work(&app))
        .await
        .map_err(|error| {
            CommandError::new(
                "command_failed",
                format!("Scribe could not finish that action. {}", error),
            )
        })?
}

/// Like [`off_main`], for commands that open a native file dialog. Each dialog
/// gets a fresh thread, so the dialog's COM apartment can never clash with
/// whatever an earlier task on a reused pool thread set up.
async fn on_dialog_thread<T, F>(app: tauri::AppHandle, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&tauri::AppHandle) -> Result<T, CommandError> + Send + 'static,
{
    off_main(app, move |app| {
        let app = app.clone();
        std::thread::Builder::new()
            .name("scribe-file-dialog".to_string())
            .spawn(move || work(&app))
            .map_err(|error| {
                CommandError::new(
                    "command_failed",
                    format!("Scribe could not open the file dialog. {}", error),
                )
            })?
            .join()
            .map_err(|_| {
                CommandError::new("command_failed", "The file dialog closed unexpectedly.")
            })?
    })
    .await
}

#[tauri::command]
pub async fn get_app_state(app: tauri::AppHandle) -> Result<AppStateSnapshot, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        Ok(state.app_state()?.snapshot())
    })
    .await
}

#[tauri::command]
pub async fn get_settings(app: tauri::AppHandle) -> Result<AppSettings, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        state.db()?.get_settings()
    })
    .await
}

/// Changes only the settings named in `patch` (camelCase keys, as the
/// frontend sends them), so a window never writes back fields it did not
/// touch. Patches are applied one at a time against the stored settings, and
/// every window hears the result through `scribe:settings-changed`.
#[tauri::command]
pub async fn patch_settings(
    app: tauri::AppHandle,
    patch: serde_json::Map<String, serde_json::Value>,
) -> Result<AppSettings, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        settings_store::patch_locked(
            state.settings_writes(),
            &patch,
            || state.db()?.get_settings(),
            |previous, next| apply_settings_change(app, state, previous, next),
        )
    })
    .await
}

/// Stores `settings` over `previous` and applies what changed: launch at
/// startup, hotkey registration, GitHub activity, and retention. Rolls the
/// side effects back when the save fails. The caller holds the settings write
/// lock.
fn apply_settings_change(
    app: &tauri::AppHandle,
    state: &BackendState,
    previous: &AppSettings,
    settings: AppSettings,
) -> Result<AppSettings, CommandError> {
    settings
        .validate()
        .map_err(CommandError::invalid_settings)?;

    // Apply launch-at-startup first: when the OS registration fails the new
    // value is never saved, so the UI toggle reverts.
    if previous.launch_at_startup != settings.launch_at_startup {
        apply_autostart(app, settings.launch_at_startup)?;
    }

    // Transform Selection is only bound while Local AI is on, so turning it on
    // or off re-registers the hotkey set too.
    let transform_toggled = previous.notes_analysis_enabled != settings.notes_analysis_enabled;
    let rebind = previous.hotkeys != settings.hotkeys || transform_toggled;
    let mut hotkey_failures = Vec::new();
    if rebind {
        hotkeys::set_transform_enabled(app, settings.notes_analysis_enabled);
        hotkey_failures = match hotkeys::replace_hotkeys(app, &previous.hotkeys, &settings.hotkeys)
        {
            Ok(failures) => failures,
            Err(error) => {
                hotkeys::set_transform_enabled(app, previous.notes_analysis_enabled);
                return Err(error);
            }
        };
    }

    let save_result = if previous.github_repo != settings.github_repo {
        state
            .db()?
            .save_settings_and_clear_github_activity(&settings)
    } else {
        state.db()?.save_settings(&settings)
    };
    if let Err(error) = save_result {
        if rebind {
            hotkeys::set_transform_enabled(app, previous.notes_analysis_enabled);
            let _ = hotkeys::replace_hotkeys(app, &settings.hotkeys, &previous.hotkeys);
        }
        if previous.launch_at_startup != settings.launch_at_startup {
            let _ = apply_autostart(app, previous.launch_at_startup);
        }

        return Err(error);
    }

    // Every window updates from this event, so it goes out as soon as the
    // new settings are stored.
    settings_store::emit_changed(app, &settings);
    hotkeys::emit_registration_failures(app, &hotkey_failures);

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
pub async fn get_last_transcript(
    app: tauri::AppHandle,
) -> Result<Option<Transcript>, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        state.last_transcript()
    })
    .await
}

#[tauri::command]
pub async fn clear_last_transcript(app: tauri::AppHandle) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        state.set_unsaved_last_transcript(None);
        state.db()?.clear_last_transcript()
    })
    .await
}

#[tauri::command]
pub async fn list_recent_transcripts(
    app: tauri::AppHandle,
    limit: Option<u32>,
) -> Result<Vec<Transcript>, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let limit = limit.unwrap_or(20).clamp(1, 100);
        state.db()?.list_recent_transcripts(limit)
    })
    .await
}

#[tauri::command]
// Tauri maps these flat parameters directly from the stable frontend command
// payload, so grouping them would be a breaking IPC contract change.
#[allow(clippy::too_many_arguments)]
pub async fn search_transcripts(
    app: tauri::AppHandle,
    query: Option<String>,
    notes_only: Option<bool>,
    from: Option<String>,
    to: Option<String>,
    sort: Option<TranscriptSort>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<TranscriptSearchResult, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
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
    })
    .await
}

#[tauri::command]
pub async fn get_transcript(
    app: tauri::AppHandle,
    id: String,
) -> Result<Option<Transcript>, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        state.db()?.get_transcript_by_id(&id)
    })
    .await
}

#[tauri::command]
pub async fn update_transcript(
    app: tauri::AppHandle,
    id: String,
    text: String,
) -> Result<Transcript, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        state.db()?.update_transcript(&id, &text)
    })
    .await
}

#[tauri::command]
pub async fn delete_transcript(app: tauri::AppHandle, id: String) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        state.db()?.delete_transcript(&id)?;
        // "Open in editor" may have left a plain-text copy; it goes with the
        // transcript.
        let _ = std::fs::remove_file(state.dirs().editor_copy(&id));
        Ok(())
    })
    .await
}

/// Clears dictation history and everything that still holds a copy of it:
/// the Last Transcript Buffer, failed recordings and editor copies. Notes are
/// kept.
#[tauri::command]
pub async fn clear_transcript_history(app: tauri::AppHandle) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        state.set_unsaved_last_transcript(None);
        let db = state.db()?;
        db.clear_transcript_history()?;
        crate::data_hygiene::clear_history_leftovers(&db, state.dirs())
    })
    .await
}

#[tauri::command]
pub async fn clear_notes(app: tauri::AppHandle) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let db = state.db()?;
        db.clear_notes()?;
        db.vacuum()
    })
    .await
}

/// Loads the given transcripts, orders them oldest-first, and joins their text
/// with `separator` (default "\n\n"). Ids that don't resolve are skipped.
#[tauri::command]
pub async fn combine_transcripts(
    app: tauri::AppHandle,
    ids: Vec<String>,
    separator: Option<String>,
) -> Result<String, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let separator = separator.unwrap_or_else(|| "\n\n".to_string());
        state.db()?.combine_transcripts(&ids, &separator)
    })
    .await
}

/// Persists `text` as a new (non-note) history entry and makes it the Last
/// Transcript Buffer, mirroring how a dictation is saved. Returns the saved
/// transcript.
#[tauri::command]
pub async fn save_combined_transcript(
    app: tauri::AppHandle,
    text: String,
) -> Result<Transcript, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
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
    })
    .await
}

/// Writes a transcript's text to a temp `.txt` file under the app cache dir and
/// opens it with the OS default text app.
#[tauri::command]
pub async fn open_transcript_externally(
    app: tauri::AppHandle,
    id: String,
) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let transcript = state.db()?.get_transcript_by_id(&id)?.ok_or_else(|| {
            CommandError::new("transcript_not_found", "Transcript was not found.")
        })?;

        let export_dir = state.dirs().editor_copies();
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

        let path = state.dirs().editor_copy(&id);
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
    })
    .await
}

/// Returns a transcript's saved audio clip as a base64 WAV string for
/// in-app playback. Errors with code "transcript_audio_missing" when the
/// transcript has no clip or the file is gone.
#[tauri::command]
pub async fn get_transcript_audio(
    app: tauri::AppHandle,
    id: String,
) -> Result<String, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
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
    })
    .await
}

fn transcript_audio_missing() -> CommandError {
    CommandError::new(
        "transcript_audio_missing",
        "No audio clip is available for this transcript. It may have been deleted, or the dictation was recorded while clip saving was off.",
    )
}

#[tauri::command]
pub async fn get_basic_stats(app: tauri::AppHandle) -> Result<BasicStats, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        state.db()?.get_basic_stats()
    })
    .await
}

#[tauri::command]
pub async fn refresh_basic_stats(app: tauri::AppHandle) -> Result<BasicStats, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        state.db()?.get_basic_stats()
    })
    .await
}

#[tauri::command]
pub async fn get_hotkey_status(app: tauri::AppHandle) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let settings = state.db()?.get_settings()?;
        hotkeys::status(app, &settings.hotkeys)
    })
    .await
}

#[tauri::command]
pub async fn rebind_hotkey(
    app: tauri::AppHandle,
    action: String,
    shortcut: String,
) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let action = hotkeys::HotkeyAction::parse(&action)?;
        let mut settings = state.db()?.get_settings()?;
        let previous_hotkeys = settings.hotkeys.clone();
        let mut next_hotkeys = previous_hotkeys.clone();

        action.set_shortcut(&mut next_hotkeys, shortcut);
        hotkeys::validate_hotkeys(&next_hotkeys)?;
        let failures = hotkeys::replace_hotkeys(app, &previous_hotkeys, &next_hotkeys)?;

        // When the binding being changed is the one that failed, restore the
        // previous set (other bindings stay registered either way) and surface
        // the error inline to the rebind UI.
        if let Some(failure) = failures.iter().find(|failure| failure.action == action) {
            let message = failure.message.clone();
            let _ = hotkeys::replace_hotkeys(app, &next_hotkeys, &previous_hotkeys);
            return Err(CommandError::new("hotkey_registration_failed", message));
        }

        settings.hotkeys = next_hotkeys.clone();
        if let Err(error) = state.db()?.save_settings(&settings) {
            let _ = hotkeys::replace_hotkeys(app, &next_hotkeys, &previous_hotkeys);
            return Err(error);
        }

        settings_store::emit_changed(app, &settings);
        hotkeys::emit_registration_failures(app, &failures);
        hotkeys::status(app, &settings.hotkeys)
    })
    .await
}

/// Sets whether a single-shot bind (Toggle, Paste, Open Dashboard) acts on key
/// press or release. Rejected for Hold-to-Talk, which is push-to-talk. Re-runs
/// registration so the toggle watcher picks up the new edge, then persists.
#[tauri::command]
pub async fn set_hotkey_trigger(
    app: tauri::AppHandle,
    action: String,
    trigger: crate::settings::TriggerEdge,
) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
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
        let failures = hotkeys::replace_hotkeys(app, &previous_hotkeys, &next_hotkeys)?;

        settings.hotkeys = next_hotkeys.clone();
        if let Err(error) = state.db()?.save_settings(&settings) {
            let _ = hotkeys::replace_hotkeys(app, &next_hotkeys, &previous_hotkeys);
            return Err(error);
        }

        settings_store::emit_changed(app, &settings);
        hotkeys::emit_registration_failures(app, &failures);
        hotkeys::status(app, &settings.hotkeys)
    })
    .await
}

#[tauri::command]
pub async fn reset_hotkeys_to_defaults(
    app: tauri::AppHandle,
) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let mut settings = state.db()?.get_settings()?;
        let previous_hotkeys = settings.hotkeys.clone();
        // "Defaults" are flavor-specific: the Dev flavor resets to its
        // non-conflicting binds, stable resets to the production binds.
        let next_hotkeys = if crate::is_dev_flavor(app) {
            crate::settings::HotkeySettings::dev_defaults()
        } else {
            AppSettings::default().hotkeys
        };

        hotkeys::validate_hotkeys(&next_hotkeys)?;
        let failures = hotkeys::replace_hotkeys(app, &previous_hotkeys, &next_hotkeys)?;

        settings.hotkeys = next_hotkeys.clone();
        settings.dev_hotkeys_seeded = true;
        if let Err(error) = state.db()?.save_settings(&settings) {
            let _ = hotkeys::replace_hotkeys(app, &next_hotkeys, &previous_hotkeys);
            return Err(error);
        }

        settings_store::emit_changed(app, &settings);
        hotkeys::emit_registration_failures(app, &failures);
        hotkeys::status(app, &settings.hotkeys)
    })
    .await
}

/// Loads the production (stable-flavor) hotkey defaults. Exposed for the Dev
/// flavor's Developer panel so you can switch Dev back to your real binds when
/// running it alone. Sets `dev_hotkeys_seeded` so the one-shot dev seeding
/// never re-applies the Dev binds on the next launch.
#[tauri::command]
pub async fn load_production_hotkey_defaults(
    app: tauri::AppHandle,
) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let mut settings = state.db()?.get_settings()?;
        let previous_hotkeys = settings.hotkeys.clone();
        let next_hotkeys = AppSettings::default().hotkeys;

        hotkeys::validate_hotkeys(&next_hotkeys)?;
        let failures = hotkeys::replace_hotkeys(app, &previous_hotkeys, &next_hotkeys)?;

        settings.hotkeys = next_hotkeys.clone();
        settings.dev_hotkeys_seeded = true;
        if let Err(error) = state.db()?.save_settings(&settings) {
            let _ = hotkeys::replace_hotkeys(app, &next_hotkeys, &previous_hotkeys);
            return Err(error);
        }

        settings_store::emit_changed(app, &settings);
        hotkeys::emit_registration_failures(app, &failures);
        hotkeys::status(app, &settings.hotkeys)
    })
    .await
}

#[tauri::command]
pub fn open_dashboard(app: tauri::AppHandle) -> Result<(), CommandError> {
    crate::tray::open_dashboard(&app, None)
}

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

#[tauri::command]
pub async fn open_data_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
    off_main(app, move |app| open_folder(app, effective_data_dir(app)?)).await
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
pub async fn get_data_dir(app: tauri::AppHandle) -> Result<String, CommandError> {
    off_main(app, move |app| {
        Ok(effective_data_dir(app)?.to_string_lossy().into_owned())
    })
    .await
}

/// Opens a native folder picker so the user can choose a new data directory.
/// Returns the chosen absolute path, or None when the user cancels. Saving the
/// choice is the frontend's job (it writes `dataDir` via update_settings),
/// matching how the pill persists its position.
#[tauri::command]
pub async fn pick_data_dir(app: tauri::AppHandle) -> Result<Option<String>, CommandError> {
    on_dialog_thread(app, move |app| {
        #[cfg(windows)]
        {
            let start = effective_data_dir(app).unwrap_or_else(|_| std::path::PathBuf::from("."));
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
    })
    .await
}

#[tauri::command]
pub async fn open_models_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let dir = model_manager::models_dir(app)?;
        open_folder(app, dir)
    })
    .await
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
pub async fn open_logs_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let dir = logs_dir(app)?;
        open_folder(app, dir)
    })
    .await
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
pub async fn open_failed_recordings_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let dir = failed_recordings_dir(app)?;
        open_folder(app, dir)
    })
    .await
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
pub async fn save_window_size(app: tauri::AppHandle) -> Result<AppSettings, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let window = app.get_webview_window("main").ok_or_else(|| {
            CommandError::new("window_unavailable", "The main window is not available.")
        })?;
        let size = window.inner_size().map_err(|error| {
            CommandError::new(
                "window_size_unavailable",
                format!("Could not read the window size. {}", error),
            )
        })?;

        let _writes = settings_store::lock(state.settings_writes());
        let mut settings = state.db()?.get_settings()?;
        settings.window_width = Some(size.width as i32);
        settings.window_height = Some(size.height as i32);
        state.db()?.save_settings(&settings)?;
        settings_store::emit_changed(app, &settings);
        log::info!("Saved default window size {}x{}", size.width, size.height);
        Ok(settings)
    })
    .await
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
        let state = app.state::<BackendState>().inner();
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
pub async fn llm_status(
    app: tauri::AppHandle,
    endpoint: Option<String>,
) -> Result<crate::note_analysis::LlmStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let endpoint = match endpoint {
            Some(endpoint) => endpoint,
            None => state.db()?.get_settings()?.notes_analysis_endpoint,
        };
        Ok(crate::note_analysis::check_status(&endpoint))
    })
    .await
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

/// Connection / configuration status for the GitHub backup page.
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
pub async fn github_status(app: tauri::AppHandle) -> Result<GithubStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
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
    })
    .await
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

    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let mut settings = state.db()?.get_settings()?;
        settings.github_account_login = login;
        state.db()?.save_settings(&settings)?;
        settings_store::emit_changed(app, &settings);
        Ok(settings)
    })
    .await
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

    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let mut settings = state.db()?.get_settings()?;
        settings.github_account_login = String::new();
        settings.github_sync_enabled = false;
        settings.github_sync_all_transcripts = false;
        settings.github_repo = String::new();
        state
            .db()?
            .save_settings_and_clear_github_activity(&settings)?;
        settings_store::emit_changed(app, &settings);
        Ok(settings)
    })
    .await
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
        let state = app.state::<BackendState>().inner();
        let _guard = state.github_sync_lock();
        let attempt = crate::note_sync::collect_sync_attempt(&app, &service);
        crate::note_sync::record_sync_attempt(
            &app,
            crate::sync_history::SyncSource::Manual,
            &attempt,
        );
        attempt.into_result()
    })
    .await
    .map_err(|error| CommandError::new("github_sync_failed", error.to_string()))?
}

#[tauri::command]
pub async fn github_sync_activity(
    app: tauri::AppHandle,
) -> Result<Option<crate::sync_history::GitHubSyncActivity>, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        state.db()?.get_github_sync_activity()
    })
    .await
}

/// Exports transcripts to a local file the user picks via a native save dialog.
/// `scope` selects which rows ("all" | "notes" | "dictation"); `format` selects
/// the renderer ("markdown" | "csv" | "json"). Returns the saved absolute path,
/// or None when the user cancels the dialog. Unlike the GitHub backup this needs
/// no connected account — it is a purely local save.
#[tauri::command]
pub async fn export_transcripts(
    app: tauri::AppHandle,
    scope: String,
    format: String,
) -> Result<Option<String>, CommandError> {
    on_dialog_thread(app, move |app| {
        let state = app.state::<BackendState>().inner();
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
            let start = effective_data_dir(app).unwrap_or_else(|_| std::path::PathBuf::from("."));
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
    })
    .await
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
pub async fn preview_transcript_import(
    app: tauri::AppHandle,
) -> Result<Option<TranscriptImportPreview>, CommandError> {
    on_dialog_thread(app, move |app| {
        let state = app.state::<BackendState>().inner();
        #[cfg(windows)]
        {
            let start = effective_data_dir(app).unwrap_or_else(|_| std::path::PathBuf::from("."));
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
    })
    .await
}

/// Revalidates the selected export, then imports every row in one database
/// transaction. Re-reading at confirmation time prevents a stale preview from
/// bypassing validation if the source file changed while the dialog was open.
#[tauri::command]
pub async fn restore_transcript_import(
    app: tauri::AppHandle,
    path: String,
    replace_existing: bool,
    expected_fingerprint: String,
) -> Result<ImportReport, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let loaded = load_transcript_import(std::path::Path::new(&path))?;
        crate::import::verify_fingerprint(&loaded.fingerprint, &expected_fingerprint)?;
        state
            .db()?
            .restore_transcripts(&loaded.prepared.transcripts, replace_existing)
    })
    .await
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

#[tauri::command]
pub async fn copy_last_transcript(app: tauri::AppHandle) -> Result<OutputResult, CommandError> {
    off_main(app, output::copy_last_transcript).await
}

#[tauri::command]
pub async fn paste_transcript(
    app: tauri::AppHandle,
    id: String,
) -> Result<OutputResult, CommandError> {
    off_main(app, move |app| output::paste_transcript(app, &id)).await
}

#[tauri::command]
pub async fn copy_transcript(
    app: tauri::AppHandle,
    id: String,
) -> Result<OutputResult, CommandError> {
    off_main(app, move |app| output::copy_transcript(app, &id)).await
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
        let state = app.state::<BackendState>().inner();
        let settings = state.db()?.get_settings()?;
        selection_transform::ensure_enabled(&settings)?;
        (
            settings.notes_analysis_endpoint,
            settings.notes_analysis_model,
        )
    };

    // 1. The button click left Scribe in front: hand focus back to the app the
    //    user was in, so the copy reads their selection and not Scribe's.
    selection_transform::focus_previous_app()?;

    // 2. Copy the current selection out of that app, remembering the user's
    //    prior clipboard so it can be restored.
    let captured = selection_transform::capture_selection()?
        .filter(|captured| !captured.selection.trim().is_empty())
        .ok_or_else(|| {
            CommandError::new(
                "selection_empty",
                "No text was selected. Highlight some text in the other app first, then transform it.",
            )
        })?;

    // 3. Rewrite it with the local LLM. On failure, dropping `captured`
    //    restores the clipboard.
    let transformed =
        selection_transform::transform(&captured.selection, instruction, &endpoint, &model)?;

    // 4. Paste the result back over the still-selected original, then restore
    //    the pre-capture clipboard.
    selection_transform::apply_result(app, &transformed, captured)
}

#[tauri::command]
pub async fn list_models(app: tauri::AppHandle) -> Result<Vec<ModelInfo>, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let db = state.db()?;
        model_manager::list_models(app, &db, state.model_downloads())
    })
    .await
}

#[tauri::command]
pub async fn download_model(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main(app, move |app| {
        model_manager::download_model(app, app.state::<BackendState>().inner(), &model_id)
    })
    .await
}

#[tauri::command]
pub async fn cancel_model_download(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        if model_manager::request_cancel_download(state.model_downloads(), &model_id)? {
            return Ok(());
        }

        let db = state.db()?;
        model_manager::cancel_model_download(&db, state.model_downloads(), &model_id)
    })
    .await
}

#[tauri::command]
pub async fn retry_model_download(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main(app, move |app| {
        model_manager::retry_model_download(app, app.state::<BackendState>().inner(), &model_id)
    })
    .await
}

#[tauri::command]
pub async fn delete_model(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let (info, settings) = {
            let db = state.db()?;
            let before = db.get_settings()?;
            let info = model_manager::delete_model(app, &db, &model_id)?;
            let after = db.get_settings()?;
            (info, (after != before).then_some(after))
        };
        // Deleting the selected model clears the selection.
        if let Some(settings) = settings {
            settings_store::emit_changed(app, &settings);
        }
        Ok(info)
    })
    .await
}

#[tauri::command]
pub async fn select_model(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let info = model_manager::select_model(app, state, &model_id)?;
        let settings = state.db()?.get_settings()?;
        settings_store::emit_changed(app, &settings);
        Ok(info)
    })
    .await
}
