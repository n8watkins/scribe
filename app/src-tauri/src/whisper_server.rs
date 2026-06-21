//! Warm transcription backend: keeps a resident `whisper-server.exe` process
//! alive across dictations so the GGML model is loaded once instead of on
//! every transcription.
//!
//! Design notes:
//! - `WarmTranscriber::transcribe` is a stateless, serialized primitive:
//!   request in (WAV + model + language + prompt), text out. Nothing assumes
//!   "one transcription per recording", so incremental/segment transcription
//!   can reuse it as-is.
//! - The server is spawned lazily on first use and reused while the model and
//!   language match. The vocabulary prompt is sent per request, so prompt
//!   changes never require a server restart.
//! - After 10 minutes without a transcription a watchdog thread stops the
//!   server so it does not linger holding model memory.
//! - Any server-path failure falls back to the `whisper-cli.exe` path for
//!   that request. After 3 consecutive server failures the server path is
//!   disabled for the rest of the session; a model/language change re-enables
//!   it.

use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Deserialize;
use tauri::AppHandle;

use crate::{
    error::CommandError,
    whisper::{self, WhisperRequest, WhisperTranscription},
};

const SERVER_HOST: &str = "127.0.0.1";
const SERVER_EXECUTABLE: &str = "whisper-server.exe";
/// Stop the server after this long without a transcription.
const IDLE_SHUTDOWN: Duration = Duration::from_secs(10 * 60);
/// How often the watchdog thread checks for idleness.
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(30);
/// Hard cap on waiting for the server to load the model and start listening.
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Generous per-request cap so long dictations on big models still finish.
const INFERENCE_TIMEOUT: Duration = Duration::from_secs(300);
/// Consecutive server failures before the server path is disabled.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// The launch configuration a server process is bound to. A request whose
/// configuration differs requires a restart; everything else (WAV, prompt)
/// is per-request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerConfig {
    model_path: PathBuf,
    language: String,
    /// Translate task (English output). Part of the launch config: changing it
    /// requires a server restart, since it is passed as a launch argument.
    translate: bool,
    /// GPU acceleration preference. A launch-time concern (`--no-gpu`), so a
    /// change restarts the server.
    gpu: crate::settings::GpuAcceleration,
    /// Pinned Vulkan device index (set as an env var at spawn). Also launch-time,
    /// so a change restarts the server.
    gpu_device_index: Option<u32>,
}

impl ServerConfig {
    fn for_request(request: &WhisperRequest) -> Self {
        Self {
            model_path: request.model_path.clone(),
            language: request.language.clone(),
            translate: request.translate,
            gpu: request.gpu,
            gpu_device_index: request.gpu_device_index,
        }
    }
}

/// Failure policy: fall back per request, disable the server path for the
/// session after `MAX_CONSECUTIVE_FAILURES` consecutive failures, re-enable
/// when the configuration (model) changes.
#[derive(Debug, Default)]
struct FailureTracker {
    consecutive_failures: u32,
    disabled: bool,
}

impl FailureTracker {
    /// Records a failure. Returns true when this failure newly disabled the
    /// server path (so the caller can log the disablement exactly once).
    fn record_failure(&mut self) -> bool {
        if self.disabled {
            return false;
        }
        self.consecutive_failures += 1;
        if self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            self.disabled = true;
            return true;
        }
        false
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    fn reset(&mut self) {
        self.consecutive_failures = 0;
        self.disabled = false;
    }

    fn server_allowed(&self) -> bool {
        !self.disabled
    }
}

struct ServerProcess {
    child: Child,
    port: u16,
    config: ServerConfig,
}

impl ServerProcess {
    fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Kills the child process. Safe to call multiple times (kill on an
    /// already-exited child is a no-op error that we ignore).
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

#[derive(Default)]
struct Inner {
    server: Option<ServerProcess>,
    /// Configuration of the most recent server attempt; used to detect model
    /// changes even while the server path is disabled.
    last_config: Option<ServerConfig>,
    failures: FailureTracker,
    last_used: Option<Instant>,
    http: Option<reqwest::blocking::Client>,
    watchdog_running: bool,
}

impl Inner {
    fn stop_server(&mut self) {
        if let Some(mut server) = self.server.take() {
            server.kill();
        }
    }
}

/// Tauri-managed warm transcription service. `Send + Sync`; all internal
/// state lives behind one mutex, which also serializes transcriptions (the
/// server runs one inference at a time anyway).
pub struct WarmTranscriber {
    inner: Arc<Mutex<Inner>>,
}

impl Default for WarmTranscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl WarmTranscriber {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
        }
    }

