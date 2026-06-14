//! Lightweight update *detection* against GitHub releases: compares the running
//! version with the latest release tag so the UI can surface "update available"
//! and point at the release page. This module never downloads or installs — the
//! actual signed download + install is handled by tauri-plugin-updater (driven
//! from the frontend: auto-install on launch in App.tsx and About → Install).
//! Kept separate so this check stays cheap and side-effect-free.

use serde::Serialize;
use std::time::Duration;

use crate::error::CommandError;

pub const RELEASES_PAGE_URL: &str = "https://github.com/n8watkins/scribe/releases/latest";
const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/n8watkins/scribe/releases/latest";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_url: String,
}

pub fn check_for_update() -> Result<UpdateCheckResult, CommandError> {
    let current = env!("CARGO_PKG_VERSION");

    let response = reqwest::blocking::Client::builder()
        .user_agent(concat!("Scribe/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| CommandError::new("update_check_failed", error.to_string()))?
        .get(LATEST_RELEASE_API)
        .send()
        .map_err(|error| {
            CommandError::new(
                "update_check_failed",
                format!("Could not reach GitHub to check for updates. {}", error),
            )
        })?;

    if !response.status().is_success() {
        return Err(CommandError::new(
            "update_check_failed",
            format!(
                "GitHub returned HTTP {} for the latest-release lookup.",
                response.status()
            ),
        ));
    }

    let body = response.text().map_err(|error| {
        CommandError::new(
            "update_check_failed",
            format!("Could not read GitHub's release response. {}", error),
        )
    })?;
    let body: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
        CommandError::new(
            "update_check_failed",
            format!("Could not parse GitHub's release response. {}", error),
        )
    })?;

    let tag = body
        .get("tag_name")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            CommandError::new(
                "update_check_failed",
                "GitHub's latest-release response had no tag_name.",
            )
        })?;
    let latest = tag.trim_start_matches('v').to_string();
    let release_url = body
        .get("html_url")
        .and_then(|value| value.as_str())
        .unwrap_or(RELEASES_PAGE_URL)
        .to_string();

    Ok(UpdateCheckResult {
        update_available: version_is_newer(&latest, current),
        current_version: current.to_string(),
        latest_version: latest,
        release_url,
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
    use super::version_is_newer;

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
