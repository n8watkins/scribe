//! Optional local-LLM polish of a finished dictation, applied just before the
//! transcript is saved and pasted. It strips filler words, fixes punctuation,
//! capitalization, and obvious transcription errors, and applies light
//! per-mode formatting — always faithful to the speaker's words.
//!
//! Non-blocking by contract: the call uses a short timeout and ANY failure
//! (server down, timeout, blank reply) falls back to the ORIGINAL transcript,
//! so the user always gets their words even when the LLM is slow or offline.

use std::time::Duration;

use crate::settings::DictationCleanupMode;

/// Cleanup is a quick touch-up, not a long analysis: cap the round-trip so a
/// stuck local model degrades to the raw transcript instead of stalling output.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(20);

/// The shared base instruction. Every mode builds on this faithful-cleanup
/// contract; the formatting modes append their own shaping on top.
const STANDARD_PROMPT: &str = "You clean up dictated speech-to-text. Fix \
    punctuation, capitalization, and obvious transcription errors, and remove \
    filler words (um, uh, like, you know). Keep the speaker's wording and \
    meaning faithful — do not summarize, add, or answer. Output ONLY the \
    cleaned text.";

const EMAIL_PROMPT: &str = "You clean up dictated speech-to-text and format it \
    as a polite email body. Fix punctuation, capitalization, and obvious \
    transcription errors, and remove filler words (um, uh, like, you know). \
    Keep the speaker's wording and meaning faithful — do not summarize, invent \
    details, or answer. Lay it out as a courteous email body with greeting and \
    sign-off only if the dictation implies them. Output ONLY the email text.";

const CHAT_PROMPT: &str = "You clean up dictated speech-to-text into a concise, \
    casual chat message. Fix punctuation, capitalization, and obvious \
    transcription errors, and remove filler words (um, uh, like, you know). \
    Keep the speaker's wording and meaning faithful — do not summarize away \
    content, add, or answer. Keep it natural and informal. Output ONLY the \
    message text.";

const CODE_PROMPT: &str = "You clean up dictated speech-to-text into a code \
    comment / technical phrasing. Fix punctuation, capitalization, and obvious \
    transcription errors, and remove filler words (um, uh, like, you know). \
    Keep the speaker's wording and meaning faithful — do not summarize, add, or \
    answer. Phrase it as a clear technical note or code comment. Output ONLY the \
    cleaned text.";

/// The system prompt for a given mode. `Custom` uses the user's prompt, falling
/// back to the Standard prompt when it is blank.
fn system_prompt(mode: DictationCleanupMode, custom_prompt: &str) -> &str {
    match mode {
        DictationCleanupMode::Standard => STANDARD_PROMPT,
        DictationCleanupMode::Email => EMAIL_PROMPT,
        DictationCleanupMode::Chat => CHAT_PROMPT,
        DictationCleanupMode::Code => CODE_PROMPT,
        DictationCleanupMode::Custom => {
            let trimmed = custom_prompt.trim();
            if trimmed.is_empty() {
                STANDARD_PROMPT
            } else {
                custom_prompt
            }
        }
    }
}

/// Polishes `text` with the local LLM, or returns it unchanged on any problem.
///
/// Contract: never panics, never blocks for long, and never returns empty for
/// non-empty input. Blank input is returned as-is without contacting the
/// server; otherwise a short-timeout LLM call is made and its trimmed result is
/// returned. On any error/timeout/blank reply this logs a warning and returns
/// the ORIGINAL `text`, so the dictation pipeline always has usable output.
pub fn cleanup(
    text: &str,
    mode: DictationCleanupMode,
    custom_prompt: &str,
    endpoint: &str,
    model: &str,
) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }

    let prompt = system_prompt(mode, custom_prompt);

    match crate::note_analysis::analyze_text_with_timeout(
        endpoint,
        model,
        prompt,
        text,
        CLEANUP_TIMEOUT,
    ) {
        Ok(outcome) if !outcome.analysis.trim().is_empty() => outcome.analysis,
        Ok(_) => {
            // analyze_text already rejects blank content, but guard anyway so a
            // future change can never silently blank the user's transcript.
            log::warn!("Dictation cleanup returned blank text; keeping the raw transcript.");
            text.to_string()
        }
        Err(error) => {
            log::warn!(
                "Dictation cleanup failed ({}); keeping the raw transcript.",
                error.message
            );
            text.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// One-shot OpenAI-compatible mock: answers `responses` connections in
    /// order, capturing each request's first line + body. (Mirrors the helper
    /// in note_analysis.rs tests.)
    fn mock_server(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());

        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 65536];
                let mut request = Vec::new();
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
    fn reachable_server_replaces_the_text() {
        let (endpoint, handle) = mock_server(vec![completion_response(
            "I am going to the store.",
            "local-model",
        )]);

        let cleaned = cleanup(
            "um i am going to the store like",
            DictationCleanupMode::Standard,
            "",
            &endpoint,
            "local-model",
        );

        assert_eq!(cleaned, "I am going to the store.");

        let requests = handle.join().unwrap();
        assert!(requests[0].starts_with("POST /v1/chat/completions"));
        // The Standard system prompt drove the request.
        assert!(requests[0].contains("clean up dictated speech-to-text"));
        // The raw transcript was the user message.
        assert!(requests[0].contains("um i am going to the store like"));
    }

    #[test]
    fn unreachable_endpoint_falls_back_to_original_text() {
        // A port from the dynamic range with nothing listening.
        let original = "um this should survive unchanged";
        let cleaned = cleanup(
            original,
            DictationCleanupMode::Standard,
            "",
            "http://127.0.0.1:59996/v1",
            "m",
        );

        assert_eq!(cleaned, original);
    }

    #[test]
    fn blank_input_returns_blank_without_calling_the_server() {
        // No mock server: a contacted server would hang the test, so reaching
        // for one here would fail. Blank in -> blank out, untouched.
        let cleaned = cleanup(
            "   \n  ",
            DictationCleanupMode::Standard,
            "",
            "http://127.0.0.1:59995/v1",
            "m",
        );

        assert_eq!(cleaned, "   \n  ");
    }

    #[test]
    fn custom_mode_uses_the_user_prompt() {
        let (endpoint, handle) =
            mock_server(vec![completion_response("done", "local-model")]);

        let _ = cleanup(
            "some words",
            DictationCleanupMode::Custom,
            "Translate to pirate speak.",
            &endpoint,
            "local-model",
        );

        let requests = handle.join().unwrap();
        assert!(requests[0].contains("Translate to pirate speak."));
    }

    #[test]
    fn custom_mode_blank_prompt_falls_back_to_standard() {
        let (endpoint, handle) =
            mock_server(vec![completion_response("done", "local-model")]);

        let _ = cleanup(
            "some words",
            DictationCleanupMode::Custom,
            "   ",
            &endpoint,
            "local-model",
        );

        let requests = handle.join().unwrap();
        assert!(requests[0].contains("clean up dictated speech-to-text"));
    }
}
