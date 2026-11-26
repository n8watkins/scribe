//! What the engine asks of the desktop shell's tray and main window.
//!
//! The tray and the window helpers live in the shell (app/src-tauri), which
//! depends on this crate, so the engine cannot call them directly. The shell
//! installs them here once, first thing at startup; until then (and in this
//! crate's own tests) the tray calls do nothing and the window calls answer as
//! they would with no main window, which is what they did before the tray and
//! the window existed.

use std::sync::OnceLock;

use tauri::AppHandle;

use crate::{app_state::AppStatus, error::CommandError, settings::AppSettings};

/// The shell's tray and main-window functions the engine calls.
pub struct ShellHooks {
    /// The tray's status word (tooltip and title).
    pub update_tray_status: fn(&AppHandle, AppStatus),
    /// The tray tooltip's speech-model line.
    pub update_engine_summary: fn(&AppHandle, String),
    /// The tray menu's "Launch at startup" tick.
    pub sync_settings: fn(&AppHandle, &AppSettings),
    /// Shows and focuses the main window on a route ("dashboard" when None).
    pub open_dashboard: fn(&AppHandle, Option<&str>) -> Result<(), CommandError>,
    /// Hides the main window when it is in front, else opens it.
    pub toggle_dashboard: fn(&AppHandle) -> Result<(), CommandError>,
    /// The main window's inner size in logical pixels.
    pub main_window_logical_size: fn(&AppHandle) -> Option<(u32, u32)>,
}

static HOOKS: OnceLock<ShellHooks> = OnceLock::new();

/// Installs the shell's functions. Only the first call counts.
pub fn install(hooks: ShellHooks) {
    if HOOKS.set(hooks).is_err() {
        log::warn!("The shell hooks were installed twice; kept the first");
    }
}

fn no_main_window() -> CommandError {
    CommandError::new("window_not_found", "Could not find the main window.")
}

pub(crate) fn update_tray_status(app: &AppHandle, status: AppStatus) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.update_tray_status)(app, status);
    }
}

pub(crate) fn update_engine_summary(app: &AppHandle, summary: String) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.update_engine_summary)(app, summary);
    }
}

pub(crate) fn sync_settings(app: &AppHandle, settings: &AppSettings) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.sync_settings)(app, settings);
    }
}

pub(crate) fn open_dashboard(app: &AppHandle, route: Option<&str>) -> Result<(), CommandError> {
    match HOOKS.get() {
        Some(hooks) => (hooks.open_dashboard)(app, route),
        None => Err(no_main_window()),
    }
}

pub(crate) fn toggle_dashboard(app: &AppHandle) -> Result<(), CommandError> {
    match HOOKS.get() {
        Some(hooks) => (hooks.toggle_dashboard)(app),
        None => Err(no_main_window()),
    }
}

pub(crate) fn main_window_logical_size(app: &AppHandle) -> Option<(u32, u32)> {
    HOOKS
        .get()
        .and_then(|hooks| (hooks.main_window_logical_size)(app))
}
