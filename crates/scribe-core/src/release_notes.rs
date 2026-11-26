//! The "What's new" card shown once after an update.
//!
//! The front end renders the notes from the bundled CHANGELOG.md; this module
//! only remembers which version's card was dismissed. An install that was
//! used before this feature existed has no record yet, so it sees the card
//! once; a brand-new install records its version silently and shows nothing.

use crate::{core::Core, db::Database, error::CommandError};

/// [`pending_version`] for the running app's `version` (the
/// `whats_new_pending` command). The shell passes the app's own version:
/// Dev and test builds override it, so it can differ from this crate's.
pub fn pending_version_with(core: &Core, version: &str) -> Result<Option<String>, CommandError> {
    pending_version(&*core.backend().db()?, version)
}

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

    /// Dev and test builds carry a version the shell overrides (say
    /// 0.16.4-123); the card follows that version, not this crate's.
    #[test]
    fn the_card_follows_the_version_passed_in() {
        let db = Database::in_memory().unwrap();
        db.set_whats_new_seen_version("0.16.4-123").unwrap();
        assert_eq!(pending_version(&db, "0.16.4-123").unwrap(), None);
        assert_eq!(
            pending_version(&db, "0.16.4-124").unwrap().as_deref(),
            Some("0.16.4-124")
        );
    }

    #[test]
    fn saving_settings_never_forgets_the_dismissed_version() {
        let db = Database::in_memory().unwrap();
        db.set_whats_new_seen_version("0.10.0").unwrap();
        db.save_settings(&AppSettings::default()).unwrap();
        assert_eq!(pending_version(&db, "0.10.0").unwrap(), None);
    }
}
