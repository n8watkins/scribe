//! Commands of the `status` domain: the dictation state the interface and
//! other apps read (app_state, state_server, status_file).

use tauri::Manager;

use super::off_main;
use crate::{app_state::AppStateSnapshot, commands::BackendState, error::CommandError};

#[tauri::command]
pub async fn get_app_state(app: tauri::AppHandle) -> Result<AppStateSnapshot, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        Ok(state.app_state()?.snapshot())
    })
    .await
}
