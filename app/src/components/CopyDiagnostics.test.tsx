import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const { collectMock } = vi.hoisted(() => ({ collectMock: vi.fn() }));

vi.mock("../backend", async (importOriginal) => {
  const original = await importOriginal<typeof import("../backend")>();
  return { ...original, collectDiagnostics: collectMock };
});

import { CopyDiagnostics } from "./CopyDiagnostics";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function stubClipboard(writeText: (text: string) => Promise<void>) {
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
}

describe("CopyDiagnostics", () => {
  it("copies the backend report with the WebView version", async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
    stubClipboard(writeText);
    collectMock.mockResolvedValue("Scribe diagnostics\nApp: Scribe 0.9.0\n");
    render(<CopyDiagnostics />);

    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));

    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
    expect(writeText).toHaveBeenCalledTimes(1);
    const copied = writeText.mock.calls[0][0];
    expect(copied).toMatch(/^Scribe diagnostics\nApp: Scribe 0\.9\.0\nWebView: /);
  });

  it("names the OS detail the report includes", () => {
    render(<CopyDiagnostics />);
    expect(screen.getByText(/Windows version/)).toBeInTheDocument();
    cleanup();

    window.__SCRIBE_PLATFORM__ = "linux";
    try {
      render(<CopyDiagnostics />);
      expect(screen.getByText(/version, Linux distribution, a summary/)).toBeInTheDocument();
    } finally {
      delete window.__SCRIBE_PLATFORM__;
    }
  });

  it("shows why copying failed", async () => {
    stubClipboard(() => Promise.resolve());
    collectMock.mockRejectedValue({ code: "database_error", message: "Database is busy." });
    render(<CopyDiagnostics />);

    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Database is busy.");
    expect(screen.getByRole("button", { name: "Copy diagnostics" })).toBeEnabled();
  });
});
