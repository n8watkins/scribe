//! The resident speech engine: keeps the selected model loaded in its own
//! process across dictations, so it is loaded once instead of on every
//! transcription.
//!
//! Design notes:
//! - `WarmTranscriber::transcribe` is a stateless, serialized primitive:
//!   request in (WAV + model + language + prompt), text out. Nothing assumes
//!   "one transcription per recording", so incremental/segment transcription
//!   can reuse it as-is.
//! - At most one engine process holds a model at any time ([`Slot`]): loading
//!   another model, engine or launch setting stops the current process and
//!   waits for it before the new one starts. One-off model runs (whisper-cli
//!   for files and the GPU probe) go through [`WarmTranscriber::run_exclusive`],
//!   which unloads the resident model first.
//! - "Keep the speech model loaded" (on by default, per engine): the model is
//!   loaded at startup and stays loaded. Off: it is unloaded a minute after
//!   the last dictation and loaded again when the next recording starts, so
//!   the load overlaps with speaking.
//! - The vocabulary prompt is sent per request, so prompt changes never
//!   require a restart.
//! - The server is started through `child_guard`, so it dies with Scribe even
//!   when Scribe crashes or is killed, and a survivor is stopped at the next
//!   start.
//! - Any server-path failure falls back to the `whisper-cli.exe` path for
//!   that request. After 3 consecutive server failures the server path is
//!   disabled for the rest of the session; a model/language change re-enables
//!   it.

use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, TryLockError,
    },
    time::{Duration, Instant},
};

use serde::Deserialize;
use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppStatus,
    commands::BackendState,
    error::CommandError,
    events::CoreEvent,
    host::{no_main_window, Host},
    models::{self, EngineKind},
    parakeet::ParakeetProcess,
    settings::AppSettings,
    speech_engine::{EngineConfig, EngineStatus, Ensured, LoadState, Measurements, Resident, Slot},
    whisper::{self, WhisperRequest, WhisperTranscription},
};

/// Tauri event carrying the new [`EngineStatus`] whenever it changes.
pub const ENGINE_STATUS_EVENT: &str = "scribe:engine-status";

const SERVER_HOST: &str = "127.0.0.1";
const SERVER_EXECUTABLE: &str = "whisper-server.exe";
/// With "Keep the speech model loaded" off, unload this long after the last
/// dictation. Long enough that a quick follow-up dictation stays instant,
/// short enough that the memory comes back soon after you stop.
const UNLOAD_AFTER_IDLE: Duration = Duration::from_secs(60);
/// How often the watchdog checks idleness and samples memory.
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
/// Memory changes smaller than this don't re-publish the status.
const MEMORY_REPORT_STEP: u64 = 4 * 1024 * 1024;
/// Hard cap on waiting for the server to load the model and start listening.
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(60);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Generous per-request cap so long dictations on big models still finish.
const INFERENCE_TIMEOUT: Duration = Duration::from_secs(300);
/// Consecutive server failures before the server path is disabled.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// Whether `engine` can stay resident on this system. Whisper's warm server
/// runs on Windows; elsewhere each dictation runs whisper-cli.
pub fn resident_supported(engine: EngineKind) -> bool {
    match engine {
        EngineKind::Whisper => cfg!(windows),
        EngineKind::Parakeet => true,
    }
}

impl EngineConfig {
    fn for_request(request: &WhisperRequest) -> Self {
        // Parakeet detects the language itself and has no translate task, so
        // those settings never restart it.
        let parakeet = request.engine == EngineKind::Parakeet;
        Self {
            engine: request.engine,
            model_id: request.model_id.clone(),
            model_path: request.model_path.clone(),
            language: if parakeet {
                crate::settings::LANGUAGE_AUTO.to_string()
            } else {
                request.language.clone()
            },
            translate: request.translate && !parakeet,
            gpu: request.gpu,
            gpu_device_index: request.gpu_device_index,
        }
    }
}

/// The one resident engine process: whisper-server or the Parakeet sidecar.
enum EngineProcess {
    Whisper(ServerProcess),
    Parakeet(ParakeetProcess),
}

impl Resident for EngineProcess {
    fn pid(&self) -> u32 {
        match self {
            Self::Whisper(server) => server.pid(),
            Self::Parakeet(sidecar) => sidecar.pid(),
        }
    }

    fn config(&self) -> &EngineConfig {
        match self {
            Self::Whisper(server) => server.config(),
            Self::Parakeet(sidecar) => sidecar.config(),
        }
    }

    fn has_exited(&mut self) -> bool {
        match self {
            Self::Whisper(server) => server.has_exited(),
            Self::Parakeet(sidecar) => sidecar.has_exited(),
        }
    }

    fn stop(&mut self) {
        match self {
            Self::Whisper(server) => server.stop(),
            Self::Parakeet(sidecar) => sidecar.stop(),
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
    /// Random secret route prefix (`--request-path`) the server was started
    /// with. Every route, including `/load`, sits under it, so a web page
    /// that finds the loopback port cannot reach the server without it.
    request_path: String,
    config: EngineConfig,
}

impl ServerProcess {
    /// Kills the child process. Safe to call multiple times (kill on an
    /// already-exited child is a no-op error that we ignore).
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        crate::child_guard::untrack(self.child.id());
    }
}

impl Resident for ServerProcess {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn config(&self) -> &EngineConfig {
        &self.config
    }

    fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    fn stop(&mut self) {
        log::info!("Unloading {} (whisper-server)", self.config.model_id);
        self.kill();
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

struct Inner {
    slot: Slot<EngineProcess>,
    /// Configuration of the most recent server attempt; used to detect model
    /// changes even while the server path is disabled.
    last_config: Option<EngineConfig>,
    failures: FailureTracker,
    last_used: Option<Instant>,
    http: Option<reqwest::blocking::Client>,
    measurements: Measurements,
}

/// The selected model and switch, cached so the watchdog and the status
/// never need the database.
#[derive(Debug, Clone)]
struct Selection {
    engine: EngineKind,
    model_id: Option<String>,
    model_name: Option<String>,
    keep_loaded: bool,
}

impl Selection {
    fn from_settings(settings: &AppSettings) -> Self {
        let model = settings
            .selected_model_id
            .as_deref()
            .and_then(models::catalog_model);
        let engine = model
            .map(|model| model.engine)
            .unwrap_or(EngineKind::Whisper);
        Self {
            engine,
            model_id: model.map(|model| model.id.to_string()),
            model_name: model.map(|model| model.status_name()),
            keep_loaded: keep_loaded(settings, engine),
        }
    }
}

/// The "Keep the speech model loaded" switch for `engine`.
pub fn keep_loaded(settings: &AppSettings, engine: EngineKind) -> bool {
    match engine {
        EngineKind::Whisper => settings.keep_whisper_loaded,
        EngineKind::Parakeet => settings.keep_parakeet_loaded,
    }
}

/// What the status shows besides the selection; written only by whoever
/// holds the slot, read without it.
#[derive(Default)]
struct Live {
    selection: Option<Selection>,
    /// The model id being loaded right now.
    loading: Option<String>,
    /// The loaded model's id and process id.
    loaded: Option<(String, u32)>,
    memory_bytes: Option<u64>,
    published: Option<EngineStatus>,
}

struct Shared {
    /// Where the status goes (the event and the tray line) and where the
    /// models and the engine binaries are. None only when the transcriber was
    /// built before the shell installed its host (never in the app).
    host: Option<Arc<dyn Host>>,
    /// The settings, the selected model and the dictation state. None only
    /// when the transcriber was built before the backend existed.
    backend: Option<BackendState>,
    /// Takes each new status to the host on the "speech-engine-status"
    /// thread, in order. The engine publishes while it holds its own locks,
    /// and must never wait on its host there: the tray waits for the shell's
    /// main thread, and at exit that thread waits for the engine lock in
    /// [`WarmTranscriber::shutdown`]. None while that thread is not running:
    /// the next status starts it again, and a status that finds no thread is
    /// dropped (and logged) rather than sent from here.
    status_out: Mutex<Option<crossbeam_channel::Sender<EngineStatus>>>,
    live: Mutex<Live>,
    prewarm_pending: AtomicBool,
    /// Set by [`WarmTranscriber::shutdown`]: no load starts from then on.
    shutting_down: AtomicBool,
    /// Every engine process started and maybe still running, so quitting
    /// can stop them without the engine lock (a load or a transcription can
    /// hold it for a while). Checked by identity before stopping, so a
    /// reused pid is never touched.
    processes: Mutex<Vec<crate::process_info::ProcessIdentity>>,
    /// Whether a background load first runs the GPU probe
    /// ([`PROBE_GPU_BEFORE_LOAD`]; tests turn it on everywhere).
    probe_gpu_before_load: bool,
}

/// Only Windows runs the GPU probe (the bundled whisper-cli loads a model to
/// list the Vulkan devices), so only there does a load probe first.
const PROBE_GPU_BEFORE_LOAD: bool = cfg!(windows);

/// What a transcription or a load answers once Scribe is quitting.
fn quitting_error() -> CommandError {
    CommandError::new("speech_engine_stopped", "Scribe is quitting.")
}

/// Starts the thread that tells `host` each new engine status: the event
/// (`scribe:engine-status`), then the tray line. It ends when the last
/// sender is gone.
fn spawn_status_publisher(host: Arc<dyn Host>) -> Option<crossbeam_channel::Sender<EngineStatus>> {
    let (sender, receiver) = crossbeam_channel::unbounded::<EngineStatus>();
    let spawned = spawn_named("speech-engine-status", move || {
        for status in receiver {
            tell_host(host.as_ref(), status);
        }
    });
    spawned.then_some(sender)
}

fn tell_host(host: &dyn Host, status: EngineStatus) {
    let summary = status.summary();
    host.emit(CoreEvent::EngineStatus(status));
    host.update_engine_summary(summary);
}

/// Tauri-managed warm transcription service. `Send + Sync`; the slot and its
/// bookkeeping live behind one mutex, which also serializes transcriptions
/// (an engine runs one inference at a time anyway).
///
/// A cheap handle: clones share the same engine (Tauri state and
/// `core::Core` hold the same one). Stop the engine with [`Self::shutdown`]
/// (`Core::shutdown` does, at exit): the handles are never all dropped while
/// the app runs or as it exits, so nothing may rely on drop to stop it.
#[derive(Clone)]
pub struct WarmTranscriber {
    inner: Arc<Mutex<Inner>>,
    shared: Arc<Shared>,
    _stop_on_last_drop: Arc<StopOnDrop>,
}

/// Stops the speech engine if the last [`WarmTranscriber`] handle is ever
/// dropped (a test, or a shell that drops its `Core`). A safety net only: the
/// app stops the engine explicitly through `Core::shutdown`.
struct StopOnDrop(Arc<Mutex<Inner>>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        stop_engine(&self.0);
    }
}

fn stop_engine(inner: &Mutex<Inner>) {
    let mut inner = lock(inner);
    if inner.slot.unload() {
        log::info!("Stopped the speech engine");
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl WarmTranscriber {
    /// The shell's setup builds the transcriber before the `Core` exists, so
    /// this finds the installed host and the managed backend directly (both
    /// come first at startup).
    pub fn new(app: AppHandle, measurements_path: PathBuf) -> Self {
        let host = crate::shell::host().cloned();
        let backend = app
            .try_state::<BackendState>()
            .map(|state| state.inner().clone());
        if host.is_none() || backend.is_none() {
            log::warn!("The speech engine was set up before the shell's host or backend");
        }
        debug_assert!(
            host.is_some() && backend.is_some(),
            "WarmTranscriber::new needs the shell's host installed and BackendState managed first \
             (host: {}, backend: {})",
            host.is_some(),
            backend.is_some()
        );
        Self::build(host, backend, measurements_path, PROBE_GPU_BEFORE_LOAD)
    }

    /// The speech engine for `host`, reading the settings and the dictation
    /// state from `backend`. Starts the watchdog; loads nothing until
    /// [`Self::sync_with_settings`] or a transcription asks for a model.
    pub fn new_with(
        host: Arc<dyn Host>,
        backend: BackendState,
        measurements_path: PathBuf,
    ) -> Self {
        Self::build(
            Some(host),
            Some(backend),
            measurements_path,
            PROBE_GPU_BEFORE_LOAD,
        )
    }

    fn build(
        host: Option<Arc<dyn Host>>,
        backend: Option<BackendState>,
        measurements_path: PathBuf,
        probe_gpu_before_load: bool,
    ) -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            slot: Slot::default(),
            last_config: None,
            failures: FailureTracker::default(),
            last_used: None,
            http: None,
            measurements: Measurements::load(measurements_path),
        }));
        let transcriber = Self {
            _stop_on_last_drop: Arc::new(StopOnDrop(inner.clone())),
            inner,
            shared: Arc::new(Shared {
                status_out: Mutex::new(host.clone().and_then(spawn_status_publisher)),
                host,
                backend,
                live: Mutex::new(Live::default()),
                prewarm_pending: AtomicBool::new(false),
                shutting_down: AtomicBool::new(false),
                processes: Mutex::new(Vec::new()),
                probe_gpu_before_load,
            }),
        };
        transcriber.start_watchdog();
        transcriber
    }

    /// The current status, for the `get_engine_status` command.
    pub fn status(&self) -> EngineStatus {
        let mut live = lock(&self.shared.live);
        if live.selection.is_none() {
            if let Some(settings) = current_settings(&self.shared) {
                live.selection = Some(Selection::from_settings(&settings));
            }
        }
        let measurements = self.inner.try_lock().ok().map(|inner| {
            live.selection
                .as_ref()
                .and_then(|selection| selection.model_id.as_deref())
                .and_then(|id| inner.measurements.get(id))
        });
        let mut status = build_status(&live, measurements.flatten());
        if measurements.is_none() {
            // The slot is busy (a transcription or a load); keep the last
            // published measurement rather than wait.
            status.measured = live.published.as_ref().and_then(|p| p.measured);
        }
        status
    }

    /// Call at startup and after any settings change: loads the model when
    /// "Keep the speech model loaded" is on, and unloads a model that no
    /// longer matches the settings (or the switch turned off) right away.
    pub fn sync_with_settings(&self, settings: &AppSettings) {
        let selection = Selection::from_settings(settings);
        let previous = lock(&self.shared.live).selection.replace(selection.clone());
        self.publish(None);
        // The switch for this engine went from on to off just now.
        let switched_off = !selection.keep_loaded
            && previous.is_some_and(|p| p.keep_loaded && p.engine == selection.engine);

        let desired = desired_config(&self.shared, settings);
        let keep = selection.keep_loaded && resident_supported(selection.engine);
        if keep {
            let loaded_config = self
                .inner
                .try_lock()
                .ok()
                .and_then(|inner| inner.slot.resident().map(|r| r.config().clone()));
            if desired.is_some() && loaded_config != desired {
                if dictation_in_progress(&self.shared) {
                    // A dictation keeps the model it started with (see
                    // session_model); the new one loads once it is done.
                    log::info!("Model changed during a dictation; it loads after this one.");
                } else {
                    self.prewarm();
                }
            }
            return;
        }

        // Off, or a model that can't stay resident. Decided under the slot
        // lock, after any running transcription.
        let shared = self.shared.clone();
        let inner = self.inner.clone();
        let resident_ok = resident_supported(selection.engine);
        spawn_named("speech-engine-unload", move || {
            let mut inner = lock(&inner);
            let loaded = inner.slot.resident().map(|r| r.config().clone());
            if unload_on_settings_change(
                resident_ok,
                switched_off,
                loaded.as_ref(),
                desired.as_ref(),
            ) && inner.slot.unload()
            {
                shared.after_unload();
                shared.publish(None);
            }
        });
    }

    /// After a dictation: loads the selected model in the background when
    /// "Keep the speech model loaded" is on, so a model chosen during the
    /// dictation is ready for the next one. A no-op when it is loaded.
    pub fn load_if_kept(&self) {
        let keep = lock(&self.shared.live)
            .selection
            .as_ref()
            .is_some_and(|selection| selection.keep_loaded && resident_supported(selection.engine));
        if keep {
            self.prewarm();
        }
    }

