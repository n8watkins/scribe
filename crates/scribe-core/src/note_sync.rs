//! Auto-sync: when a note is saved and GitHub sync is enabled, push the day's
//! notes to a private GitHub repo off the dictation thread. A background worker
//! debounces a burst of saves into a single sync.

use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use tauri::{AppHandle, Manager};

use crate::commands::BackendState;
use crate::core::Core;
use crate::error::CommandError;
use crate::github_backup::{GitHubBackup, SyncReport};
use crate::replacements_file::ReplacementStore;
use crate::sync_history::{GitHubSyncActivity, SyncSource};

/// Quiet period after the last note save before a sync fires, so rapid notes
/// collapse into one upload.
const DEBOUNCE: Duration = Duration::from_secs(3);

/// Gathers the notes from the DB and syncs them to GitHub. Shared by the manual
/// `github_sync_now` command and the auto-sync worker. Blocking (DB + network),
/// so callers run it off the main thread. The DB lock is dropped before any
/// network call.
///
/// The `AppHandle` wrapper of [`collect_and_sync_with`]; the sign-in is
/// looked up under the Core's flavour, which is the app's identifier.
pub fn collect_and_sync(app: &AppHandle, _service: &str) -> Result<SyncReport, CommandError> {
    match app.try_state::<Core>() {
        Some(core) => collect_and_sync_with(core.inner()),
        None => {
            log::warn!("Could not sync notes to GitHub: the engine is not ready yet");
            Err(crate::host::no_main_window())
        }
    }
}

/// [`collect_and_sync`] on the engine handle.
pub fn collect_and_sync_with(core: &Core) -> Result<SyncReport, CommandError> {
    collect_notes(
        core.backend(),
        core.replacements(),
        core.flavor().identifier(),
    )
}

fn collect_notes(
    state: &BackendState,
    replacements_store: &ReplacementStore,
    service: &str,
) -> Result<SyncReport, CommandError> {
    let (repo, notes) = {
        let db = state.db()?;
        let settings = db.get_settings()?;
        // Gate on the notes-sync setting, mirroring the all-transcripts gate in
        // collect_and_sync_all_transcripts: the worker (and Sync now) must not
        // push notes when only "Back up all transcripts" is on.
        if !settings.github_sync_enabled {
            return Ok(SyncReport {
                synced_notes: 0,
                files_written: 0,
            });
        }
        if !crate::github_oauth::has_stored_token(service) {
            return Err(CommandError::new(
                "github_not_signed_in",
                "Connect GitHub on the GitHub backup page first.",
            ));
        }
        if settings.github_repo.trim().is_empty() {
            return Err(CommandError::new(
                "github_repo_unset",
                "Set a GitHub repository (owner/name) on the GitHub backup page first.",
            ));
        }
        // The daily GitHub file is a clean notes-only log.
        let notes = db
            .search_transcripts(
                None,
                true,
                None,
                None,
                crate::transcript::TranscriptSort::default(),
                100_000,
                0,
            )?
            .transcripts;
        (settings.github_repo, notes)
    };

    // replacements.txt goes along with the notes, when it changed since the
    // last upload in this session.
    let replacements = pending_replacements(replacements_store);
    if notes.is_empty() && replacements.is_none() {
        return Ok(SyncReport {
            synced_notes: 0,
            files_written: 0,
        });
    }

    let token = crate::github_oauth::access_token(service)?;
    let result = sync_notes_and_replacements(token, &repo, &notes, replacements);
    clear_token_if_unauthorized(service, result)
}

/// Writes the notes' daily files, then replacements.txt when it changed.
fn sync_notes_and_replacements(
    token: String,
    repo: &str,
    notes: &[crate::transcript::Transcript],
    replacements: Option<(String, u64)>,
) -> Result<SyncReport, CommandError> {
    let backup = GitHubBackup::new(token, repo)?;
    let mut report = if notes.is_empty() {
        SyncReport {
            synced_notes: 0,
            files_written: 0,
        }
    } else {
        backup.sync_notes(notes)?
    };
    if let Some((text, hash)) = replacements {
        backup.sync_file(REPLACEMENTS_PATH, &text)?;
        *lock_uploaded() = Some(hash);
        report.files_written += 1;
    }
    Ok(report)
}

