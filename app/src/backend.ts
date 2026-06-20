import { invoke } from "@tauri-apps/api/core";

export type AppStatus =
  | "Idle"
  | "Recording"
  | "Stopping"
  | "Transcribing"
  | "Pasting"
  | "Ready"
  | "Error"
  | "Paused";

export type AppErrorInfo = {
  code: string;
  message: string;
};

export type AppStateSnapshot = {
  status: AppStatus;
  error: AppErrorInfo | null;
  updatedAt: string;
};

export type RecordingMode = "hold" | "toggle" | "both";
/** Transcription language preference: the Whisper auto-detect sentinel "auto",
 * or a lowercase ISO-639-1 code ("en", "es", "fr", ...). Stored as a bare
 * string, so values written by older English-only builds ("auto"/"en") still
 * load. See SUPPORTED_LANGUAGES for the curated picker set. */
export type Language = string;

/** Auto-detect sentinel value for `AppSettings.language`. */
export const LANGUAGE_AUTO = "auto";

/** Curated [code, label] languages for the picker — a sensible subset of
 * Whisper's ~99 languages. Kept in sync with `SUPPORTED_LANGUAGES` in the Rust
 * `settings.rs`. "auto" (auto-detect) is offered separately in the UI. */
export const SUPPORTED_LANGUAGES: readonly (readonly [string, string])[] = [
  ["en", "English"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["it", "Italian"],
  ["pt", "Portuguese"],
  ["nl", "Dutch"],
  ["ru", "Russian"],
  ["uk", "Ukrainian"],
  ["pl", "Polish"],
  ["tr", "Turkish"],
  ["sv", "Swedish"],
  ["no", "Norwegian"],
  ["da", "Danish"],
  ["fi", "Finnish"],
  ["cs", "Czech"],
  ["el", "Greek"],
  ["ro", "Romanian"],
  ["hu", "Hungarian"],
  ["ar", "Arabic"],
  ["he", "Hebrew"],
  ["hi", "Hindi"],
  ["id", "Indonesian"],
  ["vi", "Vietnamese"],
  ["th", "Thai"],
  ["ko", "Korean"],
  ["ja", "Japanese"],
  ["zh", "Chinese"],
  ["ca", "Catalan"],
];
export type OutputMode =
  | "save_only"
  | "auto_paste"
  | "copy_clipboard"
  | "copy_and_paste";
// "clipboard_paste" is the current opt-in mode. "clipboard_restore" is its
// legacy name, still accepted by the Rust enum via a serde alias and kept here
// for back-compat (e.g. settings stored before the rename, and the UI's
// not-yet-migrated option list). New code should use "clipboard_paste".
export type PasteMethod =
  | "direct_insert"
  | "clipboard_paste"
  | "clipboard_restore";
export type HistoryRetentionDays = 7 | 30 | 90 | 365 | null;
export type PillDisplayMode = "dot" | "visualizer" | "visualizer_with_text";
/** Which built-in style the optional dictation cleanup pass applies (or
 * "custom" to use `dictationCleanupPrompt`). */
export type DictationCleanupMode =
  | "standard"
  | "email"
  | "chat"
  | "code"
  | "custom";

/** Which key edge a single-shot bind acts on. Hold-to-Talk has no trigger —
 * it is push-to-talk and uses both edges (press starts, release stops). */
export type TriggerEdge = "press" | "release";

export type HotkeySettings = {
  holdToTalk: string;
  toggleDictation: string;
  pasteLastTranscript: string;
  openDashboard: string;
  discardDictation: string;
  /** Selected-text transform: rewrite the highlighted text with the local LLM
   * per a typed/spoken instruction, replacing it in place. */
  transformSelection: string;
  toggleDictationTrigger: TriggerEdge;
  pasteLastTranscriptTrigger: TriggerEdge;
  openDashboardTrigger: TriggerEdge;
  discardDictationTrigger: TriggerEdge;
  transformSelectionTrigger: TriggerEdge;
};

/** Deterministic post-transcription replacement: whenever `from` is spoken,
 * Scribe writes `to`. Matched case-insensitively on word boundaries. */
export type TextReplacement = { from: string; to: string };

export type AppSettings = {
  defaultsVersion: number;
  launchAtStartup: boolean;
  minimizeToTray: boolean;
  showFloatingPill: boolean;
  pillDisplayMode: PillDisplayMode;
  dashboardHotkeyToggles: boolean;
  notificationsEnabled: boolean;
  soundsEnabled: boolean;
  developerSettingsEnabled: boolean;
  autoUpdateCheckEnabled: boolean;
  autoInstallUpdates: boolean;
  /** Selected color theme key for the main window (e.g. "midnight", "ocean").
   * Maps to a CSS palette via `data-theme`; "midnight" is the default look. */
  theme: string;
  devHotkeysSeeded: boolean;
  recordingMode: RecordingMode;
  minRecordingMs: number;
  maxRecordingMs: number;
  silenceTrimEnabled: boolean;
  silenceAutoStopEnabled: boolean;
  silenceAutoStopMs: number;
  incrementalTranscriptionEnabled: boolean;
  selectedMicId: string | null;
  selectedModelId: string | null;
  language: Language;
  /** Run Whisper's translate task: emit English for any spoken language.
   * Requires a multilingual model. Defaults false. */
  translateToEnglish: boolean;
  vocabularyPrompt: string;
  textReplacements: TextReplacement[];
  outputMode: OutputMode;
  pasteMethod: PasteMethod;
  historyEnabled: boolean;
  saveAudioClips: boolean;
  historyRetentionDays: HistoryRetentionDays;
  notesRetentionDays: HistoryRetentionDays;
  notesAnalysisEnabled: boolean;
  notesAnalysisPrompt: string;
  notesAnalysisEndpoint: string;
  notesAnalysisModel: string;
  dictationCleanupEnabled: boolean;
  dictationCleanupMode: DictationCleanupMode;
  dictationCleanupPrompt: string;
  driveSyncEnabled: boolean;
  driveSyncAllTranscripts: boolean;
  driveOrganizeHour: number;
  driveOrganizeEnabled: boolean;
  driveOrganizePrompt: string;
  driveLastOrganizedDate: string;
  driveAccountEmail: string;
  hotkeys: HotkeySettings;
  pillX: number | null;
  pillY: number | null;
  pillColorNormal: string;
  pillColorNote: string;
  pillColorBackground: string;
  /** Custom data directory for FUTURE data; null uses the OS app-data dir. */
  dataDir: string | null;
  /** Saved default main-window size (physical pixels); null uses the config default. */
  windowWidth: number | null;
  windowHeight: number | null;
};

export type Transcript = {
  id: string;
  text: string;
  createdAt: string;
  durationMs: number | null;
  wordCount: number;
  characterCount: number;
  modelId: string | null;
  language: string | null;
  outputMode: OutputMode | null;
  pasteMethod: PasteMethod | null;
  transcriptionLatencyMs: number | null;
  audioPath: string | null;
  isNote: boolean;
  analysis: string | null;
  analysisModel: string | null;
  analysisCreatedAt: string | null;
};

export type TranscriptSearchResult = {
  transcripts: Transcript[];
  total: number;
  limit: number;
  offset: number;
};

export type TranscriptSort = "newest" | "oldest" | "longest";

export type OutputAction =
  | "save_only"
  | "copy_clipboard"
  | "paste"
  | "copy_and_paste";

export type OutputStatus = "completed" | "clipboard_restore_failed";

export type ClipboardPreservation =
  // The system clipboard was never read or written (the opt-in keystroke insert).
  | "untouched"
  // The transcript was pasted via a borrowed clipboard, then the user's
  // previous clipboard text was restored (the default paste).
  | "restored_after_paste"
  // The transcript was pasted, but the previous clipboard text could not be
  // restored, so the transcript is still on the clipboard.
  | "restore_failed"
  // The transcript was placed on the clipboard and left there on purpose
  // (Copy and Copy + Paste output modes).
  | "replaced_with_transcript";

export type OutputResult = {
  transcriptId: string;
  action: OutputAction;
  status: OutputStatus;
  outputMode: OutputMode;
  pasteMethod: PasteMethod | null;
  copied: boolean;
  pasted: boolean;
  clipboardRestored: boolean | null;
  clipboardPreservation: ClipboardPreservation;
  clipboardRestoreError: string | null;
  message: string;
};

export type BasicStats = {
  wordsToday: number;
  dictationsToday: number;
  averageWpm: number | null;
  averageTranscriptionLatencyMs: number | null;
  averageRecordingDurationMs: number | null;
  mostUsedModel: string | null;
  totalWordsTranscribed: number;
};

export type ModelStatus =
  | "not_downloaded"
  | "downloading"
  | "downloaded"
  | "selected"
  | "loaded"
  | "failed"
  | "update_available";

export type ModelChecksum = {
  kind: "sha1";
  value: string;
};

export type ModelSource = "app_data" | "external_cache";

export type ModelInfo = {
  id: string;
  name: string;
  filename: string;
  downloadUrl: string;
  diskSizeLabel: string;
  localPath: string | null;
  source: ModelSource | null;
  sizeBytes: number | null;
  status: ModelStatus;
  checksum: ModelChecksum | null;
  selected: boolean;
  downloadedAt: string | null;
  /** Whether the model can transcribe non-English languages (and translate to
   * English). English-only (`.en`) builds are false. */
  multilingual: boolean;
};

export type ModelDownloadProgress = {
  modelId: string;
  bytesDownloaded: number;
  totalBytes: number | null;
  percent: number | null;
  status: ModelStatus;
};

export type MicrophoneInfo = {
  id: string;
  name: string;
  endpointId: string | null;
  isDefault: boolean;
  isSelected: boolean;
  isAvailable: boolean;
};

export type StartRecordingRequest = {
  microphoneId?: string | null;
  maxDurationMs?: number;
  isNote?: boolean;
};

export type RecordingSessionInfo = {
  sessionId: string;
  microphoneId: string;
  microphoneName: string;
  sampleRate: number;
  channels: number;
  startedAt: string;
  maxDurationMs: number;
  isTestClip: boolean;
  isNote: boolean;
};

export type RecordingResultStatus =
  | "completed"
  | "too_short"
  | "cancelled"
  | "timed_out";

export type RecordingResult = {
  sessionId: string;
  status: RecordingResultStatus;
  wavPath: string | null;
  durationMs: number;
  sampleRate: number;
  channels: number;
  bitsPerSample: number;
  bytesWritten: number | null;
  reason: string | null;
  startedAt: string;
  stoppedAt: string;
  isNote: boolean;
};

export type AudioLevelEvent = {
  sessionId: string;
  level: number;
  peak: number;
  rms: number;
};

export type RecordingErrorEvent = {
  code: string;
  message: string;
};

export type PartialTranscriptEvent = {
  sessionId: string;
  text: string;
  segments: number;
  finalized: boolean;
};

export type TranscribeFileResult = {
  text: string;
  durationMs: number | null;
  latencyMs: number;
};

export type DictationResult = {
  sessionId: string;
  status: "saved";
  transcript: Transcript;
  modelId: string;
  durationMs: number;
  transcriptionLatencyMs: number;
};

export type CommandError = {
  code?: string;
  message?: string;
};

export type HotkeyAction =
  | "holdToTalk"
  | "toggleDictation"
  | "pasteLastTranscript"
  | "openDashboard"
  | "discardDictation"
  | "transformSelection";

export type HotkeyBinding = {
  action: string;
  shortcut: string;
  normalizedShortcut: string | null;
  /** Which key edge this bind acts on, or null for Hold-to-Talk (a hold). */
  trigger: TriggerEdge | null;
  registered: boolean;
  error: string | null;
};

export type HotkeyStatus = {
  bindings: HotkeyBinding[];
  /** Whether the hold-toggle-key + tap-Q note chord is currently usable. */
  noteChordActive: boolean;
  holdReleaseVerificationRequired: boolean;
  windowsFallbackNote: string;
};

export type HotkeyRegistrationFailure = {
  action: string;
  shortcut: string;
  message: string;
};

export type HotkeyRegistrationFailedEvent = {
  failures: HotkeyRegistrationFailure[];
};

export type DashboardData = {
  appState: AppStateSnapshot;
  settings: AppSettings;
  lastTranscript: Transcript | null;
  recentTranscripts: Transcript[];
  stats: BasicStats;
};

export async function getDashboardData(limit = 8): Promise<DashboardData> {
  const [appState, settings, lastTranscript, recentTranscripts, stats] =
    await Promise.all([
      getAppState(),
      getSettings(),
      getLastTranscript(),
      listRecentTranscripts({ limit }),
      getBasicStats(),
    ]);

  return { appState, settings, lastTranscript, recentTranscripts, stats };
}

export function getAppState(): Promise<AppStateSnapshot> {
  return invoke("get_app_state");
}

export function getSettings(): Promise<AppSettings> {
  return invoke("get_settings");
}

export function updateSettings(settings: AppSettings): Promise<AppSettings> {
  return invoke("update_settings", { settings });
}

export function getLastTranscript(): Promise<Transcript | null> {
  return invoke("get_last_transcript");
}

export function clearLastTranscript(): Promise<void> {
  return invoke("clear_last_transcript");
}

export function pasteLastTranscript(): Promise<OutputResult> {
  return invoke("paste_last_transcript");
}

export function copyLastTranscript(): Promise<OutputResult> {
  return invoke("copy_last_transcript");
}

export function pasteTranscript(id: string): Promise<OutputResult> {
  return invoke("paste_transcript", { id });
}

export function copyTranscript(id: string): Promise<OutputResult> {
  return invoke("copy_transcript", { id });
}

/** Selected-text transform: copies whatever text is highlighted in the focused
 * app, rewrites it with the local LLM per `instruction`, and pastes the result
 * back over the selection. Windows-only (errors elsewhere). */
export function transformSelection(
  instruction: string,
): Promise<OutputResult> {
  return invoke("transform_selection", { instruction });
}

export function listRecentTranscripts({
  limit,
}: {
  limit?: number;
} = {}): Promise<Transcript[]> {
  return invoke("list_recent_transcripts", { limit });
}

export function searchTranscripts({
  query,
  notesOnly,
  from,
  to,
  sort,
  limit,
  offset,
}: {
  query?: string;
  notesOnly?: boolean;
  /** Inclusive RFC3339 lower bound on createdAt. */
  from?: string;
  /** Inclusive RFC3339 upper bound on createdAt. */
  to?: string;
  sort?: TranscriptSort;
  limit?: number;
  offset?: number;
} = {}): Promise<TranscriptSearchResult> {
  return invoke("search_transcripts", {
    query,
    notesOnly,
    from,
    to,
    sort,
    limit,
    offset,
  });
}

export function getTranscript(id: string): Promise<Transcript | null> {
  return invoke("get_transcript", { id });
}

export function updateTranscript(id: string, text: string): Promise<Transcript> {
  return invoke("update_transcript", { id, text });
}

/** Returns the saved dictation clip for a transcript as base64 WAV. */
export function getTranscriptAudio(id: string): Promise<string> {
  return invoke("get_transcript_audio", { id });
}

export function deleteTranscript(id: string): Promise<void> {
  return invoke("delete_transcript", { id });
}

export function clearTranscriptHistory(): Promise<void> {
  return invoke("clear_transcript_history");
}

/** Deletes only notes (is_note=1); leaves dictation transcripts intact. */
export function clearNotes(): Promise<void> {
  return invoke("clear_notes");
}

/** Loads the given transcripts (oldest-first) and joins their text with
 * `separator` (default "\n\n"). Ids that don't resolve are skipped. */
export function combineTranscripts(
  ids: string[],
  separator?: string,
): Promise<string> {
  return invoke("combine_transcripts", { ids, separator });
}

/** Saves `text` as a new history entry and the Last Transcript Buffer;
 * returns the saved transcript. */
export function saveCombinedTranscript(text: string): Promise<Transcript> {
  return invoke("save_combined_transcript", { text });
}

/** Writes a transcript to a temp .txt file and opens it in the OS default app. */
export function openTranscriptExternally(id: string): Promise<void> {
  return invoke("open_transcript_externally", { id });
}

export function getBasicStats(): Promise<BasicStats> {
  return invoke("get_basic_stats");
}

export function refreshBasicStats(): Promise<BasicStats> {
  return invoke("refresh_basic_stats");
}

export function listModels(): Promise<ModelInfo[]> {
  return invoke("list_models");
}

export function downloadModel(modelId: string): Promise<ModelInfo> {
  return invoke("download_model", { modelId });
}

export function cancelModelDownload(modelId: string): Promise<void> {
  return invoke("cancel_model_download", { modelId });
}

export function retryModelDownload(modelId: string): Promise<ModelInfo> {
  return invoke("retry_model_download", { modelId });
}

export function deleteModel(modelId: string): Promise<ModelInfo> {
  return invoke("delete_model", { modelId });
}

export function selectModel(modelId: string): Promise<ModelInfo> {
  return invoke("select_model", { modelId });
}

export function listMicrophones(): Promise<MicrophoneInfo[]> {
  return invoke("list_microphones");
}

export function startRecording(
  request?: StartRecordingRequest,
): Promise<RecordingSessionInfo> {
  return invoke("start_recording", { request });
}

export function stopRecording(): Promise<RecordingResult> {
  return invoke("stop_recording");
}

export function cancelRecording(): Promise<void> {
  return invoke("cancel_recording");
}

export function recordTestClip(durationMs?: number): Promise<RecordingResult> {
  return invoke("record_test_clip", { durationMs });
}

export function getTestClipAudio(): Promise<string> {
  return invoke("get_test_clip_audio");
}

export function openDataFolder(): Promise<void> {
  return invoke("open_data_folder");
}

export function openModelsFolder(): Promise<void> {
  return invoke("open_models_folder");
}

/** Opens the folder holding Scribe's rotating local log files, so the user can
 * find them to attach to a bug report. */
export function openLogsFolder(): Promise<void> {
  return invoke("open_logs_folder");
}

/** The current effective data directory (custom when set, else the OS app-data
 * dir) as a display string. */
export function getDataDir(): Promise<string> {
  return invoke("get_data_dir");
}

/** The OS log directory where Scribe writes its rotating log files. */
export function getLogsDir(): Promise<string> {
  return invoke("get_logs_dir");
}

/** Opens a native folder picker for the data directory; resolves to the chosen
 * absolute path, or null when the user cancels. Persist the choice with
 * updateSettings({ dataDir }). */
export function pickDataDir(): Promise<string | null> {
  return invoke("pick_data_dir");
}

/** Reads the main window's current size and saves it as the default; returns
 * the updated settings. */
export function saveWindowSize(): Promise<AppSettings> {
  return invoke("save_window_size");
}

export function transcribeRecording(
  recording: RecordingResult,
): Promise<DictationResult | null> {
  return invoke("transcribe_recording", { recording });
}

/** Transcribes an existing audio/video file; long files can take minutes. */
export function transcribeFile(path: string): Promise<TranscribeFileResult> {
  return invoke("transcribe_file", { path });
}

/** Writes text to `<source>.txt` next to the source file; returns the path written. */
export function saveTextFile(path: string, text: string): Promise<string> {
  return invoke("save_text_file", { path, text });
}

export type UpdateCheckResult = {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  releaseUrl: string;
};

export function checkForUpdate(): Promise<UpdateCheckResult> {
  return invoke("check_for_update");
}

/** Runs the notes-analysis prompt over a transcript via the configured
 * local LLM server; returns the transcript with its stored analysis. */
export function analyzeNote(transcriptId: string): Promise<Transcript> {
  return invoke("analyze_note", { transcriptId });
}

export type LlmStatus = {
  reachable: boolean;
  endpoint: string;
  models: string[];
  error: string | null;
};

/** Probes the local LLM (notes analysis) server. Pass an endpoint to test a
 * typed-but-unsaved value; omit it to use the saved one. A down server comes
 * back as `reachable: false`, not a rejection. */
export function llmStatus(endpoint?: string): Promise<LlmStatus> {
  return invoke("llm_status", { endpoint });
}

export type GoogleStatus = {
  configured: boolean;
  signedIn: boolean;
  email: string;
};

export type DriveSyncReport = {
  syncedNotes: number;
  filesWritten: number;
};

/** Reports whether this build is configured for Google Drive sync and the
 * current sign-in state. */
export function googleStatus(): Promise<GoogleStatus> {
  return invoke("google_status");
}

/** Opens a browser for Google OAuth; can take up to a minute while the user
 * consents. Returns the full updated settings with the account email filled in. */
export function googleSignIn(): Promise<AppSettings> {
  return invoke("google_sign_in");
}

/** Clears the stored Google account; returns the full updated settings
 * (email back to "", driveSyncEnabled false). */
export function googleSignOut(): Promise<AppSettings> {
  return invoke("google_sign_out");
}

/** Syncs notes (and optionally all transcripts) to Google Drive now. */
export function driveSyncNow(): Promise<DriveSyncReport> {
  return invoke("drive_sync_now");
}

/** Runs the end-of-day organize pass now for a local calendar day (today when
 * omitted). Resolves true when an organized file was written, false when the
 * day had no notes. Needs the local LLM (notes analysis) running. */
export function driveOrganizeNow(day?: string): Promise<boolean> {
  return invoke("drive_organize_now", { day });
}

/** Which transcripts a local export includes. */
export type ExportScope = "all" | "notes" | "dictation";
/** The file format a local export renders to. */
export type ExportFormat = "markdown" | "csv" | "json";

/** Exports transcripts (by scope) to a local file (in the chosen format) the
 * user picks via a native save dialog. Resolves to the saved absolute path, or
 * null when the user cancels. Purely local — no Google account required. */
export function exportTranscripts(
  scope: ExportScope,
  format: ExportFormat,
): Promise<string | null> {
  return invoke("export_transcripts", { scope, format });
}

export function openReleasePage(url?: string): Promise<void> {
  return invoke("open_release_page", { url });
}

export function getHotkeyStatus(): Promise<HotkeyStatus> {
  return invoke("get_hotkey_status");
}

export function rebindHotkey(
  action: HotkeyAction,
  shortcut: string,
): Promise<HotkeyStatus> {
  return invoke("rebind_hotkey", { action, shortcut });
}

/** Sets whether a single-shot bind acts on key press or release. Rejected by
 * the backend for Hold-to-Talk, which is push-to-talk. */
export function setHotkeyTrigger(
  action: HotkeyAction,
  trigger: TriggerEdge,
): Promise<HotkeyStatus> {
  return invoke("set_hotkey_trigger", { action, trigger });
}

export function resetHotkeysToDefaults(): Promise<HotkeyStatus> {
  return invoke("reset_hotkeys_to_defaults");
}

/** Switches the Dev flavor's hotkeys to the production (stable) defaults — for
 * running Scribe Dev alone with your real binds. */
export function loadProductionHotkeyDefaults(): Promise<HotkeyStatus> {
  return invoke("load_production_hotkey_defaults");
}

export function commandErrorMessage(error: unknown): string {
  if (error && typeof error === "object") {
    const commandError = error as CommandError;
    if (commandError.code) {
      const mapped = friendlyCommandErrorMessage(commandError.code);
      if (mapped) {
        return mapped;
      }
    }
    if (commandError.message) {
      return commandError.message;
    }
    if (commandError.code) {
      return commandError.code;
    }
  }

  return error instanceof Error ? error.message : String(error);
}

function friendlyCommandErrorMessage(code: string): string | null {
  const messages: Record<string, string> = {
    audio_platform_unsupported:
      "Audio capture is available in the Windows build. Use Windows 10 or 11 for V1 recording.",
    no_microphone_selected:
      "Choose a microphone before recording, or switch back to the default input device.",
    microphone_permission_denied:
      "Windows denied microphone access. Enable microphone permission for Scribe in Windows privacy settings.",
    microphone_unavailable:
      "The selected microphone is unavailable. Reconnect it or choose another input device.",
    recording_failed:
      "Recording failed. Check the selected microphone, then try again.",
    recording_not_active:
      "No recording is currently active. Start a new dictation first.",
    recording_not_transcribable:
      "This recording cannot be transcribed. Record again and let the capture finish normally.",
    audio_too_short:
      "The recording was too short. Hold the hotkey a little longer and try again.",
    whisper_model_missing:
      "Selected Whisper model is missing. Re-download the model or choose another downloaded model.",
    missing_whisper_executable:
      "The bundled whisper.cpp executable is missing. Install a build that includes resources/bin/windows/whisper-cli.exe.",
    whisper_transcription_failed:
      "Whisper transcription failed. Try again or choose another model.",
    empty_transcript:
      "Whisper returned an empty transcript. The previous Last Transcript Buffer was preserved.",
    model_download_failed:
      "Model download failed. Check your connection and retry the download.",
    model_checksum_mismatch:
      "The downloaded model failed verification. Delete it and retry the download.",
    model_download_in_progress:
      "That model is already downloading. Wait for it to finish or cancel it first.",
    autostart_failed:
      "Windows rejected the startup registration, so Launch at startup was not changed. Try again or check Windows startup settings.",
    hotkey_registration_failed:
      "Could not register the hotkey. Another app may be using it; choose a different shortcut.",
    paste_failed:
      "Paste failed. Focus the target app and try again, or switch paste method in Settings.",
    clipboard_restore_failed:
      "The transcript was pasted, but clipboard restore failed. Check your clipboard contents before continuing.",
    app_database_error:
      "Scribe could not access its local database. Restart the app; if it persists, inspect the app data folder.",
    app_data_dir_unavailable:
      "Scribe could not locate its app data folder. Check Windows profile permissions and restart.",
  };

  return messages[code] ?? null;
}
