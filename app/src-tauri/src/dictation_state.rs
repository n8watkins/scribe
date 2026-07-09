//! The single, authoritative dictation-state model.
//!
//! This module owns the mapping from the internal `AppStatus` to the two
//! consumer-facing booleans - `dictating` (mic capturing) and `busy` (anywhere
//! in the capture-to-insert cycle) - and the wire snapshot shape. Both the
//! HTTP+SSE server (`state_server`) and the on-disk status file (`status_file`)
//! derive their view from here, so the "what counts as dictating" decision
//! lives in exactly one place and cannot drift between transports.
//!
//! The wire contract these types implement is frozen in
//! `docs/integrations/dictation-state-contract.md`.

use crate::app_state::{AppStatus, AppStateSnapshot};
use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Snapshot contract version. Bumped only on a breaking change to the snapshot
/// shape; see the wire contract's versioning rules.
pub const SCHEMA_VERSION: u32 = 1;

/// Constant producer name every payload carries, so a generic consumer can
/// confirm which app it reached.
pub const APP_NAME: &str = "scribe";

/// Scribe's semver, baked in at compile time from `Cargo.toml`. Used verbatim
/// as the `appVersion` field across every transport.
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Narrow flag: the microphone is actively capturing the user's voice.
///
/// True for `Recording`, and briefly for `Stopping` while the audio tail
/// flushes and the user may still be finishing a word. This is exactly the
/// legacy `listening` predicate, renamed.
pub fn is_dictating(status: &AppStatus) -> bool {
    matches!(status, AppStatus::Recording | AppStatus::Stopping)
}

/// Broad flag: the user is inside a dictation cycle and external output would
/// collide.
///
/// A superset of [`is_dictating`]: it additionally covers `Transcribing`
/// (Whisper is producing the text the user is waiting for) and `Pasting`
/// (Scribe is inserting text into the focused app). A "do not talk over me"
/// consumer holds through `busy`; a "show the mic state" consumer watches
/// `dictating`.
pub fn is_busy(status: &AppStatus) -> bool {
    matches!(
        status,
        AppStatus::Recording
            | AppStatus::Stopping
            | AppStatus::Transcribing
            | AppStatus::Pasting
    )
}

/// The full wire snapshot - the object returned by the point-in-time query and
/// carried as the payload of every stream event. `rename_all = "camelCase"`
/// yields the exact JSON keys the contract pins (`schemaVersion`, `appVersion`,
/// `updatedAt`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationSnapshot {
    pub schema_version: u32,
    pub app: &'static str,
    pub app_version: &'static str,
    /// Raw internal status, PascalCase, the consumer escape hatch.
    pub status: AppStatus,
    pub dictating: bool,
    pub busy: bool,
    /// When the current `status` was entered (the state machine's `updated_at`).
    pub since: DateTime<Utc>,
    /// When this snapshot was produced; refreshed on a heartbeat, not only on
    /// transitions.
    pub updated_at: DateTime<Utc>,
    pub pid: u32,
}

impl DictationSnapshot {
    /// Project a state-machine snapshot onto the wire shape. `now` is this
    /// snapshot's production time (`updatedAt`); the state's own `updated_at`
    /// becomes `since`.
    pub fn from_state(state: &AppStateSnapshot, now: DateTime<Utc>, pid: u32) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            app: APP_NAME,
            app_version: app_version(),
            status: state.status.clone(),
            dictating: is_dictating(&state.status),
            busy: is_busy(&state.status),
            since: state.updated_at,
            updated_at: now,
            pid,
        }
    }
}

