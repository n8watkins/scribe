//! Commands of the `output` domain (output, selection_transform,
//! text_replace, the platform clipboard and paste permission).

use super::off_main;
use crate::{
    error::CommandError,
    output::{self, paste_permission, OutputResult, PastePermission},
    selection_transform::transform_selection_blocking,
};

#[tauri::command]
pub async fn get_paste_permission(app: tauri::AppHandle) -> Result<PastePermission, CommandError> {
    off_main(app, |_| Ok(paste_permission())).await
}

/// Starts granting the paste permission where one is needed: on macOS the
/// system prompt and the Accessibility settings (the grant happens there, so
/// the result is usually still not granted), on Wayland the desktop's
/// one-time dialog (which this waits for).
#[tauri::command]
pub async fn request_paste_permission(
    app: tauri::AppHandle,
) -> Result<PastePermission, CommandError> {
    off_main(app, |_| {
        #[cfg(target_os = "macos")]
        crate::platform::request_accessibility();
        #[cfg(target_os = "linux")]
        crate::platform::request_wayland_paste_permission()?;
        Ok(paste_permission())
    })
    .await
}

#[tauri::command]
pub async fn copy_last_transcript(app: tauri::AppHandle) -> Result<OutputResult, CommandError> {
    off_main(app, output::copy_last_transcript).await
}

#[tauri::command]
pub async fn paste_transcript(
    app: tauri::AppHandle,
    id: String,
) -> Result<OutputResult, CommandError> {
    off_main(app, move |app| output::paste_transcript(app, &id)).await
}

#[tauri::command]
pub async fn copy_transcript(
    app: tauri::AppHandle,
    id: String,
) -> Result<OutputResult, CommandError> {
    off_main(app, move |app| output::copy_transcript(app, &id)).await
}

/// Selected-text transform: copy the text the user highlighted in the focused
/// app, rewrite it with the local LLM per `instruction`, and paste the result
/// back over the selection. The instruction is typed here (the v1 path); the
/// voice path will route a spoken instruction into the same engine.
///
/// Runs on a blocking worker because the local inference (and the capture
/// settle) can be slow; settings are read up front so no lock is held across
/// the HTTP call. Reuses `notes_analysis_endpoint` / `notes_analysis_model` as
/// the LLM server — no new settings.
#[tauri::command]
pub async fn transform_selection(
    app: tauri::AppHandle,
    instruction: String,
) -> Result<OutputResult, CommandError> {
    tauri::async_runtime::spawn_blocking(move || transform_selection_blocking(&app, &instruction))
        .await
        .map_err(|error| CommandError::new("selection_transform_failed", error.to_string()))?
}