    /// Transcribes one WAV. Tries the warm server first and transparently
    /// falls back to the `whisper-cli.exe` path on any server failure.
    pub fn transcribe(
        &self,
        app: &AppHandle,
        request: WhisperRequest,
    ) -> Result<WhisperTranscription, CommandError> {
        // FILLER: the warm server now serves filler requests too — it returns
        // verbose_json with per-word timings (parsed in transcribe_via_server),
        // so suppression runs on the warm path with no model-reload overhead. The
        // whisper-cli path stays the fallback (and also handles filler).

        // Missing inputs are user-facing errors, not server failures; let the
        // CLI path produce its existing error messages without touching the
        // failure counter.
        if request.model_path.is_file() && request.wav_path.is_file() {
            if let Some(transcription) = self.try_server(app, &request) {
                return Ok(transcription);
            }
        }

        // whisper-cli fallback. It uses the GPU per the request; if that fails
        // while the GPU was in use (a driver crash or VRAM OOM on a big model),
        // retry once on CPU (--no-gpu) so a GPU problem degrades to
        // slower-but-working instead of losing the dictation. WS5.
        match whisper::transcribe(app, request.clone()) {
            Ok(transcription) => Ok(transcription),
            Err(error) => match cpu_retry_request(&request) {
                Some(cpu_request) => {
                    log::warn!(
                        "GPU transcription failed ({}); retrying once on CPU (--no-gpu)",
                        error.message
                    );
                    whisper::transcribe(app, cpu_request)
                }
                None => Err(error),
            },
        }
    }

