import type { AppSettings } from "./backend";

export type ViewName =
  | "Home"
  | "History"
  | "Transcribe a file"
  | "Dictation"
  | "Words"
  | "Microphone"
  | "Models & language"
  | "Hotkeys"
  | "Local AI"
  | "GitHub backup"
  | "Your data"
  | "Appearance"
  | "General"
  | "Developer"
  | "About";

export type LoadState = "loading" | "ready" | "error";

export type SettingsPatch = Partial<AppSettings>;

export type ViewActions = {
  cancelRecording: () => Promise<void>;
  clearLastTranscript: () => Promise<void>;
  clearingLastTranscript: boolean;
  copyLastTranscript: () => Promise<void>;
  copyingLastTranscript: boolean;
  recordingBusy: boolean;
  pasteLastTranscript: () => Promise<void>;
  pastingLastTranscript: boolean;
  openSettings: (tabId?: string) => void;
  refresh: () => Promise<void>;
  saveError: string | null;
  savingSettings: boolean;
  startRecording: () => Promise<void>;
  stopRecording: () => Promise<void>;
  updateSettings: (patch: SettingsPatch) => void;
};

export type ToastNotice = {
  id: number;
  tone: "info" | "success" | "error";
  message: string;
};
