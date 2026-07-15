import "@testing-library/jest-dom/vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../backend";
import type { ViewActions } from "../types";

const {
  githubStatusMock,
  githubSyncActivityMock,
  githubSyncNowMock,
  previewTranscriptImportMock,
  restoreTranscriptImportMock,
} = vi.hoisted(() => ({
  githubStatusMock: vi.fn(),
  githubSyncActivityMock: vi.fn(),
  githubSyncNowMock: vi.fn(),
  previewTranscriptImportMock: vi.fn(),
  restoreTranscriptImportMock: vi.fn(),
}));

vi.mock("../backend", async (importOriginal) => {
  const original = await importOriginal<typeof import("../backend")>();
  return {
    ...original,
    exportTranscripts: vi.fn(),
    githubDeviceCancel: vi.fn(),
    githubDevicePoll: vi.fn(),
    githubDeviceStart: vi.fn(),
    githubDisconnect: vi.fn(),
    githubStatus: githubStatusMock,
    githubSyncActivity: githubSyncActivityMock,
    githubSyncNow: githubSyncNowMock,
    previewTranscriptImport: previewTranscriptImportMock,
    restoreTranscriptImport: restoreTranscriptImportMock,
  };
});

import { SyncView } from "./Sync";

const settings = {
  githubAccountLogin: "alice",
  githubRepo: "alice/scribe-notes",
  githubSyncAllTranscripts: true,
  githubSyncEnabled: true,
} as AppSettings;

const refreshMock = vi.fn();
const actions = {
  refresh: refreshMock,
  savingSettings: false,
  updateSettings: vi.fn(),
} as unknown as ViewActions;

afterEach(cleanup);

describe("SyncView backup status", () => {
  beforeEach(() => {
    githubStatusMock.mockReset();
    githubStatusMock.mockResolvedValue({
      configured: true,
      connected: true,
      repo: settings.githubRepo,
      username: "alice",
    });
    githubSyncActivityMock.mockReset();
    githubSyncActivityMock.mockResolvedValue(null);
    githubSyncNowMock.mockReset();
    previewTranscriptImportMock.mockReset();
    restoreTranscriptImportMock.mockReset();
    refreshMock.mockReset();
  });

  it("shows readiness, coverage, and empty backup history", async () => {
    render(<SyncView actions={actions} settings={settings} />);

    expect(await screen.findByText("Ready to back up")).toBeInTheDocument();
    expect(
      screen.getByText(/existing private repo installed for Scribe Local Backup/),
    ).toBeInTheDocument();
    expect(screen.getByText("Notes and dictations")).toBeInTheDocument();
    expect(
      screen.getByText("Not run for this repository"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sync now" })).toBeEnabled();
  });

  it("explains that Scribe writes only to an installed private repository", async () => {
    githubStatusMock.mockResolvedValue({
      configured: true,
      connected: false,
      repo: null,
      username: null,
    });

    render(<SyncView actions={actions} settings={settings} />);

    expect(
      await screen.findByText(/access to the private repositories where you install Scribe/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/only writes to the private backup repository you select/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/create and write/)).not.toBeInTheDocument();
  });

  it("loads the persisted manual result after a sync and view remount", async () => {
    githubSyncNowMock.mockResolvedValue({ filesWritten: 2, syncedNotes: 3 });
    githubSyncActivityMock
      .mockResolvedValueOnce(null)
      .mockResolvedValue({
        completedAt: "2026-07-14T18:30:00.000Z",
        errorCode: null,
        errorMessage: null,
        filesWritten: 2,
        outcome: "success",
        repo: settings.githubRepo,
        source: "manual",
        syncedItems: 3,
      });
    const first = render(<SyncView actions={actions} settings={settings} />);
    await screen.findByText("Ready to back up");

    fireEvent.click(screen.getByRole("button", { name: "Sync now" }));
    expect(
      await screen.findByText("Backed up 3 items into 2 files."),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(
        screen.getByText(/^Manual: 3 items, 2 files on /),
      ).toBeInTheDocument(),
    );

    first.unmount();
    render(<SyncView actions={actions} settings={settings} />);

    expect(
      await screen.findByText(/^Manual: 3 items, 2 files on /),
    ).toBeInTheDocument();
  });

  it("surfaces the most recent automatic backup failure", async () => {
    githubSyncActivityMock.mockResolvedValue({
      completedAt: "2026-07-14T18:30:00.000Z",
      errorCode: "github_sync_failed",
      errorMessage: "GitHub was unavailable",
      filesWritten: 1,
      outcome: "error",
      repo: settings.githubRepo,
      source: "automatic",
      syncedItems: 2,
    });

    render(<SyncView actions={actions} settings={settings} />);

    expect(
      await screen.findByText(/^Automatic backup failed /),
    ).toBeInTheDocument();
    expect(screen.getByText("GitHub was unavailable")).toBeInTheDocument();
    expect(screen.getByText("Last backup attempt")).toBeInTheDocument();
  });

  it("blocks a manual backup until at least one backup type is selected", async () => {
    render(
      <SyncView
        actions={actions}
        settings={{
          ...settings,
          githubSyncAllTranscripts: false,
          githubSyncEnabled: false,
        }}
      />,
    );

    expect(
      await screen.findByText("Choose what to back up"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sync now" })).toBeDisabled();
  });

  it("previews and restores the exact selected backup with safe defaults", async () => {
    previewTranscriptImportMock.mockResolvedValue({
      audioPathsRemoved: 1,
      conflicts: 1,
      dictations: 3,
      fileName: "scribe-export.json",
      fingerprint: "abc123",
      metadataCorrected: 1,
      notes: 2,
      path: "C:\\Backups\\scribe-export.json",
      total: 5,
    });
    restoreTranscriptImportMock.mockResolvedValue({
      imported: 4,
      replaced: 0,
      skipped: 1,
    });
    render(<SyncView actions={actions} settings={settings} />);

    fireEvent.click(screen.getByRole("button", { name: "Choose backup…" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Restore Scribe backup?",
    });
    expect(dialog).toHaveTextContent("2 notes");
    expect(dialog).toHaveTextContent("3 dictations");
    expect(dialog).toHaveTextContent("1 existing");
    expect(
      screen.getByRole("checkbox", {
        name: /Replace existing transcript data/,
      }),
    ).not.toBeChecked();

    fireEvent.click(screen.getByRole("button", { name: "Restore" }));
    await waitFor(() =>
      expect(restoreTranscriptImportMock).toHaveBeenCalledWith(
        "C:\\Backups\\scribe-export.json",
        false,
        "abc123",
      ),
    );
    expect(
      await screen.findByText(
        "Restored 4 transcripts. Skipped 1 existing transcript.",
      ),
    ).toBeInTheDocument();
    expect(refreshMock).toHaveBeenCalledOnce();
  });
});
