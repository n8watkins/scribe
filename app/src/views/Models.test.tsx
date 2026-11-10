import "@testing-library/jest-dom/vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, ModelInfo } from "../backend";
import type { ViewActions } from "../types";

const { deleteModelMock, listModelsMock } = vi.hoisted(() => ({
  deleteModelMock: vi.fn(),
  listModelsMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("../backend", async (importOriginal) => {
  const original = await importOriginal<typeof import("../backend")>();
  return {
    ...original,
    deleteModel: deleteModelMock,
    getEngineStatus: vi.fn(() => new Promise(() => {})),
    listModels: listModelsMock,
  };
});

import { ModelsView } from "./Models";

const tinyEn = {
  id: "tiny.en",
  name: "Tiny English",
  filename: "ggml-tiny.en.bin",
  downloadUrl: "",
  diskSizeLabel: "75 MB",
  localPath: "/models/ggml-tiny.en.bin",
  source: "app_data",
  sizeBytes: 77_691_713,
  status: "downloaded",
  checksum: null,
  selected: false,
  downloadedAt: null,
  multilingual: false,
  engine: "whisper",
  credit: null,
} as ModelInfo;

const settings = {
  keepModelLoaded: false,
  selectedModelId: "base.en",
} as unknown as AppSettings;

const actions = {
  refresh: vi.fn(() => Promise.resolve()),
  savingSettings: false,
  updateSettings: vi.fn(),
} as unknown as ViewActions;

afterEach(cleanup);

describe("ModelsView delete", () => {
  beforeEach(() => {
    deleteModelMock.mockReset();
    listModelsMock.mockReset();
    listModelsMock.mockResolvedValue([tinyEn]);
  });

  async function openDeleteDialog() {
    render(<ModelsView actions={actions} settings={settings} />);
    fireEvent.click(
      await screen.findByRole("button", { name: /browse models/i }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Delete model" }));
    return screen.getByRole("dialog", { name: "Delete Tiny English?" });
  }

  it("asks in a Scribe dialog and deletes only after confirming", async () => {
    const confirmSpy = vi.spyOn(window, "confirm");
    let finishDelete: (value: ModelInfo) => void = () => {};
    deleteModelMock.mockReturnValue(
      new Promise<ModelInfo>((resolve) => {
        finishDelete = resolve;
      }),
    );

    const dialog = await openDeleteDialog();
    expect(confirmSpy).not.toHaveBeenCalled();
    expect(dialog).toHaveTextContent(
      "This frees 75 MB on this device. You can download it again any time.",
    );
    expect(deleteModelMock).not.toHaveBeenCalled();

    fireEvent.click(within(dialog).getByRole("button", { name: "Delete model" }));
    expect(deleteModelMock).toHaveBeenCalledWith("tiny.en");
    // Busy while the delete runs: both buttons are locked.
    await waitFor(() =>
      expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled(),
    );

    listModelsMock.mockResolvedValue([
      { ...tinyEn, status: "not_downloaded", source: null },
    ]);
    finishDelete({ ...tinyEn, status: "not_downloaded" });
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    confirmSpy.mockRestore();
  });

  it("keeps the model when the dialog is cancelled", async () => {
    const dialog = await openDeleteDialog();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(deleteModelMock).not.toHaveBeenCalled();
  });
});
