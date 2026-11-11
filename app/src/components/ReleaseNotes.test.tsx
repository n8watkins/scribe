import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const { pendingMock, dismissMock } = vi.hoisted(() => ({
  pendingMock: vi.fn(),
  dismissMock: vi.fn(),
}));

vi.mock("../backend", async (importOriginal) => {
  const original = await importOriginal<typeof import("../backend")>();
  return {
    ...original,
    whatsNewPending: pendingMock,
    dismissWhatsNew: dismissMock,
    openReleasePage: vi.fn(() => Promise.resolve()),
  };
});

import { ReleaseNotesBody, WhatsNewBanner } from "./ReleaseNotes";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("WhatsNewBanner", () => {
  it("is one slim line that opens the notes and remembers the dismissal", async () => {
    pendingMock.mockResolvedValue("0.8.0");
    dismissMock.mockResolvedValue(undefined);
    const onOpen = vi.fn();
    const { container } = render(<WhatsNewBanner onOpen={onOpen} />);

    expect(await screen.findByText("What's new in 0.8")).toBeInTheDocument();
    // A single line, not the notes themselves.
    expect(container.querySelectorAll("li")).toHaveLength(0);
    expect(container.firstElementChild).toHaveClass("whats-new-banner");

    fireEvent.click(screen.getByRole("button", { name: "See what changed" }));
    expect(onOpen).toHaveBeenCalledOnce();
    expect(dismissMock).toHaveBeenCalledWith("0.8.0");
    expect(screen.queryByText("What's new in 0.8")).not.toBeInTheDocument();
  });

  it("can be dismissed without opening the notes", async () => {
    pendingMock.mockResolvedValue("0.8.0");
    dismissMock.mockResolvedValue(undefined);
    const onOpen = vi.fn();
    render(<WhatsNewBanner onOpen={onOpen} />);
    fireEvent.click(await screen.findByRole("button", { name: "Dismiss what's new" }));
    expect(onOpen).not.toHaveBeenCalled();
    expect(dismissMock).toHaveBeenCalledWith("0.8.0");
  });

  it("renders nothing when nothing is pending", async () => {
    pendingMock.mockResolvedValue(null);
    const { container } = render(<WhatsNewBanner onOpen={vi.fn()} />);
    await Promise.resolve();
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing for a version without notes", async () => {
    pendingMock.mockResolvedValue("0.0.1-7");
    const { container } = render(<WhatsNewBanner onOpen={vi.fn()} />);
    await Promise.resolve();
    await Promise.resolve();
    expect(container).toBeEmptyDOMElement();
  });
});

describe("ReleaseNotesBody", () => {
  it("never renders HTML from the notes", () => {
    const { container } = render(
      <ReleaseNotesBody markdown={"- <img src=x onerror=alert(1)> **ok**"} />,
    );
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByText("ok").tagName).toBe("STRONG");
  });
});
