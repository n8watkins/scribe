use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{
    commands::BackendState,
    dictation::whisper_language,
    error::CommandError,
    model_manager,
    whisper::{self, WhisperRequest},
};

/// Formats recent whisper-cli builds decode on their own; everything else
/// (video containers, m4a, ...) is converted with ffmpeg first.
const WHISPER_NATIVE_EXTENSIONS: [&str; 4] = ["wav", "mp3", "flac", "ogg"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeFileResult {
    pub text: String,
    /// Known only when the audio fed to whisper-cli is a WAV (the source
    /// itself, or the ffmpeg conversion output).
    pub duration_ms: Option<u64>,
    pub latency_ms: u32,
}

/// Transcribes a user-provided audio/video file with whisper-cli directly
/// (never the warm server, which is tuned for short dictation segments).
/// Long files are expected; the caller runs this on a worker thread.
pub fn transcribe_file(app: &AppHandle, path: &str) -> Result<TranscribeFileResult, CommandError> {
    let source = PathBuf::from(path.trim());
    if !source.is_file() {
        return Err(CommandError::new(
            "file_not_found",
            format!(
                "No file found at {}. Check the path and try again.",
                source.display()
            ),
        ));
    }

    let state = app.state::<BackendState>();
    let (settings, model_path) = {
        let db = state.db()?;
        let settings = db.get_settings()?;
        let (_, model_path) = model_manager::selected_model_path(app, &db)?;
        (settings, model_path)
    };

    let work_dir = work_dir(app)?;
    let started = Instant::now();

    // Owns the converted temp WAV (when one exists) for the whole
    // transcription; Drop removes it on every exit path.
    let converted = if needs_ffmpeg(&source) {
        Some(convert_with_ffmpeg(&source, &work_dir)?)
    } else {
        None
    };
    let input_path = converted
        .as_ref()
        .map(|temp| temp.path.clone())
        .unwrap_or_else(|| source.clone());
    let duration_ms = wav_duration_ms(&input_path);

    let request = WhisperRequest {
        model_path,
        wav_path: input_path,
        language: whisper_language(&settings.language),
        translate: settings.translate_to_english,
        vocabulary_prompt: settings.vocabulary_prompt.clone(),
        // FILLER: gate from settings (None = off, unchanged path).
        filler: crate::filler::FillerConfig::from_settings(&settings),
        gpu: settings.gpu_acceleration,
        gpu_device_index: settings.gpu_device_index,
    };
    let output_prefix = work_dir.join(format!("file-{}", Uuid::new_v4().simple()));
    let transcription = whisper::transcribe_with_output_prefix(app, &request, &output_prefix)?;
    drop(converted);

    Ok(TranscribeFileResult {
        text: transcription.text,
        duration_ms,
        latency_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
    })
}

/// Writes `text` to `<source>.txt` (extension appended, so the source can
/// never be overwritten; an earlier output artifact can). Returns the path
/// written.
pub fn save_text_file(path: &str, text: &str) -> Result<String, CommandError> {
    let source = PathBuf::from(path.trim());
    let target = text_target_for_source(&source)?;
    fs::write(&target, text).map_err(|error| {
        CommandError::new(
            "save_text_failed",
            format!("Could not write {}. {}", target.display(), error),
        )
    })?;
    Ok(target.to_string_lossy().into_owned())
}

fn text_target_for_source(source: &Path) -> Result<PathBuf, CommandError> {
    let mut file_name = source
        .file_name()
        .ok_or_else(|| {
            CommandError::new(
                "save_text_failed",
                format!("{} is not a file path.", source.display()),
            )
        })?
        .to_os_string();
    file_name.push(".txt");
    Ok(source.with_file_name(file_name))
}

fn needs_ffmpeg(source: &Path) -> bool {
    let extension = source
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    !matches!(
        extension.as_deref(),
        Some(extension) if WHISPER_NATIVE_EXTENSIONS.contains(&extension)
    )
}

fn convert_with_ffmpeg(source: &Path, work_dir: &Path) -> Result<TempFile, CommandError> {
    let target = TempFile {
        path: work_dir.join(format!("ffmpeg-{}.wav", Uuid::new_v4().simple())),
    };

    let mut command = Command::new("ffmpeg");
    command
        .arg("-y")
        .arg("-i")
        .arg(source)
        .args(["-vn", "-ar", "16000", "-ac", "1", "-f", "wav"])
        .arg(&target.path);
    whisper::suppress_console_window(&mut command);

    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CommandError::new(
                "ffmpeg_required",
                "This file format needs ffmpeg to decode. Install ffmpeg and add it to PATH, or provide a WAV, MP3, FLAC, or OGG file.",
            )
        } else {
            CommandError::new(
                "ffmpeg_conversion_failed",
                format!("Could not start ffmpeg. {}", error),
            )
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // ffmpeg banners are long; the failure reason is at the end.
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .trim()
            .to_string();
        return Err(CommandError::new(
            "ffmpeg_conversion_failed",
            format!(
                "ffmpeg could not extract audio from {}. {}",
                source.display(),
                detail
            ),
        ));
    }

    Ok(target)
}

/// Temp WAVs written under the app cache survive nothing: Drop removes them
/// on success, error, and panic alike.
struct TempFile {
    path: PathBuf,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn work_dir(app: &AppHandle) -> Result<PathBuf, CommandError> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| {
            CommandError::new(
                "app_data_dir_unavailable",
                format!("Could not locate Scribe cache directory. {}", error),
            )
        })?
        .join("file-transcribe");
    fs::create_dir_all(&dir).map_err(|error| {
        CommandError::new(
            "app_data_dir_unavailable",
            format!("Could not create {}. {}", dir.display(), error),
        )
    })?;
    Ok(dir)
}

fn wav_duration_ms(path: &Path) -> Option<u64> {
    let reader = hound::WavReader::open(path).ok()?;
    let sample_rate = reader.spec().sample_rate;
    if sample_rate == 0 {
        return None;
    }
    Some(u64::from(reader.duration()) * 1000 / u64::from(sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_formats_skip_ffmpeg() {
        for name in ["a.wav", "b.MP3", "c.flac", "d.Ogg"] {
            assert!(!needs_ffmpeg(Path::new(name)), "{name} should be native");
        }
    }

    #[test]
    fn other_formats_need_ffmpeg() {
        for name in ["a.mp4", "b.mkv", "c.m4a", "d.webm", "noextension"] {
            assert!(needs_ffmpeg(Path::new(name)), "{name} should need ffmpeg");
        }
    }

    #[test]
    fn text_target_appends_txt_to_full_name() {
        assert_eq!(
            text_target_for_source(Path::new("/tmp/meeting.mp4")).unwrap(),
            Path::new("/tmp/meeting.mp4.txt")
        );
        assert_eq!(
            text_target_for_source(Path::new("clip.wav")).unwrap(),
            Path::new("clip.wav.txt")
        );
    }

    #[test]
    fn text_target_rejects_non_file_paths() {
        assert!(text_target_for_source(Path::new("/")).is_err());
    }

    #[test]
    fn save_text_file_writes_next_to_source() {
        let dir = std::env::temp_dir().join(format!("ld-test-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("audio.m4a");
        fs::write(&source, b"fake").unwrap();

        let written = save_text_file(source.to_str().unwrap(), "hello").unwrap();
        assert_eq!(Path::new(&written), dir.join("audio.m4a.txt"));
        assert_eq!(fs::read_to_string(&written).unwrap(), "hello");

        let _ = fs::remove_dir_all(&dir);
    }
}
