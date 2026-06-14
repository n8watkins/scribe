//! Phase 2 auto-sync: when a note is saved and Drive sync is enabled, push the
//! day's notes to Google Drive off the dictation thread. A background worker
//! debounces a burst of saves into a single sync.

use std::thread;
use std::time::Duration;

use chrono::{Local, Timelike};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use tauri::{AppHandle, Manager};

use crate::commands::BackendState;
use crate::error::CommandError;
use crate::google_drive::{Drive, SyncReport};

/// Quiet period after the last note save before a sync fires, so rapid notes
/// collapse into one upload.
const DEBOUNCE: Duration = Duration::from_secs(3);

/// How often the end-of-day organize scheduler wakes to check whether the
/// configured hour has passed for an un-organized day.
const ORGANIZE_TICK: Duration = Duration::from_secs(300);

/// Gathers the notes from the DB and syncs them to Google Drive. Shared by the
/// manual `drive_sync_now` command and the auto-sync worker. Blocking (DB +
/// network), so callers run it off the main thread. The DB lock is dropped
/// before any network call.
pub fn collect_and_sync(app: &AppHandle, service: &str) -> Result<SyncReport, CommandError> {
    let state = app.state::<BackendState>();
    let notes = {
        let db = state.db()?;
        let settings = db.get_settings()?;
        if settings.drive_account_email.is_empty()
            && !crate::google_oauth::has_stored_token(service)
        {
            return Err(CommandError::new(
                "google_not_signed_in",
                "Sign in to Google in Settings → Integrations first.",
            ));
        }
        // The daily Drive file is a clean notes-only log (Phase 1/2).
        db.search_transcripts(
            None,
            true,
            None,
            None,
            crate::transcript::TranscriptSort::default(),
            100_000,
            0,
        )?
        .transcripts
    };

    if notes.is_empty() {
        return Ok(SyncReport {
            synced_notes: 0,
            files_written: 0,
        });
    }

    let token = crate::google_oauth::access_token(service)?;
    Drive::new(token)?.sync_notes(&notes)
}

/// Owns the channel that note-save events are pushed onto. Held in Tauri's
/// managed state; dropping it (on app exit) ends the worker thread.
pub struct DriveSyncWorker {
    tx: Sender<()>,
}

impl DriveSyncWorker {
    /// Spawns the background debounce-and-sync thread.
    pub fn spawn(app: AppHandle) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        let service = app.config().identifier.clone();
        let _ = thread::Builder::new()
            .name("scribe-drive-sync".into())
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
        match collect_and_sync(&app, &service) {
            Ok(report) => log::info!(
                "Auto-synced {} note(s) into {} Drive file(s)",
                report.synced_notes,
                report.files_written
            ),
            Err(error) => {
                log::warn!("Auto-sync to Google Drive failed: {}", error.message)
            }
        }
    }
}

