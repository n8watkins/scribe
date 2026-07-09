//! The HTTP + SSE client (contract sections 3, 4, 7).
//!
//! Blocking reqwest over rustls - no async runtime. `fetch_status` is a
//! point-in-time GET; `watch` opens the SSE stream, is level-triggered on each
//! snapshot, and on any disconnect reverts to not-dictating and reconnects with
//! backoff.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::config::Target;
use crate::snapshot::Snapshot;

/// TCP keepalive probe interval for the SSE stream. reqwest's *blocking* client
/// has no per-read timeout, so we lean on OS-level keepalive to detect a
/// silently-dropped (half-open) connection. The server's `: ping` heartbeat
/// (~10s) keeps the byte stream active; the primary liveness signal remains an
/// EOF on graceful close or `ECONNREFUSED` on connect.
pub const DEFAULT_TCP_KEEPALIVE: Duration = Duration::from_secs(15);
/// Default connect timeout for both status and watch.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Overall timeout for the point-in-time status GET.
pub const DEFAULT_STATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// What happened while trying to read the status snapshot.
#[derive(Debug)]
pub enum ClientError {
    /// Could not connect / connection refused / timed out -> Scribe offline.
    Offline(String),
    /// Server answered `401` - the read token is missing or wrong.
    Unauthorized,
    /// Any other transport or protocol failure.
    Transport(String),
    /// The body was not valid snapshot JSON.
    Decode(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Offline(e) => write!(f, "offline: {e}"),
            ClientError::Unauthorized => write!(f, "unauthorized (read token rejected)"),
            ClientError::Transport(e) => write!(f, "transport error: {e}"),
            ClientError::Decode(e) => write!(f, "could not decode snapshot: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// A successfully fetched snapshot plus the raw JSON exactly as the server sent
/// it. Keeping the raw value lets `--json` emit the payload with full fidelity,
/// preserving additive/unknown fields we do not model.
#[derive(Debug, Clone)]
pub struct FetchedSnapshot {
    pub snapshot: Snapshot,
    pub raw: Value,
}

fn apply_token(
    req: reqwest::blocking::RequestBuilder,
    token: Option<&str>,
) -> reqwest::blocking::RequestBuilder {
    // Header is the primary form and takes precedence per the contract.
    match token {
        Some(t) => req.header("Authorization", format!("Bearer {t}")),
        None => req,
    }
}

fn build_client(
    connect_timeout: Duration,
    overall: Option<Duration>,
    keepalive: Option<Duration>,
) -> reqwest::Result<reqwest::blocking::Client> {
    let mut b = reqwest::blocking::Client::builder().connect_timeout(connect_timeout);
    b = match overall {
        Some(d) => b.timeout(d),
        // Streaming: no overall cap, or a healthy long-lived stream would be cut.
        None => b.timeout(None),
    };
    if let Some(k) = keepalive {
        b = b.tcp_keepalive(k);
    }
    b.build()
}

fn map_reqwest_err(e: &reqwest::Error) -> ClientError {
    if e.is_connect() || e.is_timeout() {
        ClientError::Offline(e.to_string())
    } else {
        ClientError::Transport(e.to_string())
    }
}

/// Point-in-time `GET /v1/status`.
pub fn fetch_status(target: &Target) -> Result<FetchedSnapshot, ClientError> {
    let client = build_client(DEFAULT_CONNECT_TIMEOUT, Some(DEFAULT_STATUS_TIMEOUT), None)
        .map_err(|e| ClientError::Transport(e.to_string()))?;
    let req = apply_token(client.get(&target.status_url), target.token.as_deref());
    let resp = req.send().map_err(|e| map_reqwest_err(&e))?;

    match resp.status().as_u16() {
        200 => {
            // Parse via serde_json directly (avoids pulling reqwest's `json`
            // feature) and keep the raw Value for fidelity in --json output.
            let body = resp.text().map_err(|e| map_reqwest_err(&e))?;
            let raw: Value =
                serde_json::from_str(&body).map_err(|e| ClientError::Decode(e.to_string()))?;
            let snapshot: Snapshot = serde_json::from_value(raw.clone())
                .map_err(|e| ClientError::Decode(e.to_string()))?;
            Ok(FetchedSnapshot { snapshot, raw })
        }
        401 => Err(ClientError::Unauthorized),
        other => Err(ClientError::Transport(format!("unexpected HTTP {other}"))),
    }
}

/// One item surfaced to a [`watch`] caller.
#[derive(Debug, Clone)]
pub enum WatchItem {
    /// A parsed SSE event carrying a snapshot. `event` is the SSE event name
    /// (`state`, `dictation.started`, `dictation.stopped`).
    Event {
        event: String,
        snapshot: Snapshot,
        raw: Value,
    },
    /// The connection was refused or the stream ended: we are reverting to
    /// not-dictating. Carries the synthetic offline snapshot.
    Offline {
        snapshot: Snapshot,
        raw: Value,
        reason: String,
    },
}

/// Tunables for [`watch`].
#[derive(Debug, Clone)]
pub struct WatchOptions {
    /// Reconnect after a disconnect (default true). When false, `watch` returns
    /// after the first stream end.
    pub reconnect: bool,
    /// Initial reconnect backoff.
    pub backoff_initial: Duration,
    /// Maximum reconnect backoff (exponential up to this ceiling).
    pub backoff_max: Duration,
    pub connect_timeout: Duration,
    /// TCP keepalive probe interval (half-open detection); see
    /// [`DEFAULT_TCP_KEEPALIVE`].
    pub tcp_keepalive: Duration,
    /// Test hook: stop after this many connection attempts. `None` = unbounded.
    pub max_connects: Option<usize>,
}

impl Default for WatchOptions {
    fn default() -> Self {
        WatchOptions {
            reconnect: true,
            backoff_initial: Duration::from_millis(500),
            backoff_max: Duration::from_secs(10),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            tcp_keepalive: DEFAULT_TCP_KEEPALIVE,
            max_connects: None,
        }
    }
}

/// Stream `GET /v1/events`, invoking `emit` for each [`WatchItem`].
///
/// `emit` returns `false` to stop the loop (used by the binary's signal path
/// and by tests). `stop` is an out-of-band flag checked between reconnects.
///
/// Level-triggered: every `state` event (and the initial replay) re-establishes
/// truth, so a reconnect is always safe. On disconnect the caller first sees a
/// synthetic `Offline` item, guaranteeing a crashed producer can never leave a
/// consumer believing dictation is still active.
pub fn watch<F>(target: &Target, opts: &WatchOptions, stop: Arc<AtomicBool>, mut emit: F)
where
    F: FnMut(WatchItem) -> bool,
{
    let mut backoff = opts.backoff_initial;
    let mut connects: usize = 0;

    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if let Some(max) = opts.max_connects {
            if connects >= max {
                return;
            }
        }
        connects += 1;

        let outcome = stream_once(target, opts, &stop, &mut emit);
        match outcome {
            StreamOutcome::StoppedByCaller => return,
            StreamOutcome::Ended(reason) => {
                // Revert to not-dictating immediately.
                let snap = Snapshot::offline();
                let raw = serde_json::to_value(&snap).unwrap_or(Value::Null);
                let keep_going = emit(WatchItem::Offline {
                    snapshot: snap,
                    raw,
                    reason,
                });
                if !keep_going || !opts.reconnect {
                    return;
                }
            }
        }

        // Backoff before reconnecting, checking stop as we wait.
        if !sleep_with_stop(backoff, &stop) {
            return;
        }
        backoff = (backoff * 2).min(opts.backoff_max);
    }
}

enum StreamOutcome {
    /// `emit` asked us to stop.
    StoppedByCaller,
    /// The stream ended / could not be opened; carries a human reason.
    Ended(String),
}

fn stream_once<F>(
    target: &Target,
    opts: &WatchOptions,
    stop: &Arc<AtomicBool>,
    emit: &mut F,
) -> StreamOutcome
where
    F: FnMut(WatchItem) -> bool,
{
    let client = match build_client(opts.connect_timeout, None, Some(opts.tcp_keepalive)) {
        Ok(c) => c,
        Err(e) => return StreamOutcome::Ended(format!("client build failed: {e}")),
    };
    let req = apply_token(client.get(&target.events_url), target.token.as_deref())
        .header("Accept", "text/event-stream");
    let resp = match req.send() {
        Ok(r) => r,
        Err(e) => return StreamOutcome::Ended(map_reqwest_err(&e).to_string()),
    };
    match resp.status().as_u16() {
        200 => {}
        401 => return StreamOutcome::Ended(ClientError::Unauthorized.to_string()),
        other => return StreamOutcome::Ended(format!("unexpected HTTP {other}")),
    }

    // Reset backoff conceptually happens at the call site on the next loop; the
    // successful connect means the next `Ended` starts a fresh offline cycle.
    parse_sse(resp, stop, emit)
}

/// Parse the SSE byte stream frame by frame, dispatching each complete event.
fn parse_sse<R, F>(reader: R, stop: &Arc<AtomicBool>, emit: &mut F) -> StreamOutcome
where
    R: std::io::Read,
    F: FnMut(WatchItem) -> bool,
{
    let mut buf = BufReader::new(reader);
    let mut event_name = String::new();
    let mut data = String::new();
    let mut line = String::new();

    loop {
        if stop.load(Ordering::Relaxed) {
            return StreamOutcome::StoppedByCaller;
        }
        line.clear();
        match buf.read_line(&mut line) {
            Ok(0) => return StreamOutcome::Ended("stream closed (EOF)".to_string()),
            Ok(_) => {}
            Err(e) => return StreamOutcome::Ended(format!("read error: {e}")),
        }

        // Strip the trailing newline (and optional CR).
        let content = line.trim_end_matches(['\n', '\r']);

        if content.is_empty() {
            // Blank line dispatches the buffered event.
            if !data.is_empty() {
                let name = if event_name.is_empty() {
                    "message".to_string()
                } else {
                    event_name.clone()
                };
                if let Some(false) = dispatch(&name, &data, emit) {
                    return StreamOutcome::StoppedByCaller;
                }
            }
            event_name.clear();
            data.clear();
            continue;
        }

        if content.starts_with(':') {
            // Comment line (e.g. `: ping` heartbeat). Ignored - but the fact we
            // received it proves the connection is alive.
            continue;
        }

        // Field parsing: `field` or `field:value` (one optional leading space
        // after the colon is stripped, per the SSE spec).
        let (field, value) = match content.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (content, ""),
        };
        match field {
            "event" => {
                event_name.clear();
                event_name.push_str(value);
            }
            "data" => {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value);
            }
            // `id` / `retry` / unknown fields are ignored.
            _ => {}
        }
    }
}

