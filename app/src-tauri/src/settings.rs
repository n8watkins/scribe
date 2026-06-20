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
    /// Selectable color theme for the MAIN window. The value is a theme key
    /// (e.g. "midnight", "ocean") that the frontend maps to a CSS palette via
    /// `data-theme`. Defaults to "midnight", which equals the historical look,
    /// so existing installs are visually unchanged. The floating pill keeps its
    /// own separate color settings and is not affected.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Poll GitHub for new releases in the background (on launch, on window
    /// focus, and on a timer). On by default; turn it off in About. Manual
    /// "Check for updates" still works when off.
    #[serde(default = "default_auto_update_check_enabled")]
    pub auto_update_check_enabled: bool,
    /// Silently download and install a detected update on launch behind a
    /// Scribe-branded screen (no native Windows installer popups), then
    /// relaunch. On by default; opt out in About. Only ever runs on launch
    /// (never mid-session) and only when `auto_update_check_enabled` is also on.
    /// Any failure falls back to the manual "Install" path — it never blocks
    /// the app.
    #[serde(default = "default_auto_install_updates")]
    pub auto_install_updates: bool,
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
    /// When incremental transcription is on, finalize the current segment after
    /// this much continuous silence (a pause). Turning this off leaves only the
    /// max-length cap, so a phrase is split only when it gets long — never at a
    /// pause.
    #[serde(default = "default_segment_pause_enabled")]
    pub segment_pause_enabled: bool,
    /// The pause length (ms) that finalizes a segment. Longer = fewer sentence
    /// breaks manufactured at brief mid-speech pauses (less stray punctuation),
    /// at a little more stop-to-text latency. Only used when
    /// `segment_pause_enabled`.
    #[serde(default = "default_segment_pause_ms")]
    pub segment_pause_ms: u32,
    /// Hard cap (ms) on segment length. Bounded to Whisper's safe window: it can
    /// only transcribe ~30 s at once, so a longer segment would be truncated and
    /// silently lose words. 25 s is the safe maximum, 10 s the minimum.
    #[serde(default = "default_segment_max_ms")]
    pub segment_max_ms: u32,
    pub selected_mic_id: Option<String>,
    pub selected_model_id: Option<String>,
    pub language: Language,
    /// Run Whisper's translate task: transcribe any spoken language and emit
    /// English. Requires a multilingual model. Off by default and absent from
    /// pre-multilingual settings JSON, so `#[serde(default)]` fills it false —
    /// existing installs are unaffected.
    #[serde(default)]
    pub translate_to_english: bool,
    /// Custom vocabulary / spelling hints passed to whisper.cpp via
    /// `--prompt` when non-empty.
    #[serde(default)]
    pub vocabulary_prompt: String,
    /// Deterministic post-transcription "say X -> get Y" replacements, applied
    /// to the final Whisper text. Distinct from `vocabulary_prompt`, which only
    /// biases recognition. Empty by default.
    #[serde(default)]
    pub text_replacements: Vec<crate::text_replace::TextReplacement>,
    pub output_mode: OutputMode,
    pub paste_method: PasteMethod,
    pub history_enabled: bool,
    pub save_audio_clips: bool,
    pub history_retention_days: Option<u16>,
    /// Auto-prune saved notes (`is_note = 1`) older than N days. Unlike
    /// dictation transcripts, notes are deliberate saves, so this defaults to
    /// `None` (keep forever) and the user opts in. Same allowed values as
    /// `history_retention_days`; tracked separately so the two never share a
    /// window.
    #[serde(default)]
    pub notes_retention_days: Option<u16>,
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
    /// Polish each finished dictation with the local LLM before it is saved or
    /// pasted (strip filler, fix punctuation/casing, light formatting). Off by
    /// default; non-blocking with a raw-text fallback (see `dictation_cleanup.rs`).
    /// Reuses the same `notes_analysis_endpoint` / `notes_analysis_model` server.
    #[serde(default)]
    pub dictation_cleanup_enabled: bool,
    /// Which built-in cleanup style to apply (or Custom to use the prompt below).
    #[serde(default = "default_dictation_cleanup_mode")]
    pub dictation_cleanup_mode: DictationCleanupMode,
    /// The system prompt used only when `dictation_cleanup_mode` is `Custom`.
    /// Blank falls back to the Standard prompt.
    #[serde(default)]
    pub dictation_cleanup_prompt: String,
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
    /// Waveform / dot color for normal dictation on the floating pill (CSS
    /// color string). Defaults to the shipped amber.
    #[serde(default = "default_pill_color_normal")]
    pub pill_color_normal: String,
    /// Waveform / dot color while a note-taking session is recording (CSS
    /// color string). Defaults to the shipped cyan.
    #[serde(default = "default_pill_color_note")]
    pub pill_color_note: String,
    /// Background color of the floating pill (hex; applied with slight
    /// translucency by the frontend). Defaults to the shipped dark blue.
    #[serde(default = "default_pill_color_background")]
    pub pill_color_background: String,
    /// Where Scribe stores FUTURE data (DB, clips, models) when the user picks
    /// a custom folder. None means the OS app-data directory. Changing it does
    /// not migrate existing data.
    #[serde(default)]
    pub data_dir: Option<String>,
    /// Saved main-window size (physical pixels). None means the shipped default
    /// from tauri.conf.json. Stored like pill_x/pill_y so the window reopens at
    /// the size the user last saved.
    #[serde(default)]
    pub window_width: Option<i32>,
    #[serde(default)]
    pub window_height: Option<i32>,
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

