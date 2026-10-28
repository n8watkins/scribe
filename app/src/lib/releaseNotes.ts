// The changelog ships inside the app, so release notes show offline and
// without a request to GitHub. release-kit writes each version's section in
// the release commit, so a build always carries its own notes.
import changelog from "../../../CHANGELOG.md?raw";

/** One block of a release-notes section, ready to render. */
export type NotesBlock =
  | { kind: "heading"; text: string }
  | { kind: "paragraph"; text: string }
  | { kind: "list"; items: string[] };

/** Inline pieces of one line of notes. */
export type NotesInline =
  | { kind: "text"; text: string }
  | { kind: "strong"; text: string }
  | { kind: "code"; text: string }
  | { kind: "link"; text: string; url: string };

/** "0.10.2" -> "0.10"; "0.9.1-42" -> "0.9". */
export function shortVersion(version: string): string {
  const [major, minor] = version.split(/[.-]/);
  return minor === undefined ? version : `${major}.${minor}`;
}

/** The body of `## <version>` in a Keep-a-Changelog style file, without its
 * heading, or null when the file has no section for that version. */
export function notesForVersion(
  markdown: string,
  version: string,
): string | null {
  const lines = markdown.split(/\r?\n/);
  const heading = new RegExp(`^##\\s+v?${escapeRegExp(version)}(\\s|$)`);
  const start = lines.findIndex((line) => heading.test(line));
  if (start < 0) {
    return null;
  }
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((line) => /^##\s/.test(line));
  const body = (end < 0 ? rest : rest.slice(0, end)).join("\n").trim();
  return body || null;
}

/** The bundled notes for `version`, or null. */
export function bundledNotes(version: string): string | null {
  return notesForVersion(changelog, version);
}

/** Splits release notes into headings, paragraphs and bullet lists. Indented
 * lines continue the bullet above them; anything else is plain text, so no
 * HTML from the notes is ever rendered. */
export function parseNotes(markdown: string): NotesBlock[] {
  const blocks: NotesBlock[] = [];
  let paragraph: string[] = [];
  let list: string[] | null = null;

  const flushParagraph = () => {
    if (paragraph.length) {
      blocks.push({ kind: "paragraph", text: paragraph.join(" ") });
      paragraph = [];
    }
  };
  const flushList = () => {
    if (list?.length) {
      blocks.push({ kind: "list", items: list });
    }
    list = null;
  };

  for (const raw of markdown.split(/\r?\n/)) {
    const line = raw.trimEnd();
    const trimmed = line.trim();
    if (!trimmed) {
      flushParagraph();
      flushList();
      continue;
    }
    const heading = /^#{1,6}\s+(.*)$/.exec(trimmed);
    const bullet = /^[-*]\s+(.*)$/.exec(trimmed);
    const boldHeading = /^\*\*([^*]+)\*\*$/.exec(trimmed);
    if (heading || boldHeading) {
      flushParagraph();
      flushList();
      blocks.push({
        kind: "heading",
        text: (heading?.[1] ?? boldHeading?.[1] ?? "").trim(),
      });
    } else if (bullet && !/^\s{2,}/.test(line)) {
      flushParagraph();
      list ??= [];
      list.push(bullet[1]);
    } else if (list?.length && /^\s+/.test(line)) {
      list[list.length - 1] += ` ${trimmed}`;
    } else {
      flushList();
      paragraph.push(trimmed);
    }
  }
  flushParagraph();
  flushList();
  return blocks;
}

/** Splits one line into text, **bold**, `code` and [links](https://...). */
export function parseInline(text: string): NotesInline[] {
  const pieces: NotesInline[] = [];
  const pattern = /\*\*([^*]+)\*\*|`([^`]+)`|\[([^\]]+)\]\((https:\/\/[^)\s]+)\)/g;
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    const index = match.index ?? 0;
    if (index > last) {
      pieces.push({ kind: "text", text: text.slice(last, index) });
    }
    if (match[1] !== undefined) {
      pieces.push({ kind: "strong", text: match[1] });
    } else if (match[2] !== undefined) {
      pieces.push({ kind: "code", text: match[2] });
    } else {
      pieces.push({ kind: "link", text: match[3], url: match[4] });
    }
    last = index + match[0].length;
  }
  if (last < text.length) {
    pieces.push({ kind: "text", text: text.slice(last) });
  }
  return pieces;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
