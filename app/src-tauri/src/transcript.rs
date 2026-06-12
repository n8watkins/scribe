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
    /// Absolute path of the saved audio clip. None when clip saving is off
    /// or the transcript predates saved clips; the default keeps persisted
    /// pre-clip JSON (e.g. the Last Transcript Buffer) deserializing.
    #[serde(default)]
    pub audio_path: Option<String>,
    /// True for note-taking dictations (tilde+Q): saved to history but never
    /// auto-pasted, and listed in the dashboard's Notes view.
    #[serde(default)]
    pub is_note: bool,
    /// Local-LLM analysis of the transcript text (shape is whatever the
    /// user's analysis prompt asked for). Set on demand from the Notes view.
    #[serde(default)]
    pub analysis: Option<String>,
    /// The model that produced `analysis`, as reported by the LLM server.
    #[serde(default)]
    pub analysis_model: Option<String>,
    #[serde(default)]
    pub analysis_created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSearchResult {
    pub transcripts: Vec<Transcript>,
    pub total: u32,
    pub limit: u32,
    pub offset: u32,
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
            audio_path: None,
            is_note: false,
            analysis: None,
            analysis_model: None,
            analysis_created_at: None,
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
        // This is the single empty-transcription classifier: both the
        // incremental and full-clip dictation paths funnel through it, and a
        // None here means "benign empty dictation", never an error.
        assert!(Transcript::new_last_buffer("", Some(1000), None, None).is_none());
        assert!(Transcript::new_last_buffer("   ", Some(1000), None, None).is_none());
        assert!(Transcript::new_last_buffer("\n\t \r\n", Some(1000), None, None).is_none());
    }

    #[test]
    fn audio_path_defaults_to_none_for_pre_clip_json() {
        // Buffer JSON persisted before saved clips existed has no audioPath
        // key and must keep deserializing.
        let json = r#"{
            "id": "tx_legacy",
            "text": "hello",
            "createdAt": "2026-01-01T00:00:00Z",
            "durationMs": 1000,
            "wordCount": 1,
            "characterCount": 5,
            "modelId": null,
            "language": "en",
            "outputMode": null,
            "pasteMethod": null,
            "transcriptionLatencyMs": null
        }"#;

        let transcript: Transcript = serde_json::from_str(json).unwrap();
        assert_eq!(transcript.audio_path, None);
    }

    #[test]
    fn audio_path_serializes_as_camel_case() {
        let mut transcript = Transcript::new_last_buffer("hello", None, None, None).unwrap();
        transcript.audio_path = Some("C:\\clips\\tx_1.wav".to_string());

        let json = serde_json::to_value(&transcript).unwrap();
        assert_eq!(json["audioPath"], "C:\\clips\\tx_1.wav");
    }

    #[test]
    fn last_buffer_accepts_text_with_surrounding_whitespace() {
        let transcript = Transcript::new_last_buffer(" hello \n", Some(1000), None, None).unwrap();

        assert_eq!(transcript.word_count, 1);
    }
}
