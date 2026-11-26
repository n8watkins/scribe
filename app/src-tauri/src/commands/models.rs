//! Commands of the `models` domain (model_manager, whisper, whisper_server,
//! parakeet, speech_engine, gpu, session_model).

use tauri::Manager;

use super::off_main;
use crate::{
    commands::BackendState, data_hygiene::open_folder, error::CommandError, gpu::GpuProbe,
    model_manager, models::ModelInfo, settings_store,
};

#[tauri::command]
pub async fn open_models_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let dir = model_manager::models_dir(app)?;
        open_folder(app, dir)
    })
    .await
}

#[tauri::command]
pub async fn list_models(app: tauri::AppHandle) -> Result<Vec<ModelInfo>, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let db = state.db()?;
        model_manager::list_models(app, &db, state.model_downloads())
    })
    .await
}

#[tauri::command]
pub async fn download_model(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main(app, move |app| {
        model_manager::download_model(app, app.state::<BackendState>().inner(), &model_id)
    })
    .await
}

#[tauri::command]
pub async fn cancel_model_download(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<(), CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        if model_manager::request_cancel_download(state.model_downloads(), &model_id)? {
            return Ok(());
        }

        let db = state.db()?;
        model_manager::cancel_model_download(&db, state.model_downloads(), &model_id)
    })
    .await
}

#[tauri::command]
pub async fn retry_model_download(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main(app, move |app| {
        model_manager::retry_model_download(app, app.state::<BackendState>().inner(), &model_id)
    })
    .await
}

#[tauri::command]
pub async fn delete_model(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let (info, settings) = {
            let db = state.db()?;
            let before = db.get_settings()?;
            let info = model_manager::delete_model(app, &db, &model_id)?;
            let after = db.get_settings()?;
            (info, (after != before).then_some(after))
        };
        // Deleting the selected model clears the selection.
        if let Some(settings) = settings {
            settings_store::emit_changed(app, &settings);
            app.state::<crate::whisper_server::WarmTranscriber>()
                .sync_with_settings(&settings);
        }
        Ok(info)
    })
    .await
}

#[tauri::command]
pub async fn select_model(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let info = model_manager::select_model(app, state, &model_id)?;
        let settings = state.db()?.get_settings()?;
        settings_store::emit_changed(app, &settings);
        // Switching models unloads the old one before the new one loads.
        app.state::<crate::whisper_server::WarmTranscriber>()
            .sync_with_settings(&settings);
        Ok(info)
    })
    .await
}

/// Which speech model is active, whether it is loaded, and what it uses.
/// Changes arrive as `scribe:engine-status` events.
#[tauri::command]
pub async fn get_engine_status(
    app: tauri::AppHandle,
) -> Result<crate::speech_engine::EngineStatus, CommandError> {
    off_main(app, move |app| {
        Ok(app
            .state::<crate::whisper_server::WarmTranscriber>()
            .status())
    })
    .await
}

/// Tauri command: probe the Vulkan GPUs available for transcription. Runs the
/// (blocking, model-loading) probe on a worker thread so it never hitches the UI
/// / IPC thread, and returns instantly from the session cache after the first
/// successful probe.
#[tauri::command]
pub async fn probe_gpu_devices(app: tauri::AppHandle) -> Result<GpuProbe, CommandError> {
    tauri::async_runtime::spawn_blocking(move || crate::gpu::probe(&app))
        .await
        .map_err(|error| CommandError::new("gpu_probe_failed", error.to_string()))
}
