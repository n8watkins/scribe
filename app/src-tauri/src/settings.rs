use serde::{Deserialize, Serialize};

/// Bumped whenever a shipped default changes in a way that should be applied
/// once to existing installs (see `migrate_defaults`). Stored settings with a
/// lower `defaults_version` get the new defaults applied exactly once.
pub const CURRENT_DEFAULTS_VERSION: u32 = 5;

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
    /// What the floating pill shows while recording: a minimal status dot,
    /// the waveform visualizer, or the visualizer with live transcript text.
    #[serde(default = "default_pill_display_mode")]
    pub pill_display_mode: PillDisplayMode,
    /// The dashboard hotkey hides the window again when it is already
    /// focused, instead of only ever opening it.
    #[serde(default = "default_dashboard_hotkey_toggles")]
    pub dashboard_hotkey_toggles: bool,
    pub notifications_enabled: bool,
    pub sounds_enabled: bool,
    /// Reveals a Developer panel in the sidebar with diagnostics (e.g. the live
    /// window resolution). Off by default; opt-in from Settings.
    #[serde(default)]
    pub developer_settings_enabled: bool,
    /// True once the Scribe Dev flavor has seeded its non-conflicting hotkey
    /// defaults (or the user loaded production defaults), so the one-shot dev
    /// seeding never overrides the binds again.
    #[serde(default)]
    pub dev_hotkeys_seeded: bool,
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
    /// Transcribe finished phrases in the background while the user is still
    /// talking, so stopping only has to transcribe the tail phrase. Applies
    /// to dictation recordings only — never test clips.
    #[serde(default = "default_incremental_transcription_enabled")]
    pub incremental_transcription_enabled: bool,
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
    /// Local-LLM analysis of notes (on demand from the Notes view). Off by
    /// default; talks to an OpenAI-compatible server (LM Studio, Ollama, ...).
    #[serde(default)]
    pub notes_analysis_enabled: bool,
    /// The user-editable instruction sent as the system prompt; the note text
    /// is the user message. Defines what "analysis" produces.
    #[serde(default = "default_notes_analysis_prompt")]
    pub notes_analysis_prompt: String,
    /// Base URL of the OpenAI-compatible API (no trailing /chat/completions).
    #[serde(default = "default_notes_analysis_endpoint")]
    pub notes_analysis_endpoint: String,
    /// Model name to request. Empty means "first model the server lists",
    /// which on LM Studio is whatever model is loaded.
    #[serde(default)]
    pub notes_analysis_model: String,
    /// Sync dictated notes to Google Drive as dated Markdown files. Off until
    /// the user signs in and enables it. The OAuth refresh token lives in the
    /// OS keychain (see `google_oauth.rs`), never in this JSON.
    #[serde(default)]
    pub drive_sync_enabled: bool,
    /// Also back up every transcript (not just notes) as text. Default off;
    /// text is tiny (~40 MB/yr) but the owner opts in explicitly.
    #[serde(default)]
    pub drive_sync_all_transcripts: bool,
    /// Hour of day (0-23, local time) the end-of-day organize / weekly passes
    /// run. Used from Phase 3 on; stored now so the setting is stable.
    #[serde(default = "default_drive_organize_hour")]
    pub drive_organize_hour: u32,
    /// Run the end-of-day pass: at `drive_organize_hour`, the local LLM
    /// reorganizes the previous day's notes into a `{day}-organized.md` Drive
    /// file. Opt-in (needs the local LLM running).
    #[serde(default)]
    pub drive_organize_enabled: bool,
    /// Instruction sent to the local LLM as the system prompt for the
    /// end-of-day organize pass (the day's notes are the user message).
    #[serde(default = "default_drive_organize_prompt")]
    pub drive_organize_prompt: String,
    /// The last local calendar day (YYYY-MM-DD) already organized, so the
    /// scheduler doesn't redo it. Empty until the first pass runs.
    #[serde(default)]
    pub drive_last_organized_date: String,
    /// Email of the signed-in Google account, for display only. Empty when
    /// signed out. The tokens themselves live in the OS keychain.
    #[serde(default)]
    pub drive_account_email: String,
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
    60_000
}

fn default_incremental_transcription_enabled() -> bool {
    true
}

fn default_pill_display_mode() -> PillDisplayMode {
    PillDisplayMode::VisualizerWithText
}

fn default_dashboard_hotkey_toggles() -> bool {
    true
}

