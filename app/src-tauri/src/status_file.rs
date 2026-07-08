//! Publishes Scribe's dictation state to a small on-disk status file so a
//! second app (T-Hub) can tell when the user is actively talking and hold its
//! own voice announcements until they stop.
//!
//! The file lives at `app_cache_dir()/status.json`. Using the cache dir gives
//! the dev/prod split for free: the Scribe Dev flavor writes under its own
//! `.dev` identifier, so stable and dev never clobber each other's status.
//!
//! Writes are atomic (stage to a temp file, then rename over the target), the
//! same discipline the reader (T-Hub) uses, so a reader never observes a
//! half-written or truncated payload.

use crate::app_state::{AppStatus, AppStateSnapshot};
use crate::dictation_state::{self, SCHEMA_VERSION};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Manager};

/// The on-disk payload. `rename_all = "camelCase"` keeps the JSON keys aligned
/// with the rest of Scribe's IPC surface (e.g. `updatedAt`).
///
/// This is a superset of the canonical snapshot in the dictation-state wire
/// contract; the derivation of `dictating` / `busy` lives in `dictation_state`
/// so this file and the HTTP transport cannot disagree.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusFile {
    /// File-format version, matching the snapshot `schemaVersion` in the wire
    /// contract.
    schema_version: u32,
    /// Constant `"scribe"`, so a reader can confirm which producer wrote this.
    app: &'static str,
    /// Scribe's semver, for reader diagnostics / feature-gating.
    app_version: &'static str,
    /// Current state-machine status, serialized exactly as the webview sees it
    /// (PascalCase: "Idle", "Recording", ...).
    status: AppStatus,
    /// Narrow flag: the microphone is actively capturing (mirrors the HTTP
    /// snapshot's `dictating`). Derived once, in `dictation_state`.
    dictating: bool,
    /// Broad flag: the user is anywhere inside the capture-to-insert cycle.
    busy: bool,
    /// DEPRECATED alias of `dictating`, kept for one release so existing readers
    /// (T-Hub) keep working while they migrate. Remove in a future version.
    listening: bool,
    /// When the current status was entered (the state machine's `updated_at`).
    since: DateTime<Utc>,
    /// When this file was last written. A reader compares this to wall-clock
    /// time to spot a stale file left behind by a crashed Scribe.
    updated_at: DateTime<Utc>,
    /// The writing process, so a reader can confirm Scribe is still alive.
    pid: u32,
}

/// Build the on-disk payload from a state snapshot. `now` is the file's write
/// time; `since` comes from the snapshot's `updated_at`.
fn build(snapshot: &AppStateSnapshot, now: DateTime<Utc>, pid: u32) -> StatusFile {
    let dictating = dictation_state::is_dictating(&snapshot.status);
    StatusFile {
        schema_version: SCHEMA_VERSION,
        app: dictation_state::APP_NAME,
        app_version: dictation_state::app_version(),
        status: snapshot.status.clone(),
        dictating,
        busy: dictation_state::is_busy(&snapshot.status),
        listening: dictating,
        since: snapshot.updated_at,
        updated_at: now,
        pid,
    }
}

/// Atomically write `payload` as JSON to `dir/status.json`: stage to a sibling
/// temp file, then rename over the target so a reader never sees a partial
/// write. `dir` is assumed to already exist.
fn write_atomic(dir: &Path, payload: &StatusFile) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(payload)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let target = dir.join("status.json");
    // The temp file carries the pid so two flavors (or a stale run) can never
    // fight over the same staging path mid-write.
    let staging = dir.join(format!("status.json.{}.tmp", payload.pid));
    std::fs::write(&staging, &json)?;
    match std::fs::rename(&staging, &target) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&staging);
            Err(error)
        }
    }
}

/// Publish a state snapshot to the status file. Best-effort: a failure is
/// logged and swallowed, never propagated, so status publishing can never
/// disrupt dictation. Called next to every `emit_state_snapshot`.
pub fn publish(app: &AppHandle, snapshot: &AppStateSnapshot) {
    let dir = match app.path().app_cache_dir() {
        Ok(dir) => dir,
        Err(error) => {
            log::warn!("Could not resolve cache dir for status file: {}", error);
            return;
        }
    };
    // The cache dir is created during setup(), but a first write on a fresh
    // profile (or after a manual wipe) should still succeed.
    if let Err(error) = std::fs::create_dir_all(&dir) {
        log::warn!("Could not create cache dir for status file: {}", error);
        return;
    }
    let payload = build(snapshot, Utc::now(), std::process::id());
    if let Err(error) = write_atomic(&dir, &payload) {
        log::warn!("Could not write status file: {}", error);
    }
}

/// Publish a fresh `Idle` snapshot. Used at startup (so the file exists before
/// the first real transition) and at shutdown (so a reader sees Scribe is no
/// longer listening).
pub fn publish_idle(app: &AppHandle) {
    let snapshot = AppStateSnapshot {
        status: AppStatus::Idle,
        error: None,
        updated_at: Utc::now(),
    };
    publish(app, &snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(status: AppStatus) -> AppStateSnapshot {
        AppStateSnapshot {
            status,
            error: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn build_derives_flags_and_carries_since() {
        let snap = snapshot(AppStatus::Recording);
        let now = Utc::now();
        let payload = build(&snap, now, 4242);
        assert!(payload.dictating);
        assert!(payload.busy);
        // `listening` stays a faithful alias of `dictating` for one release.
        assert_eq!(payload.listening, payload.dictating);
        assert_eq!(payload.since, snap.updated_at);
        assert_eq!(payload.updated_at, now);
        assert_eq!(payload.pid, 4242);
    }

    #[test]
    fn build_marks_idle_not_dictating_or_busy() {
        let payload = build(&snapshot(AppStatus::Idle), Utc::now(), 1);
        assert!(!payload.dictating);
        assert!(!payload.busy);
        assert!(!payload.listening);
    }

    #[test]
    fn build_marks_transcribing_busy_but_not_dictating() {
        // The broad flag must cover the whole cycle: mid-transcription, the mic
        // is closed but output would still collide with the pending result.
        let payload = build(&snapshot(AppStatus::Transcribing), Utc::now(), 1);
        assert!(!payload.dictating);
        assert!(payload.busy);
        assert!(!payload.listening);
    }

    #[test]
    fn payload_serializes_with_camelcase_keys() {
        let payload = build(&snapshot(AppStatus::Recording), Utc::now(), 7);
        let value: serde_json::Value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["app"], "scribe");
        assert!(value.get("appVersion").is_some(), "appVersion must be present");
        assert_eq!(value["status"], "Recording");
        assert_eq!(value["dictating"], true);
        assert_eq!(value["busy"], true);
        assert_eq!(value["listening"], true);
        assert!(value.get("since").is_some());
        assert!(
            value.get("updatedAt").is_some(),
            "updatedAt must be camelCase"
        );
        assert_eq!(value["pid"], 7);
    }

    #[test]
    fn write_atomic_writes_valid_json_and_leaves_no_temp() {
        let dir =
            std::env::temp_dir().join(format!("scribe-status-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let payload = build(&snapshot(AppStatus::Stopping), Utc::now(), std::process::id());
        write_atomic(&dir, &payload).unwrap();

        let contents = std::fs::read_to_string(dir.join("status.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(value["status"], "Stopping");
        assert_eq!(value["dictating"], true);
        assert_eq!(value["listening"], true);

        // A successful write leaves no staging file behind.
        let staging = dir.join(format!("status.json.{}.tmp", payload.pid));
        assert!(!staging.exists(), "temp file must be renamed away");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
