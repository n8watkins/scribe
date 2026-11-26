//! An end-to-end self-test for the installed app, used by the release smoke
//! test: `Scribe.exe --self-test-transcribe <audio.wav> <result.json>`.
//!
//! The app starts exactly as it does for a user (same resources, database,
//! bundled whisper.cpp and model store), downloads the selected model through
//! the normal verified download path if it is missing, transcribes the file
//! through the same path as File transcription, writes the outcome as JSON and
//! exits. Nothing else changes: without the flag the app never enters this
//! module.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{backend::BackendState, core::Core, file_transcribe, host::Host, model_manager};

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
    let Some(core) = app.try_state::<Core>() else {
        log::error!("Could not start the self-test: the engine is not ready yet");
        app.exit(1);
        return;
    };
    spawn_with(app, &core, request);
}

/// [`spawn`] on the engine. The app's version and the exit go through the
/// host: Dev and installer-upgrade builds override the version with
/// `--config`, so it is not the engine crate's `CARGO_PKG_VERSION`. Still
/// takes the app, only to pass it on to `file_transcribe::transcribe_file_with`
/// (until P1-4 step 4).
pub fn spawn_with(app: &AppHandle, core: &Core, request: SelfTestRequest) {
    let worker_app = app.clone();
    let worker_core = core.clone();
    start(core.host().clone(), request, move |app_version, audio| {
        run(&worker_app, &worker_core, app_version, audio)
    });
}

/// The body of [`spawn_with`] over the host and the run itself, so tests can
/// drive it without a `Core`. Reads the version on the calling thread, runs
/// the self-test on its own thread, writes the result, and exits through the
/// host with 0 on success and 1 otherwise (and 1 when the thread cannot
/// start).
fn start(
    host: Arc<dyn Host>,
    request: SelfTestRequest,
    run: impl FnOnce(String, &Path) -> SelfTestResult + Send + 'static,
) {
    let app_version = host.app_version();
    let exit_host = host.clone();
    let spawned = std::thread::Builder::new()
        .name("self-test".into())
        .spawn(move || {
            let outcome = run(app_version, &request.audio);
            let ok = outcome.ok;
            if let Err(error) = write_result(&request.result, &outcome) {
                log::error!("Self-test could not write its result. {}", error);
            }
            log::info!("Self-test finished (ok: {})", ok);
            exit_host.exit(if ok { 0 } else { 1 });
        });
    if let Err(error) = spawned {
        log::error!("Could not start the self-test thread. {}", error);
        host.exit(1);
    }
}

/// `app_version` is the running app's version (see [`spawn_with`]). Still
/// takes the app for `file_transcribe::transcribe_file_with`.
fn run(app: &AppHandle, core: &Core, app_version: String, audio: &Path) -> SelfTestResult {
    let mut result = SelfTestResult {
        ok: false,
        app_version,
        model_id: None,
        text: None,
        latency_ms: None,
        error: None,
    };
    log::info!("Self-test: transcribing {}", audio.display());

    if let Err(error) = ensure_selected_model(core.host().as_ref(), core.backend(), &mut result) {
        result.error = Some(error);
        return result;
    }

    match file_transcribe::transcribe_file_with(app, core, &audio.to_string_lossy()) {
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
fn ensure_selected_model(
    host: &dyn Host,
    state: &BackendState,
    result: &mut SelfTestResult,
) -> Result<(), String> {
    // The download takes the database lock for each short step, so the lock
    // is released before it starts.
    let (model_id, present) = {
        let db = state.db().map_err(|error| error.message)?;
        let model_id = db
            .get_settings()
            .map_err(|error| error.message)?
            .selected_model_id
            .unwrap_or_else(|| crate::models::DEFAULT_MODEL_ID.to_string());
        let present = model_manager::selected_model_path_with(host, &db).is_ok();
        (model_id, present)
    };
    result.model_id = Some(model_id.clone());
    if present {
        return Ok(());
    }
    log::info!("Self-test: downloading model {}", model_id);
    model_manager::download_model_with(host, state, &model_id)
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

    use crate::host::{Flavor, HostCall, RecordingHost};
    use crate::paths::Paths;

    /// A Dev build's version: not the engine crate's `CARGO_PKG_VERSION`.
    const DEV_VERSION: &str = "0.16.4-57";

    /// Runs [`start`] with `run` on a [`RecordingHost`] that answers
    /// [`DEV_VERSION`], waits for the self-test to exit, and returns the
    /// host's calls, the version `run` was handed, and the result file.
    fn self_test(ok: bool) -> (Vec<HostCall>, String, serde_json::Value, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let recorder = Arc::new(
            RecordingHost::new(
                Paths::from_parts(
                    root.path().join("data"),
                    root.path().join("cache"),
                    root.path().join("logs"),
                    None,
                    None,
                ),
                Flavor::Dev,
            )
            .with_app_version(DEV_VERSION),
        );
        let request = SelfTestRequest {
            audio: root.path().join("jfk.wav"),
            result: root.path().join("result.json"),
        };
        let audio_path = request.audio.clone();
        let (handed, handed_rx) = std::sync::mpsc::channel();
        start(recorder.clone(), request, move |app_version, audio| {
            handed.send(app_version.clone()).unwrap();
            assert_eq!(audio, audio_path);
            SelfTestResult {
                ok,
                app_version,
                model_id: Some("base.en".to_string()),
                text: ok.then(|| "ask not what your country can do".to_string()),
                latency_ms: Some(12),
                error: (!ok).then(|| "The transcript was empty.".to_string()),
            }
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while recorder.calls().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "the self-test never exited"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let handed = handed_rx.recv().unwrap();
        let result = serde_json::from_str(
            &std::fs::read_to_string(root.path().join("result.json")).unwrap(),
        )
        .unwrap();
        // The self-test sends no event of its own.
        assert!(recorder.events().is_empty());
        (recorder.calls(), handed, result, root)
    }

    #[test]
    fn a_passing_self_test_reports_the_hosts_version_and_exits_with_0() {
        let (calls, handed, result, _root) = self_test(true);
        assert_eq!(calls, [HostCall::Exit(0)]);
        assert_eq!(handed, DEV_VERSION);
        assert_eq!(result["ok"], true);
        // What the smoke tests compare with the version they installed.
        assert_eq!(result["appVersion"], DEV_VERSION);
        assert_ne!(result["appVersion"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_failing_self_test_exits_with_1_once() {
        let (calls, handed, result, _root) = self_test(false);
        assert_eq!(calls, [HostCall::Exit(1)]);
        assert_eq!(handed, DEV_VERSION);
        assert_eq!(result["ok"], false);
        assert_eq!(result["appVersion"], DEV_VERSION);
        assert_eq!(result["error"], "The transcript was empty.");
    }
}
