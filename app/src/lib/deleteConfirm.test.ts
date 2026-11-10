import { describe, expect, it } from "vitest";
import type { Transcript } from "../backend";
import { deleteModelMessage, deleteTranscriptMessage } from "./deleteConfirm";

describe("delete confirm messages", () => {
  it("names the text and the audio clip a transcript loses", () => {
    const item = { audioPath: "/clips/a.wav", isNote: false } as Transcript;
    expect(deleteTranscriptMessage(item)).toBe(
      "Its text and audio clip are removed from this device. This transcript can't be recovered.",
    );
  });

  it("leaves out the audio clip when none was kept, and says note for notes", () => {
    const note = { audioPath: null, isNote: true } as Transcript;
    expect(deleteTranscriptMessage(note)).toBe(
      "Its text is removed from this device. This note can't be recovered.",
    );
  });

  it("says how much space a model frees and that it can come back", () => {
    expect(
      deleteModelMessage({ id: "tiny.en", diskSizeLabel: "75 MB" }, "base.en"),
    ).toBe("This frees 75 MB on this device. You can download it again any time.");
  });

  it("warns when the model is the one Scribe uses now", () => {
    expect(
      deleteModelMessage({ id: "base.en", diskSizeLabel: "142 MB" }, "base.en"),
    ).toContain("It's the model Scribe uses now");
  });
});
