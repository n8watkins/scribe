pub mod app_state;
pub mod audio;
pub mod commands;
pub mod db;
pub mod dictation;
pub mod dictation_state;
pub mod error;
pub mod export;
pub mod file_transcribe;
pub mod filler;
pub mod github_backup;
pub mod github_oauth;
pub mod gpu;
pub mod hotkeys;
pub mod import;
pub mod incremental;
pub mod model_manager;
pub mod models;
pub mod note_analysis;
pub mod note_sync;
pub mod output;
pub mod selection_transform;
pub mod settings;
pub mod state_server;
pub mod stats;
pub mod status_file;
pub mod text_replace;
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

/// Routes panics into the app log in addition to the default stderr behavior.
///
/// Several subsystems run on spawned threads (the audio worker, the timeout
/// thread, the hotkey chord/toggle watchers, and the GitHub note-sync
/// worker). A panic on one of those threads is only observed where a `join()`
/// exists; everywhere else it dies silently with nothing in the log file,
/// which makes field diagnosis from a user's logs nearly impossible. Chaining
/// the previous hook keeps the standard stderr backtrace while also emitting an
/// `log::error!` line (which the file logger captures) with the thread name and
/// panic location.
fn install_panic_logger() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown location".to_string());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");
        log::error!(
            "PANIC on thread '{}' at {}: {}",
            thread_name,
            location,
            message
        );
        previous(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_logger();
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
                // The plugin defaults (40 KB max, KeepOne) self-delete the log
                // every ~40 KB, so under normal dictation the file churns past
                // a few sessions within minutes and nothing older survives —
                // useless when a user reports "it failed an hour ago." Keep a
                // few MB across several rotated files, and stamp timestamps in
                // the user's local time so log lines line up with the clock
                // they saw the problem on.
                .max_file_size(5_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
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
        .plugin(tauri_plugin_notification::init())
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
            db.enforce_notes_retention(settings.notes_retention_days)?;
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
            // Offer Scribe's live dictation state to external tools over an
            // in-process loopback HTTP+SSE server. Additive and best-effort: a
            // bind failure must never block the app, so log and carry on.
            match state_server::start(app.handle()) {
                Ok(server) => {
                    log::info!("Dictation-state interface on {}", server.base_url());
                    app.manage(server);
                }
                Err(error) => {
                    log::warn!("Could not start dictation-state server: {}", error);
                }
            }
            // Background worker that debounces note saves into GitHub auto-syncs.
            app.manage(note_sync::NoteSyncWorker::spawn(app.handle().clone()));
            hotkeys::setup(app.handle(), &settings.hotkeys)?;
            tray::setup(app.handle())?;
            // Restore the user's saved default window size (physical pixels),
            // mirroring how the pill restores its saved position. Absent values
            // leave the tauri.conf.json default in place.
            if let (Some(width), Some(height)) = (settings.window_width, settings.window_height) {
                if width > 0 && height > 0 {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.set_size(tauri::PhysicalSize::new(
                            width as u32,
                            height as u32,
                        ));
                    }
                }
            }
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

            // Seed the on-disk status file with the startup (Idle) state so a
            // second app (T-Hub) always has a file to read, even before the
            // first dictation.
            status_file::publish_idle(app.handle());

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
            commands::clear_notes,
            commands::combine_transcripts,
            commands::save_combined_transcript,
            commands::open_transcript_externally,
            commands::get_transcript_audio,
            commands::get_basic_stats,
            commands::refresh_basic_stats,
            commands::get_hotkey_status,
            commands::rebind_hotkey,
            commands::set_hotkey_trigger,
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
            commands::open_logs_folder,
            commands::get_logs_dir,
            commands::open_failed_recordings_folder,
            commands::get_failed_recordings_dir,
            commands::get_data_dir,
            commands::pick_data_dir,
            commands::save_window_size,
            commands::transcribe_recording,
            commands::transcribe_file,
            commands::analyze_note,
            commands::llm_status,
            gpu::probe_gpu_devices,
            commands::save_text_file,
            commands::paste_last_transcript,
            commands::copy_last_transcript,
            commands::paste_transcript,
            commands::copy_transcript,
            commands::transform_selection,
            commands::list_models,
            commands::download_model,
            commands::cancel_model_download,
            commands::retry_model_download,
            commands::delete_model,
            commands::select_model,
            commands::check_for_update,
            commands::open_release_page,
            commands::github_status,
            commands::github_device_start,
            commands::github_device_poll,
            commands::github_device_cancel,
            commands::github_disconnect,
            commands::github_sync_now,
            commands::export_transcripts,
            commands::preview_transcript_import,
            commands::restore_transcript_import
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // Tear down the resident whisper-server process so no orphan
            // survives app quit (shutdown is safe to call multiple times).
            if let tauri::RunEvent::Exit = event {
                // Leave the status file reflecting a not-listening app so a
                // reader isn't stranded on a stale Recording/Stopping state.
                status_file::publish_idle(app);
                // Stop the dictation-state server, close live SSE streams, and
                // remove the ~/.scribe discovery file so a consumer sees Scribe
                // is gone (stream EOF and a missing control file both resolve to
                // not-dictating).
                if let Some(server) = app.try_state::<state_server::DictationServer>() {
                    server.shutdown();
                }
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
