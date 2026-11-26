use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::{
    commands::BackendState,
    core::Core,
    db::Database,
    error::CommandError,
    events::CoreEvent,
    host::{no_main_window, Host},
    models::{
        self, CatalogModel, ModelInfo, ModelRecord, ModelSource, ModelStatus, DEFAULT_MODEL_ID,
    },
    paths::Paths,
    settings::AppSettings,
};

/// The engine's host, for the `AppHandle` wrappers below: found through the
/// managed [`Core`]. None (logged) before the `Core` is managed.
fn host_of(app: &AppHandle, function: &str) -> Option<std::sync::Arc<dyn Host>> {
    match app.try_state::<Core>() {
        Some(core) => Some(core.host().clone()),
        None => {
            log::warn!("model_manager::{function} ran before the engine was ready");
            None
        }
    }
}

#[derive(Default)]
pub struct DownloadRegistry {
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Debug, Clone)]
struct ResolvedModelFile {
    path: PathBuf,
    source: ModelSource,
    size_bytes: u64,
}

impl DownloadRegistry {
    fn start(&self, model_id: &str) -> Result<Arc<AtomicBool>, CommandError> {
        let mut active = self.active.lock().map_err(|_| {
            CommandError::new(
                "model_download_state_unavailable",
                "Could not access model download state.",
            )
        })?;

        if active.contains_key(model_id) {
            return Err(CommandError::new(
                "model_download_in_progress",
                format!("{} is already downloading.", model_id),
            ));
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        active.insert(model_id.to_string(), cancel_flag.clone());
        Ok(cancel_flag)
    }

    fn finish(&self, model_id: &str) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(model_id);
        }
    }

    fn cancel(&self, model_id: &str) -> Result<bool, CommandError> {
        let active = self.active.lock().map_err(|_| {
            CommandError::new(
                "model_download_state_unavailable",
                "Could not access model download state.",
            )
        })?;

        let Some(cancel_flag) = active.get(model_id) else {
            return Ok(false);
        };

        cancel_flag.store(true, Ordering::SeqCst);
        Ok(true)
    }

    fn is_downloading(&self, model_id: &str) -> bool {
        self.active
            .lock()
            .map(|active| active.contains_key(model_id))
            .unwrap_or(false)
    }
}

