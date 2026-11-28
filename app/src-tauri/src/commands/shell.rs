//! Commands that belong to the shell itself (windows, the release page, the
//! update check), not to an engine domain. No P1-3 lane owns this file.

use tauri_plugin_opener::OpenerExt;

use super::off_main_core;
use crate::{error::CommandError, settings::AppSettings, settings_store, Core};

#[tauri::command]
pub fn open_dashboard(core: tauri::State<'_, Core>) -> Result<(), CommandError> {
    core.host().open_dashboard(None)
}

/// Saves the main window's current inner size, in logical pixels, as the size
/// Scribe opens at (Appearance > Window size). Restored at launch by
/// `window_size::restore`.
#[tauri::command]
pub async fn save_window_size(core: tauri::State<'_, Core>) -> Result<AppSettings, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let state = core.backend();
        let (width, height) = core.host().main_window_logical_size().ok_or_else(|| {
            CommandError::new("window_size_unavailable", "Could not read the window size.")
        })?;

        let _writes = settings_store::lock(state.settings_writes());
        let mut settings = state.db()?.get_settings()?;
        settings.window_width = Some(width as i32);
        settings.window_height = Some(height as i32);
        settings.window_size_logical = true;
        state.db()?.save_settings(&settings)?;
        settings_store::emit_changed_with(core.host().as_ref(), &settings);
        log::info!("Saved default window size {}x{}", width, height);
        Ok(settings)
    })
    .await
}

#[tauri::command]
pub async fn check_for_update() -> Result<crate::update_check::UpdateCheckResult, CommandError> {
    tauri::async_runtime::spawn_blocking(crate::update_check::check_for_update)
        .await
        .map_err(|error| CommandError::new("update_check_failed", error.to_string()))?
}

#[tauri::command]
pub fn open_release_page(app: tauri::AppHandle, url: Option<String>) -> Result<(), CommandError> {
    let url = url.unwrap_or_else(|| crate::update_check::RELEASES_PAGE_URL.to_string());
    if !url.starts_with("https://github.com/n8watkins/scribe/") {
        return Err(CommandError::new(
            "invalid_url",
            "Refusing to open a non-release URL.",
        ));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| CommandError::new("open_url_failed", error.to_string()))
}
