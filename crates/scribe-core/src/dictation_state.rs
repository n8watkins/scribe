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

use std::sync::Arc;

use crate::{
    app_state::{AppStateSnapshot, AppStatus},
    backend::BackendState,
    core::Core,
    events::CoreEvent,
    host::Host,
    state_server::DictationServer,
};
use chrono::{DateTime, Utc};
use serde::Serialize;

/// Snapshot contract version. Bumped only on a breaking change to the snapshot
/// shape; see the wire contract's versioning rules.
pub const SCHEMA_VERSION: u32 = 1;

/// Constant producer name every payload carries, so a generic consumer can
/// confirm which app it reached.
pub const APP_NAME: &str = "scribe";

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
        AppStatus::Recording | AppStatus::Stopping | AppStatus::Transcribing | AppStatus::Pasting
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
    /// The running app's version, as its host reports it
    /// ([`Host::app_version`]). The same in every transport (this snapshot,
    /// the status file, the discovery file).
    pub app_version: String,
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
    /// becomes `since`. `app_version` is the host's [`Host::app_version`].
    pub fn from_state(
        state: &AppStateSnapshot,
        now: DateTime<Utc>,
        pid: u32,
        app_version: &str,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            app: APP_NAME,
            app_version: app_version.to_string(),
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
/// Every state transition (`audio`, `dictation`, `output`) reaches here, which
/// keeps the three side effects in one place so they cannot drift. All three are
/// best-effort and must never fail the caller, so a status consumer can never
/// disrupt dictation:
///   1. emit the `scribe:app-state` Tauri event to the webview,
///   2. mirror the snapshot to the on-disk status file (the file fallback),
///   3. broadcast it to the in-process HTTP+SSE server, if it is running.
pub fn emit_state_snapshot_with(core: &Core, snapshot: &AppStateSnapshot) {
    Parts::of(core).publish(snapshot);
}

/// The parts of the engine the audio lane's recording and dictation paths
/// use: the backend state, the host, and the dictation-state server. Built
/// from a [`Core`] in the app, and from the bare parts in tests (which cannot
/// build a `Core`). Each is a cheap handle, so a thread that outlives the
/// call can take owned clones ([`Parts::to_owned`]).
#[derive(Clone, Copy)]
pub(crate) struct Parts<'a> {
    pub backend: &'a BackendState,
    pub host: &'a Arc<dyn Host>,
    pub server: Option<&'a DictationServer>,
}

impl<'a> Parts<'a> {
    pub fn of(core: &'a Core) -> Self {
        Self {
            backend: core.backend(),
            host: core.host(),
            server: core.dictation_server(),
        }
    }

    /// Fans a fresh state snapshot out: the `scribe:app-state` event, the
    /// status file, and the dictation-state server when it runs.
    pub fn publish(&self, snapshot: &AppStateSnapshot) {
        publish_snapshot(self.host.as_ref(), self.server, snapshot);
    }

    /// Owned clones of the parts, for a thread that outlives the call.
    pub fn to_owned(self) -> OwnedParts {
        OwnedParts {
            backend: self.backend.clone(),
            host: self.host.clone(),
            server: self.server.cloned(),
        }
    }
}

/// [`Parts`] a thread can own.
pub(crate) struct OwnedParts {
    pub backend: BackendState,
    pub host: Arc<dyn Host>,
    pub server: Option<DictationServer>,
}

impl OwnedParts {
    pub fn parts(&self) -> Parts<'_> {
        Parts {
            backend: &self.backend,
            host: &self.host,
            server: self.server.as_ref(),
        }
    }
}

/// The three side effects of a new state, in their order: the event to the
/// interface, the status file, the dictation-state server.
fn publish_snapshot(
    host: &dyn Host,
    server: Option<&DictationServer>,
    snapshot: &AppStateSnapshot,
) {
    host.emit(CoreEvent::AppState(snapshot.clone()));
    crate::status_file::publish_with(host, snapshot);
    if let Some(server) = server {
        server.publish(snapshot);
    }
}

/// [`Parts`] for the audio lane's tests, which cannot build a `Core`: a
/// backend on an in-memory database, a [`crate::host::RecordingHost`] on a
/// temporary folder, and no dictation-state server. The folder is removed on
/// drop.
#[cfg(test)]
pub(crate) mod test_parts {
    use std::{path::PathBuf, sync::Arc};

    use super::Parts;
    use crate::{
        app_state::{AppEvent, AppStatus},
        backend::BackendState,
        host::{Flavor, Host, RecordingHost},
        paths::Paths,
    };

