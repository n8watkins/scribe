//! Commands of the `status` domain: the dictation state the interface and
//! other apps read (app_state, state_server, status_file).
//!
//! Converted to the engine handle: commands here take the managed [`Core`]
//! instead of the `AppHandle` (docs/CORE-SPLIT.md).

use super::off_main_core;
use crate::{app_state::AppStateSnapshot, error::CommandError, Core};

#[tauri::command]
pub async fn get_app_state(core: tauri::State<'_, Core>) -> Result<AppStateSnapshot, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        Ok(core.backend().app_state()?.snapshot())
    })
    .await
}
