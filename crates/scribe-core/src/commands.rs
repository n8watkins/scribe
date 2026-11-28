//! The old home of the Tauri commands and the state they share.
//!
//! The commands moved to the shell (app/src-tauri/src/commands, one file per
//! domain); their shared state is in [`crate::backend`] and their helpers in
//! the modules they belong to. These re-exports keep the old paths
//! (`crate::commands::BackendState` and the launch-at-startup helpers the
//! shell's tray and window events call) resolving.

pub use crate::backend::{Backend, BackendState};
pub use crate::replacements_file::REPLACEMENTS_CHANGED_EVENT;