/// Where replacements.txt is kept in the backup repository.
const REPLACEMENTS_PATH: &str = "scribe/replacements.txt";

fn lock_uploaded() -> std::sync::MutexGuard<'static, Option<u64>> {
    static UPLOADED: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);
    UPLOADED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// replacements.txt and its hash, when it has content that this session
/// hasn't uploaded yet.
fn pending_replacements(store: &ReplacementStore) -> Option<(String, u64)> {
    use std::hash::{Hash, Hasher};
    let text = std::fs::read_to_string(store.path()).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    let hash = hasher.finish();
    (*lock_uploaded() != Some(hash)).then_some((text, hash))
}

/// When a GitHub call comes back `github_unauthorized` (HTTP 401 — the token was
/// revoked or expired), drop the dead token from the keyring so `github_status`
/// immediately flips to not-connected and the UI prompts a reconnect. The Err is
/// still returned for the caller to log. Sign-out is best-effort; its own failure
/// never masks the original error.
fn clear_token_if_unauthorized(
    service: &str,
    result: Result<SyncReport, CommandError>,
) -> Result<SyncReport, CommandError> {
    if let Err(error) = &result {
        if error.code == "github_unauthorized" {
            let _ = crate::github_oauth::sign_out(service);
        }
    }
    result
}

/// Backs up every DICTATION transcript to the repo's distinct `transcripts/`
/// folder when `github_sync_all_transcripts` is on. Notes are deliberately
/// excluded here: they already sync as the curated daily log via
/// `collect_and_sync`/`sync_notes`, so this covers only the ordinary dictation
/// transcripts that the notes path skips — together the two give a full backup
/// without writing any transcript into both folders. Blocking (DB + network),
/// so callers run it off the main thread; the DB lock is dropped before the
/// network call. A no-op (empty report) when the setting is off or there are no
/// dictation transcripts.
///
/// The `AppHandle` wrapper of [`collect_and_sync_all_transcripts_with`].
pub fn collect_and_sync_all_transcripts(
    app: &AppHandle,
    _service: &str,
) -> Result<SyncReport, CommandError> {
    match app.try_state::<Core>() {
        Some(core) => collect_and_sync_all_transcripts_with(core.inner()),
        None => {
            log::warn!("Could not back up transcripts to GitHub: the engine is not ready yet");
            Err(crate::host::no_main_window())
        }
    }
}

/// [`collect_and_sync_all_transcripts`] on the engine handle.
pub fn collect_and_sync_all_transcripts_with(core: &Core) -> Result<SyncReport, CommandError> {
    collect_transcripts(core.backend(), core.flavor().identifier())
}

fn collect_transcripts(state: &BackendState, service: &str) -> Result<SyncReport, CommandError> {
    let (repo, dictations) = {
        let db = state.db()?;
        let settings = db.get_settings()?;
        if !settings.github_sync_all_transcripts {
            return Ok(SyncReport {
                synced_notes: 0,
                files_written: 0,
            });
        }
        if !crate::github_oauth::has_stored_token(service) {
            return Err(CommandError::new(
                "github_not_signed_in",
                "Connect GitHub on the GitHub backup page first.",
            ));
        }
        if settings.github_repo.trim().is_empty() {
            return Err(CommandError::new(
                "github_repo_unset",
                "Set a GitHub repository (owner/name) on the GitHub backup page first.",
            ));
        }
        let dictations = db.search_dictation_transcripts(100_000, 0)?.transcripts;
        (settings.github_repo, dictations)
    };

    if dictations.is_empty() {
        return Ok(SyncReport {
            synced_notes: 0,
            files_written: 0,
        });
    }

    let token = crate::github_oauth::access_token(service)?;
    let result = GitHubBackup::new(token, &repo)?.sync_all_transcripts(&dictations);
    clear_token_if_unauthorized(service, result)
}

#[derive(Debug)]
pub struct SyncAttempt {
    pub repo: String,
    pub report: SyncReport,
    pub error: Option<CommandError>,
}

impl SyncAttempt {
    pub fn into_result(self) -> Result<SyncReport, CommandError> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(self.report),
        }
    }
}

