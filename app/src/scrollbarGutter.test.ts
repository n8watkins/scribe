import { describe, expect, it } from "vitest";
import app from "./App.css?raw";
import releaseNotes from "./components/release-notes.css?raw";
import transcribe from "./views/transcribe.css?raw";

// Scroll containers whose content can grow or shrink reserve the scrollbar's
// width, so nothing shifts sideways when a scrollbar appears.
const css = { app, releaseNotes, transcribe };

/** The body of the first top-level rule for exactly this selector. */
function rule(source: string, selector: string): string {
  const start = source.search(
    new RegExp(`^${selector.replace(/[.]/g, "\\.")} \\{`, "m"),
  );
  expect(start, `${selector} rule`).toBeGreaterThanOrEqual(0);
  return source.slice(start, source.indexOf("}", start));
}

describe("scrollbar gutter", () => {
  it.each([
    [css.app, ".dashboard"],
    [css.app, ".sidebar"],
    [css.app, ".history-scroll"],
    [css.app, ".model-scroll"],
    [css.app, ".live-transcript-text"],
    [css.releaseNotes, ".update-notes"],
    [css.transcribe, ".transcribe-file-result"],
  ])("%#: %s reserves the scrollbar gutter", (source, selector) => {
    expect(rule(source, selector)).toMatch(/scrollbar-gutter: stable;/);
  });

  it("keeps the main area's right margin equal to its left one", () => {
    expect(rule(css.app, ".dashboard")).toMatch(
      /padding: 14px calc\(16px - var\(--scribe-scrollbar-size\)\) 14px 16px;/,
    );
    expect(rule(css.app, ":root")).toMatch(/--scribe-scrollbar-size: 10px;/);
    expect(rule(css.app, "::-webkit-scrollbar")).toMatch(
      /width: var\(--scribe-scrollbar-size\);/,
    );
  });
});
