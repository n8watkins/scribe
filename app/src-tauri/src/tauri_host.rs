//! The Tauri side of the engine's seams: [`TauriHost`], the engine's
//! [`Host`] on top of Tauri, and the folders Tauri resolves, checked against
//! the engine's own [`Paths`].

use scribe_core::{
    app_state::AppStatus,
    error::CommandError,
    events::CoreEvent,
    host::{Flavor, Host},
    paths::Paths,
    settings::AppSettings,
};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// The engine's host in the Tauri shell: events go to every webview through
/// `app.emit`, the tray and the main window are the shell's own.
pub struct TauriHost {
    app: AppHandle,
    paths: Paths,
    flavor: Flavor,
}

impl TauriHost {
    pub fn new(app: AppHandle, paths: Paths) -> Self {
        let flavor = Flavor::from_identifier(&app.config().identifier);
        Self { app, paths, flavor }
    }
}

impl Host for TauriHost {
    /// Exactly what the engine's `app.emit(name, payload)` calls send: the
    /// event serializes as its bare payload.
    fn emit(&self, event: CoreEvent) {
        let _ = self.app.emit(event.name(), &event);
    }

    fn paths(&self) -> &Paths {
        &self.paths
    }

    fn flavor(&self) -> Flavor {
        self.flavor
    }

    fn update_tray_status(&self, status: AppStatus) {
        crate::tray::update_tray_status(&self.app, status);
    }

    fn update_engine_summary(&self, summary: String) {
        crate::tray::update_engine_summary(&self.app, summary);
    }

    fn sync_settings(&self, settings: &AppSettings) {
        crate::tray::sync_settings(&self.app, settings);
    }

    fn open_dashboard(&self, route: Option<&str>) -> Result<(), CommandError> {
        crate::tray::open_dashboard(&self.app, route)
    }

    fn toggle_dashboard(&self) -> Result<(), CommandError> {
        crate::tray::toggle_dashboard(&self.app)
    }

    fn main_window_logical_size(&self) -> Option<(u32, u32)> {
        crate::window_size::current_logical(&self.app)
    }
}

/// Resolves the engine's folders for this app (its identifier and product
/// name), the same folders `app.path()` gives.
pub fn resolve_paths<R: Runtime, M: Manager<R>>(
    app: &M,
) -> Result<Paths, scribe_core::paths::UnknownFolder> {
    Paths::resolve(&app.config().identifier, &app.package_info().name)
}

/// Every folder where `paths` and Tauri's resolver disagree, as
/// "folder: engine vs Tauri" lines. Empty when they agree.
pub fn paths_mismatches<R: Runtime, M: Manager<R>>(app: &M, paths: &Paths) -> Vec<String> {
    let resolver = app.path();
    let folders = [
        (
            "data",
            Some(paths.data_dir().to_path_buf()),
            resolver.app_data_dir().ok(),
        ),
        (
            "cache",
            Some(paths.cache_dir().to_path_buf()),
            resolver.app_cache_dir().ok(),
        ),
        (
            "log",
            Some(paths.log_dir().to_path_buf()),
            resolver.app_log_dir().ok(),
        ),
        (
            "resource",
            paths.resource_dir().map(|dir| dir.to_path_buf()),
            resolver.resource_dir().ok(),
        ),
        (
            "home",
            paths.home_dir().map(|dir| dir.to_path_buf()),
            resolver.home_dir().ok(),
        ),
    ];
    folders
        .into_iter()
        .filter(|(_, ours, tauris)| ours != tauris)
        .map(|(name, ours, tauris)| format!("{name}: {ours:?} vs {tauris:?}"))
        .collect()
}

// The tests that build a Tauri mock app (the folders against Tauri's
// resolver, the emitted payloads) are in tests/tauri_host.rs: on Windows a
// test executable that builds an app needs the Common Controls manifest,
// which build.rs embeds into integration tests only.
#[cfg(test)]
mod tests {
    /// Tauri can move the app folders through `app > appDirectoriesOverride`;
    /// the engine's `Paths` does not follow it, so no config may set it.
    #[test]
    fn no_config_overrides_the_app_folders() {
        for source in [
            include_str!("../tauri.conf.json"),
            include_str!("../tauri.dev-flavor.conf.json"),
            include_str!("../tauri.e2e.conf.json"),
            include_str!("../tauri.linux.conf.json"),
            include_str!("../tauri.macos.conf.json"),
            include_str!("../tauri.unsigned-build.conf.json"),
        ] {
            assert!(
                !source.contains("appDirectoriesOverride"),
                "a Tauri config overrides the app folders"
            );
        }
    }
}