fn default_segment_pause_enabled() -> bool {
    true
}

fn default_segment_pause_ms() -> u32 {
    3_000
}

fn default_segment_max_ms() -> u32 {
    25_000
}

fn default_pill_display_mode() -> PillDisplayMode {
    PillDisplayMode::VisualizerWithText
}

fn default_dashboard_hotkey_toggles() -> bool {
    true
}

fn default_auto_update_check_enabled() -> bool {
    true
}

fn default_theme() -> String {
    // "midnight" maps to the historical default palette, so the app looks
    // identical when no theme has been chosen.
    "midnight".to_string()
}

fn default_auto_install_updates() -> bool {
    true
}

fn default_pill_color_normal() -> String {
    "#fbbf24".to_string()
}

fn default_pill_color_note() -> String {
    "#38bdf8".to_string()
}

fn default_pill_color_background() -> String {
    "#0f1e38".to_string()
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

/// The transcription language preference, stored as a lowercase ISO-639-1 code
/// (e.g. "en", "es", "fr") or the sentinel "auto" for Whisper's language
/// auto-detection.
///
/// Serialized transparently as a bare string, so settings JSON written by
/// earlier English-only builds — which only ever stored `"auto"` or `"en"` —
/// deserializes unchanged. New codes round-trip the same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Language(String);

/// Whisper's auto-detect sentinel.
pub const LANGUAGE_AUTO: &str = "auto";

/// Curated set of languages offered in the UI: the code Whisper expects and an
/// English display name. Deliberately a sensible subset of Whisper's ~99
/// languages (not the full list, which is unwieldy in a dropdown). "auto" is
/// surfaced separately as the auto-detect option, so it is not repeated here.
/// Any code Whisper itself accepts still works if stored directly; this list
/// only drives the picker and validation.
pub const SUPPORTED_LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("nl", "Dutch"),
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("pl", "Polish"),
    ("tr", "Turkish"),
    ("sv", "Swedish"),
    ("no", "Norwegian"),
    ("da", "Danish"),
    ("fi", "Finnish"),
    ("cs", "Czech"),
    ("el", "Greek"),
    ("ro", "Romanian"),
    ("hu", "Hungarian"),
    ("ar", "Arabic"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("id", "Indonesian"),
    ("vi", "Vietnamese"),
    ("th", "Thai"),
    ("ko", "Korean"),
    ("ja", "Japanese"),
    ("zh", "Chinese"),
    ("ca", "Catalan"),
];

impl Language {
    /// Auto-detect (`"auto"`).
    pub fn auto() -> Self {
        Self(LANGUAGE_AUTO.to_string())
    }

    /// English (`"en"`), the historical default.
    pub fn english() -> Self {
        Self("en".to_string())
    }

    /// The raw stored code ("auto" or an ISO-639-1 code).
    pub fn code(&self) -> &str {
        &self.0
    }

    /// True for the auto-detect sentinel.
    pub fn is_auto(&self) -> bool {
        self.0 == LANGUAGE_AUTO
    }

    /// True when the selection is plain English (so an English-only model is
    /// fine). Auto-detect is treated as needing a multilingual model.
    pub fn is_english(&self) -> bool {
        self.0 == "en"
    }

    /// Whether the code is recognized: "auto" or one of the curated codes.
    pub fn is_known(&self) -> bool {
        self.is_auto() || SUPPORTED_LANGUAGES.iter().any(|(code, _)| *code == self.0)
    }
}

impl Default for Language {
    fn default() -> Self {
        Self::english()
    }
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

/// How an LLM cleanup pass should reshape a finished dictation. The mode
/// selects a built-in system prompt (see `dictation_cleanup.rs`); `Custom`
/// uses the user's own `dictation_cleanup_prompt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DictationCleanupMode {
    /// Punctuation/capitalization fixes + filler removal, wording faithful.
    Standard,
    /// Standard cleanup, then format as a polite email body.
    Email,
    /// Standard cleanup, condensed into a concise casual chat message.
    Chat,
    /// Standard cleanup, phrased/formatted as a code comment.
    Code,
    /// Use the user's `dictation_cleanup_prompt` verbatim.
    Custom,
}

fn default_dictation_cleanup_mode() -> DictationCleanupMode {
    DictationCleanupMode::Standard
}

/// Which key edge a single-shot hotkey acts on. Hold-to-Talk is excluded — it
/// is push-to-talk and inherently uses both edges (press starts, release stops).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TriggerEdge {
    /// Fire as soon as the chord is pressed down.
    #[default]
    Press,
    /// Fire when the chord is released. Required for the Toggle key's
    /// hold-and-tap-Q note chord, which needs the key to stay held to arm Q.
    Release,
}

