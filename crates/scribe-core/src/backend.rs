//! The state the engine's commands share: the dictation state machine, the
//! audio service, the database, the download and sign-in registries, and the
//! locks that keep settings and GitHub writes in order.
//!
//! It lived in `commands.rs` until the Tauri commands moved to the shell
//! (app/src-tauri/src/commands).

use std::sync::Mutex;

use crate::{
    app_state::{AppEvent, AppStateMachine, AppStateSnapshot},
    audio,
    db::Database,
    error::CommandError,
    model_manager::DownloadRegistry,
    output,
    transcript::Transcript,
};

/// The state the commands share. A cheap handle: clones share one
/// [`Backend`], so the instance in Tauri state and the one [`crate::core::Core`]
/// holds are the same.
#[derive(Clone)]
pub struct BackendState(std::sync::Arc<Backend>);

impl std::ops::Deref for BackendState {
    type Target = Backend;

    fn deref(&self) -> &Backend {
        &self.0
    }
}

pub struct Backend {
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
        Self(std::sync::Arc::new(Backend {
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
        }))
    }
}

impl Backend {
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
