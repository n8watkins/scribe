//! The run's settings (command line `--key=value` or environment `TRIAL_KEY`)
//! and an append-only status file of JSON lines the measuring script reads.

use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

pub struct Config {
    /// `skia-software` (default), `software`, or `skia` (GPU, for comparison only).
    pub renderer: String,
    /// Open the main window at start on this page (0 history, 1 editor, 2 input, 3 controls).
    pub open: Option<i32>,
    /// Show the pill at start in this state (see `pill::Scenario`).
    pub pill: Option<String>,
    /// Keep the pill in its state: no auto-hide after Ready or the confirmation.
    pub hold: bool,
    /// Close (destroy) the main window this many ms after start.
    pub close_after: Option<u64>,
    /// Quit this many ms after start (a test run always has one).
    pub exit_after: Option<u64>,
    pub status: Option<PathBuf>,
    /// `scroll`, `editor` or `open`.
    pub probe: Option<String>,
    /// Pretend the saved pill position is the centre of monitor N.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub monitor: Option<usize>,
    /// Pretend this physical point is the saved pill position.
    pub pill_pos: Option<(i32, i32)>,
    /// Post a click to the pill (our own window only) this many ms after start.
    pub fake_click: Option<u64>,
    /// Log the Copy button instead of writing the clipboard.
    pub no_clipboard: bool,
    /// Do not hand idle pages back to Windows when a window hides or closes.
    pub no_trim: bool,
    /// Save a picture of every pill state into this folder, then quit.
    pub snapshot: Option<PathBuf>,
    /// Save Slint's own picture of the main window (with `open`) to this file, then quit.
    pub snapshot_main: Option<PathBuf>,
    /// Extra fonts for Slint's fallback chain (sets SLINT_FONT_PATH before start).
    pub font_path: Option<String>,
    pub pill_font: Option<String>,
}

fn opt(key: &str) -> Option<String> {
    let flag = format!("--{key}=");
    for a in std::env::args().skip(1) {
        if let Some(v) = a.strip_prefix(&flag) {
            return Some(v.to_string());
        }
        if a == format!("--{key}") {
            return Some("1".into());
        }
    }
    std::env::var(format!("TRIAL_{}", key.to_uppercase().replace('-', "_"))).ok()
}

fn ms(key: &str) -> Option<u64> {
    opt(key).and_then(|v| v.parse().ok())
}

impl Config {
    pub fn load() -> Self {
        let page = |v: String| match v.as_str() {
            "editor" | "1" => 1,
            "input" | "2" => 2,
            "controls" | "3" => 3,
            _ => 0,
        };
        let pair = |v: String| {
            let (a, b) = v.split_once(',')?;
            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
        };
        let open = opt("open").map(page);
        let pill = opt("pill");
        let status = opt("status").map(PathBuf::from);
        let probe = opt("probe");
        let snapshot = opt("snapshot").map(PathBuf::from);
        let test_run = open.is_some()
            || pill.is_some()
            || status.is_some()
            || probe.is_some()
            || snapshot.is_some();
        Self {
            renderer: opt("renderer").unwrap_or_else(|| "skia-software".into()),
            open,
            pill,
            hold: opt("hold").is_some_and(|v| v == "1"),
            close_after: ms("close-after"),
            // A test run never outlives ten minutes, whatever drives it.
            exit_after: ms("exit-after").or(if test_run { Some(600_000) } else { None }),
            status,
            probe,
            monitor: opt("monitor").and_then(|v| v.parse().ok()),
            pill_pos: opt("pill-pos").and_then(pair),
            fake_click: ms("fake-click"),
            no_clipboard: opt("no-clipboard").is_some_and(|v| v == "1"),
            no_trim: opt("no-trim").is_some_and(|v| v == "1"),
            snapshot,
            snapshot_main: opt("snapshot-main").map(PathBuf::from),
            font_path: opt("font-path"),
            pill_font: opt("pill-font"),
        }
    }
}

thread_local! {
    static SINK: RefCell<Option<(PathBuf, Instant)>> = const { RefCell::new(None) };
}

pub fn init(path: Option<PathBuf>, start: Instant) {
    SINK.with(|s| *s.borrow_mut() = path.map(|p| (p, start)));
}

/// A JSON string literal.
pub fn js(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Appends `{"t": <ms since start>, "event": ..., fields...}`; field values are raw JSON.
pub fn log(event: &str, fields: &[(&str, String)]) {
    SINK.with(|s| {
        let s = s.borrow();
        let Some((path, start)) = s.as_ref() else {
            return;
        };
        let mut line = format!(
            "{{\"t\":{:.1},\"event\":{}",
            start.elapsed().as_secs_f64() * 1000.0,
            js(event)
        );
        for (k, v) in fields {
            line.push_str(&format!(",{}:{}", js(k), v));
        }
        line.push_str("}\n");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = f.write_all(line.as_bytes());
        }
    });
}
