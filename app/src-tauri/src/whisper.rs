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
    /// Custom vocabulary / spelling hints; passed as `--prompt` when the
    /// trimmed value is non-empty.
    pub vocabulary_prompt: String,
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

    let executable = resolve_bundled_executable(app, "whisper-cli.exe")?;
    let output_prefix = output_prefix_for_wav(&request.wav_path);
    let output_txt_path = output_prefix.with_extension("txt");
    let args = whisper_args(
        &request.model_path,
        &request.wav_path,
        &request.language,
        &output_prefix,
        &request.vocabulary_prompt,
    );
    let started = Instant::now();
    let output = run_whisper_command(&executable, &args)?;
    let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        let _ = fs::remove_file(&output_txt_path);
        return Err(CommandError::new(
            "whisper_transcription_failed",
            format!("Whisper transcription failed. {}", detail),
        ));
    }

    let text = parse_output_text(&output_txt_path, &output.stdout)?;
    let _ = fs::remove_file(&output_txt_path);

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

fn whisper_args(
    model_path: &Path,
    wav_path: &Path,
    language: &str,
    output_prefix: &Path,
    vocabulary_prompt: &str,
) -> Vec<String> {
    let mut args = vec![
        "-m".to_string(),
        model_path.to_string_lossy().to_string(),
        "-f".to_string(),
        wav_path.to_string_lossy().to_string(),
        "--language".to_string(),
        language.to_string(),
        "--output-txt".to_string(),
        "--output-file".to_string(),
        output_prefix.to_string_lossy().to_string(),
        "--no-timestamps".to_string(),
    ];

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

fn run_whisper_command(
    executable: &Path,
    args: &[String],
) -> Result<std::process::Output, CommandError> {
    let mut command = Command::new(executable);
    command.args(args);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

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

/// Shared transcript normalization so the warm-server path and the CLI path
/// produce identically trimmed text.
pub(crate) fn normalize_transcript_text(raw_text: &str) -> String {
    raw_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_whisper_args_without_shell_concatenation() {
        let args = whisper_args(
            Path::new("models/ggml-small.en-q5_1.bin"),
            Path::new("temp/input.wav"),
            "en",
            Path::new("temp/out"),
            "",
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
            ]
        );
    }

    #[test]
    fn omits_prompt_when_vocabulary_is_blank() {
        let args = whisper_args(
            Path::new("model.bin"),
            Path::new("input.wav"),
            "en",
            Path::new("out"),
            "   \n\t ",
        );

        assert!(!args.iter().any(|arg| arg == "--prompt"));
    }

    #[test]
    fn appends_prompt_as_separate_argv_entries() {
        let args = whisper_args(
            Path::new("model.bin"),
            Path::new("input.wav"),
            "en",
            Path::new("out"),
            "  LocalDictate, Tauri, WASAPI; \"quoted\" & spaced terms  ",
        );

        let prompt_index = args.iter().position(|arg| arg == "--prompt").unwrap();
        assert_eq!(
            args[prompt_index + 1],
            "LocalDictate, Tauri, WASAPI; \"quoted\" & spaced terms"
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
}
