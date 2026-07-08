//! Behavioral tests for the client against a mock HTTP+SSE server.

mod support;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use scribe_cli::client::{self, ClientError, WatchItem, WatchOptions};
use scribe_cli::config::{self, Offline, Resolution, ResolveOptions};
use scribe_cli::discovery::Channel;

use support::{frame, MockServer};

/// Build a Target pointing directly at a base URL (bypassing discovery).
fn target_for(base_url: &str, token: Option<&str>) -> config::Target {
    let opts = ResolveOptions {
        base_url: Some(base_url.to_string()),
        token: token.map(|t| t.to_string()),
        ..Default::default()
    };
    match config::resolve(&opts) {
        Resolution::Online(t) => t,
        other => panic!("expected Online, got {other:?}"),
    }
}

fn fast_watch_opts(max_connects: usize) -> WatchOptions {
    WatchOptions {
        reconnect: true,
        backoff_initial: Duration::from_millis(10),
        backoff_max: Duration::from_millis(20),
        max_connects: Some(max_connects),
        ..Default::default()
    }
}

/// A loopback base URL guaranteed to refuse connections: bind an ephemeral
/// port, then drop the listener so the port is closed. Connecting yields an
/// immediate `ECONNREFUSED` rather than hitting the connect timeout.
fn a_closed_base_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

#[test]
fn status_query_returns_snapshot() {
    let server = MockServer::builder().token("abc").start();
    let target = target_for(&server.base_url, Some(&server.token));

    let fetched = client::fetch_status(&target).expect("status ok");
    assert!(fetched.snapshot.dictating);
    assert!(fetched.snapshot.busy);
    assert_eq!(fetched.snapshot.status.as_deref(), Some("Recording"));
    assert_eq!(fetched.snapshot.app, "scribe");
    // Raw JSON preserved for --json.
    assert_eq!(fetched.raw["pid"], 1234);
}

#[test]
fn status_reports_busy_but_not_dictating() {
    // A Transcribing snapshot: busy (output would collide) yet not dictating.
    let transcribing = r#"{"schemaVersion":1,"app":"scribe","status":"Transcribing","dictating":false,"busy":true}"#;
    let server = MockServer::builder()
        .token("abc")
        .status_body(transcribing)
        .start();
    let target = target_for(&server.base_url, Some(&server.token));

    let fetched = client::fetch_status(&target).expect("status ok");
    assert!(!fetched.snapshot.dictating);
    assert!(fetched.snapshot.busy);
    assert_eq!(fetched.snapshot.status.as_deref(), Some("Transcribing"));
}

