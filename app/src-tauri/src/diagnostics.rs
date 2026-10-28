//! "Copy diagnostics" in About: a plain-text report a user can paste into a
//! bug report.
//!
//! It carries the app version, OS, a summary of settings, the current state,
//! recent warnings and errors and the end of the log. It never carries
//! transcript text, tokens or free-text settings: settings go through an
//! allow-list (free text is reduced to "set"/"empty" or a count), and every
//! line from the log has the home folder, the user name and anything shaped
//! like a token masked.

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use chrono::Utc;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::{commands::BackendState, error::CommandError, settings::AppSettings};

/// How much of the newest log file is read (its end).
const LOG_READ_BYTES: u64 = 256 * 1024;
/// Lines of log shown at the end of the report.
const LOG_TAIL_LINES: usize = 60;
/// Warning and error lines listed under "Recent problems".
const RECENT_PROBLEMS: usize = 12;

/// Settings copied as they are: switches, numbers, choices and hotkeys.
/// Anything not listed here (free text, paths, account names, colors) is left
/// out or summarized in `settings_summary`.
const SETTINGS_ALLOW_LIST: &[&str] = &[
    "defaultsVersion",
    "launchAtStartup",
    "minimizeToTray",
    "showFloatingPill",
    "pillDisplayMode",
    "dashboardHotkeyToggles",
    "notificationsEnabled",
    "soundsEnabled",
    "developerSettingsEnabled",
    "theme",
    "autoUpdateCheckEnabled",
    "autoInstallUpdates",
    "minRecordingMs",
    "maxRecordingMs",
    "silenceTrimEnabled",
    "silenceAutoStopEnabled",
    "silenceAutoStopMs",
    "incrementalTranscriptionEnabled",
    "segmentPauseEnabled",
    "segmentPauseMs",
    "segmentMaxMs",
    "selectedModelId",
    "language",
    "translateToEnglish",
    "gpuAcceleration",
    "gpuDeviceIndex",
    "outputMode",
    "pasteMethod",
    "historyEnabled",
    "saveAudioClips",
    "historyRetentionDays",
    "notesRetentionDays",
    "notesAnalysisEnabled",
    "fillerSuppressionEnabled",
    "fillerPauseThresholdMs",
    "githubSyncEnabled",
    "githubSyncAllTranscripts",
    "hotkeys",
];

/// Builds the diagnostics report.
#[tauri::command]
pub fn collect_diagnostics(
    app: AppHandle,
    state: State<'_, BackendState>,
) -> Result<String, CommandError> {
    let (settings, models) = {
        let db = state.db()?;
        (db.get_settings()?, db.list_model_records()?)
    };
    let snapshot = state.app_state()?.snapshot();
    let home = app.path().home_dir().ok();
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .ok();
    let redactor = Redactor::new(home.as_deref(), user.as_deref());

    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push('\n');
    };

    line("Scribe diagnostics".to_string());
    line(format!(
        "Generated: {}",
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    ));
    line(format!(
        "App: {} {} ({})",
        crate::display_name(&app),
        app.package_info().version,
        app.config().identifier
    ));
    line(format!(
        "OS: {} ({} {})",
        os_description(),
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    line(format!("State: {:?}", snapshot.status));
    if let Some(error) = &snapshot.error {
        line(format!(
            "Current error: {:?}: {}",
            error.code,
            redactor.apply(&error.message)
        ));
    }

    line(String::new());
    line("Models:".to_string());
    if models.is_empty() {
        line("  none recorded".to_string());
    }
    for model in &models {
        line(format!("  {}: {:?}", model.id, model.status));
    }

    line(String::new());
    line("Settings:".to_string());
    for (key, value) in settings_summary(&settings) {
        line(format!("  {key}: {value}"));
    }

    let log = read_newest_log(&app);
    let lines: Vec<String> = log
        .as_deref()
        .unwrap_or("")
        .lines()
        .map(|text| redactor.apply(text))
        .collect();

    line(String::new());
    line("Recent problems:".to_string());
    let problems = recent_problems(&lines, RECENT_PROBLEMS);
    if problems.is_empty() {
        line("  none in the current log".to_string());
    }
    for problem in problems {
        line(format!("  {problem}"));
    }

    line(String::new());
    line(format!("Log tail (last {LOG_TAIL_LINES} lines):"));
    if log.is_none() {
        line("  no log file found".to_string());
    }
    for text in tail(&lines, LOG_TAIL_LINES) {
        line(format!("  {text}"));
    }

    Ok(out)
}

/// Settings as `(name, value)` pairs: allow-listed values as they are, and
/// free-text settings reduced to whether they are set or how many there are.
pub fn settings_summary(settings: &AppSettings) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    if let Ok(Value::Object(map)) = serde_json::to_value(settings) {
        for key in SETTINGS_ALLOW_LIST {
            if let Some(value) = map.get(*key) {
                pairs.push((key.to_string(), compact(value)));
            }
        }
    }
    let set_or_empty = |text: &str| {
        if text.trim().is_empty() {
            "empty"
        } else {
            "set"
        }
    };
    pairs.push((
        "selectedMic".into(),
        if settings.selected_mic_id.is_some() {
            "chosen device"
        } else {
            "default"
        }
        .into(),
    ));
    pairs.push((
        "vocabularyPrompt".into(),
        set_or_empty(&settings.vocabulary_prompt).into(),
    ));
    pairs.push((
        "textReplacements".into(),
        settings.text_replacements.len().to_string(),
    ));
    pairs.push((
        "fillerWords".into(),
        settings.filler_words.len().to_string(),
    ));
    pairs.push((
        "notesAnalysisEndpoint".into(),
        endpoint_kind(&settings.notes_analysis_endpoint).into(),
    ));
    pairs.push((
        "githubAccount".into(),
        if settings.github_account_login.trim().is_empty() {
            "not connected"
        } else {
            "connected"
        }
        .into(),
    ));
    pairs.push((
        "dataFolder".into(),
        if settings.data_dir.is_some() {
            "custom"
        } else {
            "default"
        }
        .into(),
    ));
    pairs
}

