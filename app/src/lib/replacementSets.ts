import type { TextReplacement } from "../backend";

/** The entries of `set` not already in `current` (by heard phrase). */
export function missingFrom(
  current: readonly TextReplacement[],
  set: readonly TextReplacement[],
): TextReplacement[] {
  const have = new Set(current.map((row) => row.from.trim().toLowerCase()));
  return set.filter((row) => !have.has(row.from.trim().toLowerCase()));
}

export type ParsedReplacements = {
  pairs: TextReplacement[];
  /** Lines that look like content but have no separator, with their number. */
  errors: { line: number; text: string }[];
};

// "wrong -> right", "wrong => right", "wrong → right", "wrong = right",
// "wrong<TAB>right", or "wrong, right" (the first comma).
const ARROW = /\s*(?:->|=>|→|=|\t)\s*/;
const COMMA = /\s*,\s*/;
const BULLET = /^(?:[-*•+]\s+|\d+[.)]\s+)/;
const QUOTES = /^["'“”‘’`]+|["'“”‘’`]+$/g;

const clean = (text: string) => text.trim().replace(QUOTES, "").trim();

/** Reads pasted replacements, one per line. Bullets, numbering, quotes and
 * surrounding spaces are dropped; blank lines and comments (# or //) are
 * skipped. */
export function parseReplacementLines(text: string): ParsedReplacements {
  const pairs: TextReplacement[] = [];
  const errors: ParsedReplacements["errors"] = [];
  text.split(/\r?\n/).forEach((raw, index) => {
    const line = raw.trim().replace(BULLET, "");
    if (!line || line.startsWith("#") || line.startsWith("//")) {
      return;
    }
    const match = ARROW.exec(line) ?? COMMA.exec(line);
    if (!match || match.index === 0) {
      errors.push({ line: index + 1, text: raw.trim() });
      return;
    }
    const from = clean(line.slice(0, match.index));
    const to = clean(line.slice(match.index + match[0].length));
    if (!from || !to) {
      errors.push({ line: index + 1, text: raw.trim() });
      return;
    }
    pairs.push({ from, to });
  });
  return { pairs, errors };
}

/** Replacements as text, one "wrong -> right" per line, for copying out and
 * pasting back. */
export function formatReplacementLines(rows: readonly TextReplacement[]): string {
  return rows
    .filter((row) => row.from.trim() && row.to.trim())
    .map((row) => `${row.from.trim()} -> ${row.to.trim()}`)
    .join("\n");
}
