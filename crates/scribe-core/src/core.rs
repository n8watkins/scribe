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

use std::sync::{Arc, OnceLock, Weak};

use crate::{
    backend::BackendState,
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

    /// Stops the engine when the app exits. Every shell must call this once,
    /// on its way out: nothing in the engine stops on drop (the handles are
    /// shared, and Tauri never drops its managed state at exit).
    ///
    /// In order: the status file says Scribe is idle (so a reader is not
    /// stranded on Recording), the dictation-state server closes its streams
    /// and removes its discovery file, and the speech engine process stops.
    /// Safe to call more than once.
    pub fn shutdown(&self) {
        shutdown_in_order(
            self.0.host.as_ref(),
            self.0
                .parts
                .dictation_server
                .as_ref()
                .map(|s| s as &dyn Stop),
            &self.0.parts.warm_transcriber,
        );
    }
}

/// Hands the [`Core`] to the parts built before it (the dictation and
/// note-sync workers), without letting them keep it alive.
///
/// The shell makes an empty cell, gives a clone to each worker, builds the
/// `Core` and fills the cell with it. A worker calls [`CoreCell::get`] once
/// per job (or per sync) and drops the `Core` it got when the job ends; it
/// never stores one. The cell holds only a weak reference, so once every
/// `Core` handle is dropped, `get` answers `None` again, the worker senders
/// the `Core` held are dropped with it, and the worker threads end.
#[derive(Clone, Default)]
pub struct CoreCell(WeakCell<Inner>);

impl CoreCell {
    /// An empty cell: [`CoreCell::get`] answers `None` until it is filled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores a weak reference to `core`. Filling a cell twice keeps the first
    /// `Core` and logs a warning.
    pub fn fill(&self, core: &Core) {
        if !self.0.fill(&core.0) {
            log::warn!("The engine was handed to its workers twice; kept the first");
        }
    }

    /// The `Core`: `None` before the cell is filled, and again once every
    /// `Core` handle has been dropped.
    pub fn get(&self) -> Option<Core> {
        self.0.get().map(Core)
    }
}

/// A cheap `Clone` handle onto one shared, set-once weak reference.
struct WeakCell<T>(Arc<OnceLock<Weak<T>>>);

impl<T> Default for WeakCell<T> {
    fn default() -> Self {
        Self(Arc::new(OnceLock::new()))
    }
}

impl<T> Clone for WeakCell<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> WeakCell<T> {
    /// Stores a weak reference to `value`; false (and nothing changes) when
    /// the cell was already filled.
    fn fill(&self, value: &Arc<T>) -> bool {
        self.0.set(Arc::downgrade(value)).is_ok()
    }

    fn get(&self) -> Option<Arc<T>> {
        self.0.get()?.upgrade()
    }
}

/// A part [`Core::shutdown`] stops.
trait Stop {
    fn stop(&self);
}

impl Stop for DictationServer {
    fn stop(&self) {
        self.shutdown();
    }
}

impl Stop for WarmTranscriber {
    fn stop(&self) {
        self.shutdown();
    }
}

fn shutdown_in_order(host: &dyn Host, dictation_server: Option<&dyn Stop>, warm: &dyn Stop) {
    crate::status_file::publish_idle_with(host);
    if let Some(server) = dictation_server {
        server.stop();
    }
    warm.stop();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::RecordingHost;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Records when it was stopped, and whether the status file already
    /// said idle by then.
    struct FakePart {
        name: &'static str,
        status_file: PathBuf,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Stop for FakePart {
        fn stop(&self) {
            let idle = std::fs::read_to_string(&self.status_file)
                .map(|text| text.contains("\"Idle\""))
                .unwrap_or(false);
            self.log
                .lock()
                .unwrap()
                .push(format!("{} (status idle: {})", self.name, idle));
        }
    }

    #[test]
    fn shutdown_publishes_idle_then_stops_the_server_then_the_engine() {
        let root =
            std::env::temp_dir().join(format!("scribe-core-shutdown-test-{}", std::process::id()));
        let cache = root.join("cache");
        let host = RecordingHost::new(
            Paths::from_parts(
                root.join("data"),
                cache.clone(),
                root.join("logs"),
                None,
                None,
            ),
            Flavor::Stable,
        );
        let log = Arc::new(Mutex::new(Vec::new()));
        let part = |name| FakePart {
            name,
            status_file: cache.join("status.json"),
            log: log.clone(),
        };
        let server = part("server");
        let warm = part("speech engine");

        shutdown_in_order(&host, Some(&server), &warm);
        assert_eq!(
            *log.lock().unwrap(),
            [
                "server (status idle: true)",
                "speech engine (status idle: true)"
            ]
        );

        // With no dictation-state server (it is best-effort), the engine
        // still stops.
        log.lock().unwrap().clear();
        shutdown_in_order(&host, None, &warm);
        assert_eq!(*log.lock().unwrap(), ["speech engine (status idle: true)"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    // No test can build a `Core` yet (docs/CORE-SPLIT.md), so the cell's
    // behaviour is pinned on `WeakCell`, the whole of `CoreCell` apart from
    // wrapping the `Arc<Inner>` it hands back in a `Core`.

    #[test]
    fn an_empty_core_cell_has_no_core() {
        let cell = CoreCell::new();
        assert!(cell.get().is_none());
        assert!(cell.clone().get().is_none());
    }

    #[test]
    fn a_cell_answers_none_before_it_is_filled_and_the_value_after() {
        let cell = WeakCell::<String>::default();
        let worker = cell.clone();
        assert!(worker.get().is_none());

        let core = Arc::new("engine".to_string());
        assert!(cell.fill(&core));
        // Every clone sees the fill, including one made before it.
        let got = worker.get().unwrap();
        assert!(Arc::ptr_eq(&got, &core));
        assert!(Arc::ptr_eq(&cell.clone().get().unwrap(), &core));
    }

    #[test]
    fn a_cell_filled_twice_keeps_the_first() {
        let cell = WeakCell::<String>::default();
        let first = Arc::new("first".to_string());
        let second = Arc::new("second".to_string());
        assert!(cell.fill(&first));
        assert!(!cell.fill(&second));
        assert!(Arc::ptr_eq(&cell.get().unwrap(), &first));
    }

    #[test]
    fn a_cell_keeps_no_strong_reference() {
        let cell = WeakCell::<String>::default();
        let worker = cell.clone();
        let core = Arc::new("engine".to_string());
        cell.fill(&core);
        assert_eq!(Arc::strong_count(&core), 1);

        // A worker's job holds the value only while it runs.
        let job = worker.get().unwrap();
        assert_eq!(Arc::strong_count(&core), 2);
        drop(job);
        assert_eq!(Arc::strong_count(&core), 1);

        // Once the last handle is gone, the cell answers None again, and a
        // later fill cannot bring a value back.
        let weak = Arc::downgrade(&core);
        drop(core);
        assert!(weak.upgrade().is_none());
        assert!(worker.get().is_none());
        assert!(!cell.fill(&Arc::new("late".to_string())));
        assert!(cell.get().is_none());
    }

    #[test]
    fn a_core_cell_is_cheap_to_hand_to_a_worker_thread() {
        fn send_sync_clone<T: Send + Sync + Clone + 'static>() {}
        send_sync_clone::<CoreCell>();
    }
}
