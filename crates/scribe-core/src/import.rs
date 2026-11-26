//! Validation and normalization for restoring Scribe's lossless JSON exports.

use std::collections::HashSet;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    error::CommandError,
    transcript::{metadata_for_text, Transcript},
};

pub const MAX_IMPORT_BYTES: u64 = 100 * 1024 * 1024;
const MAX_IMPORT_RECORDS: usize = 100_000;
const MAX_TEXT_BYTES: usize = 5 * 1024 * 1024;
const MAX_MODEL_ID_BYTES: usize = 256;
const MAX_LANGUAGE_BYTES: usize = 64;
const MAX_ANALYSIS_MODEL_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedImport {
    #[serde(skip)]
    pub transcripts: Vec<Transcript>,
    pub total: u32,
    pub notes: u32,
    pub dictations: u32,
    pub audio_paths_removed: u32,
    pub metadata_corrected: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: u32,
    pub skipped: u32,
    pub replaced: u32,
}

pub fn prepare_json(contents: &[u8]) -> Result<PreparedImport, CommandError> {
    if contents.len() as u64 > MAX_IMPORT_BYTES {
        return Err(CommandError::new(
            "import_too_large",
            "The backup is larger than the 100 MB restore limit.",
        ));
    }

    let mut transcripts: Vec<Transcript> = serde_json::from_slice(contents).map_err(|error| {
        CommandError::new(
            "invalid_import_file",
            format!("This is not a valid Scribe JSON export. {error}"),
        )
    })?;
    if transcripts.len() > MAX_IMPORT_RECORDS {
        return Err(CommandError::new(
            "import_too_many_records",
            format!(
                "The backup contains more than the {} record restore limit.",
                MAX_IMPORT_RECORDS
            ),
        ));
    }

    let mut seen = HashSet::with_capacity(transcripts.len());
    let mut notes = 0_u32;
    let mut audio_paths_removed = 0_u32;
    let mut metadata_corrected = 0_u32;

    for (index, transcript) in transcripts.iter_mut().enumerate() {
        validate_transcript(transcript, index)?;
        if !seen.insert(transcript.id.clone()) {
            return Err(CommandError::new(
                "duplicate_import_id",
                format!(
                    "The backup contains the transcript id {:?} more than once.",
                    transcript.id
                ),
            ));
        }

        if transcript.audio_path.take().is_some() {
            audio_paths_removed += 1;
        }
        let metadata = metadata_for_text(&transcript.text);
        if transcript.word_count != metadata.word_count
            || transcript.character_count != metadata.character_count
        {
            transcript.word_count = metadata.word_count;
            transcript.character_count = metadata.character_count;
            metadata_corrected += 1;
        }
        if transcript.analysis.is_none() {
            transcript.analysis_model = None;
            transcript.analysis_created_at = None;
        }
        if transcript.is_note {
            notes += 1;
        }
    }

    let total = transcripts.len() as u32;
    Ok(PreparedImport {
        transcripts,
        total,
        notes,
        dictations: total - notes,
        audio_paths_removed,
        metadata_corrected,
    })
}

pub fn sha256_fingerprint(contents: &[u8]) -> String {
    format!("{:x}", Sha256::digest(contents))
}

pub fn verify_fingerprint(actual: &str, expected: &str) -> Result<(), CommandError> {
    if actual == expected {
        return Ok(());
    }
    Err(CommandError::new(
        "import_file_changed",
        "The backup changed after it was previewed. Choose it again before restoring.",
    ))
}

fn validate_transcript(transcript: &Transcript, index: usize) -> Result<(), CommandError> {
    if !valid_transcript_id(&transcript.id) {
        return Err(invalid_record(index, "has an invalid transcript id"));
    }
    if transcript.text.trim().is_empty() {
        return Err(invalid_record(index, "has no transcript text"));
    }
    if transcript.text.len() > MAX_TEXT_BYTES {
        return Err(invalid_record(index, "has more than 5 MB of text"));
    }
    if transcript
        .analysis
        .as_ref()
        .is_some_and(|analysis| analysis.len() > MAX_TEXT_BYTES)
    {
        return Err(invalid_record(index, "has more than 5 MB of analysis"));
    }
    validate_optional_string(
        transcript.model_id.as_deref(),
        MAX_MODEL_ID_BYTES,
        index,
        "model id",
    )?;
    validate_optional_string(
        transcript.language.as_deref(),
        MAX_LANGUAGE_BYTES,
        index,
        "language",
    )?;
    validate_optional_string(
        transcript.analysis_model.as_deref(),
        MAX_ANALYSIS_MODEL_BYTES,
        index,
        "analysis model",
    )?;
    Ok(())
}

