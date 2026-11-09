import { describe, expect, it } from "vitest";
import type { EngineStatus } from "../backend";
import {
  engineStateLabel,
  formatMemory,
  keepLoadedCaveat,
} from "./engineStatus";

const MB = 1024 * 1024;

function status(overrides: Partial<EngineStatus> = {}): EngineStatus {
  return {
    engine: "whisper",
    modelId: "small.en-q5_1",
    modelName: "Whisper Small English Q5_1",
    state: "loaded",
    memoryBytes: 822 * MB,
    keepLoaded: true,
    measured: { loadMs: 1430, memoryBytes: 822 * MB },
    ...overrides,
  };
}

describe("engine status text", () => {
  it("formats memory like the tray", () => {
    expect(formatMemory(374 * MB)).toBe("374 MB");
    expect(formatMemory(922 * MB)).toBe("0.9 GB");
    expect(formatMemory(1700 * MB)).toBe("1.7 GB");
  });

  it("says whether the model is loaded", () => {
    expect(engineStateLabel(status())).toBe("Loaded · 822 MB");
    expect(engineStateLabel(status({ state: "not_loaded", memoryBytes: null }))).toBe(
      "Not loaded, loads on next dictation",
    );
    expect(engineStateLabel(status({ state: "loading" }))).toBe("Loading...");
  });

  it("puts this machine's numbers in the caveat", () => {
    expect(keepLoadedCaveat(status())).toBe(
      "Saves about 822 MB while you're not dictating. The first dictation after a pause can take up to 1.4 s longer.",
    );
  });

  it("explains itself before anything was measured", () => {
    expect(
      keepLoadedCaveat(status({ state: "not_loaded", memoryBytes: null, measured: null })),
    ).toBe(
      "Saves the model's memory while you're not dictating. The first dictation after a pause takes longer while the model loads.",
    );
  });
});
