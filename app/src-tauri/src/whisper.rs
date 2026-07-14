use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::error::CommandError;

#[derive(Debug, Clone)]
pub struct WhisperRequest {
    pub model_path: PathBuf,
    pub wav_path: PathBuf,
    pub language: String,
    /// Run Whisper's translate task (`--translate`): output English regardless
    /// of the spoken language. Requires a multilingual model.
    pub translate: bool,
    /// Custom vocabulary / spelling hints; passed as `--prompt` when the
    /// trimmed value is non-empty.
    pub vocabulary_prompt: String,
    /// FILLER: when set, transcription takes the word-timestamp path
    /// (`--output-json-full` -> parse -> pause-aware filler removal). `None`
    /// leaves the plain-text path byte-for-byte unchanged.
    pub filler: Option<crate::filler::FillerConfig>,
    /// GPU (Vulkan) acceleration preference. `Off` appends `--no-gpu`; otherwise
    /// the Vulkan-enabled binaries use the GPU when present (with CPU fallback).
    pub gpu: crate::settings::GpuAcceleration,
    /// Vulkan device to pin via `GGML_VK_VISIBLE_DEVICES` (None = ggml's default
    /// device, normally the discrete card). Ignored when `gpu` is `Off`.
    pub gpu_device_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhisperTranscription {
    pub text: String,
    pub latency_ms: u32,
}

pub fn transcribe(
    app: &AppHandle,
    request: WhisperRequest,
) -> Result<WhisperTranscription, CommandError> {
    if !request.model_path.is_file() {
        return Err(CommandError::new(
            "whisper_model_missing",
            format!(
                "Selected Whisper model is missing at {}. Re-download the model or choose another model.",
                request.model_path.display()
            ),
        ));
    }

    if !request.wav_path.is_file() {
        return Err(CommandError::new(
            "recording_wav_missing",
            format!(
                "Recording WAV is missing at {}. Record again.",
                request.wav_path.display()
            ),
        ));
    }

    let output_prefix = output_prefix_for_wav(&request.wav_path);
    transcribe_with_output_prefix(app, &request, &output_prefix)
}

/// Direct whisper-cli invocation with a caller-chosen output prefix, so the
/// transient .txt never has to live next to the input file (file
/// transcription may read from directories the app should not write to).
pub(crate) fn transcribe_with_output_prefix(
    app: &AppHandle,
    request: &WhisperRequest,
    output_prefix: &Path,
) -> Result<WhisperTranscription, CommandError> {
    let executable = resolve_bundled_executable(app, "whisper-cli.exe")?;
    let output_txt_path = output_prefix.with_extension("txt");
    let mut args = whisper_args(
        &request.model_path,
        &request.wav_path,
        &request.language,
        request.translate,
        output_prefix,
        &request.vocabulary_prompt,
        request.filler.is_some(), // FILLER: word timestamps only when needed
    );
    push_gpu_args(&mut args, request.gpu.is_off());
    let started = Instant::now();
    let output = run_whisper_command(
        &executable,
        &args,
        gpu_visible_devices_env(request.gpu, request.gpu_device_index),
    )?;
    let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;

    // FILLER: the word-timestamp path writes <prefix>.json; the plain path .txt.
    let json_path = output_prefix.with_extension("json");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        let _ = fs::remove_file(&output_txt_path);
        let _ = fs::remove_file(&json_path);
        return Err(CommandError::new(
            "whisper_transcription_failed",
            format!("Whisper transcription failed. {}", detail),
        ));
    }

    // FILLER: with a config, parse word timings, drop pause-bracketed fillers,
    // then normalize. If the JSON is unreadable/empty, fall back to plain text so
    // a parse hiccup never loses the dictation.
    let text = match &request.filler {
        Some(config) => {
            let words = fs::read_to_string(&json_path)
                .map(|json| parse_cli_words(&json))
                .unwrap_or_default();
            if words.is_empty() {
                parse_output_text(&output_txt_path, &output.stdout)?
            } else {
                config.apply(&words) // normalizes internally
            }
        }
        None => parse_output_text(&output_txt_path, &output.stdout)?,
    };
    let _ = fs::remove_file(&output_txt_path);
    let _ = fs::remove_file(&json_path);

    Ok(WhisperTranscription { text, latency_ms })
}

