//! The resident Parakeet engine: Scribe's `scribe-parakeet` sidecar
//! (app/sidecar/scribe-parakeet), built from the same reviewed whisper.cpp
//! source as whisper-server and talking JSON lines over stdin and stdout.
//!
//! The sidecar exits when its stdin closes or Scribe's pid goes away, and it
//! is started through `child_guard`, so it never outlives Scribe.

use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Stdio},
    time::Duration,
};

use crossbeam_channel::{Receiver, RecvTimeoutError};
use serde::Deserialize;
use tauri::AppHandle;

use crate::{
    filler::TimedWord,
    speech_engine::{EngineConfig, Resident},
    whisper,
};

pub const SIDECAR_EXECUTABLE: &str = "scribe-parakeet.exe";
/// Loading the model from a slow disk can take a while the first time.
const LOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Long files take a while on a CPU.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct ParakeetProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    config: EngineConfig,
    /// What the sidecar measured for loading the model itself.
    pub model_load_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ready {
    ready: bool,
    #[serde(default)]
    load_ms: u64,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Reply {
    ok: bool,
    #[serde(default)]
    text: String,
    #[serde(default)]
    words: Vec<ReplyWord>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReplyWord {
    w: String,
    s: i64,
    e: i64,
}

/// What one transcription returned: the text and its timed words.
#[derive(Debug, Clone, PartialEq)]
pub struct ParakeetResult {
    pub text: String,
    pub words: Vec<TimedWord>,
}

impl ParakeetProcess {
    /// Starts the sidecar for `config` and waits until the model is loaded.
    pub fn launch(app: &AppHandle, config: &EngineConfig) -> Result<Self, String> {
        let executable = whisper::resolve_bundled_executable(app, SIDECAR_EXECUTABLE)
            .map_err(|error| error.message)?;
        Self::launch_executable(&executable, config, threads())
    }

    pub(crate) fn launch_executable(
        executable: &Path,
        config: &EngineConfig,
        threads: usize,
    ) -> Result<Self, String> {
        let args = sidecar_args(config, threads, std::process::id());
        let mut command = std::process::Command::new(executable);
        command
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        whisper::suppress_console_window(&mut command);
        if let Some(devices) = whisper::gpu_visible_devices_env(config.gpu, config.gpu_device_index)
        {
            command.env("GGML_VK_VISIBLE_DEVICES", devices);
        }

        log::info!(
            "Starting scribe-parakeet (model {}, {} threads)",
            config.model_path.display(),
            threads
        );
        let mut child = crate::child_guard::spawn(command)
            .map_err(|error| format!("Could not start scribe-parakeet. {}", error))?;
        crate::child_guard::track(child.id());

        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "scribe-parakeet has no output pipe.".to_string())?;
        let (sender, lines) = crossbeam_channel::unbounded();
        std::thread::Builder::new()
            .name("scribe-parakeet-reader".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if sender.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("Could not read from scribe-parakeet. {}", error))?;

        let mut process = Self {
            child,
            stdin,
            lines,
            config: config.clone(),
            model_load_ms: 0,
        };
        // On failure `process` is dropped, which stops the child.
        let ready: Ready = serde_json::from_str(&process.next_line(LOAD_TIMEOUT)?)
            .map_err(|error| format!("scribe-parakeet said something unexpected. {}", error))?;
        if !ready.ready {
            return Err(ready
                .error
                .unwrap_or_else(|| "scribe-parakeet could not load the model.".to_string()));
        }
        process.model_load_ms = ready.load_ms;
        Ok(process)
    }

    fn next_line(&self, timeout: Duration) -> Result<String, String> {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => Ok(line),
            Err(RecvTimeoutError::Timeout) => Err(format!(
                "scribe-parakeet did not answer within {} seconds.",
                timeout.as_secs()
            )),
            Err(RecvTimeoutError::Disconnected) => Err("scribe-parakeet stopped.".to_string()),
        }
    }

    /// Transcribes one audio file (WAV, or anything whisper.cpp's reader
    /// takes: FLAC, MP3, OGG).
    pub fn transcribe(&mut self, wav_path: &Path) -> Result<ParakeetResult, String> {
        let request = serde_json::json!({ "wav": wav_path.to_string_lossy() });
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "scribe-parakeet is not running.".to_string())?;
        writeln!(stdin, "{request}")
            .and_then(|_| stdin.flush())
            .map_err(|error| format!("Could not send audio to scribe-parakeet. {}", error))?;
        let reply: Reply = serde_json::from_str(&self.next_line(REQUEST_TIMEOUT)?)
            .map_err(|error| format!("scribe-parakeet said something unexpected. {}", error))?;
        if !reply.ok {
            return Err(reply
                .error
                .unwrap_or_else(|| "scribe-parakeet could not transcribe the audio.".to_string()));
        }
        Ok(ParakeetResult {
            text: reply.text,
            words: reply
                .words
                .into_iter()
                .map(|word| TimedWord::new(word.w, word.s, word.e))
                .collect(),
        })
    }

    fn kill(&mut self) {
        // Closing stdin asks the sidecar to exit; the kill makes sure.
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
        crate::child_guard::untrack(self.child.id());
    }
}