fn validate_optional_string(
    value: Option<&str>,
    max_bytes: usize,
    index: usize,
    field: &str,
) -> Result<(), CommandError> {
    if value.is_some_and(|value| value.len() > max_bytes) {
        return Err(invalid_record(
            index,
            &format!("has a {field} longer than {max_bytes} bytes"),
        ));
    }
    Ok(())
}

fn valid_transcript_id(id: &str) -> bool {
    let Some(suffix) = id.strip_prefix("tx_") else {
        return false;
    };
    !suffix.is_empty()
        && id.len() <= 128
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn invalid_record(index: usize, reason: &str) -> CommandError {
    CommandError::new(
        "invalid_import_record",
        format!("Backup record {} {reason}.", index + 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exported() -> Transcript {
        let mut transcript =
            Transcript::new_last_buffer("restored words", Some(800), None, Some("en".into()))
                .unwrap();
        transcript.audio_path = Some("C:\\private\\clip.wav".into());
        transcript.word_count = 999;
        transcript.character_count = 999;
        transcript
    }

    #[test]
    fn prepares_lossless_export_and_removes_untrusted_local_paths() {
        let prepared = prepare_json(&serde_json::to_vec(&vec![exported()]).unwrap()).unwrap();

        assert_eq!(prepared.total, 1);
        assert_eq!(prepared.dictations, 1);
        assert_eq!(prepared.notes, 0);
        assert_eq!(prepared.audio_paths_removed, 1);
        assert_eq!(prepared.metadata_corrected, 1);
        assert_eq!(prepared.transcripts[0].audio_path, None);
        assert_eq!(prepared.transcripts[0].word_count, 2);
        assert_eq!(prepared.transcripts[0].character_count, 14);
    }

    #[test]
    fn rejects_non_scribe_json_and_duplicate_ids() {
        let invalid = prepare_json(br#"{"not":"an export"}"#).unwrap_err();
        assert_eq!(invalid.code, "invalid_import_file");

        let transcript = exported();
        let duplicate = serde_json::to_vec(&vec![transcript.clone(), transcript]).unwrap();
        let error = prepare_json(&duplicate).unwrap_err();
        assert_eq!(error.code, "duplicate_import_id");
    }

    #[test]
    fn rejects_blank_text_and_invalid_ids() {
        let mut transcript = exported();
        transcript.text = "  ".into();
        let error = prepare_json(&serde_json::to_vec(&vec![transcript]).unwrap()).unwrap_err();
        assert_eq!(error.code, "invalid_import_record");

        let mut transcript = exported();
        transcript.id = "../../settings".into();
        let error = prepare_json(&serde_json::to_vec(&vec![transcript]).unwrap()).unwrap_err();
        assert_eq!(error.code, "invalid_import_record");
    }

    #[test]
    fn clears_analysis_provenance_when_analysis_is_absent() {
        let mut transcript = exported();
        transcript.analysis = None;
        transcript.analysis_model = Some("stale-model".into());
        transcript.analysis_created_at = Some(chrono::Utc::now());

        let prepared = prepare_json(&serde_json::to_vec(&vec![transcript]).unwrap()).unwrap();
        assert_eq!(prepared.transcripts[0].analysis_model, None);
        assert_eq!(prepared.transcripts[0].analysis_created_at, None);
    }

    #[test]
    fn bounds_every_free_form_metadata_field() {
        let updates: [fn(&mut Transcript); 3] = [
            |transcript: &mut Transcript| transcript.model_id = Some("m".repeat(257)),
            |transcript: &mut Transcript| transcript.language = Some("l".repeat(65)),
            |transcript: &mut Transcript| {
                transcript.analysis = Some("summary".into());
                transcript.analysis_model = Some("a".repeat(257));
            },
        ];
        for update in updates {
            let mut transcript = exported();
            update(&mut transcript);
            let error = prepare_json(&serde_json::to_vec(&vec![transcript]).unwrap()).unwrap_err();
            assert_eq!(error.code, "invalid_import_record");
        }
    }

    #[test]
    fn fingerprint_changes_with_any_previewed_byte() {
        let first = sha256_fingerprint(br#"[{"id":"tx_1"}]"#);
        let second = sha256_fingerprint(br#"[{"id":"tx_2"}]"#);
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
        assert!(verify_fingerprint(&first, &first).is_ok());
        let error = verify_fingerprint(&second, &first).unwrap_err();
        assert_eq!(error.code, "import_file_changed");
    }
}