    /// Stops the resident server. Called on app exit; double-stop is safe.
    pub fn shutdown(&self) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if inner.server.is_some() {
            log::info!("Stopping whisper-server");
            inner.stop_server();
        }
    }

    /// Attempts the warm-server path. Returns None when the server path is
    /// unavailable or failed (the caller falls back to whisper-cli).
    fn try_server(
        &self,
        app: &AppHandle,
        request: &WhisperRequest,
    ) -> Option<WhisperTranscription> {
        if !cfg!(windows) {
            return None;
        }

        let desired = ServerConfig::for_request(request);
        let Ok(mut inner) = self.inner.lock() else {
            return None;
        };

        // A model (or language) change resets the failure policy.
        if inner.last_config.as_ref() != Some(&desired) {
            if !inner.failures.server_allowed() {
                log::info!("Model changed; re-enabling whisper-server after earlier failures");
            }
            inner.failures.reset();
            inner.last_config = Some(desired.clone());
        }

        if !inner.failures.server_allowed() {
            return None;
        }

        let started = Instant::now();
        match self.transcribe_via_server(app, &mut inner, &desired, request) {
            Ok(text) => {
                let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
                inner.failures.record_success();
                inner.last_used = Some(Instant::now());
                Some(WhisperTranscription { text, latency_ms })
            }
            Err(error) => {
                log::warn!(
                    "whisper-server transcription failed; falling back to whisper-cli. {}",
                    error
                );
                // The server state is unknown after a failure; restart fresh
                // on the next attempt.
                inner.stop_server();
                if inner.failures.record_failure() {
                    log::warn!(
                        "Disabling whisper-server for the rest of the session after {} consecutive failures",
                        MAX_CONSECUTIVE_FAILURES
                    );
                }
                None
            }
        }
    }

    fn transcribe_via_server(
        &self,
        app: &AppHandle,
        inner: &mut Inner,
        config: &ServerConfig,
        request: &WhisperRequest,
    ) -> Result<String, String> {
        let client = http_client(inner)?;
        ensure_server(app, inner, config, &client)?;
        self.start_watchdog(inner);

        let port = inner
            .server
            .as_ref()
            .map(|server| server.port)
            .ok_or_else(|| "whisper-server is not running".to_string())?;

        let wav_bytes = fs::read(&request.wav_path)
            .map_err(|error| format!("Could not read recording WAV. {}", error))?;

        let mut form = reqwest::blocking::multipart::Form::new()
            .part(
                "file",
                reqwest::blocking::multipart::Part::bytes(wav_bytes)
                    .file_name("audio.wav")
                    .mime_str("audio/wav")
                    .map_err(|error| format!("Could not build multipart request. {}", error))?,
            )
            // FILLER: verbose_json carries per-word timings; plain json otherwise.
            .text(
                "response_format",
                if request.filler.is_some() {
                    "verbose_json"
                } else {
                    "json"
                },
            );

        let vocabulary_prompt = request.vocabulary_prompt.trim();
        if !vocabulary_prompt.is_empty() {
            form = form.text("prompt", vocabulary_prompt.to_string());
        }

        let response = client
            .post(inference_url(port))
            .multipart(form)
            .timeout(INFERENCE_TIMEOUT)
            .send()
            .map_err(|error| format!("whisper-server request failed. {}", error))?;

        let status = response.status();
        let body = response
            .text()
            .map_err(|error| format!("Could not read whisper-server response. {}", error))?;

        if !status.is_success() {
            return Err(format!(
                "whisper-server returned HTTP {}. {}",
                status,
                body.trim()
            ));
        }

        // FILLER: with a config, drop pause-bracketed fillers using the per-word
        // timings; if the words array is missing/unparseable, fall back to the
        // verbose_json top-level `text` so a hiccup never loses the dictation.
        if let Some(config) = &request.filler {
            let words = parse_server_words(&body);
            if !words.is_empty() {
                return Ok(config.apply(&words)); // normalizes internally
            }
        }

        let parsed: InferenceResponse = serde_json::from_str(&body)
            .map_err(|error| format!("Could not parse whisper-server response. {}", error))?;

        Ok(whisper::normalize_transcript_text(&parsed.text))
    }

    /// Starts the idle watchdog once per app session. It holds only a weak
    /// reference so it never keeps the transcriber alive.
    fn start_watchdog(&self, inner: &mut Inner) {
        if inner.watchdog_running {
            return;
        }
        inner.watchdog_running = true;

        let weak = Arc::downgrade(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("whisper-server-watchdog".to_string())
            .spawn(move || loop {
                std::thread::sleep(WATCHDOG_INTERVAL);
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let Ok(mut inner) = inner.lock() else {
                    return;
                };
                if inner.server.is_none() {
                    continue;
                }
                let last_used = inner.last_used.unwrap_or_else(Instant::now);
                if idle_expired(last_used, Instant::now()) {
                    log::info!(
                        "Stopping whisper-server after {} minutes idle",
                        IDLE_SHUTDOWN.as_secs() / 60
                    );
                    inner.stop_server();
                }
            });

        if let Err(error) = spawned {
            inner.watchdog_running = false;
            log::warn!("Could not start whisper-server idle watchdog. {}", error);
        }
    }
}

impl Drop for WarmTranscriber {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Deserialize)]
struct InferenceResponse {
    text: String,
}

/// FILLER: reconstruct timed words from whisper-server `verbose_json`. Each
/// segment carries a `words` array of `{word, start, end}` (start/end in
/// seconds). Returns empty on any parse problem, so the caller falls back to the
/// plain-text `text` field — a hiccup never loses the dictation.
fn parse_server_words(body: &str) -> Vec<crate::filler::TimedWord> {
    use crate::filler::TimedWord;
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(segments) = value.get("segments").and_then(|s| s.as_array()) else {
        return Vec::new();
    };
    let mut words = Vec::new();
    for segment in segments {
        let Some(arr) = segment.get("words").and_then(|w| w.as_array()) else {
            continue;
        };
        for word in arr {
            let text = word
                .get("word")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if text.is_empty() {
                continue;
            }
            let start = word.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let end = word.get("end").and_then(|v| v.as_f64()).unwrap_or(start);
            words.push(TimedWord::new(
                text.to_string(),
                (start * 1000.0).round() as i64,
                (end * 1000.0).round() as i64,
            ));
        }
    }
    words
}