    pub(crate) struct Fixture {
        pub root: PathBuf,
        pub backend: BackendState,
        pub recorder: Arc<RecordingHost>,
        pub host: Arc<dyn Host>,
    }

    impl Fixture {
        pub fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "scribe-audio-lane-test-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            let backend = BackendState::new(
                crate::db::Database::in_memory().unwrap(),
                root.join("recordings"),
                crate::data_hygiene::DataDirs {
                    data_dir: root.join("data"),
                    cache_dir: root.join("cache"),
                },
            );
            let recorder = Arc::new(RecordingHost::new(
                Paths::from_parts(
                    root.join("data"),
                    root.join("cache"),
                    root.join("logs"),
                    None,
                    None,
                ),
                Flavor::Stable,
            ));
            let host: Arc<dyn Host> = recorder.clone();
            Self {
                root,
                backend,
                recorder,
                host,
            }
        }

        pub fn parts(&self) -> Parts<'_> {
            Parts {
                backend: &self.backend,
                host: &self.host,
                server: None,
            }
        }

        /// Drives the state machine through `events` without sending anything.
        pub fn drive(&self, events: &[AppEvent]) {
            for event in events {
                self.backend.transition_app_state(event.clone()).unwrap();
            }
        }

        pub fn status(&self) -> AppStatus {
            self.backend.app_state().unwrap().snapshot().status
        }

        /// The (name, JSON payload) of every event sent so far.
        pub fn sent(&self) -> Vec<(&'static str, serde_json::Value)> {
            self.recorder
                .events()
                .iter()
                .map(|event| (event.name(), serde_json::to_value(event).unwrap()))
                .collect()
        }

        /// The statuses of the `scribe:app-state` events sent so far.
        pub fn published_states(&self) -> Vec<String> {
            self.sent()
                .into_iter()
                .filter(|(name, _)| *name == "scribe:app-state")
                .map(|(_, payload)| payload["status"].as_str().unwrap().to_string())
                .collect()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
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
            assert!(
                !is_dictating(&status),
                "{:?} was not legacy listening",
                status
            );
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
        let snap = DictationSnapshot::from_state(&state, now, 4242, "0.16.4-57");
        let value = serde_json::to_value(&snap).unwrap();

        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["app"], "scribe");
        assert_eq!(value["appVersion"], "0.16.4-57");
        assert_eq!(value["status"], "Recording");
        assert_eq!(value["dictating"], true);
        assert_eq!(value["busy"], true);
        assert_eq!(value["pid"], 4242);
        // camelCase timestamps, and `since` tracks the state's own updated_at
        // while `updatedAt` is the snapshot production time.
        assert!(value.get("since").is_some());
        assert!(
            value.get("updatedAt").is_some(),
            "updatedAt must be camelCase"
        );
        assert!(
            value.get("updated_at").is_none(),
            "snake_case must not leak"
        );
    }

    #[test]
    fn idle_snapshot_is_not_dictating_or_busy() {
        let state = AppStateSnapshot {
            status: AppStatus::Idle,
            error: None,
            updated_at: Utc::now(),
        };
        let snap = DictationSnapshot::from_state(&state, Utc::now(), 1, "0.16.3");
        assert!(!snap.dictating);
        assert!(!snap.busy);
    }

    #[test]
    fn a_new_state_goes_to_the_interface_then_the_status_file() {
        use crate::host::{Flavor, RecordingHost};
        use crate::paths::Paths;

        let root = std::env::temp_dir().join(format!(
            "scribe-dictation-state-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let cache = root.join("cache");
        let host = RecordingHost::new(
            Paths::from_parts(
                root.join("data"),
                cache.clone(),
                root.join("logs"),
                None,
                None,
            ),
            Flavor::Stable,
        )
        .with_app_version("0.16.4-57");
        let state = AppStateSnapshot {
            status: AppStatus::Recording,
            error: None,
            updated_at: Utc::now(),
        };

        publish_snapshot(&host, None, &state);

        let events = host.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name(), "scribe:app-state");
        assert_eq!(
            serde_json::to_value(&events[0]).unwrap(),
            serde_json::to_value(&state).unwrap()
        );
        let status = std::fs::read_to_string(cache.join("status.json")).unwrap();
        assert!(status.contains("\"Recording\""), "{status}");
        // The status file carries the host's version; reading it is a query,
        // so the host records no call.
        let status: serde_json::Value = serde_json::from_str(&status).unwrap();
        assert_eq!(status["appVersion"], "0.16.4-57");
        assert!(host.calls().is_empty());

        std::fs::remove_dir_all(&root).unwrap();
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
