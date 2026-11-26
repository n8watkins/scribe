use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    error::CommandError,
    import::ImportReport,
    models::{ModelRecord, ModelStatus},
    settings::AppSettings,
    stats::BasicStats,
    sync_history::GitHubSyncActivity,
    transcript::{metadata_for_text, Transcript, TranscriptSearchResult, TranscriptSort},
};

const INITIAL_MIGRATION: &str = include_str!("../migrations/001_initial.sql");
const AUDIO_CLIPS_MIGRATION: &str = include_str!("../migrations/002_audio_clips.sql");
const NOTES_MIGRATION: &str = include_str!("../migrations/003_notes.sql");
const NOTE_ANALYSIS_MIGRATION: &str = include_str!("../migrations/004_note_analysis.sql");
const SETTINGS_KEY: &str = "app_settings";
/// Where a corrupt `app_settings` value is preserved before self-healing to
/// defaults. Fixed (not timestamped) so it can never accumulate without bound.
const CORRUPT_SETTINGS_KEY: &str = "app_settings_corrupt";
const LAST_TRANSCRIPT_ID_KEY: &str = "last_transcript_id";
const LAST_TRANSCRIPT_BUFFER_KEY: &str = "last_transcript_buffer";
const GITHUB_SYNC_ACTIVITY_KEY: &str = "github_sync_activity";
/// The app version whose "What's new" card was last dismissed (or skipped on
/// a fresh install). Its own key, so whole-settings saves never overwrite it.
const WHATS_NEW_SEEN_KEY: &str = "whats_new_seen_version";

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CommandError> {
        let conn = Connection::open(path).map_err(CommandError::database)?;
        apply_migrations(&conn)?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self, CommandError> {
        let conn = Connection::open_in_memory().map_err(CommandError::database)?;
        apply_migrations(&conn)?;
        Ok(Self { conn })
    }

    pub fn get_settings(&self) -> Result<AppSettings, CommandError> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()
            .map_err(CommandError::database)?;

        match value {
            Some(value) => match serde_json::from_str::<AppSettings>(&value) {
                Ok(settings) => Ok(settings),
                Err(error) => {
                    // A corrupt or partly unreadable settings value must never
                    // make the app unlaunchable, and one bad field (say an enum
                    // value from a newer build) must not cost the user every
                    // other choice. Keep each field that still reads, fall back
                    // to the default only for the ones that don't, back up the
                    // raw value for diagnosis, and persist the repaired
                    // settings so later launches are clean.
                    let (settings, dropped) = lenient_settings(&value);
                    // The repaired settings are saved below, which validates;
                    // a combination that reads but doesn't validate must not
                    // turn into a startup failure, so it falls back to defaults.
                    let settings = if settings.validate().is_ok() {
                        settings
                    } else {
                        AppSettings::default()
                    };
                    log::error!(
                        "Stored settings could not be read as a whole ({}); reset {} to defaults and kept the rest. Raw value: {}",
                        error,
                        if dropped.is_empty() {
                            "everything".to_string()
                        } else {
                            dropped.join(", ")
                        },
                        value
                    );
                    // A single fixed key (overwritten, not timestamped): if
                    // save_settings below keeps failing across launches, for
                    // example an unwritable DB, this can never accumulate
                    // unbounded backup rows. The row's updated_at records when
                    // it was captured.
                    if let Err(backup_error) = self.upsert_setting(CORRUPT_SETTINGS_KEY, &value) {
                        log::warn!(
                            "Could not back up corrupt settings: {}",
                            backup_error.message
                        );
                    }
                    self.save_settings(&settings)?;
                    Ok(settings)
                }
            },
            None => {
                let settings = AppSettings::default();
                self.save_settings(&settings)?;
                Ok(settings)
            }
        }
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), CommandError> {
        settings
            .validate()
            .map_err(CommandError::invalid_settings)?;

        self.upsert_setting(SETTINGS_KEY, &serde_json::to_string(settings)?)?;
        Ok(())
    }

    pub fn save_settings_and_clear_github_activity(
        &mut self,
        settings: &AppSettings,
    ) -> Result<(), CommandError> {
        settings
            .validate()
            .map_err(CommandError::invalid_settings)?;
        let transaction = self.conn.transaction().map_err(CommandError::database)?;
        transaction
            .execute(
                "INSERT INTO settings (key, value, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET
                    value = excluded.value,
                    updated_at = excluded.updated_at",
                params![
                    SETTINGS_KEY,
                    serde_json::to_string(settings)?,
                    Utc::now().to_rfc3339()
                ],
            )
            .map_err(CommandError::database)?;
        transaction
            .execute(
                "DELETE FROM settings WHERE key = ?1",
                [GITHUB_SYNC_ACTIVITY_KEY],
            )
            .map_err(CommandError::database)?;
        transaction.commit().map_err(CommandError::database)
    }

    pub fn get_github_sync_activity(&self) -> Result<Option<GitHubSyncActivity>, CommandError> {
        let Some(value) = self.get_setting_value(GITHUB_SYNC_ACTIVITY_KEY)? else {
            return Ok(None);
        };
        match serde_json::from_str(&value) {
            Ok(activity) => Ok(Some(activity)),
            Err(error) => {
                log::warn!("Ignoring corrupt GitHub sync activity: {error}");
                Ok(None)
            }
        }
    }

    pub fn save_github_sync_activity(
        &self,
        activity: &GitHubSyncActivity,
    ) -> Result<(), CommandError> {
        self.upsert_setting(GITHUB_SYNC_ACTIVITY_KEY, &serde_json::to_string(activity)?)
    }

    pub fn clear_github_sync_activity(&self) -> Result<(), CommandError> {
        self.conn
            .execute(
                "DELETE FROM settings WHERE key = ?1",
                [GITHUB_SYNC_ACTIVITY_KEY],
            )
            .map_err(CommandError::database)?;
        Ok(())
    }

    pub fn list_recent_transcripts(&self, limit: u32) -> Result<Vec<Transcript>, CommandError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, text, created_at, duration_ms, word_count, character_count,
                        model_id, language, output_mode, paste_method, transcription_latency_ms,
                        audio_path, is_note, analysis, analysis_model, analysis_created_at
                 FROM transcripts
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )
            .map_err(CommandError::database)?;

        let rows = stmt
            .query_map([limit], transcript_from_row)
            .map_err(CommandError::database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(CommandError::database)
    }

    /// Searches the history with optional full-text (`LIKE`), notes-only,
    /// created_at date-range (RFC3339; `from` inclusive, `to` exclusive), and
    /// sort, paginated by `limit`/`offset`. All user-supplied values are passed
    /// as bound params; only the fixed WHERE fragments and the constant ORDER BY
    /// column list are interpolated, so user text can never reach the SQL string.
    // This mirrors the public search contract and keeps all optional filters
    // explicit at call sites.
    #[allow(clippy::too_many_arguments)]
    pub fn search_transcripts(
        &self,
        query: Option<&str>,
        notes_only: bool,
        from: Option<&str>,
        to: Option<&str>,
        sort: TranscriptSort,
        limit: u32,
        offset: u32,
    ) -> Result<TranscriptSearchResult, CommandError> {
        use rusqlite::types::ToSql;

        let normalized_query = query.map(str::trim).filter(|query| !query.is_empty());
        let normalized_from = from.map(str::trim).filter(|value| !value.is_empty());
        let normalized_to = to.map(str::trim).filter(|value| !value.is_empty());

        // Build the shared WHERE clause and its bound params in lockstep so the
        // COUNT and the page query agree on the filter (the page query then
        // appends LIMIT/OFFSET params after these).
        let mut conditions: Vec<String> = Vec::new();
        let mut filter_params: Vec<Box<dyn ToSql>> = Vec::new();

        if let Some(query) = normalized_query {
            conditions.push("text LIKE ? ESCAPE '\\'".to_string());
            filter_params.push(Box::new(format!("%{}%", escape_like_query(query))));
        }
        if notes_only {
            conditions.push("is_note = 1".to_string());
        }
        // created_at is an RFC3339 UTC string; lexicographic compares match
        // chronological order for that format, so plain string bounds work.
        // The lower bound is inclusive (`>=`); the upper bound is EXCLUSIVE
        // (`<`) so callers pass the next local-day midnight as a clean
        // half-open end (this also avoids dropping rows whose fractional-second
        // width differs from the bound's).
        if let Some(from) = normalized_from {
            conditions.push("created_at >= ?".to_string());
            filter_params.push(Box::new(from.to_string()));
        }
        if let Some(to) = normalized_to {
            conditions.push("created_at < ?".to_string());
            filter_params.push(Box::new(to.to_string()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let count_sql = format!("SELECT COUNT(*) FROM transcripts {}", where_clause);
        let filter_refs: Vec<&dyn ToSql> = filter_params.iter().map(|p| p.as_ref()).collect();
        let total: i64 = self
            .conn
            .query_row(&count_sql, filter_refs.as_slice(), |row| row.get(0))
            .map_err(CommandError::database)?;

        let page_sql = format!(
            "SELECT id, text, created_at, duration_ms, word_count, character_count,
                    model_id, language, output_mode, paste_method, transcription_latency_ms,
                    audio_path, is_note, analysis, analysis_model, analysis_created_at
             FROM transcripts
             {}
             ORDER BY {}
             LIMIT ? OFFSET ?",
            where_clause,
            sort.order_by_clause()
        );

        // The page query reuses the filter params, then limit + offset.
        let mut page_params = filter_params;
        page_params.push(Box::new(limit));
        page_params.push(Box::new(offset));
        let page_refs: Vec<&dyn ToSql> = page_params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self
            .conn
            .prepare(&page_sql)
            .map_err(CommandError::database)?;
        let rows = stmt
            .query_map(page_refs.as_slice(), transcript_from_row)
            .map_err(CommandError::database)?;
        let transcripts = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(CommandError::database)?;

        Ok(TranscriptSearchResult {
            transcripts,
            total: total.max(0) as u32,
            limit,
            offset,
        })
    }

    /// Fetches DICTATION transcripts only (`is_note = 0`), newest-first,
    /// paginated by `limit`/`offset`. The mirror of the `notes_only` path on
    /// `search_transcripts`, used by the all-transcripts GitHub backup (notes
    /// sync via the separate notes log, so they are excluded here).
    pub fn search_dictation_transcripts(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<TranscriptSearchResult, CommandError> {
        let total: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transcripts WHERE COALESCE(is_note, 0) = 0",
                [],
                |row| row.get(0),
            )
            .map_err(CommandError::database)?;

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, text, created_at, duration_ms, word_count, character_count,
                        model_id, language, output_mode, paste_method, transcription_latency_ms,
                        audio_path, is_note, analysis, analysis_model, analysis_created_at
                 FROM transcripts
                 WHERE COALESCE(is_note, 0) = 0
                 ORDER BY created_at DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .map_err(CommandError::database)?;
        let rows = stmt
            .query_map([limit, offset], transcript_from_row)
            .map_err(CommandError::database)?;
        let transcripts = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(CommandError::database)?;

        Ok(TranscriptSearchResult {
            transcripts,
            total: total.max(0) as u32,
            limit,
            offset,
        })
    }

    /// Loads the given transcripts, orders them oldest-first by `created_at`,
    /// and joins their text with `separator`. Ids that don't resolve are
    /// skipped; an empty result (no id resolved) is an error.
    pub fn combine_transcripts(
        &self,
        ids: &[String],
        separator: &str,
    ) -> Result<String, CommandError> {
        let mut found: Vec<Transcript> = Vec::new();
        for id in ids {
            if let Some(transcript) = self.get_transcript_by_id(id)? {
                found.push(transcript);
            }
        }

        if found.is_empty() {
            return Err(CommandError::new(
                "transcript_not_found",
                "None of the selected transcripts could be found in local history.",
            ));
        }

        found.sort_by_key(|transcript| transcript.created_at);
        let merged = found
            .iter()
            .map(|transcript| transcript.text.as_str())
            .collect::<Vec<_>>()
            .join(separator);
        Ok(merged)
    }

    pub fn get_last_transcript(&self) -> Result<Option<Transcript>, CommandError> {
        if let Some(value) = self.get_setting_value(LAST_TRANSCRIPT_BUFFER_KEY)? {
            if !value.trim().is_empty() {
                let transcript = serde_json::from_str(&value)?;
                return Ok(Some(transcript));
            }
        }

        let Some(id) = self.get_setting_value(LAST_TRANSCRIPT_ID_KEY)? else {
            return Ok(None);
        };

        if id.trim().is_empty() {
            return Ok(None);
        }

        self.get_transcript_by_id(&id)
    }

    pub fn clear_last_transcript(&self) -> Result<(), CommandError> {
        self.remove_buffer_only_clip()?;
        self.upsert_setting(LAST_TRANSCRIPT_BUFFER_KEY, "")?;
        self.upsert_setting(LAST_TRANSCRIPT_ID_KEY, "")?;
        Ok(())
    }

    /// A clip referenced only by the Last Transcript Buffer (its dictation
    /// was never written to history) has no other owner, so it must be
    /// removed before the buffer stops referencing it. Clips that also live
    /// in the transcripts table are left for the history deletion paths.
    fn remove_buffer_only_clip(&self) -> Result<(), CommandError> {
        let Some(value) = self.get_setting_value(LAST_TRANSCRIPT_BUFFER_KEY)? else {
            return Ok(());
        };
        if value.trim().is_empty() {
            return Ok(());
        }
        let Ok(buffer) = serde_json::from_str::<Transcript>(&value) else {
            return Ok(());
        };
        let Some(path) = buffer.audio_path else {
            return Ok(());
        };
        if self.get_transcript_by_id(&buffer.id)?.is_none() {
            remove_clip_files([path]);
        }
        Ok(())
    }

    #[allow(dead_code)]
    /// Saves a note transcript to history only: notes never replace the Last
    /// Transcript Buffer (Ctrl+Alt+V should keep pasting the last dictation),
    /// and they are saved even with history disabled - taking a note is an
    /// explicit ask to keep it.
    pub fn save_note_transcript(&self, transcript: &Transcript) -> Result<(), CommandError> {
        self.insert_transcript(transcript)
    }

    pub fn save_last_transcript(&self, transcript: &Transcript) -> Result<(), CommandError> {
        self.save_last_transcript_with_history(transcript, true)
    }

    pub fn save_last_transcript_with_history(
        &self,
        transcript: &Transcript,
        history_enabled: bool,
    ) -> Result<(), CommandError> {
        if transcript.text.trim().is_empty() {
            return Ok(());
        }

        self.remove_buffer_only_clip()?;
        self.upsert_setting(
            LAST_TRANSCRIPT_BUFFER_KEY,
            &serde_json::to_string(transcript)?,
        )?;
        if history_enabled {
            self.insert_transcript(transcript)?;
            let settings = self.get_settings()?;
            self.enforce_history_retention(settings.history_retention_days)?;
            self.enforce_notes_retention(settings.notes_retention_days)?;
        }
        self.upsert_setting(LAST_TRANSCRIPT_ID_KEY, &transcript.id)?;
        Ok(())
    }

    pub fn count_existing_transcripts(
        &self,
        transcripts: &[Transcript],
    ) -> Result<u32, CommandError> {
        let mut statement = self
            .conn
            .prepare("SELECT 1 FROM transcripts WHERE id = ?1 LIMIT 1")
            .map_err(CommandError::database)?;
        let mut count = 0_u32;
        for transcript in transcripts {
            let exists = statement
                .query_row([&transcript.id], |_| Ok(()))
                .optional()
                .map_err(CommandError::database)?
                .is_some();
            count += u32::from(exists);
        }
        Ok(count)
    }

    /// Restores prepared transcripts in one transaction. Existing ids are
    /// skipped by default. Explicit replacement updates transcript metadata
    /// while preserving any local audio path already attached to that row.
    pub fn restore_transcripts(
        &mut self,
        transcripts: &[Transcript],
        replace_existing: bool,
    ) -> Result<ImportReport, CommandError> {
        let transaction = self.conn.transaction().map_err(CommandError::database)?;
        let mut imported = 0_u32;
        let mut skipped = 0_u32;
        let mut replaced = 0_u32;

        for transcript in transcripts {
            let exists = transaction
                .query_row(
                    "SELECT 1 FROM transcripts WHERE id = ?1 LIMIT 1",
                    [&transcript.id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(CommandError::database)?
                .is_some();
            if exists && !replace_existing {
                skipped += 1;
                continue;
            }

            if exists {
                write_restored_transcript(&transaction, transcript, true)
                    .map_err(CommandError::database)?;
                replaced += 1;
            } else {
                write_restored_transcript(&transaction, transcript, false)
                    .map_err(CommandError::database)?;
                imported += 1;
            }
        }

        transaction.commit().map_err(CommandError::database)?;
        Ok(ImportReport {
            imported,
            skipped,
            replaced,
        })
    }

    pub fn list_model_records(&self) -> Result<Vec<ModelRecord>, CommandError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, filename, local_path, size_bytes, status, checksum, selected, downloaded_at
                 FROM models",
            )
            .map_err(CommandError::database)?;

        let rows = stmt
            .query_map([], model_record_from_row)
            .map_err(CommandError::database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(CommandError::database)
    }

    pub fn get_model_record(&self, id: &str) -> Result<Option<ModelRecord>, CommandError> {
        self.conn
            .query_row(
                "SELECT id, name, filename, local_path, size_bytes, status, checksum, selected, downloaded_at
                 FROM models
                 WHERE id = ?1",
                [id],
                model_record_from_row,
            )
            .optional()
            .map_err(CommandError::database)
    }

    pub fn upsert_model_record(&self, record: &ModelRecord) -> Result<(), CommandError> {
        self.conn
            .execute(
                "INSERT INTO models (
                    id, name, filename, local_path, size_bytes, status, checksum, selected, downloaded_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    filename = excluded.filename,
                    local_path = excluded.local_path,
                    size_bytes = excluded.size_bytes,
                    status = excluded.status,
                    checksum = excluded.checksum,
                    selected = excluded.selected,
                    downloaded_at = excluded.downloaded_at",
                params![
                    &record.id,
                    &record.name,
                    &record.filename,
                    &record.local_path,
                    record.size_bytes.map(|value| value as i64),
                    record.status.as_db_value(),
                    &record.checksum,
                    record.selected as i64,
                    record.downloaded_at.as_ref().map(|date| date.to_rfc3339()),
                ],
            )
            .map_err(CommandError::database)?;

        Ok(())
    }

    pub fn mark_model_selected(&self, selected_model_id: &str) -> Result<(), CommandError> {
        self.conn
            .execute("UPDATE models SET selected = 0 WHERE selected != 0", [])
            .map_err(CommandError::database)?;
        self.conn
            .execute(
                "UPDATE models SET selected = 1, status = ?2 WHERE id = ?1",
                params![selected_model_id, ModelStatus::Selected.as_db_value()],
            )
            .map_err(CommandError::database)?;
        Ok(())
    }

    pub fn get_basic_stats(&self) -> Result<BasicStats, CommandError> {
        let today = Utc::now().date_naive();

        let (words_today, dictations_today, recording_today, latency_today): (
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        ) = self
            .conn
            .query_row(
                "SELECT
                    SUM(word_count),
                    COUNT(*),
                    SUM(COALESCE(duration_ms, 0)),
                    SUM(COALESCE(transcription_latency_ms, 0))
                 FROM transcripts
                 WHERE substr(created_at, 1, 10) = ?1",
                [today.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(CommandError::database)?;

        let total_words: Option<i64> = self
            .conn
            .query_row("SELECT SUM(word_count) FROM transcripts", [], |row| {
                row.get(0)
            })
            .map_err(CommandError::database)?;

        let most_used_model: Option<String> = self
            .conn
            .query_row(
                "SELECT model_id
                 FROM transcripts
                 WHERE model_id IS NOT NULL AND model_id != ''
                 GROUP BY model_id
                 ORDER BY COUNT(*) DESC, MAX(created_at) DESC
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(CommandError::database)?;

        let words_today = words_today.unwrap_or_default() as u32;
        let dictations_today = dictations_today.unwrap_or_default() as u32;

        let average_wpm = if recording_today.unwrap_or_default() > 0 {
            Some((words_today as f64) / (recording_today.unwrap_or_default() as f64 / 60_000.0))
        } else {
            None
        };

        Ok(BasicStats {
            words_today,
            dictations_today,
            average_wpm,
            average_transcription_latency_ms: average_for(latency_today, dictations_today),
            average_recording_duration_ms: average_for(recording_today, dictations_today),
            most_used_model,
            total_words_transcribed: total_words.unwrap_or_default() as u64,
        })
    }

    fn insert_transcript(&self, transcript: &Transcript) -> Result<(), CommandError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO transcripts (
                    id, text, created_at, duration_ms, word_count, character_count,
                    model_id, language, output_mode, paste_method, transcription_latency_ms,
                    audio_path, is_note, analysis, analysis_model, analysis_created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    transcript.id,
                    transcript.text,
                    transcript.created_at.to_rfc3339(),
                    transcript.duration_ms,
                    transcript.word_count,
                    transcript.character_count,
                    transcript.model_id,
                    transcript.language,
                    transcript.output_mode.as_ref().map(enum_to_json_string),
                    transcript.paste_method.as_ref().map(enum_to_json_string),
                    transcript.transcription_latency_ms,
                    transcript.audio_path,
                    transcript.is_note,
                    transcript.analysis,
                    transcript.analysis_model,
                    transcript
                        .analysis_created_at
                        .as_ref()
                        .map(|date| date.to_rfc3339()),
                ],
            )
            .map_err(CommandError::database)?;

        Ok(())
    }

    pub fn get_transcript_by_id(&self, id: &str) -> Result<Option<Transcript>, CommandError> {
        self.conn
            .query_row(
                "SELECT id, text, created_at, duration_ms, word_count, character_count,
                        model_id, language, output_mode, paste_method, transcription_latency_ms,
                        audio_path, is_note, analysis, analysis_model, analysis_created_at
                 FROM transcripts
                 WHERE id = ?1",
                [id],
                transcript_from_row,
            )
            .optional()
            .map_err(CommandError::database)
    }

    pub fn update_transcript(&self, id: &str, text: &str) -> Result<Transcript, CommandError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CommandError::new(
                "empty_transcript",
                "Transcript text cannot be empty.",
            ));
        }

        let Some(mut transcript) = self.get_transcript_by_id(id)? else {
            return Err(transcript_not_found(id));
        };

        let metadata = metadata_for_text(text);
        transcript.text = text.to_string();
        transcript.word_count = metadata.word_count;
        transcript.character_count = metadata.character_count;

        self.conn
            .execute(
                "UPDATE transcripts
                 SET text = ?2, word_count = ?3, character_count = ?4
                 WHERE id = ?1",
                params![
                    id,
                    &transcript.text,
                    transcript.word_count,
                    transcript.character_count
                ],
            )
            .map_err(CommandError::database)?;

        Ok(transcript)
    }

    /// Stores (or replaces) the local-LLM analysis of a transcript and
    /// returns the updated row.
    pub fn save_note_analysis(
        &self,
        id: &str,
        analysis: &str,
        model: &str,
    ) -> Result<Transcript, CommandError> {
        let updated = self
            .conn
            .execute(
                "UPDATE transcripts
                 SET analysis = ?2, analysis_model = ?3, analysis_created_at = ?4
                 WHERE id = ?1",
                params![id, analysis, model, Utc::now().to_rfc3339()],
            )
            .map_err(CommandError::database)?;
        if updated == 0 {
            return Err(transcript_not_found(id));
        }
        self.get_transcript_by_id(id)?
            .ok_or_else(|| transcript_not_found(id))
    }

    pub fn delete_transcript(&self, id: &str) -> Result<(), CommandError> {
        let clips = self.clip_paths(
            "SELECT audio_path FROM transcripts WHERE id = ?1 AND audio_path IS NOT NULL",
            [id],
        )?;
        self.conn
            .execute("DELETE FROM transcripts WHERE id = ?1", [id])
            .map_err(CommandError::database)?;
        remove_clip_files(clips);
        Ok(())
    }

    /// Clears dictation transcripts only. Notes (`is_note = 1`) are deliberate
    /// saves and are preserved — matching the confirmation copy and the
    /// separate Notes view; use `clear_notes` to delete those.
    pub fn clear_transcript_history(&self) -> Result<(), CommandError> {
        let clips = self.clip_paths(
            "SELECT audio_path FROM transcripts \
             WHERE audio_path IS NOT NULL AND COALESCE(is_note, 0) = 0",
            [],
        )?;
        self.conn
            .execute("DELETE FROM transcripts WHERE COALESCE(is_note, 0) = 0", [])
            .map_err(CommandError::database)?;
        remove_clip_files(clips);
        Ok(())
    }

    /// Clears notes only (`is_note = 1`), leaving dictation transcripts intact.
    pub fn clear_notes(&self) -> Result<(), CommandError> {
        let clips = self.clip_paths(
            "SELECT audio_path FROM transcripts \
             WHERE audio_path IS NOT NULL AND is_note = 1",
            [],
        )?;
        self.conn
            .execute("DELETE FROM transcripts WHERE is_note = 1", [])
            .map_err(CommandError::database)?;
        remove_clip_files(clips);
        Ok(())
    }

    pub fn enforce_history_retention(
        &self,
        retention_days: Option<u16>,
    ) -> Result<(), CommandError> {
        let Some(retention_days) = retention_days else {
            return Ok(());
        };

        let cutoff_time = Utc::now() - chrono::Duration::days(i64::from(retention_days));
        let cutoff = cutoff_time.to_rfc3339();

        // The Last Transcript Buffer holds a copy of the newest dictation, so
        // its text must not outlive that dictation's retention window.
        if self
            .buffer_transcript()?
            .is_some_and(|buffer| buffer.created_at < cutoff_time)
        {
            self.clear_last_transcript()?;
        }

        // Notes (is_note = 1) are deliberate saves and are never auto-pruned;
        // retention only ages out ordinary dictation transcripts.
        let clips = self.clip_paths(
            "SELECT audio_path FROM transcripts \
             WHERE created_at < ?1 AND audio_path IS NOT NULL AND COALESCE(is_note, 0) = 0",
            [&cutoff],
        )?;
        self.conn
            .execute(
                "DELETE FROM transcripts WHERE created_at < ?1 AND COALESCE(is_note, 0) = 0",
                [&cutoff],
            )
            .map_err(CommandError::database)?;
        remove_clip_files(clips);

        // Notes keep their text, but their audio follows the same window as
        // dictation clips, so recordings never pile up forever.
        let note_clips = self.clip_paths(
            "SELECT audio_path FROM transcripts \
             WHERE created_at < ?1 AND audio_path IS NOT NULL AND is_note = 1",
            [&cutoff],
        )?;
        self.conn
            .execute(
                "UPDATE transcripts SET audio_path = NULL \
                 WHERE created_at < ?1 AND audio_path IS NOT NULL AND is_note = 1",
                [&cutoff],
            )
            .map_err(CommandError::database)?;
        remove_clip_files(note_clips);
        Ok(())
    }

    /// Auto-prunes saved notes (`is_note = 1`) older than `retention_days`,
    /// the notes counterpart to `enforce_history_retention`. Notes default to
    /// no retention (kept forever); this is a no-op unless the user opted into
    /// a notes window. The two retentions never touch each other's rows.
    pub fn enforce_notes_retention(&self, retention_days: Option<u16>) -> Result<(), CommandError> {
        let Some(retention_days) = retention_days else {
            return Ok(());
        };

        let cutoff = (Utc::now() - chrono::Duration::days(i64::from(retention_days))).to_rfc3339();
        let clips = self.clip_paths(
            "SELECT audio_path FROM transcripts \
             WHERE created_at < ?1 AND audio_path IS NOT NULL AND is_note = 1",
            [&cutoff],
        )?;
        self.conn
            .execute(
                "DELETE FROM transcripts WHERE created_at < ?1 AND is_note = 1",
                [&cutoff],
            )
            .map_err(CommandError::database)?;
        remove_clip_files(clips);
        Ok(())
    }

    /// Every audio clip the database still points at: transcript and note
    /// clips plus the Last Transcript Buffer's clip.
    pub fn referenced_clip_paths(&self) -> Result<Vec<String>, CommandError> {
        let mut paths = self.clip_paths(
            "SELECT audio_path FROM transcripts WHERE audio_path IS NOT NULL",
            [],
        )?;
        if let Some(buffer) = self.buffer_transcript()? {
            paths.extend(buffer.audio_path);
        }
        Ok(paths)
    }

    /// Every file path the database stores: downloaded model files and all
    /// referenced audio clips.
    pub fn stored_file_paths(&self) -> Result<Vec<String>, CommandError> {
        let mut paths = self.clip_paths(
            "SELECT local_path FROM models WHERE local_path IS NOT NULL",
            [],
        )?;
        paths.extend(self.referenced_clip_paths()?);
        Ok(paths)
    }

    /// Points every stored reference to the file `old` at `new`: model rows,
    /// transcript clips and the Last Transcript Buffer. Returns how many
    /// references changed.
    pub fn replace_stored_path(&self, old: &str, new: &str) -> Result<usize, CommandError> {
        let mut changed = self
            .conn
            .execute(
                "UPDATE models SET local_path = ?2 WHERE local_path = ?1",
                [old, new],
            )
            .map_err(CommandError::database)?;
        changed += self
            .conn
            .execute(
                "UPDATE transcripts SET audio_path = ?2 WHERE audio_path = ?1",
                [old, new],
            )
            .map_err(CommandError::database)?;
        if let Some(mut buffer) = self.buffer_transcript()? {
            if buffer.audio_path.as_deref() == Some(old) {
                buffer.audio_path = Some(new.to_string());
                self.upsert_setting(LAST_TRANSCRIPT_BUFFER_KEY, &serde_json::to_string(&buffer)?)?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// Rebuilds the database file so space freed by deletions is returned to
    /// the disk and no deleted text survives in the file.
    pub fn vacuum(&self) -> Result<(), CommandError> {
        self.conn
            .execute_batch("VACUUM")
            .map_err(CommandError::database)
    }

    /// The Last Transcript Buffer as stored, if any and readable.
    fn buffer_transcript(&self) -> Result<Option<Transcript>, CommandError> {
        let Some(value) = self.get_setting_value(LAST_TRANSCRIPT_BUFFER_KEY)? else {
            return Ok(None);
        };
        if value.trim().is_empty() {
            return Ok(None);
        }
        Ok(serde_json::from_str::<Transcript>(&value).ok())
    }

    /// Audio-clip paths matched by `sql` (which must select exactly the
    /// audio_path column), collected before their rows are deleted.
    fn clip_paths<P: rusqlite::Params>(
        &self,
        sql: &str,
        params: P,
    ) -> Result<Vec<String>, CommandError> {
        let mut stmt = self.conn.prepare(sql).map_err(CommandError::database)?;
        let rows = stmt
            .query_map(params, |row| row.get(0))
            .map_err(CommandError::database)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(CommandError::database)
    }

    pub fn whats_new_seen_version(&self) -> Result<Option<String>, CommandError> {
        self.get_setting_value(WHATS_NEW_SEEN_KEY)
    }

    pub fn set_whats_new_seen_version(&self, version: &str) -> Result<(), CommandError> {
        self.upsert_setting(WHATS_NEW_SEEN_KEY, version)
    }

    /// True once settings have been saved at least once (any install that
    /// has been used), false on a fresh install that still runs on defaults.
    pub fn has_saved_settings(&self) -> Result<bool, CommandError> {
        Ok(self.get_setting_value(SETTINGS_KEY)?.is_some())
    }

    fn get_setting_value(&self, key: &str) -> Result<Option<String>, CommandError> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(CommandError::database)
    }

    fn upsert_setting(&self, key: &str, value: &str) -> Result<(), CommandError> {
        self.conn
            .execute(
                "INSERT INTO settings (key, value, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET
                    value = excluded.value,
                    updated_at = excluded.updated_at",
                params![key, value, Utc::now().to_rfc3339()],
            )
            .map_err(CommandError::database)?;

        Ok(())
    }
}

/// The schema version this build writes, stored in `PRAGMA user_version`.
/// Versions 1 to 4 are the four files in `migrations/`; a later migration gets
/// the next number and an entry in [`LATER_MIGRATIONS`].
const SCHEMA_VERSION: i32 = 4;

/// Migrations after the version-4 baseline, as `(version, sql)`. Each runs in
/// its own transaction together with the `user_version` bump, so an
/// interrupted migration rolls back and runs again on the next launch.
const LATER_MIGRATIONS: &[(i32, &str)] = &[];

/// Brings the database up to [`SCHEMA_VERSION`].
///
/// Databases from before versioning have `user_version` 0 and any subset of
/// the columns from migrations 2 to 4 (an interrupted multi-statement
/// migration could leave some added and some missing), so the baseline checks
/// each column instead of trusting an error message, adds only what is
/// missing, and stamps version 4, all in one transaction.
fn apply_migrations(conn: &Connection) -> Result<(), CommandError> {
    // WAL keeps readers and the writer from blocking each other and survives a
    // crash mid-write; secure_delete overwrites deleted rows so cleared text
    // does not linger in free pages. Both are per-connection or persistent
    // settings that are safe to repeat on every open. In-memory databases
    // report "memory" for the journal mode, which is fine.
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(CommandError::database)?;
    conn.pragma_update(None, "secure_delete", "ON")
        .map_err(CommandError::database)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(CommandError::database)?;

    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(CommandError::database)?;
    if version > SCHEMA_VERSION {
        // A newer Scribe wrote this database and the user went back to an older
        // build. Leave the schema alone rather than guess.
        log::warn!(
            "Database schema version {} is newer than this build ({}); opening it as is",
            version,
            SCHEMA_VERSION
        );
        return Ok(());
    }

    if version < 4 {
        run_in_transaction(conn, |tx| {
            tx.execute_batch(INITIAL_MIGRATION)?;
            for migration in [
                AUDIO_CLIPS_MIGRATION,
                NOTES_MIGRATION,
                NOTE_ANALYSIS_MIGRATION,
            ] {
                for (table, column, statement) in add_column_statements(migration) {
                    if !has_column(tx, table, column)? {
                        tx.execute_batch(statement)?;
                    }
                }
            }
            tx.pragma_update(None, "user_version", 4)
        })?;
        if version > 0 {
            log::info!("Upgraded database schema from version {} to 4", version);
        }
    }

    for (target, sql) in LATER_MIGRATIONS {
        let current: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(CommandError::database)?;
        if *target <= current {
            continue;
        }
        run_in_transaction(conn, |tx| {
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", *target)
        })?;
        log::info!("Applied database migration {}", target);
    }
    Ok(())
}

fn run_in_transaction(
    conn: &Connection,
    work: impl FnOnce(&Connection) -> rusqlite::Result<()>,
) -> Result<(), CommandError> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(CommandError::database)?;
    match work(conn) {
        Ok(()) => conn.execute_batch("COMMIT").map_err(CommandError::database),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(CommandError::database(error))
        }
    }
}

/// Splits a migration made of `ALTER TABLE <table> ADD COLUMN <column> ...;`
/// statements into `(table, column, statement)` triples.
fn add_column_statements(migration: &str) -> Vec<(&str, &str, &str)> {
    migration
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .filter_map(|statement| {
            let words: Vec<&str> = statement.split_whitespace().collect();
            match words.as_slice() {
                ["ALTER", "TABLE", table, "ADD", "COLUMN", column, ..] => {
                    Some((*table, *column, statement))
                }
                _ => None,
            }
        })
        .collect()
}

fn has_column(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let names = statement.query_map([], |row| row.get::<_, String>(1))?;
    for name in names {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Reads stored settings field by field, keeping every field that deserializes
/// and dropping the ones that don't. Returns the settings and the names of the
/// dropped fields (`hotkeys.toggleDictation` for a nested one).
///
/// Fields absent from the stored value keep their serde defaults exactly as a
/// normal read would (so `defaultsVersion` stays 0 when missing and the
/// one-time defaults migrations still run). A value that is not a JSON object
/// at all falls back to full defaults.
fn lenient_settings(raw: &str) -> (AppSettings, Vec<String>) {
    use serde_json::{Map, Value};

    let Ok(Value::Object(stored)) = serde_json::from_str::<Value>(raw) else {
        return (AppSettings::default(), Vec::new());
    };
    let reads = |object: &Map<String, Value>| {
        serde_json::from_value::<AppSettings>(Value::Object(object.clone())).is_ok()
    };
    let defaults = serde_json::to_value(AppSettings::default()).unwrap_or(Value::Null);

    let mut kept = Map::new();
    let mut dropped = Vec::new();
    for (key, value) in stored {
        let single = Map::from_iter([(key.clone(), value.clone())]);
        if reads(&single) {
            kept.insert(key, value);
            continue;
        }
        // A nested object (hotkeys, the custom theme) may have just one bad
        // member: rebuild it on top of its defaults, keeping each member that
        // reads.
        if let (Value::Object(members), Some(Value::Object(base))) = (&value, defaults.get(&key)) {
            let mut repaired = base.clone();
            for (member, member_value) in members {
                let mut candidate = repaired.clone();
                candidate.insert(member.clone(), member_value.clone());
                let probe = Map::from_iter([(key.clone(), Value::Object(candidate.clone()))]);
                if reads(&probe) {
                    repaired = candidate;
                } else {
                    dropped.push(format!("{}.{}", key, member));
                }
            }
            let probe = Map::from_iter([(key.clone(), Value::Object(repaired.clone()))]);
            if reads(&probe) {
                kept.insert(key, Value::Object(repaired));
                continue;
            }
        }
        dropped.push(key);
    }
    let settings = serde_json::from_value(Value::Object(kept)).unwrap_or_default();
    (settings, dropped)
}

/// Removes saved clip files for transcripts that are being deleted. A clip
/// that is already gone is not an error.
fn remove_clip_files(paths: impl IntoIterator<Item = String>) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

fn transcript_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Transcript> {
    let created_at: String = row.get(2)?;

    Ok(Transcript {
        id: row.get(0)?,
        text: row.get(1)?,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|date| date.with_timezone(&Utc))
            .unwrap_or_else(|_| {
                NaiveDate::from_ymd_opt(1970, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc()
            }),
        duration_ms: row.get(3)?,
        word_count: row.get(4)?,
        character_count: row.get(5)?,
        model_id: row.get(6)?,
        language: row.get(7)?,
        output_mode: optional_enum_from_json(row.get(8)?),
        paste_method: optional_enum_from_json(row.get(9)?),
        transcription_latency_ms: row.get(10)?,
        audio_path: row.get(11)?,
        is_note: row.get::<_, Option<bool>>(12)?.unwrap_or(false),
        analysis: row.get(13)?,
        analysis_model: row.get(14)?,
        analysis_created_at: row.get::<_, Option<String>>(15)?.and_then(|date| {
            DateTime::parse_from_rfc3339(&date)
                .ok()
                .map(|date| date.with_timezone(&Utc))
        }),
    })
}

fn write_restored_transcript(
    connection: &Connection,
    transcript: &Transcript,
    replace: bool,
) -> rusqlite::Result<usize> {
    let sql = if replace {
        "UPDATE transcripts SET
            text = ?2, created_at = ?3, duration_ms = ?4,
            word_count = ?5, character_count = ?6, model_id = ?7,
            language = ?8, output_mode = ?9, paste_method = ?10,
            transcription_latency_ms = ?11, is_note = ?12,
            analysis = ?13, analysis_model = ?14,
            analysis_created_at = ?15
         WHERE id = ?1"
    } else {
        "INSERT INTO transcripts (
            id, text, created_at, duration_ms, word_count,
            character_count, model_id, language, output_mode,
            paste_method, transcription_latency_ms, audio_path,
            is_note, analysis, analysis_model, analysis_created_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
            NULL, ?12, ?13, ?14, ?15
         )"
    };
    connection.execute(
        sql,
        params![
            &transcript.id,
            &transcript.text,
            transcript.created_at.to_rfc3339(),
            transcript.duration_ms,
            transcript.word_count,
            transcript.character_count,
            transcript.model_id.as_deref(),
            transcript.language.as_deref(),
            transcript.output_mode.as_ref().map(enum_to_json_string),
            transcript.paste_method.as_ref().map(enum_to_json_string),
            transcript.transcription_latency_ms,
            transcript.is_note,
            transcript.analysis.as_deref(),
            transcript.analysis_model.as_deref(),
            transcript
                .analysis_created_at
                .as_ref()
                .map(|date| date.to_rfc3339()),
        ],
    )
}

fn model_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelRecord> {
    let status: String = row.get(5)?;
    let downloaded_at: Option<String> = row.get(8)?;

    Ok(ModelRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        filename: row.get(2)?,
        local_path: row.get(3)?,
        size_bytes: row
            .get::<_, Option<i64>>(4)?
            .map(|value| value.max(0) as u64),
        status: ModelStatus::from_db_value(&status),
        checksum: row.get(6)?,
        selected: row.get::<_, i64>(7)? != 0,
        downloaded_at: downloaded_at.and_then(|date| {
            DateTime::parse_from_rfc3339(&date)
                .ok()
                .map(|date| date.with_timezone(&Utc))
        }),
    })
}

fn enum_to_json_string<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

fn optional_enum_from_json<T>(value: Option<String>) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    value.and_then(|value| serde_json::from_value(serde_json::Value::String(value)).ok())
}