    /// Loads the selected model in the background unless it is already
    /// loaded. Called when a recording starts, so with the switch off the
    /// load overlaps with speaking. Repeated calls while a load is pending
    /// collapse into one.
    pub fn prewarm(&self) {
        if self.shared.quitting() || self.shared.prewarm_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let shared = self.shared.clone();
        let inner = self.inner.clone();
        // The probe below runs through this same engine.
        let warm = self.clone();
        let spawned = spawn_named("speech-engine-load", move || {
            shared.prewarm_pending.store(false, Ordering::SeqCst);
            if shared.quitting() {
                return;
            }
            // The GPU probe loads a model of its own, so it runs (once per
            // session, then cached) before this load rather than later next
            // to the loaded model, when the Models page asks for it.
            if shared.probe_gpu_before_load {
                if let Some(host) = shared.host.as_deref() {
                    let _ = crate::gpu::probe_using(
                        host,
                        crate::gpu::ProbeAsker::BackgroundLoad,
                        |probe| {
                            warm.run_unloaded(probe)
                                .unwrap_or_else(crate::gpu::not_probed)
                        },
                    );
                }
            }
            drop(warm);
            if shared.quitting() {
                return;
            }
            let Some(settings) = current_settings(&shared) else {
                return;
            };
            let Some(config) = desired_config(&shared, &settings) else {
                return;
            };
            if !resident_supported(config.engine) {
                return;
            }
            let mut inner = lock(&inner);
            if !inner.failures.server_allowed() && inner.last_config.as_ref() == Some(&config) {
                return;
            }
            let Ok(client) = http_client(&mut inner) else {
                return;
            };
            if let Err(error) = load(&shared, &mut inner, &config, &client) {
                log::warn!(
                    "Could not load {} ahead of dictation. {}",
                    config.model_id,
                    error
                );
            }
            // Count a preload as use, so the idle timer starts from here.
            inner.last_used = Some(Instant::now());
        });
        if !spawned {
            self.shared.prewarm_pending.store(false, Ordering::SeqCst);
        }
    }

    /// Runs `work` (a one-off model run such as whisper-cli on a file, or the
    /// GPU probe) with no resident model loaded, so two models are never in
    /// memory together. The resident model is unloaded first and, when the
    /// switch is on, loaded again afterwards.
    pub fn run_exclusive<T>(&self, work: impl FnOnce() -> T) -> T {
        let result = {
            let mut inner = lock(&self.inner);
            self.unload_locked(&mut inner);
            work()
        };
        if lock(&self.shared.live)
            .selection
            .as_ref()
            .is_some_and(|selection| selection.keep_loaded)
        {
            self.prewarm();
        }
        result
    }

    /// Runs `work` with no resident model loaded (it is unloaded first), and
    /// never loads one again afterwards. A background load uses this for its
    /// GPU probe: it is about to load the model itself, and asking for
    /// another background load from there would repeat without end. None,
    /// without running `work`, once Scribe is quitting (the probe loads a
    /// model of its own).
    fn run_unloaded<T>(&self, work: impl FnOnce() -> T) -> Option<T> {
        let mut inner = lock(&self.inner);
        if self.shared.quitting() {
            return None;
        }
        self.unload_locked(&mut inner);
        Some(work())
    }

    fn unload_locked(&self, inner: &mut Inner) {
        if inner.slot.unload() {
            self.shared.after_unload();
            self.publish(None);
        }
    }

    /// Transcribes one WAV. Tries the resident engine first and transparently
    /// falls back to the `whisper-cli.exe` path on any server failure.
    pub fn transcribe_with(
        &self,
        request: WhisperRequest,
    ) -> Result<WhisperTranscription, CommandError> {
        // FILLER: the warm server serves filler requests too — it returns
        // verbose_json with per-word timings (parsed in transcribe_via_server),
        // so suppression runs on the warm path with no model-reload overhead. The
        // whisper-cli path stays the fallback (and also handles filler).

        if request.engine == EngineKind::Parakeet {
            return self.transcribe_parakeet(&request);
        }

        // Missing inputs are user-facing errors, not server failures; let the
        // CLI path produce its existing error messages without touching the
        // failure counter.
        if request.model_path.is_file() && request.wav_path.is_file() {
            if let Some(transcription) = self.try_server(&request) {
                return Ok(transcription);
            }
        }

        // whisper-cli fallback, with no resident model loaded next to it. It
        // uses the GPU per the request; if that fails while the GPU was in use
        // (a driver crash or VRAM OOM on a big model), retry once on CPU
        // (--no-gpu) so a GPU problem degrades to slower-but-working instead of
        // losing the dictation. WS5.
        // After quitting began (it may have just stopped the server under a
        // request), no whisper-cli run starts: it would load the model again.
        let mut inner = lock(&self.inner);
        if self.shared.quitting() {
            return Err(quitting_error());
        }
        self.unload_locked(&mut inner);
        let result = match self.shared.host.as_deref() {
            None => Err(no_main_window()),
            Some(host) => match whisper::transcribe_with(host, request.clone()) {
                Ok(transcription) => Ok(transcription),
                Err(_) if self.shared.quitting() => Err(quitting_error()),
                Err(error) => match cpu_retry_request(&request) {
                    Some(cpu_request) => {
                        log::warn!(
                            "GPU transcription failed ({}); retrying once on CPU (--no-gpu)",
                            error.message
                        );
                        whisper::transcribe_with(host, cpu_request)
                    }
                    None => Err(error),
                },
            },
        };
        inner.last_used = Some(Instant::now());
        result
    }

    /// Parakeet always runs in its resident sidecar (there is no one-shot
    /// fallback binary). A failed attempt restarts the sidecar and tries once
    /// more, on the CPU when the GPU was in use.
    fn transcribe_parakeet(
        &self,
        request: &WhisperRequest,
    ) -> Result<WhisperTranscription, CommandError> {
        if !request.model_path.is_file() {
            return Err(CommandError::new(
                "whisper_model_missing",
                format!(
                    "The selected speech model is missing at {}. Re-download the model or choose another model.",
                    request.model_path.display()
                ),
            ));
        }
        if !request.wav_path.is_file() {
            return Err(CommandError::new(
                "recording_wav_missing",
                format!(
                    "Recording WAV is missing at {}. Record again.",
                    request.wav_path.display()
                ),
            ));
        }

        let first = EngineConfig::for_request(request);
        let retry = match cpu_retry_request(request) {
            Some(cpu) => EngineConfig::for_request(&cpu),
            None => first.clone(),
        };
        let mut inner = lock(&self.inner);
        let started = Instant::now();
        let mut last_error = String::new();
        for (attempt, config) in [first, retry].iter().enumerate() {
            match self.transcribe_via_parakeet(&mut inner, config, request) {
                Ok(text) => {
                    inner.last_used = Some(Instant::now());
                    let memory = inner.slot.resident().and_then(|resident| {
                        crate::process_info::resident_memory_bytes(resident.pid())
                    });
                    if let Some(bytes) = memory {
                        inner.measurements.record_memory(&config.model_id, bytes);
                    }
                    drop(inner);
                    self.publish(memory);
                    return Ok(WhisperTranscription {
                        text,
                        latency_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
                    });
                }
                Err(error) => {
                    log::warn!("Parakeet attempt {} failed. {}", attempt + 1, error);
                    if inner.slot.unload() {
                        self.shared.after_unload();
                    }
                    last_error = error;
                }
            }
        }
        inner.last_used = Some(Instant::now());
        drop(inner);
        self.publish(None);
        Err(CommandError::new(
            "whisper_transcription_failed",
            format!("Parakeet transcription failed. {}", last_error),
        ))
    }

    fn transcribe_via_parakeet(
        &self,
        inner: &mut Inner,
        config: &EngineConfig,
        request: &WhisperRequest,
    ) -> Result<String, String> {
        let client = http_client(inner)?;
        load(&self.shared, inner, config, &client)?;
        let Some(EngineProcess::Parakeet(sidecar)) = inner.slot.resident_mut() else {
            return Err("scribe-parakeet is not running".to_string());
        };
        let result = sidecar.transcribe(&request.wav_path)?;
        // FILLER: Parakeet gives per-word timings too, so pause-aware filler
        // removal works the same as on Whisper.
        if let Some(filler) = &request.filler {
            if !result.words.is_empty() {
                return Ok(filler.apply(&result.words));
            }
        }
        Ok(whisper::normalize_transcript_text(&result.text))
    }

    /// Stops the speech engine for good. Called on app exit; double-stop is
    /// safe. No load starts afterwards, and every engine process is stopped
    /// at once, without waiting for the engine lock (a load or a
    /// transcription may hold it for a while; it finds its process gone).
    /// The slot is then tidied when the lock is free.
    pub fn shutdown(&self) {
        self.shared.shutting_down.store(true, Ordering::SeqCst);
        let processes = std::mem::take(&mut *lock(&self.shared.processes));
        for process in &processes {
            if crate::process_info::terminate_if_same(process) {
                log::info!(
                    "Stopped the speech engine process {} ({})",
                    process.pid,
                    process.exe.display()
                );
            }
        }
        match self.inner.try_lock() {
            Ok(mut inner) => {
                if inner.slot.unload() {
                    log::info!("Stopped the speech engine");
                }
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().slot.unload();
            }
            Err(TryLockError::WouldBlock) => {
                log::info!("The speech engine is busy; its processes were stopped directly");
            }
        }
    }

