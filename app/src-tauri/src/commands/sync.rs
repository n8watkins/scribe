//! Commands of the `sync` domain (note_sync, github_backup, github_oauth,
//! sync_history, note_analysis).

use super::off_main_core;
use scribe_core::core::Core;

use crate::{
    error::CommandError,
    github_oauth::{GithubDeviceStart, GithubStatus},
    settings::AppSettings,
    transcript::Transcript,
};

/// Runs the user's notes-analysis prompt over a saved transcript via the
/// configured local LLM server and stores the result on the row. Async so
/// the (possibly slow) local inference happens on a blocking worker. The
/// Notes view only offers this for notes, but any transcript id works.
#[tauri::command]
pub async fn analyze_note(
    core: tauri::State<'_, Core>,
    transcript_id: String,
) -> Result<Transcript, CommandError> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = core.backend();
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
    core: tauri::State<'_, Core>,
    endpoint: Option<String>,
) -> Result<crate::note_analysis::LlmStatus, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let state = core.backend();
        let endpoint = match endpoint {
            Some(endpoint) => endpoint,
            None => state.db()?.get_settings()?.notes_analysis_endpoint,
        };
        Ok(crate::note_analysis::check_status(&endpoint))
    })
    .await
}

#[tauri::command]
pub async fn github_status(core: tauri::State<'_, Core>) -> Result<GithubStatus, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let settings = core.backend().db()?.get_settings()?;
        let service = core.flavor().identifier();
        Ok(GithubStatus {
            configured: crate::github_oauth::is_configured(),
            // A stored access token is the source of truth for "connected"; the
            // login is just for display.
            connected: crate::github_oauth::has_stored_token(service),
            username: settings.github_account_login,
            repo: settings.github_repo,
        })
    })
    .await
}

/// Step 1 of the GitHub device flow: requests a device + user code, opens the
/// verification URL in the browser, and returns the code for the UI to show.
/// Does NOT block on polling — the frontend then calls `github_device_poll`.
#[tauri::command]
pub async fn github_device_start(
    core: tauri::State<'_, Core>,
) -> Result<GithubDeviceStart, CommandError> {
    let device = tauri::async_runtime::spawn_blocking(crate::github_oauth::request_device_code)
        .await
        .map_err(|error| CommandError::new("github_auth_failed", error.to_string()))??;

    // Best-effort: open the verification page so the user just types the code.
    crate::github_oauth::open_verification_page(core.host().as_ref(), &device.verification_uri);

    core.backend()
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
    core: tauri::State<'_, Core>,
    device_code: String,
    interval: u64,
) -> Result<AppSettings, CommandError> {
    let cancelled = core
        .backend()
        .github_device_flows()
        .get(&device_code)
        .ok_or_else(|| {
            CommandError::new(
                "github_auth_cancelled",
                "This GitHub sign-in attempt is no longer active.",
            )
        })?;
    let service = core.flavor().identifier().to_string();
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
    core.backend().github_device_flows().finish(&device_code);
    poll_result?;

    // Fetch the login for display (best-effort: an empty login is fine).
    let login_service = service.clone();
    let login = tauri::async_runtime::spawn_blocking(move || {
        crate::github_oauth::fetch_login(&login_service)
    })
    .await
    .map_err(|error| CommandError::new("github_auth_failed", error.to_string()))?
    .unwrap_or_default();

    off_main_core(core.inner().clone(), move |core| {
        crate::github_oauth::save_connected_login(core.backend(), core.host().as_ref(), login)
    })
    .await
}

/// Cancels an active GitHub device flow and prevents its token from being
/// persisted if authorization completes after the user cancels.
#[tauri::command]
pub fn github_device_cancel(
    core: tauri::State<'_, Core>,
    device_code: String,
) -> Result<(), CommandError> {
    core.backend().github_device_flows().cancel(&device_code);
    Ok(())
}

/// Clears the stored GitHub token and fully resets the GitHub backup settings:
/// both backup toggles off, the connected login and target repo cleared. A later
/// reconnect then starts clean rather than silently resuming pushes to a stale
/// repo. Returns the updated settings.
#[tauri::command]
pub async fn github_disconnect(core: tauri::State<'_, Core>) -> Result<AppSettings, CommandError> {
    let service = core.flavor().identifier().to_string();
    tauri::async_runtime::spawn_blocking(move || crate::github_oauth::sign_out(&service))
        .await
        .map_err(|error| CommandError::new("github_auth_failed", error.to_string()))??;

    off_main_core(core.inner().clone(), move |core| {
        crate::github_oauth::reset_backup_settings(core.backend(), core.host().as_ref())
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
    core: tauri::State<'_, Core>,
) -> Result<crate::github_backup::SyncReport, CommandError> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Serialize against the debounce worker so the same daily file is never
        // PUT concurrently. Held for the whole sync; recovers from a poisoned
        // lock (the guarded value is `()`).
        let _guard = core.backend().github_sync_lock();
        let attempt = crate::note_sync::collect_sync_attempt_with(&core);
        crate::note_sync::record_sync_attempt_with(
            &core,
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
    core: tauri::State<'_, Core>,
) -> Result<Option<crate::sync_history::GitHubSyncActivity>, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        core.backend().db()?.get_github_sync_activity()
    })
    .await
}
