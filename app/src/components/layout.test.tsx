import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Gauge } from "lucide-react";
import { StatsRow, StatusCard } from "./layout";

afterEach(cleanup);

describe("StatsRow", () => {
  it("sets the number large and its unit small", () => {
    render(
      <StatsRow
        stats={{
          wordsToday: 412,
          dictationsToday: 9,
          averageWpm: 148,
          averageTranscriptionLatencyMs: 412,
          averageRecordingDurationMs: 11800,
          mostUsedModel: "base.en",
          totalWordsTranscribed: 48213,
        }}
      />,
    );
    const speed = screen.getByText("Speaking speed").nextElementSibling;
    expect(speed).toHaveTextContent("148wpm");
    expect(speed?.querySelector(".stats-row-unit")).toHaveTextContent("wpm");
    // A plain count has no unit.
    const total = screen.getByText("Total words").nextElementSibling;
    expect(total).toHaveTextContent("48,213");
    expect(total?.querySelector(".stats-row-unit")).toBeNull();
    expect(screen.getByText("Recording time")).toBeInTheDocument();
  });
});

describe("StatusCard", () => {
  it("is one button that opens its page", () => {
    const onAction = vi.fn();
    render(
      <StatusCard
        action="Open Dictation settings"
        detail="Loaded"
        Icon={Gauge}
        label="Status"
        onAction={onAction}
        value="Ready for dictation"
      />,
    );
    const card = screen.getByRole("button", { name: /Status.*Ready for dictation/ });
    expect(card).toHaveAttribute("title", "Open Dictation settings");
    fireEvent.click(card);
    expect(onAction).toHaveBeenCalledOnce();
  });
});
