//! A tiny std-only HTTP + SSE server that serves the documented `/v1/status`
//! and `/v1/events`, so the client can be tested without a real Scribe (which
//! is Windows-only and won't run in CI). No external server dependency.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// An SSE frame body: `event: <name>\ndata: <json>\n\n`.
pub fn frame(event: &str, json: &str) -> String {
    format!("event: {event}\ndata: {json}\n\n")
}

/// Behavior of the mock server, shared with its accept thread.
struct Config {
    token: String,
    /// JSON body returned by `/v1/status` (200).
    status_body: String,
    /// Frames written by successive `/v1/events` connections. Connection `i`
    /// uses `plans[i]` (clamped to the last entry), writes them, then closes -
    /// exercising the client's reconnect path.
    event_plans: Vec<Vec<String>>,
}

pub struct MockServer {
    pub base_url: String,
    pub token: String,
    shutdown: Arc<AtomicBool>,
    /// Number of `/v1/events` connections accepted.
    event_conns: Arc<AtomicUsize>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    pub fn builder() -> Builder {
        Builder {
            token: "test-token".to_string(),
            status_body: default_status_body(),
            event_plans: Vec::new(),
        }
    }

    /// How many `/v1/events` connections have been accepted so far.
    pub fn event_connection_count(&self) -> usize {
        self.event_conns.load(Ordering::SeqCst)
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub struct Builder {
    token: String,
    status_body: String,
    event_plans: Vec<Vec<String>>,
}

impl Builder {
    pub fn token(mut self, t: &str) -> Self {
        self.token = t.to_string();
        self
    }

    pub fn status_body(mut self, body: &str) -> Self {
        self.status_body = body.to_string();
        self
    }

    /// Add a plan for the next `/v1/events` connection.
    pub fn event_connection(mut self, frames: Vec<String>) -> Self {
        self.event_plans.push(frames);
        self
    }

    pub fn start(self) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("local addr");
        let base_url = format!("http://{addr}");
        let token = self.token.clone();

        let cfg = Arc::new(Config {
            token: self.token,
            status_body: self.status_body,
            event_plans: self.event_plans,
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let event_conns = Arc::new(AtomicUsize::new(0));

        let handle = {
            let cfg = cfg.clone();
            let shutdown = shutdown.clone();
            let event_conns = event_conns.clone();
            let conn_threads: Arc<Mutex<Vec<thread::JoinHandle<()>>>> =
                Arc::new(Mutex::new(Vec::new()));
            thread::spawn(move || {
                loop {
                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let cfg = cfg.clone();
                            let event_conns = event_conns.clone();
                            let t = thread::spawn(move || {
                                handle_connection(stream, &cfg, &event_conns);
                            });
                            conn_threads.lock().unwrap().push(t);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
                for t in conn_threads.lock().unwrap().drain(..) {
                    let _ = t.join();
                }
            })
        };

        MockServer {
            base_url,
            token,
            shutdown,
            event_conns,
            handle: Some(handle),
        }
    }
}

struct Request {
    path: String,
    query_token: Option<String>,
    bearer: Option<String>,
}

fn parse_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let mut parts = request_line.split_whitespace();
    let _method = parts.next()?;
    let target = parts.next()?.to_string();

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target, None),
    };
    let query_token = query.and_then(|q| {
        q.split('&')
            .find_map(|kv| kv.strip_prefix("token=").map(|v| v.to_string()))
    });

    let mut bearer = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.to_ascii_lowercase().strip_prefix("authorization:") {
            let val = rest.trim();
            if let Some(tok) = val.strip_prefix("bearer ") {
                bearer = Some(tok.trim().to_string());
            }
        }
    }

    Some(Request {
        path,
        query_token,
        bearer,
    })
}

fn authorized(req: &Request, token: &str) -> bool {
    if let Some(b) = &req.bearer {
        return b == token;
    }
    if let Some(q) = &req.query_token {
        return q == token;
    }
    false
}

fn write_401(stream: &mut TcpStream) {
    let _ = stream
        .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.flush();
}

fn write_404(stream: &mut TcpStream) {
    let _ = stream
        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.flush();
}

fn handle_connection(mut stream: TcpStream, cfg: &Config, event_conns: &AtomicUsize) {
    let req = match parse_request(&stream) {
        Some(r) => r,
        None => return,
    };

    if !authorized(&req, &cfg.token) {
        write_401(&mut stream);
        return;
    }

    match req.path.as_str() {
        "/v1/status" => {
            let body = cfg.status_body.as_bytes();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.flush();
        }
        "/v1/events" => {
            let idx = event_conns.fetch_add(1, Ordering::SeqCst);
            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.flush();

            let frames = if cfg.event_plans.is_empty() {
                &Vec::new()
            } else {
                let i = idx.min(cfg.event_plans.len() - 1);
                &cfg.event_plans[i]
            };
            for f in frames {
                if stream.write_all(f.as_bytes()).is_err() {
                    return;
                }
                let _ = stream.flush();
                // A small gap so the client parses frames incrementally.
                thread::sleep(Duration::from_millis(5));
            }
            // Drain any remaining request bytes briefly, then close (EOF) to
            // exercise the client's reconnect-on-close path.
            let mut scratch = [0u8; 64];
            let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
            let _ = stream.read(&mut scratch);
            // Dropping `stream` closes the connection.
        }
        _ => write_404(&mut stream),
    }
}

fn default_status_body() -> String {
    r#"{"schemaVersion":1,"app":"scribe","appVersion":"0.7.0","status":"Recording","dictating":true,"busy":true,"since":"2026-07-08T12:34:56.789Z","updatedAt":"2026-07-08T12:34:56.812Z","pid":1234}"#.to_string()
}
