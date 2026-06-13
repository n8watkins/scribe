import type { AppSettings } from "./backend";

export type ViewName =
  | "Dashboard"
  | "Transcribe"
  | "History"
  | "Notes"
  | "Stats"
  | "Settings"
  | "Data & Privacy"
  | "Hotkeys"
  | "Models"
  | "Audio"
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
