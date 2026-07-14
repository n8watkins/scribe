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

const { githubStatusMock, githubSyncNowMock } = vi.hoisted(() => ({
  githubStatusMock: vi.fn(),
  githubSyncNowMock: vi.fn(),
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
  };
});

import { SyncView } from "./Sync";

const settings = {
  githubAccountLogin: "alice",
  githubRepo: "alice/scribe-notes",
  githubSyncAllTranscripts: true,
  githubSyncEnabled: true,
} as AppSettings;

const actions = {
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
});
