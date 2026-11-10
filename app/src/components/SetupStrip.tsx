import { useEffect, useState } from "react";
import {
  probeGpuDevices,
  type AppSettings,
  type GpuProbe,
} from "../backend";
import type { ViewActions, ViewName } from "../types";
import { HINT_CHAR_LIMIT } from "../lib/promptBudget";
import { isParakeetModel } from "../lib/engine";
import { useReplacements } from "../lib/useReplacements";

type Cell = {
  key: string;
  label: string;
  value: string;
  /** "warn": off or over, and changing it would help. "muted": not used. */
  tone?: "warn" | "muted";
  target: ViewName;
  title: string;
  /** A one-click fix shown under a warning. */
  fix?: { label: string; run: () => void };
};

/** Home: what's on and off for dictation, one full-width row. Each cell
 * opens its setting; a setting that would help but is off says so, with a
 * one-click fix. */
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
  const hintLength = settings.vocabularyPrompt.trim().length;
  const replacements = useReplacements().info?.rows.length ?? 0;
  const keepLoaded = parakeet
    ? settings.keepParakeetLoaded
    : settings.keepWhisperLoaded;
  const gpuName =
    gpu?.devices.find((device) => device.index === gpu.recommendedIndex)?.name ??
    gpu?.devices[0]?.name;
  const gpuOff = settings.gpuAcceleration === "off";

  const cells: Cell[] = [
    {
      key: "gpu",
      label: "GPU",
      value: gpuOff ? "Off" : "On",
      tone: gpuOff && gpu?.available ? "warn" : undefined,
      target: "Models & language",
      title:
        gpuOff && gpu?.available
          ? `${gpuName ?? "A graphics card"} is available and much faster for the bigger models.`
          : gpuOff
            ? "Transcribing on the processor."
            : "Transcribing on the graphics card when it can.",
      fix:
        gpuOff && gpu?.available
          ? {
              label: "Turn on",
              run: () => actions.updateSettings({ gpuAcceleration: "auto" }),
            }
          : undefined,
    },
    parakeet
      ? {
          key: "words",
          label: "Custom words",
          value: "Not used",
          tone: "muted",
          target: "Words",
          title: "Parakeet can't take a word list. Replacements still work.",
        }
      : {
          key: "words",
          label: "Custom words",
          value: hintLength ? `${hintLength} / ${HINT_CHAR_LIMIT}` : "None",
          tone: hintLength > HINT_CHAR_LIMIT ? "warn" : undefined,
          target: "Words",
          title:
            hintLength > HINT_CHAR_LIMIT
              ? "Too long: Whisper won't see the end."
              : "Names and jargon Whisper expects.",
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
        : "Saves memory; the first dictation after a pause is slower.",
    },
    {
      key: "live",
      label: "Live transcription",
      value: settings.incrementalTranscriptionEnabled ? "On" : "Off",
      tone: settings.incrementalTranscriptionEnabled ? undefined : "warn",
      target: "Microphone",
      title: "Transcribes while you talk, so text is ready when you stop.",
      fix: settings.incrementalTranscriptionEnabled
        ? undefined
        : {
            label: "Turn on",
            run: () =>
              actions.updateSettings({ incrementalTranscriptionEnabled: true }),
          },
    },
  ];

  return (
    <section aria-label="Dictation setup" className="setup-strip">
      {cells.map((cell) => (
        <div
          className={cell.tone ? `setup-cell is-${cell.tone}` : "setup-cell"}
          key={cell.key}
        >
          <button
            className="setup-cell-open"
            onClick={() => setActiveView(cell.target)}
            title={cell.title}
            type="button"
          >
            <span className="setup-cell-label">{cell.label}</span>
            <span className="setup-cell-value">{cell.value}</span>
          </button>
          {cell.fix ? (
            <button
              className="setup-cell-fix"
              disabled={actions.savingSettings}
              onClick={cell.fix.run}
              type="button"
            >
              {cell.fix.label}
            </button>
          ) : null}
        </div>
      ))}
    </section>
  );
}

/** Whether a usable GPU is present, asked only when GPU acceleration is off
 * (the backend caches its first probe). */
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