/// Resolves a bundled whisper.cpp executable (e.g. `whisper-cli.exe` or
/// `whisper-server.exe`) under `$RESOURCE/bin/windows/`.
pub(crate) fn resolve_bundled_executable(
    app: &AppHandle,
    file_name: &str,
) -> Result<PathBuf, CommandError> {
    let resource_dir = app.path().resource_dir().map_err(|error| {
        CommandError::new(
            "missing_whisper_executable",
            format!("Could not locate bundled app resources. {}", error),
        )
    })?;
    let executable = resource_dir.join("bin").join("windows").join(file_name);

    if !executable.is_file() {
        return Err(CommandError::new(
            "missing_whisper_executable",
            format!(
                "Bundled whisper.cpp executable is missing at {}. Install the app build that includes resources/bin/windows/{}.",
                executable.display(),
                file_name
            ),
        ));
    }

    Ok(executable)
}

/// Appends `--no-gpu` when GPU acceleration is off. Shared so the CLI path and
/// the warm-server path honor the setting identically.
pub(crate) fn push_gpu_args(args: &mut Vec<String>, gpu_off: bool) {
    if gpu_off {
        args.push("--no-gpu".to_string());
    }
}

/// The `GGML_VK_VISIBLE_DEVICES` value used to pin a specific Vulkan device:
/// `Some("<index>")` only when the GPU is on AND a device is pinned; otherwise
/// `None`, so ggml selects its default device (normally the discrete card).
pub(crate) fn gpu_visible_devices_env(
    gpu: crate::settings::GpuAcceleration,
    device_index: Option<u32>,
) -> Option<String> {
    if gpu.is_off() {
        return None;
    }
    device_index.map(|index| index.to_string())
}

fn whisper_args(
    model_path: &Path,
    wav_path: &Path,
    language: &str,
    translate: bool,
    output_prefix: &Path,
    vocabulary_prompt: &str,
    word_timestamps: bool,
) -> Vec<String> {
    let mut args = vec![
        "-m".to_string(),
        model_path.to_string_lossy().to_string(),
        "-f".to_string(),
        wav_path.to_string_lossy().to_string(),
        "--language".to_string(),
        language.to_string(),
        // FILLER: word timestamps need the JSON-full output; otherwise plain txt
        // (the default path, unchanged). Both still print --no-timestamps text.
        if word_timestamps {
            "--output-json-full".to_string()
        } else {
            "--output-txt".to_string()
        },
        "--output-file".to_string(),
        output_prefix.to_string_lossy().to_string(),
        "--no-timestamps".to_string(),
        // Suppress non-speech tokens (e.g. "(laughs)", "[Music]") so silence and
        // near-silent audio hallucinate less of that filler. Mirrored in
        // server_args so the warm-server and CLI paths behave identically.
        "--suppress-nst".to_string(),
    ];

    // Translate task: whisper.cpp emits English for any spoken language. Only
    // meaningful on a multilingual model; the UI guards the pairing.
    if translate {
        args.push("--translate".to_string());
    }

    let vocabulary_prompt = vocabulary_prompt.trim();
    if !vocabulary_prompt.is_empty() {
        // Separate argv entries: no shell is involved, so the prompt text
        // needs no quoting or escaping.
        args.push("--prompt".to_string());
        args.push(vocabulary_prompt.to_string());
    }

    args
}

fn output_prefix_for_wav(wav_path: &Path) -> PathBuf {
    let parent = wav_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("whisper-{}", Uuid::new_v4().simple()))
}

/// Keeps helper processes from flashing a console window on Windows.
pub(crate) fn suppress_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn run_whisper_command(
    executable: &Path,
    args: &[String],
    vk_visible_devices: Option<String>,
) -> Result<std::process::Output, CommandError> {
    let mut command = Command::new(executable);
    command.args(args);
    // Pin a specific Vulkan device for this run (multi-GPU machines); absent
    // means ggml picks its default device.
    if let Some(devices) = vk_visible_devices {
        command.env("GGML_VK_VISIBLE_DEVICES", devices);
    }
    suppress_console_window(&mut command);

    command.output().map_err(|error| {
        CommandError::new(
            "whisper_transcription_failed",
            format!("Could not start whisper.cpp executable. {}", error),
        )
    })
}

fn parse_output_text(output_txt_path: &Path, stdout: &[u8]) -> Result<String, CommandError> {
    let raw_text = match fs::read_to_string(output_txt_path) {
        Ok(text) => text,
        Err(_) => String::from_utf8_lossy(stdout).to_string(),
    };

    Ok(normalize_transcript_text(&raw_text))
}

