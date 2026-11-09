import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  ENGINE_STATUS_EVENT,
  getEngineStatus,
  type EngineStatus,
} from "../backend";

/** "0.9 GB" from 900 MB up, "374 MB" below (matches the tray). */
export function formatMemory(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  if (mb >= 900) {
    return `${(mb / 1024).toFixed(1)} GB`;
  }
  return `${Math.round(mb)} MB`;
}

/** "1.4 s" from a load time in milliseconds. */
export function formatLoadSeconds(ms: number): string {
  return `${Math.max(0.1, ms / 1000).toFixed(1)} s`;
}

/** The short state after the model name: "Loaded · 0.9 GB",
 * "Not loaded, loads on next dictation", and so on. */
export function engineStateLabel(status: EngineStatus): string {
  switch (status.state) {
    case "loaded":
      return status.memoryBytes
        ? `Loaded · ${formatMemory(status.memoryBytes)}`
        : "Loaded";
    case "loading":
      return "Loading...";
    case "not_loaded":
      return "Not loaded, loads on next dictation";
    case "per_dictation":
      return "Loads for each dictation";
  }
}

/** The caveat next to "Keep the speech model loaded", with this machine's
 * own numbers once Scribe has measured them. */
export function keepLoadedCaveat(status: EngineStatus): string {
  if (status.state === "per_dictation") {
    return "On this system Whisper loads the model for each dictation, so it holds no memory between dictations.";
  }
  const memory = status.measured?.memoryBytes ?? status.memoryBytes;
  const loadMs = status.measured?.loadMs ?? null;
  const saves = memory
    ? `Saves about ${formatMemory(memory)} while you're not dictating.`
    : "Saves the model's memory while you're not dictating.";
  const slower =
    loadMs !== null
      ? `The first dictation after a pause can take up to ${formatLoadSeconds(loadMs)} longer.`
      : "The first dictation after a pause takes longer while the model loads.";
  return `${saves} ${slower}`;
}

/** The active speech engine's status, kept current from backend events. */
export function useEngineStatus(): EngineStatus | null {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<EngineStatus>(ENGINE_STATUS_EVENT, (event) => {
      setStatus(event.payload);
    }).then((stop) => {
      if (disposed) {
        stop();
      } else {
        unlisten = stop;
      }
    });
    getEngineStatus()
      .then((current) => {
        if (!disposed) {
          setStatus((known) => known ?? current);
        }
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
  return status;
}
