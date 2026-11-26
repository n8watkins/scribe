//! The engine's seams against a real (mock-runtime) Tauri app: the folders
//! `Paths` resolves against Tauri's resolver, and the payloads `TauriHost`
//! emits against the engine's own `app.emit` calls.
//!
//! These live in an integration test because building a Tauri app on Windows
//! loads Common Controls v6 entry points, which only a test executable with
//! the app manifest can find; build.rs embeds it into integration tests only
//! (the shipped binary gets its manifest from tauri-build as before).

use app_lib::{
    events::CoreEvent,
    tauri_host::{app_version, autostart_plugin, paths_mismatches, resolve_paths},
};
use tauri::{
    test::{mock_builder, mock_context, noop_assets, MockRuntime},
    Emitter, Listener,
};

fn config(source: &str) -> serde_json::Value {
    serde_json::from_str(source).unwrap()
}

/// A mock app carrying a flavour's identifier and product name.
fn app_for(identifier: &str, product_name: &str) -> tauri::App<MockRuntime> {
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = identifier.to_string();
    context.config_mut().product_name = Some(product_name.to_string());
    context.package_info_mut().name = product_name.to_string();
    mock_builder().build(context).unwrap()
}

/// The identifier and product name of stable Scribe and of Scribe Dev, read
/// from the configs the installers are built from.
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

/// `TauriHost::emit` sends `(event.name(), &event)`; Tauri must deliver that
/// exactly as the engine's `app.emit(name, payload)` calls do.
#[test]
fn emitting_a_core_event_sends_what_the_engine_sends_today() {
    use std::sync::{Arc, Mutex};

    let app = app_for("com.natkins.scribe", "Scribe");
    let received = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = received.clone();
    app.listen_any("scribe:dictation-empty", move |event| {
        sink.lock().unwrap().push(event.payload().to_string());
    });
    let payload = app_lib::dictation::DictationEmptyPayload {
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

/// `TauriHost::app_version` answers the app's own version, including the one
/// a Dev or installer-upgrade build sets with `--config`, never the engine
/// crate's; and it answers the same from a worker thread (the self-test asks
/// from its own).
#[test]
fn the_hosts_version_is_the_apps_from_any_thread() {
    let mut context = mock_context(noop_assets());
    context.package_info_mut().version = "0.16.4-57".parse().unwrap();
    let app = mock_builder().build(context).unwrap();
    assert_eq!(app_version(&app), "0.16.4-57");
    assert_ne!(app_version(&app), env!("CARGO_PKG_VERSION"));

    let handle = app.handle().clone();
    let answered = std::thread::spawn(move || app_version(&handle))
        .join()
        .unwrap();
    assert_eq!(answered, "0.16.4-57");
}

/// The shell builds the autostart plugin itself, and registering it gives
/// `TauriHost`'s launch-at-login calls their manager. Neither writes nor
/// removes a startup entry.
#[test]
fn the_shell_builds_the_autostart_plugin() {
    use tauri::Manager;
    use tauri_plugin_autostart::{AutoLaunchManager, ManagerExt};

    let app = mock_builder()
        .plugin(autostart_plugin())
        .build(mock_context(noop_assets()))
        .unwrap();
    assert!(app.try_state::<AutoLaunchManager>().is_some());
    let _ = app.autolaunch();
}