pub fn request_cancel_download(
    downloads: &DownloadRegistry,
    model_id: &str,
) -> Result<bool, CommandError> {
    downloads.cancel(model_id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgress {
    pub model_id: String,
    pub bytes_downloaded: u64,
    pub total_bytes: Option<u64>,
    pub percent: Option<f64>,
    pub status: ModelStatus,
}

pub fn list_models(
    app: &AppHandle,
    db: &Database,
    downloads: &DownloadRegistry,
) -> Result<Vec<ModelInfo>, CommandError> {
    let host = host_of(app, "list_models").ok_or_else(no_main_window)?;
    list_models_with(host.as_ref(), db, downloads)
}

/// Every catalog model with its status, from the models folder of `host`.
pub fn list_models_with(
    host: &dyn Host,
    db: &Database,
    downloads: &DownloadRegistry,
) -> Result<Vec<ModelInfo>, CommandError> {
    let paths = host.paths();
    let settings = db.get_settings()?;
    let records = db
        .list_model_records()?
        .into_iter()
        .map(|record| (record.id.clone(), record))
        .collect::<HashMap<_, _>>();

    models::catalog()
        .iter()
        .map(|catalog_model| {
            model_info_for_catalog(
                paths,
                *catalog_model,
                records.get(catalog_model.id),
                &settings,
                downloads,
            )
        })
        .collect()
}

pub fn download_model(
    app: &AppHandle,
    state: &BackendState,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let host = host_of(app, "download_model").ok_or_else(no_main_window)?;
    download_model_with(host.as_ref(), state, model_id)
}

/// Downloads a model into the models folder of `host`, sending its progress
/// to `host`. Blocks for the whole download, so it takes the database lock
/// only for each short read or write, never across the network transfer:
/// dictation and every other command keep working meanwhile.
pub fn download_model_with(
    host: &dyn Host,
    state: &BackendState,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let downloads = state.model_downloads();
    let cancel_flag = downloads.start(model_id)?;
    let result = download_model_inner(host, state, cancel_flag, model_id);
    downloads.finish(model_id);
    result
}

pub fn retry_model_download(
    app: &AppHandle,
    state: &BackendState,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let host = host_of(app, "retry_model_download").ok_or_else(no_main_window)?;
    retry_model_download_with(host.as_ref(), state, model_id)
}

/// Throws away a partial download, then downloads the model again.
pub fn retry_model_download_with(
    host: &dyn Host,
    state: &BackendState,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let catalog_model = catalog_model(model_id)?;
    let _ = fs::remove_file(partial_path(host.paths(), catalog_model));
    download_model_with(host, state, model_id)
}

pub fn cancel_model_download(
    db: &Database,
    downloads: &DownloadRegistry,
    model_id: &str,
) -> Result<(), CommandError> {
    if downloads.cancel(model_id)? {
        return Ok(());
    }

    if let Some(mut record) = db.get_model_record(model_id)? {
        if record.status == ModelStatus::Downloading {
            record.status = ModelStatus::NotDownloaded;
            db.upsert_model_record(&record)?;
        }
    }

    Ok(())
}

pub fn delete_model(
    app: &AppHandle,
    db: &Database,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let host = host_of(app, "delete_model").ok_or_else(no_main_window)?;
    delete_model_with(host.as_ref(), db, model_id)
}

/// Deletes a model (and any partial download of it) from the models folder of
/// `host`. Deleting the selected model clears the selection unless another
/// copy is still found.
pub fn delete_model_with(
    host: &dyn Host,
    db: &Database,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let paths = host.paths();
    let catalog_model = catalog_model(model_id)?;
    let model_path = app_data_model_path(paths, catalog_model);
    let part_path = partial_path(paths, catalog_model);

    match fs::remove_file(&model_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CommandError::new(
                "model_delete_failed",
                format!("Could not delete {}. {}", catalog_model.filename, error),
            ))
        }
    }

    let _ = fs::remove_file(part_path);

    let mut settings = db.get_settings()?;
    let was_selected = settings.selected_model_id.as_deref() == Some(model_id);
    if was_selected && resolve_model_file(paths, catalog_model).is_none() {
        settings.selected_model_id = None;
        db.save_settings(&settings)?;
    }

    let record = ModelRecord {
        id: catalog_model.id.to_string(),
        name: catalog_model.name.to_string(),
        filename: catalog_model.filename.to_string(),
        local_path: None,
        size_bytes: None,
        status: ModelStatus::NotDownloaded,
        checksum: Some(catalog_model.expected_sha256.to_owned()),
        selected: false,
        downloaded_at: None,
    };
    db.upsert_model_record(&record)?;

    let registry = DownloadRegistry::default();
    model_info_for_catalog(paths, catalog_model, Some(&record), &settings, &registry)
}

pub fn select_model(
    app: &AppHandle,
    state: &BackendState,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let host = host_of(app, "select_model").ok_or_else(no_main_window)?;
    select_model_with(host.as_ref(), state, model_id)
}

