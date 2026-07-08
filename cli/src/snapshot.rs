//! The core state snapshot (contract section 1).
//!
//! One JSON object is returned by the point-in-time query and carried as the
//! payload of every SSE event. This module models it leniently: unknown and
//! additive fields are ignored (never a decode failure), per the contract's
//! forward-compatibility rule.

use serde::{Deserialize, Serialize};

/// The snapshot `schemaVersion` this client was written against.
pub const SUPPORTED_SCHEMA_VERSION: i64 = 1;

/// A decoded state snapshot.
///
/// Consumers are told to prefer the two booleans (`dictating`, `busy`) over
/// switching on the raw `status` string, because Scribe always computes the
/// booleans correctly - even for a status variant added in a future version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(default = "default_schema_version")]
    pub schema_version: i64,
    #[serde(default)]
    pub app: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    /// Raw internal status, PascalCase (one of the eight contract variants, but
    /// the set is NOT closed - never assume it is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Narrow flag: the microphone is actively capturing the user's voice.
    pub dictating: bool,
    /// Broad flag: inside a dictation cycle; external output would collide.
    /// Superset of `dictating`.
    pub busy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i64>,
    /// Client-side marker: this snapshot was synthesized locally because Scribe
    /// is offline/unreachable (not part of the wire contract - additive, and
    /// downstream consumers that gate on the booleans simply see false/false).
    #[serde(default, skip_serializing_if = "is_false")]
    pub offline: bool,
}

fn default_schema_version() -> i64 {
    SUPPORTED_SCHEMA_VERSION
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Snapshot {
    /// The canonical "Scribe is not reachable" snapshot. Stale-or-dead ALWAYS
    /// resolves to not-dictating (contract section 7), so this is what a thin
    /// client reports whenever discovery is void or the connection is refused.
    pub fn offline() -> Self {
        Snapshot {
            schema_version: SUPPORTED_SCHEMA_VERSION,
            app: "scribe".to_string(),
            app_version: None,
            status: Some("Offline".to_string()),
            dictating: false,
            busy: false,
            since: None,
            updated_at: Some(now_iso8601()),
            pid: None,
            offline: true,
        }
    }

    /// True if the payload's major version is newer than we understand. The
    /// contract says a consumer SHOULD reject such a payload.
    pub fn is_future_version(&self) -> bool {
        self.schema_version > SUPPORTED_SCHEMA_VERSION
    }
}

/// Current time as an ISO-8601 UTC string with millisecond precision and a `Z`
/// suffix - matching the contract's timestamp format for `updatedAt`.
pub fn now_iso8601() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_full_recording_snapshot() {
        let raw = r#"{
            "schemaVersion":1,"app":"scribe","appVersion":"0.7.0",
            "status":"Recording","dictating":true,"busy":true,
            "since":"2026-07-08T12:34:56.789Z","updatedAt":"2026-07-08T12:34:56.812Z",
            "pid":1234
        }"#;
        let s: Snapshot = serde_json::from_str(raw).unwrap();
        assert_eq!(s.schema_version, 1);
        assert_eq!(s.app, "scribe");
        assert_eq!(s.status.as_deref(), Some("Recording"));
        assert!(s.dictating);
        assert!(s.busy);
        assert_eq!(s.pid, Some(1234));
        assert!(!s.offline);
    }

    #[test]
    fn tolerates_unknown_and_deprecated_fields() {
        // `listening` is the deprecated file-fallback alias; a future additive
        // field must not break decode either.
        let raw = r#"{"schemaVersion":1,"app":"scribe","status":"Recording",
            "dictating":true,"busy":true,"listening":true,"somethingNew":42}"#;
        let s: Snapshot = serde_json::from_str(raw).unwrap();
        assert!(s.dictating);
        assert!(s.busy);
    }

    #[test]
    fn flags_a_future_major_version() {
        let raw = r#"{"schemaVersion":2,"app":"scribe","status":"Recording",
            "dictating":true,"busy":true}"#;
        let s: Snapshot = serde_json::from_str(raw).unwrap();
        assert!(s.is_future_version());
    }

    #[test]
    fn offline_snapshot_is_not_dictating() {
        let s = Snapshot::offline();
        assert!(!s.dictating);
        assert!(!s.busy);
        assert!(s.offline);
        // Re-serializes with the offline marker and camelCase keys.
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["dictating"], false);
        assert_eq!(v["busy"], false);
        assert_eq!(v["offline"], true);
        assert_eq!(v["status"], "Offline");
    }

    #[test]
    fn now_iso8601_has_millis_and_z() {
        let t = now_iso8601();
        assert!(t.ends_with('Z'), "got {t}");
        // e.g. 2026-07-08T12:34:56.812Z
        assert_eq!(t.len(), "2026-07-08T12:34:56.812Z".len(), "got {t}");
    }
}
