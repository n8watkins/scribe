//! The `models` lane against a real (mock-runtime) Tauri app: the engine's
//! models folder is the one Tauri's resolver named before the split, and the
//! model events reach the windows exactly as the engine's own `app.emit`
//! calls sent them.
//!
//! An integration test, because building a Tauri app on Windows needs the app
//! manifest that build.rs embeds into integration tests only.

use app_lib::{
    events::CoreEvent,
    host::{Flavor, RecordingHost},
    model_manager::{self, ModelDownloadProgress},
    models::{EngineKind, ModelStatus},
    speech_engine::{EngineStatus, LoadState},
    tauri_host::resolve_paths,
    whisper_server::ENGINE_STATUS_EVENT,
};
use tauri::{
    test::{mock_builder, mock_context, noop_assets, MockRuntime},
    Emitter, Listener, Manager,
};

/// A mock app carrying a flavour's identifier and product name.
fn app_for(flavor: Flavor) -> tauri::App<MockRuntime> {
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = flavor.identifier().to_string();
    context.config_mut().product_name = Some(flavor.product_name().to_string());
    context.package_info_mut().name = flavor.product_name().to_string();
    mock_builder().build(context).unwrap()
}

/// Before the split the models folder was `app.path().app_data_dir()` plus
/// `models`; the engine must keep finding (and downloading into) that folder.
#[test]
fn the_models_folder_is_where_tauri_put_it() {
    for flavor in [Flavor::Stable, Flavor::Dev] {
        let app = app_for(flavor);
        let host = RecordingHost::new(resolve_paths(&app).unwrap(), flavor);
        assert_eq!(
            model_manager::models_dir_with(&host),
            app.path().app_data_dir().unwrap().join("models"),
            "{flavor:?}"
        );
    }
}

/// Sends `event` the way the shell's host does and `payload` the way the
/// engine did before the split, and returns what a window received for each.
fn received_both_ways<P: serde::Serialize + Clone>(
    name: &str,
    payload: P,
    event: CoreEvent,
) -> Vec<String> {
    use std::sync::{Arc, Mutex};

    assert_eq!(event.name(), name);
    let app = app_for(Flavor::Stable);
    let received = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = received.clone();
    app.listen_any(name, move |event| {
        sink.lock().unwrap().push(event.payload().to_string());
    });
    app.emit(name, &payload).unwrap();
    app.emit(event.name(), &event).unwrap();
    let received = received.lock().unwrap().clone();
    received
}

#[test]
fn download_progress_reaches_the_windows_unchanged() {
    let payload = ModelDownloadProgress {
        model_id: "tiny.en".into(),
        bytes_downloaded: 50,
        total_bytes: Some(200),
        percent: Some(25.0),
        status: ModelStatus::Downloading,
    };
    let received = received_both_ways(
        "model://download-progress",
        payload.clone(),
        CoreEvent::ModelDownloadProgress(payload),
    );
    assert_eq!(received.len(), 2);
    assert_eq!(received[0], received[1]);
    assert_eq!(
        received[0],
        r#"{"modelId":"tiny.en","bytesDownloaded":50,"totalBytes":200,"percent":25.0,"status":"downloading"}"#
    );
}

#[test]
fn the_engine_status_reaches_the_windows_unchanged() {
    let payload = EngineStatus {
        engine: EngineKind::Whisper,
        model_id: Some("tiny.en".into()),
        model_name: Some("Tiny (English)".into()),
        state: LoadState::Loaded,
        memory_bytes: Some(123_456_789),
        keep_loaded: true,
        measured: None,
    };
    let received = received_both_ways(
        ENGINE_STATUS_EVENT,
        payload.clone(),
        CoreEvent::EngineStatus(payload),
    );
    assert_eq!(received.len(), 2);
    assert_eq!(received[0], received[1]);
    assert!(
        received[0].contains(r#""modelId":"tiny.en""#),
        "{}",
        received[0]
    );
}
