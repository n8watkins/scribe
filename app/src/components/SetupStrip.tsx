import { useEffect, useState } from "react";
import {
  probeGpuDevices,
  type AppSettings,
  type GpuProbe,
} from "../backend";
import type { ViewActions, ViewName } from "../types";
import { analyzeHint, HINT_TOKEN_BUDGET } from "../lib/promptBudget";
import { isParakeetModel } from "../lib/engine";

type Chip = {
  key: string;
  label: string;
  value: string;
  /** "warn": off or over, and turning it on would help. "muted": not used. */
  tone?: "warn" | "muted";
  target: ViewName;
  title: string;
  /** A one-click fix shown next to a warning. */
  fix?: { label: string; run: () => void };
};

/** Home: what's on and off for dictation, at a glance. Each chip opens its
 * setting; a setting that would help but is off says so, with a fix. */
export function SetupStrip({
  actions,
  settings,
  setActiveView,
}: {
  actions: ViewActions;
  settings: AppSettings;
  setActiveView: (view: ViewName) => void;
}) {
  const gpu = useGpuProbe(settings.gpuAcceleration === "off");
  const parakeet = isParakeetModel(settings.selectedModelId);
  const hint = analyzeHint(settings.vocabularyPrompt);
  const replacements = settings.textReplacements.filter((row) =>
    row.from.trim(),
  ).length;
  const keepLoaded = parakeet
    ? settings.keepParakeetLoaded
    : settings.keepWhisperLoaded;
  const gpuName =
    gpu?.devices.find((device) => device.index === gpu.recommendedIndex)?.name ??
    gpu?.devices[0]?.name;

  const chips: Chip[] = [
    settings.gpuAcceleration === "off"
      ? gpu?.available
        ? {
            key: "gpu",
            label: "GPU",
            value: "Off",
            tone: "warn",
            target: "Models & language",
            title: `${gpuName ?? "A graphics card"} is available. Using it makes the bigger models fast enough for live dictation.`,
            fix: {
              label: "Turn on",
              run: () => actions.updateSettings({ gpuAcceleration: "auto" }),
            },
          }
        : {
            key: "gpu",
            label: "GPU",
            value: "Off",
            target: "Models & language",
            title: "Transcribing on the processor.",
          }
      : {
          key: "gpu",
          label: "GPU",
          value: "On",
          target: "Models & language",
          title: "Transcribing on the graphics card when it can.",
        },
    parakeet
      ? {
          key: "words",
          label: "Custom words",
          value: "Not used by Parakeet",
          tone: "muted",
          target: "Words",
          title: "Parakeet can't take a word list. Replacements still work.",
        }
      : {
          key: "words",
          label: "Custom words",
          value: settings.vocabularyPrompt.trim()
            ? hint.overLimit
              ? `${hint.terms} terms, too long`
              : `${hint.terms} terms, ${Math.round((hint.tokens / HINT_TOKEN_BUDGET) * 100)}% full`
            : "None",
          tone: hint.overLimit ? "warn" : undefined,
          target: "Words",
          title: hint.overLimit
            ? `Too long: Whisper never sees the last ${hint.dropped.length} terms.`
            : "Names and jargon Whisper expects. Room for about 35 to 40.",
        },
    {
      key: "replacements",
      label: "Replacements",
      value: String(replacements),
      target: "Words",
      title: "Fixes applied to every transcript, with any model.",
    },
    {
      key: "loaded",
      label: "Model kept loaded",
      value: keepLoaded ? "On" : "Off",
      target: "Models & language",
      title: keepLoaded
        ? "Dictation starts at once."
        : "Saves memory between dictations; the first one after a pause is slower.",
    },
    settings.incrementalTranscriptionEnabled
      ? {
          key: "live",
          label: "Live transcription",
          value: "On",
          target: "Microphone",
          title: "Transcribes while you talk, so text is ready when you stop.",
        }
      : {
          key: "live",
          label: "Live transcription",
          value: "Off",
          tone: "warn",
          target: "Microphone",
          title: "Turn it on and long dictations are ready sooner.",
          fix: {
            label: "Turn on",
            run: () =>
              actions.updateSettings({ incrementalTranscriptionEnabled: true }),
          },
        },
  ];

  return (
    <section aria-label="Dictation setup" className="setup-strip">
      {chips.map((chip) => (
        <span
          className={chip.tone ? `setup-chip is-${chip.tone}` : "setup-chip"}
          key={chip.key}
        >
          <button
            className="setup-chip-open"
            onClick={() => setActiveView(chip.target)}
            title={chip.title}
            type="button"
          >
            <span className="setup-chip-label">{chip.label}</span>
            <span className="setup-chip-value">{chip.value}</span>
          </button>
          {chip.fix ? (
            <button
              className="setup-chip-fix"
              disabled={actions.savingSettings}
              onClick={chip.fix.run}
              type="button"
            >
              {chip.fix.label}
            </button>
          ) : null}
        </span>
      ))}
    </section>
  );
}

/** Whether a usable GPU is present, asked only when GPU acceleration is off
 * (the answer is cached by the backend after its first probe). */
function useGpuProbe(wanted: boolean): GpuProbe | null {
  const [probe, setProbe] = useState<GpuProbe | null>(null);
  useEffect(() => {
    if (!wanted) {
      return;
    }
    let cancelled = false;
    probeGpuDevices()
      .then((result) => {
        if (!cancelled) {
          setProbe(result);
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [wanted]);
  return probe;
}
