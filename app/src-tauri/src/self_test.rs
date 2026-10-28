//! An end-to-end self-test for the installed app, used by the release smoke
//! test: `Scribe.exe --self-test-transcribe <audio.wav> <result.json>`.
//!
//! The app starts exactly as it does for a user (same resources, database,
//! bundled whisper.cpp and model store), downloads the selected model through
//! the normal verified download path if it is missing, transcribes the file
//! through the same path as File transcription, writes the outcome as JSON and
//! exits. Nothing else changes: without the flag the app never enters this
//! module.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{commands::BackendState, file_transcribe, model_manager};

/// The command-line flag that starts a self-test run.
pub const FLAG: &str = "--self-test-transcribe";

/// What a self-test run was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfTestRequest {
    pub audio: PathBuf,
    pub result: PathBuf,
}

/// The JSON written to the result path.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfTestResult {
    ok: bool,
    app_version: String,
    model_id: Option<String>,
    text: Option<String>,
    latency_ms: Option<u32>,
    error: Option<String>,
}

/// Reads `--self-test-transcribe <audio> <result>` from the process arguments.
/// Returns `None` when the flag is absent or incomplete.
pub fn parse_args<I, S>(args: I) -> Option<SelfTestRequest>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let index = args.iter().position(|arg| arg == FLAG)?;
    let audio = args.get(index + 1)?;
    let result = args.get(index + 2)?;
    if audio.starts_with("--") || result.starts_with("--") {
        return None;
    }
    Some(SelfTestRequest {
        audio: PathBuf::from(audio),
        result: PathBuf::from(result),
    })
}

/// Runs the self-test on a worker thread, then exits the app with 0 on a
/// non-empty transcript and 1 otherwise.
pub fn spawn(app: &AppHandle, request: SelfTestRequest) {
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("self-test".into())
        .spawn(move || {
            let outcome = run(&handle, &request.audio);
            let ok = outcome.ok;
            if let Err(error) = write_result(&request.result, &outcome) {
                log::error!("Self-test could not write its result. {}", error);
            }
            log::info!("Self-test finished (ok: {})", ok);
            handle.exit(if ok { 0 } else { 1 });
        });
    if let Err(error) = spawned {
        log::error!("Could not start the self-test thread. {}", error);
        app.exit(1);
    }
}

fn run(app: &AppHandle, audio: &Path) -> SelfTestResult {
    let mut result = SelfTestResult {
        ok: false,
        app_version: app.package_info().version.to_string(),
        model_id: None,
        text: None,
        latency_ms: None,
        error: None,
    };
    log::info!("Self-test: transcribing {}", audio.display());

    if let Err(error) = ensure_selected_model(app, &mut result) {
        result.error = Some(error);
        return result;
    }

    match file_transcribe::transcribe_file(app, &audio.to_string_lossy()) {
        Ok(transcription) => {
            result.ok = !transcription.text.trim().is_empty();
            if !result.ok {
                result.error = Some("The transcript was empty.".to_string());
            }
            result.latency_ms = Some(transcription.latency_ms);
            result.text = Some(transcription.text);
        }
        Err(error) => result.error = Some(error.message),
    }
    result
}

/// Downloads the selected model through the normal, SHA-256 verified path when
/// it is not on disk yet, so the self-test covers model download too.
fn ensure_selected_model(app: &AppHandle, result: &mut SelfTestResult) -> Result<(), String> {
    let state = app.state::<BackendState>();
    let db = state.db().map_err(|error| error.message)?;
    let model_id = db
        .get_settings()
        .map_err(|error| error.message)?
        .selected_model_id
        .unwrap_or_else(|| crate::models::DEFAULT_MODEL_ID.to_string());
    result.model_id = Some(model_id.clone());
    if model_manager::selected_model_path(app, &db).is_ok() {
        return Ok(());
    }
    log::info!("Self-test: downloading model {}", model_id);
    model_manager::download_model(app, &db, state.model_downloads(), &model_id)
        .map(|_| ())
        .map_err(|error| error.message)
}

fn write_result(path: &Path, result: &SelfTestResult) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(result).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_flag_and_its_two_paths() {
        let request = parse_args(["Scribe.exe", FLAG, "C:\\a b\\jfk.wav", "out.json"]).unwrap();
        assert_eq!(request.audio, PathBuf::from("C:\\a b\\jfk.wav"));
        assert_eq!(request.result, PathBuf::from("out.json"));
    }

    #[test]
    fn ignores_launches_without_a_complete_flag() {
        assert_eq!(parse_args(["Scribe.exe"]), None);
        assert_eq!(parse_args(["Scribe.exe", FLAG, "jfk.wav"]), None);
        assert_eq!(parse_args(["Scribe.exe", FLAG, "--autostart", "x"]), None);
    }
}