/// The single fan-out point for a fresh state snapshot.
///
/// Every state transition reaches here through the per-module
/// `emit_state_snapshot` shims (`audio`, `dictation`, `output`), which keeps the
/// three side effects in one place so they cannot drift. All three are
/// best-effort and must never fail the caller, so a status consumer can never
/// disrupt dictation:
///   1. emit the `scribe:app-state` Tauri event to the webview,
///   2. mirror the snapshot to the on-disk status file (the file fallback),
///   3. broadcast it to the in-process HTTP+SSE server, if it is running.
pub fn emit_state_snapshot(app: &AppHandle, snapshot: &AppStateSnapshot) {
    let _ = app.emit("scribe:app-state", snapshot);
    crate::status_file::publish(app, snapshot);
    if let Some(server) = app.try_state::<crate::state_server::DictationServer>() {
        server.publish(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `AppStatus`, paired with its expected (dictating, busy). This table
    /// is the executable form of the contract's mapping table; if a variant is
    /// added, the exhaustive match below forces this list to be updated.
    fn expectations() -> Vec<(AppStatus, bool, bool)> {
        use AppStatus::*;
        let table = vec![
            (Idle, false, false),
            (Recording, true, true),
            (Stopping, true, true),
            (Transcribing, false, true),
            (Pasting, false, true),
            (Ready, false, false),
            (Error, false, false),
            (Paused, false, false),
        ];
        // Exhaustiveness guard: force a compile error here if a variant is added
        // without extending the table above.
        for (status, _, _) in &table {
            match status {
                Idle | Recording | Stopping | Transcribing | Pasting | Ready | Error | Paused => {}
            }
        }
        table
    }

    #[test]
    fn dictating_and_busy_match_the_contract_table() {
        for (status, dictating, busy) in expectations() {
            assert_eq!(
                is_dictating(&status),
                dictating,
                "dictating wrong for {:?}",
                status
            );
            assert_eq!(is_busy(&status), busy, "busy wrong for {:?}", status);
        }
    }

    #[test]
    fn dictating_implies_busy_for_every_status() {
        for (status, _, _) in expectations() {
            if is_dictating(&status) {
                assert!(
                    is_busy(&status),
                    "invariant dictating => busy broken for {:?}",
                    status
                );
            }
        }
    }

    #[test]
    fn dictating_matches_legacy_listening_set() {
        // The legacy `listening` was exactly Recording | Stopping. `dictating`
        // must be a faithful rename so file-fallback readers keep working.
        assert!(is_dictating(&AppStatus::Recording));
        assert!(is_dictating(&AppStatus::Stopping));
        for status in [
            AppStatus::Idle,
            AppStatus::Transcribing,
            AppStatus::Pasting,
            AppStatus::Ready,
            AppStatus::Error,
            AppStatus::Paused,
        ] {
            assert!(!is_dictating(&status), "{:?} was not legacy listening", status);
        }
    }

    #[test]
    fn snapshot_serializes_with_contract_keys_and_types() {
        let state = AppStateSnapshot {
            status: AppStatus::Recording,
            error: None,
            updated_at: Utc::now(),
        };
        let now = Utc::now();
        let snap = DictationSnapshot::from_state(&state, now, 4242);
        let value = serde_json::to_value(&snap).unwrap();

        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["app"], "scribe");
        assert_eq!(value["appVersion"], app_version());
        assert_eq!(value["status"], "Recording");
        assert_eq!(value["dictating"], true);
        assert_eq!(value["busy"], true);
        assert_eq!(value["pid"], 4242);
        // camelCase timestamps, and `since` tracks the state's own updated_at
        // while `updatedAt` is the snapshot production time.
        assert!(value.get("since").is_some());
        assert!(value.get("updatedAt").is_some(), "updatedAt must be camelCase");
        assert!(value.get("updated_at").is_none(), "snake_case must not leak");
    }

    #[test]
    fn idle_snapshot_is_not_dictating_or_busy() {
        let state = AppStateSnapshot {
            status: AppStatus::Idle,
            error: None,
            updated_at: Utc::now(),
        };
        let snap = DictationSnapshot::from_state(&state, Utc::now(), 1);
        assert!(!snap.dictating);
        assert!(!snap.busy);
    }

    #[test]
    fn terminal_and_inactive_states_never_report_activity() {
        // The producer-side floor under "stale-or-dead always resolves to
        // not-dictating": the statuses a finished, reset, paused, or
        // just-shut-down (Idle) producer reports must never claim dictating or
        // busy, so a consumer reading a last-gasp snapshot cannot be trapped in
        // a permanent "hold your output" belief.
        for status in [
            AppStatus::Idle,
            AppStatus::Ready,
            AppStatus::Error,
            AppStatus::Paused,
        ] {
            assert!(!is_dictating(&status), "{:?} must not be dictating", status);
            assert!(!is_busy(&status), "{:?} must not be busy", status);
        }
    }
}
