import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../backend";
import type { ViewActions } from "../types";
import { FillerSettings, PillSettings } from "../views/Settings";
import { CustomWords } from "./CustomWords";
import { UpdatesPanel } from "./UpdatesPanel";

// Settings another setting turns off say why, and link to it when it lives
// on another page (docs/SETTINGS-MAP.md).

afterEach(cleanup);

const actions = {
  savingSettings: false,
  updateSettings: vi.fn(),
  openView: vi.fn(),
} as unknown as ViewActions;

const base = {
  fillerSuppressionEnabled: true,
  fillerWords: ["um"],
  fillerPauseThresholdMs: 300,
  showFloatingPill: true,
  pillDisplayMode: "visualizer",
  selectedModelId: "base.en",
  vocabularyPrompt: "Tauri, Claude",
} as unknown as AppSettings;

function TextArea(props: { ariaLabel: string; value: string; disabled?: boolean }) {
  return <textarea aria-label={props.ariaLabel} disabled={props.disabled} readOnly value={props.value} />;
}

describe("settings that depend on another", () => {
  it("filler pause says filler removal is off", () => {
    render(<FillerSettings actions={actions} settings={{ ...base, fillerSuppressionEnabled: false }} />);
    expect(screen.getByText("Off because Remove filler words is off.")).toBeInTheDocument();
  });

  it("filler pause has no note while filler removal is on", () => {
    render(<FillerSettings actions={actions} settings={base} />);
    expect(screen.queryByText(/Off because/)).not.toBeInTheDocument();
  });

  it("pill style says the pill is hidden", () => {
    render(<PillSettings actions={actions} settings={{ ...base, showFloatingPill: false }} />);
    expect(screen.getByText("Not shown while Show floating pill is off.")).toBeInTheDocument();
  });

  it("automatic install says automatic checks are off", () => {
    render(<UpdatesPanel autoCheckEnabled={false} autoInstallEnabled />);
    expect(
      screen.getByText("Off because Automatically check for updates is off."),
    ).toBeInTheDocument();
  });

  it("custom words say Parakeet doesn't use them and link to the model", () => {
    render(
      <CustomWords
        actions={actions}
        settings={{ ...base, selectedModelId: "parakeet-tdt-0.6b-v3" }}
        TextArea={TextArea}
      />,
    );
    expect(screen.getByText("Not used by Parakeet.")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Custom words" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Change model" }));
    expect(actions.openView).toHaveBeenCalledWith("Models & language");
  });

  it("custom words count characters against 450", () => {
    render(<CustomWords actions={actions} settings={base} TextArea={TextArea} />);
    expect(screen.getByText("13 / 450")).toBeInTheDocument();
  });
});