/// Selects a downloaded model. The checksum of a large model takes seconds, so
/// the database is locked only after it passes.
pub fn select_model_with(
    host: &dyn Host,
    state: &BackendState,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let paths = host.paths();
    let catalog_model = catalog_model(model_id)?;
    let resolved = resolve_model_file(paths, catalog_model);
    let Some(resolved) = resolved else {
        return Err(CommandError::new(
            "whisper_model_missing",
            format!(
                "The selected speech model is missing. Re-download {} or place {} in a known model cache.",
                catalog_model.name, catalog_model.filename
            ),
        ));
    };
    let path = resolved.path;

    if !path.is_file() {
        return Err(CommandError::new(
            "whisper_model_missing",
            format!(
                "The selected speech model is missing. Re-download {} or choose another model.",
                catalog_model.name
            ),
        ));
    }

    // Verify before saving the selection: a file that fails the hash check must
    // never become the active model.
    verify_model_file_checksum(catalog_model, &path)?;

    let db = state.db()?;
    let mut settings = db.get_settings()?;
    settings.selected_model_id = Some(model_id.to_string());
    db.save_settings(&settings)?;

    let record = ModelRecord {
        id: catalog_model.id.to_string(),
        name: catalog_model.name.to_string(),
        filename: catalog_model.filename.to_string(),
        local_path: Some(path.to_string_lossy().to_string()),
        size_bytes: Some(resolved.size_bytes),
        status: ModelStatus::Selected,
        checksum: Some(catalog_model.expected_sha256.to_owned()),
        selected: true,
        downloaded_at: Some(Utc::now()),
    };
    db.upsert_model_record(&record)?;
    db.mark_model_selected(model_id)?;

    let registry = DownloadRegistry::default();
    model_info_for_catalog(paths, catalog_model, Some(&record), &settings, &registry)
}

pub fn selected_model_path(
    app: &AppHandle,
    db: &Database,
) -> Result<(String, PathBuf), CommandError> {
    let host = host_of(app, "selected_model_path").ok_or_else(no_main_window)?;
    selected_model_path_with(host.as_ref(), db)
}

/// The selected model (the default one when none is selected) and its file.
pub fn selected_model_path_with(
    host: &dyn Host,
    db: &Database,
) -> Result<(String, PathBuf), CommandError> {
    let settings = db.get_settings()?;
    let model_id = settings
        .selected_model_id
        .as_deref()
        .unwrap_or(DEFAULT_MODEL_ID);
    let catalog_model = catalog_model(model_id)?;
    let resolved = resolve_model_file(host.paths(), catalog_model);

    let Some(resolved) = resolved else {
        return Err(CommandError::new(
            "whisper_model_missing",
            format!(
                "The selected speech model is missing. Re-download {} or place {} in a known model cache.",
                catalog_model.name, catalog_model.filename
            ),
        ));
    };

    Ok((catalog_model.id.to_string(), resolved.path))
}

/// The file of a catalog model by id: the model a dictation started with,
/// when it differs from the one selected now.
pub fn model_path_for_id(
    app: &AppHandle,
    model_id: &str,
) -> Result<(String, PathBuf), CommandError> {
    let host = host_of(app, "model_path_for_id").ok_or_else(no_main_window)?;
    model_path_for_id_with(host.as_ref(), model_id)
}

/// [`model_path_for_id`] in the models folder of `host`.
pub fn model_path_for_id_with(
    host: &dyn Host,
    model_id: &str,
) -> Result<(String, PathBuf), CommandError> {
    let catalog_model = catalog_model(model_id)?;
    match resolve_model_file(host.paths(), catalog_model) {
        Some(resolved) => Ok((catalog_model.id.to_string(), resolved.path)),
        None => Err(CommandError::new(
            "whisper_model_missing",
            format!(
                "{} is missing. Re-download it on the Models page.",
                catalog_model.name
            ),
        )),
    }
}

/// The model a dictation uses: the one it started with, else the selected one.
pub fn session_model_path(
    app: &AppHandle,
    db: &Database,
    session_id: &str,
) -> Result<(String, PathBuf), CommandError> {
    let host = host_of(app, "session_model_path").ok_or_else(no_main_window)?;
    session_model_path_with(host.as_ref(), db, session_id)
}

