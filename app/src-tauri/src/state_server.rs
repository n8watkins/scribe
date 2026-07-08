//! The in-process, always-on localhost HTTP + SSE server that offers Scribe's
//! live dictation state to any external tool.
//!
//! This is the canonical transport of the dictation-state interface (the CLI
//! and MCP facades are thin clients over it). It exposes two routes on an
//! ephemeral loopback port:
//!
//!   * `GET /v1/status` - the current snapshot as a point-in-time query,
//!   * `GET /v1/events` - an SSE stream that replays the current snapshot on
//!     connect and then pushes one event per state change.
//!
//! Design notes:
//!   * `tiny_http` on a dedicated `std::thread`, one thread per connection, with
//!     hand-written SSE framing. This matches the app's blocking/`std::thread`
//!     model and avoids pulling in an async runtime the codebase does not use.
//!   * The server never owns dictation state. It caches the latest snapshot
//!     pushed through the shared `emit_state_snapshot` choke point (exactly as
//!     `status_file` does) and derives `dictating`/`busy` via `dictation_state`,
//!     so there is a single source of truth and the mapping cannot drift.
//!   * Liveness is intrinsic: if Scribe is down the port refuses connections; if
//!     it crashes mid-stream the socket drops and the SSE stream EOFs. A `: ping`
//!     comment every ~10s lets a consumer detect a hung connection.
//!
//! The wire format is frozen in `docs/integrations/dictation-state-contract.md`.

use crate::app_state::{AppStatus, AppStateSnapshot};
use crate::dictation_state::{self, DictationSnapshot};
use chrono::{DateTime, Utc};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tiny_http::{Header, Method, Response, Server, StatusCode};

/// Loopback host - never bind a routable interface.
const SERVER_HOST: &str = "127.0.0.1";
/// Discovery-file contract version (see the wire contract, section 5).
const CONTROL_SCHEMA_VERSION: u32 = 1;
/// SSE comment ping cadence, so a consumer can detect a hung connection.
const PING_INTERVAL: Duration = Duration::from_secs(10);
/// How often the status-file fallback is re-written so its `updatedAt` stays
/// fresh during a long recording (the contract documents a ~5s heartbeat).
const STATUS_FILE_HEARTBEAT: Duration = Duration::from_secs(5);
/// Accept-loop / heartbeat wake granularity, so shutdown is observed promptly.
const SHUTDOWN_TICK: Duration = Duration::from_millis(500);

const STATUS_PATH: &str = "/v1/status";
const EVENTS_PATH: &str = "/v1/events";

/// The broadcast hub: the latest snapshot plus the set of live SSE subscribers.
///
/// Shared (via `Arc`) between the accept loop, every connection thread, and the
/// heartbeat thread. `publish` is the only writer of `latest`.
struct Hub {
    /// The most recent state snapshot pushed through the choke point. Seeded
    /// with `Idle` so a consumer that connects before the first transition
    /// still gets a valid snapshot.
    latest: Mutex<AppStateSnapshot>,
    /// Live SSE connections, each keyed by a unique id so it can remove itself
    /// on disconnect (`Drop`), plus its sender for broadcast.
    subscribers: Mutex<Vec<(usize, Sender<Vec<u8>>)>>,
    next_id: AtomicUsize,
    pid: u32,
    read_token: String,
}

impl Hub {
    fn new(pid: u32, read_token: String) -> Self {
        Self {
            latest: Mutex::new(AppStateSnapshot {
                status: AppStatus::Idle,
                error: None,
                updated_at: Utc::now(),
            }),
            subscribers: Mutex::new(Vec::new()),
            next_id: AtomicUsize::new(0),
            pid,
            read_token,
        }
    }

    /// Build the wire snapshot from the current cached state, stamping
    /// `updatedAt` with the caller's clock.
    fn build(&self, state: &AppStateSnapshot) -> DictationSnapshot {
        DictationSnapshot::from_state(state, Utc::now(), self.pid)
    }