fn http_client(inner: &mut Inner) -> Result<reqwest::blocking::Client, String> {
    if inner.http.is_none() {
        let client = reqwest::blocking::Client::builder()
            .timeout(INFERENCE_TIMEOUT)
            .build()
            .map_err(|error| {
                format!("Could not initialize whisper-server HTTP client. {}", error)
            })?;
        inner.http = Some(client);
    }
    Ok(inner.http.clone().expect("client initialized above"))
}

/// Ensures a healthy server process matching `config` is running.
fn ensure_server(
    app: &AppHandle,
    inner: &mut Inner,
    config: &ServerConfig,
    client: &reqwest::blocking::Client,
) -> Result<(), String> {
    if let Some(server) = inner.server.as_mut() {
        if server.has_exited() {
            log::warn!("whisper-server exited unexpectedly; restarting");
            inner.stop_server();
        }
    }

    if let Some(server) = inner.server.as_ref() {
        if needs_restart(&server.config, config) {
            log::info!(
                "Restarting whisper-server for model {} (language {})",
                config.model_path.display(),
                config.language
            );
            inner.stop_server();
        }
    }

    if inner.server.is_some() {
        return Ok(());
    }

    let executable = whisper::resolve_bundled_executable(app, SERVER_EXECUTABLE)
        .map_err(|error| error.message)?;
    let port = find_free_port()?;
    let threads = server_threads();
    let mut args = server_args(
        &config.model_path,
        &config.language,
        config.translate,
        port,
        threads,
    );
    // GPU off => --no-gpu, mirroring the whisper-cli path (whisper::push_gpu_args).
    whisper::push_gpu_args(&mut args, config.gpu.is_off());

    log::info!(
        "Starting whisper-server on {}:{} (model {}, language {}, {} threads)",
        SERVER_HOST,
        port,
        config.model_path.display(),
        config.language,
        threads
    );
    let started = Instant::now();
    let child = spawn_server_process(
        &executable,
        &args,
        whisper::gpu_visible_devices_env(config.gpu, config.gpu_device_index),
    )?;
    let mut server = ServerProcess {
        child,
        port,
        config: config.clone(),
    };

    // On failure, `server` is dropped here, which kills the child.
    wait_until_healthy(client, &mut server)?;
    log::info!(
        "whisper-server ready on port {} after {} ms",
        port,
        started.elapsed().as_millis()
    );
    inner.server = Some(server);
    Ok(())
}

fn spawn_server_process(
    executable: &Path,
    args: &[String],
    vk_visible_devices: Option<String>,
) -> Result<Child, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let mut command = std::process::Command::new(executable);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);

        // Pin a specific Vulkan device for the resident server (multi-GPU boxes);
        // absent means ggml picks its default device.
        if let Some(devices) = vk_visible_devices {
            command.env("GGML_VK_VISIBLE_DEVICES", devices);
        }

        command
            .spawn()
            .map_err(|error| format!("Could not start whisper-server. {}", error))
    }

    #[cfg(not(windows))]
    {
        let _ = (executable, args, vk_visible_devices, Stdio::null());
        Err("whisper-server is only supported on Windows.".to_string())
    }
}

fn wait_until_healthy(
    client: &reqwest::blocking::Client,
    server: &mut ServerProcess,
) -> Result<(), String> {
    let deadline = Instant::now() + HEALTH_CHECK_TIMEOUT;
    let url = server_root_url(server.port);

    loop {
        if server.has_exited() {
            return Err("whisper-server exited during startup.".to_string());
        }

        // Any HTTP response means the model is loaded and the server is
        // accepting connections (whisper-server loads the model before it
        // starts listening).
        match client.get(&url).timeout(Duration::from_secs(2)).send() {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() >= deadline => {
                return Err(format!(
                    "whisper-server did not become ready within {} seconds.",
                    HEALTH_CHECK_TIMEOUT.as_secs()
                ));
            }
            Err(_) => std::thread::sleep(HEALTH_POLL_INTERVAL),
        }
    }
}

fn find_free_port() -> Result<u16, String> {
    let listener = TcpListener::bind((SERVER_HOST, 0))
        .map_err(|error| format!("Could not allocate a local port. {}", error))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("Could not read allocated local port. {}", error))?
        .port();
    drop(listener);
    Ok(port)
}

fn server_threads() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(4)
        .saturating_sub(1)
        .clamp(1, 8)
}

