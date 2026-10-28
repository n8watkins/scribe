import type { ViewName } from "../types";

/** One searchable setting: the name shown in results, the page that holds it,
 * and the words people might type for it. `target` is the `data-setting` value
 * of the row or panel to highlight (a SettingRow's label or a SectionPanel's
 * title); it defaults to `label`. */
export type SettingsEntry = {
  label: string;
  page: ViewName;
  target?: string;
  keywords?: string;
};

export const SETTINGS_INDEX: readonly SettingsEntry[] = [
  // Home
  { label: "Your stats", page: "Home", target: "Your stats", keywords: "statistics words per minute wpm count today total speed" },
  { label: "Last transcript", page: "Home", target: "Last transcript", keywords: "buffer paste again copy recent dictation" },

  // History
  { label: "Past dictations and notes", page: "History", target: "History", keywords: "history transcripts search notes archive combine" },

  // Transcribe a file
  { label: "Transcribe a file", page: "Transcribe a file", keywords: "audio video recording mp3 wav mp4 meeting import upload" },

  // Dictation
  { label: "After dictation", page: "Dictation", keywords: "paste insert type output auto-insert copy clipboard save cursor" },
  { label: "Insert method", page: "Dictation", keywords: "paste clipboard type out keystrokes restore terminal remote desktop" },
  { label: "Show floating pill", page: "Dictation", keywords: "overlay indicator status bubble widget" },
  { label: "Pill style", page: "Dictation", keywords: "pill display mode waveform visualizer dot live text" },

  // Words
  { label: "Context hint", page: "Words", keywords: "vocabulary names jargon spelling dictionary prompt custom words expect" },
  { label: "Replacements", page: "Words", keywords: "replace substitute autocorrect say get dictionary find fix" },
  { label: "Remove filler words", page: "Words", keywords: "um uh er hmm filler cleanup" },
  { label: "Filler words", page: "Words", keywords: "um uh er hmm list" },
  { label: "Pause before removing (ms)", page: "Words", keywords: "filler threshold milliseconds ms" },

  // Microphone
  { label: "Microphone", page: "Microphone", keywords: "mic input device headset level test record playback" },
  { label: "Silence trim", page: "Microphone", keywords: "quiet trim cut" },
  { label: "Auto-stop on silence", page: "Microphone", keywords: "hands-free stop pause end automatically" },
  { label: "Live transcription", page: "Microphone", keywords: "real-time streaming incremental background" },
  { label: "Auto-stop pause (ms)", page: "Microphone", keywords: "silence milliseconds ms timing advanced" },
  { label: "Split on pause", page: "Microphone", keywords: "phrase segment sentence timing advanced" },
  { label: "Pause before splitting (ms)", page: "Microphone", keywords: "phrase segment milliseconds ms timing advanced" },
  { label: "Longest phrase (ms)", page: "Microphone", keywords: "phrase segment maximum length milliseconds ms timing advanced" },
  { label: "Shortest recording (ms)", page: "Microphone", keywords: "minimum duration length too short milliseconds ms advanced" },
  { label: "Longest recording (ms)", page: "Microphone", keywords: "maximum duration length limit cap minutes milliseconds ms advanced" },

  // Models & language
  { label: "Models", page: "Models & language", keywords: "whisper download speech model accuracy speed size" },
  { label: "Language", page: "Models & language", keywords: "spoken multilingual english spanish french german auto-detect" },
  { label: "Translate to English", page: "Models & language", keywords: "translation multilingual" },
  { label: "Use GPU acceleration", page: "Models & language", keywords: "gpu graphics card vulkan speed fast cpu" },

  // Hotkeys
  { label: "Hotkeys", page: "Hotkeys", keywords: "shortcut shortcuts keyboard keys bindings rebind reset" },
  { label: "Hold to talk", page: "Hotkeys", keywords: "push-to-talk hotkey shortcut ptt" },
  { label: "Toggle dictation", page: "Hotkeys", keywords: "start stop hands-free hotkey shortcut backquote tilde" },
  { label: "Paste last transcript", page: "Hotkeys", keywords: "paste again hotkey shortcut buffer" },
  { label: "Open Scribe", page: "Hotkeys", keywords: "show window dashboard hotkey shortcut" },
  { label: "Discard recording", page: "Hotkeys", keywords: "cancel abort hotkey shortcut escape" },
  { label: "Transform selection hotkey", page: "Hotkeys", target: "Transform selection", keywords: "rewrite ai shortcut" },

  // Local AI
  { label: "Use local AI", page: "Local AI", keywords: "ai llm lm studio ollama on off enable notes analysis transform summary" },
  { label: "Server address", page: "Local AI", keywords: "endpoint url port lm studio ollama openai" },
  { label: "AI model", page: "Local AI", target: "Model", keywords: "llm local model name" },
  { label: "Analysis prompt", page: "Local AI", keywords: "notes summary action items instruction" },
  { label: "Transform selection", page: "Local AI", keywords: "rewrite selected text edit ai instruction grammar translate" },

  // GitHub backup
  { label: "GitHub account", page: "GitHub backup", target: "Connection", keywords: "sign in login connect disconnect sync backup" },
  { label: "Repository", page: "GitHub backup", keywords: "repo github backup sync" },
  { label: "Back up notes", page: "GitHub backup", keywords: "sync notes github backup" },
  { label: "Back up all transcripts", page: "GitHub backup", keywords: "sync history github backup" },

  // Your data
  { label: "Keep history", page: "Your data", keywords: "save history privacy transcripts off" },
  { label: "Transcript retention", page: "Your data", keywords: "delete keep days auto-delete cleanup privacy" },
  { label: "Notes retention", page: "Your data", keywords: "delete keep days auto-delete cleanup" },
  { label: "Save audio clips", page: "Your data", keywords: "raw audio recordings privacy keep playback wav" },
  { label: "Folders on this device", page: "Your data", target: "Local data", keywords: "folder location path logs models data directory" },
  { label: "Export to a file", page: "Your data", keywords: "export download markdown csv json backup" },
  { label: "Restore from a backup", page: "Your data", keywords: "import restore json backup" },
  { label: "Delete data", page: "Your data", keywords: "delete clear erase wipe remove history buffer" },

  // Appearance
  { label: "Color theme", page: "Appearance", keywords: "theme dark light colors midnight pewter custom look" },
  { label: "Pill colors", page: "Appearance", keywords: "floating pill color waveform note background" },
  { label: "Window size", page: "Appearance", keywords: "default size resize window" },

  // General
  { label: "Launch at startup", page: "General", keywords: "boot windows start login autostart" },
  { label: "Minimize to tray", page: "General", keywords: "tray system close background" },
  { label: "Open Scribe hotkey hides it", page: "General", keywords: "dashboard toggle minimize window hotkey shortcut" },
  { label: "Notifications", page: "General", keywords: "toasts alerts messages popups" },
  { label: "Sounds", page: "General", keywords: "tones beep audio cue chime" },
  { label: "Developer settings", page: "General", keywords: "debug diagnostics advanced" },

  // About
  { label: "Updates", page: "About", keywords: "update version install upgrade automatic check release" },
  { label: "Release notes", page: "About", keywords: "what's new whats new changelog changes version history" },
  { label: "Copy diagnostics", page: "About", target: "Diagnostics", keywords: "bug report support logs troubleshoot debug problem error" },
];