fn default_notes_analysis_prompt() -> String {
    "Summarize this dictated note in one or two sentences, then list any \
     action items as bullet points. If there are none, say \"No action items.\""
        .to_string()
}

fn default_notes_analysis_endpoint() -> String {
    // LM Studio's local server default.
    "http://127.0.0.1:1234/v1".to_string()
}

fn default_drive_organize_hour() -> u32 {
    // 3 AM: late enough that the day's notes are in, unlikely to clash with
    // gaming/VRAM use.
    3
}

fn default_drive_organize_prompt() -> String {
    "You are organizing a day's worth of short voice notes. Group the related \
     notes under clear category headings (e.g. Tasks, Ideas, Follow-ups), \
     tighten the wording, and keep it concise. Output Markdown only."
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PillDisplayMode {
    Dot,
    Visualizer,
    VisualizerWithText,
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
    /// Opt-in fallback. Inject the transcript as synthetic keystrokes; the
    /// system clipboard is never read or written. Use this for apps that block
    /// synthetic Ctrl+V — the trade-off is that long inserts can visibly stream
    /// character by character.
    DirectInsert,
    /// Default. Borrow the clipboard to send a real Ctrl+V (one atomic paste),
    /// then restore the user's previous clipboard text. Lands cleanly in
    /// chat/browser/Electron apps without permanently consuming the clipboard.
    /// Text-clipboard restore only (see `clipboard_paste` in `output.rs`).
    ///
    /// `serde(alias)` keeps settings stored under the old `clipboard_restore`
    /// name deserializing into this variant, so existing installs are
    /// unaffected by the rename.
    #[serde(alias = "clipboard_restore")]
    ClipboardPaste,
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
            hold_to_talk: "Ctrl+Win".to_string(),
            toggle_dictation: "Backquote".to_string(),
            paste_last_transcript: "Ctrl+Alt+V".to_string(),
            open_dashboard: "Ctrl+Alt+F".to_string(),
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

    /// Non-conflicting binds for the Scribe Dev flavor, so running Dev next to
    /// stable Scribe doesn't fight over the same global shortcuts. Each differs
    /// from the production default by an extra Shift.
    pub fn dev_defaults() -> Self {
        Self {
            hold_to_talk: "Ctrl+Shift+Win".to_string(),
            toggle_dictation: "Ctrl+Shift+Backquote".to_string(),
            paste_last_transcript: "Ctrl+Alt+Shift+V".to_string(),
            open_dashboard: "Ctrl+Alt+Shift+F".to_string(),
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
            pill_display_mode: default_pill_display_mode(),
            dashboard_hotkey_toggles: default_dashboard_hotkey_toggles(),
            notifications_enabled: true,
            sounds_enabled: true,
            developer_settings_enabled: false,
            dev_hotkeys_seeded: false,
            recording_mode: RecordingMode::Both,
            min_recording_ms: 300,
            max_recording_ms: 600_000,
            silence_trim_enabled: true,
            silence_auto_stop_enabled: default_silence_auto_stop_enabled(),
            silence_auto_stop_ms: default_silence_auto_stop_ms(),
            incremental_transcription_enabled: default_incremental_transcription_enabled(),
            selected_mic_id: None,
            selected_model_id: Some("small.en-q5_1".to_string()),
            language: Language::En,
            vocabulary_prompt: String::new(),
            output_mode: OutputMode::AutoPaste,
            paste_method: PasteMethod::ClipboardPaste,
            history_enabled: true,
            save_audio_clips: true,
            history_retention_days: Some(30),
            notes_analysis_enabled: false,
            notes_analysis_prompt: default_notes_analysis_prompt(),
            notes_analysis_endpoint: default_notes_analysis_endpoint(),
            notes_analysis_model: String::new(),
            drive_sync_enabled: false,
            drive_sync_all_transcripts: false,
            drive_organize_hour: default_drive_organize_hour(),
            drive_organize_enabled: false,
            drive_organize_prompt: default_drive_organize_prompt(),
            drive_last_organized_date: String::new(),
            drive_account_email: String::new(),
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

        // v2: auto-stop on silence became opt-in. It shipped enabled by
        // default, so installs that still have it on never chose it.
        if self.defaults_version < 2 && self.silence_auto_stop_enabled {
            self.silence_auto_stop_enabled = false;
        }

        // v3: default binds changed (Ctrl+Win hold, Ctrl+Shift+V paste,
        // Ctrl+Alt+V dashboard). Only installs still on the exact v2 default
        // set are moved; customized binds are never touched.
        if self.defaults_version < 3
            && self.hotkeys.hold_to_talk == "Ctrl+Shift"
            && self.hotkeys.toggle_dictation == "Backquote"
            && self.hotkeys.paste_last_transcript == "Ctrl+Alt+V"
            && self.hotkeys.open_dashboard == "Ctrl+Alt+D"
        {
            self.hotkeys = HotkeySettings::default();
        }

        if self.defaults_version < 4 {
            // v4: longer dictations by default (10 min cap), silence
            // auto-stop on with a 60 s threshold, raw audio kept for in-app
            // playback, and paste/dashboard binds moved to Ctrl+Alt+V /
            // Ctrl+Alt+F. As everywhere above, only values still on the old
            // defaults are moved.
            if self.max_recording_ms == 180_000 {
                self.max_recording_ms = 600_000;
            }
            if !self.silence_auto_stop_enabled {
                self.silence_auto_stop_enabled = true;
            }
            if self.silence_auto_stop_ms == 5_000 {
                self.silence_auto_stop_ms = 60_000;
            }
            // save_audio_clips was inert before v4, so no install chose it.
            self.save_audio_clips = true;
            if self.hotkeys.hold_to_talk == "Ctrl+Win"
                && self.hotkeys.toggle_dictation == "Backquote"
                && self.hotkeys.paste_last_transcript == "Ctrl+Shift+V"
                && self.hotkeys.open_dashboard == "Ctrl+Alt+V"
            {
                self.hotkeys = HotkeySettings::default();
            }
        }

        // v5: the default paste method changed from DirectInsert (types the
        // transcript out as keystrokes) to ClipboardPaste (one Ctrl+V that
        // restores your clipboard). Installs still on the old DirectInsert
        // default are moved; a deliberate "Type it out" choice made after this
        // migration runs is preserved.
        if self.defaults_version < 5 && self.paste_method == PasteMethod::DirectInsert {
            self.paste_method = PasteMethod::ClipboardPaste;
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

        if !(500..=300_000).contains(&self.silence_auto_stop_ms) {
            return Err(SettingsValidationError::new(
                "silenceAutoStopMs must be between 500 and 300000.",
            ));
        }

        if !matches!(self.history_retention_days, Some(7 | 30 | 90 | 365) | None) {
            return Err(SettingsValidationError::new(
                "historyRetentionDays must be 7, 30, 90, 365, or null.",
            ));
        }

        if self.notes_analysis_enabled {
            let endpoint = self.notes_analysis_endpoint.trim();
            if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
                return Err(SettingsValidationError::new(
                    "notesAnalysisEndpoint must be an http(s) URL when notes analysis is enabled.",
                ));
            }
            if self.notes_analysis_prompt.trim().is_empty() {
                return Err(SettingsValidationError::new(
                    "notesAnalysisPrompt cannot be empty when notes analysis is enabled.",
                ));
            }
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
        assert_eq!(settings.max_recording_ms, 600_000);
        assert_eq!(settings.output_mode, OutputMode::AutoPaste);
        assert_eq!(settings.paste_method, PasteMethod::ClipboardPaste);
        assert!(settings.silence_auto_stop_enabled);
        assert_eq!(settings.silence_auto_stop_ms, 60_000);
        assert!(settings.incremental_transcription_enabled);
        assert_eq!(settings.vocabulary_prompt, "");
        assert!(settings.history_enabled);
        assert!(settings.save_audio_clips);
        assert_eq!(
            settings.pill_display_mode,
            PillDisplayMode::VisualizerWithText
        );
        assert!(!settings.notes_analysis_enabled);
        assert!(!settings.notes_analysis_prompt.is_empty());
        assert_eq!(settings.notes_analysis_endpoint, "http://127.0.0.1:1234/v1");
        assert_eq!(settings.notes_analysis_model, "");
        assert!(!settings.developer_settings_enabled);
        assert!(!settings.dev_hotkeys_seeded);
    }

    #[test]
    fn notes_analysis_validation_only_applies_when_enabled() {
        let mut settings = AppSettings::default();
        settings.notes_analysis_endpoint = "not a url".to_string();
        assert!(settings.validate().is_ok());

        settings.notes_analysis_enabled = true;
        assert!(settings.validate().is_err());

        settings.notes_analysis_endpoint = "http://127.0.0.1:1234/v1".to_string();
        assert!(settings.validate().is_ok());

        settings.notes_analysis_prompt = "  ".to_string();
        assert!(settings.validate().is_err());
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

        settings.silence_auto_stop_ms = 300_000;
        assert!(settings.validate().is_ok());

        settings.silence_auto_stop_ms = 300_001;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn migrates_direct_insert_paste_to_clipboard_once() {
        let mut settings = AppSettings {
            defaults_version: 4,
            paste_method: PasteMethod::DirectInsert,
            ..AppSettings::default()
        };

        assert!(settings.migrate_defaults());
        assert_eq!(settings.paste_method, PasteMethod::ClipboardPaste);
        assert_eq!(settings.defaults_version, CURRENT_DEFAULTS_VERSION);
    }

    #[test]
    fn does_not_override_deliberate_type_it_out_after_migration() {
        // A user who deliberately picks "Type it out" after the v5 migration
        // (defaults_version already current) is never flipped back.
        let mut settings = AppSettings {
            paste_method: PasteMethod::DirectInsert,
            ..AppSettings::default()
        };

        assert!(!settings.migrate_defaults());
        assert_eq!(settings.paste_method, PasteMethod::DirectInsert);
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
    fn migrates_auto_stop_to_enabled_with_60s_threshold() {
        let mut settings = AppSettings {
            defaults_version: 1,
            silence_auto_stop_enabled: false,
            silence_auto_stop_ms: 5_000,
            ..AppSettings::default()
        };

        assert!(settings.migrate_defaults());
        assert!(settings.silence_auto_stop_enabled);
        assert_eq!(settings.silence_auto_stop_ms, 60_000);
        assert_eq!(settings.defaults_version, CURRENT_DEFAULTS_VERSION);

        // Disabling after migration sticks.
        settings.silence_auto_stop_enabled = false;
        assert!(!settings.migrate_defaults());
        assert!(!settings.silence_auto_stop_enabled);
    }

    #[test]
    fn migrates_v3_defaults_to_v4_once() {
        let mut settings = AppSettings {
            defaults_version: 3,
            max_recording_ms: 180_000,
            silence_auto_stop_enabled: false,
            silence_auto_stop_ms: 5_000,
            save_audio_clips: false,
            hotkeys: HotkeySettings {
                hold_to_talk: "Ctrl+Win".to_string(),
                toggle_dictation: "Backquote".to_string(),
                paste_last_transcript: "Ctrl+Shift+V".to_string(),
                open_dashboard: "Ctrl+Alt+V".to_string(),
            },
            ..AppSettings::default()
        };

        assert!(settings.migrate_defaults());
        assert_eq!(settings.max_recording_ms, 600_000);
        assert!(settings.silence_auto_stop_enabled);
        assert_eq!(settings.silence_auto_stop_ms, 60_000);
        assert!(settings.save_audio_clips);
        assert_eq!(settings.hotkeys, HotkeySettings::default());
        assert_eq!(settings.defaults_version, CURRENT_DEFAULTS_VERSION);
    }

    #[test]
    fn v4_does_not_touch_customized_duration_or_hotkeys() {
        let mut settings = AppSettings {
            defaults_version: 3,
            max_recording_ms: 240_000,
            silence_auto_stop_ms: 10_000,
            hotkeys: HotkeySettings {
                hold_to_talk: "Ctrl+Win".to_string(),
                toggle_dictation: "F9".to_string(),
                paste_last_transcript: "Ctrl+Shift+V".to_string(),
                open_dashboard: "Ctrl+Alt+V".to_string(),
            },
            ..AppSettings::default()
        };

        assert!(settings.migrate_defaults());
        assert_eq!(settings.max_recording_ms, 240_000);
        assert_eq!(settings.silence_auto_stop_ms, 10_000);
        assert_eq!(settings.hotkeys.toggle_dictation, "F9");
        assert_eq!(settings.hotkeys.paste_last_transcript, "Ctrl+Shift+V");
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
        assert_eq!(settings.silence_auto_stop_ms, 60_000);
        assert!(settings.incremental_transcription_enabled);
        assert_eq!(settings.vocabulary_prompt, "");
        // The new field is absent from this legacy JSON, so #[serde(default)]
        // must fill it in as false rather than failing to deserialize.
        assert!(!settings.developer_settings_enabled);
        assert!(!settings.dev_hotkeys_seeded);
        assert_eq!(settings.output_mode, OutputMode::SaveOnly);
        assert_eq!(
            settings.pill_display_mode,
            PillDisplayMode::VisualizerWithText
        );
    }

    #[test]
    fn paste_method_serializes_with_new_name() {
        let json = serde_json::to_string(&PasteMethod::ClipboardPaste).unwrap();
        assert_eq!(json, "\"clipboard_paste\"");

        let json = serde_json::to_string(&PasteMethod::DirectInsert).unwrap();
        assert_eq!(json, "\"direct_insert\"");
    }

    #[test]
    fn paste_method_accepts_legacy_clipboard_restore_alias() {
        // Settings stored before the ClipboardRestore -> ClipboardPaste rename
        // must still deserialize so existing installs are not broken.
        let legacy: PasteMethod = serde_json::from_str("\"clipboard_restore\"").unwrap();
        assert_eq!(legacy, PasteMethod::ClipboardPaste);

        let current: PasteMethod = serde_json::from_str("\"clipboard_paste\"").unwrap();
        assert_eq!(current, PasteMethod::ClipboardPaste);
    }

    #[test]
    fn full_settings_json_with_legacy_paste_method_deserializes() {
        // A whole stored settings blob using the old paste-method string still
        // round-trips into the current AppSettings shape.
        let json = serde_json::json!({
            "launchAtStartup": false,
            "minimizeToTray": true,
            "showFloatingPill": true,
            "notificationsEnabled": true,
            "soundsEnabled": true,
            "recordingMode": "both",
            "minRecordingMs": 300,
            "maxRecordingMs": 600000,
            "silenceTrimEnabled": true,
            "selectedMicId": null,
            "selectedModelId": "small.en-q5_1",
            "language": "en",
            "outputMode": "auto_paste",
            "pasteMethod": "clipboard_restore",
            "historyEnabled": true,
            "saveAudioClips": true,
            "historyRetentionDays": 30,
            "hotkeys": {
                "holdToTalk": "Ctrl+Win",
                "toggleDictation": "Backquote",
                "pasteLastTranscript": "Ctrl+Alt+V",
                "openDashboard": "Ctrl+Alt+F"
            }
        });

        let settings: AppSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.paste_method, PasteMethod::ClipboardPaste);
    }

    #[test]
    fn default_hotkeys_avoid_windows_reserved_shortcuts() {
        let hotkeys = HotkeySettings::default();

        assert_eq!(hotkeys.hold_to_talk, "Ctrl+Win");
        assert_eq!(hotkeys.toggle_dictation, "Backquote");
        assert_eq!(hotkeys.paste_last_transcript, "Ctrl+Alt+V");
        assert_eq!(hotkeys.open_dashboard, "Ctrl+Alt+F");
    }

    #[test]
    fn migrates_v2_default_hotkeys_to_v3_once() {
        let mut settings = AppSettings {
            defaults_version: 2,
            hotkeys: HotkeySettings {
                hold_to_talk: "Ctrl+Shift".to_string(),
                toggle_dictation: "Backquote".to_string(),
                paste_last_transcript: "Ctrl+Alt+V".to_string(),
                open_dashboard: "Ctrl+Alt+D".to_string(),
            },
            ..AppSettings::default()
        };

        assert!(settings.migrate_defaults());
        assert_eq!(settings.hotkeys, HotkeySettings::default());
        assert_eq!(settings.defaults_version, CURRENT_DEFAULTS_VERSION);
    }

    #[test]
    fn does_not_migrate_customized_hotkeys_to_v3() {
        let mut settings = AppSettings {
            defaults_version: 2,
            hotkeys: HotkeySettings {
                hold_to_talk: "Ctrl+Shift".to_string(),
                toggle_dictation: "F9".to_string(),
                paste_last_transcript: "Ctrl+Alt+V".to_string(),
                open_dashboard: "Ctrl+Alt+D".to_string(),
            },
            ..AppSettings::default()
        };

        assert!(settings.migrate_defaults());
        assert_eq!(settings.hotkeys.toggle_dictation, "F9");
        assert_eq!(settings.hotkeys.hold_to_talk, "Ctrl+Shift");
        assert_eq!(settings.defaults_version, CURRENT_DEFAULTS_VERSION);
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
