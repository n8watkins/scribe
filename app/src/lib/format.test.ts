import { describe, expect, it } from "vitest";
import type {
  AppSettings,
  MicrophoneInfo,
  OutputMode,
  OutputResult,
  PasteMethod,
} from "../backend";
import {
  afterDictationOf,
  clipboardOutcome,
  clipboardStatus,
  insertMethodOf,
  microphoneDisplayName,
  outputSettingsFor,
  resolveMicrophone,
  type AfterDictation,
  type InsertMethod,
} from "./format";

describe("after-dictation choice", () => {
  it("reads every saved output mode", () => {
    expect(afterDictationOf("auto_paste")).toBe("insert");
    expect(afterDictationOf("copy_and_paste")).toBe("insert");
    expect(afterDictationOf("copy_clipboard")).toBe("copy");
    expect(afterDictationOf("save_only")).toBe("save");
  });

  it("reads the insert method from the output mode and paste method", () => {
    const read = (outputMode: OutputMode, pasteMethod: PasteMethod) =>
      insertMethodOf({ outputMode, pasteMethod });

    expect(read("auto_paste", "clipboard_paste")).toBe("paste_restore");
    // The legacy name for clipboard paste reads the same way.
    expect(read("auto_paste", "clipboard_restore")).toBe("paste_restore");
    expect(read("auto_paste", "direct_insert")).toBe("type");
    expect(read("copy_and_paste", "clipboard_paste")).toBe("paste_keep");
    // Copy + paste always goes through the clipboard.
    expect(read("copy_and_paste", "direct_insert")).toBe("paste_keep");
  });

  it("writes the matching settings for each insert method", () => {
    expect(outputSettingsFor("insert", "paste_restore")).toEqual({
      outputMode: "auto_paste",
      pasteMethod: "clipboard_paste",
    });
    expect(outputSettingsFor("insert", "paste_keep")).toEqual({
      outputMode: "copy_and_paste",
      pasteMethod: "clipboard_paste",
    });
    expect(outputSettingsFor("insert", "type")).toEqual({
      outputMode: "auto_paste",
      pasteMethod: "direct_insert",
    });
  });

  it("leaves the paste method alone for copy only and save only", () => {
    expect(outputSettingsFor("copy", "type")).toEqual({
      outputMode: "copy_clipboard",
    });
    expect(outputSettingsFor("save", "type")).toEqual({
      outputMode: "save_only",
    });
  });

  it("round-trips every choice", () => {
    const methods: InsertMethod[] = ["paste_restore", "paste_keep", "type"];
    const afters: AfterDictation[] = ["insert", "copy", "save"];
    for (const after of afters) {
      for (const method of methods) {
        const patch = outputSettingsFor(after, method);
        const settings = {
          outputMode: patch.outputMode as OutputMode,
          pasteMethod: patch.pasteMethod ?? ("clipboard_paste" as PasteMethod),
        };
        expect(afterDictationOf(settings.outputMode)).toBe(after);
        if (after === "insert") {
          expect(insertMethodOf(settings)).toBe(method);
        }
      }
    }
  });
});

describe("clipboard status label", () => {
  const label = (outputMode: OutputMode, pasteMethod: PasteMethod) =>
    clipboardStatus({ outputMode, pasteMethod } as AppSettings);

  it("describes what each output choice does to the clipboard", () => {
    expect(label("auto_paste", "clipboard_paste")).toBe("Clipboard restored");
    expect(label("auto_paste", "direct_insert")).toBe("Types it out");
    expect(label("copy_and_paste", "clipboard_paste")).toBe("On your clipboard");
    expect(label("copy_clipboard", "direct_insert")).toBe("On your clipboard");
    expect(label("save_only", "direct_insert")).toBe("Clipboard untouched");
  });
});

describe("clipboard outcome label", () => {
  const outcome = (patch: Partial<OutputResult>) =>
    clipboardOutcome({
      transcriptId: "tx_1",
      action: "paste",
      status: "completed",
      outputMode: "auto_paste",
      pasteMethod: "clipboard_paste",
      copied: false,
      pasted: true,
      clipboardRestored: true,
      clipboardPreservation: "restored_after_paste",
      clipboardRestoreError: null,
      message: "",
      ...patch,
    } as OutputResult);

  it("says the clipboard was restored only when it was", () => {
    expect(outcome({})).toBe("Clipboard restored");
  });

  it("says the text is on the clipboard when a paste was blocked", () => {
    expect(
      outcome({
        status: "paste_skipped",
        pasted: false,
        copied: true,
        clipboardRestored: null,
        clipboardPreservation: "replaced_with_transcript",
      }),
    ).toBe("On your clipboard");
  });

  it("says the text is on the clipboard when restoring failed", () => {
    expect(
      outcome({
        status: "clipboard_restore_failed",
        clipboardRestored: false,
        clipboardPreservation: "restore_failed",
      }),
    ).toBe("On your clipboard");
  });

  it("describes typing and leaving the clipboard alone", () => {
    expect(
      outcome({
        pasteMethod: "direct_insert",
        clipboardPreservation: "untouched",
      }),
    ).toBe("Typed out");
    expect(outcome({ pasted: false, clipboardPreservation: "untouched" })).toBe(
      "Clipboard untouched",
    );
  });
});

describe("resolveMicrophone", () => {
  const mic = (
    id: string,
    name: string,
    isDefault = false,
  ): MicrophoneInfo => ({
    id,
    name,
    endpointId: id,
    isDefault,
    isSelected: false,
    isAvailable: true,
  });
  const headset = mic("{0.0.1}.{headset}", "Headset Microphone (HyperX)");
  const fifine = mic("{0.0.1}.{fifine}", "Microphone (FIFINE)", true);
  const lifecam = mic("{0.0.1}.{lifecam}", "Microphone (LifeCam)");
  const devices = [headset, fifine, lifecam];

  it("uses the saved microphone when its id matches", () => {
    expect(resolveMicrophone(devices, headset.id)).toEqual({
      kind: "device",
      microphone: headset,
      fellBack: false,
    });
  });

  it("matches the saved value against the device name too", () => {
    expect(resolveMicrophone(devices, "Microphone (LifeCam)")).toEqual({
      kind: "device",
      microphone: lifecam,
      fellBack: false,
    });
  });

  it("uses the default without a fallback when nothing is saved", () => {
    expect(resolveMicrophone(devices, null)).toEqual({
      kind: "device",
      microphone: fifine,
      fellBack: false,
    });
  });

  it("falls back to the default when the saved microphone is gone", () => {
    const resolved = resolveMicrophone(
      devices,
      "{0.0.1.00000000}.{5be65c4e-0d9c-45ee-803c-f4d863cbde3c}",
    );
    expect(resolved).toEqual({
      kind: "device",
      microphone: fifine,
      fellBack: true,
    });
    expect(microphoneDisplayName(resolved)).toBe("FIFINE");
  });

  it("falls back to the first device when none is the default", () => {
    expect(resolveMicrophone([headset, lifecam], "gone")).toEqual({
      kind: "device",
      microphone: headset,
      fellBack: true,
    });
  });

  it("reports no microphone for an empty list", () => {
    const resolved = resolveMicrophone([], "gone");
    expect(resolved).toEqual({ kind: "none" });
    expect(microphoneDisplayName(resolved)).toBe("No microphone found");
    expect(resolveMicrophone([], null)).toEqual({ kind: "none" });
  });

  it("is still loading while the list is unknown", () => {
    const resolved = resolveMicrophone(null, headset.id);
    expect(resolved).toEqual({ kind: "loading" });
    expect(microphoneDisplayName(resolved)).toBe("-");
  });
});
