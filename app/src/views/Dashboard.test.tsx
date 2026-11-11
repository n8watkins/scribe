import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppStatus, DashboardData } from "../backend";
import type { ViewActions } from "../types";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("../backend", async (importOriginal) => {
  const original = await importOriginal<typeof import("../backend")>();
  const never = () => new Promise(() => {});
  return {
    ...original,
    getHotkeyStatus: vi.fn(never),
    getEngineStatus: vi.fn(never),
    getPastePermission: vi.fn(never),
    getReplacements: vi.fn(never),
    probeGpuDevices: vi.fn(never),
    whatsNewPending: vi.fn(() => Promise.resolve(null)),
  };
});

import { DashboardView } from "./Dashboard";

afterEach(cleanup);

const settings = {
  selectedModelId: "small.en",
  selectedMicId: null,
  outputMode: "auto_paste",
  pasteMethod: "clipboard_paste",
  gpuAcceleration: "auto",
  vocabularyPrompt: "Tauri, Claude",
  keepWhisperLoaded: true,
  keepParakeetLoaded: true,
  incrementalTranscriptionEnabled: true,
  notesAnalysisEnabled: false,
  hotkeys: {
    holdToTalk: "Ctrl+Win",
    toggleDictation: "Backquote",
    pasteLastTranscript: "Ctrl+Alt+V",
    openDashboard: "Ctrl+Alt+F",
    discardDictation: "Ctrl+Alt+X",
    transformSelection: "Ctrl+Alt+R",
  },
};

function data(status: AppStatus): DashboardData {
  return {
    appState: { status, error: null, updatedAt: "2026-11-10T12:00:00Z" },
    settings,
    lastTranscript: {
      id: "t1",
      text: "The last thing I said.",
      createdAt: "2026-11-10T11:59:00Z",
      durationMs: 3000,
      wordCount: 5,
      characterCount: 22,
      modelId: "small.en",
    },
    recentTranscripts: [],
    stats: {
      wordsToday: 1,
      dictationsToday: 1,
      averageWpm: null,
      averageTranscriptionLatencyMs: null,
      averageRecordingDurationMs: null,
      mostUsedModel: null,
      totalWordsTranscribed: 1,
    },
  } as unknown as DashboardData;
}

const actions = { savingSettings: false, updateSettings: vi.fn() } as unknown as ViewActions;

/** Home's blocks in order, by class: the shape of the page. */
function layout(container: HTMLElement): string[] {
  return Array.from(container.children).map((node) => node.className);
}

function renderHome(status: AppStatus, live: string | null) {
  return render(
    <DashboardView
      actions={actions}
      data={data(status)}
      liveTranscript={live === null ? null : ({ text: live } as never)}
      microphones={[]}
      models={null}
      setActiveView={vi.fn()}
    />,
  );
}

describe("Home while dictating", () => {
  it("swaps the live words into the Last transcript card, so nothing moves", () => {
    const idle = renderHome("Idle", null);
    const idleLayout = layout(idle.container);
    const idleGrid = idle.container.querySelector(".main-grid")!;
    const idleCards = Array.from(idleGrid.children).map((node) => node.className);
    expect(screen.getByText("The last thing I said.")).toBeInTheDocument();
    cleanup();

    const live = renderHome("Recording", "and then we ship it");
    expect(layout(live.container)).toEqual(idleLayout);
    const liveGrid = live.container.querySelector(".main-grid")!;
    expect(liveGrid.children).toHaveLength(idleCards.length);
    // Same card frame in the same slot; only its contents change.
    expect(liveGrid.children[0].className).toContain("buffer-card");
    expect(liveGrid.children[1].className).toBe(idleCards[1]);
    expect(screen.getByText("and then we ship it")).toBeInTheDocument();
    expect(screen.getByText(/Hearing you/)).toBeInTheDocument();
    expect(screen.queryByText("The last thing I said.")).not.toBeInTheDocument();
  });

  it("puts the hint inside the text box while it waits for words", () => {
    const { container } = renderHome("Recording", "");
    const box = container.querySelector(".main-grid .buffer-preview");
    expect(box).toHaveTextContent("The finished text lands here when you stop.");
    expect(box).toHaveClass("is-placeholder");
  });

  it("shows the new last transcript again when the dictation ends", () => {
    renderHome("Idle", null);
    expect(screen.queryByText(/Hearing you/)).not.toBeInTheDocument();
    expect(screen.getByText("The last thing I said.")).toBeInTheDocument();
  });
});
