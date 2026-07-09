//! Turning discovery + liveness into a decision: either an online [`Target`] we
//! can connect to, or an [`Offline`] reason (which always resolves to
//! not-dictating - contract section 7).

use std::path::PathBuf;

use crate::discovery::{self, Channel, ControlFile, DiscoveryError};
use crate::liveness;

/// Inputs to resolution, gathered from CLI flags and the environment.
#[derive(Debug, Clone, Default)]
pub struct ResolveOptions {
    /// Explicit `baseUrl` override; when set, discovery is bypassed entirely.
    pub base_url: Option<String>,
    /// Read token to use with an explicit `base_url` (or to override discovery).
    pub token: Option<String>,
    /// Explicit control-file path; overrides the channel default.
    pub control_path: Option<PathBuf>,
    /// Which flavor's control file to read when no explicit path is given.
    pub channel: Channel,
    /// Whether to pid-check the discovered process (default on).
    pub pid_check: bool,
}

/// A concrete server we can talk to.
#[derive(Debug, Clone)]
pub struct Target {
    pub base_url: String,
    pub token: Option<String>,
    pub status_url: String,
    pub events_url: String,
    /// Scribe's pid, when known from discovery (for diagnostics).
    pub pid: Option<i64>,
    /// Discovered app version, when known.
    pub app_version: Option<String>,
}

/// Why we consider Scribe offline. Every variant means not-dictating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offline {
    /// No control file -> Scribe not running.
    NoControlFile,
    /// Control file present but unreadable/unparseable.
    UnreadableControlFile(String),
    /// Discovery file's major version is newer than we understand.
    UnsupportedControlVersion(i64),
    /// The discovered pid is not alive -> stale file, Scribe crashed.
    ProcessDead(i64),
    /// No home directory could be resolved and no explicit path was given.
    NoHomeDirectory,
}

impl Offline {
    /// A short human phrase for the reason.
    pub fn reason(&self) -> String {
        match self {
            Offline::NoControlFile => "not running (no control file)".to_string(),
            Offline::UnreadableControlFile(e) => format!("control file unreadable ({e})"),
            Offline::UnsupportedControlVersion(v) => {
                format!("control file schemaVersion {v} is newer than supported")
            }
            Offline::ProcessDead(pid) => format!("process {pid} is not alive (stale file)"),
            Offline::NoHomeDirectory => "no home directory to locate control file".to_string(),
        }
    }
}

/// The outcome of resolving where/whether to connect.
#[derive(Debug, Clone)]
pub enum Resolution {
    Online(Target),
    Offline(Offline),
}

impl Target {
    fn from_control(cf: &ControlFile, token_override: Option<String>) -> Self {
        Target {
            base_url: cf.base_url.clone(),
            token: token_override.or_else(|| Some(cf.read_token.clone())),
            status_url: cf.endpoint_url(&cf.endpoints.status),
            events_url: cf.endpoint_url(&cf.endpoints.events),
            pid: cf.pid,
            app_version: cf.app_version.clone(),
        }
    }
}

