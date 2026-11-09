import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, PastePermission } from "../backend";

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

function mac(granted: boolean): PastePermission {
  return { required: true, granted, kind: "accessibility", declined: false, method: "native" };
}

function wayland(granted: boolean, declined = false): PastePermission {
  return { required: true, granted, kind: "waylandPortal", declined, method: "portal" };
}

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
      .mockResolvedValueOnce(mac(false))
      .mockResolvedValue(mac(true));
    requestPastePermissionMock.mockResolvedValue(mac(false));
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
    getPastePermissionMock.mockResolvedValue({
      required: false,
      granted: true,
      kind: "none",
      declined: false,
      method: "native",
    } satisfies PastePermission);
    renderRows();

    await vi.waitFor(() => expect(getPastePermissionMock).toHaveBeenCalled());
    expect(screen.getByText("After dictation")).toBeInTheDocument();
    expect(screen.queryByText("Accessibility permission")).not.toBeInTheDocument();
  });

  it("asks the desktop once for the Wayland paste permission", async () => {
    getPastePermissionMock.mockResolvedValue(wayland(false));
    let finish: (value: PastePermission) => void = () => undefined;
    requestPastePermissionMock.mockReturnValue(
      new Promise<PastePermission>((resolve) => {
        finish = resolve;
      }),
    );
    renderRows();

    fireEvent.click(await screen.findByRole("button", { name: "Allow pasting" }));
    expect(requestPastePermissionMock).toHaveBeenCalledTimes(1);
    // The desktop's dialog is up: the button waits instead of asking twice.
    expect(
      screen.getByRole("button", { name: "Waiting for your desktop..." }),
    ).toBeDisabled();

    finish(wayland(true));
    await vi.waitFor(() =>
      expect(screen.queryByText("Paste into Wayland apps")).not.toBeInTheDocument(),
    );
  });

  it("offers to ask again after the Wayland request was turned down", async () => {
    getPastePermissionMock.mockResolvedValue(wayland(false, true));
    requestPastePermissionMock.mockRejectedValue({ code: "paste_permission_declined" });
    renderRows();

    fireEvent.click(await screen.findByRole("button", { name: "Ask again" }));
    await vi.waitFor(() => expect(getPastePermissionMock).toHaveBeenCalledTimes(2));
    expect(screen.getByText(/you press Ctrl\+V/)).toBeInTheDocument();
  });
});
