// The custom words (context hint) budget. whisper.cpp reads at most 223 prompt
// tokens (n_text_ctx / 2 minus one start token) and drops the rest. Scribe
// keeps the front of the hint (src-tauri/src/prompt_budget.rs), so the terms
// at the end are the ones that go. Live transcription also adds the last
// thing you said, about 45 tokens, so the hint should stay under 165.
//
// Token counts are an estimate tuned against whisper.cpp's own tokenizer on
// real hints: it never came in under the true count and is at most about 15%
// over. Keep in step with `estimate_tokens` in prompt_budget.rs.

/** Prompt tokens whisper.cpp reads. */
export const PROMPT_TOKEN_LIMIT = 223;
/** Room the live transcription context needs, in tokens. */
export const CONTEXT_RESERVE_TOKENS = 58;
/** The budget the hint should stay inside. */
export const HINT_TOKEN_BUDGET = PROMPT_TOKEN_LIMIT - CONTEXT_RESERVE_TOKENS;

const PIECE = /[A-Za-z]+|\d+|[^\sA-Za-z\d]/g;

/** A safe (high) estimate of how many Whisper tokens `text` takes. */
export function estimateTokens(text: string): number {
  let tokens = 0;
  for (const piece of text.match(PIECE) ?? []) {
    if (/^[A-Za-z]+$/.test(piece)) {
      // Short lowercase words are usually one token; names, capitals and
      // jargon split into pieces of about three letters.
      tokens +=
        piece === piece.toLowerCase() && piece.length <= 8
          ? 1
          : Math.max(1, Math.ceil(piece.length / 3));
    } else if (/^\d+$/.test(piece)) {
      tokens += Math.ceil(piece.length / 3);
    } else {
      tokens += 1;
    }
  }
  return tokens;
}

/** The terms of a hint: comma or line separated, or the words of a sentence
 * split on commas and "and". */
export function hintTerms(text: string): string[] {
  return text
    .split(/,|\n|;|\band\b/)
    .map((term) => term.replace(/^[\s.:]+|[\s.:]+$/g, ""))
    .filter((term) => term.length > 0);
}

export type HintBudget = {
  tokens: number;
  terms: number;
  /** Over the budget that leaves room for live transcription. */
  overBudget: boolean;
  /** Over what Whisper reads at all: the dropped terms never reach it. */
  overLimit: boolean;
  /** The terms Scribe leaves out because they don't fit, in order. */
  dropped: string[];
};

/** How a hint fits the budget, and which terms at the end don't fit. */
export function analyzeHint(text: string): HintBudget {
  const trimmed = text.trim();
  const tokens = estimateTokens(trimmed);
  const terms = hintTerms(trimmed);
  const dropped: string[] = [];
  if (tokens > PROMPT_TOKEN_LIMIT) {
    // Walk from the front, the way Scribe trims: whatever starts past the
    // limit is dropped.
    let used = 0;
    let cut = false;
    for (const term of terms) {
      used += estimateTokens(`${term}, `);
      if (cut || used > PROMPT_TOKEN_LIMIT) {
        cut = true;
        dropped.push(term);
      }
    }
  }
  return {
    tokens,
    terms: terms.length,
    overBudget: tokens > HINT_TOKEN_BUDGET,
    overLimit: tokens > PROMPT_TOKEN_LIMIT,
    dropped,
  };
}

/** Turns a comma list into one sentence, which Whisper follows better: a
 * list makes it write in list style (lowercase, commas). */
export function hintAsSentence(text: string): string {
  const terms = hintTerms(text);
  if (terms.length === 0) {
    return "";
  }
  const listed =
    terms.length === 1
      ? terms[0]
      : `${terms.slice(0, -1).join(", ")} and ${terms[terms.length - 1]}`;
  return `My notes mention ${listed}.`;
}

/** The terms that fit, for "Keep what fits". */
export function trimHintToBudget(text: string, budget = HINT_TOKEN_BUDGET): string {
  const kept: string[] = [];
  let used = 0;
  for (const term of hintTerms(text)) {
    const cost = estimateTokens(`${term}, `);
    if (used + cost > budget) {
      break;
    }
    kept.push(term);
    used += cost;
  }
  return kept.join(", ");
}