/// Parse a dispatched event's data as a snapshot and hand it to `emit`.
/// Returns `Some(false)` if `emit` asked to stop, `Some(true)` to continue,
/// `None` if the data could not be parsed (skipped).
fn dispatch<F>(event: &str, data: &str, emit: &mut F) -> Option<bool>
where
    F: FnMut(WatchItem) -> bool,
{
    let raw: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return None, // ignore malformed frame, stay connected
    };
    let snapshot: Snapshot = match serde_json::from_value(raw.clone()) {
        Ok(s) => s,
        Err(_) => return None,
    };
    Some(emit(WatchItem::Event {
        event: event.to_string(),
        snapshot,
        raw,
    }))
}

/// Sleep for `dur`, waking early if `stop` is set. Returns `false` if stopped.
fn sleep_with_stop(dur: Duration, stop: &Arc<AtomicBool>) -> bool {
    let step = Duration::from_millis(100);
    let mut remaining = dur;
    while remaining > Duration::ZERO {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let s = step.min(remaining);
        std::thread::sleep(s);
        remaining = remaining.saturating_sub(s);
    }
    !stop.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(frames: &str) -> Vec<WatchItem> {
        let stop = Arc::new(AtomicBool::new(false));
        let mut items = Vec::new();
        parse_sse(frames.as_bytes(), &stop, &mut |item| {
            items.push(item);
            true
        });
        items
    }

    #[test]
    fn parses_a_state_frame() {
        let frames = "event: state\ndata: {\"schemaVersion\":1,\"app\":\"scribe\",\"status\":\"Recording\",\"dictating\":true,\"busy\":true}\n\n";
        let items = collect(frames);
        assert_eq!(items.len(), 1);
        match &items[0] {
            WatchItem::Event {
                event, snapshot, ..
            } => {
                assert_eq!(event, "state");
                assert!(snapshot.dictating);
            }
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn ignores_ping_comments_and_parses_named_edges() {
        let frames = concat!(
            ": ping\n",
            "event: state\ndata: {\"dictating\":false,\"busy\":false,\"status\":\"Idle\"}\n\n",
            ": ping\n",
            "event: dictation.started\ndata: {\"dictating\":true,\"busy\":true,\"status\":\"Recording\"}\n\n",
        );
        let items = collect(frames);
        assert_eq!(items.len(), 2);
        match &items[1] {
            WatchItem::Event {
                event, snapshot, ..
            } => {
                assert_eq!(event, "dictation.started");
                assert!(snapshot.dictating);
            }
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn concatenates_multiline_data() {
        // Two data lines join with a newline (SSE spec); here they form valid
        // JSON only when concatenated.
        let frames = "event: state\ndata: {\"dictating\":false,\ndata: \"busy\":false}\n\n";
        let items = collect(frames);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn skips_malformed_frames_without_dropping_connection() {
        let frames = concat!(
            "event: state\ndata: not-json\n\n",
            "event: state\ndata: {\"dictating\":true,\"busy\":true}\n\n",
        );
        let items = collect(frames);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn caller_can_stop_mid_stream() {
        let frames = concat!(
            "event: state\ndata: {\"dictating\":true,\"busy\":true}\n\n",
            "event: state\ndata: {\"dictating\":false,\"busy\":false}\n\n",
        );
        let stop = Arc::new(AtomicBool::new(false));
        let mut count = 0;
        let outcome = parse_sse(frames.as_bytes(), &stop, &mut |_item| {
            count += 1;
            false // stop after the first
        });
        assert!(matches!(outcome, StreamOutcome::StoppedByCaller));
        assert_eq!(count, 1);
    }
}