fn server_args(
    model_path: &Path,
    language: &str,
    translate: bool,
    port: u16,
    threads: usize,
) -> Vec<String> {
    let mut args = vec![
        "--model".to_string(),
        model_path.to_string_lossy().to_string(),
        "--host".to_string(),
        SERVER_HOST.to_string(),
        "--port".to_string(),
        port.to_string(),
        "--threads".to_string(),
        threads.to_string(),
        "--language".to_string(),
        language.to_string(),
        // Suppress non-speech tokens (e.g. "(laughs)", "[Music]"); mirrors the
        // CLI path (whisper_args) so the warm-server and fallback agree.
        "--suppress-nst".to_string(),
    ];

    // Translate task (English output for any spoken language). Launch-time
    // argument, so a change to it restarts the server (see ServerConfig).
    if translate {
        args.push("--translate".to_string());
    }

    args
}

fn server_root_url(port: u16) -> String {
    format!("http://{}:{}/", SERVER_HOST, port)
}

fn inference_url(port: u16) -> String {
    format!("http://{}:{}/inference", SERVER_HOST, port)
}

fn needs_restart(current: &ServerConfig, desired: &ServerConfig) -> bool {
    current != desired
}

fn idle_expired(last_used: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_used) >= IDLE_SHUTDOWN
}

