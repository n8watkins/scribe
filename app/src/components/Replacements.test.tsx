import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, ReplacementsInfo } from "../backend";
import type { ViewActions } from "../types";

const { getMock, addMock, removeMock } = vi.hoisted(() => ({
  getMock: vi.fn(),
  addMock: vi.fn(),
  removeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("../backend", async (importOriginal) => {
  const original = await importOriginal<typeof import("../backend")>();
  return {
    ...original,
    getReplacements: getMock,
    addReplacements: addMock,
    removeReplacement: removeMock,
    openReplacementsFile: vi.fn(() => Promise.resolve()),
  };
});

import { Replacements } from "./Replacements";

const info: ReplacementsInfo = {
  path: "C:\\data\\replacements.txt",
  rows: [
    { from: "nate builds", to: "n8builds" },
    { from: "nate bills", to: "n8builds" },
    { from: "clawed", to: "Claude" },
  ],
  skippedLines: [14, 31],
};

const actions = { savingSettings: false, updateSettings: vi.fn() } as unknown as ViewActions;
const settings = { replacementSuggestionDismissed: true } as unknown as AppSettings;

beforeEach(() => {
  getMock.mockResolvedValue(info);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Replacements", () => {
  it("shows a count, the skipped lines and a list that scrolls in place", async () => {
    render(<Replacements actions={actions} settings={settings} />);
    expect(await screen.findByText("3")).toBeInTheDocument();
    expect(screen.getByText(/2 lines skipped: 14, 31/)).toBeInTheDocument();
    const list = screen.getByRole("list", { name: "Replacements" });
    expect(list).toHaveClass("replacements-list");
    expect(list.querySelectorAll("li")).toHaveLength(3);
  });

  it("finds entries and removes one", async () => {
    removeMock.mockResolvedValue({ ...info, rows: info.rows.slice(1) });
    render(<Replacements actions={actions} settings={settings} />);
    await screen.findByText("3");
    fireEvent.change(screen.getByRole("textbox", { name: "Find a replacement" }), {
      target: { value: "nate" },
    });
    expect(screen.getAllByRole("button", { name: /Remove nate/ })).toHaveLength(2);
    expect(screen.queryByRole("button", { name: "Remove clawed" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Remove nate builds" }));
    expect(removeMock).toHaveBeenCalledWith({ from: "nate builds", to: "n8builds" });
  });

  it("adds one through the file", async () => {
    addMock.mockResolvedValue({ added: 1, duplicates: 0, info });
    render(<Replacements actions={actions} settings={settings} />);
    await screen.findByText("3");
    fireEvent.change(screen.getByRole("textbox", { name: "Heard" }), { target: { value: "versal" } });
    fireEvent.change(screen.getByRole("textbox", { name: "Written" }), { target: { value: "Vercel" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(addMock).toHaveBeenCalledWith([{ from: "versal", to: "Vercel" }]);
    expect(await screen.findByText("Added 1.")).toBeInTheDocument();
  });

  it("previews a pasted list as new and already there before adding", async () => {
    addMock.mockResolvedValue({ added: 1, duplicates: 0, info });
    render(<Replacements actions={actions} settings={settings} />);
    await screen.findByText("3");
    fireEvent.click(screen.getByRole("button", { name: "Paste a list" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Replacements to add, one per line" }), {
      target: { value: "clawed -> Claude\n- superbase = Supabase\nno arrow" },
    });
    expect(screen.getByText(/1 new, 1 already there, 1 not understood/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add 1" }));
    await waitFor(() =>
      expect(addMock).toHaveBeenCalledWith([{ from: "superbase", to: "Supabase" }]),
    );
  });

  it("opens the format rules in a dialog, with no AI prompt", async () => {
    render(<Replacements actions={actions} settings={settings} />);
    await screen.findByText("3");
    fireEvent.click(screen.getByRole("button", { name: "Format" }));
    const dialog = screen.getByRole("dialog", { name: "How to write replacements" });
    expect(dialog).toHaveTextContent("One replacement per line");
    expect(screen.queryByText(/AI chat/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("says where the suggested fixes come from", async () => {
    render(
      <Replacements
        actions={actions}
        settings={{ replacementSuggestionDismissed: false } as unknown as AppSettings}
      />,
    );
    await screen.findByText("3");
    expect(
      screen.getByText(/for words speech models often mishear in developer talk/),
    ).toBeInTheDocument();
  });
});
