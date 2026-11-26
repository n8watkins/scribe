//! The engine as one handle.
//!
//! [`Core`] owns the long-lived parts of the engine (the backend state, the
//! warm speech engine, the workers, the dictation-state server, the hotkey
//! runtime, the replacements) and the [`Host`] it runs under. It is a cheap
//! handle: clones share the same engine, so a command can move one onto a
//! worker thread.
//!
//! For now the shell also manages each part in Tauri state, so the existing
//! `app.state::<X>()` lookups find the very same instance `Core` holds (each
//! part is itself a cheap handle onto shared state). Engine code moves from
//! those lookups to `&Core` module by module (docs/CORE-SPLIT.md); once none
//! is left, only `Core` stays managed.

use std::sync::Arc;

use crate::{
    commands::BackendState,
    dictation_worker::DictationWorker,
    events::CoreEvent,
    host::{Flavor, Host},
    hotkeys::HotkeyRuntimeState,
    note_sync::NoteSyncWorker,
    paths::Paths,
    replacements_file::ReplacementStore,
    state_server::DictationServer,
    whisper_server::WarmTranscriber,
};

/// The parts a [`Core`] is built from, in the order the shell creates them
/// at startup.
pub struct CoreParts {
    pub backend: BackendState,
    pub warm_transcriber: WarmTranscriber,
    /// None when the dictation-state server could not start (it is
    /// best-effort).
    pub dictation_server: Option<DictationServer>,
    pub note_sync: NoteSyncWorker,
    pub dictation_worker: DictationWorker,
    pub hotkey_runtime: HotkeyRuntimeState,
    pub replacements: Arc<ReplacementStore>,
}

/// The engine: its parts and the host it runs under.
#[derive(Clone)]
pub struct Core(Arc<Inner>);

struct Inner {
    host: Arc<dyn Host>,
    parts: CoreParts,
}

impl Core {
    pub fn new(host: Arc<dyn Host>, parts: CoreParts) -> Self {
        Self(Arc::new(Inner { host, parts }))
    }

    /// The shell this engine runs under.
    pub fn host(&self) -> &Arc<dyn Host> {
        &self.0.host
    }

    /// Sends an event to the interface.
    pub fn emit(&self, event: CoreEvent) {
        self.0.host.emit(event);
    }

    /// The folders the engine uses.
    pub fn paths(&self) -> &Paths {
        self.0.host.paths()
    }

    /// Stable or Dev.
    pub fn flavor(&self) -> Flavor {
        self.0.host.flavor()
    }

    /// The database, the dictation state machine, the audio service and the
    /// other state the commands share.
    pub fn backend(&self) -> &BackendState {
        &self.0.parts.backend
    }

    /// The resident speech engine.
    pub fn warm_transcriber(&self) -> &WarmTranscriber {
        &self.0.parts.warm_transcriber
    }

    /// The dictation-state server, when it started.
    pub fn dictation_server(&self) -> Option<&DictationServer> {
        self.0.parts.dictation_server.as_ref()
    }

    /// The worker that debounces note saves into GitHub syncs.
    pub fn note_sync(&self) -> &NoteSyncWorker {
        &self.0.parts.note_sync
    }

    /// The one thread every dictation action runs on.
    pub fn dictation_worker(&self) -> &DictationWorker {
        &self.0.parts.dictation_worker
    }

    /// The registered hotkeys and their live state.
    pub fn hotkey_runtime(&self) -> &HotkeyRuntimeState {
        &self.0.parts.hotkey_runtime
    }

    /// replacements.txt.
    pub fn replacements(&self) -> &Arc<ReplacementStore> {
        &self.0.parts.replacements
    }
}