    /// Record a new snapshot and broadcast it to every live SSE connection.
    ///
    /// Emits a `state` event on every transition and, only on the matching edge
    /// of the `dictating` boolean, a `dictation.started` / `dictation.stopped`.
    /// Both events of a transition are sent as one channel message so a
    /// consumer sees `state` strictly before the edge event.
    fn publish(&self, state: &AppStateSnapshot) {
        // Lock order (latest, then subscribers) matches `subscribe`, so a
        // mid-connect publish is cleanly serialized against the initial replay:
        // a consumer never misses a transition nor sees a duplicate.
        let mut latest = self.latest.lock().unwrap();
        let was_dictating = dictation_state::is_dictating(&latest.status);
        *latest = state.clone();
        let snapshot = self.build(&latest);
        let now_dictating = snapshot.dictating;

        let mut payload = frame_event("state", &snapshot);
        if now_dictating && !was_dictating {
            payload.extend_from_slice(&frame_event("dictation.started", &snapshot));
        } else if !now_dictating && was_dictating {
            payload.extend_from_slice(&frame_event("dictation.stopped", &snapshot));
        }

        // Prune any connection whose receiver has gone away.
        self.subscribers
            .lock()
            .unwrap()
            .retain(|(_, tx)| tx.send(payload.clone()).is_ok());
    }

    /// The current snapshot as the `/v1/status` JSON body.
    fn status_json(&self) -> String {
        let latest = self.latest.lock().unwrap();
        let snapshot = self.build(&latest);
        serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string())
    }

    /// A clone of the cached state, for the status-file heartbeat re-write.
    fn latest_state(&self) -> AppStateSnapshot {
        self.latest.lock().unwrap().clone()
    }

    /// Register a new SSE connection. Returns its id, its receiver, and the
    /// framed initial `state` event to send before any live update.
    fn subscribe(&self) -> (usize, Receiver<Vec<u8>>, Vec<u8>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        // Hold `latest` across the snapshot-and-register so the initial replay
        // and the subscriber registration are atomic w.r.t. `publish`.
        let latest = self.latest.lock().unwrap();
        let initial = frame_event("state", &self.build(&latest));
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.subscribers.lock().unwrap().push((id, tx));
        drop(latest);
        (id, rx, initial)
    }

    fn unsubscribe(&self, id: usize) {
        self.subscribers
            .lock()
            .unwrap()
            .retain(|(sid, _)| *sid != id);
    }

    /// Drop every subscriber sender so blocked connection readers observe a
    /// disconnected channel and EOF their streams promptly (used on shutdown).
    fn disconnect_all(&self) {
        self.subscribers.lock().unwrap().clear();
    }
}

/// Frame a snapshot as a single SSE event. Compact JSON has no newlines, so a
/// single `data:` line is always valid.
fn frame_event(event: &str, snapshot: &DictationSnapshot) -> Vec<u8> {
    let data = serde_json::to_string(snapshot).unwrap_or_else(|_| "{}".to_string());
    format!("event: {event}\ndata: {data}\n\n").into_bytes()
}

/// RAII handle for a live SSE subscription: removing it from the hub on drop
/// reaps the connection on any exit path (client disconnect, server shutdown,
/// or a panic in the connection thread).
struct Subscription {
    hub: Arc<Hub>,
    id: usize,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub.unsubscribe(self.id);
    }
}

/// The managed handle to the running server. Held in Tauri state; `publish`
/// feeds it from the choke point and `shutdown` tears it down on exit.
pub struct DictationServer {
    hub: Arc<Hub>,
    shutdown: Arc<AtomicBool>,
    control_path: PathBuf,
    base_url: String,
}

impl DictationServer {
    /// Broadcast a fresh state snapshot to all SSE consumers. Called from
    /// `dictation_state::emit_state_snapshot`.
    pub fn publish(&self, snapshot: &AppStateSnapshot) {
        self.hub.publish(snapshot);
    }

    /// The loopback origin the server is bound to (for diagnostics).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Stop accepting connections, close live SSE streams, and remove the
    /// discovery file. Best-effort and safe to call once at exit.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.hub.disconnect_all();
        if let Err(error) = std::fs::remove_file(&self.control_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("Could not remove control file on shutdown: {error}");
            }
        }
        log::info!("Dictation-state server shut down");
    }
}