/// Runs both configured backup paths so one destination can still succeed when
/// the other fails. The first failure is returned while successful progress is
/// retained for persisted health reporting.
///
/// The `AppHandle` wrapper of [`collect_sync_attempt_with`]. With no engine
/// yet the attempt carries the no-engine error and no progress.
pub fn collect_sync_attempt(app: &AppHandle, _service: &str) -> SyncAttempt {
    match app.try_state::<Core>() {
        Some(core) => collect_sync_attempt_with(core.inner()),
        None => {
            log::warn!("Could not back up to GitHub: the engine is not ready yet");
            SyncAttempt {
                repo: String::new(),
                report: SyncReport {
                    synced_notes: 0,
                    files_written: 0,
                },
                error: Some(crate::host::no_main_window()),
            }
        }
    }
}

/// [`collect_sync_attempt`] on the engine handle.
pub fn collect_sync_attempt_with(core: &Core) -> SyncAttempt {
    let repo = core
        .backend()
        .db()
        .and_then(|db| db.get_settings())
        .map(|settings| settings.github_repo)
        .unwrap_or_default();
    combine_sync_results(
        repo,
        collect_and_sync_with(core),
        collect_and_sync_all_transcripts_with(core),
    )
}

/// The `AppHandle` wrapper of [`record_sync_attempt_with`]; with no engine
/// yet it only logs.
pub fn record_sync_attempt(app: &AppHandle, source: SyncSource, attempt: &SyncAttempt) {
    match app.try_state::<Core>() {
        Some(core) => record_sync_attempt_with(core.inner(), source, attempt),
        None => log::warn!("Could not save GitHub backup status: the engine is not ready yet"),
    }
}

/// [`record_sync_attempt`] on the engine handle.
pub fn record_sync_attempt_with(core: &Core, source: SyncSource, attempt: &SyncAttempt) {
    record_attempt(core.backend(), source, attempt);
}

fn record_attempt(state: &BackendState, source: SyncSource, attempt: &SyncAttempt) {
    let save = (|| -> Result<(), CommandError> {
        let db = state.db()?;
        let activity = GitHubSyncActivity::from_attempt(
            source,
            attempt.repo.clone(),
            &attempt.report,
            attempt.error.as_ref(),
        );
        db.save_github_sync_activity(&activity)
    })();
    if let Err(error) = save {
        log::warn!("Could not save GitHub backup status: {}", error.message);
    }
}

fn combine_sync_results(
    repo: String,
    notes: Result<SyncReport, CommandError>,
    transcripts: Result<SyncReport, CommandError>,
) -> SyncAttempt {
    let mut report = SyncReport {
        synced_notes: 0,
        files_written: 0,
    };
    let mut first_error = None;
    for result in [notes, transcripts] {
        match result {
            Ok(next) => {
                report.synced_notes += next.synced_notes;
                report.files_written += next.files_written;
            }
            Err(error) if first_error.is_none() => first_error = Some(error),
            Err(_) => {}
        }
    }
    SyncAttempt {
        repo,
        report,
        error: first_error,
    }
}

/// Owns the channel that note-save events are pushed onto. Held in Tauri's
/// managed state. Clones share the channel (Tauri state and `core::Core` hold
/// the same worker). The thread would end once every clone dropped, but the
/// app never drops them; it ends with the process, so nothing may rely on drop
/// to finish a sync.
#[derive(Clone)]
pub struct NoteSyncWorker {
    tx: Sender<()>,
}

impl NoteSyncWorker {
    /// Spawns the background debounce-and-sync thread.
    pub fn spawn(app: AppHandle) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        // The worker starts before the Core exists (the Core holds the
        // worker), so it finds the Core through the app each time it syncs.
        let _ = thread::Builder::new()
            .name("scribe-note-sync".into())
            .spawn(move || worker_loop(|| app.try_state::<Core>().map(|c| c.inner().clone()), rx));
        Self { tx }
    }

    /// Signals that a note was saved. The worker debounces, then syncs. Cheap
    /// and non-blocking; safe to call from the dictation path.
    pub fn notify(&self) {
        let _ = self.tx.send(());
    }
}

