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
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Manager};

/// The on-disk payload. `rename_all = "camelCase"` keeps the JSON keys aligned
/// with the rest of Scribe's IPC surface (e.g. `updatedAt`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusFile {
    /// Current state-machine status, serialized exactly as the webview sees it
    /// (PascalCase: "Idle", "Recording", ...).
    status: AppStatus,
    /// True only while the user is actively talking. Deriving this here - in
    /// the app that owns the state machine - keeps the what-counts-as-talking
    /// decision out of the reader.
    listening: bool,
    /// When the current status was entered (the state machine's `updated_at`).
    since: DateTime<Utc>,
    /// When this file was last written. A reader compares this to wall-clock
    /// time to spot a stale file left behind by a crashed Scribe.
    updated_at: DateTime<Utc>,
    /// The writing process, so a reader can confirm Scribe is still alive.
    pid: u32,
}

/// Scribe is "listening" - the user is actively talking - only while the state
/// machine is Recording, or briefly Stopping as the audio stream flushes.
fn is_listening(status: &AppStatus) -> bool {
    matches!(status, AppStatus::Recording | AppStatus::Stopping)
}

/// Build the on-disk payload from a state snapshot. `now` is the file's write
/// time; `since` comes from the snapshot's `updated_at`.
fn build(snapshot: &AppStateSnapshot, now: DateTime<Utc>, pid: u32) -> StatusFile {
    StatusFile {
        status: snapshot.status.clone(),
        listening: is_listening(&snapshot.status),
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
    fn listening_is_true_only_while_talking() {
        assert!(is_listening(&AppStatus::Recording));
        assert!(is_listening(&AppStatus::Stopping));
        for status in [
            AppStatus::Idle,
            AppStatus::Transcribing,
            AppStatus::Pasting,
            AppStatus::Ready,
            AppStatus::Error,
            AppStatus::Paused,
        ] {
            assert!(
                !is_listening(&status),
                "{:?} must not count as listening",
                status
            );
        }
    }

    #[test]
    fn build_derives_listening_and_carries_since() {
        let snap = snapshot(AppStatus::Recording);
        let now = Utc::now();
        let payload = build(&snap, now, 4242);
        assert!(payload.listening);
        assert_eq!(payload.since, snap.updated_at);
        assert_eq!(payload.updated_at, now);
        assert_eq!(payload.pid, 4242);
    }

    #[test]
    fn build_marks_idle_not_listening() {
        let payload = build(&snapshot(AppStatus::Idle), Utc::now(), 1);
        assert!(!payload.listening);
    }

    #[test]
    fn payload_serializes_with_camelcase_keys() {
        let payload = build(&snapshot(AppStatus::Recording), Utc::now(), 7);
        let value: serde_json::Value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["status"], "Recording");
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
        assert_eq!(value["listening"], true);

        // A successful write leaves no staging file behind.
        let staging = dir.join(format!("status.json.{}.tmp", payload.pid));
        assert!(!staging.exists(), "temp file must be renamed away");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
