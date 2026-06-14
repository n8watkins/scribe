pub mod app_state;
pub mod audio;
pub mod commands;
pub mod db;
pub mod dictation;
pub mod error;
pub mod file_transcribe;
pub mod google_drive;
pub mod google_oauth;
mod google_secrets;
pub mod hotkeys;
pub mod incremental;
pub mod model_manager;
pub mod models;
pub mod note_analysis;
pub mod note_sync;
pub mod output;
pub mod settings;
pub mod stats;
pub mod transcript;
pub mod tray;
pub mod update_check;
pub mod whisper;
pub mod whisper_server;

use commands::BackendState;
use db::Database;
use tauri::Manager;

/// True when running the Scribe Dev flavor (identifier baked at build
/// time by `tauri.dev-flavor.conf.json`).
pub fn is_dev_flavor(app: &tauri::AppHandle) -> bool {
    app.config().identifier.ends_with(".dev")
}

/// The flavored display name, used everywhere the UI says "Scribe".
pub fn display_name(app: &tauri::AppHandle) -> &'static str {
    if is_dev_flavor(app) {
        "Scribe Dev"
    } else {
        "Scribe"
    }
}

/// Carry forward a pre-rebrand install's data after the LocalDictate->Scribe
/// rename.
///
/// Tauri derives the per-app data directory from the bundle identifier
/// (`%APPDATA%\<identifier>\`). Renaming the identifier from
/// `com.natkins.localdictate` to `com.natkins.scribe` therefore points the app
/// at a brand-new, empty directory and would strand the user's existing
/// SQLite DB, recorded clips, and downloaded Whisper models in the old folder.
/// This helper copies that data across on first run.
///
/// Best-effort by design: any failure is logged and swallowed so a migration
/// hiccup can never block app startup.
fn migrate_pre_rebrand_data(app: &tauri::AppHandle, new_data_dir: &std::path::Path) {
    // Derive the old identifier from the current one. If they match, this build
    // wasn't renamed (or is the old flavor) and there's nothing to migrate.
    let old_identifier = app.config().identifier.replace(".scribe", ".localdictate");
    if old_identifier == app.config().identifier {
        return;
    }

    // The data dir lives directly under %APPDATA%; the old install used the
    // same parent with the old identifier as the folder name.
    let Some(parent) = new_data_dir.parent() else {
        return;
    };
    let old_data_dir = parent.join(&old_identifier);
    if !old_data_dir.is_dir() {
        return;
    }

    // Idempotency is gated on a completion marker written ONLY after a full,
    // successful migration — not merely on the new DB existing. If migration is
    // interrupted after the DB lands but before clips/models copy, guarding on
    // the DB alone would skip those forever; the marker lets the next launch
    // resume the (skip-existing) clips/models copy instead.
    let marker = new_data_dir.join(".rebrand-migrated");
    if marker.exists() {
        return;
    }

    log::info!(
        "Migrating pre-rebrand data from {} to {}",
        old_data_dir.display(),
        new_data_dir.display()
    );

    let new_db = new_data_dir.join("scribe.sqlite3");
    // Track whether every step succeeded; the marker is only written if so, so
    // an incomplete migration retries on the next launch.
    let mut complete = true;

    // Copy the DB only when the destination has none yet. Stage to a temp file
    // and atomically rename into place, so a crash or failed copy can never
    // leave a truncated scribe.sqlite3 that the app would then try to open.
    if !new_db.exists() {
        let old_db = old_data_dir.join("localdictate.sqlite3");
        if old_db.is_file() {
            let staging = new_data_dir.join("scribe.sqlite3.migrating");
            let _ = std::fs::remove_file(&staging);
            match std::fs::copy(&old_db, &staging) {
                Ok(_) => {
                    // Sidecars first (SQLite recovers if they're stale), then
                    // the main DB is renamed in last as the atomic commit point.
                    for (old_name, new_name) in [
                        ("localdictate.sqlite3-wal", "scribe.sqlite3-wal"),
                        ("localdictate.sqlite3-shm", "scribe.sqlite3-shm"),
                    ] {
                        let src = old_data_dir.join(old_name);
                        if src.exists() {
                            let _ = std::fs::copy(&src, new_data_dir.join(new_name));
                        }
                    }
                    if let Err(error) = std::fs::rename(&staging, &new_db) {
                        log::warn!("Could not finalize migrated database: {}", error);
                        let _ = std::fs::remove_file(&staging);
                        complete = false;
                    }
                }
                Err(error) => {
                    log::warn!(
                        "Could not copy database during rebrand migration: {}",
                        error
                    );
                    let _ = std::fs::remove_file(&staging);
                    complete = false;
                }
            }
        }
    }

    // Clips and models: resumable (skip-existing), so re-running after an
    // interrupted migration just fills in whatever is missing.
    for subdir in ["clips", "models"] {
        let src = old_data_dir.join(subdir);
        if src.is_dir() {
            if let Err(error) = copy_dir_skip_existing(&src, &new_data_dir.join(subdir)) {
                log::warn!(
                    "Could not copy {} directory during rebrand migration: {}",
                    subdir,
                    error
                );
                complete = false;
            }
        }
    }

    if complete {
        let _ = std::fs::write(&marker, b"migrated\n");
        log::info!("Pre-rebrand data migration complete");
    } else {
        log::warn!("Rebrand migration incomplete; it will retry on the next launch");
    }
}