fn compact(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "none".to_string(),
        other => other.to_string(),
    }
}

/// "empty", "local" (loopback) or "remote", never the address itself.
fn endpoint_kind(endpoint: &str) -> &'static str {
    let endpoint = endpoint.trim().to_ascii_lowercase();
    if endpoint.is_empty() {
        return "empty";
    }
    let authority = endpoint.split("://").nth(1).unwrap_or(&endpoint);
    let loopback = ["localhost", "127.", "[::1]"]
        .iter()
        .any(|prefix| authority.starts_with(prefix));
    if loopback {
        "local"
    } else {
        "remote"
    }
}

/// The last `count` warning or error lines, oldest first.
pub fn recent_problems(lines: &[String], count: usize) -> Vec<String> {
    let problems: Vec<String> = lines
        .iter()
        .filter(|line| line.contains("[WARN]") || line.contains("[ERROR]"))
        .cloned()
        .collect();
    tail(&problems, count).to_vec()
}

fn tail<T>(items: &[T], count: usize) -> &[T] {
    &items[items.len().saturating_sub(count)..]
}

/// The end of the most recently written `.log` file in the app's log folder.
fn read_newest_log(app: &AppHandle) -> Option<String> {
    let dir = app.path().app_log_dir().ok()?;
    let newest = newest_log_file(&dir)?;
    let mut file = fs::File::open(newest).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(LOG_READ_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    // Starting mid-file leaves a partial first line; drop it.
    Some(if start > 0 {
        text.split_once('\n')
            .map(|(_, rest)| rest.to_string())
            .unwrap_or_default()
    } else {
        text
    })
}

fn newest_log_file(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// Masks the user's home folder and name, and anything shaped like a token.
pub struct Redactor {
    homes: Vec<String>,
    user: Option<String>,
}

const TOKEN_PREFIXES: &[&str] = &[
    "github_pat_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "Bearer ",
    "bearer ",
    "token=",
    "access_token=",
];

impl Redactor {
    pub fn new(home: Option<&Path>, user: Option<&str>) -> Self {
        let mut homes = Vec::new();
        if let Some(home) = home.map(|path| path.to_string_lossy().to_string()) {
            let home = home.trim_end_matches(['/', '\\']).to_string();
            if home.len() > 3 {
                homes.push(home.replace('\\', "/"));
                homes.push(home.replace('/', "\\"));
                homes.push(home.replace('\\', "\\\\"));
                homes.sort_by_key(|path| std::cmp::Reverse(path.len()));
                homes.dedup();
            }
        }
        let user = user
            .map(str::trim)
            .filter(|name| name.chars().count() >= 3)
            .map(str::to_string);
        Self { homes, user }
    }

    pub fn apply(&self, text: &str) -> String {
        let mut text = text.to_string();
        for home in &self.homes {
            text = replace_ignore_ascii_case(&text, home, "~");
        }
        if let Some(user) = &self.user {
            text = replace_ignore_ascii_case(&text, user, "<user>");
        }
        mask_tokens(&text)
    }
}

fn replace_ignore_ascii_case(text: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let lower_text = text.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (index, _) in lower_text.match_indices(&lower_needle) {
        if index < last {
            continue;
        }
        out.push_str(&text[last..index]);
        out.push_str(replacement);
        last = index + needle.len();
    }
    out.push_str(&text[last..]);
    out
}

fn mask_tokens(text: &str) -> String {
    let mut out = text.to_string();
    for prefix in TOKEN_PREFIXES {
        let mut result = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(index) = rest.find(prefix) {
            let after = index + prefix.len();
            let secret_len = rest[after..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
                .unwrap_or(rest.len() - after);
            result.push_str(&rest[..after]);
            if secret_len > 0 {
                result.push_str("[redacted]");
            }
            rest = &rest[after + secret_len..];
        }
        result.push_str(rest);
        out = result;
    }
    out
}

/// A human-readable OS name and version.
fn os_description() -> String {
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("cmd");
        command.args(["/d", "/c", "ver"]);
        crate::whisper::suppress_console_window(&mut command);
        if let Ok(output) = command.output() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
        "Windows".to_string()
    }
    #[cfg(not(windows))]
    {
        fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                    .map(|name| name.trim_matches('"').to_string())
            })
            .unwrap_or_else(|| std::env::consts::OS.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_summary_leaves_out_free_text_and_accounts() {
        let settings = AppSettings {
            vocabulary_prompt: "Secret Project Falcon".into(),
            github_account_login: "someone".into(),
            github_repo: "someone/private-notes".into(),
            notes_analysis_endpoint: "http://10.0.0.5:1234/v1".into(),
            notes_analysis_prompt: "Summarize my diary".into(),
            data_dir: Some("C:\\Users\\someone\\Scribe".into()),
            ..Default::default()
        };
        let text = settings_summary(&settings)
            .into_iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>()
            .join("\n");
        for secret in ["Falcon", "someone", "private-notes", "10.0.0.5", "diary"] {
            assert!(!text.contains(secret), "{secret} leaked into:\n{text}");
        }
        assert!(text.contains("vocabularyPrompt: set"));
        assert!(text.contains("githubAccount: connected"));
        assert!(text.contains("notesAnalysisEndpoint: remote"));
        assert!(text.contains("dataFolder: custom"));
        assert!(text.contains("selectedModelId: small.en-q5_1"));
        assert!(text.contains("hotkeys: {"));
    }

    #[test]
    fn endpoint_kind_only_says_local_or_remote() {
        assert_eq!(endpoint_kind(""), "empty");
        assert_eq!(endpoint_kind("http://localhost:1234/v1"), "local");
        assert_eq!(endpoint_kind("http://127.0.0.1:11434"), "local");
        assert_eq!(endpoint_kind("http://[::1]:8080"), "local");
        assert_eq!(endpoint_kind("https://api.example.com/v1"), "remote");
    }

    #[test]
    fn redactor_masks_home_user_and_tokens() {
        let redactor = Redactor::new(Some(Path::new("C:\\Users\\Natha")), Some("natha"));
        let line = "[ERROR] Could not open C:\\Users\\natha\\AppData\\x.wav for natha with ghp_abcDEF123456 and Bearer xyz.123";
        let masked = redactor.apply(line);
        assert_eq!(
            masked,
            "[ERROR] Could not open ~\\AppData\\x.wav for <user> with ghp_[redacted] and Bearer [redacted]"
        );
        let forward = redactor.apply("path C:/Users/Natha/.scribe/control.json");
        assert_eq!(forward, "path ~/.scribe/control.json");
    }

    #[test]
    fn short_user_names_are_not_masked_everywhere() {
        let redactor = Redactor::new(None, Some("al"));
        assert_eq!(redactor.apply("all ok"), "all ok");
    }

    #[test]
    fn recent_problems_keeps_the_last_warnings_and_errors() {
        let lines: Vec<String> = ["[INFO] a", "[WARN] b", "[ERROR] c", "[INFO] d", "[WARN] e"]
            .iter()
            .map(|line| line.to_string())
            .collect();
        assert_eq!(recent_problems(&lines, 2), vec!["[ERROR] c", "[WARN] e"]);
    }
}