    fn publish(&self, memory: Option<u64>) {
        self.shared.publish(memory);
    }

    /// Attempts the resident path. Returns None when it is unavailable or
    /// failed (the caller falls back to whisper-cli).
    fn try_server(&self, request: &WhisperRequest) -> Option<WhisperTranscription> {
        if !resident_supported(request.engine) {
            return None;
        }

        let desired = EngineConfig::for_request(request);
        let mut inner = lock(&self.inner);

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
        match self.transcribe_via_server(&mut inner, &desired, request) {
            Ok(text) => {
                let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
                inner.failures.record_success();
                inner.last_used = Some(Instant::now());
                // The first inference allocates working buffers: sample now so
                // the status and the measurement include them.
                let memory = inner.slot.resident().and_then(|resident| {
                    crate::process_info::resident_memory_bytes(resident.pid())
                });
                if let Some(bytes) = memory {
                    inner.measurements.record_memory(&desired.model_id, bytes);
                }
                drop(inner);
                self.publish(memory);
                Some(WhisperTranscription { text, latency_ms })
            }
            Err(error) => {
                log::warn!(
                    "whisper-server transcription failed; falling back to whisper-cli. {}",
                    error
                );
                // The server state is unknown after a failure; restart fresh
                // on the next attempt.
                if inner.slot.unload() {
                    self.shared.after_unload();
                }
                if inner.failures.record_failure() {
                    log::warn!(
                        "Disabling whisper-server for the rest of the session after {} consecutive failures",
                        MAX_CONSECUTIVE_FAILURES
                    );
                }
                drop(inner);
                self.publish(None);
                None
            }
        }
    }

    fn transcribe_via_server(
        &self,
        inner: &mut Inner,
        config: &EngineConfig,
        request: &WhisperRequest,
    ) -> Result<String, String> {
        let client = http_client(inner)?;
        load(&self.shared, inner, config, &client)?;

        let url = inner
            .slot
            .resident()
            .and_then(|resident| match resident {
                EngineProcess::Whisper(server) => {
                    Some(inference_url(server.port, &server.request_path))
                }
                EngineProcess::Parakeet(_) => None,
            })
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

        // Trimmed to what whisper.cpp reads, keeping the front, and carried
        // into every 30 s window of a long recording (as in whisper_args).
        let vocabulary_prompt = crate::prompt_budget::fit_front(
            &request.vocabulary_prompt,
            crate::prompt_budget::PROMPT_TOKEN_LIMIT,
        );
        if !vocabulary_prompt.is_empty() {
            form = form
                .text("prompt", vocabulary_prompt.to_string())
                .text("carry_initial_prompt", "true");
        }

        let response = client
            .post(url)
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

    /// Checks idleness, crashes and memory every few seconds. Holds only weak
    /// references so it never keeps the transcriber alive.
    fn start_watchdog(&self) {
        let weak_inner = Arc::downgrade(&self.inner);
        let weak_shared = Arc::downgrade(&self.shared);
        spawn_named("speech-engine-watchdog", move || loop {
            std::thread::sleep(WATCHDOG_INTERVAL);
            let (Some(inner), Some(shared)) = (weak_inner.upgrade(), weak_shared.upgrade()) else {
                return;
            };
            // Busy (a transcription or a load): look again next time.
            let mut inner = match inner.try_lock() {
                Ok(guard) => guard,
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                Err(TryLockError::WouldBlock) => continue,
            };
            watchdog_tick(&shared, &mut inner);
        });
    }
}

/// With "Keep the speech model loaded" off (or an engine that can't stay
/// resident), whether a settings change should unload the model right away.
/// Any other save (a window size, a hotkey) leaves it to the idle timer, so
/// the minute after a dictation stays fast.
fn unload_on_settings_change(
    resident_ok: bool,
    switched_off: bool,
    loaded: Option<&EngineConfig>,
    desired: Option<&EngineConfig>,
) -> bool {
    let Some(loaded) = loaded else {
        return false;
    };
    !resident_ok || switched_off || desired != Some(loaded)
}

fn watchdog_tick(shared: &Shared, inner: &mut Inner) {
    let Some(resident) = inner.slot.resident_mut() else {
        return;
    };
    if resident.has_exited() {
        log::warn!("The speech engine exited on its own");
        inner.slot.unload();
        shared.after_unload();
        shared.publish(None);
        return;
    }
    let pid = resident.pid();
    let model_id = resident.config().model_id.clone();
    let keep = lock(&shared.live)
        .selection
        .as_ref()
        .is_none_or(|selection| selection.keep_loaded);
    let idle = inner
        .last_used
        .is_none_or(|used| used.elapsed() >= UNLOAD_AFTER_IDLE);
    if !keep && idle && !dictation_in_progress(shared) {
        log::info!(
            "Unloading {} after {} s without dictation (Keep the speech model loaded is off)",
            model_id,
            UNLOAD_AFTER_IDLE.as_secs()
        );
        inner.slot.unload();
        shared.after_unload();
        shared.publish(None);
        return;
    }
    if let Some(bytes) = crate::process_info::resident_memory_bytes(pid) {
        inner.measurements.record_memory(&model_id, bytes);
        shared.publish_if_memory_moved(bytes, inner.measurements.get(&model_id));
    }
}

impl Shared {
    fn after_unload(&self) {
        let mut live = lock(&self.live);
        live.loaded = None;
        live.memory_bytes = None;
    }

    /// Recomputes the status and, when it changed, emits it and updates the
    /// tray. `memory` replaces the cached memory figure when given.
    fn publish(&self, memory: Option<u64>) {
        let mut live = lock(&self.live);
        if memory.is_some() && live.loaded.is_some() {
            live.memory_bytes = memory;
        }
        let measured = live.published.as_ref().and_then(|p| p.measured);
        self.publish_locked(&mut live, measured);
    }

    fn publish_if_memory_moved(
        &self,
        bytes: u64,
        measured: Option<crate::speech_engine::Measurement>,
    ) {
        let mut live = lock(&self.live);
        let moved = live
            .memory_bytes
            .is_none_or(|known| known.abs_diff(bytes) >= MEMORY_REPORT_STEP);
        if moved {
            live.memory_bytes = Some(bytes);
        }
        let measured_changed = live.published.as_ref().and_then(|p| p.measured) != measured;
        if moved || measured_changed {
            self.publish_locked(&mut live, measured);
        }
    }

    fn publish_with_measurement(&self, measured: Option<crate::speech_engine::Measurement>) {
        let mut live = lock(&self.live);
        self.publish_locked(&mut live, measured);
    }

    fn publish_locked(&self, live: &mut Live, measured: Option<crate::speech_engine::Measurement>) {
        if live.selection.is_none() {
            if let Some(settings) = current_settings(self) {
                live.selection = Some(Selection::from_settings(&settings));
            }
        }
        let status = build_status(live, measured);
        if live.published.as_ref() == Some(&status) {
            return;
        }
        live.published = Some(status.clone());
        // `scribe:engine-status` (ENGINE_STATUS_EVENT), then the tray, from
        // the status thread: the caller may hold the engine lock.
        self.send_status(status);
    }

    /// Hands `status` to the status thread, starting it again when it is not
    /// running (it could not start, or a host call panicked on it). Never
    /// calls the host itself: the caller may hold the engine lock.
    fn send_status(&self, status: EngineStatus) {
        let Some(host) = &self.host else {
            return;
        };
        let mut status_out = lock(&self.status_out);
        let mut status = status;
        // A second try covers a thread that stopped since the last status.
        for _ in 0..2 {
            if status_out.is_none() {
                *status_out = spawn_status_publisher(host.clone());
            }
            let Some(sender) = status_out.as_ref() else {
                break;
            };
            match sender.send(status) {
                Ok(()) => return,
                Err(crossbeam_channel::SendError(unsent)) => {
                    *status_out = None;
                    status = unsent;
                }
            }
        }
        log::warn!("The speech engine status thread is not running; a status was not shown");
    }

    /// Notes an engine process the moment it starts, so quitting can stop
    /// it. Returns false when Scribe is quitting: the caller stops it at once
    /// (either quitting finds it in the list, or the caller sees the flag).
    fn started(&self, pid: u32) -> bool {
        if let Some(identity) = crate::process_info::identity(pid) {
            let mut processes = lock(&self.processes);
            processes.retain(crate::process_info::is_same_process_running);
            processes.push(identity);
        }
        !self.shutting_down.load(Ordering::SeqCst)
    }