/// [`session_model_path`] in the models folder of `host`.
pub fn session_model_path_with(
    host: &dyn Host,
    db: &Database,
    session_id: &str,
) -> Result<(String, PathBuf), CommandError> {
    let selected = db.get_settings()?.selected_model_id;
    match crate::session_model::global().model_for(session_id, selected.as_deref()) {
        Some(model_id) if Some(model_id.as_str()) != selected.as_deref() => {
            model_path_for_id_with(host, &model_id).or_else(|_| selected_model_path_with(host, db))
        }
        _ => selected_model_path_with(host, db),
    }
}

fn download_model_inner(
    host: &dyn Host,
    state: &BackendState,
    cancel_flag: Arc<AtomicBool>,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let catalog_model = catalog_model(model_id)?;
    let model_dir = models_dir_in(host.paths());
    fs::create_dir_all(&model_dir).map_err(|error| {
        CommandError::new(
            "model_download_failed",
            format!("Could not create model directory. {}", error),
        )
    })?;

    let final_path = model_dir.join(catalog_model.filename);
    let part_path = model_dir.join(format!("{}.part", catalog_model.filename));
    let mut record = ModelRecord {
        id: catalog_model.id.to_string(),
        name: catalog_model.name.to_string(),
        filename: catalog_model.filename.to_string(),
        local_path: Some(final_path.to_string_lossy().to_string()),
        size_bytes: None,
        status: ModelStatus::Downloading,
        checksum: Some(catalog_model.expected_sha256.to_owned()),
        selected: state.db()?.get_settings()?.selected_model_id.as_deref()
            == Some(catalog_model.id),
        downloaded_at: None,
    };
    state.db()?.upsert_model_record(&record)?;

    let download_result = stream_download(host, catalog_model, &part_path, cancel_flag);
    match download_result {
        Ok(downloaded) => {
            fs::rename(&part_path, &final_path).map_err(|error| {
                CommandError::new(
                    "model_download_failed",
                    format!("Could not finalize {}. {}", catalog_model.name, error),
                )
            })?;

            record.size_bytes = Some(downloaded.bytes_downloaded);
            record.status = if record.selected {
                ModelStatus::Selected
            } else {
                ModelStatus::Downloaded
            };
            record.downloaded_at = Some(Utc::now());
            state.db()?.upsert_model_record(&record)?;
            emit_progress(
                host,
                catalog_model.id,
                downloaded.bytes_downloaded,
                downloaded.total_bytes,
                record.status,
            );
        }
        Err(error) => {
            let _ = fs::remove_file(&part_path);
            record.status = if error.code == "model_download_cancelled" {
                ModelStatus::NotDownloaded
            } else {
                ModelStatus::Failed
            };
            record.local_path = None;
            record.size_bytes = None;
            state.db()?.upsert_model_record(&record)?;
            return Err(error);
        }
    }

    let settings = state.db()?.get_settings()?;
    let registry = DownloadRegistry::default();
    model_info_for_catalog(
        host.paths(),
        catalog_model,
        Some(&record),
        &settings,
        &registry,
    )
}

#[derive(Debug)]
struct DownloadedModel {
    bytes_downloaded: u64,
    total_bytes: Option<u64>,
}

