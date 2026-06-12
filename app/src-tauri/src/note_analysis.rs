//! On-demand local-LLM analysis of note transcripts via an OpenAI-compatible
//! chat-completions API (LM Studio by default, Ollama works too). The user's
//! analysis prompt is the system message and the note text is the user
//! message, so the prompt alone defines what "analysis" produces.

use std::time::Duration;

use serde::Serialize;
use serde_json::json;

use crate::error::CommandError;

/// Local models on modest hardware can take a while on long notes.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
/// Listing loaded models is instant when the server is up; fail fast when
/// it is not.
const MODELS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisOutcome {
    pub analysis: String,
    pub model: String,
}

pub fn analyze_text(
    endpoint: &str,
    model: &str,
    prompt: &str,
    note_text: &str,
) -> Result<AnalysisOutcome, CommandError> {
    let endpoint = endpoint.trim().trim_end_matches('/');

    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("LocalDictate/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| failure(error.to_string()))?;

    let model = match model.trim() {
        "" => first_listed_model(&client, endpoint)?,
        explicit => explicit.to_string(),
    };

    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": prompt },
            { "role": "user", "content": note_text },
        ],
        "stream": false,
    });

    let response = client
        .post(format!("{}/chat/completions", endpoint))
        .timeout(REQUEST_TIMEOUT)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .map_err(|error| {
            failure(format!(
                "Could not reach the local LLM server at {}. Is it running? {}",
                endpoint, error
            ))
        })?;

    let status = response.status();
    let text = response
        .text()
        .map_err(|error| failure(format!("Could not read the LLM response. {}", error)))?;

    if !status.is_success() {
        return Err(failure(format!(
            "The LLM server returned HTTP {}. {}",
            status,
            truncate(&text, 300)
        )));
    }

    let body: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| failure(format!("Could not parse the LLM response. {}", error)))?;

    let analysis = body
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .ok_or_else(|| failure("The LLM response contained no analysis text."))?
        .to_string();

    // Prefer the model name the server reports (it resolves aliases).
    let model = body
        .get("model")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(&model)
        .to_string();

    Ok(AnalysisOutcome { analysis, model })
}

/// The first model id reported by GET /models — on LM Studio that is the
/// loaded model, which makes an empty model setting "use whatever is loaded".
fn first_listed_model(
    client: &reqwest::blocking::Client,
    endpoint: &str,
) -> Result<String, CommandError> {
    let text = client
        .get(format!("{}/models", endpoint))
        .timeout(MODELS_TIMEOUT)
        .send()
        .map_err(|error| {
            failure(format!(
                "Could not reach the local LLM server at {}. Is it running? {}",
                endpoint, error
            ))
        })?
        .text()
        .map_err(|error| failure(format!("Could not read the model list. {}", error)))?;

    let body: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| failure(format!("Could not parse the model list. {}", error)))?;

    body.get("data")
        .and_then(|data| data.get(0))
        .and_then(|entry| entry.get("id"))
        .and_then(|id| id.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            failure("The LLM server lists no models. Load a model (e.g. in LM Studio) first.")
        })
}

fn failure(message: impl Into<String>) -> CommandError {
    CommandError::new("note_analysis_failed", message)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_chars).collect();
        format!("{}…", truncated)
    }
}