    fn quitting(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }
}

fn build_status(live: &Live, measured: Option<crate::speech_engine::Measurement>) -> EngineStatus {
    let selection = live.selection.clone().unwrap_or(Selection {
        engine: EngineKind::Whisper,
        model_id: None,
        model_name: None,
        keep_loaded: true,
    });
    let selected = selection.model_id.as_deref();
    let loaded_here = live
        .loaded
        .as_ref()
        .is_some_and(|(id, _)| Some(id.as_str()) == selected);
    let state = if !resident_supported(selection.engine) {
        LoadState::PerDictation
    } else if loaded_here {
        LoadState::Loaded
    } else if live
        .loading
        .as_deref()
        .is_some_and(|id| Some(id) == selected)
    {
        LoadState::Loading
    } else {
        LoadState::NotLoaded
    };
    EngineStatus {
        engine: selection.engine,
        model_id: selection.model_id,
        model_name: selection.model_name,
        state,
        memory_bytes: if loaded_here { live.memory_bytes } else { None },
        keep_loaded: selection.keep_loaded,
        measured,
    }
}

/// Loads `config` into the slot if it isn't already there: stops any other
/// engine first, then starts and waits for the new one, recording how long
/// the cold load took.
fn load(
    shared: &Shared,
    inner: &mut Inner,
    config: &EngineConfig,
    client: &reqwest::blocking::Client,
) -> Result<(), String> {
    let matches = inner
        .slot
        .resident()
        .is_some_and(|resident| resident.config() == config);
    // Checked whether or not the slot's config matches: a resident process
    // that has died would be started again by `ensure`.
    if shared.quitting() {
        return Err(quitting_error().message);
    }
    if !matches {
        if inner.slot.unload() {
            shared.after_unload();
        }
        lock(&shared.live).loading = Some(config.model_id.clone());
        shared.publish(None);
    }
    let started = Instant::now();
    // Each engine process is noted the moment it starts, for quitting.
    let note = |pid: u32| shared.started(pid);
    let outcome = inner.slot.ensure(config, |config| {
        let host = shared
            .host
            .as_deref()
            .ok_or_else(|| no_main_window().message)?;
        match config.engine {
            EngineKind::Whisper => {
                launch_whisper_server(host, config, client, &note).map(EngineProcess::Whisper)
            }
            EngineKind::Parakeet => {
                ParakeetProcess::launch_with(host, config, &note).map(EngineProcess::Parakeet)
            }
        }
    });
    lock(&shared.live).loading = None;
    match outcome {
        Ok(Ensured::AlreadyLoaded) => Ok(()),
        Ok(Ensured::Loaded) => {
            let load_ms = started.elapsed().as_millis() as u64;
            let pid = inner.slot.resident().map(|r| r.pid()).unwrap_or_default();
            inner.measurements.record_load(&config.model_id, load_ms);
            let memory = crate::process_info::resident_memory_bytes(pid);
            if let Some(bytes) = memory {
                inner.measurements.record_memory(&config.model_id, bytes);
            }
            {
                let mut live = lock(&shared.live);
                live.loaded = Some((config.model_id.clone(), pid));
                live.memory_bytes = memory;
            }
            shared.publish_with_measurement(inner.measurements.get(&config.model_id));
            log::info!(
                "Loaded {} in {} ms ({} in memory)",
                config.model_id,
                load_ms,
                memory
                    .map(crate::speech_engine::format_bytes)
                    .unwrap_or_else(|| "unknown".into())
            );
            Ok(())
        }
        Err(error) => {
            shared.publish(None);
            Err(error)
        }
    }
}

/// The engine configuration the current settings ask for, or None when no
/// downloaded model is selected.
fn desired_config(shared: &Shared, settings: &AppSettings) -> Option<EngineConfig> {
    let host = shared.host.as_deref()?;
    let state = shared.backend.as_ref()?;
    let (model_id, model_path) = {
        let db = state.db().ok()?;
        crate::model_manager::selected_model_path_with(host, &db).ok()?
    };
    let request = WhisperRequest::for_settings(&model_id, model_path, PathBuf::new(), settings);
    Some(EngineConfig::for_request(&request))
}

fn current_settings(shared: &Shared) -> Option<AppSettings> {
    let state = shared.backend.as_ref()?;
    let db = state.db().ok()?;
    db.get_settings().ok()
}

/// A recording, transcription or paste is under way: not the moment to
/// unload the model.
fn dictation_in_progress(shared: &Shared) -> bool {
    let Some(state) = shared.backend.as_ref() else {
        return false;
    };
    let Ok(machine) = state.app_state() else {
        return false;
    };
    matches!(
        machine.snapshot().status,
        AppStatus::Recording | AppStatus::Stopping | AppStatus::Transcribing | AppStatus::Pasting
    )
}

/// Spawns a named background thread; returns false when the OS refused.
fn spawn_named(name: &str, work: impl FnOnce() + Send + 'static) -> bool {
    match std::thread::Builder::new()
        .name(name.to_string())
        .spawn(work)
    {
        Ok(_) => true,
        Err(error) => {
            log::warn!("Could not start the {} thread. {}", name, error);
            false
        }
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

/// Starts whisper-server for `config` and waits until the model is loaded.
/// The caller ([`Slot::ensure`]) has already stopped any other engine.
/// `started` hears the server's pid the moment it runs; when it answers false
/// (Scribe is quitting) the server is stopped at once.
fn launch_whisper_server(
    host: &dyn Host,
    config: &EngineConfig,
    client: &reqwest::blocking::Client,
    started: &dyn Fn(u32) -> bool,
) -> Result<ServerProcess, String> {
    let executable = whisper::resolve_bundled_executable_with(host, SERVER_EXECUTABLE)
        .map_err(|error| error.message)?;
    let port = find_free_port()?;
    let request_path = new_request_path();
    let threads = server_threads();
    let mut args = server_args(
        &config.model_path,
        &config.language,
        config.translate,
        port,
        &request_path,
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
    let child = spawn_server_process(
        &executable,
        &args,
        whisper::gpu_visible_devices_env(config.gpu, config.gpu_device_index),
    )?;
    let mut server = ServerProcess {
        child,
        port,
        request_path,
        config: config.clone(),
    };
    if !started(server.child.id()) {
        // Dropping `server` stops it.
        return Err("Scribe is quitting.".to_string());
    }

    // On failure, `server` is dropped here, which kills the child.
    wait_until_healthy(client, &mut server)?;
    Ok(server)
}

fn spawn_server_process(
    executable: &Path,
    args: &[String],
    vk_visible_devices: Option<String>,
) -> Result<Child, String> {
    let mut command = std::process::Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    whisper::suppress_console_window(&mut command);

    // Pin a specific Vulkan device for the resident server (multi-GPU boxes);
    // absent means ggml picks its default device.
    if let Some(devices) = vk_visible_devices {
        command.env("GGML_VK_VISIBLE_DEVICES", devices);
    }

    // Dies with Scribe however Scribe ends, and is noted so a later start
    // can stop it if it ever survives (src/child_guard.rs).
    let child = crate::child_guard::spawn(command)
        .map_err(|error| format!("Could not start whisper-server. {}", error))?;
    crate::child_guard::track(child.id());
    Ok(child)
}

fn wait_until_healthy(
    client: &reqwest::blocking::Client,
    server: &mut ServerProcess,
) -> Result<(), String> {
    let deadline = Instant::now() + HEALTH_CHECK_TIMEOUT;
    let url = health_url(server.port, &server.request_path);

    loop {
        if Resident::has_exited(server) {
            return Err("whisper-server exited during startup.".to_string());
        }

        // Only a success from the secret health route counts: it proves the
        // model is loaded and that the listener on this port is our server,
        // not some other process that grabbed the port first.
        match client.get(&url).timeout(Duration::from_secs(2)).send() {
            Ok(response) if response.status().is_success() => return Ok(()),
            _ if Instant::now() >= deadline => {
                return Err(format!(
                    "whisper-server did not become ready within {} seconds.",
                    HEALTH_CHECK_TIMEOUT.as_secs()
                ));
            }
            _ => std::thread::sleep(HEALTH_POLL_INTERVAL),
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
    request_path: &str,
    threads: usize,
) -> Vec<String> {
    let mut args = vec![
        "--model".to_string(),
        model_path.to_string_lossy().to_string(),
        "--host".to_string(),
        SERVER_HOST.to_string(),
        "--port".to_string(),
        port.to_string(),
        "--request-path".to_string(),
        request_path.to_string(),
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

/// A fresh secret route prefix for one server process: two random v4 UUIDs
/// (about 244 random bits from the OS generator), so the URL can't be guessed
/// by anything that finds the port.
fn new_request_path() -> String {
    format!(
        "/{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn health_url(port: u16, request_path: &str) -> String {
    format!("http://{}:{}{}/health", SERVER_HOST, port, request_path)
}

fn inference_url(port: u16, request_path: &str) -> String {
    format!("http://{}:{}{}/inference", SERVER_HOST, port, request_path)
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

    /// The engine status goes to the transcriber's host as
    /// `scribe:engine-status`, once per change. "Keep the speech model
    /// loaded" is off and no model file exists, so nothing is started.
    #[test]
    fn the_engine_status_goes_to_the_host() {
        use crate::host::{Flavor, RecordingHost};
        use crate::paths::Paths;

        let root = std::env::temp_dir().join(format!("scribe-warm-status-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let host = Arc::new(RecordingHost::new(
            Paths::from_parts(
                root.join("data"),
                root.join("cache"),
                root.join("logs"),
                None,
                None,
            ),
            Flavor::Stable,
        ));
        let db = crate::db::Database::in_memory().unwrap();
        let mut settings = db.get_settings().unwrap();
        settings.selected_model_id = Some("tiny.en".into());
        settings.keep_whisper_loaded = false;
        db.save_settings(&settings).unwrap();
        let backend = BackendState::new(
            db,
            root.join("cache").join("recordings"),
            crate::data_hygiene::DataDirs {
                data_dir: root.join("data"),
                cache_dir: root.join("cache"),
            },
        );

        let warm = WarmTranscriber::new_with(
            host.clone(),
            backend,
            root.join("data").join("engine-measurements.json"),
        );
        warm.sync_with_settings(&settings);
        // The same settings again change nothing, so nothing new is sent.
        warm.sync_with_settings(&settings);

        // The status thread tells the host; give it time to, and to send a
        // second event if it (wrongly) would.
        let started = Instant::now();
        while host.events().is_empty() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(200));
        let events = host.events();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].name(), ENGINE_STATUS_EVENT);
        let crate::events::CoreEvent::EngineStatus(status) = &events[0] else {
            panic!("not an engine status: {:?}", events[0]);
        };
        assert_eq!(status.model_id.as_deref(), Some("tiny.en"));
        assert!(!status.keep_loaded);
        assert_ne!(status.state, LoadState::Loaded);
        assert_eq!(&warm.status(), status);

        warm.shutdown();
        let _ = fs::remove_dir_all(&root);
    }

    /// A host whose tray, like Tauri's, does its work on the shell's main
    /// thread and waits for it. The test plays that main thread: it is busy
    /// (the tray call waits) until [`Self::free_main_thread`].
    struct MainThreadHost {
        paths: crate::paths::Paths,
        main_free: Mutex<bool>,
        main_freed: std::sync::Condvar,
        tray_calls: std::sync::atomic::AtomicUsize,
    }

    impl MainThreadHost {
        fn free_main_thread(&self) {
            *lock(&self.main_free) = true;
            self.main_freed.notify_all();
        }
    }

    impl Host for MainThreadHost {
        fn emit(&self, _event: CoreEvent) {}

        fn paths(&self) -> &crate::paths::Paths {
            &self.paths
        }

        fn flavor(&self) -> crate::host::Flavor {
            crate::host::Flavor::Stable
        }

        fn update_tray_status(&self, _status: AppStatus) {}

        fn update_engine_summary(&self, _summary: String) {
            self.tray_calls.fetch_add(1, Ordering::SeqCst);
            // Bounded, so a regression fails the test instead of hanging it.
            let free = lock(&self.main_free);
            let _ = self
                .main_freed
                .wait_timeout_while(free, Duration::from_secs(10), |free| !*free);
        }

        fn sync_settings(&self, _settings: &AppSettings) {}

        fn open_dashboard(&self, _route: Option<&str>) -> Result<(), CommandError> {
            Err(no_main_window())
        }

        fn toggle_dashboard(&self) -> Result<(), CommandError> {
            Err(no_main_window())
        }

        fn main_window_logical_size(&self) -> Option<(u32, u32)> {
            None
        }

        fn app_version(&self) -> String {
            crate::host::RECORDING_HOST_VERSION.to_string()
        }

        fn exit(&self, _code: i32) {}

        fn open_path(&self, _path: &std::path::Path) -> Result<(), String> {
            Ok(())
        }

        fn open_url(&self, _url: &str) -> Result<(), String> {
            Ok(())
        }

        fn notify(&self, _title: &str, _body: &str) -> Result<(), String> {
            Ok(())
        }

        fn register_shortcut(
            &self,
            _shortcut: global_hotkey::hotkey::HotKey,
        ) -> Result<(), String> {
            Ok(())
        }

        fn unregister_shortcut(
            &self,
            _shortcut: global_hotkey::hotkey::HotKey,
        ) -> Result<(), String> {
            Ok(())
        }

        fn is_shortcut_registered(&self, _shortcut: global_hotkey::hotkey::HotKey) -> bool {
            false
        }

        fn launch_at_login(&self) -> Result<bool, String> {
            Ok(false)
        }

        fn set_launch_at_login(&self, _enabled: bool) -> Result<(), String> {
            Ok(())
        }
    }

    /// The Windows end-to-end hang: the app quit (the main thread waiting in
    /// `shutdown` for the engine) while a background load held the engine and
    /// told the tray its new status (which waits for the main thread). The
    /// engine must never wait on its host while holding its own lock, so
    /// `shutdown` finishes while the main thread is still busy.
    #[test]
    fn shutdown_finishes_while_the_shells_main_thread_is_busy() {
        let root = std::env::temp_dir().join(format!("scribe-warm-exit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let host = Arc::new(MainThreadHost {
            paths: crate::paths::Paths::from_parts(
                root.join("data"),
                root.join("cache"),
                root.join("logs"),
                None,
                None,
            ),
            main_free: Mutex::new(false),
            main_freed: std::sync::Condvar::new(),
            tray_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        // A Parakeet model on disk, so a load starts (and announces itself);
        // with no resources folder the sidecar then fails to start, so no
        // process ever runs. "Keep loaded" is off, so nothing reloads.
        let parakeet = models::catalog_model(models::PARAKEET_V3_ID).unwrap();
        let model = root.join("data").join("models").join(parakeet.filename);
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(&model, b"not really a model").unwrap();
        let db = crate::db::Database::in_memory().unwrap();
        let mut settings = db.get_settings().unwrap();
        settings.selected_model_id = Some(models::PARAKEET_V3_ID.into());
        settings.keep_parakeet_loaded = false;
        db.save_settings(&settings).unwrap();
        let backend = BackendState::new(
            db,
            root.join("cache").join("recordings"),
            crate::data_hygiene::DataDirs {
                data_dir: root.join("data"),
                cache_dir: root.join("cache"),
            },
        );
        let warm = WarmTranscriber::new_with(
            host.clone(),
            backend,
            root.join("data").join("engine-measurements.json"),
        );

        warm.prewarm();
        let started = Instant::now();
        while host.tray_calls.load(Ordering::SeqCst) == 0 {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the load never announced itself"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        // The app quits now, on the busy main thread.
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let quitting = warm.clone();
        std::thread::spawn(move || {
            quitting.shutdown();
            let _ = done_tx.send(());
        });
        let finished = done_rx.recv_timeout(Duration::from_secs(5)).is_ok();
        host.free_main_thread();
        assert!(
            finished,
            "shutdown waited for the engine while the engine waited for the main thread"
        );

        let _ = fs::remove_dir_all(&root);
    }

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
        let args = server_args(
            Path::new("models/ggml-base.en.bin"),
            "en",
            false,
            8090,
            "/secret",
            4,
        );

        assert_eq!(
            args,
            vec![
                "--model",
                "models/ggml-base.en.bin",
                "--host",
                "127.0.0.1",
                "--port",
                "8090",
                "--request-path",
                "/secret",
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
        let args = server_args(
            Path::new("models/ggml-small.bin"),
            "es",
            true,
            8090,
            "/s",
            4,
        );

        let lang_index = args.iter().position(|arg| arg == "--language").unwrap();
        assert_eq!(args[lang_index + 1], "es");
        assert_eq!(args.iter().filter(|arg| *arg == "--translate").count(), 1);
    }

    #[test]
    fn builds_loopback_urls() {
        assert_eq!(
            inference_url(8090, "/abc"),
            "http://127.0.0.1:8090/abc/inference"
        );
        assert_eq!(health_url(8090, "/abc"), "http://127.0.0.1:8090/abc/health");
    }

    #[test]
    fn request_paths_are_long_random_and_unique() {
        let first = new_request_path();
        let second = new_request_path();
        assert!(first.starts_with('/'));
        // 64 hex characters after the slash.
        assert_eq!(first.len(), 65);
        assert!(first[1..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn picks_a_sane_thread_count() {
        let threads = server_threads();
        assert!((1..=8).contains(&threads));
    }

    #[test]
    fn with_keep_loaded_off_only_real_changes_unload_at_once() {
        let loaded = EngineConfig {
            engine: EngineKind::Whisper,
            model_id: "small".into(),
            model_path: PathBuf::from("m.bin"),
            language: "en".into(),
            translate: false,
            gpu: crate::settings::GpuAcceleration::Auto,
            gpu_device_index: None,
        };
        let other = EngineConfig {
            model_id: "base".into(),
            ..loaded.clone()
        };
        // An unrelated save (window size, hotkeys) right after a dictation:
        // the idle minute decides, not the save.
        assert!(!unload_on_settings_change(
            true,
            false,
            Some(&loaded),
            Some(&loaded)
        ));
        // The switch was just turned off: free the memory now.
        assert!(unload_on_settings_change(
            true,
            true,
            Some(&loaded),
            Some(&loaded)
        ));
        // Another model, language or GPU choice: the loaded one is stale.
        assert!(unload_on_settings_change(
            true,
            false,
            Some(&loaded),
            Some(&other)
        ));
        assert!(unload_on_settings_change(true, false, Some(&loaded), None));
        // An engine that can't stay resident here.
        assert!(unload_on_settings_change(
            false,
            false,
            Some(&loaded),
            Some(&loaded)
        ));
        // Nothing loaded: nothing to do.
        assert!(!unload_on_settings_change(true, true, None, Some(&loaded)));
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
            engine: EngineKind::Whisper,
            model_id: "small".to_string(),
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

    /// A speech engine with Parakeet selected and its model on disk, in a
    /// fresh folder under `root`. There is no resources folder, so starting
    /// the sidecar fails at once: a load is announced ("Loading") and ends
    /// without ever running a process. The GPU probe runs before each load,
    /// as it does on Windows; with no Whisper model it can never succeed.
    fn parakeet_engine(root: &Path, host: Arc<dyn Host>, keep_loaded: bool) -> WarmTranscriber {
        let _ = fs::remove_dir_all(root);
        let parakeet = models::catalog_model(models::PARAKEET_V3_ID).unwrap();
        let model = root.join("data").join("models").join(parakeet.filename);
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(&model, b"not really a model").unwrap();
        let db = crate::db::Database::in_memory().unwrap();
        let mut settings = db.get_settings().unwrap();
        settings.selected_model_id = Some(models::PARAKEET_V3_ID.into());
        settings.keep_parakeet_loaded = keep_loaded;
        db.save_settings(&settings).unwrap();
        let backend = BackendState::new(
            db,
            root.join("cache").join("recordings"),
            crate::data_hygiene::DataDirs {
                data_dir: root.join("data"),
                cache_dir: root.join("cache"),
            },
        );
        WarmTranscriber::build(
            Some(host),
            Some(backend),
            root.join("data").join("engine-measurements.json"),
            true,
        )
    }

    fn recording_host(root: &Path) -> Arc<crate::host::RecordingHost> {
        Arc::new(crate::host::RecordingHost::new(
            crate::paths::Paths::from_parts(
                root.join("data"),
                root.join("cache"),
                root.join("logs"),
                None,
                None,
            ),
            crate::host::Flavor::Stable,
        ))
    }

    /// How many loads the engine has announced so far.
    fn loads_started(host: &crate::host::RecordingHost) -> usize {
        host.events()
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    CoreEvent::EngineStatus(status) if status.state == LoadState::Loading
                )
            })
            .count()
    }

    /// With "Keep the speech model loaded" on, Parakeet selected and no
    /// Whisper model (so the GPU probe can never run), each background load
    /// used to probe through `run_exclusive`, which unloaded the engine and
    /// asked for another background load: load, unload, load, without end.
    /// One request must lead to one load, and then the engine must settle.
    #[test]
    fn a_background_load_settles_when_the_gpu_probe_cannot_run() {
        let root = std::env::temp_dir().join(format!("scribe-warm-loop-{}", std::process::id()));
        let host = recording_host(&root);
        let warm = parakeet_engine(&root, host.clone(), true);
        let settings = current_settings(&warm.shared).unwrap();

        warm.sync_with_settings(&settings);
        let first = settled_loads(&host, 1);
        let later = settled_loads(&host, 1);
        warm.shutdown();

        assert_eq!((first, later), (1, 1), "loads started, then a while later");
        let _ = fs::remove_dir_all(&root);
    }

    /// Waits until `host` has seen `count` loads start (or 5 s pass), then
    /// half a second longer, so a load that should not happen has time to
    /// show (the old loop started thousands in that time).
    fn settled_loads(host: &crate::host::RecordingHost, count: usize) -> usize {
        let started = Instant::now();
        while loads_started(host) < count && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(500));
        loads_started(host)
    }

    /// The background load's GPU probe runs with the engine unloaded and
    /// asks for no further load; a one-off run from elsewhere (a file
    /// transcription) loads the kept model again afterwards, as before.
    #[test]
    fn only_a_one_off_run_from_elsewhere_loads_the_model_again() {
        let root = std::env::temp_dir().join(format!("scribe-warm-rerun-{}", std::process::id()));
        let host = recording_host(&root);
        let warm = parakeet_engine(&root, host.clone(), true);
        warm.sync_with_settings(&current_settings(&warm.shared).unwrap());
        assert_eq!(settled_loads(&host, 1), 1);

        warm.run_unloaded(|| ());
        assert_eq!(settled_loads(&host, 1), 1, "run_unloaded asked for a load");

        warm.run_exclusive(|| ());
        assert_eq!(settled_loads(&host, 2), 2, "run_exclusive did not reload");

        warm.shutdown();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn no_load_starts_after_shutdown() {
        let root = std::env::temp_dir().join(format!("scribe-warm-after-{}", std::process::id()));
        let host = recording_host(&root);
        let warm = parakeet_engine(&root, host.clone(), true);
        warm.shutdown();

        warm.sync_with_settings(&current_settings(&warm.shared).unwrap());
        warm.prewarm();
        warm.load_if_kept();
        assert_eq!(settled_loads(&host, 1), 0);
        // A transcription after quitting cannot start the sidecar either.
        let mut inner = lock(&warm.inner);
        let config = desired_config(&warm.shared, &current_settings(&warm.shared).unwrap())
            .expect("the Parakeet model is on disk");
        let client = http_client(&mut inner).unwrap();
        assert_eq!(
            load(&warm.shared, &mut inner, &config, &client),
            Err("Scribe is quitting.".to_string())
        );
        drop(inner);

        let _ = fs::remove_dir_all(&root);
    }

    /// A command that runs for half a minute and does nothing.
    fn long_running_process() -> std::process::Child {
        let mut command = if cfg!(windows) {
            let mut command = std::process::Command::new("ping");
            command.args(["-n", "30", "127.0.0.1"]);
            command
        } else {
            let mut command = std::process::Command::new("sleep");
            command.arg("30");
            command
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Quitting stops every engine process at once, even while a load or a
    /// transcription holds the engine lock, and no process may start after.
    #[test]
    fn shutdown_stops_engine_processes_without_the_engine_lock() {
        let root = std::env::temp_dir().join(format!("scribe-warm-kill-{}", std::process::id()));
        let host = recording_host(&root);
        let warm = parakeet_engine(&root, host.clone(), false);
        let mut process = long_running_process();
        assert!(warm.shared.started(process.id()));

        // Something holds the engine lock for longer than quitting may take.
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let busy = warm.clone();
        let holder = std::thread::spawn(move || {
            let _inner = lock(&busy.inner);
            held_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_secs(3));
        });
        held_rx.recv().unwrap();

        let quitting = Instant::now();
        warm.shutdown();
        assert!(
            quitting.elapsed() < Duration::from_secs(2),
            "shutdown waited {:?} for the engine lock",
            quitting.elapsed()
        );
        let stopped = Instant::now();
        while process.try_wait().unwrap().is_none() {
            assert!(
                stopped.elapsed() < Duration::from_secs(5),
                "the engine process is still running"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        // A process that starts now is refused, so its launcher stops it.
        let mut late = long_running_process();
        assert!(!warm.shared.started(late.id()));
        let _ = late.kill();
        let _ = late.wait();

        holder.join().unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    /// A host whose first `emit` panics, which stops the status thread.
    struct PanicsOnceHost {
        inner: crate::host::RecordingHost,
        emits: std::sync::atomic::AtomicUsize,
    }

    impl Host for PanicsOnceHost {
        fn emit(&self, event: CoreEvent) {
            if self.emits.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("the first status fails on purpose");
            }
            self.inner.emit(event);
        }

        fn paths(&self) -> &crate::paths::Paths {
            self.inner.paths()
        }

        fn flavor(&self) -> crate::host::Flavor {
            self.inner.flavor()
        }

        fn update_tray_status(&self, _status: AppStatus) {}

        fn update_engine_summary(&self, _summary: String) {}

        fn sync_settings(&self, _settings: &AppSettings) {}

        fn open_dashboard(&self, _route: Option<&str>) -> Result<(), CommandError> {
            Err(no_main_window())
        }

        fn toggle_dashboard(&self) -> Result<(), CommandError> {
            Err(no_main_window())
        }

        fn main_window_logical_size(&self) -> Option<(u32, u32)> {
            None
        }

        fn app_version(&self) -> String {
            self.inner.app_version()
        }

        fn exit(&self, code: i32) {
            self.inner.exit(code);
        }

        fn open_path(&self, path: &std::path::Path) -> Result<(), String> {
            self.inner.open_path(path)
        }

        fn open_url(&self, url: &str) -> Result<(), String> {
            self.inner.open_url(url)
        }

        fn notify(&self, title: &str, body: &str) -> Result<(), String> {
            self.inner.notify(title, body)
        }

        fn register_shortcut(&self, shortcut: global_hotkey::hotkey::HotKey) -> Result<(), String> {
            self.inner.register_shortcut(shortcut)
        }

        fn unregister_shortcut(
            &self,
            shortcut: global_hotkey::hotkey::HotKey,
        ) -> Result<(), String> {
            self.inner.unregister_shortcut(shortcut)
        }

        fn is_shortcut_registered(&self, shortcut: global_hotkey::hotkey::HotKey) -> bool {
            self.inner.is_shortcut_registered(shortcut)
        }

        fn launch_at_login(&self) -> Result<bool, String> {
            self.inner.launch_at_login()
        }

        fn set_launch_at_login(&self, enabled: bool) -> Result<(), String> {
            self.inner.set_launch_at_login(enabled)
        }
    }

    /// When the status thread has stopped, the next status starts it again
    /// (statuses are never sent from the engine's own threads).
    #[test]
    fn a_stopped_status_thread_is_started_again() {
        let root = std::env::temp_dir().join(format!("scribe-warm-restart-{}", std::process::id()));
        let host = Arc::new(PanicsOnceHost {
            inner: crate::host::RecordingHost::new(
                crate::paths::Paths::from_parts(
                    root.join("data"),
                    root.join("cache"),
                    root.join("logs"),
                    None,
                    None,
                ),
                crate::host::Flavor::Stable,
            ),
            emits: std::sync::atomic::AtomicUsize::new(0),
        });
        let warm = parakeet_engine(&root, host.clone(), false);
        let mut settings = current_settings(&warm.shared).unwrap();

        warm.sync_with_settings(&settings);
        let started = Instant::now();
        while host.emits.load(Ordering::SeqCst) == 0 {
            assert!(started.elapsed() < Duration::from_secs(5), "no status sent");
            std::thread::sleep(Duration::from_millis(10));
        }
        // New statuses until one arrives: a status sent while the dead thread
        // is still unwinding is lost with it, the next one restarts it.
        settings.keep_whisper_loaded = false;
        let models = ["tiny.en", "base.en"];
        let started = Instant::now();
        let mut sent = 0;
        while host.inner.events().is_empty() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "no status reached the host after {sent} tries"
            );
            settings.selected_model_id = Some(models[sent % 2].into());
            warm.sync_with_settings(&settings);
            sent += 1;
            std::thread::sleep(Duration::from_millis(20));
        }
        let events = host.inner.events();
        let CoreEvent::EngineStatus(status) = &events[0] else {
            panic!("not an engine status: {:?}", events[0]);
        };
        assert!(
            models.contains(&status.model_id.as_deref().unwrap_or_default()),
            "{status:?}"
        );

        warm.shutdown();
        let _ = fs::remove_dir_all(&root);
    }

    /// After quitting began, a Whisper transcription does not fall back to
    /// whisper-cli (which would load the model again), and the background
    /// load's probe step runs nothing.
    #[test]
    fn nothing_loads_a_model_once_quitting_began() {
        let root = std::env::temp_dir().join(format!("scribe-warm-quitcli-{}", std::process::id()));
        let host = recording_host(&root);
        let warm = parakeet_engine(&root, host.clone(), false);
        let model = root.join("data").join("models").join("ggml-tiny.en.bin");
        let wav = root.join("cache").join("clip.wav");
        for file in [&model, &wav] {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"not really audio or a model").unwrap();
        }
        let request = WhisperRequest {
            engine: EngineKind::Whisper,
            model_id: "tiny.en".into(),
            model_path: model,
            wav_path: wav,
            language: "en".into(),
            translate: false,
            vocabulary_prompt: String::new(),
            filler: None,
            gpu: crate::settings::GpuAcceleration::Auto,
            gpu_device_index: None,
        };
        // Before quitting, the fallback runs (and fails: no whisper-cli here).
        assert_eq!(
            warm.transcribe_with(request.clone()).unwrap_err().code,
            "missing_whisper_executable"
        );

        warm.shutdown();
        assert_eq!(
            warm.transcribe_with(request).unwrap_err().code,
            "speech_engine_stopped"
        );
        let mut ran = false;
        assert_eq!(warm.run_unloaded(|| ran = true), None);
        assert!(!ran);

        let _ = fs::remove_dir_all(&root);
    }

    /// One of the two loop fixes: the background load probes through
    /// `run_unloaded`, which asks for no further load. With a probe that can
    /// run and fails, probing through `run_exclusive` instead would start a
    /// second load (the failure memory then stops a third), so exactly one
    /// load and one probe run are expected.
    #[test]
    fn a_background_probe_that_runs_and_fails_leads_to_one_load() {
        let root =
            std::env::temp_dir().join(format!("scribe-warm-probefail-{}", std::process::id()));
        let host = recording_host(&root);
        let warm = parakeet_engine(&root, host.clone(), true);
        let probe_runs = crate::gpu::fake::failing_probe(&root.join("data"));

        warm.sync_with_settings(&current_settings(&warm.shared).unwrap());
        assert_eq!(settled_loads(&host, 1), 1);
        assert_eq!(settled_loads(&host, 1), 1);
        assert_eq!(probe_runs.load(Ordering::SeqCst), 1);

        warm.shutdown();
        let _ = fs::remove_dir_all(&root);
    }

    /// The Audio page's probe ignores the background failure memory, so it
    /// can still try; each try loads the kept model again once (as any
    /// one-off run does) and then the engine settles.
    #[test]
    fn the_person_can_probe_again_and_the_engine_still_settles() {
        let root = std::env::temp_dir().join(format!("scribe-warm-uiprobe-{}", std::process::id()));
        let host = recording_host(&root);
        let warm = parakeet_engine(&root, host.clone(), true);
        let probe_runs = crate::gpu::fake::failing_probe(&root.join("data"));
        warm.sync_with_settings(&current_settings(&warm.shared).unwrap());
        assert_eq!(settled_loads(&host, 1), 1);
        assert_eq!(probe_runs.load(Ordering::SeqCst), 1);

        for click in 1..=2 {
            let result = crate::gpu::probe_with(host.as_ref(), Some(&warm));
            assert!(!result.probed);
            assert_eq!(settled_loads(&host, 1 + click), 1 + click, "click {click}");
            assert_eq!(settled_loads(&host, 1 + click), 1 + click, "click {click}");
        }
        // Each click ran the probe; the background loads it caused did not.
        assert_eq!(probe_runs.load(Ordering::SeqCst), 3);

        warm.shutdown();
        let _ = fs::remove_dir_all(&root);
    }
}