fn escape_like_query(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn transcript_not_found(id: &str) -> CommandError {
    CommandError::new(
        "transcript_not_found",
        format!("Transcript {} was not found in local history.", id),
    )
}

fn average_for(total: Option<i64>, count: u32) -> Option<f64> {
    if count == 0 {
        None
    } else {
        Some(total.unwrap_or_default() as f64 / count as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_history::{GitHubSyncActivity, SyncOutcome, SyncSource};
    use crate::transcript::Transcript;
    use chrono::Duration;

    fn transcript_with_text(text: &str) -> Transcript {
        Transcript::new_last_buffer(
            text,
            Some(1200),
            Some("small.en-q5_1".to_string()),
            Some("en".to_string()),
        )
        .unwrap()
    }

    #[test]
    fn github_sync_activity_round_trips_and_corruption_is_non_fatal() {
        let db = Database::in_memory().unwrap();
        let activity = GitHubSyncActivity {
            completed_at: Utc::now(),
            outcome: SyncOutcome::Error,
            source: SyncSource::Automatic,
            repo: "alice/notes".into(),
            synced_items: 2,
            files_written: 1,
            error_code: Some("github_sync_failed".into()),
            error_message: Some("Network unavailable".into()),
        };

        db.save_github_sync_activity(&activity).unwrap();
        assert_eq!(db.get_github_sync_activity().unwrap(), Some(activity));

        db.clear_github_sync_activity().unwrap();
        assert_eq!(db.get_github_sync_activity().unwrap(), None);

        db.upsert_setting(GITHUB_SYNC_ACTIVITY_KEY, "not json")
            .unwrap();
        assert_eq!(db.get_github_sync_activity().unwrap(), None);
    }

    #[test]
    fn restore_skips_conflicts_and_inserts_new_rows_atomically() {
        let mut db = Database::in_memory().unwrap();
        let existing = transcript_with_text("local copy");
        db.save_last_transcript(&existing).unwrap();
        let mut conflicting = existing.clone();
        conflicting.text = "backup copy".into();
        let new = transcript_with_text("new from backup");

        assert_eq!(
            db.count_existing_transcripts(&[conflicting.clone(), new.clone()])
                .unwrap(),
            1
        );
        let report = db
            .restore_transcripts(&[conflicting, new.clone()], false)
            .unwrap();

        assert_eq!(
            report,
            ImportReport {
                imported: 1,
                skipped: 1,
                replaced: 0,
            }
        );
        assert_eq!(
            db.get_transcript_by_id(&existing.id).unwrap(),
            Some(existing)
        );
        assert_eq!(db.get_transcript_by_id(&new.id).unwrap(), Some(new));
    }

    #[test]
    fn explicit_restore_replacement_preserves_local_audio() {
        let mut db = Database::in_memory().unwrap();
        let mut existing = transcript_with_text("local copy");
        existing.audio_path = Some("C:\\local\\clip.wav".into());
        db.save_last_transcript(&existing).unwrap();
        let mut replacement = existing.clone();
        replacement.text = "backup copy".into();
        replacement.audio_path = None;

        let report = db.restore_transcripts(&[replacement], true).unwrap();
        let restored = db.get_transcript_by_id(&existing.id).unwrap().unwrap();

        assert_eq!(report.imported, 0);
        assert_eq!(report.replaced, 1);
        assert_eq!(restored.text, "backup copy");
        assert_eq!(restored.audio_path, existing.audio_path);
    }

    #[test]
    fn notes_only_search_filters_regular_transcripts() {
        let db = Database::in_memory().unwrap();
        db.save_last_transcript(&transcript_with_text("regular dictation"))
            .unwrap();
        let mut note = transcript_with_text("note to self");
        note.is_note = true;
        db.save_last_transcript(&note).unwrap();

        let notes = db
            .search_transcripts(None, true, None, None, TranscriptSort::default(), 10, 0)
            .unwrap();
        assert_eq!(notes.total, 1);
        assert_eq!(notes.transcripts[0].text, "note to self");
        assert!(notes.transcripts[0].is_note);

        let all = db
            .search_transcripts(None, false, None, None, TranscriptSort::default(), 10, 0)
            .unwrap();
        assert_eq!(all.total, 2);

        let queried = db
            .search_transcripts(
                Some("note"),
                true,
                None,
                None,
                TranscriptSort::default(),
                10,
                0,
            )
            .unwrap();
        assert_eq!(queried.total, 1);
    }

    #[test]
    fn dictation_only_search_excludes_notes() {
        let db = Database::in_memory().unwrap();
        db.save_last_transcript(&transcript_with_text("regular dictation"))
            .unwrap();
        let mut note = transcript_with_text("note to self");
        note.is_note = true;
        db.save_last_transcript(&note).unwrap();

        let dictations = db.search_dictation_transcripts(10, 0).unwrap();
        assert_eq!(dictations.total, 1);
        assert_eq!(dictations.transcripts[0].text, "regular dictation");
        assert!(!dictations.transcripts[0].is_note);
    }

    #[test]
    fn clearing_and_retention_preserve_notes() {
        let db = Database::in_memory().unwrap();
        db.save_last_transcript(&transcript_with_text("regular dictation"))
            .unwrap();
        let mut note = transcript_with_text("note to self");
        note.is_note = true;
        db.save_last_transcript(&note).unwrap();

        // Clearing transcript history removes the dictation row but keeps the note.
        db.clear_transcript_history().unwrap();
        let remaining = db
            .search_transcripts(None, false, None, None, TranscriptSort::default(), 10, 0)
            .unwrap();
        assert_eq!(
            remaining.total, 1,
            "note must survive clear_transcript_history"
        );
        assert!(remaining.transcripts[0].is_note);

        // clear_notes then removes the note.
        db.clear_notes().unwrap();
        let after = db
            .search_transcripts(None, false, None, None, TranscriptSort::default(), 10, 0)
            .unwrap();
        assert_eq!(after.total, 0);
    }

    #[test]
    fn history_retention_clears_an_expired_last_transcript_buffer() {
        let db = Database::in_memory().unwrap();
        let mut old = transcript_with_text("months-old dictation");
        old.created_at = Utc::now() - Duration::days(45);
        db.save_last_transcript_with_history(&old, false).unwrap();
        assert!(db.get_last_transcript().unwrap().is_some());

        db.enforce_history_retention(Some(30)).unwrap();
        assert!(
            db.get_last_transcript().unwrap().is_none(),
            "an expired dictation must not stay pasteable from the buffer"
        );

        let fresh = transcript_with_text("fresh dictation");
        db.save_last_transcript_with_history(&fresh, false).unwrap();
        db.enforce_history_retention(Some(30)).unwrap();
        assert_eq!(db.get_last_transcript().unwrap().unwrap().id, fresh.id);
    }

    #[test]
    fn notes_retention_prunes_old_notes_and_spares_transcripts() {
        let db = Database::in_memory().unwrap();

        // An old note, an old dictation transcript, and a fresh note. Save all
        // three first, then back-date the two "old" rows directly — back-dating
        // before the last save would let that save's transcript-retention sweep
        // (default 30 days) purge the aged transcript prematurely.
        let mut old_note = transcript_with_text("old note");
        old_note.is_note = true;
        db.save_last_transcript(&old_note).unwrap();
        let old_tx = transcript_with_text("old transcript");
        db.save_last_transcript(&old_tx).unwrap();
        let mut fresh_note = transcript_with_text("fresh note");
        fresh_note.is_note = true;
        db.save_last_transcript(&fresh_note).unwrap();
        for id in [&old_note.id, &old_tx.id] {
            db.conn
                .execute(
                    "UPDATE transcripts SET created_at = ?2 WHERE id = ?1",
                    params![id, (Utc::now() - Duration::days(45)).to_rfc3339()],
                )
                .unwrap();
        }

        // Notes retention removes only the aged note; the aged transcript stays.
        db.enforce_notes_retention(Some(30)).unwrap();
        let notes = db
            .search_transcripts(None, true, None, None, TranscriptSort::default(), 10, 0)
            .unwrap();
        assert_eq!(notes.total, 1, "only the fresh note should remain");
        assert_eq!(notes.transcripts[0].id, fresh_note.id);
        assert!(
            db.get_transcript_by_id(&old_tx.id).unwrap().is_some(),
            "notes retention must not touch dictation transcripts"
        );

        // Transcript retention is the mirror: it removes the aged transcript and
        // leaves notes (even ones older than its window) intact.
        db.enforce_history_retention(Some(30)).unwrap();
        assert!(db.get_transcript_by_id(&old_tx.id).unwrap().is_none());
        assert!(db.get_transcript_by_id(&fresh_note.id).unwrap().is_some());
    }

    #[test]
    fn note_analysis_round_trips_and_replaces() {
        let db = Database::in_memory().unwrap();
        let mut note = transcript_with_text("remember to file taxes");
        note.is_note = true;
        db.save_last_transcript(&note).unwrap();

        let saved = db
            .save_note_analysis(&note.id, "Summary.\n- File taxes", "qwen2.5-7b")
            .unwrap();
        assert_eq!(saved.analysis.as_deref(), Some("Summary.\n- File taxes"));
        assert_eq!(saved.analysis_model.as_deref(), Some("qwen2.5-7b"));
        assert!(saved.analysis_created_at.is_some());

        // Re-running replaces the stored analysis.
        let replaced = db
            .save_note_analysis(&note.id, "New summary.", "llama-3.1-8b")
            .unwrap();
        assert_eq!(replaced.analysis.as_deref(), Some("New summary."));
        assert_eq!(replaced.analysis_model.as_deref(), Some("llama-3.1-8b"));

        // The analysis comes back through the search path too.
        let result = db
            .search_transcripts(None, true, None, None, TranscriptSort::default(), 10, 0)
            .unwrap();
        assert_eq!(
            result.transcripts[0].analysis.as_deref(),
            Some("New summary.")
        );

        assert!(db.save_note_analysis("tx_missing", "x", "y").is_err());
    }

    /// A transcript plus a real on-disk clip file (the transcript id keeps
    /// the path unique across parallel tests).
    fn transcript_with_clip(text: &str) -> (Transcript, std::path::PathBuf) {
        let mut transcript = transcript_with_text(text);
        let clip_path = std::env::temp_dir().join(format!("{}.wav", transcript.id));
        std::fs::write(&clip_path, b"RIFF").unwrap();
        transcript.audio_path = Some(clip_path.to_string_lossy().into_owned());
        (transcript, clip_path)
    }

    #[test]
    fn migration_adds_audio_path_to_existing_databases() {
        let path = std::env::temp_dir().join(format!(
            "scribe_migration_{}.sqlite3",
            uuid::Uuid::new_v4().simple()
        ));
        {
            // A database created before saved clips: 001 only, no audio_path.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(INITIAL_MIGRATION).unwrap();
        }

        // The first open adds the column; the second tolerates it existing.
        for _ in 0..2 {
            let db = Database::open(&path).unwrap();
            assert!(db.list_recent_transcripts(1).unwrap().is_empty());
        }

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn settings_round_trip_through_database() {
        let db = Database::in_memory().unwrap();
        let settings = AppSettings {
            notifications_enabled: false,
            ..AppSettings::default()
        };

        db.save_settings(&settings).unwrap();

        assert_eq!(db.get_settings().unwrap(), settings);
    }

    fn user_version(conn: &Connection) -> i32 {
        conn.pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    }

    fn transcript_columns(conn: &Connection) -> Vec<String> {
        let mut statement = conn.prepare("PRAGMA table_info(transcripts)").unwrap();
        let names = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        names
    }

    /// Builds a database file the way an older Scribe left it: migrations up
    /// to `applied` (1 to 4) run with no user_version, plus optionally only the
    /// first statement of 004, as an interrupted batch would leave it.
    fn historic_db(path: &Path, applied: usize, partial_004: bool) {
        let conn = Connection::open(path).unwrap();
        let migrations = [
            INITIAL_MIGRATION,
            AUDIO_CLIPS_MIGRATION,
            NOTES_MIGRATION,
            NOTE_ANALYSIS_MIGRATION,
        ];
        for migration in &migrations[..applied] {
            conn.execute_batch(migration).unwrap();
        }
        if partial_004 {
            conn.execute_batch("ALTER TABLE transcripts ADD COLUMN analysis TEXT;")
                .unwrap();
        }
        conn.execute(
            "INSERT INTO transcripts (id, text, created_at, word_count, character_count) \
             VALUES ('old', 'kept text', '2026-06-12T00:00:00+00:00', 2, 9)",
            [],
        )
        .unwrap();
        assert_eq!(user_version(&conn), 0);
    }

    #[test]
    fn opens_databases_at_every_historic_schema_shape() {
        let cases = [(1, false), (2, false), (3, false), (3, true), (4, false)];
        for (applied, partial) in cases {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("scribe.sqlite3");
            historic_db(&path, applied, partial);

            let db = Database::open(&path).unwrap();

            assert_eq!(
                user_version(&db.conn),
                SCHEMA_VERSION,
                "case {applied}/{partial}"
            );
            for column in [
                "audio_path",
                "is_note",
                "analysis",
                "analysis_model",
                "analysis_created_at",
            ] {
                assert!(
                    transcript_columns(&db.conn)
                        .iter()
                        .any(|name| name == column),
                    "case {applied}/{partial} is missing {column}"
                );
            }
            let kept = db.get_transcript_by_id("old").unwrap().unwrap();
            assert_eq!(kept.text, "kept text");
            drop(db);
            // Reopening a current database is a no-op.
            let db = Database::open(&path).unwrap();
            assert_eq!(user_version(&db.conn), SCHEMA_VERSION);
        }
    }

    #[test]
    fn file_databases_use_wal_and_secure_delete() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("scribe.sqlite3")).unwrap();
        let mode: String = db
            .conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        let secure: i64 = db
            .conn
            .pragma_query_value(None, "secure_delete", |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        assert_eq!(secure, 1);
    }

    #[test]
    fn a_newer_schema_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scribe.sqlite3");
        drop(Database::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();

        let db = Database::open(&path).unwrap();

        assert_eq!(user_version(&db.conn), SCHEMA_VERSION + 1);
    }

    #[test]
    fn add_column_statements_cover_every_migration_column() {
        let all: Vec<_> = [
            AUDIO_CLIPS_MIGRATION,
            NOTES_MIGRATION,
            NOTE_ANALYSIS_MIGRATION,
        ]
        .iter()
        .flat_map(|migration| add_column_statements(migration))
        .map(|(table, column, _)| format!("{table}.{column}"))
        .collect();
        assert_eq!(
            all,
            [
                "transcripts.audio_path",
                "transcripts.is_note",
                "transcripts.analysis",
                "transcripts.analysis_model",
                "transcripts.analysis_created_at"
            ]
        );
    }

    #[test]
    fn retention_expires_note_audio_but_keeps_the_note() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("note.wav");
        std::fs::write(&clip, b"audio").unwrap();
        let db = Database::in_memory().unwrap();
        let mut note = Transcript::new_last_buffer("an old note", None, None, None).unwrap();
        note.is_note = true;
        note.audio_path = Some(clip.to_string_lossy().into_owned());
        note.created_at = Utc::now() - chrono::Duration::days(45);
        db.save_note_transcript(&note).unwrap();

        db.enforce_history_retention(Some(30)).unwrap();

        let kept = db.get_transcript_by_id(&note.id).unwrap().unwrap();
        assert_eq!(kept.text, "an old note");
        assert_eq!(kept.audio_path, None);
        assert!(!clip.exists());
    }

    #[test]
    fn replace_stored_path_updates_models_transcripts_and_buffer() {
        let db = Database::in_memory().unwrap();
        let mut transcript = Transcript::new_last_buffer("hello", None, None, None).unwrap();
        transcript.audio_path = Some(r"C:\old\clips\a.wav".to_string());
        db.save_last_transcript(&transcript).unwrap();

        let changed = db
            .replace_stored_path(r"C:\old\clips\a.wav", r"C:\new\clips\a.wav")
            .unwrap();

        assert_eq!(changed, 2, "the history row and the buffer");
        assert_eq!(
            db.referenced_clip_paths().unwrap(),
            vec![r"C:\new\clips\a.wav".to_string(); 2]
        );
    }

    #[test]
    fn one_bad_setting_keeps_the_rest() {
        let db = Database::in_memory().unwrap();
        let custom = AppSettings {
            history_retention_days: Some(90),
            theme: "ocean".to_string(),
            ..AppSettings::default()
        };
        let mut stored = serde_json::to_value(&custom).unwrap();
        stored["outputMode"] = serde_json::json!("teleport");
        stored["hotkeys"]["toggleDictation"] = serde_json::json!(42);
        db.upsert_setting(SETTINGS_KEY, &stored.to_string())
            .unwrap();

        let recovered = db.get_settings().unwrap();

        assert_eq!(recovered.history_retention_days, Some(90));
        assert_eq!(recovered.theme, "ocean");
        assert_eq!(recovered.output_mode, AppSettings::default().output_mode);
        assert_eq!(
            recovered.hotkeys.toggle_dictation,
            AppSettings::default().hotkeys.toggle_dictation
        );
        assert_eq!(
            recovered.hotkeys.hold_to_talk, custom.hotkeys.hold_to_talk,
            "the other hotkeys survive"
        );
        // The repaired value is persisted, so the next load reads cleanly.
        assert_eq!(db.get_settings().unwrap(), recovered);
    }

    #[test]
    fn lenient_settings_keeps_missing_fields_at_their_serde_defaults() {
        let (settings, dropped) =
            lenient_settings(r#"{"historyRetentionDays": 7, "outputMode": 5}"#);
        assert_eq!(settings.history_retention_days, Some(7));
        assert_eq!(
            settings.defaults_version, 0,
            "one-time migrations still run"
        );
        assert_eq!(dropped, vec!["outputMode".to_string()]);
    }

    #[test]
    fn corrupt_settings_self_heal_to_defaults_and_are_backed_up() {
        let db = Database::in_memory().unwrap();

        // Simulate a truncated / hand-edited / partially-written settings row
        // that no longer deserializes into AppSettings.
        db.upsert_setting(SETTINGS_KEY, "{ this is not valid settings json")
            .unwrap();

        // get_settings must NOT propagate an error (which at startup would
        // abort setup() and panic) — it self-heals to defaults instead.
        let recovered = db.get_settings().unwrap();
        assert_eq!(recovered, AppSettings::default());

        // The corrupt value is preserved under a timestamped backup key.
        let backup_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key = 'app_settings_corrupt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(backup_count, 1);

        // The main row now holds valid (default) settings, so a second load is
        // stable and never re-triggers recovery.
        assert_eq!(db.get_settings().unwrap(), AppSettings::default());
        let backup_count_after: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key = 'app_settings_corrupt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(backup_count_after, 1);
    }

    #[test]
    fn last_transcript_round_trip_keeps_metadata() {
        let db = Database::in_memory().unwrap();
        let transcript = Transcript::new_last_buffer(
            "A local transcript",
            Some(1200),
            Some("small.en-q5_1".to_string()),
            Some("en".to_string()),
        )
        .unwrap();

        db.save_last_transcript(&transcript).unwrap();
        let saved = db.get_last_transcript().unwrap().unwrap();

        assert_eq!(saved.id, transcript.id);
        assert_eq!(saved.word_count, 3);
        assert_eq!(saved.character_count, 18);

        db.clear_last_transcript().unwrap();
        assert!(db.get_last_transcript().unwrap().is_none());
        assert_eq!(db.list_recent_transcripts(10).unwrap().len(), 1);
    }

    #[test]
    fn last_transcript_buffer_can_save_without_history() {
        let db = Database::in_memory().unwrap();
        let transcript =
            Transcript::new_last_buffer("Buffer only", Some(900), None, Some("en".to_string()))
                .unwrap();

        db.save_last_transcript_with_history(&transcript, false)
            .unwrap();

        assert_eq!(
            db.get_last_transcript().unwrap().unwrap().text,
            "Buffer only"
        );
        assert!(db.list_recent_transcripts(10).unwrap().is_empty());
    }

    #[test]
    fn search_transcripts_filters_and_paginates_history_rows() {
        let db = Database::in_memory().unwrap();
        let first = transcript_with_text("Alpha meeting notes");
        let second = transcript_with_text("Beta alpha follow up");
        let third = transcript_with_text("Unrelated transcript");

        db.save_last_transcript(&first).unwrap();
        db.save_last_transcript(&second).unwrap();
        db.save_last_transcript(&third).unwrap();

        let result = db
            .search_transcripts(
                Some("alpha"),
                false,
                None,
                None,
                TranscriptSort::default(),
                1,
                0,
            )
            .expect("search should work");

        assert_eq!(result.total, 2);
        assert_eq!(result.limit, 1);
        assert_eq!(result.offset, 0);
        assert_eq!(result.transcripts.len(), 1);

        let second_page = db
            .search_transcripts(
                Some("alpha"),
                false,
                None,
                None,
                TranscriptSort::default(),
                1,
                1,
            )
            .unwrap();
        assert_eq!(second_page.total, 2);
        assert_eq!(second_page.transcripts.len(), 1);
    }

    #[test]
    fn update_delete_and_clear_history_are_reflected_in_stats() {
        let db = Database::in_memory().unwrap();
        let first = transcript_with_text("one two three");
        let second = transcript_with_text("four five");

        db.save_last_transcript(&first).unwrap();
        db.save_last_transcript(&second).unwrap();
        assert_eq!(db.get_basic_stats().unwrap().total_words_transcribed, 5);

        let updated = db.update_transcript(&first.id, "one").unwrap();
        assert_eq!(updated.word_count, 1);
        assert_eq!(db.get_basic_stats().unwrap().total_words_transcribed, 3);

        db.delete_transcript(&second.id).unwrap();
        assert_eq!(db.get_basic_stats().unwrap().total_words_transcribed, 1);

        db.clear_transcript_history().unwrap();
        assert_eq!(db.get_basic_stats().unwrap(), BasicStats::default());
    }

    #[test]
    fn retention_removes_old_history_without_clearing_last_buffer() {
        let db = Database::in_memory().unwrap();
        let old = transcript_with_text("old transcript");
        let recent = transcript_with_text("recent transcript");

        db.save_last_transcript(&old).unwrap();
        db.conn
            .execute(
                "UPDATE transcripts SET created_at = ?2 WHERE id = ?1",
                params![old.id, (Utc::now() - Duration::days(45)).to_rfc3339()],
            )
            .unwrap();

        db.save_last_transcript(&recent).unwrap();
        db.enforce_history_retention(Some(30)).unwrap();

        let result = db
            .search_transcripts(None, false, None, None, TranscriptSort::default(), 10, 0)
            .unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.transcripts[0].id, recent.id);
        assert_eq!(db.get_last_transcript().unwrap().unwrap().id, recent.id);
    }

    #[test]
    fn audio_path_round_trips_through_history_and_buffer() {
        let db = Database::in_memory().unwrap();
        let mut transcript = transcript_with_text("clip backed transcript");
        transcript.audio_path = Some("C:\\clips\\tx_1.wav".to_string());

        db.save_last_transcript(&transcript).unwrap();

        assert_eq!(
            db.get_transcript_by_id(&transcript.id)
                .unwrap()
                .unwrap()
                .audio_path
                .as_deref(),
            Some("C:\\clips\\tx_1.wav")
        );
        assert_eq!(
            db.get_last_transcript().unwrap().unwrap().audio_path,
            transcript.audio_path
        );
    }

    #[test]
    fn delete_transcript_removes_clip_file() {
        let db = Database::in_memory().unwrap();
        let (transcript, clip_path) = transcript_with_clip("clip to delete");

        db.save_last_transcript(&transcript).unwrap();
        assert!(clip_path.exists());

        db.delete_transcript(&transcript.id).unwrap();
        assert!(!clip_path.exists());

        // Deleting again (the clip is already gone) must not error.
        db.delete_transcript(&transcript.id).unwrap();
    }

    #[test]
    fn clear_history_removes_clip_files() {
        let db = Database::in_memory().unwrap();
        let (first, first_clip) = transcript_with_clip("first clip");
        let (second, second_clip) = transcript_with_clip("second clip");

        db.save_last_transcript(&first).unwrap();
        db.save_last_transcript(&second).unwrap();

        db.clear_transcript_history().unwrap();

        assert!(!first_clip.exists());
        assert!(!second_clip.exists());
    }

    #[test]
    fn retention_removes_clip_files_of_purged_rows() {
        let db = Database::in_memory().unwrap();
        let (old, old_clip) = transcript_with_clip("old clip");
        let (recent, recent_clip) = transcript_with_clip("recent clip");

        db.save_last_transcript(&old).unwrap();
        db.conn
            .execute(
                "UPDATE transcripts SET created_at = ?2 WHERE id = ?1",
                params![old.id, (Utc::now() - Duration::days(45)).to_rfc3339()],
            )
            .unwrap();
        db.save_last_transcript(&recent).unwrap();

        db.enforce_history_retention(Some(30)).unwrap();

        assert!(!old_clip.exists());
        assert!(recent_clip.exists());
        std::fs::remove_file(recent_clip).unwrap();
    }

    #[test]
    fn buffer_only_clip_is_removed_when_buffer_is_replaced_or_cleared() {
        let db = Database::in_memory().unwrap();
        let (first, first_clip) = transcript_with_clip("buffer only clip");
        let (second, second_clip) = transcript_with_clip("replacement clip");

        // History disabled: the clip's only owner is the buffer, so
        // replacing the buffer must remove it.
        db.save_last_transcript_with_history(&first, false).unwrap();
        db.save_last_transcript_with_history(&second, false)
            .unwrap();
        assert!(!first_clip.exists());
        assert!(second_clip.exists());

        db.clear_last_transcript().unwrap();
        assert!(!second_clip.exists());
    }

    #[test]
    fn history_owned_clip_survives_buffer_replacement() {
        let db = Database::in_memory().unwrap();
        let (first, first_clip) = transcript_with_clip("history owned clip");
        let second = transcript_with_text("next dictation");

        db.save_last_transcript(&first).unwrap();
        db.save_last_transcript(&second).unwrap();

        // The clip belongs to the history row; only its deletion removes it.
        assert!(first_clip.exists());
        db.delete_transcript(&first.id).unwrap();
        assert!(!first_clip.exists());
    }

    #[test]
    fn retention_null_keeps_history_rows() {
        let db = Database::in_memory().unwrap();
        let old = transcript_with_text("old transcript");

        db.save_last_transcript(&old).unwrap();
        db.conn
            .execute(
                "UPDATE transcripts SET created_at = ?2 WHERE id = ?1",
                params![old.id, (Utc::now() - Duration::days(45)).to_rfc3339()],
            )
            .unwrap();

        db.enforce_history_retention(None).unwrap();

        assert_eq!(
            db.search_transcripts(None, false, None, None, TranscriptSort::default(), 10, 0)
                .unwrap()
                .total,
            1
        );
        assert_eq!(db.get_last_transcript().unwrap().unwrap().id, old.id);
    }

    /// Saves a transcript and forces its stored `created_at`, so date-range and
    /// sort tests can pin rows to known instants regardless of save time.
    fn save_with_created_at(db: &Database, text: &str, created_at: DateTime<Utc>) -> Transcript {
        let mut transcript = transcript_with_text(text);
        transcript.created_at = created_at;
        db.save_last_transcript(&transcript).unwrap();
        db.conn
            .execute(
                "UPDATE transcripts SET created_at = ?2 WHERE id = ?1",
                params![transcript.id, created_at.to_rfc3339()],
            )
            .unwrap();
        transcript
    }

    #[test]
    fn search_filters_by_created_at_date_range() {
        let db = Database::in_memory().unwrap();
        // Anchor inside the default 30-day retention window (and not in the
        // future) so the save path's retention sweep keeps every row.
        let base = Utc::now() - Duration::days(20);
        let early = save_with_created_at(&db, "early one", base);
        let middle = save_with_created_at(&db, "middle one", base + Duration::days(5));
        let late = save_with_created_at(&db, "late one", base + Duration::days(10));

        // Inclusive lower bound only.
        let from_only = db
            .search_transcripts(
                None,
                false,
                Some(&(base + Duration::days(5)).to_rfc3339()),
                None,
                TranscriptSort::OldestFirst,
                10,
                0,
            )
            .unwrap();
        assert_eq!(from_only.total, 2);
        assert_eq!(from_only.transcripts[0].id, middle.id);
        assert_eq!(from_only.transcripts[1].id, late.id);

        // Exclusive upper bound only: `to` is half-open, so a row exactly at the
        // `to` instant (here `middle`, at base + 5 days) is EXCLUDED. Only the
        // strictly-earlier `early` row remains.
        let to_only = db
            .search_transcripts(
                None,
                false,
                None,
                Some(&(base + Duration::days(5)).to_rfc3339()),
                TranscriptSort::OldestFirst,
                10,
                0,
            )
            .unwrap();
        assert_eq!(to_only.total, 1);
        assert_eq!(to_only.transcripts[0].id, early.id);

        // To include `middle`, the exclusive `to` must sit strictly after it.
        let to_just_past_middle = db
            .search_transcripts(
                None,
                false,
                None,
                Some(&(base + Duration::days(5) + Duration::milliseconds(1)).to_rfc3339()),
                TranscriptSort::OldestFirst,
                10,
                0,
            )
            .unwrap();
        assert_eq!(to_just_past_middle.total, 2);
        assert_eq!(to_just_past_middle.transcripts[0].id, early.id);
        assert_eq!(to_just_past_middle.transcripts[1].id, middle.id);

        // Both bounds: a half-open window [from, to) that captures only the
        // middle row (from inclusive at <= middle, to exclusive past middle).
        let windowed = db
            .search_transcripts(
                None,
                false,
                Some(&(base + Duration::days(1)).to_rfc3339()),
                Some(&(base + Duration::days(9)).to_rfc3339()),
                TranscriptSort::default(),
                10,
                0,
            )
            .unwrap();
        assert_eq!(windowed.total, 1);
        assert_eq!(windowed.transcripts[0].id, middle.id);
    }

    #[test]
    fn search_honors_each_sort_order() {
        let db = Database::in_memory().unwrap();
        // Inside the default 30-day retention window so saves don't purge rows.
        let base = Utc::now() - Duration::days(10);
        // oldest + shortest
        let first = save_with_created_at(&db, "short", base);
        // newest, medium length
        let second = save_with_created_at(&db, "medium length text", base + Duration::days(2));
        // middle date, longest text
        let third = save_with_created_at(
            &db,
            "the longest text of the three by far",
            base + Duration::days(1),
        );

        let newest = db
            .search_transcripts(None, false, None, None, TranscriptSort::NewestFirst, 10, 0)
            .unwrap();
        assert_eq!(
            newest
                .transcripts
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            vec![second.id.as_str(), third.id.as_str(), first.id.as_str()]
        );

        let oldest = db
            .search_transcripts(None, false, None, None, TranscriptSort::OldestFirst, 10, 0)
            .unwrap();
        assert_eq!(
            oldest
                .transcripts
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            vec![first.id.as_str(), third.id.as_str(), second.id.as_str()]
        );

        let longest = db
            .search_transcripts(None, false, None, None, TranscriptSort::Longest, 10, 0)
            .unwrap();
        assert_eq!(longest.transcripts[0].id, third.id);
        // The shortest text sorts last under Longest.
        assert_eq!(longest.transcripts[2].id, first.id);
    }

    #[test]
    fn combine_transcripts_orders_oldest_first_and_joins() {
        let db = Database::in_memory().unwrap();
        // Inside the default 30-day retention window so saves don't purge rows.
        let base = Utc::now() - Duration::days(10);
        let newest = save_with_created_at(&db, "third", base + Duration::days(2));
        let oldest = save_with_created_at(&db, "first", base);
        let middle = save_with_created_at(&db, "second", base + Duration::days(1));

        // Ids passed out of chronological order; result is oldest-first.
        let combined = db
            .combine_transcripts(
                &[newest.id.clone(), oldest.id.clone(), middle.id.clone()],
                "\n\n",
            )
            .unwrap();
        assert_eq!(combined, "first\n\nsecond\n\nthird");

        // A custom separator is honored.
        let with_sep = db
            .combine_transcripts(&[oldest.id.clone(), middle.id.clone()], " | ")
            .unwrap();
        assert_eq!(with_sep, "first | second");

        // Unresolved ids are skipped; resolved ones still combine.
        let with_missing = db
            .combine_transcripts(&["tx_missing".to_string(), oldest.id.clone()], "\n\n")
            .unwrap();
        assert_eq!(with_missing, "first");

        // No id resolving is an error.
        assert!(db
            .combine_transcripts(&["tx_a".to_string(), "tx_b".to_string()], "\n\n")
            .is_err());
    }
}