fn worker_loop(find_core: impl Fn() -> Option<Core>, rx: Receiver<()>) {
    loop {
        // Block until at least one note-saved signal arrives.
        if rx.recv().is_err() {
            return; // sender dropped — app is shutting down
        }
        // Debounce: keep waiting while more saves keep coming in.
        loop {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        // Serialize this whole sync against a manual "Sync now" so the same
        // daily file is never PUT concurrently. Acquired once here (not inside
        // the collect_* helpers) to stay re-entrancy-/deadlock-free, and held
        // across both collect calls below. Dropped at the end of the iteration.
        let Some(core) = find_core() else {
            log::warn!("Skipped an automatic GitHub backup: the engine is not ready yet");
            continue;
        };
        let _guard = core.backend().github_sync_lock();

        // A github_unauthorized error has already cleared the dead token inside
        // collect_*, so by the time we record this attempt connection state is
        // clean. Both configured paths run even if one fails.
        let attempt = collect_sync_attempt_with(&core);
        record_sync_attempt_with(&core, SyncSource::Automatic, &attempt);
        if let Some(error) = &attempt.error {
            log::warn!(
                "Automatic GitHub backup failed after syncing {} item(s) into {} file(s): {}",
                attempt.report.synced_notes,
                attempt.report.files_written,
                error.message
            );
        } else {
            log::info!(
                "Automatically backed up {} item(s) into {} GitHub file(s)",
                attempt.report.synced_notes,
                attempt.report.files_written
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(items: u32, files: u32) -> Result<SyncReport, CommandError> {
        Ok(SyncReport {
            synced_notes: items,
            files_written: files,
        })
    }

    fn state(root: &std::path::Path) -> BackendState {
        BackendState::new(
            crate::db::Database::in_memory().unwrap(),
            root.join("cache").join("recordings"),
            crate::data_hygiene::DataDirs {
                data_dir: root.join("data"),
                cache_dir: root.join("cache"),
            },
        )
    }

    #[test]
    fn both_backups_are_no_ops_while_their_settings_are_off() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let store = ReplacementStore::open(dir.path().join("replacements.txt"), &[]);
        // Off by default: neither path reaches the keychain or the network.
        let notes = collect_notes(&state, &store, "test.scribe.sync").unwrap();
        assert_eq!((notes.synced_notes, notes.files_written), (0, 0));
        let all = collect_transcripts(&state, "test.scribe.sync").unwrap();
        assert_eq!((all.synced_notes, all.files_written), (0, 0));
    }

    #[test]
    fn a_recorded_attempt_reads_back_as_the_sync_activity() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let attempt = SyncAttempt {
            repo: "alice/notes".into(),
            report: SyncReport {
                synced_notes: 2,
                files_written: 1,
            },
            error: Some(CommandError::new("github_unauthorized", "revoked")),
        };
        record_attempt(&state, SyncSource::Manual, &attempt);
        let activity = state.db().unwrap().get_github_sync_activity().unwrap();
        let activity = activity.expect("the attempt was saved");
        assert_eq!(activity.repo, "alice/notes");
        assert_eq!(activity.source, SyncSource::Manual);
        assert_eq!(activity.synced_items, 2);
        assert_eq!(activity.error_code.as_deref(), Some("github_unauthorized"));
    }

    #[test]
    fn a_blank_replacements_file_is_not_pending() {
        let dir = tempfile::tempdir().unwrap();
        let empty = ReplacementStore::open(dir.path().join("replacements.txt"), &[]);
        std::fs::write(empty.path(), "  \n").unwrap();
        assert!(pending_replacements(&empty).is_none());
    }

    #[test]
    fn combines_successful_note_and_transcript_progress() {
        let attempt = combine_sync_results("alice/notes".into(), report(2, 1), report(3, 2));
        assert_eq!(attempt.repo, "alice/notes");
        assert_eq!(attempt.report.synced_notes, 5);
        assert_eq!(attempt.report.files_written, 3);
        assert!(attempt.error.is_none());
    }

    #[test]
    fn keeps_partial_progress_and_first_error() {
        let attempt = combine_sync_results(
            "alice/notes".into(),
            Err(CommandError::new("notes_failed", "notes failed")),
            report(3, 2),
        );
        assert_eq!(attempt.report.synced_notes, 3);
        assert_eq!(attempt.report.files_written, 2);
        assert_eq!(attempt.error.unwrap().code, "notes_failed");
    }
}
