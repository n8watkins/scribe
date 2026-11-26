//! What the desktop portals granted Scribe, kept so the user is asked once,
//! ever: the RemoteDesktop restore token and whether a request was turned
//! down. Stored as a small JSON file in Scribe's data folder, readable by
//! the user only (0600), and replaced atomically.

use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "wayland-portal.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PortalGrants {
    /// The RemoteDesktop restore token. Portals hand out a fresh one each
    /// time a session starts and the old one stops working, so it is saved
    /// after every start.
    pub remote_desktop_token: Option<String>,
    /// The user cancelled the RemoteDesktop permission dialog.
    pub remote_desktop_declined: bool,
    /// The user cancelled the GlobalShortcuts dialog.
    pub shortcuts_declined: bool,
}

/// Where the grants live. A trait so the portal logic is tested against an
/// in-memory store.
pub trait GrantStore: Send {
    fn load(&self) -> PortalGrants;
    fn save(&self, grants: &PortalGrants) -> Result<(), String>;

    /// Changes some fields and keeps the rest: the paste worker and the
    /// shortcuts thread each own different fields of the same file.
    fn update(&self, change: &mut dyn FnMut(&mut PortalGrants)) -> Result<(), String> {
        let _guard = UPDATES.lock();
        let mut grants = self.load();
        change(&mut grants);
        self.save(&grants)
    }
}

/// Serializes read-modify-write cycles within the process.
static UPDATES: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct FileGrantStore {
    path: PathBuf,
}

impl FileGrantStore {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: dir.join(FILE_NAME),
        }
    }
}

impl GrantStore for FileGrantStore {
    fn load(&self) -> PortalGrants {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return PortalGrants::default();
        };
        // Tighten a file an older build or the user left readable by others.
        if let Ok(metadata) = fs::metadata(&self.path) {
            if metadata.permissions().mode() & 0o077 != 0 {
                let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600));
            }
        }
        serde_json::from_str(&text).unwrap_or_else(|error| {
            log::warn!("Ignoring unreadable {}: {error}", self.path.display());
            PortalGrants::default()
        })
    }

    fn save(&self, grants: &PortalGrants) -> Result<(), String> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| "no folder for the portal grants".to_string())?;
        fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        let temp = self.path.with_extension("json.tmp");
        let text = serde_json::to_vec_pretty(grants).map_err(|error| error.to_string())?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|error| error.to_string())?;
        // `mode` only applies when the file is created; a leftover temp file
        // keeps its old mode otherwise.
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
        file.write_all(&text).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temp, &self.path).map_err(|error| error.to_string())
    }
}

/// Keeps the grants in memory: tests, and a session without a data folder.
#[derive(Default)]
pub struct MemoryGrantStore {
    grants: std::sync::Mutex<PortalGrants>,
}

impl GrantStore for MemoryGrantStore {
    fn load(&self) -> PortalGrants {
        self.grants
            .lock()
            .map(|grants| grants.clone())
            .unwrap_or_default()
    }

    fn save(&self, grants: &PortalGrants) -> Result<(), String> {
        if let Ok(mut stored) = self.grants.lock() {
            *stored = grants.clone();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_round_trip_through_a_user_only_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileGrantStore::in_dir(dir.path());
        assert_eq!(store.load(), PortalGrants::default());

        let grants = PortalGrants {
            remote_desktop_token: Some("token-1".to_string()),
            remote_desktop_declined: false,
            shortcuts_declined: true,
        };
        store.save(&grants).unwrap();
        assert_eq!(store.load(), grants);

        let mode = fs::metadata(dir.path().join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(!dir.path().join("wayland-portal.json.tmp").exists());
    }

    #[test]
    fn a_readable_file_is_tightened_and_a_broken_one_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, r#"{"remoteDesktopToken":"t"}"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let store = FileGrantStore::in_dir(dir.path());
        assert_eq!(store.load().remote_desktop_token.as_deref(), Some("t"));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::write(&path, "not json").unwrap();
        assert_eq!(store.load(), PortalGrants::default());
    }

    #[test]
    fn updates_keep_the_fields_they_dont_touch() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileGrantStore::in_dir(dir.path());
        store
            .update(&mut |grants| grants.shortcuts_declined = true)
            .unwrap();
        store
            .update(&mut |grants| grants.remote_desktop_token = Some("t".to_string()))
            .unwrap();
        let grants = store.load();
        assert!(grants.shortcuts_declined);
        assert_eq!(grants.remote_desktop_token.as_deref(), Some("t"));
    }

    #[test]
    fn the_store_creates_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileGrantStore::in_dir(&dir.path().join("nested"));
        store.save(&PortalGrants::default()).unwrap();
        assert_eq!(store.load(), PortalGrants::default());
    }
}