/// Reconstructs words (with millisecond timing) from whisper-cli
/// `--output-json-full` output, for pause-aware filler suppression. Whisper emits
/// sub-word tokens; a token whose text starts with a space begins a new word, and
/// continuation tokens (incl. trailing punctuation) extend the current word. The
/// special tokens it interleaves (`[_BEG_]`, `[_TT_123]`, …) are skipped. Returns
/// an empty vec on any parse problem, so the caller can fall back to plain text.
pub(crate) fn parse_cli_words(json_full: &str) -> Vec<crate::filler::TimedWord> {
    use crate::filler::TimedWord;
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json_full) else {
        return Vec::new();
    };
    let Some(segments) = value.get("transcription").and_then(|t| t.as_array()) else {
        return Vec::new();
    };

    let mut words: Vec<TimedWord> = Vec::new();
    for segment in segments {
        let Some(tokens) = segment.get("tokens").and_then(|t| t.as_array()) else {
            continue;
        };
        for token in tokens {
            let raw = token.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let starts_word = raw.starts_with(' ');
            let trimmed = raw.trim();
            // Skip whisper's special tokens ([_BEG_], [_TT_..], [_EOT_], …).
            if trimmed.is_empty() || trimmed.starts_with("[_") {
                continue;
            }
            let offsets = token.get("offsets");
            let from = offsets
                .and_then(|o| o.get("from"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let to = offsets
                .and_then(|o| o.get("to"))
                .and_then(|v| v.as_i64())
                .unwrap_or(from);

            match words.last_mut() {
                // Continuation sub-word/punctuation: append, extend the end time.
                // max() guards a stray token whose `to` is stale/zero from
                // shrinking the word's end below its start (negative duration).
                Some(last) if !starts_word => {
                    last.text.push_str(trimmed);
                    last.end_ms = to.max(last.end_ms);
                }
                _ => words.push(TimedWord::new(trimmed.to_string(), from, to)),
            }
        }
    }
    words
}

/// Shared transcript normalization so the warm-server path and the CLI path
/// produce identically trimmed text. Whisper's non-speech annotations are
/// removed: silent audio otherwise types literal "[BLANK_AUDIO]" markers.
pub(crate) fn normalize_transcript_text(raw_text: &str) -> String {
    let joined = raw_text
        .lines()
        .map(strip_noise_annotations)
        .map(|line| strip_pause_ellipses(&line))
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    tidy_punctuation_spacing(&joined)
}

/// Whisper renders speech pauses as an ellipsis ("..." or the "…" character).
/// Drop them so dictation reads as normal speech instead of hesitant,
/// dotted-out fragments. A single "." is a real sentence period and is kept;
/// only runs of two or more dots (and the unicode ellipsis) are removed.
fn strip_pause_ellipses(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '…' => out.push(' '),
            '.' => {
                let mut dots = 1;
                while chars.peek() == Some(&'.') {
                    chars.next();
                    dots += 1;
                }
                // 2+ dots = a pause ellipsis -> a space; a lone dot stays.
                out.push(if dots >= 2 { ' ' } else { '.' });
            }
            other => out.push(other),
        }
    }
    out
}

/// Removes whitespace left immediately before sentence punctuation (a common
/// artifact once a pause ellipsis between a word and its punctuation is
/// dropped, e.g. "Where ?" -> "Where?").
fn tidy_punctuation_spacing(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, ',' | '.' | '?' | '!' | ';' | ':') {
            while out.ends_with(' ') {
                out.pop();
            }
        }
        out.push(c);
    }
    out
}