/// Bind the loopback server and spawn its accept loop. Returns the shared server
/// handle and the bound port. Kept separate from `start` so it can be exercised
/// in tests without a Tauri `AppHandle`.
fn spawn_http(hub: Arc<Hub>, shutdown: Arc<AtomicBool>) -> Result<u16, String> {
    let server = Server::http((SERVER_HOST, 0))
        .map_err(|error| format!("could not bind dictation-state server: {error}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|addr| addr.port())
        .ok_or_else(|| "dictation-state server bound to a non-IP address".to_string())?;

    let server = Arc::new(server);
    thread::Builder::new()
        .name("dictation-state-server".to_string())
        .spawn(move || accept_loop(server, hub, shutdown))
        .map_err(|error| format!("could not spawn dictation-state server thread: {error}"))?;

    Ok(port)
}

/// Accept connections until shutdown, handing each to its own thread. A
/// long-lived SSE connection therefore never blocks `/v1/status` requests.
fn accept_loop(server: Arc<Server>, hub: Arc<Hub>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Relaxed) {
        match server.recv_timeout(SHUTDOWN_TICK) {
            Ok(Some(request)) => {
                let hub = hub.clone();
                if let Err(error) = thread::Builder::new()
                    .name("dictation-state-conn".to_string())
                    .spawn(move || handle_request(request, hub))
                {
                    log::warn!("Could not spawn dictation-state connection thread: {error}");
                }
            }
            Ok(None) => {} // timeout: re-check the shutdown flag
            Err(error) => {
                log::warn!("Dictation-state server accept error: {error}");
                break;
            }
        }
    }
    log::info!("Dictation-state server accept loop stopped");
}

/// Route one request. Every route requires the read token.
fn handle_request(request: tiny_http::Request, hub: Arc<Hub>) {
    let authorized = request_token(&request)
        .map(|token| token == hub.read_token)
        .unwrap_or(false);

    // Strip any query string before matching the path.
    let path = request
        .url()
        .split('?')
        .next()
        .unwrap_or("")
        .to_string();
    let is_get = request.method() == &Method::Get;

    if is_get && path == STATUS_PATH {
        if !authorized {
            respond_unauthorized(request);
            return;
        }
        let response = Response::from_string(hub.status_json())
            .with_header(header("Content-Type", "application/json"));
        let _ = request.respond(response);
    } else if is_get && path == EVENTS_PATH {
        if !authorized {
            respond_unauthorized(request);
            return;
        }
        serve_events(request, hub);
    } else {
        let response = Response::from_string("Not Found").with_status_code(StatusCode(404));
        let _ = request.respond(response);
    }
}

/// Open an SSE stream.
///
/// The response is written directly to the raw socket rather than through
/// tiny_http's `Response`, because tiny_http wraps a streaming body in a
/// `chunked_transfer::Encoder` that buffers ~8 KB before flushing - small SSE
/// events would never reach the consumer in real time. Writing the response
/// ourselves lets us flush after every frame. With no `Content-Length` and no
/// chunked encoding, the body is delimited by connection close, which is exactly
/// the SSE lifecycle: the stream stays open until Scribe exits or the consumer
/// disconnects.
fn serve_events(request: tiny_http::Request, hub: Arc<Hub>) {
    let (id, rx, initial) = hub.subscribe();
    // Reaps the subscription on every exit path.
    let _subscription = Subscription {
        hub: hub.clone(),
        id,
    };

    let mut writer = request.into_writer();
    let head = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream\r\n",
        "Cache-Control: no-cache\r\n",
        "Connection: keep-alive\r\n",
        // Defeat any intermediary's response buffering; harmless direct.
        "X-Accel-Buffering: no\r\n",
        "\r\n",
    );
    if write_flush(&mut writer, head.as_bytes()).is_err() {
        return;
    }

    // Replay the current snapshot immediately, then stream every subsequent
    // event, emitting a comment ping on idle so the connection keeps proving
    // itself alive and a broken pipe is noticed within one ping interval.
    let mut frame = initial;
    loop {
        if write_flush(&mut writer, &frame).is_err() {
            return; // consumer disconnected
        }
        frame = match rx.recv_timeout(PING_INTERVAL) {
            Ok(bytes) => bytes,
            Err(RecvTimeoutError::Timeout) => b": ping\n\n".to_vec(),
            Err(RecvTimeoutError::Disconnected) => return, // server shut down
        };
    }
}

fn write_flush<W: Write>(writer: &mut W, bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(bytes)?;
    writer.flush()
}

fn respond_unauthorized(request: tiny_http::Request) {
    let response = Response::from_string("Unauthorized").with_status_code(StatusCode(401));
    let _ = request.respond(response);
}

/// Extract the read token from `Authorization: Bearer <token>` (primary) or a
/// `?token=<token>` query parameter (fallback for browser `EventSource`).
fn request_token(request: &tiny_http::Request) -> Option<String> {
    for header in request.headers() {
        if header.field.to_string().eq_ignore_ascii_case("authorization") {
            let value = header.value.to_string();
            let token = value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "));
            if let Some(token) = token {
                return Some(token.trim().to_string());
            }
        }
    }
    let query = request.url().split_once('?').map(|(_, q)| q)?;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("token=").map(|token| token.to_string()))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .expect("static header name/value are valid ASCII")
}

/// The discovery-file payload written to `~/.scribe/control.json`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ControlFile {
    schema_version: u32,
    app: &'static str,
    app_version: &'static str,
    pid: u32,
    transport: &'static str,
    base_url: String,
    endpoints: Endpoints,
    read_token: String,
    updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Endpoints {
    status: &'static str,
    events: &'static str,
}