/// Recursively copy `from` into `to`, creating `to` (and subdirectories) as
/// needed and skipping any destination file that already exists. std-only.
fn copy_dir_skip_existing(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_skip_existing(&src, &dst)?;
        } else if !dst.exists() {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // The single-instance plugin must be the first plugin registered so
        // it can take over before anything else initializes.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            log::info!("Second instance launch detected; focusing the main window");
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .level_for("app_lib", log::LevelFilter::Debug)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                ])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    hotkeys::handle_shortcut(app, shortcut, event);
                })
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            // The macOS launcher choice is irrelevant boilerplate for this
            // Windows app.
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let audio_temp_dir = app.path().app_cache_dir()?.join("recordings");
            std::fs::create_dir_all(&app_data_dir)?;
            std::fs::create_dir_all(&audio_temp_dir)?;
            // Carry forward data from the pre-rebrand identifier so existing
            // users keep their DB/clips/models after the LocalDictate->Scribe
            // rename. Best-effort; never blocks startup.
            migrate_pre_rebrand_data(app.handle(), &app_data_dir);
            let db = Database::open(app_data_dir.join("scribe.sqlite3"))?;
            let mut settings = db.get_settings()?;
            let mut settings_migrated = false;
            // One-time migration: replace the old default hotkeys (which
            // collide with Windows shortcuts like Ctrl+Win+Space) with the
            // current defaults on installs that never customized them.
            if settings.hotkeys.migrate_legacy_defaults() {
                log::info!("Migrated legacy default hotkeys to the current defaults");
                settings_migrated = true;
            }
            // One-time migration of changed shipped defaults (e.g. the
            // default output mode moving from SaveOnly to AutoPaste).
            if settings.migrate_defaults() {
                log::info!(
                    "Migrated settings defaults to version {} (output mode now {:?})",
                    settings.defaults_version,
                    settings.output_mode
                );
                settings_migrated = true;
            }
            // The Scribe Dev flavor seeds non-conflicting hotkey defaults the
            // first time it runs, so it can run alongside stable Scribe without
            // fighting over the same global binds. Only untouched (still
            // production) binds are remapped; the flag makes this one-shot so a
            // later "Load my production defaults" sticks.
            if is_dev_flavor(app.handle()) && !settings.dev_hotkeys_seeded {
                if settings.hotkeys == crate::settings::HotkeySettings::default() {
                    settings.hotkeys = crate::settings::HotkeySettings::dev_defaults();
                    log::info!("Seeded Scribe Dev hotkey defaults");
                }
                settings.dev_hotkeys_seeded = true;
                settings_migrated = true;
            }
            if settings_migrated {
                db.save_settings(&settings)?;
            }
            log::info!(
                "Settings loaded (defaults version {}, output mode {:?}, model {:?})",
                settings.defaults_version,
                settings.output_mode,
                settings.selected_model_id
            );
            db.enforce_history_retention(settings.history_retention_days)?;
            // Reconcile the OS launch-at-startup registration with the stored
            // setting (they can drift when the registry entry is removed by
            // hand or the setting was saved while registration failed).
            {
                use tauri_plugin_autostart::ManagerExt;
                match app.autolaunch().is_enabled() {
                    Ok(enabled) if enabled != settings.launch_at_startup => {
                        log::info!(
                            "Reconciling launch-at-startup: OS registration {} but setting {}; applying the setting",
                            enabled,
                            settings.launch_at_startup
                        );
                        if let Err(error) =
                            commands::apply_autostart(app.handle(), settings.launch_at_startup)
                        {
                            log::warn!(
                                "Could not reconcile launch at startup. {}",
                                error.message
                            );
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        log::warn!("Could not read launch-at-startup state. {}", error);
                    }
                }
            }
            app.manage(BackendState::new(db, audio_temp_dir));
            app.manage(whisper_server::WarmTranscriber::new());
            // Background worker that debounces note saves into Drive auto-syncs.
            app.manage(note_sync::DriveSyncWorker::spawn(app.handle().clone()));
            // Scheduler that runs the end-of-day organize pass at the configured
            // hour (local LLM reorganizes the previous day's notes to Drive).
            app.manage(note_sync::DriveOrganizeScheduler::spawn(
                app.handle().clone(),
            ));
            hotkeys::setup(app.handle(), &settings.hotkeys)?;
            tray::setup(app.handle())?;
            // The dev flavor must be tellable from stable at a glance: the
            // window title (and taskbar entry) carries the flavored name.
            if is_dev_flavor(app.handle()) {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_title("Scribe Dev");
                }
            }
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                style_native_titlebar(&window);
            }

            log::info!("Scribe setup complete");
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }

            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let minimize_to_tray = window
                    .app_handle()
                    .try_state::<BackendState>()
                    .and_then(|state| {
                        state
                            .db()
                            .ok()
                            .and_then(|db| db.get_settings().ok())
                            .map(|settings| settings.minimize_to_tray)
                    })
                    .unwrap_or(true);

                if minimize_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::get_settings,
            commands::update_settings,
            commands::get_last_transcript,
            commands::clear_last_transcript,
            commands::list_recent_transcripts,
            commands::search_transcripts,
            commands::get_transcript,
            commands::update_transcript,
            commands::delete_transcript,
            commands::clear_transcript_history,
            commands::combine_transcripts,
            commands::save_combined_transcript,
            commands::open_transcript_externally,
            commands::get_transcript_audio,
            commands::get_basic_stats,
            commands::refresh_basic_stats,
            commands::get_hotkey_status,
            commands::rebind_hotkey,
            commands::reset_hotkeys_to_defaults,
            commands::load_production_hotkey_defaults,
            commands::open_dashboard,
            commands::list_microphones,
            commands::start_recording,
            commands::stop_recording,
            commands::cancel_recording,
            commands::record_test_clip,
            commands::get_test_clip_audio,
            commands::open_data_folder,
            commands::open_models_folder,
            commands::transcribe_recording,
            commands::transcribe_file,
            commands::analyze_note,
            commands::save_text_file,
            commands::paste_last_transcript,
            commands::copy_last_transcript,
            commands::paste_transcript,
            commands::copy_transcript,
            commands::list_models,
            commands::download_model,
            commands::cancel_model_download,
            commands::retry_model_download,
            commands::delete_model,
            commands::select_model,
            commands::check_for_update,
            commands::open_release_page,
            commands::google_status,
            commands::google_sign_in,
            commands::google_sign_out,
            commands::drive_sync_now,
            commands::drive_organize_now
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // Tear down the resident whisper-server process so no orphan
            // survives app quit (shutdown is safe to call multiple times).
            if let tauri::RunEvent::Exit = event {
                if let Some(warm) = app.try_state::<whisper_server::WarmTranscriber>() {
                    warm.shutdown();
                }
            }
        });
}

/// Colors the native Windows title bar to match the app background so the
/// window top blends into the dashboard instead of sitting as a gray strip.
#[cfg(windows)]
fn style_native_titlebar(window: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    };

    let Ok(handle) = window.hwnd() else {
        return;
    };
    let hwnd = HWND(handle.0);

    // App background #070b14 as COLORREF (0x00BBGGRR).
    let caption_color: u32 = 0x0014_0B07;
    let dark_mode: i32 = 1;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_mode as *const _ as *const _,
            std::mem::size_of::<i32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption_color as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );
    }
}
