//! The one resident speech engine, and what Scribe tells people about it.
//!
//! - [`Slot`] holds at most one resident engine process (whisper-server or
//!   the Parakeet sidecar). Loading a different engine or model always stops
//!   and waits for the current process *before* the new one starts, so two
//!   models are never loaded at once.
//! - [`EngineStatus`] is what the Home dashboard, the Models page and the tray
//!   show: which model is active, whether it is loaded, and how much memory
//!   it holds right now.
//! - [`Measurements`] remembers, per model, how long the last cold load took
//!   and how much memory the loaded model used on this machine. These are the
//!   numbers in the caveat next to "Keep the speech model loaded".

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{models::EngineKind, settings::GpuAcceleration};

/// The launch configuration a resident process is bound to. A request whose
/// configuration differs needs a different process; everything else (the
/// audio, the vocabulary prompt) is sent per request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineConfig {
    pub engine: EngineKind,
    pub model_id: String,
    pub model_path: PathBuf,
    pub language: String,
    /// Whisper's translate task. A launch argument, so a change restarts it.
    pub translate: bool,
    /// `--no-gpu` when off; a launch argument.
    pub gpu: GpuAcceleration,
    /// Pinned Vulkan device (an environment variable at spawn).
    pub gpu_device_index: Option<u32>,
}

/// A resident speech engine process.
pub trait Resident {
    fn pid(&self) -> u32;
    fn config(&self) -> &EngineConfig;
    fn has_exited(&mut self) -> bool;
    /// Stops the process and waits for it to exit, so its memory is free
    /// when this returns.
    fn stop(&mut self);
}

/// What [`Slot::ensure`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ensured {
    /// A matching process was already running.
    AlreadyLoaded,
    /// A process was started (a cold load).
    Loaded,
}

/// Holds at most one resident speech engine.
pub struct Slot<R: Resident> {
    resident: Option<R>,
}

impl<R: Resident> Default for Slot<R> {
    fn default() -> Self {
        Self { resident: None }
    }
}

impl<R: Resident> Slot<R> {
    pub fn resident(&self) -> Option<&R> {
        self.resident.as_ref()
    }

    pub fn resident_mut(&mut self) -> Option<&mut R> {
        self.resident.as_mut()
    }

    pub fn is_loaded(&self) -> bool {
        self.resident.is_some()
    }

    /// Makes sure a resident matching `desired` is running. A process that
    /// exited, or that is bound to anything else (another engine, model,
    /// language or GPU choice), is stopped and waited for before `launch`
    /// runs. When `launch` fails the slot stays empty.
    pub fn ensure<E>(
        &mut self,
        desired: &EngineConfig,
        launch: impl FnOnce(&EngineConfig) -> Result<R, E>,
    ) -> Result<Ensured, E> {
        if let Some(resident) = self.resident.as_mut() {
            if resident.has_exited() {
                log::warn!(
                    "The {} speech engine exited unexpectedly; starting it again",
                    resident.config().engine.label()
                );
                self.unload();
            } else if resident.config() == desired {
                return Ok(Ensured::AlreadyLoaded);
            } else {
                log::info!(
                    "Unloading {} before loading {}",
                    resident.config().model_id,
                    desired.model_id
                );
                self.unload();
            }
        }
        debug_assert!(self.resident.is_none());
        let resident = launch(desired)?;
        self.resident = Some(resident);
        Ok(Ensured::Loaded)
    }

    /// Stops the resident process, if any. Returns true when one was running.
    pub fn unload(&mut self) -> bool {
        match self.resident.take() {
            Some(mut resident) => {
                resident.stop();
                true
            }
            None => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadState {
    /// Nothing is loaded; the model loads when the next dictation starts.
    NotLoaded,
    /// The model is being loaded right now.
    Loading,
    /// The model is loaded and ready.
    Loaded,
    /// This engine can't stay resident on this system: each dictation loads
    /// the model and lets it go.
    PerDictation,
}

/// One measurement of a model on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    /// The last cold load: from starting the process to the model answering.
    pub load_ms: Option<u64>,
    /// The most memory the loaded model held during its last stay in memory.
    pub memory_bytes: Option<u64>,
}

/// The active speech engine, for the dashboard, the Models page and the tray.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub engine: EngineKind,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub state: LoadState,
    /// Memory the loaded model holds right now (None when not loaded).
    pub memory_bytes: Option<u64>,
    /// The "Keep the speech model loaded" switch for this engine.
    pub keep_loaded: bool,
    /// What this model measured on this machine, for the caveat text.
    pub measured: Option<Measurement>,
}

impl EngineStatus {
    /// One line for the tray tooltip, like
    /// "Parakeet v3 · loaded · 0.9 GB" or
    /// "Whisper Small · not loaded, loads on next dictation".
    pub fn summary(&self) -> String {
        let name = self.model_name.as_deref().unwrap_or("No model");
        match self.state {
            LoadState::Loaded => match self.memory_bytes {
                Some(bytes) => format!("{name} · loaded · {}", format_bytes(bytes)),
                None => format!("{name} · loaded"),
            },
            LoadState::Loading => format!("{name} · loading"),
            LoadState::NotLoaded => format!("{name} · not loaded, loads on next dictation"),
            LoadState::PerDictation => format!("{name} · loads for each dictation"),
        }
    }
}

/// "0.9 GB" from 900 MB up, "374 MB" below.
pub fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= MB * 900.0 {
        format!("{:.1} GB", value / GB)
    } else {
        format!("{:.0} MB", value / MB)
    }
}

