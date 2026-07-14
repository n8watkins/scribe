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

/// Health snapshot of the local LLM server for the Settings connection card.
/// `reachable: false` is a normal result (server not running), not a hard
/// error — `check_status` never returns `Err`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmStatus {
    pub reachable: bool,
    pub endpoint: String,
    pub models: Vec<String>,
    pub error: Option<String>,
}

/// Probes the local LLM server with a short-timeout `GET {endpoint}/models`
/// and reports whether it is reachable plus the model ids it advertises.
/// Unreachable / bad-response cases come back as `reachable: false` with a
/// friendly `error`, never as `Err`.
pub fn check_status(endpoint: &str) -> LlmStatus {
    let endpoint = endpoint.trim().trim_end_matches('/').to_string();

    if endpoint.is_empty() {
        return LlmStatus {
            reachable: false,
            endpoint,
            models: Vec::new(),
            error: Some("No server endpoint is set.".to_string()),
        };
    }

    let unreachable = |error: String| LlmStatus {
        reachable: false,
        endpoint: endpoint.clone(),
        models: Vec::new(),
        error: Some(error),
    };

    let client = match reqwest::blocking::Client::builder()
        .user_agent(concat!("Scribe/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(error) => return unreachable(format!("Could not build the HTTP client. {}", error)),
    };

    let response = match client
        .get(format!("{}/models", endpoint))
        .timeout(MODELS_TIMEOUT)
        .send()
    {
        Ok(response) => response,
        Err(_) => {
            return unreachable(format!(
                "Could not reach a local LLM server at {}. Is it running?",
                endpoint
            ))
        }
    };

    let status = response.status();
    let text = match response.text() {
        Ok(text) => text,
        Err(error) => return unreachable(format!("Could not read the model list. {}", error)),
    };

    if !status.is_success() {
        return unreachable(format!(
            "The server at {} returned HTTP {}.",
            endpoint, status
        ));
    }

    let body: serde_json::Value = match serde_json::from_str(&text) {
        Ok(body) => body,
        Err(_) => {
            return unreachable(format!(
                "The server at {} did not return an OpenAI-compatible model list.",
                endpoint
            ))
        }
    };

    let models = body
        .get("data")
        .and_then(|data| data.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("id").and_then(|id| id.as_str()))
                .map(ToOwned::to_owned)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    LlmStatus {
        reachable: true,
        endpoint,
        models,
        error: None,
    }
}

pub fn analyze_text(
    endpoint: &str,
    model: &str,
    prompt: &str,
    note_text: &str,
) -> Result<AnalysisOutcome, CommandError> {
    analyze_text_with_timeout(endpoint, model, prompt, note_text, REQUEST_TIMEOUT)
}

/// Like [`analyze_text`] but with a caller-chosen request timeout. The
/// dictation cleanup pass uses a short one so a slow/stuck local model never
/// stalls the user's output; notes analysis keeps the long default.
pub fn analyze_text_with_timeout(
    endpoint: &str,
    model: &str,
    prompt: &str,
    note_text: &str,
    timeout: Duration,
) -> Result<AnalysisOutcome, CommandError> {
    let endpoint = endpoint.trim().trim_end_matches('/');

    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("Scribe/", env!("CARGO_PKG_VERSION")))
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
        .timeout(timeout)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// One-shot OpenAI-compatible mock: answers `responses` connections in
    /// order, capturing each request's first line + body.
    fn mock_server(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());

        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 65536];
                let mut request = Vec::new();
                // Read until the headers are complete, then the body per
                // Content-Length (requests here are small and unchunked).
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some(headers_end) = text.find("\r\n\r\n") {
                        let content_length = text
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= headers_end + 4 + content_length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                let payload = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
                stream.write_all(payload.as_bytes()).unwrap();
            }
            requests
        });

        (endpoint, handle)
    }

    fn completion_response(content: &str, model: &str) -> String {
        serde_json::json!({
            "model": model,
            "choices": [{ "message": { "role": "assistant", "content": content } }],
        })
        .to_string()
    }

    #[test]
    fn explicit_model_round_trips_prompt_and_note() {
        let (endpoint, handle) = mock_server(vec![completion_response(
            " Summary.\n- Do the thing ",
            "qwen2.5-7b-instruct",
        )]);

        let outcome = analyze_text(
            &format!("{}/", endpoint), // trailing slash must be tolerated
            "my-model",
            "Summarize.",
            "note text here",
        )
        .unwrap();

        assert_eq!(outcome.analysis, "Summary.\n- Do the thing");
        // The server-reported model wins over the requested one.
        assert_eq!(outcome.model, "qwen2.5-7b-instruct");

        let requests = handle.join().unwrap();
        assert!(requests[0].starts_with("POST /v1/chat/completions"));
        assert!(requests[0].contains("\"my-model\""));
        assert!(requests[0].contains("Summarize."));
        assert!(requests[0].contains("note text here"));
    }

    #[test]
    fn empty_model_uses_first_listed_model() {
        let models = serde_json::json!({
            "data": [{ "id": "loaded-model" }, { "id": "other" }],
        })
        .to_string();
        let (endpoint, handle) =
            mock_server(vec![models, completion_response("ok", "loaded-model")]);

        let outcome = analyze_text(&endpoint, "  ", "p", "n").unwrap();
        assert_eq!(outcome.model, "loaded-model");

        let requests = handle.join().unwrap();
        assert!(requests[0].starts_with("GET /v1/models"));
        assert!(requests[1].contains("\"loaded-model\""));
    }

    #[test]
    fn empty_model_list_is_a_clear_error() {
        let (endpoint, handle) = mock_server(vec![serde_json::json!({ "data": [] }).to_string()]);

        let error = analyze_text(&endpoint, "", "p", "n").unwrap_err();
        assert!(error.to_string().contains("lists no models"));
        handle.join().unwrap();
    }

    #[test]
    fn blank_completion_content_is_an_error() {
        let (endpoint, handle) = mock_server(vec![completion_response("   ", "m")]);

        let error = analyze_text(&endpoint, "m", "p", "n").unwrap_err();
        assert!(error.to_string().contains("no analysis text"));
        handle.join().unwrap();
    }

    #[test]
    fn unreachable_server_mentions_the_endpoint() {
        // A port from the dynamic range with nothing listening.
        let error = analyze_text("http://127.0.0.1:59997/v1", "m", "p", "n").unwrap_err();
        assert!(error.to_string().contains("127.0.0.1:59997"));
    }

    #[test]
    fn check_status_lists_models_when_reachable() {
        let models = serde_json::json!({
            "data": [{ "id": "loaded-model" }, { "id": "other" }],
        })
        .to_string();
        let (endpoint, handle) = mock_server(vec![models]);

        // Trailing slash must be tolerated like analyze_text does.
        let status = check_status(&format!("{}/", endpoint));
        assert!(status.reachable);
        assert!(status.error.is_none());
        assert_eq!(status.models, vec!["loaded-model", "other"]);
        // The endpoint is normalized (no trailing slash).
        assert_eq!(status.endpoint, endpoint);

        let requests = handle.join().unwrap();
        assert!(requests[0].starts_with("GET /v1/models"));
    }

    #[test]
    fn check_status_reachable_with_empty_model_list() {
        let (endpoint, handle) = mock_server(vec![serde_json::json!({ "data": [] }).to_string()]);

        let status = check_status(&endpoint);
        assert!(status.reachable);
        assert!(status.models.is_empty());
        assert!(status.error.is_none());
        handle.join().unwrap();
    }

    #[test]
    fn check_status_unreachable_is_not_an_error() {
        // A port from the dynamic range with nothing listening.
        let status = check_status("http://127.0.0.1:59997/v1");
        assert!(!status.reachable);
        assert!(status.models.is_empty());
        assert_eq!(status.endpoint, "http://127.0.0.1:59997/v1");
        assert!(status.error.as_deref().unwrap().contains("127.0.0.1:59997"));
    }

    #[test]
    fn check_status_empty_endpoint_is_unreachable() {
        let status = check_status("   ");
        assert!(!status.reachable);
        assert!(status.error.is_some());
        assert!(status.endpoint.is_empty());
    }
}
