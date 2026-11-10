import { describe, expect, it } from "vitest";
import {
  analyzeHint,
  estimateTokens,
  hintAsSentence,
  HINT_TOKEN_BUDGET,
  PROMPT_TOKEN_LIMIT,
  trimHintToBudget,
} from "./promptBudget";
import { DEVELOPER_REPLACEMENTS, missingFrom, parseReplacementLines } from "./replacementSets";
import { modelGuide } from "./modelGuide";

// whisper.cpp's tokenizer (base.en) counts this hint as 137 tokens.
const SUGGESTED =
  "Nate's dev notes on n8builds, Appturnity, T-Hub, Scribe, Solara, Wren, PR Post, N8 Tray and pass-env, working with Claude, Claude Code, Codex, ChatGPT and Gemini: GitHub, GitHub Actions, worktree, monorepo, repo, PR, rebase, cherry-pick, CI, pnpm, Tauri, Supabase, Postgres, Drizzle, Cloudflare, wrangler, Vercel, Next.js, TypeScript, Tailwind, shadcn, whisper.cpp, Parakeet, WSL, localhost, subagent.";

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

describe("hint budget", () => {
  it("fits a 40-term sentence with room for live transcription", () => {
    const budget = analyzeHint(SUGGESTED);
    expect(budget.overBudget).toBe(false);
    expect(budget.dropped).toEqual([]);
    expect(HINT_TOKEN_BUDGET).toBeLessThan(PROMPT_TOKEN_LIMIT);
  });

  it("names the terms at the end that Whisper never sees", () => {
    const terms = Array.from({ length: 120 }, (_, i) => `Term${i}`);
    const budget = analyzeHint(terms.join(", "));
    expect(budget.overLimit).toBe(true);
    expect(budget.dropped[0]).not.toBe("Term0");
    expect(budget.dropped.at(-1)).toBe("Term119");
  });

  it("keeps the front when trimming", () => {
    const terms = Array.from({ length: 120 }, (_, i) => `Term${i}`);
    const trimmed = trimHintToBudget(terms.join(", "));
    expect(trimmed.startsWith("Term0, Term1")).toBe(true);
    expect(estimateTokens(trimmed)).toBeLessThanOrEqual(HINT_TOKEN_BUDGET);
  });

  it("turns a list into a sentence", () => {
    expect(hintAsSentence("Tauri, Claude, worktree")).toBe(
      "My notes mention Tauri, Claude and worktree.",
    );
    expect(hintAsSentence("  ")).toBe("");
  });
});

describe("replacement sets", () => {
  it("offers only the fixes you don't have yet", () => {
    const missing = missingFrom([{ from: "Clawed", to: "Claude" }], DEVELOPER_REPLACEMENTS);
    expect(missing.some((row) => row.from === "clawed")).toBe(false);
    expect(missing.length).toBe(DEVELOPER_REPLACEMENTS.length - 1);
  });

  it("reads pasted lines with ->, => or a tab", () => {
    expect(
      parseReplacementLines("nate builds -> n8builds\nsuperbase => Supabase\nwork tree\tworktree\nnot a rule"),
    ).toEqual([
      { from: "nate builds", to: "n8builds" },
      { from: "superbase", to: "Supabase" },
      { from: "work tree", to: "worktree" },
    ]);
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
