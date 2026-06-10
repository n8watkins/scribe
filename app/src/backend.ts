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
  launchAtStartup: boolean;
  minimizeToTray: boolean;
  showFloatingPill: boolean;
  notificationsEnabled: boolean;
  soundsEnabled: boolean;
  recordingMode: RecordingMode;
  minRecordingMs: number;
  maxRecordingMs: number;
  silenceTrimEnabled: boolean;
  selectedMicId: string | null;
  selectedModelId: string | null;
  language: Language;
  outputMode: OutputMode;
  pasteMethod: PasteMethod;
  historyEnabled: boolean;
  saveAudioClips: boolean;
  historyRetentionDays: HistoryRetentionDays;
  hotkeys: HotkeySettings;
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

export type BasicStats = {
  wordsToday: number;
  dictationsToday: number;
  averageWpm: number | null;
  averageTranscriptionLatencyMs: number | null;
  averageRecordingDurationMs: number | null;
  mostUsedModel: string | null;
  totalWordsTranscribed: number;
};

export type CommandError = {
  code?: string;
  message?: string;
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

export function listRecentTranscripts({
  limit,
}: {
  limit?: number;
} = {}): Promise<Transcript[]> {
  return invoke("list_recent_transcripts", { limit });
}

export function getBasicStats(): Promise<BasicStats> {
  return invoke("get_basic_stats");
}

export function commandErrorMessage(error: unknown): string {
  if (error && typeof error === "object") {
    const commandError = error as CommandError;
    if (commandError.message) {
      return commandError.message;
    }
    if (commandError.code) {
      return commandError.code;
    }
  }

  return error instanceof Error ? error.message : String(error);
}