/// Resolve the discovery-file path: `~/.scribe/control.json` for stable, and
/// `control.dev.json` for the Dev flavor so the two can run side by side without
/// clobbering each other. Creates `~/.scribe/` if needed.
fn control_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("could not resolve home dir: {error}"))?;
    let dir = home.join(".scribe");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create ~/.scribe: {error}"))?;
    let name = if crate::is_dev_flavor(app) {
        "control.dev.json"
    } else {
        "control.json"
    };
    Ok(dir.join(name))
}

/// Atomically write the discovery file (temp + rename), then best-effort lock it
/// down to the owner on Unix.
fn write_control_file(
    path: &Path,
    base_url: &str,
    read_token: &str,
    pid: u32,
) -> Result<(), String> {
    let payload = ControlFile {
        schema_version: CONTROL_SCHEMA_VERSION,
        app: dictation_state::APP_NAME,
        app_version: dictation_state::app_version(),
        pid,
        transport: "http-sse",
        base_url: base_url.to_string(),
        endpoints: Endpoints {
            status: STATUS_PATH,
            events: EVENTS_PATH,
        },
        read_token: read_token.to_string(),
        updated_at: Utc::now(),
    };
    let json = serde_json::to_vec_pretty(&payload)
        .map_err(|error| format!("could not serialize control file: {error}"))?;
    let staging = path.with_extension(format!("json.{pid}.tmp"));
    std::fs::write(&staging, &json)
        .map_err(|error| format!("could not stage control file: {error}"))?;
    if let Err(error) = std::fs::rename(&staging, path) {
        let _ = std::fs::remove_file(&staging);
        return Err(format!("could not finalize control file: {error}"));
    }
    restrict_permissions(path);
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {
    // Windows relies on the per-user profile ACL; no portable chmod equivalent.
}

/// Periodically re-write the status-file fallback so its `updatedAt` stays fresh
/// during a long recording (transitions alone can be minutes apart). Exits when
/// the server shuts down.
fn spawn_status_heartbeat(app: AppHandle, hub: Arc<Hub>, shutdown: Arc<AtomicBool>) {
    let _ = thread::Builder::new()
        .name("dictation-status-heartbeat".to_string())
        .spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                sleep_until_due(&shutdown, STATUS_FILE_HEARTBEAT);
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                crate::status_file::publish(&app, &hub.latest_state());
            }
        });
}

/// Sleep for `total`, waking every `SHUTDOWN_TICK` to observe the shutdown flag.
fn sleep_until_due(shutdown: &AtomicBool, total: Duration) {
    let mut slept = Duration::ZERO;
    while slept < total {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        let step = SHUTDOWN_TICK.min(total - slept);
        thread::sleep(step);
        slept += step;
    }
}