/// Resolve a [`Resolution`] from options. Pure aside from reading the control
/// file and (optionally) checking pid liveness, so it is straightforward to
/// unit-test by pointing `control_path` at a fixture.
pub fn resolve(opts: &ResolveOptions) -> Resolution {
    // 1. An explicit base URL bypasses discovery (power users / tests).
    if let Some(base) = &opts.base_url {
        return Resolution::Online(Target {
            base_url: base.clone(),
            token: opts.token.clone(),
            status_url: discovery::join_url(base, "/v1/status"),
            events_url: discovery::join_url(base, "/v1/events"),
            pid: None,
            app_version: None,
        });
    }

    // 2. Locate the control file.
    let path = match &opts.control_path {
        Some(p) => p.clone(),
        None => match discovery::home_dir() {
            Some(home) => discovery::default_control_path(opts.channel, &home),
            None => return Resolution::Offline(Offline::NoHomeDirectory),
        },
    };

    // 3. Read + parse it.
    let cf = match discovery::read_control_file(&path) {
        Ok(cf) => cf,
        Err(DiscoveryError::NotFound(_)) => return Resolution::Offline(Offline::NoControlFile),
        Err(e) => return Resolution::Offline(Offline::UnreadableControlFile(e.to_string())),
    };

    // 4. Reject a discovery file we cannot understand.
    if cf.is_future_version() {
        return Resolution::Offline(Offline::UnsupportedControlVersion(cf.schema_version));
    }

    // 5. Optional pid liveness: a dead process voids the file.
    if opts.pid_check {
        if let Some(pid) = cf.pid {
            if !liveness::pid_alive(pid) {
                return Resolution::Offline(Offline::ProcessDead(pid));
            }
        }
    }

    Resolution::Online(Target::from_control(&cf, opts.token.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_control(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    const CANONICAL: &str = r#"{
      "schemaVersion":1,"app":"scribe","appVersion":"0.7.0","pid":PIDPLACEHOLDER,
      "transport":"http-sse","baseUrl":"http://127.0.0.1:52431",
      "endpoints":{"status":"/v1/status","events":"/v1/events"},
      "readToken":"tok-123","updatedAt":"2026-07-08T12:34:56.000Z"
    }"#;

    #[test]
    fn missing_control_file_is_offline() {
        let dir = tempfile::tempdir().unwrap();
        let opts = ResolveOptions {
            control_path: Some(dir.path().join("control.json")),
            pid_check: true,
            ..Default::default()
        };
        assert!(matches!(
            resolve(&opts),
            Resolution::Offline(Offline::NoControlFile)
        ));
    }

    #[test]
    fn unparseable_control_file_is_offline() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_control(dir.path(), "control.json", "garbage{");
        let opts = ResolveOptions {
            control_path: Some(path),
            pid_check: true,
            ..Default::default()
        };
        assert!(matches!(
            resolve(&opts),
            Resolution::Offline(Offline::UnreadableControlFile(_))
        ));
    }

    #[test]
    fn live_pid_resolves_online() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        let body = CANONICAL.replace("PIDPLACEHOLDER", &me.to_string());
        let path = write_control(dir.path(), "control.json", &body);
        let opts = ResolveOptions {
            control_path: Some(path),
            pid_check: true,
            ..Default::default()
        };
        match resolve(&opts) {
            Resolution::Online(t) => {
                assert_eq!(t.status_url, "http://127.0.0.1:52431/v1/status");
                assert_eq!(t.events_url, "http://127.0.0.1:52431/v1/events");
                assert_eq!(t.token.as_deref(), Some("tok-123"));
            }
            other => panic!("expected Online, got {other:?}"),
        }
    }

    #[test]
    fn dead_pid_is_offline() {
        let dir = tempfile::tempdir().unwrap();
        let body = CANONICAL.replace("PIDPLACEHOLDER", "2000000000");
        let path = write_control(dir.path(), "control.json", &body);
        let opts = ResolveOptions {
            control_path: Some(path),
            pid_check: true,
            ..Default::default()
        };
        // On platforms where pid_alive can't verify (non-unix/windows) this
        // returns Online; on unix the high pid is dead -> Offline.
        if cfg!(unix) {
            assert!(matches!(
                resolve(&opts),
                Resolution::Offline(Offline::ProcessDead(_))
            ));
        }
    }

    #[test]
    fn pid_check_disabled_ignores_dead_pid() {
        let dir = tempfile::tempdir().unwrap();
        let body = CANONICAL.replace("PIDPLACEHOLDER", "2000000000");
        let path = write_control(dir.path(), "control.json", &body);
        let opts = ResolveOptions {
            control_path: Some(path),
            pid_check: false,
            ..Default::default()
        };
        assert!(matches!(resolve(&opts), Resolution::Online(_)));
    }

    #[test]
    fn future_control_version_is_offline() {
        let dir = tempfile::tempdir().unwrap();
        let body = r#"{"schemaVersion":2,"baseUrl":"http://127.0.0.1:1","readToken":"t"}"#;
        let path = write_control(dir.path(), "control.json", body);
        let opts = ResolveOptions {
            control_path: Some(path),
            pid_check: false,
            ..Default::default()
        };
        assert!(matches!(
            resolve(&opts),
            Resolution::Offline(Offline::UnsupportedControlVersion(2))
        ));
    }

    #[test]
    fn explicit_base_url_bypasses_discovery() {
        let opts = ResolveOptions {
            base_url: Some("http://127.0.0.1:9/".to_string()),
            token: Some("t".to_string()),
            pid_check: true,
            ..Default::default()
        };
        match resolve(&opts) {
            Resolution::Online(t) => {
                assert_eq!(t.status_url, "http://127.0.0.1:9/v1/status");
                assert_eq!(t.token.as_deref(), Some("t"));
            }
            other => panic!("expected Online, got {other:?}"),
        }
    }
}
