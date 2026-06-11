use serde::{Deserialize, Serialize};

/// Bumped whenever a shipped default changes in a way that should be applied
/// once to existing installs (see `migrate_defaults`). Stored settings with a
/// lower `defaults_version` get the new defaults applied exactly once.
pub const CURRENT_DEFAULTS_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Version of the shipped defaults already applied to these settings.
    /// Missing in pre-versioning DBs, which deserialize as 0.
    #[serde(default)]
    pub defaults_version: u32,
    pub launch_at_startup: bool,
    pub minimize_to_tray: bool,
    pub show_floating_pill: bool,
    pub notifications_enabled: bool,
    pub sounds_enabled: bool,
    /// Inert: hold-to-talk and toggle hotkeys both always work now. The field
    /// is kept so existing DB settings JSON (and serde round-trips) keep
    /// working unchanged.
    pub recording_mode: RecordingMode,
    pub min_recording_ms: u32,
    pub max_recording_ms: u32,
    pub silence_trim_enabled: bool,
    /// Automatically stop toggle/UI-started recordings after a stretch of
    /// continuous silence. Never applies to hold-to-talk or test clips.
    #[serde(default = "default_silence_auto_stop_enabled")]
    pub silence_auto_stop_enabled: bool,
    /// How long the audio must stay silent before auto-stop fires.
    #[serde(default = "default_silence_auto_stop_ms")]
    pub silence_auto_stop_ms: u32,
    pub selected_mic_id: Option<String>,
    pub selected_model_id: Option<String>,
    pub language: Language,
    /// Custom vocabulary / spelling hints passed to whisper.cpp via
    /// `--prompt` when non-empty.
    #[serde(default)]
    pub vocabulary_prompt: String,
    pub output_mode: OutputMode,
    pub paste_method: PasteMethod,
    pub history_enabled: bool,
    pub save_audio_clips: bool,
    pub history_retention_days: Option<u16>,
    /// Saved floating pill window position (physical pixels). None means the
    /// frontend places the pill at its bottom-center default.
    #[serde(default)]
    pub pill_x: Option<i32>,
    #[serde(default)]
    pub pill_y: Option<i32>,
    pub hotkeys: HotkeySettings,
}

fn default_silence_auto_stop_enabled() -> bool {
    true
}

