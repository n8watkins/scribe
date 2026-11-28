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

use crate::{
    commands::BackendState,
    db::Database,
    error::CommandError,
    events::CoreEvent,
    host::Host,
    models::{
        self, CatalogModel, ModelInfo, ModelRecord, ModelSource, ModelStatus, DEFAULT_MODEL_ID,
    },
    paths::Paths,
    settings::AppSettings,
};

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

/// Every catalog model with its status, from the models folder of `host`.
pub fn list_models_with(
    host: &dyn Host,
    db: &Database,
    downloads: &DownloadRegistry,
) -> Result<Vec<ModelInfo>, CommandError> {
    list_models_in(&ModelFolders::of(host), db, downloads)
}

fn list_models_in(
    folders: &ModelFolders,
    db: &Database,
    downloads: &DownloadRegistry,
) -> Result<Vec<ModelInfo>, CommandError> {
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
                folders,
                *catalog_model,
                records.get(catalog_model.id),
                &settings,
                downloads,
            )
        })
        .collect()
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

/// Throws away a partial download, then downloads the model again.
pub fn retry_model_download_with(
    host: &dyn Host,
    state: &BackendState,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let catalog_model = catalog_model(model_id)?;
    let _ = fs::remove_file(ModelFolders::of(host).partial_path(catalog_model));
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

fn delete_model_in(
    folders: &ModelFolders,
    db: &Database,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let catalog_model = catalog_model(model_id)?;
    let model_path = folders.model_path(catalog_model);
    let part_path = folders.partial_path(catalog_model);

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
    if was_selected && resolve_model_file(folders, catalog_model).is_none() {
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
    model_info_for_catalog(folders, catalog_model, Some(&record), &settings, &registry)
}

fn select_model_in(
    folders: &ModelFolders,
    state: &BackendState,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let catalog_model = catalog_model(model_id)?;
    let resolved = resolve_model_file(folders, catalog_model);
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
    model_info_for_catalog(folders, catalog_model, Some(&record), &settings, &registry)
}

/// The selected model (the default one when none is selected) and its file.
pub fn selected_model_path_with(
    host: &dyn Host,
    db: &Database,
) -> Result<(String, PathBuf), CommandError> {
    selected_model_path_in(&ModelFolders::of(host), db)
}

fn selected_model_path_in(
    folders: &ModelFolders,
    db: &Database,
) -> Result<(String, PathBuf), CommandError> {
    let settings = db.get_settings()?;
    let model_id = settings
        .selected_model_id
        .as_deref()
        .unwrap_or(DEFAULT_MODEL_ID);
    let catalog_model = catalog_model(model_id)?;
    let resolved = resolve_model_file(folders, catalog_model);

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

fn model_path_for_id_in(
    folders: &ModelFolders,
    model_id: &str,
) -> Result<(String, PathBuf), CommandError> {
    let catalog_model = catalog_model(model_id)?;
    match resolve_model_file(folders, catalog_model) {
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
/// In the models folder of `host`.
pub fn session_model_path_with(
    host: &dyn Host,
    db: &Database,
    session_id: &str,
) -> Result<(String, PathBuf), CommandError> {
    session_model_path_in(&ModelFolders::of(host), db, session_id)
}

fn session_model_path_in(
    folders: &ModelFolders,
    db: &Database,
    session_id: &str,
) -> Result<(String, PathBuf), CommandError> {
    let selected = db.get_settings()?.selected_model_id;
    match crate::session_model::global().model_for(session_id, selected.as_deref()) {
        Some(model_id) if Some(model_id.as_str()) != selected.as_deref() => {
            model_path_for_id_in(folders, &model_id)
                .or_else(|_| selected_model_path_in(folders, db))
        }
        _ => selected_model_path_in(folders, db),
    }
}

fn download_model_inner(
    host: &dyn Host,
    state: &BackendState,
    cancel_flag: Arc<AtomicBool>,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let catalog_model = catalog_model(model_id)?;
    let folders = ModelFolders::of(host);
    let model_dir = folders.app_data.clone();
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
    model_info_for_catalog(&folders, catalog_model, Some(&record), &settings, &registry)
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
    folders: &ModelFolders,
    catalog_model: CatalogModel,
    record: Option<&ModelRecord>,
    settings: &AppSettings,
    downloads: &DownloadRegistry,
) -> Result<ModelInfo, CommandError> {
    let resolved = resolve_model_file(folders, catalog_model);
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

/// Opens the models folder of `host` in the system's file manager, creating it
/// first when it is missing.
pub fn open_models_folder_with(host: &dyn Host) -> Result<(), CommandError> {
    crate::data_hygiene::open_folder_with(host, models_dir_with(host))
}

/// The `delete_model` command: deletes the model, and when that cleared the
/// selection announces the new settings (`scribe:settings-changed`, then the
/// tray menu) and then runs `after` (the speech engine follows the settings).
/// Holds the settings write lock from the read to the announcement, so
/// announcements go out in the order the writes happened. Nothing is sent
/// when the settings did not change.
pub fn delete_model_announced_with(
    host: &dyn Host,
    state: &BackendState,
    model_id: &str,
    after: impl FnOnce(&AppSettings),
) -> Result<ModelInfo, CommandError> {
    delete_model_announced_in(&ModelFolders::of(host), host, state, model_id, after)
}

fn delete_model_announced_in(
    folders: &ModelFolders,
    host: &dyn Host,
    state: &BackendState,
    model_id: &str,
    after: impl FnOnce(&AppSettings),
) -> Result<ModelInfo, CommandError> {
    let _writes = crate::settings_store::lock(state.settings_writes());
    let (info, settings) = {
        let db = state.db()?;
        let before = db.get_settings()?;
        let info = delete_model_in(folders, &db, model_id)?;
        let now = db.get_settings()?;
        (info, (now != before).then_some(now))
    };
    // Deleting the selected model clears the selection.
    if let Some(settings) = settings {
        crate::settings_store::emit_changed_with(host, &settings);
        after(&settings);
    }
    Ok(info)
}

/// The `select_model` command: selects the model, announces the new settings
/// (`scribe:settings-changed`, then the tray menu) and runs `after` (the
/// speech engine unloads the old model before the new one loads). Holds the
/// settings write lock from the write to the announcement. Sends nothing when
/// the selection fails.
pub fn select_model_announced_with(
    host: &dyn Host,
    state: &BackendState,
    model_id: &str,
    after: impl FnOnce(&AppSettings),
) -> Result<ModelInfo, CommandError> {
    select_model_announced_in(&ModelFolders::of(host), host, state, model_id, after)
}

fn select_model_announced_in(
    folders: &ModelFolders,
    host: &dyn Host,
    state: &BackendState,
    model_id: &str,
    after: impl FnOnce(&AppSettings),
) -> Result<ModelInfo, CommandError> {
    announce_selection_in(
        host,
        state,
        || select_model_in(folders, state, model_id),
        after,
    )
}

/// Runs `select` under the settings write lock, then announces the stored
/// settings and runs `after`. Nothing is announced when `select` fails.
fn announce_selection_in(
    host: &dyn Host,
    state: &BackendState,
    select: impl FnOnce() -> Result<ModelInfo, CommandError>,
    after: impl FnOnce(&AppSettings),
) -> Result<ModelInfo, CommandError> {
    let _writes = crate::settings_store::lock(state.settings_writes());
    let info = select()?;
    let settings = state.db()?.get_settings()?;
    crate::settings_store::emit_changed_with(host, &settings);
    after(&settings);
    Ok(info)
}

/// The folder Scribe downloads models into: `models` in the app data folder
/// of `host` (Tauri's `app_data_dir()`, not the user-chosen data folder).
pub fn models_dir_with(host: &dyn Host) -> PathBuf {
    models_dir_in(host.paths())
}

fn models_dir_in(paths: &Paths) -> PathBuf {
    paths.data_dir().join("models")
}

/// Where model files are looked for: Scribe's own models folder first, then
/// the folders named by `SCRIBE_MODEL_DIR` and `LOCALDICTATE_MODEL_DIR`. The
/// engine reads those variables each time it takes the folders; tests name
/// their own folders so a developer's variables never leak in.
struct ModelFolders {
    /// `<app data>/models`, where downloads go.
    app_data: PathBuf,
    /// Read-only folders searched after it.
    external: Vec<PathBuf>,
}

impl ModelFolders {
    fn of(host: &dyn Host) -> Self {
        Self {
            app_data: models_dir_in(host.paths()),
            external: external_model_dirs(),
        }
    }

    fn model_path(&self, catalog_model: CatalogModel) -> PathBuf {
        self.app_data.join(catalog_model.filename)
    }

    fn partial_path(&self, catalog_model: CatalogModel) -> PathBuf {
        self.app_data
            .join(format!("{}.part", catalog_model.filename))
    }
}

fn resolve_model_file(
    folders: &ModelFolders,
    catalog_model: CatalogModel,
) -> Option<ResolvedModelFile> {
    for (path, source) in model_file_candidates(folders, catalog_model) {
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

/// Path of the smallest downloaded model, for cheap operations that only need
/// *a* model loaded rather than the user's selected one — e.g. the GPU device
/// probe, where device enumeration is model-independent so loading a multi-GB
/// model would be pure waste. Returns `None` when nothing is downloaded.
pub fn smallest_downloaded_model_path_with(host: &dyn Host) -> Option<PathBuf> {
    smallest_downloaded_model_path_in(&ModelFolders::of(host))
}

fn smallest_downloaded_model_path_in(folders: &ModelFolders) -> Option<PathBuf> {
    crate::models::catalog()
        .iter()
        // The probe runs whisper-cli, which only loads Whisper models.
        .filter(|model| model.engine == models::EngineKind::Whisper)
        .filter_map(|model| {
            resolve_model_file(folders, *model).map(|resolved| (resolved.size_bytes, resolved.path))
        })
        .min_by_key(|(size, _)| *size)
        .map(|(_, path)| path)
}

fn model_file_candidates(
    folders: &ModelFolders,
    catalog_model: CatalogModel,
) -> Vec<(PathBuf, ModelSource)> {
    let mut candidates = vec![(folders.model_path(catalog_model), ModelSource::AppData)];

    candidates.extend(
        folders
            .external
            .iter()
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
    use crate::host::{Flavor, HostCall, RecordingHost};

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

    /// The folders the engine takes for `host`, without the folders a
    /// developer's `SCRIBE_MODEL_DIR` or `LOCALDICTATE_MODEL_DIR` would add.
    fn hosts_folders_only(host: &RecordingHost) -> ModelFolders {
        ModelFolders {
            external: Vec::new(),
            ..ModelFolders::of(host)
        }
    }

    #[test]
    fn models_live_in_the_hosts_data_folder() {
        let (host, root) = temp_host("folder");
        let models = root.join("data").join("models");
        assert_eq!(models_dir_with(&host), models);
        assert_eq!(ModelFolders::of(&host).app_data, models);
    }

    #[test]
    fn a_model_in_the_hosts_folder_is_listed_selected_and_found() {
        let (host, root) = temp_host("list");
        let folders = hosts_folders_only(&host);
        let db = Database::in_memory().unwrap();
        let tiny = models::catalog_model("tiny.en").unwrap();
        let file = root.join("data").join("models").join(tiny.filename);
        place(&file);

        let listed = list_models_in(&folders, &db, &DownloadRegistry::default()).unwrap();
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
        let listed = list_models_in(&folders, &db, &DownloadRegistry::default()).unwrap();
        let info = listed.iter().find(|info| info.id == "tiny.en").unwrap();
        assert_eq!(info.status, ModelStatus::Selected);
        assert_eq!(
            selected_model_path_in(&folders, &db).unwrap(),
            ("tiny.en".to_string(), file.clone())
        );
        assert_eq!(
            model_path_for_id_in(&folders, "tiny.en").unwrap().1,
            file.clone()
        );
        // With no dictation pinning a model, a session uses the selected one.
        assert_eq!(
            session_model_path_in(&folders, &db, "no-such-session")
                .unwrap()
                .1,
            file
        );
        assert_eq!(
            model_path_for_id_in(&folders, "base.en").unwrap_err().code,
            "whisper_model_missing"
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_external_model_folder_is_searched_after_the_hosts() {
        let (host, root) = temp_host("external");
        let external = root.join("external");
        let folders = ModelFolders {
            external: vec![external.clone()],
            ..ModelFolders::of(&host)
        };
        let tiny = models::catalog_model("tiny.en").unwrap();
        let outside = external.join(tiny.filename);
        place(&outside);
        assert_eq!(
            model_path_for_id_in(&folders, "tiny.en").unwrap().1,
            outside
        );
        let info = model_info_for_catalog(
            &folders,
            tiny,
            None,
            &AppSettings::default(),
            &DownloadRegistry::default(),
        )
        .unwrap();
        assert_eq!(info.source, Some(ModelSource::ExternalCache));

        // A copy in the app's own folder wins.
        let inside = root.join("data").join("models").join(tiny.filename);
        place(&inside);
        assert_eq!(model_path_for_id_in(&folders, "tiny.en").unwrap().1, inside);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn deleting_the_selected_model_removes_its_files_and_the_selection() {
        let (host, root) = temp_host("delete");
        let folders = hosts_folders_only(&host);
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

        let info = delete_model_in(&folders, &db, "tiny.en").unwrap();
        assert_eq!(info.status, ModelStatus::NotDownloaded);
        assert!(!file.exists());
        assert!(!part.exists());
        assert_eq!(db.get_settings().unwrap().selected_model_id, None);
        assert_eq!(
            delete_model_in(&folders, &db, "no-such-model")
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

    fn backend_in(root: &Path) -> BackendState {
        BackendState::new(
            Database::in_memory().unwrap(),
            root.join("cache").join("recordings"),
            crate::data_hygiene::DataDirs {
                data_dir: root.join("data"),
                cache_dir: root.join("cache"),
            },
        )
    }

    #[test]
    fn opening_the_models_folder_creates_it_and_opens_it_through_the_host() {
        use crate::host::HostCall;
        let (host, root) = temp_host("open");
        let models = root.join("data").join("models");
        assert!(!models.exists());

        open_models_folder_with(&host).unwrap();

        assert!(models.is_dir());
        assert_eq!(host.calls(), vec![HostCall::OpenPath(models)]);
        assert!(host.events().is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_failed_open_of_the_models_folder_keeps_its_error_code() {
        use crate::host::Failing;
        let (host, root) = temp_host("open-fails");
        host.fail(Failing::OpenPath);

        let error = open_models_folder_with(&host).unwrap_err();

        assert_eq!(error.code, "open_folder_failed");
        assert!(error.message.contains(&Failing::OpenPath.message()));
        assert!(host.events().is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn deleting_the_selected_model_announces_the_cleared_selection_once() {
        let (host, root) = temp_host("delete-selected");
        let folders = hosts_folders_only(&host);
        let state = backend_in(&root);
        let tiny = models::catalog_model("tiny.en").unwrap();
        place(&root.join("data").join("models").join(tiny.filename));
        {
            let db = state.db().unwrap();
            let mut settings = db.get_settings().unwrap();
            settings.selected_model_id = Some("tiny.en".to_string());
            db.save_settings(&settings).unwrap();
        }

        let mut followed = Vec::new();
        let info = delete_model_announced_in(&folders, &host, &state, "tiny.en", |settings| {
            // The announcement is already out when the engine follows.
            followed.push((host.events().len(), settings.selected_model_id.clone()));
        })
        .unwrap();

        assert_eq!(info.id, "tiny.en");
        assert_eq!(followed, vec![(1, None)]);
        let events = host.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name(), "scribe:settings-changed");
        assert_eq!(
            serde_json::to_value(&events[0]).unwrap()["selectedModelId"],
            serde_json::Value::Null
        );
        // The tray menu is synced with the stored (cleared) settings, and
        // nothing else is asked of the host.
        assert_eq!(
            host.calls(),
            [HostCall::SyncSettings(
                state.db().unwrap().get_settings().unwrap()
            )]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn deleting_a_model_that_was_not_selected_sends_nothing() {
        let (host, root) = temp_host("delete-other");
        let folders = hosts_folders_only(&host);
        let state = backend_in(&root);
        let tiny = models::catalog_model("tiny.en").unwrap();
        place(&root.join("data").join("models").join(tiny.filename));

        let mut followed = 0;
        delete_model_announced_in(&folders, &host, &state, "tiny.en", |_| followed += 1).unwrap();

        assert_eq!(followed, 0);
        assert!(host.events().is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn deleting_an_unknown_model_fails_and_sends_nothing() {
        let (host, root) = temp_host("delete-unknown");
        let folders = hosts_folders_only(&host);
        let state = backend_in(&root);

        let mut followed = 0;
        let error =
            delete_model_announced_in(&folders, &host, &state, "no-such-model", |_| followed += 1)
                .unwrap_err();

        assert_eq!(error.code, "unknown_model");
        assert_eq!(followed, 0);
        assert!(host.events().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn selecting_a_model_that_is_not_downloaded_fails_and_sends_nothing() {
        let (host, root) = temp_host("select-missing");
        let state = backend_in(&root);

        let folders = hosts_folders_only(&host);

        let mut followed = 0;
        let error =
            select_model_announced_in(&folders, &host, &state, "tiny.en", |_| followed += 1)
                .unwrap_err();

        assert_eq!(error.code, "whisper_model_missing");
        assert_eq!(followed, 0);
        assert!(host.events().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_selection_is_announced_once_before_the_engine_follows() {
        let (host, root) = temp_host("select-announced");
        let folders = hosts_folders_only(&host);
        let state = backend_in(&root);
        let tiny = models::catalog_model("tiny.en").unwrap();

        let mut followed = Vec::new();
        let info = announce_selection_in(
            &host,
            &state,
            || {
                let db = state.db()?;
                let mut settings = db.get_settings()?;
                settings.selected_model_id = Some("tiny.en".to_string());
                db.save_settings(&settings)?;
                model_info_for_catalog(
                    &folders,
                    tiny,
                    None,
                    &settings,
                    &DownloadRegistry::default(),
                )
            },
            |settings| followed.push((host.events().len(), settings.selected_model_id.clone())),
        )
        .unwrap();

        assert_eq!(info.id, "tiny.en");
        assert_eq!(followed, vec![(1, Some("tiny.en".to_string()))]);
        let events = host.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name(), "scribe:settings-changed");
        assert_eq!(
            serde_json::to_value(&events[0]).unwrap()["selectedModelId"],
            "tiny.en"
        );
        // The tray menu is synced with the stored settings, and nothing else
        // is asked of the host.
        assert_eq!(
            host.calls(),
            [HostCall::SyncSettings(
                state.db().unwrap().get_settings().unwrap()
            )]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_gpu_probe_picks_the_smallest_whisper_model_in_the_hosts_folder() {
        let (host, root) = temp_host("smallest");
        let folders = hosts_folders_only(&host);
        assert_eq!(smallest_downloaded_model_path_in(&folders), None);
        let models = root.join("data").join("models");
        let base = models.join(models::catalog_model("base.en").unwrap().filename);
        let tiny = models.join(models::catalog_model("tiny.en").unwrap().filename);
        place(&base);
        fs::write(&tiny, b"tiny").unwrap();
        assert_eq!(smallest_downloaded_model_path_in(&folders), Some(tiny));

        fs::remove_dir_all(&root).unwrap();
    }
}
