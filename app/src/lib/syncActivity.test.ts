import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  backupCoverage,
  backupReadiness,
  clearSyncActivity,
  formatActivity,
  loadSyncActivity,
  rememberSyncError,
  rememberSyncSuccess,
} from "./syncActivity";

describe("sync activity", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.useRealTimers();
  });

  it("persists and reloads a successful manual backup", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-14T18:30:00.000Z"));

    const activity = rememberSyncSuccess({
      filesWritten: 2,
      syncedNotes: 5,
    });

    expect(activity).toEqual({
      completedAt: "2026-07-14T18:30:00.000Z",
      filesWritten: 2,
      itemCount: 5,
      outcome: "success",
      version: 1,
    });
    expect(loadSyncActivity()).toEqual(activity);
  });

  it("stores failure state without persisting a potentially sensitive error", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-14T18:30:00.000Z"));

    expect(rememberSyncError()).toEqual({
      completedAt: "2026-07-14T18:30:00.000Z",
      outcome: "error",
      version: 1,
    });
    expect(
      localStorage.getItem("scribe.github.last-manual-sync"),
    ).not.toContain("message");
  });

  it("rejects malformed stored data and clears it", () => {
    localStorage.setItem(
      "scribe.github.last-manual-sync",
      JSON.stringify({ completedAt: "never", outcome: "success", version: 1 }),
    );

    expect(loadSyncActivity()).toBeNull();
    expect(localStorage.getItem("scribe.github.last-manual-sync")).toBeNull();
  });

  it("clears a previous manual result", () => {
    rememberSyncError();
    clearSyncActivity();
    expect(loadSyncActivity()).toBeNull();
  });
});

describe("backup status labels", () => {
  const neither = {
    githubSyncAllTranscripts: false,
    githubSyncEnabled: false,
  };

  it("explains each readiness blocker in priority order", () => {
    expect(backupReadiness(neither, false)).toBe("Set a valid repository");
    expect(backupReadiness(neither, true)).toBe("Choose what to back up");
    expect(backupReadiness({ ...neither, githubSyncEnabled: true }, true)).toBe(
      "Connect GitHub",
    );
  });

  it("labels every backup coverage combination", () => {
    expect(backupCoverage(neither)).toBe("Nothing selected");
    expect(backupCoverage({ ...neither, githubSyncEnabled: true })).toBe(
      "Notes only",
    );
    expect(backupCoverage({ ...neither, githubSyncAllTranscripts: true })).toBe(
      "Dictations only",
    );
    expect(
      backupCoverage({
        githubSyncAllTranscripts: true,
        githubSyncEnabled: true,
      }),
    ).toBe("Notes and dictations");
  });

  it("formats empty, successful, and failed activity clearly", () => {
    expect(formatActivity(null)).toBe("Not run on this device");
    expect(
      formatActivity({
        completedAt: "2026-07-14T18:30:00.000Z",
        filesWritten: 1,
        itemCount: 2,
        outcome: "success",
        version: 1,
      }),
    ).toMatch(/^2 items, 1 file on /);
    expect(
      formatActivity({
        completedAt: "2026-07-14T18:30:00.000Z",
        outcome: "error",
        version: 1,
      }),
    ).toMatch(/^Failed /);
  });
});
