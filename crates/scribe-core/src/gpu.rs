//! GPU (Vulkan) device probe.
//!
//! whisper.cpp's ggml-vulkan backend logs the Vulkan devices it finds to stderr
//! at init, e.g.:
//!
//! ```text
//! ggml_vulkan: Found 2 Vulkan devices:
//! ggml_vulkan: 0 = AMD Radeon RX 7800 XT (AMD proprietary driver) | uma: 0 | fp16: 1 | ...
//! ggml_vulkan: 1 = AMD Radeon(TM) Graphics (AMD proprietary driver) | uma: 1 | ...
//! ```
//!
//! We run a tiny whisper-cli pass over a fraction of a second of silence and
//! parse that listing, so the UI can show the detected GPU(s) and let the user
//! pin one on a multi-GPU machine (a discrete card next to an integrated one).
//!
//! Best-effort: any failure yields an empty list and the transcription paths
//! keep using ggml's own default device selection — the probe never gates
//! dictation.

use serde::Serialize;
use tauri::AppHandle;

use crate::host::Host;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VulkanDevice {
    /// ggml device index — the value to pass via `GGML_VK_VISIBLE_DEVICES`.
    pub index: u32,
    pub name: String,
    /// `uma: 1` in the ggml listing — an integrated GPU sharing system memory.
    /// The UI uses this to recommend the discrete card.
    pub integrated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuProbe {
    /// At least one Vulkan device was detected.
    pub available: bool,
    pub devices: Vec<VulkanDevice>,
    /// The probe actually ran (a model was available and the binary resolved).
    /// When false the UI shows "couldn't detect" rather than "no GPU found".
    pub probed: bool,
    /// The index the UI should suggest pinning: the first non-integrated device,
    /// else the first device. `None` when no devices were found.
    pub recommended_index: Option<u32>,
}

/// Parses ggml-vulkan's device listing out of whisper.cpp stderr. Pure so it can
/// be unit-tested against captured real output.
pub fn parse_vulkan_devices(stderr: &str) -> Vec<VulkanDevice> {
    let mut devices = Vec::new();
    for line in stderr.lines() {
        // Lines look like: "ggml_vulkan: 0 = <name> (<driver>) | uma: 0 | ...".
        let Some(rest) = line.trim().strip_prefix("ggml_vulkan:") else {
            continue;
        };
        let rest = rest.trim();
        let Some((index_str, after_eq)) = rest.split_once('=') else {
            continue;
        };
        // The "<index> =" prefix; bail unless the left side is a bare integer
        // (skips the "Found N Vulkan devices:" header and any other log lines).
        let Ok(index) = index_str.trim().parse::<u32>() else {
            continue;
        };
        // Split the name from the "| attr: val | ..." section.
        let (name_part, attrs) = match after_eq.split_once('|') {
            Some((name, attrs)) => (name, attrs),
            None => (after_eq, ""),
        };
        let name = clean_device_name(name_part);
        if name.is_empty() {
            continue;
        }
        let integrated = attrs.contains("uma: 1") || attrs.contains("uma:1");
        devices.push(VulkanDevice {
            index,
            name,
            integrated,
        });
    }
    devices
}

/// Strips a trailing "(driver ...)" tag and collapses internal whitespace, so
/// "AMD  Radeon RX 7800 XT (AMD proprietary driver)" -> "AMD Radeon RX 7800 XT".
fn clean_device_name(raw: &str) -> String {
    let mut name = raw.trim();
    if name.ends_with(')') {
        if let Some(open) = name.rfind('(') {
            name = name[..open].trim();
        }
    }
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The index to recommend pinning: first non-integrated device, else the first.
fn recommended_index(devices: &[VulkanDevice]) -> Option<u32> {
    devices
        .iter()
        .find(|device| !device.integrated)
        .or_else(|| devices.first())
        .map(|device| device.index)
}

fn probe_from_stderr(stderr: &str, probed: bool) -> GpuProbe {
    let devices = parse_vulkan_devices(stderr);
    GpuProbe {
        available: !devices.is_empty(),
        recommended_index: recommended_index(&devices),
        probed,
        devices,
    }
}

/// Writes ~0.1 s of 16 kHz mono 16-bit silence — just enough for whisper-cli to
/// reach backend init (where ggml-vulkan enumerates devices) and exit quickly.
#[cfg(windows)]
fn write_silent_wav(path: &std::path::Path) -> std::io::Result<()> {
    const SAMPLE_RATE: u32 = 16_000;
    const SAMPLES: u32 = 1_600; // 0.1 s
    let data_len = SAMPLES * 2; // 16-bit mono
    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    buf.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits/sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    buf.resize(44 + data_len as usize, 0); // silence
    std::fs::write(path, buf)
}

/// Session cache for a *successful* probe. The detected GPU set doesn't change
/// within a run, so we enumerate once and reuse it — opening the Audio tab again
/// never re-loads a model. A failed/not-probed result (e.g. no model downloaded
/// yet) is deliberately NOT cached, so it retries once a model exists.
static PROBE_CACHE: std::sync::Mutex<Option<GpuProbe>> = std::sync::Mutex::new(None);

/// The model and binary of each background probe that ran and failed, so a
/// background load does not try the same probe again this session (each try
/// unloads the resident engine). A new model or binary tries again, and a
/// probe the person asks for (the Audio page) always runs.
static FAILED_PROBES: std::sync::Mutex<Vec<ProbeInputs>> = std::sync::Mutex::new(Vec::new());

/// Who wants the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeAsker {
    /// A background load, before it loads the model: skips a probe that
    /// already failed with the same model and binary.
    BackgroundLoad,
    /// The person, on the Audio page: always tries (a failure may have been
    /// a passing one), and leaves the background memory alone.
    Person,
}

/// What a probe runs: the bundled whisper-cli on the smallest Whisper model.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProbeInputs {
    model: std::path::PathBuf,
    executable: std::path::PathBuf,
}

