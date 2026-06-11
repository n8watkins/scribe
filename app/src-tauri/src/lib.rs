pub mod app_state;
pub mod audio;
pub mod commands;
pub mod db;
pub mod dictation;
pub mod error;
pub mod hotkeys;
pub mod model_manager;
pub mod models;
pub mod output;
pub mod settings;
pub mod stats;
pub mod transcript;
pub mod tray;
pub mod whisper;

use commands::BackendState;
use db::Database;
use tauri::Manager;

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
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let audio_temp_dir = app.path().app_cache_dir()?.join("recordings");
            std::fs::create_dir_all(&app_data_dir)?;
            std::fs::create_dir_all(&audio_temp_dir)?;
            let db = Database::open(app_data_dir.join("localdictate.sqlite3"))?;
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
            app.manage(BackendState::new(db, audio_temp_dir));
            hotkeys::setup(app.handle(), &settings.hotkeys)?;
            tray::setup(app.handle())?;
            log::info!("LocalDictate setup complete");
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
            commands::get_basic_stats,
            commands::refresh_basic_stats,
            commands::get_hotkey_status,
            commands::rebind_hotkey,
            commands::reset_hotkeys_to_defaults,
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
            commands::paste_last_transcript,
            commands::copy_last_transcript,
            commands::paste_transcript,
            commands::copy_transcript,
            commands::list_models,
            commands::download_model,
            commands::cancel_model_download,
            commands::retry_model_download,
            commands::delete_model,
            commands::select_model
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
