//! The installed [`Host`], for engine code that still takes an `AppHandle`.
//!
//! The tray and the window helpers live in the shell (app/src-tauri), which
//! depends on this crate, so the engine cannot call them directly: it asks the
//! [`Host`] the shell installs here first thing in its setup. Until then (and
//! in this crate's own tests) the tray calls do nothing and the window calls
//! answer as they would with no main window, which is what they did before the
//! tray and the window existed.
//!
//! Code converted to `&Core` calls `core.host()` instead; these functions go
//! once nothing calls them (docs/CORE-SPLIT.md).

use std::sync::{Arc, OnceLock};

use tauri::AppHandle;

use crate::{
    app_state::AppStatus,
    error::CommandError,
    host::{no_main_window, Host},
    settings::AppSettings,
};

static HOST: OnceLock<Arc<dyn Host>> = OnceLock::new();

/// Installs the shell's host. Only the first call counts.
pub fn install(host: Arc<dyn Host>) {
    if HOST.set(host).is_err() {
        log::warn!("The shell host was installed twice; kept the first");
    }
}

/// The installed host, once the shell's setup has started.
pub fn host() -> Option<&'static Arc<dyn Host>> {
    HOST.get()
}

pub fn update_tray_status(_app: &AppHandle, status: AppStatus) {
    if let Some(host) = HOST.get() {
        host.update_tray_status(status);
    }
}

pub fn update_engine_summary(_app: &AppHandle, summary: String) {
    if let Some(host) = HOST.get() {
        host.update_engine_summary(summary);
    }
}

pub fn sync_settings(_app: &AppHandle, settings: &AppSettings) {
    if let Some(host) = HOST.get() {
        host.sync_settings(settings);
    }
}

pub fn open_dashboard(_app: &AppHandle, route: Option<&str>) -> Result<(), CommandError> {
    match HOST.get() {
        Some(host) => host.open_dashboard(route),
        None => Err(no_main_window()),
    }
}

pub fn toggle_dashboard(_app: &AppHandle) -> Result<(), CommandError> {
    match HOST.get() {
        Some(host) => host.toggle_dashboard(),
        None => Err(no_main_window()),
    }
}

pub fn main_window_logical_size(_app: &AppHandle) -> Option<(u32, u32)> {
    HOST.get().and_then(|host| host.main_window_logical_size())
}
