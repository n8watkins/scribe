//! Every folder the engine reads or writes, resolved without Tauri.
//!
//! Tauri's path resolver (`app.path()`) names Scribe's folders after the app
//! identifier (`com.natkins.scribe`, or `com.natkins.scribe.dev` for the Dev
//! flavour) and, for the bundled resources, after the product name. [`Paths`]
//! resolves the same folders with the same rules, so the engine can run under
//! a shell other than Tauri. The desktop shell checks at startup (debug
//! builds) and in a test that both agree on every folder.
//!
//! The folders `app.path()` is asked for today:
//!
//! | `Paths`          | Tauri               | Windows                         | macOS                                | Linux                              |
//! |------------------|---------------------|---------------------------------|--------------------------------------|------------------------------------|
//! | `data_dir`       | `app_data_dir()`    | `%APPDATA%\<id>`                | `~/Library/Application Support/<id>` | `$XDG_DATA_HOME/<id>`              |
//! | `cache_dir`      | `app_cache_dir()`   | `%LOCALAPPDATA%\<id>`           | `~/Library/Caches/<id>`              | `$XDG_CACHE_HOME/<id>`             |
//! | `log_dir`        | `app_log_dir()`     | `%LOCALAPPDATA%\<id>\logs`      | `~/Library/Logs/<id>`                | `$XDG_DATA_HOME/<id>/logs`         |
//! | `resource_dir`   | `resource_dir()`    | the executable's folder         | `<bundle>/Contents/Resources`        | `/usr/lib/<product>` (see below)   |
//! | `home_dir`       | `home_dir()`        | `%USERPROFILE%`                 | `$HOME`                              | `$HOME`                            |
//!
//! Scribe sets no `appDirectoriesOverride` in its Tauri config, so the
//! overrides Tauri supports never apply.

use std::path::{Path, PathBuf};

/// The folders Scribe uses, resolved once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    data_dir: PathBuf,
    cache_dir: PathBuf,
    log_dir: PathBuf,
    resource_dir: Option<PathBuf>,
    home_dir: Option<PathBuf>,
}

/// A folder the operating system could not name (no home folder, say).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFolder(pub &'static str);

impl std::fmt::Display for UnknownFolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Could not locate the {} folder", self.0)
    }
}

impl std::error::Error for UnknownFolder {}

impl Paths {
    /// Resolves every folder for the app `identifier` (Tauri's `identifier`)
    /// and `product_name` (Tauri's `productName`, which names the bundled
    /// resources folder on Linux), exactly as Tauri does.
    pub fn resolve(identifier: &str, product_name: &str) -> Result<Self, UnknownFolder> {
        let data_dir = dirs::data_dir()
            .ok_or(UnknownFolder("app data"))?
            .join(identifier);
        let cache_dir = dirs::cache_dir()
            .ok_or(UnknownFolder("app cache"))?
            .join(identifier);
        #[cfg(target_os = "macos")]
        let log_dir = dirs::home_dir()
            .ok_or(UnknownFolder("app log"))?
            .join("Library/Logs")
            .join(identifier);
        #[cfg(not(target_os = "macos"))]
        let log_dir = dirs::data_local_dir()
            .ok_or(UnknownFolder("app log"))?
            .join(identifier)
            .join("logs");
        Ok(Self {
            data_dir,
            cache_dir,
            log_dir,
            resource_dir: resource_dir(product_name),
            home_dir: dirs::home_dir(),
        })
    }

    /// Paths with the given folders, for tests and for shells that keep
    /// Scribe's data somewhere else.
    pub fn from_parts(
        data_dir: PathBuf,
        cache_dir: PathBuf,
        log_dir: PathBuf,
        resource_dir: Option<PathBuf>,
        home_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            data_dir,
            cache_dir,
            log_dir,
            resource_dir,
            home_dir,
        }
    }

    /// Scribe's data folder (Tauri's `app_data_dir()`): the database, clips,
    /// models, replacements.txt. Not the user-chosen `data_dir` setting.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Scribe's cache folder (Tauri's `app_cache_dir()`): recordings in
    /// progress, the status file, editor copies.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// The folder the log files rotate in (Tauri's `app_log_dir()`).
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// The bundled resources (Tauri's `resource_dir()`): the speech engine
    /// binaries. None where Tauri reports an error.
    pub fn resource_dir(&self) -> Option<&Path> {
        self.resource_dir.as_deref()
    }

    /// The user's home folder (Tauri's `home_dir()`).
    pub fn home_dir(&self) -> Option<&Path> {
        self.home_dir.as_deref()
    }
}

