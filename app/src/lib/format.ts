import type {
  AppSettings,
  AppStateSnapshot,
  DashboardData,
  HistoryRetentionDays,
  MicrophoneInfo,
  ModelDownloadProgress,
  ModelInfo,
  OutputMode,
  Transcript,
} from "../backend";
import type { ViewName } from "../types";

/// Renders a millisecond value as a human-readable duration, e.g.
/// 300 -> "0.3 s", 5000 -> "5 s", 600000 -> "10 min".
export function formatMsReadable(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) {
    return "0 s";
  }
  const totalSeconds = ms / 1000;
  if (totalSeconds < 60) {
    return `${Number(totalSeconds.toFixed(1))} s`;
  }
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.round(totalSeconds - minutes * 60);
  return seconds > 0 ? `${minutes} min ${seconds} s` : `${minutes} min`;
}

// The four `outputMode` values are presented to the owner as two independent
// switches: "Auto-insert after dictation" (paste for you vs. buffer only) and
// "Keep my clipboard" (borrow-and-restore vs. leave the transcript on the
// clipboard). These helpers map between the two representations.
export function isAutoInsert(mode: OutputMode): boolean {
  return mode === "auto_paste" || mode === "copy_and_paste";
}

export function isKeepClipboard(mode: OutputMode): boolean {
  return mode === "auto_paste" || mode === "save_only";
}

export function outputModeFromToggles(
  autoInsert: boolean,
  keepClipboard: boolean,
): OutputMode {
  if (autoInsert) {
    return keepClipboard ? "auto_paste" : "copy_and_paste";
  }
  return keepClipboard ? "save_only" : "copy_clipboard";
}

// Short, honest label driven by the actual paste method and output mode.
// `direct_insert` types the transcript out as keystrokes; otherwise, when the
// owner keeps their clipboard, Scribe momentarily borrows it for one Ctrl+V and
// then restores the previous contents — and when they don't, it simply leaves
// the transcript on the clipboard for them to paste.
export function clipboardStatus(settings: AppSettings) {
  if (settings.pasteMethod === "direct_insert") {
    return "Types it out";
  }
  return isKeepClipboard(settings.outputMode)
    ? "Clipboard restored"
    : "On your clipboard";
}

export function routeToView(route: string): ViewName | null {
  const normalized = route.trim().toLowerCase();
  const routes: Record<string, ViewName> = {
    dashboard: "Dashboard",
    transcribe: "Transcribe",
    history: "History",
    stats: "Stats",
    settings: "Settings",
    data: "Data & Privacy",
    privacy: "Data & Privacy",
    hotkeys: "Hotkeys",
    models: "Models",
    audio: "Audio",
    notes: "Notes",
    about: "About",
  };

  return routes[normalized] ?? null;
}

export function canStartRecording(data: DashboardData | null) {
  if (!data) {
    return false;
  }

  return (
    data.appState.status === "Idle" ||
    data.appState.status === "Ready" ||
    data.appState.status === "Error"
  );
}

export function stateTone(status: AppStateSnapshot["status"]) {
  switch (status) {
    case "Recording":
      return "recording";
    case "Stopping":
    case "Transcribing":
    case "Pasting":
      return "pending";
    case "Ready":
      return "ready";
    case "Error":
      return "error";
    case "Paused":
      return "preserve";
    case "Idle":
    default:
      return "idle";
  }
}

export function isSelectedModelReady(
  models: ModelInfo[] | null,
  selectedModelId: string | null,
) {
  if (!selectedModelId) {
    return false;
  }

  if (!models) {
    // Model list is still loading or unavailable; assume the selected
    // model works instead of flashing a false call-to-action.
    return true;
  }

  const selected = models.find(
    (model) => model.id === selectedModelId || model.selected,
  );
  if (!selected) {
    return false;
  }

  return isModelDownloaded(selected.status);
}

// A model is considered downloaded (present on disk and usable) for these
// download-states. `selected`/`loaded` are runtime overlays the backend may
// report on top of a downloaded file. Selection itself is the `model.selected`
// boolean — kept separate from this download-state check on purpose.
export function isModelDownloaded(status: ModelInfo["status"]) {
  return (
    status === "downloaded" ||
    status === "selected" ||
    status === "loaded" ||
    status === "update_available"
  );
}

// Label for the *download state* only (never "Selected"; selection is shown by
// its own control). "not_downloaded" returns "" so the resting state of most
// models stays quiet rather than rendering a loud badge.
export function modelStatusLabel(status: ModelInfo["status"]) {
  const labels: Record<ModelInfo["status"], string> = {
    not_downloaded: "",
    downloading: "Downloading",
    downloaded: "Downloaded",
    selected: "Downloaded",
    loaded: "Loaded",
    failed: "Failed",
    update_available: "Update available",
  };

  return labels[status];
}

// Pill class for the *download state* only. "not_downloaded" gets no pill
// (returns ""); callers render it as plain muted text instead of a badge.
export function modelStatusClass(status: ModelInfo["status"]) {
  if (status === "failed") {
    return "pill error";
  }

  if (status === "downloading") {
    return "pill pending";
  }

  if (status === "update_available") {
    return "pill pending";
  }

  if (status === "loaded") {
    return "pill selected";
  }

  if (status === "downloaded" || status === "selected") {
    return "pill ready";
  }

  return "";
}

// Human-readable total of a byte count, mirroring the backend's MiB/GiB
// (base-1024) catalog labels so the summary header matches per-model sizes.
export function diskUsedLabel(totalBytes: number) {
  if (!Number.isFinite(totalBytes) || totalBytes <= 0) {
    return "0 MiB";
  }

  const gib = totalBytes / 1024 ** 3;
  if (gib >= 1) {
    return `${Number(gib.toFixed(1))} GiB`;
  }

  const mib = totalBytes / 1024 ** 2;
  return `${Math.max(1, Math.round(mib))} MiB`;
}

