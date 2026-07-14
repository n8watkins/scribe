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

/** GPU (Vulkan) acceleration preference. "auto" uses the GPU when a usable
 * Vulkan device is present (with automatic CPU fallback); "off" forces CPU. */
export type GpuAcceleration = "auto" | "off";

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
   * Maps to a CSS palette via `data-theme`; "midnight" is the default look.
   * "custom" derives its palette from `customTheme` (see deriveCustomThemeVars). */
  theme: string;
  /** The five core colors of the user-defined "custom" theme. Only used when
   * `theme === "custom"`; the rest of the palette is derived from these. */
  customTheme: {
    background: string;
    surface: string;
    accent: string;
    text: string;
    textMuted: string;
  };
  devHotkeysSeeded: boolean;
  recordingMode: RecordingMode;
  minRecordingMs: number;
  maxRecordingMs: number;
  silenceTrimEnabled: boolean;
  silenceAutoStopEnabled: boolean;
  silenceAutoStopMs: number;
  incrementalTranscriptionEnabled: boolean;
  /** When live transcription is on, split a segment after this much silence (a
   * pause). Off = split only at the max length. */
  segmentPauseEnabled: boolean;
  /** Pause length (ms) that splits a segment. Higher = fewer sentence breaks at
   * brief pauses (less stray punctuation), slightly more latency. */
  segmentPauseMs: number;
  /** Max segment length (ms), 10000–25000. Bounded to Whisper's safe window so
   * audio is never truncated. */
  segmentMaxMs: number;
  selectedMicId: string | null;
  selectedModelId: string | null;
  language: Language;
  /** Run Whisper's translate task: emit English for any spoken language.
   * Requires a multilingual model. Defaults false. */
  translateToEnglish: boolean;
  /** GPU (Vulkan) acceleration: "auto" uses the GPU when present (CPU fallback),
   * "off" forces CPU. Defaults "auto". */
  gpuAcceleration: GpuAcceleration;
  /** Pinned Vulkan device index for multi-GPU machines; null = ggml's default
   * device (usually the discrete card). */
  gpuDeviceIndex: number | null;
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
  // FILLER: pause-aware filler suppression (deterministic, no LLM).
  fillerSuppressionEnabled: boolean;
  fillerWords: string[];
  fillerPauseThresholdMs: number;
  githubSyncEnabled: boolean;
  githubRepo: string;
  githubSyncAllTranscripts: boolean;
  githubAccountLogin: string;
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
  isTransform: boolean;
  // True when the recording ended because the mic was disconnected mid-capture
  // (salvaged + saved to History, never auto-pasted). Kept on the type so the
  // round-trip through transcribe_recording doesn't silently drop it.
  disconnected: boolean;
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

/** Opens the folder holding audio from dictations whose transcription failed,
 * so the user can recover the recording from disk. */
export function openFailedRecordingsFolder(): Promise<void> {
  return invoke("open_failed_recordings_folder");
}

/** The folder where audio is kept when a transcription fails (the `failed/`
 * quarantine subfolder of the app-data dir). */