/// The probe's model and binary, or None when it cannot run here: not on
/// Windows (the only platform whose bundled whisper-cli lists Vulkan
/// devices), no Whisper model downloaded, or no whisper-cli.
fn probe_inputs(host: &dyn Host) -> Option<ProbeInputs> {
    #[cfg(test)]
    if let Some(inputs) = fake::inputs(host) {
        return Some(inputs);
    }
    if !cfg!(windows) {
        return None;
    }
    let model = crate::model_manager::smallest_downloaded_model_path_with(host)?;
    let executable =
        crate::whisper::resolve_bundled_executable_with(host, "whisper-cli.exe").ok()?;
    Some(ProbeInputs { model, executable })
}

/// Probe the Vulkan GPUs, using the session cache. Blocking (loads a model);
/// callers should run it off the main thread.
pub fn probe(app: &AppHandle) -> GpuProbe {
    match tauri::Manager::try_state::<crate::core::Core>(app) {
        Some(core) => probe_with(
            core.host().as_ref(),
            cfg!(windows).then_some(core.warm_transcriber()),
        ),
        None => {
            log::warn!("gpu::probe ran before the engine was ready");
            probe_from_stderr("", false)
        }
    }
}

/// [`probe`] with the resources and models of `host`. The probe loads a model
/// in whisper-cli (on Windows), so it runs through `warm` (when given) to
/// never sit next to the resident one; callers pass `warm` only where the
/// probe really runs.
pub fn probe_with(
    host: &dyn Host,
    warm: Option<&crate::whisper_server::WarmTranscriber>,
) -> GpuProbe {
    probe_using(host, ProbeAsker::Person, |probe| match warm {
        Some(warm) => warm.run_exclusive(probe),
        None => probe(),
    })
}

/// "Not probed": the result when no probe ran.
pub(crate) fn not_probed() -> GpuProbe {
    probe_from_stderr("", false)
}

/// The probe, with `run` choosing how the model run is fenced off from the
/// resident engine. `run` is called only when a probe can really run (see
/// [`probe_inputs`]) and, for a background load, has not already failed
/// with the same model and binary, so a probe that cannot succeed never
/// unloads the engine for it.
pub(crate) fn probe_using(
    host: &dyn Host,
    asker: ProbeAsker,
    run: impl FnOnce(&mut dyn FnMut() -> GpuProbe) -> GpuProbe,
) -> GpuProbe {
    if let Ok(guard) = PROBE_CACHE.lock() {
        if let Some(cached) = guard.as_ref() {
            return cached.clone();
        }
    }
    let Some(inputs) = probe_inputs(host) else {
        return not_probed();
    };
    let background = asker == ProbeAsker::BackgroundLoad;
    if background {
        let already_failed = FAILED_PROBES
            .lock()
            .map(|failed| failed.contains(&inputs))
            .unwrap_or(false);
        if already_failed {
            return not_probed();
        }
    }
    let result = run(&mut || run_probe(&inputs));
    if result.probed {
        if let Ok(mut guard) = PROBE_CACHE.lock() {
            *guard = Some(result.clone());
        }
    } else if background {
        if let Ok(mut failed) = FAILED_PROBES.lock() {
            log::warn!(
                "The GPU probe failed with {}; background loads will not try it again this \
                 session",
                inputs.model.display()
            );
            if !failed.contains(&inputs) {
                failed.push(inputs);
            }
        }
    }
    result
}

