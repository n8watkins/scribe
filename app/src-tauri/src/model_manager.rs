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
use sha1::{Digest, Sha1};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    db::Database,
    error::CommandError,
    models::{
        self, CatalogModel, ModelInfo, ModelRecord, ModelSource, ModelStatus, DEFAULT_MODEL_ID,
    },
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

pub fn list_models(
    app: &AppHandle,
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
                app,
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
    db: &Database,
    downloads: &DownloadRegistry,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let cancel_flag = downloads.start(model_id)?;
    let result = download_model_inner(app, db, cancel_flag, model_id);
    downloads.finish(model_id);
    result
}

pub fn retry_model_download(
    app: &AppHandle,
    db: &Database,
    downloads: &DownloadRegistry,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let catalog_model = catalog_model(model_id)?;
    let _ = fs::remove_file(partial_path(app, catalog_model)?);
    download_model(app, db, downloads, model_id)
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
    let catalog_model = catalog_model(model_id)?;
    let model_path = app_data_model_path(app, catalog_model)?;
    let part_path = partial_path(app, catalog_model)?;

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
    if was_selected && resolve_model_file(app, catalog_model)?.is_none() {
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
        checksum: catalog_model.expected_sha1.map(ToOwned::to_owned),
        selected: false,
        downloaded_at: None,
    };
    db.upsert_model_record(&record)?;

    let registry = DownloadRegistry::default();
    model_info_for_catalog(app, catalog_model, Some(&record), &settings, &registry)
}

pub fn select_model(
    app: &AppHandle,
    db: &Database,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let catalog_model = catalog_model(model_id)?;
    let resolved = resolve_model_file(app, catalog_model)?;
    let Some(resolved) = resolved else {
        return Err(CommandError::new(
            "whisper_model_missing",
            format!(
                "Selected Whisper model is missing. Re-download {} or place {} in a known model cache.",
                catalog_model.name, catalog_model.filename
            ),
        ));
    };
    let path = resolved.path;

    if !path.is_file() {
        return Err(CommandError::new(
            "whisper_model_missing",
            format!(
                "Selected Whisper model is missing. Re-download {} or choose another model.",
                catalog_model.name
            ),
        ));
    }

    let mut settings = db.get_settings()?;
    settings.selected_model_id = Some(model_id.to_string());
    db.save_settings(&settings)?;

    verify_model_file_checksum(catalog_model, &path)?;
    let record = ModelRecord {
        id: catalog_model.id.to_string(),
        name: catalog_model.name.to_string(),
        filename: catalog_model.filename.to_string(),
        local_path: Some(path.to_string_lossy().to_string()),
        size_bytes: Some(resolved.size_bytes),
        status: ModelStatus::Selected,
        checksum: catalog_model.expected_sha1.map(ToOwned::to_owned),
        selected: true,
        downloaded_at: Some(Utc::now()),
    };
    db.upsert_model_record(&record)?;
    db.mark_model_selected(model_id)?;

    let registry = DownloadRegistry::default();
    model_info_for_catalog(app, catalog_model, Some(&record), &settings, &registry)
}

pub fn selected_model_path(
    app: &AppHandle,
    db: &Database,
) -> Result<(String, PathBuf), CommandError> {
    let settings = db.get_settings()?;
    let model_id = settings
        .selected_model_id
        .as_deref()
        .unwrap_or(DEFAULT_MODEL_ID);
    let catalog_model = catalog_model(model_id)?;
    let resolved = resolve_model_file(app, catalog_model)?;

    let Some(resolved) = resolved else {
        return Err(CommandError::new(
            "whisper_model_missing",
            format!(
                "Selected Whisper model is missing. Re-download {} or place {} in a known model cache.",
                catalog_model.name, catalog_model.filename
            ),
        ));
    };

    Ok((catalog_model.id.to_string(), resolved.path))
}

fn download_model_inner(
    app: &AppHandle,
    db: &Database,
    cancel_flag: Arc<AtomicBool>,
    model_id: &str,
) -> Result<ModelInfo, CommandError> {
    let catalog_model = catalog_model(model_id)?;
    let model_dir = models_dir(app)?;
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
        checksum: catalog_model.expected_sha1.map(ToOwned::to_owned),
        selected: db.get_settings()?.selected_model_id.as_deref() == Some(catalog_model.id),
        downloaded_at: None,
    };
    db.upsert_model_record(&record)?;

    let download_result = stream_download(app, catalog_model, &part_path, cancel_flag);
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
            db.upsert_model_record(&record)?;
            emit_progress(
                app,
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
            db.upsert_model_record(&record)?;
            return Err(error);
        }
    }

    let settings = db.get_settings()?;
    let registry = DownloadRegistry::default();
    model_info_for_catalog(app, catalog_model, Some(&record), &settings, &registry)
}

#[derive(Debug)]
struct DownloadedModel {
    bytes_downloaded: u64,
    total_bytes: Option<u64>,
}

