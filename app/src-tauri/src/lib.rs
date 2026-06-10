pub mod app_state;
pub mod commands;
pub mod db;
pub mod error;
pub mod settings;
pub mod stats;
pub mod transcript;

use commands::BackendState;
use db::Database;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let db = Database::open(app_data_dir.join("localdictate.sqlite3"))?;
            app.manage(BackendState::new(db));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::get_settings,
            commands::update_settings,
            commands::get_last_transcript,
            commands::clear_last_transcript,
            commands::list_recent_transcripts,
            commands::get_basic_stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
