//! What the engine needs from the shell it runs under.
//!
//! The engine never talks to a window, a tray or Tauri's event system
//! directly: it asks its [`Host`]. Today's shell (app/src-tauri) implements
//! `Host` on top of Tauri (`TauriHost`); a native shell implements it again
//! without changing the engine.
//!
//! `Host` also carries what `shell::ShellHooks` used to (the tray and the
//! main window), so there is one seam rather than two. Engine code that still
//! takes an `AppHandle` reaches those through the free functions in
//! [`crate::shell`], which forward to the installed host.

use std::sync::Mutex;

use crate::{
    app_state::AppStatus, error::CommandError, events::CoreEvent, paths::Paths,
    settings::AppSettings,
};

/// Which build of Scribe this is. Stable and Dev keep separate folders,
/// hotkeys and settings so both can run side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Stable,
    Dev,
}

impl Flavor {
    /// The flavour an app identifier belongs to: the Dev flavour's identifier
    /// ends in `.dev` (`tauri.dev-flavor.conf.json`).
    pub fn from_identifier(identifier: &str) -> Self {
        if identifier.ends_with(".dev") {
            Self::Dev
        } else {
            Self::Stable
        }
    }

    /// The app identifier, which names the data, cache and log folders.
    pub fn identifier(self) -> &'static str {
        match self {
            Self::Stable => "com.natkins.scribe",
            Self::Dev => "com.natkins.scribe.dev",
        }
    }

    /// The product name, which names the install and resources folders.
    pub fn product_name(self) -> &'static str {
        match self {
            Self::Stable => "Scribe",
            Self::Dev => "Scribe Dev",
        }
    }

    /// The name the interface shows: "Scribe" or "Scribe Dev".
    pub fn display_name(self) -> &'static str {
        self.product_name()
    }

    pub fn is_dev(self) -> bool {
        self == Self::Dev
    }
}

/// The shell the engine runs under: where events go, where the folders are,
/// which flavour this is, and the tray and main window.
pub trait Host: Send + Sync + 'static {
    /// Sends an event to every window (the shell picks how).
    fn emit(&self, event: CoreEvent);

    /// The folders the engine uses.
    fn paths(&self) -> &Paths;

    /// Stable or Dev.
    fn flavor(&self) -> Flavor;

    /// The tray's status word (tooltip and title).
    fn update_tray_status(&self, status: AppStatus);

    /// The tray tooltip's speech-model line.
    fn update_engine_summary(&self, summary: String);

    /// The tray menu's "Launch at startup" tick.
    fn sync_settings(&self, settings: &AppSettings);

    /// Shows and focuses the main window on a route ("dashboard" when None).
    fn open_dashboard(&self, route: Option<&str>) -> Result<(), CommandError>;

    /// Hides the main window when it is in front, else opens it.
    fn toggle_dashboard(&self) -> Result<(), CommandError>;

    /// The main window's inner size in logical pixels.
    fn main_window_logical_size(&self) -> Option<(u32, u32)>;
}

/// The error a host with no main window answers window calls with.
pub fn no_main_window() -> CommandError {
    CommandError::new("window_not_found", "Could not find the main window.")
}

/// A host with no windows and no tray that keeps every event it is sent,
/// for tests (in this crate and in the shell). Window calls answer as they
/// do with no main window.
pub struct RecordingHost {
    paths: Paths,
    flavor: Flavor,
    events: Mutex<Vec<CoreEvent>>,
}

impl RecordingHost {
    pub fn new(paths: Paths, flavor: Flavor) -> Self {
        Self {
            paths,
            flavor,
            events: Mutex::new(Vec::new()),
        }
    }

    /// The events sent so far, oldest first.
    pub fn events(&self) -> Vec<CoreEvent> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Host for RecordingHost {
    fn emit(&self, event: CoreEvent) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
    }

    fn paths(&self) -> &Paths {
        &self.paths
    }

    fn flavor(&self) -> Flavor {
        self.flavor
    }

    fn update_tray_status(&self, _status: AppStatus) {}

    fn update_engine_summary(&self, _summary: String) {}

    fn sync_settings(&self, _settings: &AppSettings) {}

    fn open_dashboard(&self, _route: Option<&str>) -> Result<(), CommandError> {
        Err(no_main_window())
    }

    fn toggle_dashboard(&self) -> Result<(), CommandError> {
        Err(no_main_window())
    }

    fn main_window_logical_size(&self) -> Option<(u32, u32)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flavour_follows_the_identifier() {
        assert_eq!(
            Flavor::from_identifier("com.natkins.scribe"),
            Flavor::Stable
        );
        assert_eq!(
            Flavor::from_identifier("com.natkins.scribe.dev"),
            Flavor::Dev
        );
        for flavor in [Flavor::Stable, Flavor::Dev] {
            assert_eq!(Flavor::from_identifier(flavor.identifier()), flavor);
        }
    }

    #[test]
    fn the_flavours_match_the_tauri_configs() {
        let stable: serde_json::Value =
            serde_json::from_str(include_str!("../../../app/src-tauri/tauri.conf.json")).unwrap();
        let dev: serde_json::Value = serde_json::from_str(include_str!(
            "../../../app/src-tauri/tauri.dev-flavor.conf.json"
        ))
        .unwrap();
        for (config, flavor) in [(stable, Flavor::Stable), (dev, Flavor::Dev)] {
            assert_eq!(config["identifier"], flavor.identifier());
            assert_eq!(config["productName"], flavor.product_name());
        }
    }

    #[test]
    fn a_recording_host_keeps_what_it_is_sent() {
        let host = RecordingHost::new(
            Paths::from_parts("/d".into(), "/c".into(), "/l".into(), None, None),
            Flavor::Dev,
        );
        host.emit(CoreEvent::SelectionTransformed("done".into()));
        let events = host.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name(), "scribe:selection-transformed");
        assert_eq!(host.flavor(), Flavor::Dev);
        assert!(host.open_dashboard(None).is_err());
        assert_eq!(host.main_window_logical_size(), None);
    }
}
