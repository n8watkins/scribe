import { describe, expect, it } from "vitest";
import type { AppSettings } from "../backend";
import { changedSettings, settingsFor } from "./settingsPatch";

const current = {
  pillX: 10,
  pillY: 20,
  theme: "midnight",
  selectedMicId: "usb-mic",
  customTheme: { background: "#000000", surface: "#111111", accent: "#ff0000" },
  textReplacements: [{ from: "dot", to: "." }],
} as unknown as AppSettings;

describe("changedSettings", () => {
  it("keeps only the fields that differ", () => {
    expect(changedSettings(current, { pillX: 10, pillY: 30 })).toEqual({
      pillY: 30,
    });
  });

  it("returns nothing for a whole settings object that matches", () => {
    expect(changedSettings(current, { ...current })).toEqual({});
  });

  it("compares nested values by content", () => {
    expect(
      changedSettings(current, {
        customTheme: { ...current.customTheme },
        textReplacements: [{ from: "dot", to: "." }],
      } as Partial<AppSettings>),
    ).toEqual({});
    const customTheme = { ...current.customTheme, accent: "#00ff00" };
    expect(
      changedSettings(current, { customTheme } as Partial<AppSettings>),
    ).toEqual({ customTheme });
  });

  it("sends a cleared value as null", () => {
    expect(changedSettings(current, { selectedMicId: undefined })).toEqual({
      selectedMicId: null,
    });
    expect(changedSettings(current, { selectedMicId: null })).toEqual({
      selectedMicId: null,
    });
  });
});

describe("settingsFor", () => {
  it("picks the current values of the patched keys", () => {
    expect(settingsFor(current, { pillX: 99, theme: "pewter" })).toEqual({
      pillX: 10,
      theme: "midnight",
    });
  });
});
