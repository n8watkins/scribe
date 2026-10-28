import type {
  AppSettings,
  AppStateSnapshot,
  DashboardData,
  HistoryRetentionDays,
  MicrophoneInfo,
  ModelDownloadProgress,
  ModelInfo,
  OutputMode,
  OutputResult,
  Transcript,
} from "../backend";

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

// The four `outputMode` values are presented as one "After dictation" choice
// (insert at the cursor, copy only, or save only) plus, for inserting, an
// "Insert method". These helpers map between the two representations without
// changing what any saved value means.
export type AfterDictation = "insert" | "copy" | "save";

// How an inserted dictation reaches the app:
// - paste_restore: borrow the clipboard for one Ctrl+V, then restore it
//   (`auto_paste` + `clipboard_paste`, the default).
// - paste_keep: paste and leave the transcript on the clipboard
//   (`copy_and_paste`).
// - type: type the text out as keystrokes for apps that block paste
//   (`auto_paste` + `direct_insert`).
export type InsertMethod = "paste_restore" | "paste_keep" | "type";

export function isAutoInsert(mode: OutputMode): boolean {
  return mode === "auto_paste" || mode === "copy_and_paste";
}

export function isKeepClipboard(mode: OutputMode): boolean {
  return mode === "auto_paste" || mode === "save_only";
}

export function afterDictationOf(mode: OutputMode): AfterDictation {
  if (isAutoInsert(mode)) {
    return "insert";
  }
  return mode === "copy_clipboard" ? "copy" : "save";
}

export function insertMethodOf(
  settings: Pick<AppSettings, "outputMode" | "pasteMethod">,
): InsertMethod {
  // Copy + paste always pastes through the clipboard (the backend ignores the
  // paste method there), so it reads as "paste and keep" either way.
  if (settings.outputMode === "copy_and_paste") {
    return "paste_keep";
  }
  return settings.pasteMethod === "direct_insert" ? "type" : "paste_restore";
}

// The settings patch for a new choice. Copy only and Save only leave the
// paste method alone, because Paste Last and History's Insert still use it.
export function outputSettingsFor(
  after: AfterDictation,
  method: InsertMethod,
): Partial<Pick<AppSettings, "outputMode" | "pasteMethod">> {
  if (after === "copy") {
    return { outputMode: "copy_clipboard" };
  }
  if (after === "save") {
    return { outputMode: "save_only" };
  }
  switch (method) {
    case "paste_keep":
      return { outputMode: "copy_and_paste", pasteMethod: "clipboard_paste" };
    case "type":
      return { outputMode: "auto_paste", pasteMethod: "direct_insert" };
    default:
      return { outputMode: "auto_paste", pasteMethod: "clipboard_paste" };
  }
}

// Short, honest label for what the last dictation did to the clipboard,
// driven by the output mode and, when inserting, the insert method.
export function clipboardStatus(settings: AppSettings) {
  switch (settings.outputMode) {
    case "save_only":
      return "Clipboard untouched";
    case "copy_clipboard":
    case "copy_and_paste":
      return "On your clipboard";
    default:
      return settings.pasteMethod === "direct_insert"
        ? "Types it out"
        : "Clipboard restored";
  }
}

// Short, honest label for what one paste or copy actually did to the
// clipboard. Unlike clipboardStatus, this reflects the outcome: a paste that
// was blocked (switched windows, admin target) leaves the text on the clipboard.
export function clipboardOutcome(result: OutputResult) {
  switch (result.clipboardPreservation) {
    case "restored_after_paste":
      return "Clipboard restored";
    case "untouched":
      return result.pasted ? "Typed out" : "Clipboard untouched";
    default:
      return "On your clipboard";
  }
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
    "-"
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
    return "-";
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

/** What each hotkey action is called everywhere it is shown. */
export const HOTKEY_LABELS = {
  holdToTalk: "Hold to talk",
  toggleDictation: "Toggle dictation",
  pasteLastTranscript: "Paste last transcript",
  openDashboard: "Open Scribe",
  discardDictation: "Discard recording",
  transformSelection: "Transform selection",
} as const;

export type HotkeyAction = keyof typeof HOTKEY_LABELS;

export function hotkeyRows(settings: AppSettings) {
  return (Object.keys(HOTKEY_LABELS) as HotkeyAction[]).map((action) => ({
    action,
    label: HOTKEY_LABELS[action],
    value: settings.hotkeys[action],
  }));
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
