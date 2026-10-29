import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../backend";

const { getPastePermissionMock, requestPastePermissionMock } = vi.hoisted(() => ({
  getPastePermissionMock: vi.fn(),
  requestPastePermissionMock: vi.fn(),
}));

vi.mock("../backend", async (importOriginal) => {
  const original = await importOriginal<typeof import("../backend")>();
  return {
    ...original,
    getPastePermission: getPastePermissionMock,
    requestPastePermission: requestPastePermissionMock,
  };
});

import { OutputSettingRows } from "./OutputSettings";

const settings = {
  outputMode: "auto_paste",
  pasteMethod: "clipboard_paste",
  hotkeys: { pasteLastTranscript: "Ctrl+Alt+V" },
} as AppSettings;

function renderRows() {
  return render(<OutputSettingRows onChange={vi.fn()} settings={settings} />);
}

afterEach(cleanup);

beforeEach(() => {
  getPastePermissionMock.mockReset();
  requestPastePermissionMock.mockReset();
});

describe("OutputSettingRows paste permission", () => {
  it("offers to open System Settings while the permission is missing", async () => {
    getPastePermissionMock
      .mockResolvedValueOnce({ required: true, granted: false })
      .mockResolvedValue({ required: true, granted: true });
    requestPastePermissionMock.mockResolvedValue({ required: true, granted: false });
    renderRows();

    const button = await screen.findByRole("button", { name: "Open System Settings" });
    fireEvent.click(button);
    expect(requestPastePermissionMock).toHaveBeenCalledTimes(1);

    // Coming back from System Settings with the permission on hides the row.
    fireEvent.focus(window);
    await vi.waitFor(() =>
      expect(screen.queryByText("Accessibility permission")).not.toBeInTheDocument(),
    );
    expect(getPastePermissionMock).toHaveBeenCalledTimes(2);
  });

  it("shows nothing extra where no permission is needed", async () => {
    getPastePermissionMock.mockResolvedValue({ required: false, granted: true });
    renderRows();

    await vi.waitFor(() => expect(getPastePermissionMock).toHaveBeenCalled());
    expect(screen.getByText("After dictation")).toBeInTheDocument();
    expect(screen.queryByText("Accessibility permission")).not.toBeInTheDocument();
  });
});
