import { describe, expect, it } from "vitest";
import {
  NAV_GROUPS,
  routeToView,
  settingsTabToView,
  visibleNavGroups,
} from "./navigation";

const labels = (developer: boolean) =>
  visibleNavGroups(developer).flatMap((group) =>
    group.items.map((item) => item.label),
  );

describe("sidebar", () => {
  it("groups the pages under Use, Set up, Integrations and App", () => {
    expect(NAV_GROUPS.map((group) => group.title)).toEqual([
      "Use",
      "Set up",
      "Integrations",
      "App",
    ]);
  });

  it("has no Stats, Notes, Settings or Record entries any more", () => {
    for (const gone of ["Stats", "Notes", "Settings", "Transcribe", "Dashboard"]) {
      expect(labels(false)).not.toContain(gone);
    }
  });

  it("adds Developer just before About only when developer settings are on", () => {
    expect(labels(false)).not.toContain("Developer");
    expect(labels(true).slice(-2)).toEqual(["Developer", "About"]);
    expect(labels(true)).toHaveLength(labels(false).length + 1);
  });
});

describe("routes from the tray and hotkeys", () => {
  it("maps current and older route names to pages", () => {
    expect(routeToView("dashboard")).toBe("Home");
    expect(routeToView("history")).toBe("History");
    expect(routeToView("notes")).toBe("History");
    expect(routeToView("stats")).toBe("Home");
    expect(routeToView(" Settings ")).toBe("Dictation");
    expect(routeToView("audio")).toBe("Microphone");
    expect(routeToView("privacy")).toBe("Your data");
    expect(routeToView("transcribe")).toBe("Transcribe a file");
  });

  it("ignores unknown routes", () => {
    expect(routeToView("nowhere")).toBeNull();
  });
});

describe("old Settings tab links", () => {
  it("land on the page that now holds those settings", () => {
    expect(settingsTabToView("notes")).toBe("Local AI");
    expect(settingsTabToView("dictionary")).toBe("Words");
    expect(settingsTabToView("history")).toBe("Your data");
    expect(settingsTabToView()).toBe("Dictation");
  });
});