/// WS5: build the CPU-only retry for a request whose GPU attempt failed. Returns
/// `None` when the request was already CPU-only (nothing to retry differently),
/// else the same request forced to `--no-gpu` with no pinned device.
fn cpu_retry_request(request: &WhisperRequest) -> Option<WhisperRequest> {
    if request.gpu.is_off() {
        return None;
    }
    Some(WhisperRequest {
        gpu: crate::settings::GpuAcceleration::Off,
        gpu_device_index: None,
        ..request.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_a_usable_free_port() {
        let port = find_free_port().expect("port allocation should succeed");
        assert!(port > 0);
        // The listener was dropped, so the port must be bindable again.
        TcpListener::bind((SERVER_HOST, port)).expect("allocated port should be bindable");
    }

    #[test]
    fn parses_server_verbose_json_words() {
        // Mirrors the real whisper-server verbose_json: segments each with a
        // words[] of {word, start, end} (seconds, word text space-prefixed).
        let body = r#"{"text":" has a stack","segments":[
            {"text":" has a","words":[
                {"word":" has","start":3.47,"end":3.52,"probability":0.66},
                {"word":" a","start":3.52,"end":3.60}
            ]},
            {"text":" stack","words":[
                {"word":" stack","start":3.60,"end":3.90}
            ]}
        ]}"#;
        let words = parse_server_words(body);
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].text, "has"); // trimmed
        assert_eq!((words[0].start_ms, words[0].end_ms), (3470, 3520)); // s -> ms
        assert_eq!(words[2].text, "stack");
        assert_eq!((words[2].start_ms, words[2].end_ms), (3600, 3900));
    }

    #[test]
    fn parse_server_words_empty_without_segments() {
        assert!(parse_server_words(r#"{"text":"hi"}"#).is_empty());
        assert!(parse_server_words("garbage").is_empty());
    }

    #[test]
    fn builds_server_args_without_prompt() {
        let args = server_args(Path::new("models/ggml-base.en.bin"), "en", false, 8090, 4);

        assert_eq!(
            args,
            vec![
                "--model",
                "models/ggml-base.en.bin",
                "--host",
                "127.0.0.1",
                "--port",
                "8090",
                "--threads",
                "4",
                "--language",
                "en",
                "--suppress-nst",
            ]
        );
        // The vocabulary prompt is per-request, never a launch argument.
        assert!(!args.iter().any(|arg| arg == "--prompt"));
        // Translate is off here, so no --translate flag.
        assert!(!args.iter().any(|arg| arg == "--translate"));
    }

    #[test]
    fn server_args_include_translate_when_enabled() {
        let args = server_args(Path::new("models/ggml-small.bin"), "es", true, 8090, 4);

        let lang_index = args.iter().position(|arg| arg == "--language").unwrap();
        assert_eq!(args[lang_index + 1], "es");
        assert_eq!(args.iter().filter(|arg| *arg == "--translate").count(), 1);
    }

    #[test]
    fn builds_loopback_urls() {
        assert_eq!(inference_url(8090), "http://127.0.0.1:8090/inference");
        assert_eq!(server_root_url(8090), "http://127.0.0.1:8090/");
    }

    #[test]
    fn picks_a_sane_thread_count() {
        let threads = server_threads();
        assert!((1..=8).contains(&threads));
    }

    #[test]
    fn idle_expiry_triggers_only_after_the_idle_window() {
        let now = Instant::now();
        assert!(!idle_expired(now, now));
        assert!(!idle_expired(
            now,
            now + IDLE_SHUTDOWN - Duration::from_secs(1)
        ));
        assert!(idle_expired(now, now + IDLE_SHUTDOWN));
        assert!(idle_expired(
            now,
            now + IDLE_SHUTDOWN + Duration::from_secs(1)
        ));
    }

    #[test]
    fn restart_needed_only_when_config_changes() {
        use crate::settings::GpuAcceleration;
        let current = ServerConfig {
            model_path: PathBuf::from("models/ggml-base.en.bin"),
            language: "en".to_string(),
            translate: false,
            gpu: GpuAcceleration::Auto,
            gpu_device_index: None,
        };

        assert!(!needs_restart(&current, &current.clone()));
        assert!(needs_restart(
            &current,
            &ServerConfig {
                model_path: PathBuf::from("models/ggml-small.en-q5_1.bin"),
                ..current.clone()
            }
        ));
        assert!(needs_restart(
            &current,
            &ServerConfig {
                language: "auto".to_string(),
                ..current.clone()
            }
        ));
        // Toggling translate also requires a restart (it is a launch arg).
        assert!(needs_restart(
            &current,
            &ServerConfig {
                translate: true,
                ..current.clone()
            }
        ));
        // GPU on/off is a launch arg (--no-gpu), so toggling it restarts.
        assert!(needs_restart(
            &current,
            &ServerConfig {
                gpu: GpuAcceleration::Off,
                ..current.clone()
            }
        ));
        // Pinning a different Vulkan device (env at spawn) also restarts.
        assert!(needs_restart(
            &current,
            &ServerConfig {
                gpu_device_index: Some(1),
                ..current.clone()
            }
        ));
    }

    #[test]
    fn failure_tracker_disables_after_three_consecutive_failures() {
        let mut tracker = FailureTracker::default();
        assert!(tracker.server_allowed());

        assert!(!tracker.record_failure());
        assert!(!tracker.record_failure());
        assert!(tracker.server_allowed());

        // Third consecutive failure disables the server path and reports the
        // transition exactly once.
        assert!(tracker.record_failure());
        assert!(!tracker.server_allowed());
        assert!(!tracker.record_failure());
    }

    #[test]
    fn failure_tracker_resets_on_success_and_model_change() {
        let mut tracker = FailureTracker::default();
        tracker.record_failure();
        tracker.record_failure();
        tracker.record_success();
        tracker.record_failure();
        tracker.record_failure();
        assert!(tracker.server_allowed());

        tracker.record_failure();
        assert!(!tracker.server_allowed());

        // A model change resets the policy entirely.
        tracker.reset();
        assert!(tracker.server_allowed());
        assert_eq!(tracker.consecutive_failures, 0);
    }

    #[test]
    fn cpu_retry_only_when_gpu_was_on() {
        use crate::settings::GpuAcceleration;
        let base = WhisperRequest {
            model_path: PathBuf::from("m.bin"),
            wav_path: PathBuf::from("a.wav"),
            language: "en".to_string(),
            translate: false,
            vocabulary_prompt: String::new(),
            filler: None,
            gpu: GpuAcceleration::Auto,
            gpu_device_index: Some(1),
        };

        // GPU was on => retry on CPU, dropping the device pin, other fields kept.
        let retry = cpu_retry_request(&base).expect("auto attempt should retry on CPU");
        assert!(retry.gpu.is_off());
        assert_eq!(retry.gpu_device_index, None);
        assert_eq!(retry.model_path, base.model_path);
        assert_eq!(retry.language, base.language);

        // Already CPU => nothing different to retry.
        let cpu = WhisperRequest {
            gpu: GpuAcceleration::Off,
            ..base
        };
        assert!(cpu_retry_request(&cpu).is_none());
    }
}
