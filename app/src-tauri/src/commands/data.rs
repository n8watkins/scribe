//! Commands of the `data` domain (db, settings_store, data_hygiene, export,
//! import, stats, release_notes, diagnostics, replacements_file).
//!
//! Converted to the engine handle: commands take the managed [`Core`]. The
//! ones that still take the `AppHandle` need it for the opener plugin, for
//! the settings side effects of other domains (see
//! `settings_store::patch_settings_now_with`), or for `on_dialog_thread`.

use tauri::State;
use tauri_plugin_opener::OpenerExt;

use super::{off_main_core, on_dialog_thread};
use crate::{
    data_hygiene::{
        effective_data_dir_with, failed_recordings_dir_with, logs_dir_with, open_folder,
    },
    db::transcript_audio_missing,
    error::CommandError,
    import::{load_transcript_import, ImportReport, TranscriptImportPreview},
    replacements_file::{notify_backup_with, replacements_error},
    settings::AppSettings,
    settings_store::patch_settings_now_with,
    stats::BasicStats,
    transcript::{Transcript, TranscriptSearchResult, TranscriptSort},
    Core,
};

#[tauri::command]
pub async fn get_settings(core: State<'_, Core>) -> Result<AppSettings, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        core.backend().db()?.get_settings()
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
    core: State<'_, Core>,
    patch: serde_json::Map<String, serde_json::Value>,
) -> Result<AppSettings, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        patch_settings_now_with(&app, core, &patch)
    })
    .await
}

#[tauri::command]
pub async fn get_last_transcript(
    core: State<'_, Core>,
) -> Result<Option<Transcript>, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        core.backend().last_transcript()
    })
    .await
}

#[tauri::command]
pub async fn clear_last_transcript(core: State<'_, Core>) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let state = core.backend();
        state.set_unsaved_last_transcript(None);
        state.db()?.clear_last_transcript()
    })
    .await
}

#[tauri::command]
pub async fn list_recent_transcripts(
    core: State<'_, Core>,
    limit: Option<u32>,
) -> Result<Vec<Transcript>, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let limit = limit.unwrap_or(20).clamp(1, 100);
        core.backend().db()?.list_recent_transcripts(limit)
    })
    .await
}

#[tauri::command]
// Tauri maps these flat parameters directly from the stable frontend command
// payload, so grouping them would be a breaking IPC contract change.
#[allow(clippy::too_many_arguments)]
pub async fn search_transcripts(
    core: State<'_, Core>,
    query: Option<String>,
    notes_only: Option<bool>,
    from: Option<String>,
    to: Option<String>,
    sort: Option<TranscriptSort>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<TranscriptSearchResult, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let limit = limit.unwrap_or(20).clamp(1, 100);
        let offset = offset.unwrap_or_default();
        core.backend().db()?.search_transcripts(
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
    core: State<'_, Core>,
    id: String,
) -> Result<Option<Transcript>, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        core.backend().db()?.get_transcript_by_id(&id)
    })
    .await
}

#[tauri::command]
pub async fn update_transcript(
    core: State<'_, Core>,
    id: String,
    text: String,
) -> Result<Transcript, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        core.backend().db()?.update_transcript(&id, &text)
    })
    .await
}

#[tauri::command]
pub async fn delete_transcript(core: State<'_, Core>, id: String) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let state = core.backend();
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
pub async fn clear_transcript_history(core: State<'_, Core>) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let state = core.backend();
        state.set_unsaved_last_transcript(None);
        let db = state.db()?;
        db.clear_transcript_history()?;
        crate::data_hygiene::clear_history_leftovers(&db, state.dirs())
    })
    .await
}

#[tauri::command]
pub async fn clear_notes(core: State<'_, Core>) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let db = core.backend().db()?;
        db.clear_notes()?;
        db.vacuum()
    })
    .await
}

/// Loads the given transcripts, orders them oldest-first, and joins their text
/// with `separator` (default "\n\n"). Ids that don't resolve are skipped.
#[tauri::command]
pub async fn combine_transcripts(
    core: State<'_, Core>,
    ids: Vec<String>,
    separator: Option<String>,
) -> Result<String, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let separator = separator.unwrap_or_else(|| "\n\n".to_string());
        core.backend().db()?.combine_transcripts(&ids, &separator)
    })
    .await
}

/// Persists `text` as a new (non-note) history entry and makes it the Last
/// Transcript Buffer, mirroring how a dictation is saved. Returns the saved
/// transcript.
#[tauri::command]
pub async fn save_combined_transcript(
    core: State<'_, Core>,
    text: String,
) -> Result<Transcript, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let transcript = Transcript::new_last_buffer(text, None, None, None).ok_or_else(|| {
            CommandError::new(
                "empty_transcript",
                "Cannot save an empty combined transcript.",
            )
        })?;
        let db = core.backend().db()?;
        let history_enabled = db.get_settings()?.history_enabled;
        db.save_last_transcript_with_history(&transcript, history_enabled)?;
        Ok(transcript)
    })
    .await
}