fn stream_download(
    app: &AppHandle,
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
    let mut file = fs::File::create(part_path).map_err(|error| {
        CommandError::new(
            "model_download_failed",
            format!("Could not create partial model file. {}", error),
        )
    })?;
    let mut hasher = Sha1::new();
    let mut bytes_downloaded = 0_u64;
    let mut buffer = [0_u8; 1024 * 128];

    emit_progress(
        app,
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
        emit_progress(
            app,
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

    if let Some(expected_sha1) = catalog_model.expected_sha1 {
        let actual_sha1 = format!("{:x}", hasher.finalize());
        if actual_sha1 != expected_sha1 {
            return Err(CommandError::new(
                "model_checksum_mismatch",
                format!(
                    "Downloaded {} did not match the expected checksum. Retry the download.",
                    catalog_model.name
                ),
            ));
        }
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
    let Some(expected_sha1) = catalog_model.expected_sha1 else {
        return Ok(());
    };

    let mut file = fs::File::open(path).map_err(|error| {
        CommandError::new(
            "whisper_model_missing",
            format!("Could not read {}. {}", path.display(), error),
        )
    })?;
    let mut hasher = Sha1::new();
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

    let actual_sha1 = format!("{:x}", hasher.finalize());
    if actual_sha1 != expected_sha1 {
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

fn emit_progress(
    app: &AppHandle,
    model_id: &str,
    bytes_downloaded: u64,
    total_bytes: Option<u64>,
    status: ModelStatus,
) {
    let percent = total_bytes
        .filter(|total| *total > 0)
        .map(|total| (bytes_downloaded as f64 / total as f64 * 100.0).min(100.0));
    let _ = app.emit(
        "model://download-progress",
        ModelDownloadProgress {
            model_id: model_id.to_string(),
            bytes_downloaded,
            total_bytes,
            percent,
            status,
        },
    );
}

fn model_info_for_catalog(
    app: &AppHandle,
    catalog_model: CatalogModel,
    record: Option<&ModelRecord>,
    settings: &AppSettings,
    downloads: &DownloadRegistry,
) -> Result<ModelInfo, CommandError> {
    let resolved = resolve_model_file(app, catalog_model)?;
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
    })
}

pub fn models_dir(app: &AppHandle) -> Result<PathBuf, CommandError> {
    Ok(app_data_dir(app)?.join("models"))
}

fn app_data_model_path(
    app: &AppHandle,
    catalog_model: CatalogModel,
) -> Result<PathBuf, CommandError> {
    Ok(models_dir(app)?.join(catalog_model.filename))
}

fn partial_path(app: &AppHandle, catalog_model: CatalogModel) -> Result<PathBuf, CommandError> {
    Ok(models_dir(app)?.join(format!("{}.part", catalog_model.filename)))
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, CommandError> {
    app.path().app_data_dir().map_err(|error| {
        CommandError::new(
            "app_data_dir_unavailable",
            format!(
                "Could not locate Scribe app data directory. {}",
                error
            ),
        )
    })
}

fn resolve_model_file(
    app: &AppHandle,
    catalog_model: CatalogModel,
) -> Result<Option<ResolvedModelFile>, CommandError> {
    for (path, source) in model_file_candidates(app, catalog_model)? {
        let metadata = fs::metadata(&path)
            .ok()
            .filter(|metadata| metadata.is_file());

        if let Some(metadata) = metadata {
            return Ok(Some(ResolvedModelFile {
                path,
                source,
                size_bytes: metadata.len(),
            }));
        }
    }

    Ok(None)
}

/// Path of the smallest downloaded model, for cheap operations that only need
/// *a* model loaded rather than the user's selected one — e.g. the GPU device
/// probe, where device enumeration is model-independent so loading a multi-GB
/// model would be pure waste. Returns `None` when nothing is downloaded.
pub fn smallest_downloaded_model_path(app: &AppHandle) -> Option<PathBuf> {
    crate::models::catalog()
        .iter()
        .filter_map(|model| {
            resolve_model_file(app, *model)
                .ok()
                .flatten()
                .map(|resolved| (resolved.size_bytes, resolved.path))
        })
        .min_by_key(|(size, _)| *size)
        .map(|(_, path)| path)
}

fn model_file_candidates(
    app: &AppHandle,
    catalog_model: CatalogModel,
) -> Result<Vec<(PathBuf, ModelSource)>, CommandError> {
    let mut candidates = vec![(
        app_data_model_path(app, catalog_model)?,
        ModelSource::AppData,
    )];

    candidates.extend(
        external_model_dirs()
            .into_iter()
            .map(|dir| (dir.join(catalog_model.filename), ModelSource::ExternalCache)),
    );

    Ok(candidates)
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
                "Unknown Whisper model '{}'. Choose a model from the catalog.",
                model_id
            ),
        )
    })
}