fn run_probe(inputs: &ProbeInputs) -> GpuProbe {
    #[cfg(test)]
    if let Some(result) = fake::run(inputs) {
        return result;
    }
    probe_uncached(inputs)
}

/// A probe that can run on any platform and always fails, for tests of what
/// the engine does around the probe. Keyed by the host's data folder, so
/// tests running at the same time never see each other's.
#[cfg(test)]
pub(crate) mod fake {
    use std::{
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    use super::{GpuProbe, ProbeInputs};
    use crate::host::Host;

    static FAILING: Mutex<Vec<(PathBuf, Arc<AtomicUsize>)>> = Mutex::new(Vec::new());

    /// Makes the probe for hosts with `data_dir` runnable and failing.
    /// Returns how many times it has run.
    pub(crate) fn failing_probe(data_dir: &Path) -> Arc<AtomicUsize> {
        let runs = Arc::new(AtomicUsize::new(0));
        FAILING
            .lock()
            .unwrap()
            .push((data_dir.to_path_buf(), runs.clone()));
        runs
    }

    pub(super) fn inputs(host: &dyn Host) -> Option<ProbeInputs> {
        let data_dir = host.paths().data_dir();
        let failing = FAILING.lock().unwrap();
        failing
            .iter()
            .any(|(dir, _)| dir == data_dir)
            .then(|| ProbeInputs {
                model: data_dir.join("fake-probe-model.bin"),
                executable: data_dir.join("fake-whisper-cli"),
            })
    }

    pub(super) fn run(inputs: &ProbeInputs) -> Option<GpuProbe> {
        let failing = FAILING.lock().unwrap();
        let (_, runs) = failing
            .iter()
            .find(|(dir, _)| Some(dir.as_path()) == inputs.model.parent())?;
        runs.fetch_add(1, Ordering::SeqCst);
        Some(super::not_probed())
    }
}

/// Runs the bundled whisper-cli against a sliver of silence and parses the
/// Vulkan device listing from its stderr. Windows-only (the bundled binaries are
/// Windows); elsewhere it reports "not probed".
#[cfg(windows)]
fn probe_uncached(inputs: &ProbeInputs) -> GpuProbe {
    use std::process::Command;

    let model_path = &inputs.model;
    let executable = &inputs.executable;

    let wav_path = std::env::temp_dir().join("scribe-gpu-probe.wav");
    if write_silent_wav(&wav_path).is_err() {
        return probe_from_stderr("", false);
    }

    // Homogeneous Vec<String> (mixing &str + Cow<str> in one array won't compile).
    let args = vec![
        "-m".to_string(),
        model_path.to_string_lossy().to_string(),
        "-f".to_string(),
        wav_path.to_string_lossy().to_string(),
        "--no-timestamps".to_string(),
    ];
    let mut command = Command::new(executable);
    command.args(&args);
    crate::whisper::suppress_console_window(&mut command);

    let output = crate::child_guard::output(command);
    let _ = std::fs::remove_file(&wav_path);

    match output {
        Ok(output) => probe_from_stderr(&String::from_utf8_lossy(&output.stderr), true),
        Err(_) => probe_from_stderr("", false),
    }
}

#[cfg(not(windows))]
fn probe_uncached(inputs: &ProbeInputs) -> GpuProbe {
    // Never reached: `probe_inputs` is None off Windows.
    let _ = (&inputs.model, &inputs.executable);
    // The bundled whisper binaries are Windows-only, so there's nothing to probe
    // on other platforms (dev/CI on Linux).
    probe_from_stderr("", false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no Whisper model and no whisper-cli the probe cannot run, so it
    /// must not unload the speech engine for nothing (and, from a background
    /// load, start the load and unload loop).
    #[test]
    fn a_probe_that_cannot_run_never_touches_the_engine() {
        use crate::host::{Flavor, RecordingHost};
        let root = std::env::temp_dir().join(format!("scribe-gpu-noprobe-{}", std::process::id()));
        let host = RecordingHost::new(
            crate::paths::Paths::from_parts(
                root.join("data"),
                root.join("cache"),
                root.join("logs"),
                None,
                None,
            ),
            Flavor::Stable,
        );
        for asker in [ProbeAsker::BackgroundLoad, ProbeAsker::Person] {
            let mut fenced = false;
            let result = probe_using(&host, asker, |probe| {
                fenced = true;
                probe()
            });
            assert!(
                !fenced,
                "the engine was unloaded for a probe that cannot run ({asker:?})"
            );
            assert!(!result.probed);
            assert!(result.devices.is_empty());
        }
    }

    /// A background load does not retry a probe that failed with the same
    /// model and binary; the person on the Audio page always can.
    #[test]
    fn only_background_loads_skip_a_probe_that_already_failed() {
        use crate::host::{Flavor, RecordingHost};
        let root =
            std::env::temp_dir().join(format!("scribe-gpu-failedprobe-{}", std::process::id()));
        let host = RecordingHost::new(
            crate::paths::Paths::from_parts(
                root.join("data"),
                root.join("cache"),
                root.join("logs"),
                None,
                None,
            ),
            Flavor::Stable,
        );
        let runs = fake::failing_probe(&root.join("data"));
        let probe = |asker| probe_using(&host, asker, |probe| probe());

        // The person's failures are not remembered.
        assert!(!probe(ProbeAsker::Person).probed);
        assert!(!probe(ProbeAsker::Person).probed);
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 2);
        // The first background failure is, for background loads only.
        assert!(!probe(ProbeAsker::BackgroundLoad).probed);
        assert!(!probe(ProbeAsker::BackgroundLoad).probed);
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert!(!probe(ProbeAsker::Person).probed);
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    const REAL_7800XT: &str = "\
whisper_init_with_params_no_state: use gpu    = 1
ggml_vulkan: Found 2 Vulkan devices:
ggml_vulkan: 0 = AMD  Radeon RX 7800 XT (AMD proprietary driver) | uma: 0 | fp16: 1 | bf16: 1 | warp size: 64 | matrix cores: KHR_coopmat
ggml_vulkan: 1 = AMD Radeon(TM) Graphics (AMD proprietary driver) | uma: 1 | fp16: 1 | bf16: 0 | warp size: 32 | matrix cores: none
whisper_backend_init_gpu: using Vulkan0 backend";

    #[test]
    fn parses_the_real_7800xt_listing() {
        let devices = parse_vulkan_devices(REAL_7800XT);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].index, 0);
        // Driver tag stripped, doubled space collapsed.
        assert_eq!(devices[0].name, "AMD Radeon RX 7800 XT");
        assert!(!devices[0].integrated); // uma: 0 = discrete
        assert_eq!(devices[1].index, 1);
        assert_eq!(devices[1].name, "AMD Radeon(TM) Graphics");
        assert!(devices[1].integrated); // uma: 1 = integrated
    }

