//! Discovery via `~/.scribe/control.json` (contract section 5).
//!
//! A consumer finds the running server by reading a well-known file in the
//! user's home directory. The Dev flavor writes `control.dev.json` so the two
//! never clobber each other.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The discovery-file `schemaVersion` this client understands.
pub const SUPPORTED_SCHEMA_VERSION: i64 = 1;

/// Which Scribe flavor to discover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channel {
    /// Normal Scribe: `~/.scribe/control.json`.
    #[default]
    Stable,
    /// Dev flavor (can run alongside stable): `~/.scribe/control.dev.json`.
    Dev,
}

impl Channel {
    /// The control-file name for this channel.
    pub fn control_file_name(self) -> &'static str {
        match self {
            Channel::Stable => "control.json",
            Channel::Dev => "control.dev.json",
        }
    }
}

/// The parsed discovery file.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlFile {
    #[serde(default)]
    pub schema_version: i64,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub app_version: Option<String>,
    #[serde(default)]
    pub pid: Option<i64>,
    #[serde(default)]
    pub transport: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub endpoints: Endpoints,
    pub read_token: String,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Logical-endpoint-name -> path map. Defaults to the v1 paths so a consumer
/// keeps working even if the map is somehow absent.
#[derive(Debug, Clone, Deserialize)]
pub struct Endpoints {
    #[serde(default = "default_status_path")]
    pub status: String,
    #[serde(default = "default_events_path")]
    pub events: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            status: default_status_path(),
            events: default_events_path(),
        }
    }
}

fn default_status_path() -> String {
    "/v1/status".to_string()
}

fn default_events_path() -> String {
    "/v1/events".to_string()
}

impl ControlFile {
    /// True if the discovery file's major version is newer than we understand.
    pub fn is_future_version(&self) -> bool {
        self.schema_version > SUPPORTED_SCHEMA_VERSION
    }

    /// Absolute URL for a logical endpoint (`baseUrl` + endpoint path), joined
    /// so exactly one `/` separates them regardless of stray slashes.
    pub fn endpoint_url(&self, path: &str) -> String {
        join_url(&self.base_url, path)
    }
}

/// Join a base origin and a path with exactly one separating slash.
pub fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}

/// Why discovery failed to yield a usable target.
#[derive(Debug)]
pub enum DiscoveryError {
    /// The control file does not exist -> Scribe not running.
    NotFound(PathBuf),
    /// The control file could not be read (permissions, etc.).
    Io(std::io::Error),
    /// The control file exists but is not valid JSON / is missing fields.
    Parse(serde_json::Error),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiscoveryError::NotFound(p) => write!(f, "control file not found: {}", p.display()),
            DiscoveryError::Io(e) => write!(f, "reading control file: {e}"),
            DiscoveryError::Parse(e) => write!(f, "control file is not valid: {e}"),
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// Read and parse a control file at an explicit path.
pub fn read_control_file(path: &Path) -> Result<ControlFile, DiscoveryError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(DiscoveryError::NotFound(path.to_path_buf()));
        }
        Err(e) => return Err(DiscoveryError::Io(e)),
    };
    serde_json::from_slice(&bytes).map_err(DiscoveryError::Parse)
}

/// Resolve the user's home directory the same way the contract describes:
/// `$HOME` on Unix, `%USERPROFILE%` on Windows, with the other as a fallback.
pub fn home_dir() -> Option<PathBuf> {
    for key in ["HOME", "USERPROFILE"] {
        if let Some(v) = std::env::var_os(key) {
            if !v.is_empty() {
                return Some(PathBuf::from(v));
            }
        }
    }
    None
}

/// The default control-file path for a channel: `<home>/.scribe/<name>`.
pub fn default_control_path(channel: Channel, home: &Path) -> PathBuf {
    home.join(".scribe").join(channel.control_file_name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_canonical_control_file() {
        let raw = r#"{
          "schemaVersion":1,"app":"scribe","appVersion":"0.7.0","pid":1234,
          "transport":"http-sse","baseUrl":"http://127.0.0.1:52431",
          "endpoints":{"status":"/v1/status","events":"/v1/events"},
          "readToken":"c3b54f25-7df6-4cfa-9ae2-342dd8d548c6",
          "updatedAt":"2026-07-08T12:34:56.000Z"
        }"#;
        let cf: ControlFile = serde_json::from_str(raw).unwrap();
        assert_eq!(cf.base_url, "http://127.0.0.1:52431");
        assert_eq!(cf.read_token, "c3b54f25-7df6-4cfa-9ae2-342dd8d548c6");
        assert_eq!(cf.pid, Some(1234));
        assert_eq!(
            cf.endpoint_url(&cf.endpoints.status),
            "http://127.0.0.1:52431/v1/status"
        );
        assert_eq!(
            cf.endpoint_url(&cf.endpoints.events),
            "http://127.0.0.1:52431/v1/events"
        );
        assert!(!cf.is_future_version());
    }

    #[test]
    fn endpoints_default_when_absent() {
        let raw = r#"{"baseUrl":"http://127.0.0.1:1","readToken":"t"}"#;
        let cf: ControlFile = serde_json::from_str(raw).unwrap();
        assert_eq!(cf.endpoints.status, "/v1/status");
        assert_eq!(cf.endpoints.events, "/v1/events");
    }

    #[test]
    fn tolerates_unknown_fields() {
        let raw = r#"{"baseUrl":"http://127.0.0.1:1","readToken":"t","futureField":true}"#;
        let cf: ControlFile = serde_json::from_str(raw).unwrap();
        assert_eq!(cf.read_token, "t");
    }

    #[test]
    fn join_url_normalizes_slashes() {
        assert_eq!(
            join_url("http://h:1/", "/v1/status"),
            "http://h:1/v1/status"
        );
        assert_eq!(join_url("http://h:1", "v1/status"), "http://h:1/v1/status");
        assert_eq!(join_url("http://h:1", "/v1/status"), "http://h:1/v1/status");
    }

    #[test]
    fn missing_file_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        match read_control_file(&path) {
            Err(DiscoveryError::NotFound(_)) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn unparseable_file_is_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.json");
        std::fs::write(&path, b"not json{").unwrap();
        match read_control_file(&path) {
            Err(DiscoveryError::Parse(_)) => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn channel_selects_control_file_name() {
        let home = Path::new("/home/u");
        assert_eq!(
            default_control_path(Channel::Stable, home),
            Path::new("/home/u/.scribe/control.json")
        );
        assert_eq!(
            default_control_path(Channel::Dev, home),
            Path::new("/home/u/.scribe/control.dev.json")
        );
    }
}
