import { describe, expect, it } from "vitest";
import type { AppSettings, OutputMode, PasteMethod } from "../backend";
import {
  afterDictationOf,
  clipboardStatus,
  insertMethodOf,
  outputSettingsFor,
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
