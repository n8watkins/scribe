import { describe, expect, it } from "vitest";
import {
  bundledNotes,
  notesForVersion,
  parseInline,
  parseNotes,
  shortVersion,
} from "./releaseNotes";

const CHANGELOG = `# Changelog

## 0.10.0 - 2026-11-02

Summary line.

### New

- First thing.
  Continued on the next line.
- Second with [#12](https://github.com/n8watkins/scribe/pull/12)

## 0.9.0 - 2026-10-28

### Fixed

- Older fix
`;

describe("release notes", () => {
  it("finds a version's section and stops at the next one", () => {
    const notes = notesForVersion(CHANGELOG, "0.10.0");
    expect(notes).toContain("Summary line.");
    expect(notes).toContain("Second with");
    expect(notes).not.toContain("Older fix");
    expect(notesForVersion(CHANGELOG, "0.9.0")).toBe("### Fixed\n\n- Older fix");
  });

  it("does not match a version that only shares a prefix", () => {
    expect(notesForVersion(CHANGELOG, "0.1")).toBeNull();
    expect(notesForVersion(CHANGELOG, "0.10.0-5")).toBeNull();
  });

  it("parses headings, paragraphs and lists with continuation lines", () => {
    const blocks = parseNotes(notesForVersion(CHANGELOG, "0.10.0") ?? "");
    expect(blocks).toEqual([
      { kind: "paragraph", text: "Summary line." },
      { kind: "heading", text: "New" },
      {
        kind: "list",
        items: [
          "First thing. Continued on the next line.",
          "Second with [#12](https://github.com/n8watkins/scribe/pull/12)",
        ],
      },
    ]);
  });

  it("splits inline bold, code and https links only", () => {
    expect(
      parseInline("A **b** `c` [d](https://x.dev/e) [f](javascript:g)"),
    ).toEqual([
      { kind: "text", text: "A " },
      { kind: "strong", text: "b" },
      { kind: "text", text: " " },
      { kind: "code", text: "c" },
      { kind: "text", text: " " },
      { kind: "link", text: "d", url: "https://x.dev/e" },
      { kind: "text", text: " [f](javascript:g)" },
    ]);
  });

  it("shortens versions to major.minor", () => {
    expect(shortVersion("0.10.2")).toBe("0.10");
    expect(shortVersion("0.9.1-42")).toBe("0.9");
  });

  it("bundles the real changelog", () => {
    expect(bundledNotes("0.8.0")).toContain("GitHub backup");
  });
});
