//! The "What's new" card shown once after an update.
//!
//! The front end renders the notes from the bundled CHANGELOG.md; this module
//! only remembers which version's card was dismissed. An install that was
//! used before this feature existed has no record yet, so it sees the card
//! once; a brand-new install records its version silently and shows nothing.

use tauri::{AppHandle, State};

use crate::{commands::BackendState, db::Database, error::CommandError};

/// The version whose card should show now, or `None`.
pub fn pending_version(db: &Database, current: &str) -> Result<Option<String>, CommandError> {
    match db.whats_new_seen_version()? {
        Some(seen) if seen == current => Ok(None),
        Some(_) => Ok(Some(current.to_string())),
        None => {
            let fresh_install =
                !db.has_saved_settings()? && db.list_recent_transcripts(1)?.is_empty();
            if fresh_install {
                db.set_whats_new_seen_version(current)?;
                Ok(None)
            } else {
                Ok(Some(current.to_string()))
            }
        }
    }
}

/// The running version when its "What's new" card has not been dismissed yet.
#[tauri::command]
pub fn whats_new_pending(
    app: AppHandle,
    state: State<'_, BackendState>,
) -> Result<Option<String>, CommandError> {
    let db = state.db()?;
    pending_version(&db, &app.package_info().version.to_string())
}

/// Remembers that the card for `version` was dismissed.
#[tauri::command]
pub fn dismiss_whats_new(
    state: State<'_, BackendState>,
    version: String,
) -> Result<(), CommandError> {
    let version = version.trim();
    if version.is_empty() || version.len() > 64 {
        return Err(CommandError::new("invalid_version", "Invalid app version."));
    }
    state.db()?.set_whats_new_seen_version(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;

    #[test]
    fn a_fresh_install_skips_the_card_and_remembers_its_version() {
        let db = Database::in_memory().unwrap();
        assert_eq!(pending_version(&db, "0.10.0").unwrap(), None);
        assert_eq!(
            db.whats_new_seen_version().unwrap().as_deref(),
            Some("0.10.0")
        );
        assert_eq!(
            pending_version(&db, "0.11.0").unwrap().as_deref(),
            Some("0.11.0")
        );
    }

    #[test]
    fn an_existing_install_sees_the_card_once() {
        let db = Database::in_memory().unwrap();
        db.save_settings(&AppSettings::default()).unwrap();
        assert_eq!(
            pending_version(&db, "0.10.0").unwrap().as_deref(),
            Some("0.10.0")
        );
        db.set_whats_new_seen_version("0.10.0").unwrap();
        assert_eq!(pending_version(&db, "0.10.0").unwrap(), None);
    }

    #[test]
    fn saving_settings_never_forgets_the_dismissed_version() {
        let db = Database::in_memory().unwrap();
        db.set_whats_new_seen_version("0.10.0").unwrap();
        db.save_settings(&AppSettings::default()).unwrap();
        assert_eq!(pending_version(&db, "0.10.0").unwrap(), None);
    }
}
