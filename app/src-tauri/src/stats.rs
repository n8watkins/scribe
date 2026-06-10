use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicStats {
    pub words_today: u32,
    pub dictations_today: u32,
    pub average_wpm: Option<f64>,
    pub average_transcription_latency_ms: Option<f64>,
    pub average_recording_duration_ms: Option<f64>,
    pub most_used_model: Option<String>,
    pub total_words_transcribed: u64,
}

impl Default for BasicStats {
    fn default() -> Self {
        Self {
            words_today: 0,
            dictations_today: 0,
            average_wpm: None,
            average_transcription_latency_ms: None,
            average_recording_duration_ms: None,
            most_used_model: None,
            total_words_transcribed: 0,
        }
    }
}
