//! The engine as one handle.
//!
//! [`Core`] owns the long-lived parts of the engine (the backend state, the
//! warm speech engine, the workers, the dictation-state server, the hotkey
//! runtime, the replacements) and the [`Host`] it runs under. It is a cheap
//! handle: clones share the same engine, so a command can move one onto a
//! worker thread.
//!
//! The Tauri shell manages only the `Core`; each part is itself a cheap handle
//! onto shared state, so `core.backend()` and the clone a part was built with
//! are the same instance.

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
/// `Core` and fills the cell with it. A worker calls [`CoreCell::lookup`]
/// once per job (or per sync) and drops the `Core` it got when the job ends;
/// it never stores one. The cell holds only a weak reference, so once every
/// `Core` handle is dropped, `lookup` answers [`CoreLookup::Gone`], for good,
/// and the workers stop: the dictation worker answers every later job
/// `dictation_worker_unavailable`, and the note-sync thread ends at its next
/// signal.
#[derive(Clone, Default)]
pub struct CoreCell(WeakCell<Inner>);

/// What a [`CoreCell`] answers, generic so a worker can map the `Core` to
/// what it runs a job with ([`CoreLookup::map`]).
///
/// The answer moves only forward: `NotFilled`, then `Ready` once the cell is
/// filled, then `Gone` once every `Core` handle is dropped. `Gone` is final:
/// a cell keeps its first fill, and a dropped `Core` never comes back.
#[derive(Debug)]
pub enum CoreLookup<C = Core> {
    /// The shell has not built the engine yet: wait for it.
    NotFilled,
    /// The engine was built and every handle to it is gone: it never comes
    /// back.
    Gone,
    /// The engine.
    Ready(C),
}

impl<C> CoreLookup<C> {
    /// The same answer, with `Ready`'s value mapped through `f`.
    pub fn map<D>(self, f: impl FnOnce(C) -> D) -> CoreLookup<D> {
        match self {
            CoreLookup::NotFilled => CoreLookup::NotFilled,
            CoreLookup::Gone => CoreLookup::Gone,
            CoreLookup::Ready(value) => CoreLookup::Ready(f(value)),
        }
    }

    /// `Ready`'s value, or `None` for both other answers.
    pub fn ready(self) -> Option<C> {
        match self {
            CoreLookup::Ready(value) => Some(value),
            CoreLookup::NotFilled | CoreLookup::Gone => None,
        }
    }
}

impl CoreCell {
    /// An empty cell: [`CoreCell::lookup`] answers
    /// [`CoreLookup::NotFilled`] until it is filled.
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

    /// The `Core`, or why there is none: not filled yet, or gone for good.
    /// One read of the cell, so a fill or a drop that lands meanwhile can
    /// never be misread.
    pub fn lookup(&self) -> CoreLookup {
        self.0.lookup().map(Core)
    }