/// Stand up the dictation-state server: bind the loopback port, start the accept
/// and heartbeat threads, and publish the discovery file. Called once from the
/// Tauri `setup`.
pub fn start(app: &AppHandle) -> Result<DictationServer, String> {
    let pid = std::process::id();
    let read_token = uuid::Uuid::new_v4().to_string();
    let hub = Arc::new(Hub::new(pid, read_token.clone()));
    let shutdown = Arc::new(AtomicBool::new(false));

    let port = spawn_http(hub.clone(), shutdown.clone())?;
    let base_url = format!("http://{SERVER_HOST}:{port}");

    let control_path = control_file_path(app)?;
    write_control_file(&control_path, &base_url, &read_token, pid)?;

    spawn_status_heartbeat(app.clone(), hub.clone(), shutdown.clone());

    log::info!("Dictation-state server listening on {base_url}");
    Ok(DictationServer {
        hub,
        shutdown,
        control_path,
        base_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn recording(now: DateTime<Utc>) -> AppStateSnapshot {
        AppStateSnapshot {
            status: AppStatus::Recording,
            error: None,
            updated_at: now,
        }
    }

    fn idle(now: DateTime<Utc>) -> AppStateSnapshot {
        AppStateSnapshot {
            status: AppStatus::Idle,
            error: None,
            updated_at: now,
        }
    }

    /// Bind a server backed by a fresh hub and return (base_url, token, hub,
    /// shutdown) for a test to drive.
    fn test_server() -> (String, String, Arc<Hub>, Arc<AtomicBool>) {
        let token = "test-token-1234".to_string();
        let hub = Arc::new(Hub::new(4242, token.clone()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let port = spawn_http(hub.clone(), shutdown.clone()).expect("bind test server");
        (format!("http://{SERVER_HOST}:{port}"), token, hub, shutdown)
    }

    #[test]
    fn status_requires_a_valid_token() {
        let (base, token, _hub, shutdown) = test_server();
        let client = reqwest::blocking::Client::new();

        let missing = client.get(format!("{base}{STATUS_PATH}")).send().unwrap();
        assert_eq!(missing.status().as_u16(), 401);

        let wrong = client
            .get(format!("{base}{STATUS_PATH}"))
            .header("Authorization", "Bearer nope")
            .send()
            .unwrap();
        assert_eq!(wrong.status().as_u16(), 401);

        let ok = client
            .get(format!("{base}{STATUS_PATH}"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .unwrap();
        assert_eq!(ok.status().as_u16(), 200);
        shutdown.store(true, Ordering::Relaxed);
    }

    #[test]
    fn status_serves_the_current_snapshot() {
        let (base, token, hub, shutdown) = test_server();
        hub.publish(&recording(Utc::now()));

        let client = reqwest::blocking::Client::new();
        let text = client
            .get(format!("{base}{STATUS_PATH}"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .unwrap()
            .text()
            .unwrap();
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();

        assert_eq!(body["schemaVersion"], 1);
        assert_eq!(body["app"], "scribe");
        assert_eq!(body["status"], "Recording");
        assert_eq!(body["dictating"], true);
        assert_eq!(body["busy"], true);
        assert_eq!(body["pid"], 4242);
        shutdown.store(true, Ordering::Relaxed);
    }

    #[test]
    fn status_token_accepted_via_query_fallback() {
        let (base, token, _hub, shutdown) = test_server();
        let client = reqwest::blocking::Client::new();
        let ok = client
            .get(format!("{base}{STATUS_PATH}?token={token}"))
            .send()
            .unwrap();
        assert_eq!(ok.status().as_u16(), 200);
        shutdown.store(true, Ordering::Relaxed);
    }

    #[test]
    fn events_replays_initial_snapshot_then_streams_a_transition() {
        let (base, token, hub, shutdown) = test_server();
        // Seed a known starting state before anyone connects.
        hub.publish(&idle(Utc::now()));

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let mut stream = client
            .get(format!("{base}{EVENTS_PATH}"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .unwrap();
        assert_eq!(stream.status().as_u16(), 200);
        assert_eq!(
            stream
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );

        // The initial replay must be a `state` event carrying the current
        // (Idle) snapshot, delivered without a separate query.
        let initial = read_available(&mut stream);
        assert!(initial.contains("event: state"), "got: {initial}");
        assert!(initial.contains("\"status\":\"Idle\""), "got: {initial}");

        // A live transition into Recording must push `state` then the
        // `dictation.started` edge event, in that order.
        hub.publish(&recording(Utc::now()));
        let update = read_available(&mut stream);
        assert!(update.contains("event: state"), "got: {update}");
        assert!(update.contains("event: dictation.started"), "got: {update}");
        assert!(update.contains("\"dictating\":true"), "got: {update}");
        let state_at = update.find("event: state").unwrap();
        let edge_at = update.find("event: dictation.started").unwrap();
        assert!(state_at < edge_at, "state must precede the edge event");

        shutdown.store(true, Ordering::Relaxed);
        hub.disconnect_all();
    }

    #[test]
    fn events_edge_fires_only_on_the_dictating_boundary() {
        let (base, token, hub, shutdown) = test_server();
        hub.publish(&recording(Utc::now()));

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let mut stream = client
            .get(format!("{base}{EVENTS_PATH}"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .unwrap();
        let _initial = read_available(&mut stream);

        // Recording -> Stopping keeps `dictating` true: only a `state` event,
        // no edge event.
        hub.publish(&AppStateSnapshot {
            status: AppStatus::Stopping,
            error: None,
            updated_at: Utc::now(),
        });
        let update = read_available(&mut stream);
        assert!(update.contains("event: state"), "got: {update}");
        assert!(
            !update.contains("dictation.started") && !update.contains("dictation.stopped"),
            "no edge event expected when dictating stays true: {update}"
        );

        shutdown.store(true, Ordering::Relaxed);
        hub.disconnect_all();
    }

    /// Read one chunk of bytes from a streaming response into a String. The
    /// server writes each event as its own frame and flushes, so a single read
    /// returns a whole event (or events) frame.
    fn read_available(stream: &mut reqwest::blocking::Response) -> String {
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).expect("read sse chunk");
        String::from_utf8_lossy(&buf[..n]).to_string()
    }
}