/// Per-model measurements, kept in a small JSON file in the app data folder
/// so the caveat has real numbers from the first launch after an update.
#[derive(Debug, Default)]
pub struct Measurements {
    path: Option<PathBuf>,
    by_model: HashMap<String, Measurement>,
}

impl Measurements {
    pub fn load(path: PathBuf) -> Self {
        let by_model = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path: Some(path),
            by_model,
        }
    }

    pub fn get(&self, model_id: &str) -> Option<Measurement> {
        self.by_model.get(model_id).copied()
    }

    /// Records a cold load. The memory figure starts over with each load.
    pub fn record_load(&mut self, model_id: &str, load_ms: u64) {
        let entry = self
            .by_model
            .entry(model_id.to_string())
            .or_insert(Measurement {
                load_ms: None,
                memory_bytes: None,
            });
        entry.load_ms = Some(load_ms);
        entry.memory_bytes = None;
        self.save();
    }

    /// Records the memory the loaded model holds now; keeps the highest value
    /// seen since the last load (the first transcription allocates working
    /// buffers on top of the weights). Returns true when it changed.
    pub fn record_memory(&mut self, model_id: &str, bytes: u64) -> bool {
        let entry = self
            .by_model
            .entry(model_id.to_string())
            .or_insert(Measurement {
                load_ms: None,
                memory_bytes: None,
            });
        if entry.memory_bytes.is_some_and(|known| known >= bytes) {
            return false;
        }
        entry.memory_bytes = Some(bytes);
        self.save();
        true
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&self.by_model) {
            let _ = fs::write(path, json);
        }
    }
}