#[test]
fn status_query_rejects_bad_token() {
    let server = MockServer::builder().token("right").start();
    let target = target_for(&server.base_url, Some("wrong"));

    match client::fetch_status(&target) {
        Err(ClientError::Unauthorized) => {}
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}

#[test]
fn status_query_offline_when_connection_refused() {
    // Nothing is listening on this port -> connection refused -> Offline.
    let target = target_for(&a_closed_base_url(), Some("t"));
    match client::fetch_status(&target) {
        Err(ClientError::Offline(_)) => {}
        other => panic!("expected Offline, got {other:?}"),
    }
}

#[test]
fn watch_receives_initial_replay_and_edge_event() {
    let idle =
        r#"{"schemaVersion":1,"app":"scribe","status":"Idle","dictating":false,"busy":false}"#;
    let recording =
        r#"{"schemaVersion":1,"app":"scribe","status":"Recording","dictating":true,"busy":true}"#;
    let server = MockServer::builder()
        .token("abc")
        .event_connection(vec![
            frame("state", idle),
            frame("dictation.started", recording),
        ])
        .start();
    let target = target_for(&server.base_url, Some("abc"));

    let events: Arc<Mutex<Vec<(String, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let sink = events.clone();
    client::watch(&target, &fast_watch_opts(1), stop, move |item| {
        if let WatchItem::Event {
            event, snapshot, ..
        } = item
        {
            let mut g = sink.lock().unwrap();
            g.push((event, snapshot.dictating));
            // Stop after we've seen both frames.
            return g.len() < 2;
        }
        true
    });

    let got = events.lock().unwrap();
    assert_eq!(got.len(), 2, "got {got:?}");
    assert_eq!(got[0], ("state".to_string(), false));
    assert_eq!(got[1], ("dictation.started".to_string(), true));
}

#[test]
fn watch_reverts_to_not_dictating_and_reconnects_on_close() {
    let recording =
        r#"{"schemaVersion":1,"app":"scribe","status":"Recording","dictating":true,"busy":true}"#;
    let stopped =
        r#"{"schemaVersion":1,"app":"scribe","status":"Idle","dictating":false,"busy":false}"#;
    // Connection #1 sends one recording frame then closes; connection #2 sends
    // a stopped frame. The client must emit an Offline item in between.
    let server = MockServer::builder()
        .token("abc")
        .event_connection(vec![frame("state", recording)])
        .event_connection(vec![frame("state", stopped)])
        .start();
    let target = target_for(&server.base_url, Some("abc"));

    #[derive(Debug, PartialEq)]
    enum Kind {
        Event(bool),
        Offline,
    }
    let log: Arc<Mutex<Vec<Kind>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let sink = log.clone();
    client::watch(&target, &fast_watch_opts(3), stop, move |item| {
        let mut g = sink.lock().unwrap();
        match item {
            WatchItem::Event { snapshot, .. } => g.push(Kind::Event(snapshot.dictating)),
            WatchItem::Offline { snapshot, .. } => {
                assert!(!snapshot.dictating, "offline must be not-dictating");
                assert!(!snapshot.busy);
                g.push(Kind::Offline);
            }
        }
        // Stop once we've seen the second connection's event.
        let events_seen = g.iter().filter(|k| matches!(k, Kind::Event(_))).count();
        events_seen < 2
    });

    let got = log.lock().unwrap();
    // Expected order: recording event, synthetic offline, stopped event.
    assert!(got.contains(&Kind::Event(true)), "got {got:?}");
    assert!(got.contains(&Kind::Offline), "got {got:?}");
    assert!(got.contains(&Kind::Event(false)), "got {got:?}");
    // The Offline must sit strictly between the two events (reconnect happened).
    let first_offline = got.iter().position(|k| *k == Kind::Offline).unwrap();
    let last_event = got
        .iter()
        .rposition(|k| matches!(k, Kind::Event(_)))
        .unwrap();
    assert!(
        first_offline < last_event,
        "offline should precede reconnect event: {got:?}"
    );
    assert!(server.event_connection_count() >= 2);
}

#[test]
fn watch_offline_when_events_unauthorized() {
    let server = MockServer::builder().token("right").start();
    let target = target_for(&server.base_url, Some("wrong"));

    let saw_offline = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let flag = saw_offline.clone();
    // No frames configured; a 401 makes stream_once end -> Offline emitted.
    client::watch(&target, &fast_watch_opts(1), stop, move |item| {
        if let WatchItem::Offline { snapshot, .. } = item {
            assert!(!snapshot.dictating);
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        false
    });
    assert!(saw_offline.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn resolve_missing_control_file_is_offline() {
    let dir = tempfile::tempdir().unwrap();
    let opts = ResolveOptions {
        control_path: Some(dir.path().join("control.json")),
        channel: Channel::Stable,
        pid_check: true,
        ..Default::default()
    };
    assert!(matches!(
        config::resolve(&opts),
        Resolution::Offline(Offline::NoControlFile)
    ));
}

#[test]
fn end_to_end_discovery_then_status() {
    // Write a real control.json pointing at the mock server, then resolve it
    // through discovery (no base_url override) and fetch status.
    let server = MockServer::builder().token("disco-token").start();
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("control.json");
    let body = format!(
        r#"{{"schemaVersion":1,"app":"scribe","appVersion":"0.7.0","pid":{pid},
        "transport":"http-sse","baseUrl":"{base}",
        "endpoints":{{"status":"/v1/status","events":"/v1/events"}},
        "readToken":"disco-token","updatedAt":"2026-07-08T12:34:56.000Z"}}"#,
        pid = std::process::id(),
        base = server.base_url,
    );
    std::fs::write(&control, body).unwrap();

    let opts = ResolveOptions {
        control_path: Some(control),
        channel: Channel::Stable,
        pid_check: true,
        ..Default::default()
    };
    let target = match config::resolve(&opts) {
        Resolution::Online(t) => t,
        other => panic!("expected Online, got {other:?}"),
    };
    assert_eq!(target.token.as_deref(), Some("disco-token"));
    let fetched = client::fetch_status(&target).expect("status ok");
    assert!(fetched.snapshot.dictating);
}