export function getFailedRecordingsDir(): Promise<string> {
  return invoke("get_failed_recordings_dir");
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

/** A Vulkan GPU detected by whisper.cpp's ggml backend. */
export type VulkanDevice = {
  /** ggml device index — the value pinned via GGML_VK_VISIBLE_DEVICES. */
  index: number;
  name: string;
  /** Integrated GPU (shares system memory). Used to recommend the discrete card. */
  integrated: boolean;
};

export type GpuProbe = {
  /** At least one Vulkan device was detected. */
  available: boolean;
  devices: VulkanDevice[];
  /** The probe actually ran (model present + binary resolved). When false the UI
   * shows "couldn't detect" rather than "no GPU". */
  probed: boolean;
  /** Index the UI should suggest pinning (first discrete device), or null. */
  recommendedIndex: number | null;
};

/** Detects the Vulkan GPU(s) available for transcription. Best-effort: returns
 * an empty list (probed:false) on any failure or on non-Windows. */
export function probeGpuDevices(): Promise<GpuProbe> {
  return invoke("probe_gpu_devices");
}

export type GithubStatus = {
  /** Whether this build ships a GitHub OAuth client id (device flow configured). */
  configured: boolean;
  /** Whether an access token is stored in the keyring. */
  connected: boolean;
  /** GitHub login of the connected account, "" when not connected. */
  username: string;
  /** The configured target repo ("owner/name"), "" when unset. */
  repo: string;
};

/** The device-flow code to show the user. `deviceCode` + `intervalSecs` are
 * passed back into githubDevicePoll(). */
export type GithubDeviceCode = {
  deviceCode: string;
  userCode: string;
  verificationUri: string;
  expiresInSecs: number;
  intervalSecs: number;
};

export type GithubSyncReport = {
  syncedNotes: number;
  filesWritten: number;
};

export type GithubSyncActivity = {
  completedAt: string;
  outcome: "success" | "error";
  source: "manual" | "automatic";
  repo: string;
  syncedItems: number;
  filesWritten: number;
  errorCode: string | null;
  errorMessage: string | null;
};

/** Reports whether this build is configured for GitHub sync, the current
 * connection state, the connected username, and the configured repo. */
export function githubStatus(): Promise<GithubStatus> {
  return invoke("github_status");
}

/** Starts the OAuth device flow: opens the verification page and returns the
 * user code to show the user. Does not block on polling. */
export function githubDeviceStart(): Promise<GithubDeviceCode> {
  return invoke("github_device_start");
}

/** Polls until the user authorizes (or the code expires); stores the token in
 * the keyring and returns the updated settings. Long-running. */
export function githubDevicePoll(
  deviceCode: string,
  interval: number,
): Promise<AppSettings> {
  return invoke("github_device_poll", { deviceCode, interval });
}

/** Stops an active device-flow poll and prevents credential persistence. */
export function githubDeviceCancel(deviceCode: string): Promise<void> {
  return invoke("github_device_cancel", { deviceCode });
}

/** Clears the stored GitHub token; returns the updated settings
 * (githubSyncEnabled false). */
export function githubDisconnect(): Promise<AppSettings> {
  return invoke("github_disconnect");
}

/** Backs up notes (and optionally all transcripts) to the configured repo now. */
export function githubSyncNow(): Promise<GithubSyncReport> {
  return invoke("github_sync_now");
}

/** Returns the most recent persisted manual or automatic backup attempt. */
export function githubSyncActivity(): Promise<GithubSyncActivity | null> {
  return invoke("github_sync_activity");
}

/** Which transcripts a local export includes. */
export type ExportScope = "all" | "notes" | "dictation";
/** The file format a local export renders to. */
export type ExportFormat = "markdown" | "csv" | "json";

/** Exports transcripts (by scope) to a local file (in the chosen format) the
 * user picks via a native save dialog. Resolves to the saved absolute path, or
 * null when the user cancels. Purely local — no connected account required. */
export function exportTranscripts(
  scope: ExportScope,
  format: ExportFormat,
): Promise<string | null> {
  return invoke("export_transcripts", { scope, format });
}

export type TranscriptImportPreview = {
  path: string;
  fileName: string;
  total: number;
  notes: number;
  dictations: number;
  conflicts: number;
  audioPathsRemoved: number;
  metadataCorrected: number;
  fingerprint: string;
};

export type TranscriptImportReport = {
  imported: number;
  skipped: number;
  replaced: number;
};

/** Opens and validates a Scribe JSON export without changing local data. */
export function previewTranscriptImport(): Promise<TranscriptImportPreview | null> {
  return invoke("preview_transcript_import");
}

/** Revalidates and atomically restores a previously previewed JSON export. */
export function restoreTranscriptImport(
  path: string,
  replaceExisting: boolean,
  expectedFingerprint: string,
): Promise<TranscriptImportReport> {
  return invoke("restore_transcript_import", {
    path,
    replaceExisting,
    expectedFingerprint,
  });
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
