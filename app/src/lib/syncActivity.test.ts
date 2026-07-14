import { describe, expect, it } from "vitest";
import {
  backupCoverage,
  backupReadiness,
  formatActivity,
  syncActivityError,
} from "./syncActivity";

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

  it("formats empty and successful persisted activity", () => {
    expect(formatActivity(null, "alice/notes")).toBe(
      "Not run for this repository",
    );
    expect(
      formatActivity(
        {
          completedAt: "2026-07-14T18:30:00.000Z",
          errorCode: null,
          errorMessage: null,
          filesWritten: 1,
          outcome: "success",
          repo: "alice/notes",
          source: "automatic",
          syncedItems: 2,
        },
        "alice/notes",
      ),
    ).toMatch(/^Automatic: 2 items, 1 file on /);
  });

  it("shows a persisted error only for the active repository", () => {
    const activity = {
      completedAt: "2026-07-14T18:30:00.000Z",
      errorCode: "github_sync_failed",
      errorMessage: "Network unavailable",
      filesWritten: 1,
      outcome: "error" as const,
      repo: "alice/notes",
      source: "manual" as const,
      syncedItems: 2,
    };

    expect(formatActivity(activity, "alice/notes")).toMatch(
      /^Manual backup failed /,
    );
    expect(syncActivityError(activity, "alice/notes")).toBe(
      "Network unavailable",
    );
    expect(syncActivityError(activity, "alice/other")).toBeNull();
  });
});