/// Runs the local LLM over one local calendar `day`'s notes and writes the
/// reorganized Markdown to Drive as `{day}-organized.md`. Blocking (DB + two
/// network calls), so callers run it off the main thread; the DB lock is
/// dropped before any network call. Returns `Ok(true)` when an organized file
/// was written, `Ok(false)` when the day had no notes (nothing to do).
pub fn organize_day(app: &AppHandle, service: &str, day: &str) -> Result<bool, CommandError> {
    // Gather everything up front and drop the DB lock before the LLM/Drive
    // calls, which can take a while.
    let state = app.state::<BackendState>();
    let (settings, day_notes) = {
        let db = state.db()?;
        let settings = db.get_settings()?;
        if settings.drive_account_email.is_empty()
            && !crate::google_oauth::has_stored_token(service)
        {
            return Err(CommandError::new(
                "google_not_signed_in",
                "Sign in to Google in Settings → Integrations first.",
            ));
        }
        // Pull all notes, then filter to the requested local calendar day.
        let mut notes = db
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
        notes.retain(|note| {
            note.created_at
                .with_timezone(&Local)
                .format("%Y-%m-%d")
                .to_string()
                == day
        });
        notes.sort_by_key(|note| note.created_at);
        (settings, notes)
    };

    if day_notes.is_empty() {
        return Ok(false);
    }

    if !settings.notes_analysis_enabled {
        return Err(CommandError::new(
            "notes_analysis_disabled",
            "The local LLM (notes analysis) is turned off in Settings; \
             enable it to auto-organize the day's notes.",
        ));
    }

    // Build one combined document: each note's text and, when present, its
    // existing analysis summary — the same material the daily file shows.
    let mut combined = String::new();
    for note in &day_notes {
        combined.push_str(note.text.trim());
        combined.push('\n');
        if let Some(summary) = note
            .analysis
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            combined.push_str("(summary: ");
            combined.push_str(summary);
            combined.push_str(")\n");
        }
        combined.push('\n');
    }

    let outcome = crate::note_analysis::analyze_text(
        &settings.notes_analysis_endpoint,
        &settings.notes_analysis_model,
        &settings.drive_organize_prompt,
        &combined,
    )?;

    let token = crate::google_oauth::access_token(service)?;
    Drive::new(token)?.write_organized(day, &outcome.analysis)?;
    Ok(true)
}

/// Owns the end-of-day organize scheduler thread. Held in Tauri's managed
/// state; dropping it (on app exit) lets the loop exit on its next tick.
pub struct DriveOrganizeScheduler {
    _private: (),
}

impl DriveOrganizeScheduler {
    /// Spawns the background thread that, once a day at the configured hour,
    /// organizes the previous day's notes with the local LLM.
    pub fn spawn(app: AppHandle) -> Self {
        let service = app.config().identifier.clone();
        let _ = thread::Builder::new()
            .name("scribe-drive-organize".into())
            .spawn(move || organize_loop(app, service));
        Self { _private: () }
    }
}

fn organize_loop(app: AppHandle, service: String) {
    loop {
        thread::sleep(ORGANIZE_TICK);

        // Read settings fresh each tick so toggling the feature or changing the
        // hour takes effect without a restart.
        let settings = match app.state::<BackendState>().db().and_then(|db| db.get_settings()) {
            Ok(settings) => settings,
            Err(error) => {
                log::warn!("Organize scheduler could not read settings: {}", error.message);
                continue;
            }
        };

        if !settings.drive_organize_enabled {
            continue;
        }

        // Organize *yesterday* (local): by the time the configured hour rolls
        // around, the previous day is complete.
        let now = Local::now();
        let target = (now - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();

        if (now.hour() as u32) < settings.drive_organize_hour
            || settings.drive_last_organized_date == target
        {
            continue;
        }

        match organize_day(&app, &service, &target) {
            Ok(organized) => {
                log::info!(
                    "End-of-day organize for {} complete ({}).",
                    target,
                    if organized { "wrote file" } else { "no notes" }
                );
                // Mark the day done whether or not there were notes, so an
                // empty day isn't retried every tick.
                if let Err(error) = mark_organized(&app, &target) {
                    log::warn!(
                        "Could not record last-organized date {}: {}",
                        target,
                        error.message
                    );
                }
            }
            // E.g. the LLM server is down: don't record the date, so it retries
            // on the next tick (the hour gate stays open for the rest of today).
            Err(error) => log::warn!(
                "End-of-day organize for {} failed: {}",
                target,
                error.message
            ),
        }
    }
}

/// Persists `day` as the last-organized date so the scheduler doesn't redo it.
fn mark_organized(app: &AppHandle, day: &str) -> Result<(), CommandError> {
    let state = app.state::<BackendState>();
    let db = state.db()?;
    let mut settings = db.get_settings()?;
    settings.drive_last_organized_date = day.to_string();
    db.save_settings(&settings)
}