/** Lowercased words of a string, split on anything that isn't a letter or digit. */
function words(text: string): string[] {
  return text.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean);
}

/** Scores how well `query` matches `entry`, or returns 0 when it doesn't.
 * Every query word must start some word of the label, keywords or page name.
 * Matches on the label rank above matches through keywords alone. */
export function scoreEntry(entry: SettingsEntry, query: string): number {
  const queryWords = words(query);
  if (queryWords.length === 0) {
    return 0;
  }
  const labelWords = words(entry.label);
  const otherWords = [...words(entry.keywords ?? ""), ...words(entry.page)];
  let score = 0;
  for (const q of queryWords) {
    if (labelWords.some((w) => w === q)) {
      score += 30;
    } else if (labelWords.some((w) => w.startsWith(q))) {
      score += 20;
    } else if (otherWords.some((w) => w === q)) {
      score += 12;
    } else if (otherWords.some((w) => w.startsWith(q))) {
      score += 8;
    } else {
      return 0;
    }
  }
  const label = entry.label.toLowerCase();
  const phrase = query.trim().toLowerCase();
  if (label === phrase) {
    score += 50;
  } else if (label.startsWith(phrase)) {
    score += 25;
  }
  return score;
}

/** Settings matching `query`, best first; ties keep the index order, which
 * follows the sidebar. */
export function searchSettings(
  query: string,
  limit = 8,
  index: readonly SettingsEntry[] = SETTINGS_INDEX,
): SettingsEntry[] {
  return index
    .map((entry, position) => ({ entry, position, score: scoreEntry(entry, query) }))
    .filter((hit) => hit.score > 0)
    .sort((a, b) => b.score - a.score || a.position - b.position)
    .slice(0, limit)
    .map((hit) => hit.entry);
}