/// Drops whisper's bracketed/parenthesized non-speech annotations, e.g.
/// "[BLANK_AUDIO]", "(silence)", "[Music]". Only known annotation words are
/// removed so legitimately dictated brackets survive.
fn strip_noise_annotations(line: &str) -> String {
    const NOISE: [&str; 14] = [
        "blank audio",
        "silence",
        "silent",
        "music",
        "applause",
        "laughter",
        "laughing",
        "laughs",
        "laugh",
        "noise",
        "inaudible",
        "no audio",
        "no speech",
        "speaking in foreign language",
    ];

    let mut result = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find(['[', '(']) {
        let close = if rest.as_bytes()[start] == b'[' {
            ']'
        } else {
            ')'
        };
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find(close) {
            Some(end) => {
                let inner = after[..end].trim().to_lowercase().replace('_', " ");
                if !NOISE.contains(&inner.as_str()) {
                    result.push_str(&rest[start..start + 1 + end + 1]);
                }
                rest = &after[end + 1..];
            }
            None => {
                result.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_words_with_timing_from_json_full() {
        // Shape mirrors real whisper-cli --output-json-full: a [_BEG_] special
        // token, space-prefixed word starts, a sub-word continuation, and
        // trailing punctuation as its own token.
        let json = r#"{"transcription":[{"text":" Activity cards.","tokens":[
            {"text":"[_BEG_]","offsets":{"from":0,"to":0}},
            {"text":" Activ","offsets":{"from":0,"to":300}},
            {"text":"ity","offsets":{"from":300,"to":650}},
            {"text":" cards","offsets":{"from":1100,"to":1320}},
            {"text":",","offsets":{"from":1320,"to":1320}}
        ]}]}"#;
        let words = parse_cli_words(json);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Activity"); // sub-word tokens merged
        assert_eq!((words[0].start_ms, words[0].end_ms), (0, 650));
        assert_eq!(words[1].text, "cards,"); // punctuation attached
        assert_eq!((words[1].start_ms, words[1].end_ms), (1100, 1320));
    }

    #[test]
    fn parse_cli_words_is_empty_on_garbage() {
        assert!(parse_cli_words("not json").is_empty());
        assert!(parse_cli_words(r#"{"transcription":[]}"#).is_empty());
    }

    #[test]
    fn builds_whisper_args_without_shell_concatenation() {
        let args = whisper_args(
            Path::new("models/ggml-small.en-q5_1.bin"),
            Path::new("temp/input.wav"),
            "en",
            false,
            Path::new("temp/out"),
            "",
            false,
        );

        assert_eq!(
            args,
            vec![
                "-m",
                "models/ggml-small.en-q5_1.bin",
                "-f",
                "temp/input.wav",
                "--language",
                "en",
                "--output-txt",
                "--output-file",
                "temp/out",
                "--no-timestamps",
                "--suppress-nst",
            ]
        );
        // No translate task unless explicitly requested.
        assert!(!args.iter().any(|arg| arg == "--translate"));
    }

    #[test]
    fn passes_language_code_and_translate_flag() {
        // A non-English language code is forwarded verbatim, and translate adds
        // exactly one --translate flag (English output for any spoken language).
        let args = whisper_args(
            Path::new("models/ggml-small.bin"),
            Path::new("temp/input.wav"),
            "es",
            true,
            Path::new("temp/out"),
            "",
            false,
        );

        let lang_index = args.iter().position(|arg| arg == "--language").unwrap();
        assert_eq!(args[lang_index + 1], "es");
        assert_eq!(args.iter().filter(|arg| *arg == "--translate").count(), 1);
    }

    #[test]
    fn auto_detect_language_is_passed_through() {
        let args = whisper_args(
            Path::new("models/ggml-base.bin"),
            Path::new("temp/input.wav"),
            "auto",
            false,
            Path::new("temp/out"),
            "",
            false,
        );
        let lang_index = args.iter().position(|arg| arg == "--language").unwrap();
        assert_eq!(args[lang_index + 1], "auto");
    }

    #[test]
    fn noise_annotations_are_stripped() {
        assert_eq!(normalize_transcript_text("[BLANK_AUDIO]"), "");
        assert_eq!(normalize_transcript_text(" (silence) \n[Music]"), "");
        assert_eq!(
            normalize_transcript_text("Hello [BLANK_AUDIO] world."),
            "Hello world."
        );
        assert_eq!(
            normalize_transcript_text("Control. [blank audio]"),
            "Control."
        );
        // Legitimate brackets survive.
        assert_eq!(
            normalize_transcript_text("Use array[0] and (see notes)."),
            "Use array[0] and (see notes)."
        );
    }

    #[test]
    fn pause_ellipses_are_removed_but_periods_kept() {
        // The pause-dots that Whisper inserts for hesitations are dropped.
        assert_eq!(
            normalize_transcript_text("So, uh... Where... Okay, what's the status?"),
            "So, uh Where Okay, what's the status?"
        );
        // The unicode ellipsis is handled too, and the space it leaves before
        // punctuation is tidied.
        assert_eq!(normalize_transcript_text("Where… ?"), "Where?");
        // A normal sentence period survives untouched.
        assert_eq!(
            normalize_transcript_text("I went home. Then I slept."),
            "I went home. Then I slept."
        );
        // A line that was only an ellipsis collapses to nothing.
        assert_eq!(normalize_transcript_text("..."), "");
    }

    #[test]
    fn real_outro_like_dictation_is_kept_verbatim() {
        // No more hallucination denylist: these are returned untouched. Custom
        // phrase removal is the dictionary replacements feature's job.
        assert_eq!(
            normalize_transcript_text("Thanks for watching!"),
            "Thanks for watching!"
        );
        assert_eq!(
            normalize_transcript_text("Please subscribe to my newsletter."),
            "Please subscribe to my newsletter."
        );
        assert_eq!(
            normalize_transcript_text("Thank you. Please subscribe. Bye."),
            "Thank you. Please subscribe. Bye."
        );
    }

    #[test]
    fn whisper_args_suppress_non_speech_tokens() {
        let args = whisper_args(
            Path::new("model.bin"),
            Path::new("audio.wav"),
            "en",
            false,
            Path::new("out"),
            "",
            false,
        );
        assert!(args.iter().any(|arg| arg == "--suppress-nst"));
    }

    #[test]
    fn word_timestamps_switch_output_format() {
        let plain = whisper_args(
            Path::new("m.bin"),
            Path::new("a.wav"),
            "en",
            false,
            Path::new("out"),
            "",
            false,
        );
        assert!(plain.iter().any(|arg| arg == "--output-txt"));
        assert!(!plain.iter().any(|arg| arg == "--output-json-full"));

        let timed = whisper_args(
            Path::new("m.bin"),
            Path::new("a.wav"),
            "en",
            false,
            Path::new("out"),
            "",
            true,
        );
        assert!(timed.iter().any(|arg| arg == "--output-json-full"));
        assert!(!timed.iter().any(|arg| arg == "--output-txt"));
    }

    #[test]
    fn omits_prompt_when_vocabulary_is_blank() {
        let args = whisper_args(
            Path::new("model.bin"),
            Path::new("input.wav"),
            "en",
            false,
            Path::new("out"),
            "   \n\t ",
            false,
        );

        assert!(!args.iter().any(|arg| arg == "--prompt"));
    }

    #[test]
    fn appends_prompt_as_separate_argv_entries() {
        let args = whisper_args(
            Path::new("model.bin"),
            Path::new("input.wav"),
            "en",
            false,
            Path::new("out"),
            "  Scribe, Tauri, WASAPI; \"quoted\" & spaced terms  ",
            false,
        );

        let prompt_index = args.iter().position(|arg| arg == "--prompt").unwrap();
        assert_eq!(
            args[prompt_index + 1],
            "Scribe, Tauri, WASAPI; \"quoted\" & spaced terms"
        );
        assert_eq!(args.len(), prompt_index + 2);
    }

    #[test]
    fn normalizes_whisper_text_output() {
        assert_eq!(
            normalize_transcript_text("\n Hello local dictation.\n\nSecond line. \n"),
            "Hello local dictation. Second line."
        );
    }

    #[test]
    fn no_gpu_arg_appended_only_when_off() {
        use crate::settings::GpuAcceleration;
        let mut auto = vec!["-m".to_string()];
        push_gpu_args(&mut auto, GpuAcceleration::Auto.is_off());
        assert!(!auto.iter().any(|arg| arg == "--no-gpu"));

        let mut off = vec!["-m".to_string()];
        push_gpu_args(&mut off, GpuAcceleration::Off.is_off());
        assert!(off.iter().any(|arg| arg == "--no-gpu"));
    }

    #[test]
    fn vk_visible_devices_env_respects_off_and_pin() {
        use crate::settings::GpuAcceleration;
        // Off => never pin (CPU only).
        assert_eq!(gpu_visible_devices_env(GpuAcceleration::Off, Some(1)), None);
        // On without a pin => let ggml choose its default device.
        assert_eq!(gpu_visible_devices_env(GpuAcceleration::Auto, None), None);
        // On with a pin => exactly that index, as a string.
        assert_eq!(
            gpu_visible_devices_env(GpuAcceleration::Auto, Some(0)),
            Some("0".to_string())
        );
        assert_eq!(
            gpu_visible_devices_env(GpuAcceleration::Auto, Some(1)),
            Some("1".to_string())
        );
    }
}