/// Where the measurements live.
pub fn measurements_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("engine-measurements.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A stand-in engine process. `alive` is the set of fake processes that
    /// hold a model right now, shared by every fake.
    struct FakeResident {
        pid: u32,
        config: EngineConfig,
        exited: bool,
        alive: Arc<Mutex<Vec<u32>>>,
    }

    impl Resident for FakeResident {
        fn pid(&self) -> u32 {
            self.pid
        }
        fn config(&self) -> &EngineConfig {
            &self.config
        }
        fn has_exited(&mut self) -> bool {
            self.exited
        }
        fn stop(&mut self) {
            self.alive.lock().unwrap().retain(|pid| *pid != self.pid);
        }
    }

    struct Harness {
        alive: Arc<Mutex<Vec<u32>>>,
        next_pid: u32,
        /// The most fake processes that ever held a model at the same time.
        peak: usize,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                alive: Arc::new(Mutex::new(Vec::new())),
                next_pid: 100,
                peak: 0,
            }
        }

        fn launch(&mut self, config: &EngineConfig) -> Result<FakeResident, String> {
            self.next_pid += 1;
            let mut alive = self.alive.lock().unwrap();
            // The guarantee under test: nothing else holds a model when a new
            // engine starts loading.
            assert!(
                alive.is_empty(),
                "a model was still loaded ({alive:?}) when {} started",
                config.model_id
            );
            alive.push(self.next_pid);
            self.peak = self.peak.max(alive.len());
            Ok(FakeResident {
                pid: self.next_pid,
                config: config.clone(),
                exited: false,
                alive: self.alive.clone(),
            })
        }

        fn alive(&self) -> Vec<u32> {
            self.alive.lock().unwrap().clone()
        }
    }

    fn config(engine: EngineKind, model_id: &str) -> EngineConfig {
        EngineConfig {
            engine,
            model_id: model_id.to_string(),
            model_path: PathBuf::from(format!("models/{model_id}.bin")),
            language: "en".to_string(),
            translate: false,
            gpu: GpuAcceleration::Auto,
            gpu_device_index: None,
        }
    }

    #[test]
    fn switching_engines_never_keeps_both_models_loaded() {
        let mut harness = Harness::new();
        let mut slot: Slot<FakeResident> = Slot::default();
        let whisper = config(EngineKind::Whisper, "small.en-q5_1");
        let parakeet = config(EngineKind::Parakeet, "parakeet-tdt-0.6b-v3");

        let sequence = [
            &whisper, &whisper, &parakeet, &parakeet, &whisper, &parakeet, &whisper,
        ];
        let mut loads = 0;
        for desired in sequence {
            let outcome = slot.ensure(desired, |c| harness.launch(c)).unwrap();
            if outcome == Ensured::Loaded {
                loads += 1;
            }
            assert_eq!(
                harness.alive().len(),
                1,
                "exactly one engine after {}",
                desired.model_id
            );
            assert_eq!(slot.resident().unwrap().config(), desired);
        }
        // Repeats reuse the loaded model; every switch is one cold load.
        assert_eq!(loads, 5);
        assert_eq!(harness.peak, 1, "two models were loaded at the same time");
    }

    #[test]
    fn any_launch_setting_change_replaces_the_process_one_at_a_time() {
        let mut harness = Harness::new();
        let mut slot: Slot<FakeResident> = Slot::default();
        let base = config(EngineKind::Whisper, "small");
        slot.ensure(&base, |c| harness.launch(c)).unwrap();
        for changed in [
            EngineConfig {
                model_id: "medium".into(),
                ..base.clone()
            },
            EngineConfig {
                language: "de".into(),
                ..base.clone()
            },
            EngineConfig {
                translate: true,
                ..base.clone()
            },
            EngineConfig {
                gpu: GpuAcceleration::Off,
                ..base.clone()
            },
            EngineConfig {
                gpu_device_index: Some(1),
                ..base.clone()
            },
        ] {
            assert_eq!(
                slot.ensure(&changed, |c| harness.launch(c)).unwrap(),
                Ensured::Loaded
            );
            assert_eq!(harness.alive().len(), 1);
        }
        assert_eq!(harness.peak, 1);
    }

    #[test]
    fn unload_frees_the_model_and_a_failed_load_leaves_nothing() {
        let mut harness = Harness::new();
        let mut slot: Slot<FakeResident> = Slot::default();
        let whisper = config(EngineKind::Whisper, "small");
        slot.ensure(&whisper, |c| harness.launch(c)).unwrap();
        assert!(slot.unload());
        assert!(harness.alive().is_empty());
        assert!(!slot.unload(), "a second unload is a no-op");

        slot.ensure(&whisper, |c| harness.launch(c)).unwrap();
        let parakeet = config(EngineKind::Parakeet, "parakeet");
        let failed: Result<Ensured, String> = slot.ensure(&parakeet, |_| Err("no model".into()));
        assert!(failed.is_err());
        // The old engine was stopped before trying, and nothing replaced it.
        assert!(harness.alive().is_empty());
        assert!(!slot.is_loaded());
    }

    #[test]
    fn an_exited_engine_is_replaced() {
        let mut harness = Harness::new();
        let mut slot: Slot<FakeResident> = Slot::default();
        let whisper = config(EngineKind::Whisper, "small");
        slot.ensure(&whisper, |c| harness.launch(c)).unwrap();
        let first = slot.resident().unwrap().pid();
        slot.resident_mut().unwrap().exited = true;
        assert_eq!(
            slot.ensure(&whisper, |c| harness.launch(c)).unwrap(),
            Ensured::Loaded
        );
        assert_ne!(slot.resident().unwrap().pid(), first);
        assert_eq!(harness.alive().len(), 1);
    }

    #[test]
    fn summary_lines_read_naturally() {
        let mut status = EngineStatus {
            engine: EngineKind::Parakeet,
            model_id: Some("parakeet-tdt-0.6b-v3".into()),
            model_name: Some("Parakeet v3".into()),
            state: LoadState::Loaded,
            memory_bytes: Some(966_367_641),
            keep_loaded: true,
            measured: None,
        };
        assert_eq!(status.summary(), "Parakeet v3 · loaded · 0.9 GB");
        status.memory_bytes = Some(374 * 1024 * 1024);
        assert_eq!(status.summary(), "Parakeet v3 · loaded · 374 MB");
        status.state = LoadState::NotLoaded;
        status.model_name = Some("Whisper Small".into());
        assert_eq!(
            status.summary(),
            "Whisper Small · not loaded, loads on next dictation"
        );
    }

    #[test]
    fn measurements_persist_and_keep_the_peak_since_the_last_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = measurements_path(dir.path());
        let mut measurements = Measurements::load(path.clone());
        measurements.record_load("small", 1200);
        assert!(measurements.record_memory("small", 500));
        assert!(measurements.record_memory("small", 800));
        assert!(!measurements.record_memory("small", 700));

        let reloaded = Measurements::load(path);
        assert_eq!(
            reloaded.get("small"),
            Some(Measurement {
                load_ms: Some(1200),
                memory_bytes: Some(800)
            })
        );

        // A new load starts the memory figure over.
        let mut reloaded = reloaded;
        reloaded.record_load("small", 900);
        assert_eq!(reloaded.get("small").unwrap().memory_bytes, None);
    }
}
