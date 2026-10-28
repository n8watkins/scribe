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

import { ReleaseNotesBody, WhatsNewCard } from "./ReleaseNotes";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("WhatsNewCard", () => {
  it("shows the bundled notes once and remembers the dismissal", async () => {
    pendingMock.mockResolvedValue("0.8.0");
    dismissMock.mockResolvedValue(undefined);
    render(<WhatsNewCard />);

    expect(await screen.findByText("What's new in 0.8")).toBeInTheDocument();
    expect(screen.getByText(/GitHub backup signs in/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Got it" }));
    expect(dismissMock).toHaveBeenCalledWith("0.8.0");
    expect(screen.queryByText("What's new in 0.8")).not.toBeInTheDocument();
  });

  it("renders nothing when no card is pending", async () => {
    pendingMock.mockResolvedValue(null);
    const { container } = render(<WhatsNewCard />);
    await Promise.resolve();
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing for a version without notes", async () => {
    pendingMock.mockResolvedValue("0.0.1-7");
    const { container } = render(<WhatsNewCard />);
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