/// Writes a transcript's text to a temp `.txt` file under the app cache dir and
/// opens it with the OS default text app. Takes the `AppHandle` for the
/// opener plugin.
#[tauri::command]
pub async fn open_transcript_externally(
    app: tauri::AppHandle,
    core: State<'_, Core>,
    id: String,
) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let state = core.backend();
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
    core: State<'_, Core>,
    id: String,
) -> Result<String, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let db = core.backend().db()?;
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

#[tauri::command]
pub async fn get_basic_stats(core: State<'_, Core>) -> Result<BasicStats, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        core.backend().db()?.get_basic_stats()
    })
    .await
}

#[tauri::command]
pub async fn refresh_basic_stats(core: State<'_, Core>) -> Result<BasicStats, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        core.backend().db()?.get_basic_stats()
    })
    .await
}

/// Opens the data folder (the one the user chose, else the app data folder).
/// Takes the `AppHandle` for the opener plugin.
#[tauri::command]
pub async fn open_data_folder(
    app: tauri::AppHandle,
    core: State<'_, Core>,
) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        open_folder(&app, effective_data_dir_with(core))
    })
    .await
}

/// Returns the current effective data directory as a display string, so the
/// Data & Privacy view can show the path prominently.
#[tauri::command]
pub async fn get_data_dir(core: State<'_, Core>) -> Result<String, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        Ok(effective_data_dir_with(core).to_string_lossy().into_owned())
    })
    .await
}

/// Opens a native folder picker so the user can choose a new data directory.
/// Returns the chosen absolute path, or None when the user cancels. Saving the
/// choice is the frontend's job (it writes `dataDir` via update_settings),
/// matching how the pill persists its position. Takes the `AppHandle` for
/// `on_dialog_thread`.
#[tauri::command]
pub async fn pick_data_dir(
    app: tauri::AppHandle,
    core: State<'_, Core>,
) -> Result<Option<String>, CommandError> {
    let core = core.inner().clone();
    on_dialog_thread(app, move |_app| {
        #[cfg(windows)]
        {
            let start = effective_data_dir_with(&core);
            let picked = rfd::FileDialog::new()
                .set_title("Choose Scribe data folder")
                .set_directory(&start)
                .pick_folder();
            Ok(picked.map(|path| path.to_string_lossy().into_owned()))
        }

        #[cfg(not(windows))]
        {
            let _ = core;
            Err(CommandError::new(
                "folder_picker_unsupported",
                "The folder picker is only available in the Windows build.",
            ))
        }
    })
    .await
}

/// Opens the folder that holds Scribe's rotating local log files, so a user can
/// find and attach them to a bug report. Takes the `AppHandle` for the
/// opener plugin.
#[tauri::command]
pub async fn open_logs_folder(
    app: tauri::AppHandle,
    core: State<'_, Core>,
) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        open_folder(&app, logs_dir_with(core.host().as_ref()))
    })
    .await
}

/// Returns the logs directory as a display string for the Data & Privacy view,
/// mirroring get_data_dir.
#[tauri::command]
pub fn get_logs_dir(core: State<'_, Core>) -> Result<String, CommandError> {
    Ok(logs_dir_with(core.host().as_ref())
        .to_string_lossy()
        .into_owned())
}

/// Opens the folder that holds audio from dictations whose transcription failed,
/// so a user can recover the recording from disk (risk R2 follow-up to Fix E).
/// Takes the `AppHandle` for the opener plugin.
#[tauri::command]
pub async fn open_failed_recordings_folder(
    app: tauri::AppHandle,
    core: State<'_, Core>,
) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        open_folder(&app, failed_recordings_dir_with(core.host().as_ref()))
    })
    .await
}

/// Returns the failed-recordings directory as a display string for the Data &
/// Privacy view, mirroring get_logs_dir.
#[tauri::command]
pub fn get_failed_recordings_dir(core: State<'_, Core>) -> Result<String, CommandError> {
    Ok(failed_recordings_dir_with(core.host().as_ref())
        .to_string_lossy()
        .into_owned())
}

