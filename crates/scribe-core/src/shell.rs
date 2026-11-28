//! The installed [`Host`], for the engine's remaining wrappers that still take
//! an `AppHandle` and so cannot reach a `Core`.
//!
//! The shell installs its host here first thing in its setup. Until then (and
//! in this crate's own tests) [`host`] answers `None`, and each wrapper answers
//! as it would with no host: it logs and does nothing, or reports
//! `host::no_main_window()`.
//!
//! Code converted to `&Core` calls `core.host()` instead; this module goes
//! once nothing calls [`host`] (docs/CORE-SPLIT.md, step 4).

use std::sync::{Arc, OnceLock};

use crate::host::Host;

static HOST: OnceLock<Arc<dyn Host>> = OnceLock::new();

/// Installs the shell's host. Only the first call counts.
pub fn install(host: Arc<dyn Host>) {
    if HOST.set(host).is_err() {
        log::warn!("The shell host was installed twice; kept the first");
    }
}

/// The installed host, once the shell's setup has started.
pub fn host() -> Option<&'static Arc<dyn Host>> {
    HOST.get()
}
