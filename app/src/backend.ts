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
export type Language = "auto" | "en";
export type OutputMode =
  | "save_only"
  | "auto_paste"
  | "copy_clipboard"
  | "copy_and_paste";
export type PasteMethod = "direct_insert" | "clipboard_restore";
export type HistoryRetentionDays = 7 | 30 | 90 | 365 | null;

export type HotkeySettings = {
  holdToTalk: string;
  toggleDictation: string;
  pasteLastTranscript: string;
  openDashboard: string;
};

export type AppSettings = {
  defaultsVersion: number;
  launchAtStartup: boolean;
  minimizeToTray: boolean;
  showFloatingPill: boolean;
  notificationsEnabled: boolean;
  soundsEnabled: boolean;
  recordingMode: RecordingMode;
  minRecordingMs: number;
  maxRecordingMs: number;
  silenceTrimEnabled: boolean;
  silenceAutoStopEnabled: boolean;
  silenceAutoStopMs: number;
  selectedMicId: string | null;
  selectedModelId: string | null;
  language: Language;
  vocabularyPrompt: string;
  outputMode: OutputMode;
  pasteMethod: PasteMethod;
  historyEnabled: boolean;
  saveAudioClips: boolean;
  historyRetentionDays: HistoryRetentionDays;
  hotkeys: HotkeySettings;
  pillX: number | null;
  pillY: number | null;
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
};

export type TranscriptSearchResult = {
  transcripts: Transcript[];
  total: number;
  limit: number;
  offset: number;
};

export type OutputAction =
  | "save_only"
  | "copy_clipboard"
  | "paste"
  | "copy_and_paste";

export type OutputStatus = "completed" | "clipboard_restore_failed";

export type ClipboardPreservation =
  | "not_needed"
  | "preserved"
  | "text_only_preserved"
  | "text_only_restore_failed"
  | "clipboard_owned_by_mode";

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
  | "openDashboard";

export type HotkeyBinding = {
  action: string;
  shortcut: string;
  normalizedShortcut: string | null;
  registered: boolean;
  error: string | null;
};

export type HotkeyStatus = {
  bindings: HotkeyBinding[];
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

export function listRecentTranscripts({
  limit,
}: {
  limit?: number;
} = {}): Promise<Transcript[]> {
  return invoke("list_recent_transcripts", { limit });
}

export function searchTranscripts({
  query,
  limit,
  offset,
}: {
  query?: string;
  limit?: number;
  offset?: number;
} = {}): Promise<TranscriptSearchResult> {
  return invoke("search_transcripts", { query, limit, offset });
}

export function getTranscript(id: string): Promise<Transcript | null> {
  return invoke("get_transcript", { id });
}

export function updateTranscript(id: string, text: string): Promise<Transcript> {
  return invoke("update_transcript", { id, text });
}

export function deleteTranscript(id: string): Promise<void> {
  return invoke("delete_transcript", { id });
}

export function clearTranscriptHistory(): Promise<void> {
  return invoke("clear_transcript_history");
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

export function transcribeRecording(
  recording: RecordingResult,
): Promise<DictationResult> {
  return invoke("transcribe_recording", { recording });
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

export function resetHotkeysToDefaults(): Promise<HotkeyStatus> {
  return invoke("reset_hotkeys_to_defaults");
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
      "Windows denied microphone access. Enable microphone permission for LocalDictate in Windows privacy settings.",
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
    hotkey_registration_failed:
      "Could not register the hotkey. Another app may be using it; choose a different shortcut.",
    paste_failed:
      "Paste failed. Focus the target app and try again, or switch paste method in Settings.",
    clipboard_restore_failed:
      "The transcript was pasted, but clipboard restore failed. Check your clipboard contents before continuing.",
    app_database_error:
      "LocalDictate could not access its local database. Restart the app; if it persists, inspect the app data folder.",
    app_data_dir_unavailable:
      "LocalDictate could not locate its app data folder. Check Windows profile permissions and restart.",
  };

  return messages[code] ?? null;
}