/// Exports transcripts to a local file the user picks via a native save dialog.
/// `scope` selects which rows ("all" | "notes" | "dictation"); `format` selects
/// the renderer ("markdown" | "csv" | "json"). Returns the saved absolute path,
/// or None when the user cancels the dialog. Unlike the GitHub backup this needs
/// no connected account — it is a purely local save. Takes the `AppHandle` for
/// `on_dialog_thread`.
#[tauri::command]
pub async fn export_transcripts(
    app: tauri::AppHandle,
    core: State<'_, Core>,
    scope: String,
    format: String,
) -> Result<Option<String>, CommandError> {
    let core = core.inner().clone();
    on_dialog_thread(app, move |_app| {
        // Fetch the rows for the scope. A large cap so a normal history exports in
        // one pass; notes/dictation reuse the same filtered queries the rest of the
        // app uses.
        let db = core.backend().db()?;
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
            let start = effective_data_dir_with(&core);
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
            let _ = (core, contents, default_name);
            Err(CommandError::new(
                "save_dialog_unsupported",
                "The export save dialog is only available in the Windows build.",
            ))
        }
    })
    .await
}

/// Opens a native picker and validates a Scribe JSON export without changing
/// the database. The returned counts drive the explicit confirmation UI.
/// Takes the `AppHandle` for `on_dialog_thread`.
#[tauri::command]
pub async fn preview_transcript_import(
    app: tauri::AppHandle,
    core: State<'_, Core>,
) -> Result<Option<TranscriptImportPreview>, CommandError> {
    let core = core.inner().clone();
    on_dialog_thread(app, move |_app| {
        #[cfg(windows)]
        {
            let start = effective_data_dir_with(&core);
            let picked = rfd::FileDialog::new()
                .set_title("Restore Scribe backup")
                .set_directory(&start)
                .add_filter("Scribe JSON export", &["json"])
                .pick_file();
            let Some(path) = picked else {
                return Ok(None);
            };
            let loaded = load_transcript_import(&path)?;
            let conflicts = core
                .backend()
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
            let _ = core;
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
    core: State<'_, Core>,
    path: String,
    replace_existing: bool,
    expected_fingerprint: String,
) -> Result<ImportReport, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let loaded = load_transcript_import(std::path::Path::new(&path))?;
        crate::import::verify_fingerprint(&loaded.fingerprint, &expected_fingerprint)?;
        core.backend()
            .db()?
            .restore_transcripts(&loaded.prepared.transcripts, replace_existing)
    })
    .await
}

/// The replacements in replacements.txt, and any lines that couldn't be read.
#[tauri::command]
pub async fn get_replacements(
    core: State<'_, Core>,
) -> Result<crate::replacements_file::ReplacementsInfo, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        Ok(core.replacements().current())
    })
    .await
}

/// Appends replacements whose heard phrase isn't in the file yet.
#[tauri::command]
pub async fn add_replacements(
    core: State<'_, Core>,
    rows: Vec<crate::text_replace::TextReplacement>,
) -> Result<crate::replacements_file::AddReport, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let report = core.replacements().add(&rows).map_err(replacements_error)?;
        notify_backup_with(core);
        Ok(report)
    })
    .await
}

/// Removes the first line holding this replacement.
#[tauri::command]
pub async fn remove_replacement(
    core: State<'_, Core>,
    row: crate::text_replace::TextReplacement,
) -> Result<crate::replacements_file::ReplacementsInfo, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let info = core
            .replacements()
            .remove(&row)
            .map_err(replacements_error)?;
        notify_backup_with(core);
        Ok(info)
    })
    .await
}

/// Opens replacements.txt in the default text editor. Takes the `AppHandle`
/// for the opener plugin.
#[tauri::command]
pub async fn open_replacements_file(
    app: tauri::AppHandle,
    core: State<'_, Core>,
) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let path = core.replacements().path().to_path_buf();
        app.opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(|error| {
                CommandError::new(
                    "replacements_open_failed",
                    format!("Could not open replacements.txt. {}", error),
                )
            })
    })
    .await
}

/// The running version when its "What's new" card has not been dismissed yet.
/// Takes the `AppHandle` for the app's version, which Dev builds override.
#[tauri::command]
pub fn whats_new_pending(
    app: tauri::AppHandle,
    core: State<'_, Core>,
) -> Result<Option<String>, CommandError> {
    crate::release_notes::pending_version_with(&core, &app.package_info().version.to_string())
}

/// Remembers that the card for `version` was dismissed.
#[tauri::command]
pub fn dismiss_whats_new(core: State<'_, Core>, version: String) -> Result<(), CommandError> {
    let version = version.trim();
    if version.is_empty() || version.len() > 64 {
        return Err(CommandError::new("invalid_version", "Invalid app version."));
    }
    core.backend().db()?.set_whats_new_seen_version(version)
}

/// Builds the diagnostics report. Takes the `AppHandle` for the app's
/// version, which Dev builds override.
#[tauri::command]
pub fn collect_diagnostics(
    app: tauri::AppHandle,
    core: State<'_, Core>,
) -> Result<String, CommandError> {
    crate::diagnostics::collect_with(&core, &app.package_info().version.to_string())
}