    /// The `Core`: `None` before the cell is filled, and again once every
    /// `Core` handle has been dropped. A worker that must tell those two
    /// apart uses [`CoreCell::lookup`].
    pub fn get(&self) -> Option<Core> {
        self.lookup().ready()
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

    fn lookup(&self) -> CoreLookup<Arc<T>> {
        match self.0.get() {
            None => CoreLookup::NotFilled,
            Some(weak) => match weak.upgrade() {
                Some(value) => CoreLookup::Ready(value),
                None => CoreLookup::Gone,
            },
        }
    }

    #[cfg(test)]
    fn get(&self) -> Option<Arc<T>> {
        self.lookup().ready()
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

/// A real [`Core`] for tests, built the way the shell builds it, on a
/// temporary folder and a [`RecordingHost`].
///
/// It has a database and replacements.txt in the folder, a speech engine that
/// loads nothing until asked, no dictation-state server, an empty hotkey
/// runtime, and both workers on the cell it fills. The dictation worker runs
/// each job on a [`TestExecutor`], so no test reaches the microphone, the
/// clipboard or the keyboard. Every test ends with `core.shutdown()`, as
/// every shell must.
#[cfg(test)]
pub(crate) mod test_core {
    use super::*;
    use crate::app_state::AppStatus;
    use crate::data_hygiene::DataDirs;
    use crate::db::Database;
    use crate::dictation_worker::{DictationCommand, Executor, Outcome};
    use crate::error::CommandError;
    use crate::host::RecordingHost;
    use std::sync::Mutex;

    /// The built engine and what a test inspects it through. Destructure it
    /// to drop the `Core` while keeping the folder (`dir`) alive.
    pub(crate) struct TestCore {
        pub(crate) core: Core,
        /// The cell both workers find the `Core` through, already filled.
        pub(crate) cell: CoreCell,
        pub(crate) host: Arc<RecordingHost>,
        /// The commands the dictation worker ran, in order (`Debug` text).
        pub(crate) ran: Arc<Mutex<Vec<String>>>,
        pub(crate) dir: tempfile::TempDir,
    }

    impl TestCore {
        /// The engine with the production note-sync worker
        /// ([`NoteSyncWorker::spawn_with`]).
        pub(crate) fn new() -> Self {
            Self::with_note_sync(NoteSyncWorker::spawn_with)
        }

        /// The engine with the note-sync worker `note_sync` starts on the
        /// cell, for a test that needs the worker's thread.
        pub(crate) fn with_note_sync(note_sync: impl FnOnce(CoreCell) -> NoteSyncWorker) -> Self {
            let dir = tempfile::tempdir().expect("a temporary folder");
            let data = dir.path().join("data");
            let cache = dir.path().join("cache");
            let logs = dir.path().join("logs");
            for folder in [&data, &cache, &logs] {
                std::fs::create_dir_all(folder).expect("the test folders");
            }
            let host = Arc::new(RecordingHost::new(
                Paths::from_parts(data.clone(), cache.clone(), logs, None, None),
                Flavor::Stable,
            ));
            let shared_host: Arc<dyn Host> = host.clone();

            let db = Database::open(data.join("scribe.sqlite3")).expect("the test database");
            let backend = BackendState::new(
                db,
                cache.join("recordings"),
                DataDirs {
                    data_dir: data.clone(),
                    cache_dir: cache,
                },
            );
            let warm_transcriber = WarmTranscriber::new_with(
                shared_host.clone(),
                backend.clone(),
                crate::speech_engine::measurements_path(&data),
            );
            let cell = CoreCell::new();
            let ran = Arc::new(Mutex::new(Vec::new()));
            let log = ran.clone();
            let dictation_worker =
                DictationWorker::spawn_on_for_tests(cell.clone(), move |core| TestExecutor {
                    core,
                    ran: log.clone(),
                })
                .expect("the dictation worker thread");
            let parts = CoreParts {
                backend,
                warm_transcriber,
                dictation_server: None,
                note_sync: note_sync(cell.clone()),
                dictation_worker,
                hotkey_runtime: HotkeyRuntimeState::default(),
                replacements: Arc::new(ReplacementStore::open(data.join("replacements.txt"), &[])),
            };
            let core = Core::new(shared_host, parts);
            cell.fill(&core);
            Self {
                core,
                cell,
                host,
                ran,
                dir,
            }
        }
    }

    /// Runs a dictation job against the real `Core`'s state without doing
    /// it: reads the status from the dictation state machine, records the
    /// command and answers `Done`.
    pub(crate) struct TestExecutor {
        core: Core,
        ran: Arc<Mutex<Vec<String>>>,
    }

    impl Executor for TestExecutor {
        fn status(&self) -> Result<AppStatus, CommandError> {
            Ok(self.core.backend().app_state()?.status().clone())
        }

        fn execute(&self, command: DictationCommand) -> Result<Outcome, CommandError> {
            self.ran
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(format!("{command:?}"));
            Ok(Outcome::Done)
        }

        fn report(&self, error: CommandError) {
            log::warn!("Test dictation job failed: {}", error.message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_core::TestCore;
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

    #[test]
    fn an_empty_core_cell_has_no_core() {
        let cell = CoreCell::new();
        assert!(matches!(cell.lookup(), CoreLookup::NotFilled));
        assert!(cell.get().is_none());
        assert!(cell.clone().get().is_none());
    }

    #[test]
    fn a_core_cell_on_a_real_core_answers_not_filled_then_ready_then_gone() {
        let TestCore {
            core,
            cell: filled,
            dir: _dir,
            ..
        } = TestCore::new();

        // A cell made before the fill, and every clone of it, sees it.
        let cell = CoreCell::new();
        let worker = cell.clone();
        assert!(matches!(worker.lookup(), CoreLookup::NotFilled));
        cell.fill(&core);
        let CoreLookup::Ready(found) = worker.lookup() else {
            panic!("a filled cell answers Ready");
        };
        assert!(Arc::ptr_eq(&found.0, &core.0));
        assert!(Arc::ptr_eq(&worker.get().unwrap().0, &core.0));
        drop(found);

        // The cell holds no strong reference: the test's handle is the only
        // one between jobs.
        assert_eq!(Arc::strong_count(&core.0), 1);

        // A second fill keeps the first `Core`.
        let other = TestCore::new();
        filled.fill(&other.core);
        assert!(Arc::ptr_eq(&filled.get().unwrap().0, &core.0));
        other.core.shutdown();
        drop(other);

        core.shutdown();
        let weak = Arc::downgrade(&core.0);
        drop(core);
        assert!(weak.upgrade().is_none(), "nothing else holds the engine");
        // Gone, not NotFilled, on every cell that held it, and for good.
        for cell in [&cell, &worker, &filled] {
            assert!(matches!(cell.lookup(), CoreLookup::Gone));
            assert!(cell.get().is_none());
        }
        let late = TestCore::new();
        cell.fill(&late.core);
        assert!(matches!(cell.lookup(), CoreLookup::Gone));
        late.core.shutdown();
    }

    #[test]
    fn a_lookup_maps_only_a_ready_value() {
        assert!(matches!(
            CoreLookup::Ready(2).map(|n| n * 10),
            CoreLookup::Ready(20)
        ));
        assert!(matches!(
            CoreLookup::<u8>::NotFilled.map(|n| n * 10),
            CoreLookup::NotFilled
        ));
        assert!(matches!(
            CoreLookup::<u8>::Gone.map(|n| n * 10),
            CoreLookup::Gone
        ));
        assert_eq!(CoreLookup::Ready(3).ready(), Some(3));
        assert_eq!(CoreLookup::<u8>::Gone.ready(), None);
        assert_eq!(CoreLookup::<u8>::NotFilled.ready(), None);
    }

    /// The panic message of a thread, whichever form the payload took.
    fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
        match payload.downcast::<String>() {
            Ok(text) => *text,
            Err(payload) => payload
                .downcast::<&'static str>()
                .map(|text| text.to_string())
                .unwrap_or_default(),
        }
    }

    /// A real command path (the window-focus and tray handler's) on a thread
    /// named "main" is caught before it takes the settings write lock.
    #[cfg(debug_assertions)]
    #[test]
    fn syncing_launch_at_startup_on_the_main_thread_panics_in_debug_builds() {
        let TestCore {
            core,
            host,
            dir: _dir,
            ..
        } = TestCore::new();
        let on_main = core.clone();
        let panic = std::thread::Builder::new()
            .name("main".into())
            .spawn(move || {
                let _ = crate::settings_store::sync_launch_at_startup_with(&on_main);
            })
            .expect("the thread")
            .join()
            .unwrap_err();
        assert_eq!(
            panic_text(panic),
            "the settings write lock must never be taken on the main thread"
        );
        // The assertion fires before the lock is taken, so it is not
        // poisoned, and no settings were announced.
        assert!(core.backend().settings_writes().try_lock().is_ok());
        assert!(!host
            .events()
            .iter()
            .any(|event| event.name() == "scribe:settings-changed"));

        // Off the main thread the same path runs.
        let off_main = core.clone();
        std::thread::Builder::new()
            .name("launch-at-startup".into())
            .spawn(move || crate::settings_store::sync_launch_at_startup_with(&off_main))
            .expect("the thread")
            .join()
            .expect("no panic")
            .expect("the settings");
        core.shutdown();
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
