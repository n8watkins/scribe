//! Scribe's engine: recording, transcription, output, storage, sync and the
//! interfaces other tools read. The desktop shell (app/src-tauri) builds the
//! Tauri app, its windows and tray on top of it and re-exports these modules
//! under their old `app_lib::` paths.
//!
//! For now the engine still runs inside Tauri (it takes an `AppHandle` and
//! emits Tauri events). What it needs from the windows and the tray goes
//! through [`shell`], which the desktop shell fills in at startup.

pub mod app_state;
pub mod audio;
pub mod autostart;
pub mod child_guard;
pub mod commands;
pub mod data_hygiene;
pub mod db;
pub mod diagnostics;
pub mod dictation;
pub mod dictation_state;
pub mod dictation_worker;
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
pub mod parakeet;
pub mod platform;
pub mod process_info;
pub mod prompt_budget;
pub mod release_notes;
pub mod remote_command;
pub mod replacements_file;
pub mod selection_transform;
pub mod self_test;
pub mod session_model;
pub mod settings;
pub mod settings_store;
pub mod shell;
pub mod sounds;
pub mod speech_engine;
pub mod state_server;
pub mod stats;
pub mod status_file;
pub mod sync_history;
#[cfg(feature = "e2e-audio")]
pub mod test_audio;
pub mod text_replace;
pub mod transcript;
pub mod update_check;
pub mod whisper;
pub mod whisper_server;

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
