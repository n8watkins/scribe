import { describe, expect, it } from "vitest";
import {
  estimateTokens,
  HINT_CHAR_LIMIT,
  hintAsSentence,
  PROMPT_TOKEN_LIMIT,
  trimHintToChars,
} from "./promptBudget";
import {
  formatReplacementLines,
  missingFrom,
  parseReplacementLines,
} from "./replacementSets";
import { modelGuide } from "./modelGuide";

// whisper.cpp's tokenizer (base.en) counts this 401-character hint as 137
// tokens.
const SUGGESTED =
  "Nate's dev notes on n8builds, Appturnity, T-Hub, Scribe, Solara, Wren, PR Post, N8 Tray and pass-env, working with Claude, Claude Code, Codex, ChatGPT and Gemini: GitHub, GitHub Actions, worktree, monorepo, repo, PR, rebase, cherry-pick, CI, pnpm, Tauri, Supabase, Postgres, Drizzle, Cloudflare, wrangler, Vercel, Next.js, TypeScript, Tailwind, shadcn, whisper.cpp, Parakeet, WSL, localhost, subagent.";

/** Live transcription's context: the last 200 characters said. Spoken
 * English runs about 4 to 4.5 characters per token, so 45 to 50 tokens. */
const CONTEXT_TOKENS = 50;

describe("token estimate", () => {
  it("never comes in under whisper.cpp's count and stays close", () => {
    const estimate = estimateTokens(SUGGESTED);
    expect(estimate).toBeGreaterThanOrEqual(137);
    expect(estimate).toBeLessThanOrEqual(137 * 1.15);
  });

  it("counts short lowercase words as one token and splits names", () => {
    expect(estimateTokens("the repo")).toBe(2);
    expect(estimateTokens("Supabase")).toBe(3);
    expect(estimateTokens("whisper.cpp")).toBe(3);
    expect(estimateTokens("")).toBe(0);
  });
});

describe("character limit", () => {
  it("leaves room for live transcription with a full, name-heavy hint", () => {
    // The suggested hint repeated to the limit: the densest real hint we have.
    const full = SUGGESTED.repeat(2).slice(0, HINT_CHAR_LIMIT);
    expect(estimateTokens(full) + CONTEXT_TOKENS).toBeLessThanOrEqual(PROMPT_TOKEN_LIMIT);
  });

  it("keeps the front when trimming, at a whole term", () => {
    const terms = Array.from({ length: 120 }, (_, i) => `Term${i}`).join(", ");
    const trimmed = trimHintToChars(terms);
    expect(trimmed.startsWith("Term0, Term1")).toBe(true);
    expect(trimmed.length).toBeLessThanOrEqual(HINT_CHAR_LIMIT);
    expect(trimmed).toMatch(/Term\d+$/);
    expect(trimHintToChars("short")).toBe("short");
  });

  it("turns a list into a sentence", () => {
    expect(hintAsSentence("Tauri, Claude, worktree")).toBe(
      "My notes mention Tauri, Claude and worktree.",
    );
    expect(hintAsSentence("  ")).toBe("");
  });
});

describe("replacement sets", () => {
  it("finds the entries you don't have yet, ignoring case", () => {
    const missing = missingFrom(
      [{ from: "Clawed", to: "Claude" }],
      [
        { from: "clawed", to: "Claude" },
        { from: "superbase", to: "Supabase" },
      ],
    );
    expect(missing).toEqual([{ from: "superbase", to: "Supabase" }]);
  });

  it("reads every separator, and drops bullets, numbering and quotes", () => {
    const { pairs, errors } = parseReplacementLines(
      [
        "nate builds -> n8builds",
        "superbase => Supabase",
        "versal → Vercel",
        "monoripo = monorepo",
        "clawed, Claude",
        "tari\tTauri",
        "- cloud md -> CLAUDE.md",
        "2. shared cn -> shadcn",
        '"cloud code" -> "Claude Code"',
        "",
        "# a comment",
        "// another",
        "not a rule",
      ].join("\n"),
    );
    expect(pairs).toEqual([
      { from: "nate builds", to: "n8builds" },
      { from: "superbase", to: "Supabase" },
      { from: "versal", to: "Vercel" },
      { from: "monoripo", to: "monorepo" },
      { from: "clawed", to: "Claude" },
      { from: "tari", to: "Tauri" },
      { from: "cloud md", to: "CLAUDE.md" },
      { from: "shared cn", to: "shadcn" },
      { from: "cloud code", to: "Claude Code" },
    ]);
    expect(errors).toEqual([{ line: 13, text: "not a rule" }]);
  });

  it("prefers an arrow over a comma, so a phrase can hold one", () => {
    expect(parseReplacementLines("my email, work -> me@example.com").pairs).toEqual([
      { from: "my email, work", to: "me@example.com" },
    ]);
  });

  it("copies out in the same format it reads", () => {
    const rows = [
      { from: "nate builds", to: "n8builds" },
      { from: "cloud code", to: "Claude Code" },
    ];
    const text = formatReplacementLines([...rows, { from: " ", to: "" }]);
    expect(parseReplacementLines(text).pairs).toEqual(rows);
  });
});

describe("model guide", () => {
  it("orders Tiny < Base < Small < Medium < Large on accuracy", () => {
    const accuracy = ["tiny.en", "base.en", "small.en", "medium.en", "large-v3-turbo"].map(
      (id) => modelGuide(id, "whisper")?.accuracy,
    );
    expect(accuracy).toEqual([1, 2, 3, 4, 5]);
    expect(modelGuide("small.en-q5_1", "whisper")?.recommended).toBe(true);
    expect(modelGuide("parakeet-tdt-0.6b-v3", "parakeet")?.speed).toBe(5);
  });
});
