//! The Tauri commands the windows invoke, one file per engine domain.
//!
//! Each file is owned by one lane of the core split (docs/CORE-SPLIT.md):
//! `audio`, `output`, `models`, `hotkeys`, `data`, `sync` and `status`.
//! `shell` holds the commands that belong to the shell itself. Commands keep
//! the names the windows call them by; `lib.rs` registers every one.
//!
//! The helpers the engine kept under its old `commands` module (the backend
//! state, the launch-at-startup helpers) are re-exported here, so
//! `crate::commands::BackendState` and friends still resolve.

pub mod audio;
pub mod data;
pub mod hotkeys;
pub mod models;
pub mod output;
pub mod shell;
pub mod status;
pub mod sync;

pub use scribe_core::commands::*;

use scribe_core::{core::Core, error::CommandError};

/// Runs blocking command work on Tauri's blocking thread pool. Tauri runs every
/// non-async command on the main thread, where anything slow (the database,
/// disk, the network, the clipboard, a file dialog) freezes every window, so
/// commands that touch any of those wrap their body in this.
pub(crate) async fn off_main<T, F>(app: tauri::AppHandle, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&tauri::AppHandle) -> Result<T, CommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || work(&app))
        .await
        .map_err(|error| {
            CommandError::new(
                "command_failed",
                format!("Scribe could not finish that action. {}", error),
            )
        })?
}

/// [`off_main`] for commands converted to the engine handle: runs `work`
/// with the [`Core`] on Tauri's blocking thread pool. Take the core as
/// `core: tauri::State<'_, Core>` and pass `core.inner().clone()` (a cheap
/// handle clone).
pub(crate) async fn off_main_core<T, F>(core: Core, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&Core) -> Result<T, CommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || work(&core))
        .await
        .map_err(|error| {
            CommandError::new(
                "command_failed",
                format!("Scribe could not finish that action. {}", error),
            )
        })?
}

/// Like [`off_main_core`], for commands that open a native file dialog (`rfd`).
/// Each dialog gets a fresh thread, so the dialog's COM apartment can never
/// clash with whatever an earlier task on a reused pool thread set up.
pub(crate) async fn on_dialog_thread<T, F>(core: Core, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&Core) -> Result<T, CommandError> + Send + 'static,
{
    off_main_core(core, move |core| {
        let core = core.clone();
        std::thread::Builder::new()
            .name("scribe-file-dialog".to_string())
            .spawn(move || work(&core))
            .map_err(|error| {
                CommandError::new(
                    "command_failed",
                    format!("Scribe could not open the file dialog. {}", error),
                )
            })?
            .join()
            .map_err(|_| {
                CommandError::new("command_failed", "The file dialog closed unexpectedly.")
            })?
    })
    .await
}
