pub mod app_state;
pub mod audio;
pub mod commands;
pub mod db;
pub mod error;
pub mod hotkeys;
pub mod settings;
pub mod stats;
pub mod transcript;
pub mod tray;

use commands::BackendState;
use db::Database;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            let settings = db.get_settings()?;
            app.manage(BackendState::new(db, audio_temp_dir));
            hotkeys::setup(app.handle(), &settings.hotkeys)?;
            tray::setup(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::get_settings,
            commands::update_settings,
            commands::get_last_transcript,
            commands::clear_last_transcript,
            commands::list_recent_transcripts,
            commands::get_basic_stats,
            commands::get_hotkey_status,
            commands::rebind_hotkey,
            commands::reset_hotkeys_to_defaults,
            commands::open_dashboard,
            commands::list_microphones,
            commands::start_recording,
            commands::stop_recording,
            commands::cancel_recording,
            commands::record_test_clip
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
