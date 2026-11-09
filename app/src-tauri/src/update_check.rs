//! Lightweight update *detection*: compares the running version with the one
//! in the release's `latest.json` (the updater manifest, served from GitHub's
//! release-asset CDN, so it has no API rate limit) and points at the release
//! page. This module never downloads or installs.
//!
//! On Windows the frontend uses tauri-plugin-updater instead, which reads the
//! same manifest and also installs the signed update. The manifest only
//! carries a Windows package so far, so the plugin errors on macOS and Linux;
//! there the frontend asks this command, says a new version is out and links
//! to the release for a manual install.

use serde::Serialize;
use std::time::Duration;

use crate::error::CommandError;

pub const RELEASES_PAGE_URL: &str = "https://github.com/n8watkins/scribe/releases/latest";
const LATEST_MANIFEST_URL: &str =
    "https://github.com/n8watkins/scribe/releases/latest/download/latest.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_url: String,
}

fn failure(message: impl Into<String>) -> CommandError {
    CommandError::new("update_check_failed", message)
}

pub fn check_for_update() -> Result<UpdateCheckResult, CommandError> {
    let response = reqwest::blocking::Client::builder()
        .user_agent(concat!("Scribe/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| failure(error.to_string()))?
        .get(LATEST_MANIFEST_URL)
        .send()
        .map_err(|error| {
            failure(format!(
                "Could not reach GitHub to check for updates. {}",
                error
            ))
        })?;

    if !response.status().is_success() {
        return Err(failure(format!(
            "GitHub returned HTTP {} for the latest release.",
            response.status()
        )));
    }

    let body = response
        .text()
        .map_err(|error| failure(format!("Could not read the latest release. {}", error)))?;
    result_from_manifest(&body, env!("CARGO_PKG_VERSION"))
}

/// Reads `version` from an updater manifest and compares it with `current`.
fn result_from_manifest(body: &str, current: &str) -> Result<UpdateCheckResult, CommandError> {
    let manifest: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| failure(format!("Could not parse the latest release. {}", error)))?;
    let latest = manifest
        .get("version")
        .and_then(|value| value.as_str())
        .map(|version| version.trim().trim_start_matches('v'))
        .filter(|version| !version.is_empty())
        .ok_or_else(|| failure("The latest release has no version."))?
        .to_string();

    Ok(UpdateCheckResult {
        update_available: version_is_newer(&latest, current),
        current_version: current.to_string(),
        latest_version: latest,
        release_url: RELEASES_PAGE_URL.to_string(),
    })
}

/// Numeric dotted-version comparison; missing or non-numeric parts compare
/// as 0, so "0.2" == "0.2.0" and "v0.2.0-beta" never beats "0.2.0".
fn version_is_newer(candidate: &str, current: &str) -> bool {
    fn parts(version: &str) -> Vec<u64> {
        version
            .split('.')
            .map(|part| {
                part.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    }

    let candidate = parts(candidate);
    let current = parts(current);
    for i in 0..candidate.len().max(current.len()) {
        let a = candidate.get(i).copied().unwrap_or(0);
        let b = current.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{result_from_manifest, version_is_newer};

    #[test]
    fn reads_the_version_from_the_updater_manifest() {
        let manifest = r#"{"version":"0.12.0","pub_date":"2026-11-09T00:00:00Z","platforms":{"windows-x86_64":{"signature":"x","url":"https://example.test/a.exe"}}}"#;
        let result = result_from_manifest(manifest, "0.11.0").unwrap();
        assert!(result.update_available);
        assert_eq!(result.latest_version, "0.12.0");
        assert_eq!(result.current_version, "0.11.0");
        assert_eq!(result.release_url, super::RELEASES_PAGE_URL);

        let current = result_from_manifest(manifest, "0.12.0").unwrap();
        assert!(!current.update_available);
    }

    #[test]
    fn a_manifest_without_a_version_is_an_error() {
        assert!(result_from_manifest(r#"{"platforms":{}}"#, "0.11.0").is_err());
        assert!(result_from_manifest(r#"{"version":""}"#, "0.11.0").is_err());
        assert!(result_from_manifest("not json", "0.11.0").is_err());
    }

    #[test]
    fn newer_versions_are_detected() {
        assert!(version_is_newer("0.2.0", "0.1.3"));
        assert!(version_is_newer("1.0.0", "0.9.9"));
        assert!(version_is_newer("0.1.10", "0.1.9"));
    }

    #[test]
    fn equal_or_older_versions_are_not_updates() {
        assert!(!version_is_newer("0.2.0", "0.2.0"));
        assert!(!version_is_newer("0.2", "0.2.0"));
        assert!(!version_is_newer("0.1.9", "0.2.0"));
    }

    #[test]
    fn suffixes_and_garbage_compare_as_zero() {
        assert!(!version_is_newer("0.2.0-beta", "0.2.0"));
        assert!(version_is_newer("0.3.0-rc1", "0.2.0"));
        assert!(!version_is_newer("not-a-version", "0.2.0"));
    }
}
