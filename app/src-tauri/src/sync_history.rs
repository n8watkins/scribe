use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{error::CommandError, github_backup::SyncReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncSource {
    Manual,
    Automatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOutcome {
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubSyncActivity {
    pub completed_at: DateTime<Utc>,
    pub outcome: SyncOutcome,
    pub source: SyncSource,
    pub repo: String,
    pub synced_items: u32,
    pub files_written: u32,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

impl GitHubSyncActivity {
    pub fn from_attempt(
        source: SyncSource,
        repo: String,
        report: &SyncReport,
        error: Option<&CommandError>,
    ) -> Self {
        Self {
            completed_at: Utc::now(),
            outcome: if error.is_some() {
                SyncOutcome::Error
            } else {
                SyncOutcome::Success
            },
            source,
            repo,
            synced_items: report.synced_notes,
            files_written: report.files_written,
            error_code: error.map(|error| truncate(&error.code, 100)),
            error_message: error.map(|error| truncate(&error.message, 500)),
        }
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_activity_keeps_partial_progress_and_bounds_error_text() {
        let report = SyncReport {
            synced_notes: 3,
            files_written: 2,
        };
        let error = CommandError::new("x".repeat(150), "detail".repeat(200));
        let activity = GitHubSyncActivity::from_attempt(
            SyncSource::Automatic,
            "alice/notes".into(),
            &report,
            Some(&error),
        );

        assert_eq!(activity.outcome, SyncOutcome::Error);
        assert_eq!(activity.synced_items, 3);
        assert_eq!(activity.files_written, 2);
        assert_eq!(activity.error_code.unwrap().chars().count(), 100);
        assert_eq!(activity.error_message.unwrap().chars().count(), 500);
    }

    #[test]
    fn success_activity_has_no_error_details() {
        let activity = GitHubSyncActivity::from_attempt(
            SyncSource::Manual,
            "alice/notes".into(),
            &SyncReport {
                synced_notes: 1,
                files_written: 1,
            },
            None,
        );

        assert_eq!(activity.outcome, SyncOutcome::Success);
        assert_eq!(activity.error_code, None);
        assert_eq!(activity.error_message, None);
    }
}