fn default_discard_dictation() -> String {
    "Ctrl+Alt+X".to_string()
}

fn default_transform_selection() -> String {
    "Ctrl+Alt+R".to_string()
}

fn default_toggle_trigger() -> TriggerEdge {
    // Toggle fires on release by default so the note chord (hold the toggle
    // key, tap Q) keeps working, matching the shipped Backquote behavior.
    TriggerEdge::Release
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeySettings {
    pub hold_to_talk: String,
    pub toggle_dictation: String,
    pub paste_last_transcript: String,
    pub open_dashboard: String,
    /// Cancels (discards) the in-progress recording without transcribing.
    /// Single-shot; absent from pre-discard settings JSON, so #[serde(default)]
    /// fills the shipped Ctrl+Alt+X.
    #[serde(default = "default_discard_dictation")]
    pub discard_dictation: String,
    /// Selected-text transform: copy the highlighted text, rewrite it with the
    /// local LLM per a typed/spoken instruction, and paste the result back over
    /// the selection. Single-shot; absent from pre-transform settings JSON, so
    /// #[serde(default)] fills the shipped Ctrl+Alt+R.
    #[serde(default = "default_transform_selection")]
    pub transform_selection: String,
    /// Which edge Toggle Dictation acts on. Release (default) keeps the
    /// hold-and-tap-Q note chord; Press fires immediately and disables it.
    #[serde(default = "default_toggle_trigger")]
    pub toggle_dictation_trigger: TriggerEdge,
    /// Which edge Paste Last Transcript acts on (default Press).
    #[serde(default)]
    pub paste_last_transcript_trigger: TriggerEdge,
    /// Which edge Open Dashboard acts on (default Press).
    #[serde(default)]
    pub open_dashboard_trigger: TriggerEdge,
    /// Which edge Discard / Cancel acts on (default Press).
    #[serde(default)]
    pub discard_dictation_trigger: TriggerEdge,
    /// Which edge Transform Selection acts on (default Press).
    #[serde(default)]
    pub transform_selection_trigger: TriggerEdge,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            hold_to_talk: "Ctrl+Win".to_string(),
            toggle_dictation: "Backquote".to_string(),
            paste_last_transcript: "Ctrl+Alt+V".to_string(),
            open_dashboard: "Ctrl+Alt+F".to_string(),
            discard_dictation: default_discard_dictation(),
            transform_selection: default_transform_selection(),
            toggle_dictation_trigger: TriggerEdge::Release,
            paste_last_transcript_trigger: TriggerEdge::Press,
            open_dashboard_trigger: TriggerEdge::Press,
            discard_dictation_trigger: TriggerEdge::Press,
            transform_selection_trigger: TriggerEdge::Press,
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
            discard_dictation: "Ctrl+Alt+Shift+X".to_string(),
            transform_selection: "Ctrl+Alt+Shift+R".to_string(),
            toggle_dictation_trigger: TriggerEdge::Release,
            paste_last_transcript_trigger: TriggerEdge::Press,
            open_dashboard_trigger: TriggerEdge::Press,
            discard_dictation_trigger: TriggerEdge::Press,
            transform_selection_trigger: TriggerEdge::Press,
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
            auto_update_check_enabled: true,
            theme: default_theme(),
            auto_install_updates: true,
            dev_hotkeys_seeded: false,
            recording_mode: RecordingMode::Both,
            min_recording_ms: 300,
            max_recording_ms: 600_000,
            silence_trim_enabled: true,
            silence_auto_stop_enabled: default_silence_auto_stop_enabled(),
            silence_auto_stop_ms: default_silence_auto_stop_ms(),
            incremental_transcription_enabled: default_incremental_transcription_enabled(),
            segment_pause_enabled: default_segment_pause_enabled(),
            segment_pause_ms: default_segment_pause_ms(),
            segment_max_ms: default_segment_max_ms(),
            selected_mic_id: None,
            selected_model_id: Some("small.en-q5_1".to_string()),
            language: Language::english(),
            translate_to_english: false,
            vocabulary_prompt: String::new(),
            text_replacements: Vec::new(),
            output_mode: OutputMode::AutoPaste,
            paste_method: PasteMethod::ClipboardPaste,
            history_enabled: true,
            save_audio_clips: true,
            history_retention_days: Some(30),
            notes_retention_days: None,
            notes_analysis_enabled: false,
            notes_analysis_prompt: default_notes_analysis_prompt(),
            notes_analysis_endpoint: default_notes_analysis_endpoint(),
            notes_analysis_model: String::new(),
            dictation_cleanup_enabled: false,
            dictation_cleanup_mode: default_dictation_cleanup_mode(),
            dictation_cleanup_prompt: String::new(),
            drive_sync_enabled: false,
            drive_sync_all_transcripts: false,
            drive_organize_hour: default_drive_organize_hour(),
            drive_organize_enabled: false,
            drive_organize_prompt: default_drive_organize_prompt(),
            drive_last_organized_date: String::new(),
            drive_account_email: String::new(),
            pill_x: None,
            pill_y: None,
            pill_color_normal: default_pill_color_normal(),
            pill_color_note: default_pill_color_note(),
            pill_color_background: default_pill_color_background(),
            data_dir: None,
            window_width: None,
            window_height: None,
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

        if !(200..=10_000).contains(&self.segment_pause_ms) {
            return Err(SettingsValidationError::new(
                "segmentPauseMs must be between 200 and 10000.",
            ));
        }

        // Bounded to Whisper's ~30 s window so a segment can never be truncated.
        if !(10_000..=25_000).contains(&self.segment_max_ms) {
            return Err(SettingsValidationError::new(
                "segmentMaxMs must be between 10000 and 25000.",
            ));
        }

        // The language must be "auto" or a code from the curated list. Legacy
        // values ("auto"/"en") are covered by the list, so old settings stay
        // valid.
        if !self.language.is_known() {
            return Err(SettingsValidationError::new(
                "language must be \"auto\" or a supported ISO-639-1 code.",
            ));
        }

        if !matches!(self.history_retention_days, Some(7 | 30 | 90 | 365) | None) {
            return Err(SettingsValidationError::new(
                "historyRetentionDays must be 7, 30, 90, 365, or null.",
            ));
        }

        if !matches!(self.notes_retention_days, Some(7 | 30 | 90 | 365) | None) {
            return Err(SettingsValidationError::new(
                "notesRetentionDays must be 7, 30, 90, 365, or null.",
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
        assert!(settings.segment_pause_enabled);
        assert_eq!(settings.segment_pause_ms, 3_000);
        assert_eq!(settings.segment_max_ms, 25_000);
        assert_eq!(settings.vocabulary_prompt, "");
        assert!(settings.text_replacements.is_empty());
        assert!(settings.history_enabled);
        assert!(settings.save_audio_clips);
        assert_eq!(settings.history_retention_days, Some(30));
        // Notes are deliberate saves: kept forever unless the user opts in.
        assert_eq!(settings.notes_retention_days, None);
        assert_eq!(
            settings.pill_display_mode,
            PillDisplayMode::VisualizerWithText
        );
        assert!(!settings.notes_analysis_enabled);
        assert!(!settings.notes_analysis_prompt.is_empty());
        assert_eq!(settings.notes_analysis_endpoint, "http://127.0.0.1:1234/v1");
        assert_eq!(settings.notes_analysis_model, "");
        // Dictation cleanup is opt-in and defaults to the Standard style.
        assert!(!settings.dictation_cleanup_enabled);
        assert_eq!(
            settings.dictation_cleanup_mode,
            DictationCleanupMode::Standard
        );
        assert_eq!(settings.dictation_cleanup_prompt, "");
        assert!(!settings.developer_settings_enabled);
        assert!(!settings.dev_hotkeys_seeded);
        // The default theme equals the historical look so installs are unchanged.
        assert_eq!(settings.theme, "midnight");
        assert_eq!(settings.pill_color_normal, "#fbbf24");
        assert_eq!(settings.pill_color_note, "#38bdf8");
    }

    #[test]
    fn language_legacy_values_round_trip() {
        // The two values earlier builds ever stored must still deserialize, as
        // the bare strings they always were.
        let auto: Language = serde_json::from_str("\"auto\"").unwrap();
        assert!(auto.is_auto());
        assert_eq!(auto.code(), "auto");

        let en: Language = serde_json::from_str("\"en\"").unwrap();
        assert!(en.is_english());
        assert_eq!(en.code(), "en");

        // And they serialize back to the same bare strings (transparent repr).
        assert_eq!(serde_json::to_string(&Language::auto()).unwrap(), "\"auto\"");
        assert_eq!(
            serde_json::to_string(&Language::english()).unwrap(),
            "\"en\""
        );
    }

    #[test]
    fn language_new_codes_round_trip_and_validate() {
        let es: Language = serde_json::from_str("\"es\"").unwrap();
        assert_eq!(es.code(), "es");
        assert!(es.is_known());
        assert!(!es.is_english());
        assert!(!es.is_auto());

        let mut settings = AppSettings {
            language: es,
            ..AppSettings::default()
        };
        assert!(settings.validate().is_ok());

        // An unknown code is rejected by validation.
        settings.language = Language("zz".to_string());
        assert!(settings.validate().is_err());
    }

    #[test]
    fn translate_to_english_defaults_off_and_is_absent_from_legacy_json() {
        assert!(!AppSettings::default().translate_to_english);

        // Legacy settings JSON never had translateToEnglish; serde default
        // fills it false so existing installs keep the English-only behavior.
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
            "pasteMethod": "clipboard_paste",
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
        assert!(!settings.translate_to_english);
        assert!(settings.language.is_english());
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
    fn validates_notes_retention_options() {
        let mut settings = AppSettings::default();
        // Same allowed set as transcripts; an off-list value is rejected.
        settings.notes_retention_days = Some(14);
        assert!(settings.validate().is_err());

        for ok in [None, Some(7), Some(30), Some(90), Some(365)] {
            settings.notes_retention_days = ok;
            assert!(settings.validate().is_ok());
        }
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
    fn validates_segment_pause_and_max_ranges() {
        let mut settings = AppSettings::default();

        // Pause threshold: 200..=10000 ms.
        settings.segment_pause_ms = 199;
        assert!(settings.validate().is_err());
        settings.segment_pause_ms = 200;
        assert!(settings.validate().is_ok());
        settings.segment_pause_ms = 10_000;
        assert!(settings.validate().is_ok());
        settings.segment_pause_ms = 10_001;
        assert!(settings.validate().is_err());
        settings.segment_pause_ms = default_segment_pause_ms();

        // Max segment length is clamped to Whisper's safe window: 10000..=25000.
        settings.segment_max_ms = 9_999;
        assert!(settings.validate().is_err());
        settings.segment_max_ms = 10_000;
        assert!(settings.validate().is_ok());
        settings.segment_max_ms = 25_000;
        assert!(settings.validate().is_ok());
        settings.segment_max_ms = 25_001;
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
                ..Default::default()
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
                ..Default::default()
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
        // Absent from this legacy JSON; #[serde(default)] fills it as empty.
        assert!(settings.text_replacements.is_empty());
        // The new field is absent from this legacy JSON, so #[serde(default)]
        // must fill it in as false rather than failing to deserialize.
        assert!(!settings.developer_settings_enabled);
        // `theme` is absent from this legacy JSON; the serde default fills it
        // with "midnight" so existing installs keep the historical look.
        assert_eq!(settings.theme, "midnight");
        assert!(!settings.dev_hotkeys_seeded);
        // Absent from this legacy JSON, so the serde defaults fill them in.
        assert_eq!(settings.pill_color_normal, "#fbbf24");
        assert_eq!(settings.pill_color_note, "#38bdf8");
        assert_eq!(settings.output_mode, OutputMode::SaveOnly);
        assert_eq!(
            settings.pill_display_mode,
            PillDisplayMode::VisualizerWithText
        );
        // Trigger fields are absent from this legacy hotkeys JSON, so the serde
        // defaults preserve today's behavior: toggle on release (note chord),
        // paste/dashboard on press.
        assert_eq!(
            settings.hotkeys.toggle_dictation_trigger,
            TriggerEdge::Release
        );
        assert_eq!(
            settings.hotkeys.paste_last_transcript_trigger,
            TriggerEdge::Press
        );
        assert_eq!(settings.hotkeys.open_dashboard_trigger, TriggerEdge::Press);
    }

    #[test]
    fn hotkey_trigger_defaults_preserve_current_behavior() {
        let defaults = HotkeySettings::default();
        assert_eq!(defaults.toggle_dictation_trigger, TriggerEdge::Release);
        assert_eq!(defaults.paste_last_transcript_trigger, TriggerEdge::Press);
        assert_eq!(defaults.open_dashboard_trigger, TriggerEdge::Press);

        let dev = HotkeySettings::dev_defaults();
        assert_eq!(dev.toggle_dictation_trigger, TriggerEdge::Release);
        assert_eq!(dev.paste_last_transcript_trigger, TriggerEdge::Press);
        assert_eq!(dev.open_dashboard_trigger, TriggerEdge::Press);
    }

    #[test]
    fn explicit_triggers_round_trip_through_json() {
        let mut settings = AppSettings::default();
        settings.hotkeys.toggle_dictation_trigger = TriggerEdge::Press;
        settings.hotkeys.paste_last_transcript_trigger = TriggerEdge::Release;

        let json = serde_json::to_string(&settings).unwrap();
        // snake_case rename: enum variants serialize lowercase.
        assert!(json.contains("\"toggleDictationTrigger\":\"press\""));
        assert!(json.contains("\"pasteLastTranscriptTrigger\":\"release\""));

        let restored: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(
            restored.hotkeys.toggle_dictation_trigger,
            TriggerEdge::Press
        );
        assert_eq!(
            restored.hotkeys.paste_last_transcript_trigger,
            TriggerEdge::Release
        );
        assert_eq!(restored.hotkeys.open_dashboard_trigger, TriggerEdge::Press);
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
                ..Default::default()
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
                ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