/// Tauri's `resource_dir()` (tauri-utils `platform::resource_dir`), step for
/// step: the folder holding the executable on Windows and in a Cargo output
/// folder, else the bundle's resources folder for the platform.
fn resource_dir(product_name: &str) -> Option<PathBuf> {
    let exe = starting_binary()?;
    let exe_dir = exe.parent()?;
    let shown = exe_dir.display().to_string();
    let parts: Vec<&str> = shown.split(std::path::MAIN_SEPARATOR).collect();
    let len = parts.len();
    let in_target =
        (len >= 2 && parts[len - 2] == "target") || (len >= 3 && parts[len - 3] == "target");
    if cfg!(target_os = "windows") || (in_target && exe_dir.join(".cargo-lock").exists()) {
        return Some(exe_dir.to_path_buf());
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(bundle_dir) = exe_dir
            .join(format!("../lib/{product_name}"))
            .canonicalize()
        {
            return Some(bundle_dir);
        }
        // Tauri reads APPDIR (set inside an AppImage) without further checks
        // beyond a warning, so this does too.
        if let Some(appdir) = std::env::var_os("APPDIR") {
            return Some(PathBuf::from(format!(
                "{}/usr/lib/{}",
                Path::new(&appdir).display(),
                product_name
            )));
        }
        Some(PathBuf::from(format!("/usr/lib/{product_name}")))
    }

    #[cfg(target_os = "macos")]
    {
        let _ = product_name;
        exe_dir.join("../Resources").canonicalize().ok()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = product_name;
        None
    }
}

/// The running executable as Tauri sees it: `current_exe()` with symlinks
/// resolved, refused on macOS when any folder on the way is a symlink.
fn starting_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    #[cfg(target_os = "macos")]
    if exe.ancestors().any(|ancestor| {
        ancestor
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
    }) {
        return None;
    }
    exe.canonicalize().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_app_folder_is_named_after_the_identifier() {
        let paths = Paths::resolve("com.natkins.scribe", "Scribe").unwrap();
        assert!(paths.data_dir().ends_with("com.natkins.scribe"));
        assert!(paths.cache_dir().ends_with("com.natkins.scribe"));
        if cfg!(target_os = "macos") {
            assert!(paths.log_dir().ends_with("Library/Logs/com.natkins.scribe"));
        } else {
            assert!(paths.log_dir().ends_with("com.natkins.scribe/logs"));
        }
    }

    #[test]
    fn the_dev_flavour_gets_folders_of_its_own() {
        let stable = Paths::resolve("com.natkins.scribe", "Scribe").unwrap();
        let dev = Paths::resolve("com.natkins.scribe.dev", "Scribe Dev").unwrap();
        assert_ne!(stable.data_dir(), dev.data_dir());
        assert_ne!(stable.cache_dir(), dev.cache_dir());
        assert_ne!(stable.log_dir(), dev.log_dir());
        assert_eq!(stable.home_dir(), dev.home_dir());
    }

    #[test]
    fn from_parts_keeps_the_given_folders() {
        let paths = Paths::from_parts(
            "/d".into(),
            "/c".into(),
            "/l".into(),
            None,
            Some("/h".into()),
        );
        assert_eq!(paths.data_dir(), Path::new("/d"));
        assert_eq!(paths.cache_dir(), Path::new("/c"));
        assert_eq!(paths.log_dir(), Path::new("/l"));
        assert_eq!(paths.resource_dir(), None);
        assert_eq!(paths.home_dir(), Some(Path::new("/h")));
    }
}
