//! The "What's new" card shown once after an update.
//!
//! The front end renders the notes from the bundled CHANGELOG.md; this module
//! only remembers which version's card was dismissed. An install that was
//! used before this feature existed has no record yet, so it sees the card
//! once; a brand-new install records its version silently and shows nothing.

use crate::{backend::BackendState, core::Core, db::Database, error::CommandError, host::Host};

/// [`pending_version`] for the running app (the `whats_new_pending`
/// command).
pub fn pending_version_with(core: &Core) -> Result<Option<String>, CommandError> {
    pending_version_on(core.backend(), core.host().as_ref())
}

/// [`pending_version`] for the version `host` runs: the app's own, which Dev
/// and test builds override, so it can differ from this crate's. The version
/// is read before the database lock is taken, so the host is never called
/// while that lock is held.
fn pending_version_on(
    state: &BackendState,
    host: &dyn Host,
) -> Result<Option<String>, CommandError> {
    let version = host.app_version();
    pending_version(&*state.db()?, &version)
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
    /// 0.16.4-123); the card follows the version it is given.
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

    /// The running app's version comes from the host, never from this
    /// crate, and asking for it neither calls nor tells the host anything.
    #[test]
    fn the_card_follows_the_hosts_version() {
        use crate::host::{Flavor, RecordingHost};

        let root = tempfile::tempdir().unwrap();
        let host = RecordingHost::new(
            crate::paths::Paths::from_parts(
                root.path().join("data"),
                root.path().join("cache"),
                root.path().join("logs"),
                None,
                None,
            ),
            Flavor::Dev,
        )
        .with_app_version("0.16.4-123");
        let state = BackendState::new(
            Database::in_memory().unwrap(),
            root.path().join("cache").join("recordings"),
            crate::data_hygiene::DataDirs {
                data_dir: root.path().join("data"),
                cache_dir: root.path().join("cache"),
            },
        );
        state
            .db()
            .unwrap()
            .save_settings(&AppSettings::default())
            .unwrap();

        assert_eq!(
            pending_version_on(&state, &host).unwrap().as_deref(),
            Some("0.16.4-123")
        );
        state
            .db()
            .unwrap()
            .set_whats_new_seen_version("0.16.4-123")
            .unwrap();
        assert_eq!(pending_version_on(&state, &host).unwrap(), None);
        assert!(host.calls().is_empty());
        assert!(host.events().is_empty());
    }

    #[test]
    fn saving_settings_never_forgets_the_dismissed_version() {
        let db = Database::in_memory().unwrap();
        db.set_whats_new_seen_version("0.10.0").unwrap();
        db.save_settings(&AppSettings::default()).unwrap();
        assert_eq!(pending_version(&db, "0.10.0").unwrap(), None);
    }
}