fn default_silence_auto_stop_ms() -> u32 {
    2_000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingMode {
    Hold,
    Toggle,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Auto,
    En,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    SaveOnly,
    AutoPaste,
    CopyClipboard,
    CopyAndPaste,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteMethod {
    DirectInsert,
    ClipboardRestore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeySettings {
    pub hold_to_talk: String,
    pub toggle_dictation: String,
    pub paste_last_transcript: String,
    pub open_dashboard: String,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            hold_to_talk: "Ctrl+Shift".to_string(),
            toggle_dictation: "Backquote".to_string(),
            paste_last_transcript: "Ctrl+Alt+V".to_string(),
            open_dashboard: "Ctrl+Alt+D".to_string(),
        }
    }
}

impl HotkeySettings {
    /// The defaults that shipped before modifier-only chord support. Windows
    /// intercepts Ctrl+Win+Space (layout switcher) and Ctrl+Win+D (new
    /// virtual desktop), so installs still on these exact values are migrated
    /// to the current defaults.
    pub fn matches_legacy_defaults(&self) -> bool {
        self.hold_to_talk == "Ctrl+Win+Space"
            && self.toggle_dictation == "Ctrl+Win+D"
            && self.paste_last_transcript == "Ctrl+Alt+V"
            && self.open_dashboard == "Ctrl+Win+H"
    }

    /// Replaces the stored hotkeys with the current defaults when they still
    /// exactly equal the legacy defaults. Returns true when a migration
    /// happened and the settings should be saved back.
    pub fn migrate_legacy_defaults(&mut self) -> bool {
        if self.matches_legacy_defaults() {
            *self = Self::default();
            true
        } else {
            false
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            defaults_version: CURRENT_DEFAULTS_VERSION,
            launch_at_startup: false,
            minimize_to_tray: true,
            show_floating_pill: true,
            notifications_enabled: true,
            sounds_enabled: true,
            recording_mode: RecordingMode::Both,
            min_recording_ms: 300,
            max_recording_ms: 180_000,
            silence_trim_enabled: true,
            silence_auto_stop_enabled: default_silence_auto_stop_enabled(),
            silence_auto_stop_ms: default_silence_auto_stop_ms(),
            selected_mic_id: None,
            selected_model_id: Some("small.en-q5_1".to_string()),
            language: Language::En,
            vocabulary_prompt: String::new(),
            output_mode: OutputMode::AutoPaste,
            paste_method: PasteMethod::DirectInsert,
            history_enabled: true,
            save_audio_clips: false,
            history_retention_days: Some(30),
            pill_x: None,
            pill_y: None,
            hotkeys: HotkeySettings::default(),
        }
    }
}

impl AppSettings {
    /// One-time migration of changed shipped defaults. Settings stored before
    /// defaults version 1 (i.e. `defaults_version` 0) move from the old
    /// SaveOnly default output mode to AutoPaste — but only when the stored
    /// value still is SaveOnly. After this runs once, `defaults_version` is
    /// current, so a user who later deliberately picks SaveOnly is never
    /// overridden again. Returns true when the settings changed and should be
    /// saved back.
    pub fn migrate_defaults(&mut self) -> bool {
        if self.defaults_version >= CURRENT_DEFAULTS_VERSION {
            return false;
        }

        if self.defaults_version < 1 && self.output_mode == OutputMode::SaveOnly {
            self.output_mode = OutputMode::AutoPaste;
        }

        self.defaults_version = CURRENT_DEFAULTS_VERSION;
        true
    }

    pub fn validate(&self) -> Result<(), SettingsValidationError> {
        if self.min_recording_ms == 0 {
            return Err(SettingsValidationError::new(
                "minRecordingMs must be greater than zero.",
            ));
        }

        if self.max_recording_ms < self.min_recording_ms {
            return Err(SettingsValidationError::new(
                "maxRecordingMs must be greater than or equal to minRecordingMs.",
            ));
        }

        if !(500..=10_000).contains(&self.silence_auto_stop_ms) {
            return Err(SettingsValidationError::new(
                "silenceAutoStopMs must be between 500 and 10000.",
            ));
        }

        if !matches!(self.history_retention_days, Some(7 | 30 | 90 | 365) | None) {
            return Err(SettingsValidationError::new(
                "historyRetentionDays must be 7, 30, 90, 365, or null.",
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsValidationError {
    message: String,
}

impl SettingsValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SettingsValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SettingsValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_prd_baseline() {
        let settings = AppSettings::default();

        assert_eq!(settings.defaults_version, CURRENT_DEFAULTS_VERSION);
        assert_eq!(settings.recording_mode, RecordingMode::Both);
        assert_eq!(settings.min_recording_ms, 300);
        assert_eq!(settings.max_recording_ms, 180_000);
        assert_eq!(settings.output_mode, OutputMode::AutoPaste);
        assert_eq!(settings.paste_method, PasteMethod::DirectInsert);
        assert!(settings.silence_auto_stop_enabled);
        assert_eq!(settings.silence_auto_stop_ms, 2_000);
        assert_eq!(settings.vocabulary_prompt, "");
        assert!(settings.history_enabled);
        assert!(!settings.save_audio_clips);
    }

    #[test]
    fn validates_history_retention_options() {
        let mut settings = AppSettings::default();
        settings.history_retention_days = Some(14);

        assert!(settings.validate().is_err());
    }

    #[test]
    fn validates_silence_auto_stop_ms_range() {
        let mut settings = AppSettings::default();

        settings.silence_auto_stop_ms = 499;
        assert!(settings.validate().is_err());

        settings.silence_auto_stop_ms = 500;
        assert!(settings.validate().is_ok());

        settings.silence_auto_stop_ms = 10_000;
        assert!(settings.validate().is_ok());

        settings.silence_auto_stop_ms = 10_001;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn migrates_save_only_default_to_auto_paste_once() {
        let mut settings = AppSettings {
            defaults_version: 0,
            output_mode: OutputMode::SaveOnly,
            ..AppSettings::default()
        };

        assert!(settings.migrate_defaults());
        assert_eq!(settings.output_mode, OutputMode::AutoPaste);
        assert_eq!(settings.defaults_version, CURRENT_DEFAULTS_VERSION);

        // Already migrated: never runs again.
        assert!(!settings.migrate_defaults());
    }

    #[test]
    fn does_not_migrate_non_default_output_mode() {
        let mut settings = AppSettings {
            defaults_version: 0,
            output_mode: OutputMode::CopyClipboard,
            ..AppSettings::default()
        };

        assert!(settings.migrate_defaults());
        assert_eq!(settings.output_mode, OutputMode::CopyClipboard);
        assert_eq!(settings.defaults_version, CURRENT_DEFAULTS_VERSION);
    }

    #[test]
    fn does_not_override_deliberate_save_only_after_migration() {
        let mut settings = AppSettings {
            output_mode: OutputMode::SaveOnly,
            ..AppSettings::default()
        };

        assert!(!settings.migrate_defaults());
        assert_eq!(settings.output_mode, OutputMode::SaveOnly);
    }

    #[test]
    fn legacy_settings_json_gains_new_field_defaults() {
        // Pre-versioning settings JSON: no defaultsVersion,
        // silenceAutoStop*, or vocabularyPrompt fields.
        let json = serde_json::json!({
            "launchAtStartup": false,
            "minimizeToTray": true,
            "showFloatingPill": true,
            "notificationsEnabled": true,
            "soundsEnabled": true,
            "recordingMode": "both",
            "minRecordingMs": 300,
            "maxRecordingMs": 180000,
            "silenceTrimEnabled": true,
            "selectedMicId": null,
            "selectedModelId": "small.en-q5_1",
            "language": "en",
            "outputMode": "save_only",
            "pasteMethod": "direct_insert",
            "historyEnabled": true,
            "saveAudioClips": false,
            "historyRetentionDays": 30,
            "hotkeys": {
                "holdToTalk": "Ctrl+Shift",
                "toggleDictation": "Backquote",
                "pasteLastTranscript": "Ctrl+Alt+V",
                "openDashboard": "Ctrl+Alt+D"
            }
        });

        let settings: AppSettings = serde_json::from_value(json).unwrap();

        assert_eq!(settings.defaults_version, 0);
        assert!(settings.silence_auto_stop_enabled);
        assert_eq!(settings.silence_auto_stop_ms, 2_000);
        assert_eq!(settings.vocabulary_prompt, "");
        assert_eq!(settings.output_mode, OutputMode::SaveOnly);
    }

    #[test]
    fn default_hotkeys_avoid_windows_reserved_shortcuts() {
        let hotkeys = HotkeySettings::default();

        assert_eq!(hotkeys.hold_to_talk, "Ctrl+Shift");
        assert_eq!(hotkeys.toggle_dictation, "Backquote");
        assert_eq!(hotkeys.paste_last_transcript, "Ctrl+Alt+V");
        assert_eq!(hotkeys.open_dashboard, "Ctrl+Alt+D");
    }

    #[test]
    fn migrates_exact_legacy_default_hotkeys() {
        let mut hotkeys = HotkeySettings {
            hold_to_talk: "Ctrl+Win+Space".to_string(),
            toggle_dictation: "Ctrl+Win+D".to_string(),
            paste_last_transcript: "Ctrl+Alt+V".to_string(),
            open_dashboard: "Ctrl+Win+H".to_string(),
        };

        assert!(hotkeys.migrate_legacy_defaults());
        assert_eq!(hotkeys, HotkeySettings::default());
    }

    #[test]
    fn does_not_migrate_customized_hotkeys() {
        let mut hotkeys = HotkeySettings {
            hold_to_talk: "Ctrl+Win+Space".to_string(),
            toggle_dictation: "Ctrl+Win+D".to_string(),
            paste_last_transcript: "Ctrl+Alt+V".to_string(),
            open_dashboard: "Ctrl+Alt+J".to_string(),
        };
        let before = hotkeys.clone();

        assert!(!hotkeys.migrate_legacy_defaults());
        assert_eq!(hotkeys, before);
    }

    #[test]
    fn does_not_migrate_current_defaults() {
        let mut hotkeys = HotkeySettings::default();

        assert!(!hotkeys.migrate_legacy_defaults());
        assert_eq!(hotkeys, HotkeySettings::default());
    }
}
