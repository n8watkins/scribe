//! Commands of the `models` domain (model_manager, whisper, whisper_server,
//! parakeet, speech_engine, gpu, session_model).
//!
//! Converted to the engine handle: commands here take the managed [`Core`]
//! (docs/CORE-SPLIT.md). `open_models_folder`, `delete_model` and
//! `select_model` announce settings through the host (`emit_changed_with`)
//! and open the folder through it (`open_folder_with`).

use super::off_main_core;
use crate::{error::CommandError, gpu::GpuProbe, model_manager, models::ModelInfo, Core};

#[tauri::command]
pub async fn open_models_folder(core: tauri::State<'_, Core>) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        model_manager::open_models_folder_with(core.host().as_ref())
    })
    .await
}

#[tauri::command]
pub async fn list_models(core: tauri::State<'_, Core>) -> Result<Vec<ModelInfo>, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let state = core.backend();
        let db = state.db()?;
        model_manager::list_models_with(core.host().as_ref(), &db, state.model_downloads())
    })
    .await
}

#[tauri::command]
pub async fn download_model(
    core: tauri::State<'_, Core>,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        model_manager::download_model_with(core.host().as_ref(), core.backend(), &model_id)
    })
    .await
}

#[tauri::command]
pub async fn cancel_model_download(
    core: tauri::State<'_, Core>,
    model_id: String,
) -> Result<(), CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        let state = core.backend();
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
    core: tauri::State<'_, Core>,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        model_manager::retry_model_download_with(core.host().as_ref(), core.backend(), &model_id)
    })
    .await
}

#[tauri::command]
pub async fn delete_model(
    core: tauri::State<'_, Core>,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        model_manager::delete_model_announced_with(
            core.host().as_ref(),
            core.backend(),
            &model_id,
            // Deleting the selected model clears the selection.
            |settings| core.warm_transcriber().sync_with_settings(settings),
        )
    })
    .await
}

#[tauri::command]
pub async fn select_model(
    core: tauri::State<'_, Core>,
    model_id: String,
) -> Result<ModelInfo, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        model_manager::select_model_announced_with(
            core.host().as_ref(),
            core.backend(),
            &model_id,
            // Switching models unloads the old one before the new one loads.
            |settings| core.warm_transcriber().sync_with_settings(settings),
        )
    })
    .await
}

/// Which speech model is active, whether it is loaded, and what it uses.
/// Changes arrive as `scribe:engine-status` events.
#[tauri::command]
pub async fn get_engine_status(
    core: tauri::State<'_, Core>,
) -> Result<crate::speech_engine::EngineStatus, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        Ok(core.warm_transcriber().status())
    })
    .await
}

/// Tauri command: probe the Vulkan GPUs available for transcription. Runs the
/// (blocking, model-loading) probe on a worker thread so it never hitches the UI
/// / IPC thread, and returns instantly from the session cache after the first
/// successful probe.
#[tauri::command]
pub async fn probe_gpu_devices(core: tauri::State<'_, Core>) -> Result<GpuProbe, CommandError> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Only Windows runs a probe (the bundled whisper-cli loads a model).
        crate::gpu::probe_with(
            core.host().as_ref(),
            cfg!(windows).then_some(core.warm_transcriber()),
        )
    })
    .await
    .map_err(|error| CommandError::new("gpu_probe_failed", error.to_string()))
}