    #[test]
    fn recommends_the_discrete_card_over_the_igpu() {
        let probe = probe_from_stderr(REAL_7800XT, true);
        assert!(probe.available);
        assert!(probe.probed);
        // The discrete 7800 XT (index 0, uma:0) is recommended, not the iGPU.
        assert_eq!(probe.recommended_index, Some(0));
    }

    #[test]
    fn recommends_discrete_even_when_igpu_enumerates_first() {
        // If the integrated device were index 0, the discrete one (index 1)
        // must still be the recommendation.
        let stderr = "\
ggml_vulkan: 0 = Intel Integrated (foo) | uma: 1 | fp16: 1
ggml_vulkan: 1 = NVIDIA GeForce RTX 4080 (bar) | uma: 0 | fp16: 1";
        let probe = probe_from_stderr(stderr, true);
        assert_eq!(probe.recommended_index, Some(1));
    }

    #[test]
    fn no_devices_found_is_available_false_but_probed_true() {
        let probe = probe_from_stderr("ggml_vulkan: No devices found.", true);
        assert!(!probe.available);
        assert!(probe.probed);
        assert!(probe.devices.is_empty());
        assert_eq!(probe.recommended_index, None);
    }

    #[test]
    fn header_line_and_noise_are_ignored() {
        let stderr = "\
ggml_vulkan: Found 1 Vulkan devices:
some other log line
ggml_vulkan: 0 = Test GPU (drv) | uma: 0";
        let devices = parse_vulkan_devices(stderr);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Test GPU");
    }

    #[test]
    fn name_without_driver_paren_is_kept_whole() {
        let devices = parse_vulkan_devices("ggml_vulkan: 0 = Bare Name | uma: 0");
        assert_eq!(devices[0].name, "Bare Name");
    }
}
