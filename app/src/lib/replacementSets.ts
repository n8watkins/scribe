import type { TextReplacement } from "../backend";

/** Developer words Whisper and Parakeet keep writing the same wrong way,
 * from a benchmark on real dictation (November 2026). Offered once on the
 * Words page; nothing is added until you say so. */
export const DEVELOPER_REPLACEMENTS: readonly TextReplacement[] = [
  { from: "clawed", to: "Claude" },
  { from: "cloud code", to: "Claude Code" },
  { from: "superbase", to: "Supabase" },
  { from: "work tree", to: "worktree" },
  { from: "work trees", to: "worktrees" },
  { from: "mono repo", to: "monorepo" },
  { from: "mono-repo", to: "monorepo" },
  { from: "next js", to: "Next.js" },
  { from: "chat gpt", to: "ChatGPT" },
  { from: "type script", to: "TypeScript" },
  { from: "versal", to: "Vercel" },
  { from: "shad cn", to: "shadcn" },
  { from: "shad c and", to: "shadcn" },
  { from: "shared cn", to: "shadcn" },
  { from: "sub agent", to: "subagent" },
  { from: "sub-agent", to: "subagent" },
  { from: "sub agents", to: "subagents" },
  { from: "local host", to: "localhost" },
  { from: "cloud flare", to: "Cloudflare" },
  { from: "cloudfare", to: "Cloudflare" },
  { from: "vercell", to: "Vercel" },
  { from: "turbo pack", to: "Turbopack" },
  { from: "cloud md", to: "CLAUDE.md" },
];

/** The entries of `set` not already in `current` (by spoken phrase). */
export function missingFrom(
  current: readonly TextReplacement[],
  set: readonly TextReplacement[],
): TextReplacement[] {
  const have = new Set(current.map((row) => row.from.trim().toLowerCase()));
  return set.filter((row) => !have.has(row.from.toLowerCase()));
}

/** Parses pasted lines like "nate builds -> n8builds" (also "=>", "→" or a
 * tab). Lines without a separator are skipped. */
export function parseReplacementLines(text: string): TextReplacement[] {
  const rows: TextReplacement[] = [];
  for (const line of text.split(/\r?\n/)) {
    const match = /^(.+?)\s*(?:->|=>|→|\t)\s*(.+)$/.exec(line.trim());
    if (match && match[1].trim() && match[2].trim()) {
      rows.push({ from: match[1].trim(), to: match[2].trim() });
    }
  }
  return rows;
}
