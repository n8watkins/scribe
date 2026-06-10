use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    error::CommandError, settings::AppSettings, stats::BasicStats, transcript::Transcript,
};

const INITIAL_MIGRATION: &str = include_str!("../migrations/001_initial.sql");
const SETTINGS_KEY: &str = "app_settings";
const LAST_TRANSCRIPT_ID_KEY: &str = "last_transcript_id";

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CommandError> {
        let conn = Connection::open(path).map_err(CommandError::database)?;
        conn.execute_batch(INITIAL_MIGRATION)
            .map_err(CommandError::database)?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self, CommandError> {
        let conn = Connection::open_in_memory().map_err(CommandError::database)?;
        conn.execute_batch(INITIAL_MIGRATION)
            .map_err(CommandError::database)?;
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
            Some(value) => serde_json::from_str(&value).map_err(CommandError::from),
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

    pub fn list_recent_transcripts(&self, limit: u32) -> Result<Vec<Transcript>, CommandError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, text, created_at, duration_ms, word_count, character_count,
                        model_id, language, output_mode, paste_method, transcription_latency_ms
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

    pub fn get_last_transcript(&self) -> Result<Option<Transcript>, CommandError> {
        let Some(id) = self.get_setting_value(LAST_TRANSCRIPT_ID_KEY)? else {
            return Ok(None);
        };

        if id.trim().is_empty() {
            return Ok(None);
        }

        self.get_transcript_by_id(&id)
    }

    pub fn clear_last_transcript(&self) -> Result<(), CommandError> {
        self.upsert_setting(LAST_TRANSCRIPT_ID_KEY, "")?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn save_last_transcript(&self, transcript: &Transcript) -> Result<(), CommandError> {
        if transcript.text.trim().is_empty() {
            return Ok(());
        }

        self.insert_transcript(transcript)?;
        self.upsert_setting(LAST_TRANSCRIPT_ID_KEY, &transcript.id)?;
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
                    model_id, language, output_mode, paste_method, transcription_latency_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                ],
            )
            .map_err(CommandError::database)?;

        Ok(())
    }

    fn get_transcript_by_id(&self, id: &str) -> Result<Option<Transcript>, CommandError> {
        self.conn
            .query_row(
                "SELECT id, text, created_at, duration_ms, word_count, character_count,
                        model_id, language, output_mode, paste_method, transcription_latency_ms
                 FROM transcripts
                 WHERE id = ?1",
                [id],
                transcript_from_row,
            )
            .optional()
            .map_err(CommandError::database)
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
    use crate::transcript::Transcript;

    #[test]
    fn settings_round_trip_through_database() {
        let db = Database::in_memory().unwrap();
        let mut settings = AppSettings::default();
        settings.notifications_enabled = false;

        db.save_settings(&settings).unwrap();

        assert_eq!(db.get_settings().unwrap(), settings);
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
}