impl Resident for ParakeetProcess {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn config(&self) -> &EngineConfig {
        &self.config
    }

    fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    fn stop(&mut self) {
        log::info!("Unloading {} (scribe-parakeet)", self.config.model_id);
        self.kill();
    }
}

impl Drop for ParakeetProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

fn threads() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(4)
        .saturating_sub(1)
        .clamp(1, 8)
}

fn sidecar_args(config: &EngineConfig, threads: usize, parent_pid: u32) -> Vec<String> {
    let mut args = vec![
        "--model".to_string(),
        config.model_path.to_string_lossy().to_string(),
        "--threads".to_string(),
        threads.to_string(),
        "--parent-pid".to_string(),
        parent_pid.to_string(),
    ];
    if config.gpu.is_off() {
        args.push("--no-gpu".to_string());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::EngineKind, settings::GpuAcceleration};
    use std::path::PathBuf;

    fn config() -> EngineConfig {
        EngineConfig {
            engine: EngineKind::Parakeet,
            model_id: crate::models::PARAKEET_V3_ID.to_string(),
            model_path: PathBuf::from("models/ggml-parakeet-tdt-0.6b-v3-q8_0.bin"),
            language: "auto".to_string(),
            translate: false,
            gpu: GpuAcceleration::Auto,
            gpu_device_index: None,
        }
    }

    #[test]
    fn sidecar_args_name_the_model_threads_and_parent() {
        let args = sidecar_args(&config(), 6, 4242);
        assert_eq!(
            args,
            vec![
                "--model",
                "models/ggml-parakeet-tdt-0.6b-v3-q8_0.bin",
                "--threads",
                "6",
                "--parent-pid",
                "4242"
            ]
        );
        let cpu = sidecar_args(
            &EngineConfig {
                gpu: GpuAcceleration::Off,
                ..config()
            },
            6,
            1,
        );
        assert_eq!(cpu.last().map(String::as_str), Some("--no-gpu"));
    }

    #[test]
    fn parses_a_reply_with_timed_words() {
        let reply: Reply = serde_json::from_str(
            r#"{"ok":true,"text":"And so, my","words":[{"w":"And","s":240,"e":560},{"w":"so,","s":560,"e":880}],"ms":95}"#,
        )
        .unwrap();
        assert!(reply.ok);
        assert_eq!(reply.words.len(), 2);
        assert_eq!(reply.words[1].w, "so,");
        assert_eq!((reply.words[1].s, reply.words[1].e), (560, 880));
    }

    /// Runs the real sidecar when `SCRIBE_PARAKEET_BIN` and
    /// `SCRIBE_PARAKEET_MODEL` point at a build and a model (CI's Linux job
    /// sets them); otherwise skipped.
    #[test]
    fn real_sidecar_transcribes_jfk_and_exits_when_stdin_closes() {
        let (Some(bin), Some(model)) = (
            std::env::var_os("SCRIBE_PARAKEET_BIN"),
            std::env::var_os("SCRIBE_PARAKEET_MODEL"),
        ) else {
            return;
        };
        let config = EngineConfig {
            model_path: PathBuf::from(model),
            gpu: GpuAcceleration::Off,
            ..config()
        };
        let mut process =
            ParakeetProcess::launch_executable(Path::new(&bin), &config, 4).expect("sidecar loads");
        let jfk = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jfk.wav");
        let result = process.transcribe(&jfk).expect("sidecar transcribes");
        let lower = result.text.to_lowercase();
        for word in ["fellow", "americans", "country"] {
            assert!(
                lower.contains(word),
                "{word} missing from {:?}",
                result.text
            );
        }
        assert!(result.words.len() >= 20);
        assert!(result
            .words
            .windows(2)
            .all(|pair| pair[0].start_ms <= pair[1].start_ms));

        // Closing stdin alone makes it exit.
        process.stdin = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !process.has_exited() {
            assert!(std::time::Instant::now() < deadline, "sidecar kept running");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
