use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::{OutputMode, PasteMethod};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub id: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub duration_ms: Option<u32>,
    pub word_count: u32,
    pub character_count: u32,
    pub model_id: Option<String>,
    pub language: Option<String>,
    pub output_mode: Option<OutputMode>,
    pub paste_method: Option<PasteMethod>,
    pub transcription_latency_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptMetadata {
    pub word_count: u32,
    pub character_count: u32,
}

impl Transcript {
    pub fn new_last_buffer(
        text: impl Into<String>,
        duration_ms: Option<u32>,
        model_id: Option<String>,
        language: Option<String>,
    ) -> Option<Self> {
        let text = text.into();
        if text.trim().is_empty() {
            return None;
        }

        let metadata = metadata_for_text(&text);

        Some(Self {
            id: format!("tx_{}", Uuid::new_v4().simple()),
            text,
            created_at: Utc::now(),
            duration_ms,
            word_count: metadata.word_count,
            character_count: metadata.character_count,
            model_id,
            language,
            output_mode: None,
            paste_method: None,
            transcription_latency_ms: None,
        })
    }
}

pub fn metadata_for_text(text: &str) -> TranscriptMetadata {
    TranscriptMetadata {
        word_count: text.split_whitespace().count() as u32,
        character_count: text.chars().count() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_counts_words_and_characters() {
        let metadata = metadata_for_text("Hello local dictation.");

        assert_eq!(metadata.word_count, 3);
        assert_eq!(metadata.character_count, 22);
    }

    #[test]
    fn last_buffer_rejects_empty_text() {
        assert!(Transcript::new_last_buffer("   ", Some(1000), None, None).is_none());
    }
}
