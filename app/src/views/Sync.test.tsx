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
import { clearSyncActivity } from "../lib/syncActivity";

const {
  githubStatusMock,
  githubSyncNowMock,
  previewTranscriptImportMock,
  restoreTranscriptImportMock,
} = vi.hoisted(() => ({
  githubStatusMock: vi.fn(),
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
    clearSyncActivity();
    githubStatusMock.mockReset();
    githubStatusMock.mockResolvedValue({
      configured: true,
      connected: true,
      repo: settings.githubRepo,
      username: "alice",
    });
    githubSyncNowMock.mockReset();
    previewTranscriptImportMock.mockReset();
    restoreTranscriptImportMock.mockReset();
    refreshMock.mockReset();
  });

  it("shows readiness, coverage, and an empty manual history", async () => {
    render(<SyncView actions={actions} settings={settings} />);

    expect(await screen.findByText("Ready to back up")).toBeInTheDocument();
    expect(screen.getByText("Notes and dictations")).toBeInTheDocument();
    expect(screen.getByText("Not run on this device")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sync now" })).toBeEnabled();
  });

  it("retains the last manual result after the view remounts", async () => {
    githubSyncNowMock.mockResolvedValue({ filesWritten: 2, syncedNotes: 3 });
    const first = render(<SyncView actions={actions} settings={settings} />);
    await screen.findByText("Ready to back up");

    fireEvent.click(screen.getByRole("button", { name: "Sync now" }));
    expect(
      await screen.findByText("Backed up 3 items into 2 files."),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByText(/^3 items, 2 files on /)).toBeInTheDocument(),
    );

    first.unmount();
    render(<SyncView actions={actions} settings={settings} />);

    expect(
      await screen.findByText(/^3 items, 2 files on /),
    ).toBeInTheDocument();
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
