//! Rendering snapshots for humans and for scripts.
//!
//! - `--json` / default `watch`: compact single-line JSON of the RAW payload,
//!   so nothing is lost (newline-delimited JSON, ideal for piping to `jq`).
//! - human mode: a short, readable summary.

use serde_json::Value;

use crate::snapshot::Snapshot;

/// One compact JSON line (no trailing newline) for the raw payload.
pub fn json_line(raw: &Value) -> String {
    serde_json::to_string(raw).unwrap_or_else(|_| "{}".to_string())
}

/// A one-word headline for the snapshot's state.
fn headline(s: &Snapshot) -> &'static str {
    if s.offline {
        "offline"
    } else if s.dictating {
        "dictating"
    } else if s.busy {
        "busy"
    } else {
        "idle"
    }
}

/// Multi-line human summary for `scribe status`.
pub fn human_status(s: &Snapshot) -> String {
    let mut out = String::new();
    let status = s.status.as_deref().unwrap_or("-");
    out.push_str(&format!("Scribe: {}", headline(s)));
    if !s.offline {
        out.push_str(&format!(" ({status})"));
    }
    out.push('\n');
    out.push_str(&format!("  dictating:  {}\n", s.dictating));
    out.push_str(&format!("  busy:       {}\n", s.busy));
    out.push_str(&format!("  status:     {status}\n"));
    if let Some(since) = &s.since {
        out.push_str(&format!("  since:      {since}\n"));
    }
    if let Some(updated) = &s.updated_at {
        out.push_str(&format!("  updatedAt:  {updated}\n"));
    }
    let app = if s.app.is_empty() { "scribe" } else { &s.app };
    let ver = s.app_version.as_deref().unwrap_or("?");
    let mut meta = format!("  app:        {app} {ver}");
    if let Some(pid) = s.pid {
        meta.push_str(&format!("  pid {pid}"));
    }
    out.push_str(&meta);
    out.push('\n');
    out
}

/// A single concise human line for one `watch` event.
pub fn human_watch_line(event: &str, s: &Snapshot) -> String {
    let ts = s.updated_at.as_deref().unwrap_or("-");
    let status = s.status.as_deref().unwrap_or("-");
    format!(
        "{ts}  {event:<18} status={status:<12} dictating={} busy={}",
        s.dictating, s.busy
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_line_is_single_line() {
        let v: Value = serde_json::json!({"a":1,"b":[1,2]});
        let line = json_line(&v);
        assert!(!line.contains('\n'));
        assert!(line.contains("\"a\":1"));
    }

    #[test]
    fn human_status_headlines_dictating() {
        let mut s = Snapshot::offline();
        s.offline = false;
        s.dictating = true;
        s.busy = true;
        s.status = Some("Recording".to_string());
        let text = human_status(&s);
        assert!(text.contains("Scribe: dictating (Recording)"));
        assert!(text.contains("dictating:  true"));
    }

    #[test]
    fn human_status_headlines_offline() {
        let s = Snapshot::offline();
        let text = human_status(&s);
        assert!(text.contains("Scribe: offline"));
        assert!(text.contains("busy:       false"));
    }

    #[test]
    fn watch_line_is_compact() {
        let mut s = Snapshot::offline();
        s.offline = false;
        s.dictating = true;
        s.busy = true;
        s.status = Some("Recording".to_string());
        s.updated_at = Some("2026-07-08T12:34:56.812Z".to_string());
        let line = human_watch_line("dictation.started", &s);
        assert!(line.contains("dictation.started"));
        assert!(line.contains("dictating=true"));
    }
}
