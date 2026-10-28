import { describe, expect, it } from "vitest";
import {
  SETTINGS_INDEX,
  scoreEntry,
  searchSettings,
  type SettingsEntry,
} from "./settingsSearch";
import { NAV_GROUPS } from "./navigation";

const labels = (query: string) => searchSettings(query).map((e) => e.label);

describe("settings search index", () => {
  it("only points at pages that are in the sidebar", () => {
    const pages = new Set(
      NAV_GROUPS.flatMap((group) => group.items.map((item) => item.label)),
    );
    for (const entry of SETTINGS_INDEX) {
      expect(pages, entry.label).toContain(entry.page);
    }
  });

  it("has no two entries with the same name on the same page", () => {
    const keys = SETTINGS_INDEX.map((e) => `${e.page}/${e.label}`);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("never uses an em dash", () => {
    for (const entry of SETTINGS_INDEX) {
      expect(JSON.stringify(entry)).not.toContain(String.fromCharCode(0x2014));
    }
  });
});

describe("settings search matching", () => {
  it("finds a setting by its own name, ignoring case", () => {
    expect(labels("launch at startup")[0]).toBe("Launch at startup");
    expect(labels("LANGUAGE")[0]).toBe("Language");
  });

  it("matches the start of words, so partial typing works", () => {
    expect(labels("trans ret")).toEqual(["Transcript retention"]);
    expect(labels("gpu")).toContain("Use GPU acceleration");
  });

  it("finds settings by synonyms people type", () => {
    expect(labels("clipboard")).toEqual(
      expect.arrayContaining(["After dictation", "Insert method"]),
    );
    expect(labels("paste")).toEqual(
      expect.arrayContaining(["After dictation", "Paste last transcript"]),
    );
    expect(labels("retention")).toEqual([
      "Transcript retention",
      "Notes retention",
    ]);
    expect(labels("delete")).toEqual(
      expect.arrayContaining([
        "Delete data",
        "Transcript retention",
        "Notes retention",
      ]),
    );
    expect(labels("dark")).toContain("Color theme");
    expect(labels("ollama")).toContain("Use local AI");
  });

  it("treats hotkey and shortcut as the same thing", () => {
    for (const query of ["hotkey", "shortcut"]) {
      expect(labels(query)).toEqual(
        expect.arrayContaining(["Hold to talk", "Toggle dictation"]),
      );
    }
    expect(labels("shortcut")[0]).toBe("Hotkeys");
  });

  it("needs every word of the query to match", () => {
    expect(labels("retention zebra")).toEqual([]);
    expect(labels("notes retention")).toEqual(["Notes retention"]);
  });

  it("ranks a match on the name above a match on keywords only", () => {
    const named: SettingsEntry = { label: "Sounds", page: "General" };
    const viaKeyword: SettingsEntry = {
      label: "Notifications",
      page: "General",
      keywords: "sounds",
    };
    expect(scoreEntry(named, "sounds")).toBeGreaterThan(
      scoreEntry(viaKeyword, "sounds"),
    );
    expect(searchSettings("sounds", 8, [viaKeyword, named])[0]).toBe(named);
  });

  it("keeps sidebar order for equally good matches", () => {
    const first: SettingsEntry = { label: "A", page: "Home", keywords: "x" };
    const second: SettingsEntry = { label: "B", page: "Home", keywords: "x" };
    expect(searchSettings("x", 8, [first, second])).toEqual([first, second]);
  });

  it("returns nothing for an empty or punctuation-only query", () => {
    expect(searchSettings("")).toEqual([]);
    expect(searchSettings("   ")).toEqual([]);
    expect(searchSettings("--")).toEqual([]);
  });

  it("caps the number of results", () => {
    expect(searchSettings("e", 3)).toHaveLength(3);
  });
});