fn stream_download(
    host: &dyn Host,
    catalog_model: CatalogModel,
    part_path: &PathBuf,
    cancel_flag: Arc<AtomicBool>,
) -> Result<DownloadedModel, CommandError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60 * 60))
        .user_agent("Scribe/0.1")
        .build()
        .map_err(|error| {
            CommandError::new(
                "model_download_failed",
                format!("Could not initialize model downloader. {}", error),
            )
        })?;
    let mut response = client
        .get(catalog_model.download_url())
        .send()
        .map_err(|error| {
            CommandError::new(
                "model_download_failed",
                format!("Could not download {}. {}", catalog_model.name, error),
            )
        })?
        .error_for_status()
        .map_err(|error| {
            CommandError::new(
                "model_download_failed",
                format!("Could not download {}. {}", catalog_model.name, error),
            )
        })?;

    let total_bytes = response.content_length();
    if let Some(total_bytes) = total_bytes {
        if total_bytes != catalog_model.expected_size_bytes {
            return Err(CommandError::new(
                "model_download_failed",
                format!(
                    "Download size for {} did not match the reviewed model. Expected {} bytes, got {} bytes.",
                    catalog_model.name, catalog_model.expected_size_bytes, total_bytes
                ),
            ));
        }
    }
    let mut file = fs::File::create(part_path).map_err(|error| {
        CommandError::new(
            "model_download_failed",
            format!("Could not create partial model file. {}", error),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut bytes_downloaded = 0_u64;
    let mut buffer = [0_u8; 1024 * 128];

    emit_progress(
        host,
        catalog_model.id,
        bytes_downloaded,
        total_bytes,
        ModelStatus::Downloading,
    );

    loop {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(CommandError::new(
                "model_download_cancelled",
                format!("Download for {} was cancelled.", catalog_model.name),
            ));
        }

        let bytes_read = response.read(&mut buffer).map_err(|error| {
            CommandError::new(
                "model_download_failed",
                format!(
                    "Could not read download stream for {}. {}",
                    catalog_model.name, error
                ),
            )
        })?;

        if bytes_read == 0 {
            break;
        }

        file.write_all(&buffer[..bytes_read]).map_err(|error| {
            CommandError::new(
                "model_download_failed",
                format!("Could not write partial model file. {}", error),
            )
        })?;
        hasher.update(&buffer[..bytes_read]);
        bytes_downloaded += bytes_read as u64;
        if bytes_downloaded > catalog_model.expected_size_bytes {
            return Err(CommandError::new(
                "model_download_failed",
                format!(
                    "Download for {} exceeded the reviewed size of {} bytes.",
                    catalog_model.name, catalog_model.expected_size_bytes
                ),
            ));
        }
        emit_progress(
            host,
            catalog_model.id,
            bytes_downloaded,
            total_bytes,
            ModelStatus::Downloading,
        );
    }

    file.flush().map_err(|error| {
        CommandError::new(
            "model_download_failed",
            format!("Could not flush partial model file. {}", error),
        )
    })?;

    if let Some(total_bytes) = total_bytes {
        if bytes_downloaded != total_bytes {
            return Err(CommandError::new(
                "model_download_failed",
                format!(
                    "Downloaded size for {} did not match the server response. Expected {} bytes, got {} bytes.",
                    catalog_model.name, total_bytes, bytes_downloaded
                ),
            ));
        }
    }

    if bytes_downloaded != catalog_model.expected_size_bytes {
        return Err(CommandError::new(
            "model_download_failed",
            format!(
                "Downloaded size for {} did not match the reviewed model. Expected {} bytes, got {} bytes.",
                catalog_model.name, catalog_model.expected_size_bytes, bytes_downloaded
            ),
        ));
    }

    let actual_sha256 = format!("{:x}", hasher.finalize());
    if actual_sha256 != catalog_model.expected_sha256 {
        return Err(CommandError::new(
            "model_checksum_mismatch",
            format!(
                "Downloaded {} did not match the expected checksum. Retry the download.",
                catalog_model.name
            ),
        ));
    }

    Ok(DownloadedModel {
        bytes_downloaded,
        total_bytes,
    })
}

fn verify_model_file_checksum(
    catalog_model: CatalogModel,
    path: &Path,
) -> Result<(), CommandError> {
    let mut file = fs::File::open(path).map_err(|error| {
        CommandError::new(
            "whisper_model_missing",
            format!("Could not read {}. {}", path.display(), error),
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        CommandError::new(
            "model_checksum_mismatch",
            format!("Could not inspect {}. {}", catalog_model.name, error),
        )
    })?;
    if metadata.len() != catalog_model.expected_size_bytes {
        return Err(CommandError::new(
            "model_checksum_mismatch",
            format!(
                "{} did not match the expected size. Re-download the model.",
                catalog_model.name
            ),
        ));
    }

    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 128];

    loop {
        let bytes_read = file.read(&mut buffer).map_err(|error| {
            CommandError::new(
                "model_checksum_mismatch",
                format!("Could not verify {}. {}", catalog_model.name, error),
            )
        })?;

        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
    }

    let actual_sha256 = format!("{:x}", hasher.finalize());
    if actual_sha256 != catalog_model.expected_sha256 {
        return Err(CommandError::new(
            "model_checksum_mismatch",
            format!(
                "{} did not match the expected checksum. Re-download the model.",
                catalog_model.name
            ),
        ));
    }

    Ok(())
}

/// Sends `model://download-progress`.
fn emit_progress(
    host: &dyn Host,
    model_id: &str,
    bytes_downloaded: u64,
    total_bytes: Option<u64>,
    status: ModelStatus,
) {
    host.emit(CoreEvent::ModelDownloadProgress(download_progress(
        model_id,
        bytes_downloaded,
        total_bytes,
        status,
    )));
}

fn download_progress(
    model_id: &str,
    bytes_downloaded: u64,
    total_bytes: Option<u64>,
    status: ModelStatus,
) -> ModelDownloadProgress {
    let percent = total_bytes
        .filter(|total| *total > 0)
        .map(|total| (bytes_downloaded as f64 / total as f64 * 100.0).min(100.0));
    ModelDownloadProgress {
        model_id: model_id.to_string(),
        bytes_downloaded,
        total_bytes,
        percent,
        status,
    }
}

fn model_info_for_catalog(
    paths: &Paths,
    catalog_model: CatalogModel,
    record: Option<&ModelRecord>,
    settings: &AppSettings,
    downloads: &DownloadRegistry,
) -> Result<ModelInfo, CommandError> {
    let resolved = resolve_model_file(paths, catalog_model);
    let selected = settings.selected_model_id.as_deref() == Some(catalog_model.id);
    let status = if downloads.is_downloading(catalog_model.id) {
        ModelStatus::Downloading
    } else if resolved.is_some() && selected {
        ModelStatus::Selected
    } else if resolved.is_some() {
        ModelStatus::Downloaded
    } else {
        record
            .map(|record| record.status)
            .filter(|status| *status == ModelStatus::Failed)
            .unwrap_or(ModelStatus::NotDownloaded)
    };

    let local_path = resolved
        .as_ref()
        .map(|resolved| resolved.path.to_string_lossy().to_string());
    let source = resolved.as_ref().map(|resolved| resolved.source);
    let size_bytes = resolved
        .as_ref()
        .map(|resolved| resolved.size_bytes)
        .or_else(|| record.and_then(|record| record.size_bytes));

    Ok(ModelInfo {
        id: catalog_model.id.to_string(),
        name: catalog_model.name.to_string(),
        filename: catalog_model.filename.to_string(),
        download_url: catalog_model.download_url(),
        disk_size_label: catalog_model.disk_size_label.to_string(),
        local_path,
        source,
        size_bytes,
        status,
        checksum: catalog_model.checksum(),
        selected,
        downloaded_at: record.and_then(|record| record.downloaded_at),
        multilingual: catalog_model.multilingual,
        engine: catalog_model.engine,
        credit: catalog_model.credit,
    })
}

pub fn models_dir(app: &AppHandle) -> Result<PathBuf, CommandError> {
    let host = host_of(app, "models_dir").ok_or_else(no_main_window)?;
    Ok(models_dir_with(host.as_ref()))
}

/// The folder Scribe downloads models into: `models` in the app data folder
/// of `host` (Tauri's `app_data_dir()`, not the user-chosen data folder).
pub fn models_dir_with(host: &dyn Host) -> PathBuf {
    models_dir_in(host.paths())
}

fn models_dir_in(paths: &Paths) -> PathBuf {
    paths.data_dir().join("models")
}

fn app_data_model_path(paths: &Paths, catalog_model: CatalogModel) -> PathBuf {
    models_dir_in(paths).join(catalog_model.filename)
}

fn partial_path(paths: &Paths, catalog_model: CatalogModel) -> PathBuf {
    models_dir_in(paths).join(format!("{}.part", catalog_model.filename))
}

fn resolve_model_file(paths: &Paths, catalog_model: CatalogModel) -> Option<ResolvedModelFile> {
    for (path, source) in model_file_candidates(paths, catalog_model) {
        let metadata = fs::metadata(&path)
            .ok()
            .filter(|metadata| metadata.is_file());

        if let Some(metadata) = metadata {
            return Some(ResolvedModelFile {
                path,
                source,
                size_bytes: metadata.len(),
            });
        }
    }

    None
}

pub fn smallest_downloaded_model_path(app: &AppHandle) -> Option<PathBuf> {
    let host = host_of(app, "smallest_downloaded_model_path")?;
    smallest_downloaded_model_path_with(host.as_ref())
}

/// Path of the smallest downloaded model, for cheap operations that only need
/// *a* model loaded rather than the user's selected one — e.g. the GPU device
/// probe, where device enumeration is model-independent so loading a multi-GB
/// model would be pure waste. Returns `None` when nothing is downloaded.
pub fn smallest_downloaded_model_path_with(host: &dyn Host) -> Option<PathBuf> {
    crate::models::catalog()
        .iter()
        // The probe runs whisper-cli, which only loads Whisper models.
        .filter(|model| model.engine == models::EngineKind::Whisper)
        .filter_map(|model| {
            resolve_model_file(host.paths(), *model)
                .map(|resolved| (resolved.size_bytes, resolved.path))
        })
        .min_by_key(|(size, _)| *size)
        .map(|(_, path)| path)
}

fn model_file_candidates(
    paths: &Paths,
    catalog_model: CatalogModel,
) -> Vec<(PathBuf, ModelSource)> {
    let mut candidates = vec![(
        app_data_model_path(paths, catalog_model),
        ModelSource::AppData,
    )];

    candidates.extend(
        external_model_dirs()
            .into_iter()
            .map(|dir| (dir.join(catalog_model.filename), ModelSource::ExternalCache)),
    );

    candidates
}

fn external_model_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // Prefer the new SCRIBE_MODEL_DIR override; fall back to the legacy
    // LOCALDICTATE_MODEL_DIR for back-compat. Both are honored if set.
    if let Some(path) = std::env::var_os("SCRIBE_MODEL_DIR") {
        dirs.push(PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("LOCALDICTATE_MODEL_DIR") {
        dirs.push(PathBuf::from(path));
    }

    dirs
}

fn catalog_model(model_id: &str) -> Result<CatalogModel, CommandError> {
    models::catalog_model(model_id).ok_or_else(|| {
        CommandError::new(
            "unknown_model",
            format!(
                "Unknown speech model '{}'. Choose a model from the catalog.",
                model_id
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Flavor, RecordingHost};

    /// A host whose folders live in a fresh temporary folder.
    fn temp_host(name: &str) -> (RecordingHost, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "scribe-model-manager-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let host = RecordingHost::new(
            Paths::from_parts(
                root.join("data"),
                root.join("cache"),
                root.join("logs"),
                None,
                None,
            ),
            Flavor::Stable,
        );
        (host, root)
    }

    fn place(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"not really a model").unwrap();
    }

    #[test]
    fn models_live_in_the_hosts_data_folder() {
        let (host, root) = temp_host("folder");
        assert_eq!(models_dir_with(&host), root.join("data").join("models"));
    }

    #[test]
    fn a_model_in_the_hosts_folder_is_listed_selected_and_found() {
        let (host, root) = temp_host("list");
        let db = Database::in_memory().unwrap();
        let tiny = models::catalog_model("tiny.en").unwrap();
        let file = root.join("data").join("models").join(tiny.filename);
        place(&file);

        let listed = list_models_with(&host, &db, &DownloadRegistry::default()).unwrap();
        assert_eq!(listed.len(), models::catalog().len());
        let info = listed.iter().find(|info| info.id == "tiny.en").unwrap();
        assert_eq!(info.status, ModelStatus::Downloaded);
        assert_eq!(info.source, Some(ModelSource::AppData));
        assert_eq!(
            info.local_path.as_deref(),
            Some(file.to_string_lossy().as_ref())
        );

        let mut settings = db.get_settings().unwrap();
        settings.selected_model_id = Some("tiny.en".into());
        db.save_settings(&settings).unwrap();
        let listed = list_models_with(&host, &db, &DownloadRegistry::default()).unwrap();
        let info = listed.iter().find(|info| info.id == "tiny.en").unwrap();
        assert_eq!(info.status, ModelStatus::Selected);
        assert_eq!(
            selected_model_path_with(&host, &db).unwrap(),
            ("tiny.en".to_string(), file.clone())
        );
        assert_eq!(
            model_path_for_id_with(&host, "tiny.en").unwrap().1,
            file.clone()
        );
        // With no dictation pinning a model, a session uses the selected one.
        assert_eq!(
            session_model_path_with(&host, &db, "no-such-session")
                .unwrap()
                .1,
            file
        );
        assert_eq!(
            model_path_for_id_with(&host, "base.en").unwrap_err().code,
            "whisper_model_missing"
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn deleting_the_selected_model_removes_its_files_and_the_selection() {
        let (host, root) = temp_host("delete");
        let db = Database::in_memory().unwrap();
        let tiny = models::catalog_model("tiny.en").unwrap();
        let models = root.join("data").join("models");
        let file = models.join(tiny.filename);
        let part = models.join(format!("{}.part", tiny.filename));
        place(&file);
        place(&part);
        let mut settings = db.get_settings().unwrap();
        settings.selected_model_id = Some("tiny.en".into());
        db.save_settings(&settings).unwrap();

        let info = delete_model_with(&host, &db, "tiny.en").unwrap();
        assert_eq!(info.status, ModelStatus::NotDownloaded);
        assert!(!file.exists());
        assert!(!part.exists());
        assert_eq!(db.get_settings().unwrap().selected_model_id, None);
        assert_eq!(
            delete_model_with(&host, &db, "no-such-model")
                .unwrap_err()
                .code,
            "unknown_model"
        );
        assert!(host.events().is_empty());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn download_progress_goes_to_the_host_as_today() {
        let (host, _root) = temp_host("progress");
        emit_progress(&host, "tiny.en", 50, Some(200), ModelStatus::Downloading);
        emit_progress(&host, "tiny.en", 10, None, ModelStatus::Downloaded);

        let events = host.events();
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .all(|event| event.name() == "model://download-progress"));
        assert_eq!(
            serde_json::to_value(&events[0]).unwrap(),
            serde_json::json!({
                "modelId": "tiny.en",
                "bytesDownloaded": 50,
                "totalBytes": 200,
                "percent": 25.0,
                "status": "downloading",
            })
        );
        assert_eq!(
            serde_json::to_value(&events[1]).unwrap()["percent"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn the_gpu_probe_picks_the_smallest_whisper_model_in_the_hosts_folder() {
        let (host, root) = temp_host("smallest");
        assert_eq!(smallest_downloaded_model_path_with(&host), None);
        let models = root.join("data").join("models");
        let base = models.join(models::catalog_model("base.en").unwrap().filename);
        let tiny = models.join(models::catalog_model("tiny.en").unwrap().filename);
        place(&base);
        fs::write(&tiny, b"tiny").unwrap();
        assert_eq!(smallest_downloaded_model_path_with(&host), Some(tiny));

        fs::remove_dir_all(&root).unwrap();
    }
}