export function progressPercent(
  model: ModelInfo,
  progress: ModelDownloadProgress | undefined,
) {
  if (progress?.percent !== null && progress?.percent !== undefined) {
    return Math.max(0, Math.min(100, progress.percent));
  }

  if (isModelDownloaded(model.status)) {
    return 100;
  }

  return 0;
}

export function selectedMicrophoneLabel(
  microphones: MicrophoneInfo[],
  selectedMicId: string | null,
) {
  if (!selectedMicId) {
    return (
      microphones.find((microphone) => microphone.isDefault)?.name ??
      "Default input device"
    );
  }

  // Never render raw device IDs; show a neutral placeholder until resolved.
  return (
    microphones.find((microphone) => microphone.id === selectedMicId)?.name ??
    "—"
  );
}

/** Windows wraps capture devices as "Microphone (Realtek(R) Audio)"; show the
 * inner device name so tiles read "Realtek(R) Audio", not the boilerplate. */
export function cleanMicName(name: string): string {
  const trimmed = name.trim();
  const wrapped = /^microphone\s*\((.+)\)\s*$/i.exec(trimmed);
  return wrapped ? wrapped[1].trim() : trimmed;
}

export function microphoneDisplayName(
  microphones: MicrophoneInfo[] | null,
  selectedMicId: string | null,
) {
  // The list is still loading; show a neutral placeholder rather than a
  // generic "Default input device" that might be wrong.
  if (!microphones) {
    return "—";
  }

  // No explicit selection: resolve the actual default device's real name so
  // the tile shows the device the user will record with, not a generic label.
  if (!selectedMicId) {
    return cleanMicName(
      microphones.find((microphone) => microphone.isDefault)?.name ??
        "Default input device",
    );
  }

  return cleanMicName(selectedMicrophoneLabel(microphones, selectedMicId));
}

// Dashboard "Current status" tile value. The StatePill badge already shows the
// raw status word, so this gives a complementary phrase that is never identical
// to it (avoids the old "Recording / Recording" duplication).
export function statusCardValue(status: AppStateSnapshot["status"]) {
  switch (status) {
    case "Recording":
      return "Capturing audio";
    case "Stopping":
      return "Finishing capture";
    case "Transcribing":
      return "Transcribing locally";
    case "Pasting":
      return "Inserting transcript";
    case "Ready":
      return "Transcript ready";
    case "Error":
      return "Needs attention";
    case "Paused":
      return "Resume to record";
    case "Idle":
    default:
      return "Ready for dictation";
  }
}

export function recordingStageTitle(status: AppStateSnapshot["status"]) {
  switch (status) {
    case "Recording":
      return "Recording";
    case "Stopping":
      return "Stopping recording";
    case "Transcribing":
      return "Transcribing locally";
    case "Pasting":
      return "Inserting transcript";
    case "Ready":
      return "Transcript ready";
    case "Error":
      return "Needs attention";
    case "Paused":
      return "Paused";
    case "Idle":
    default:
      return "Ready for dictation";
  }
}

export function hotkeyRows(settings: AppSettings) {
  return [
    {
      action: "holdToTalk",
      label: "Hold-to-Talk",
      value: settings.hotkeys.holdToTalk,
    },
    {
      action: "toggleDictation",
      label: "Toggle Dictation",
      value: settings.hotkeys.toggleDictation,
    },
    {
      action: "pasteLastTranscript",
      label: "Paste Last",
      value: settings.hotkeys.pasteLastTranscript,
    },
    {
      action: "openDashboard",
      label: "Open Dashboard",
      value: settings.hotkeys.openDashboard,
    },
  ];
}

export function retentionToValue(retention: HistoryRetentionDays) {
  return retention === null ? "forever" : String(retention);
}

export function retentionFromValue(value: string): HistoryRetentionDays {
  if (value === "forever") {
    return null;
  }

  const numeric = Number(value);
  return numeric === 7 || numeric === 30 || numeric === 90 || numeric === 365
    ? numeric
    : 30;
}

const hotkeyDisplayAliases: Record<string, string> = {
  Backquote: "~ (tilde)",
};

export function formatHotkey(value: string) {
  return value
    .split("+")
    .map((part) => hotkeyDisplayAliases[part] ?? part)
    .join(" + ");
}

export function formatDateTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "Unknown time";
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

export function formatDuration(milliseconds: number | null) {
  if (milliseconds === null) {
    return "No audio duration";
  }

  if (milliseconds < 1000) {
    return `${milliseconds} ms audio`;
  }

  return `${(milliseconds / 1000).toFixed(1)}s audio`;
}

export function formatOptionalDuration(milliseconds: number | null) {
  if (milliseconds === null) {
    return "None";
  }

  if (milliseconds < 1000) {
    return `${Math.round(milliseconds)} ms`;
  }

  return `${(milliseconds / 1000).toFixed(1)}s`;
}

export function formatOptionalNumber(value: number | null) {
  return value === null ? "None" : Math.round(value).toLocaleString();
}

export function formatNumber(value: number) {
  return value.toLocaleString();
}

export function formatCount(count: number, unit: string) {
  return `${formatNumber(count)} ${count === 1 ? unit : `${unit}s`}`;
}

export function transcriptMeta(transcript: Transcript) {
  const parts = [
    formatCount(transcript.wordCount, "word"),
    transcript.modelId ?? "No model",
    formatDuration(transcript.durationMs),
  ];
  if (transcript.isNote) {
    parts.unshift("Note");
  }
  return parts.join(" | ");
}
