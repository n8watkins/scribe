//! The Tauri side of the engine's seams: [`TauriHost`], the engine's
//! [`Host`] on top of Tauri, and the folders Tauri resolves, checked against
//! the engine's own [`Paths`].

use scribe_core::{
    app_state::AppStatus,
    error::CommandError,
    events::CoreEvent,
    host::{Flavor, Host},
    paths::Paths,
    settings::AppSettings,
};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// The engine's host in the Tauri shell: events go to every webview through
/// `app.emit`, the tray and the main window are the shell's own.
pub struct TauriHost {
    app: AppHandle,
    paths: Paths,
    flavor: Flavor,
}

impl TauriHost {
    pub fn new(app: AppHandle, paths: Paths) -> Self {
        let flavor = Flavor::from_identifier(&app.config().identifier);
        Self { app, paths, flavor }
    }
}

impl Host for TauriHost {
    /// Exactly what the engine's `app.emit(name, payload)` calls send: the
    /// event serializes as its bare payload.
    fn emit(&self, event: CoreEvent) {
        let _ = self.app.emit(event.name(), &event);
    }

    fn paths(&self) -> &Paths {
        &self.paths
    }

    fn flavor(&self) -> Flavor {
        self.flavor
    }

    fn update_tray_status(&self, status: AppStatus) {
        crate::tray::update_tray_status(&self.app, status);
    }

    fn update_engine_summary(&self, summary: String) {
        crate::tray::update_engine_summary(&self.app, summary);
    }

    fn sync_settings(&self, settings: &AppSettings) {
        crate::tray::sync_settings(&self.app, settings);
    }

    fn open_dashboard(&self, route: Option<&str>) -> Result<(), CommandError> {
        crate::tray::open_dashboard(&self.app, route)
    }

    fn toggle_dashboard(&self) -> Result<(), CommandError> {
        crate::tray::toggle_dashboard(&self.app)
    }

    fn main_window_logical_size(&self) -> Option<(u32, u32)> {
        crate::window_size::current_logical(&self.app)
    }
}

/// Resolves the engine's folders for this app (its identifier and product
/// name), the same folders `app.path()` gives.
pub fn resolve_paths<R: Runtime, M: Manager<R>>(
    app: &M,
) -> Result<Paths, scribe_core::paths::UnknownFolder> {
    Paths::resolve(&app.config().identifier, &app.package_info().name)
}

/// Every folder where `paths` and Tauri's resolver disagree, as
/// "folder: engine vs Tauri" lines. Empty when they agree.
pub fn paths_mismatches<R: Runtime, M: Manager<R>>(app: &M, paths: &Paths) -> Vec<String> {
    let resolver = app.path();
    let folders = [
        (
            "data",
            Some(paths.data_dir().to_path_buf()),
            resolver.app_data_dir().ok(),
        ),
        (
            "cache",
            Some(paths.cache_dir().to_path_buf()),
            resolver.app_cache_dir().ok(),
        ),
        (
            "log",
            Some(paths.log_dir().to_path_buf()),
            resolver.app_log_dir().ok(),
        ),
        (
            "resource",
            paths.resource_dir().map(|dir| dir.to_path_buf()),
            resolver.resource_dir().ok(),
        ),
        (
            "home",
            paths.home_dir().map(|dir| dir.to_path_buf()),
            resolver.home_dir().ok(),
        ),
    ];
    folders
        .into_iter()
        .filter(|(_, ours, tauris)| ours != tauris)
        .map(|(name, ours, tauris)| format!("{name}: {ours:?} vs {tauris:?}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    fn config(source: &str) -> serde_json::Value {
        serde_json::from_str(source).unwrap()
    }

    /// A mock app carrying a flavour's identifier and product name.
    fn app_for(identifier: &str, product_name: &str) -> tauri::App<tauri::test::MockRuntime> {
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = identifier.to_string();
        context.config_mut().product_name = Some(product_name.to_string());
        context.package_info_mut().name = product_name.to_string();
        mock_builder().build(context).unwrap()
    }

    /// The identifier and product name of stable Scribe and of Scribe Dev,
    /// read from the configs the installers are built from.
    fn flavours() -> Vec<(String, String)> {
        let stable = config(include_str!("../tauri.conf.json"));
        let dev = config(include_str!("../tauri.dev-flavor.conf.json"));
        [stable, dev]
            .iter()
            .map(|config| {
                (
                    config["identifier"].as_str().unwrap().to_string(),
                    config["productName"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    /// `TauriHost::emit` sends `(event.name(), &event)`; Tauri must deliver
    /// that exactly as the engine's `app.emit(name, payload)` calls do.
    #[test]
    fn emitting_a_core_event_sends_what_the_engine_sends_today() {
        use std::sync::{Arc, Mutex};
        use tauri::Listener;

        let app = app_for("com.natkins.scribe", "Scribe");
        let received = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = received.clone();
        app.listen_any("scribe:dictation-empty", move |event| {
            sink.lock().unwrap().push(event.payload().to_string());
        });
        let payload = scribe_core::dictation::DictationEmptyPayload {
            message: "Nothing heard.".into(),
        };
        app.emit("scribe:dictation-empty", &payload).unwrap();
        let event = CoreEvent::DictationEmpty(payload);
        app.emit(event.name(), &event).unwrap();
        let received = received.lock().unwrap();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0], received[1]);
        assert_eq!(received[0], r#"{"message":"Nothing heard."}"#);
    }

    #[test]
    fn the_engine_resolves_every_folder_tauri_does() {
        let flavours = flavours();
        assert_eq!(flavours.len(), 2);
        assert_ne!(flavours[0].0, flavours[1].0);
        for (identifier, product_name) in flavours {
            let app = app_for(&identifier, &product_name);
            let paths = resolve_paths(&app).unwrap();
            assert_eq!(
                paths_mismatches(&app, &paths),
                Vec::<String>::new(),
                "{identifier}"
            );
            assert!(paths.data_dir().ends_with(&identifier));
        }
    }

    /// Tauri can move the app folders through `app > appDirectoriesOverride`;
    /// the engine's `Paths` does not follow it, so no config may set it.
    #[test]
    fn no_config_overrides_the_app_folders() {
        for source in [
            include_str!("../tauri.conf.json"),
            include_str!("../tauri.dev-flavor.conf.json"),
            include_str!("../tauri.e2e.conf.json"),
            include_str!("../tauri.linux.conf.json"),
            include_str!("../tauri.macos.conf.json"),
            include_str!("../tauri.unsigned-build.conf.json"),
        ] {
            assert!(
                !source.contains("appDirectoriesOverride"),
                "a Tauri config overrides the app folders"
            );
        }
    }
}
