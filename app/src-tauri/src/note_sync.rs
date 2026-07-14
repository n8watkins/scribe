//! Auto-sync: when a note is saved and GitHub sync is enabled, push the day's
//! notes to a private GitHub repo off the dictation thread. A background worker
//! debounces a burst of saves into a single sync.

use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use tauri::{AppHandle, Manager};

use crate::commands::BackendState;
use crate::error::CommandError;
use crate::github_backup::{GitHubBackup, SyncReport};
use crate::sync_history::{GitHubSyncActivity, SyncSource};

/// Quiet period after the last note save before a sync fires, so rapid notes
/// collapse into one upload.
const DEBOUNCE: Duration = Duration::from_secs(3);

/// Gathers the notes from the DB and syncs them to GitHub. Shared by the manual
/// `github_sync_now` command and the auto-sync worker. Blocking (DB + network),
/// so callers run it off the main thread. The DB lock is dropped before any
/// network call.
pub fn collect_and_sync(app: &AppHandle, service: &str) -> Result<SyncReport, CommandError> {
    let state = app.state::<BackendState>();
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
                "Sign in to GitHub in Settings → Sync first.",
            ));
        }
        if settings.github_repo.trim().is_empty() {
            return Err(CommandError::new(
                "github_repo_unset",
                "Set a GitHub repository (owner/name) in Settings → Sync first.",
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

    if notes.is_empty() {
        return Ok(SyncReport {
            synced_notes: 0,
            files_written: 0,
        });
    }

    let token = crate::github_oauth::access_token(service)?;
    let result = GitHubBackup::new(token, &repo)?.sync_notes(&notes);
    clear_token_if_unauthorized(service, result)
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
pub fn collect_and_sync_all_transcripts(
    app: &AppHandle,
    service: &str,
) -> Result<SyncReport, CommandError> {
    let (repo, dictations) = {
        let state = app.state::<BackendState>();
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
                "Sign in to GitHub in Settings → Sync first.",
            ));
        }
        if settings.github_repo.trim().is_empty() {
            return Err(CommandError::new(
                "github_repo_unset",
                "Set a GitHub repository (owner/name) in Settings → Sync first.",
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
pub fn collect_sync_attempt(app: &AppHandle, service: &str) -> SyncAttempt {
    let repo = app
        .state::<BackendState>()
        .db()
        .and_then(|db| db.get_settings())
        .map(|settings| settings.github_repo)
        .unwrap_or_default();
    combine_sync_results(
        repo,
        collect_and_sync(app, service),
        collect_and_sync_all_transcripts(app, service),
    )
}

pub fn record_sync_attempt(app: &AppHandle, source: SyncSource, attempt: &SyncAttempt) {
    let state = app.state::<BackendState>();
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
/// managed state; dropping it (on app exit) ends the worker thread.
pub struct NoteSyncWorker {
    tx: Sender<()>,
}

impl NoteSyncWorker {
    /// Spawns the background debounce-and-sync thread.
    pub fn spawn(app: AppHandle) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        let service = app.config().identifier.clone();
        let _ = thread::Builder::new()
            .name("scribe-note-sync".into())
            .spawn(move || worker_loop(app, service, rx));
        Self { tx }
    }

    /// Signals that a note was saved. The worker debounces, then syncs. Cheap
    /// and non-blocking; safe to call from the dictation path.
    pub fn notify(&self) {
        let _ = self.tx.send(());
    }
}

fn worker_loop(app: AppHandle, service: String, rx: Receiver<()>) {
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
        let state = app.state::<BackendState>();
        let _guard = state.github_sync_lock();

        // A github_unauthorized error has already cleared the dead token inside
        // collect_*, so by the time we record this attempt connection state is
        // clean. Both configured paths run even if one fails.
        let attempt = collect_sync_attempt(&app, &service);
        record_sync_attempt(&app, SyncSource::Automatic, &attempt);
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
