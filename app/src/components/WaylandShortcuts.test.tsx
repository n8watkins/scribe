import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HotkeyStatus } from "../backend";

const { bindMock, configureMock } = vi.hoisted(() => ({
  bindMock: vi.fn(),
  configureMock: vi.fn(),
}));

vi.mock("../backend", async (importOriginal) => {
  const original = await importOriginal<typeof import("../backend")>();
  return {
    ...original,
    bindPortalShortcuts: bindMock,
    configurePortalShortcuts: configureMock,
  };
});

import { WaylandShortcutHelp } from "./WaylandShortcuts";

function status(patch: Partial<HotkeyStatus>): HotkeyStatus {
  return {
    bindings: [],
    noteChordActive: false,
    noteChordSupported: false,
    holdReleaseVerificationRequired: false,
    backend: "portal",
    portalState: "active",
    portalConfigurable: false,
    controlCommand: "/usr/bin/Scribe",
    ...patch,
  };
}

beforeEach(() => {
  window.__SCRIBE_PLATFORM__ = "linux";
});

afterEach(() => {
  delete window.__SCRIBE_PLATFORM__;
  cleanup();
  bindMock.mockReset();
  configureMock.mockReset();
});

describe("WaylandShortcutHelp", () => {
  it.each(["windows", "macos"])(
    "never shows Wayland guidance on %s, whatever the status says",
    (platform) => {
      window.__SCRIBE_PLATFORM__ = platform;
      for (const backend of ["x11OnWayland", "portal"] as const) {
        const { container } = render(
          <WaylandShortcutHelp onStatus={vi.fn()} status={status({ backend })} />,
        );
        expect(container).toBeEmptyDOMElement();
        cleanup();
      }
    },
  );

  it("shows nothing outside Wayland", () => {
    const { container } = render(
      <WaylandShortcutHelp onStatus={vi.fn()} status={status({ backend: "native" })} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("explains X11-only hotkeys and gives the exact commands to bind", () => {
    render(
      <WaylandShortcutHelp
        onStatus={vi.fn()}
        status={status({ backend: "x11OnWayland", portalState: null })}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "only work while an X11 app is in front",
    );
    expect(screen.getByText("/usr/bin/Scribe toggle")).toBeInTheDocument();
    expect(screen.getByText("/usr/bin/Scribe paste-last")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Copy /usr/bin/Scribe toggle" }),
    ).toBeInTheDocument();
  });

  it("copies a command", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    render(
      <WaylandShortcutHelp
        onStatus={vi.fn()}
        status={status({ backend: "x11OnWayland", portalState: null })}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Copy /usr/bin/Scribe toggle" }));
    expect(writeText).toHaveBeenCalledWith("/usr/bin/Scribe toggle");
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("sets the shortcuts up again after the desktop's dialog was cancelled", async () => {
    const next = status({ portalState: "active" });
    bindMock.mockResolvedValue(next);
    const onStatus = vi.fn();
    render(<WaylandShortcutHelp onStatus={onStatus} status={status({ portalState: "declined" })} />);

    fireEvent.click(screen.getByRole("button", { name: "Set up shortcuts" }));
    await vi.waitFor(() => expect(onStatus).toHaveBeenCalledWith(next));
    expect(bindMock).toHaveBeenCalledTimes(1);
  });

  it("opens the desktop's shortcut settings where the portal can", () => {
    configureMock.mockResolvedValue(undefined);
    render(
      <WaylandShortcutHelp onStatus={vi.fn()} status={status({ portalConfigurable: true })} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Change keys" }));
    expect(configureMock).toHaveBeenCalledTimes(1);
  });
});
