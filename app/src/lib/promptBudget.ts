// Custom words (the context hint) and what Whisper can read of them.
//
// whisper.cpp reads at most 223 prompt tokens and drops the rest; live
// transcription also adds the end of what you just said, about 45 tokens.
// People see one fixed character limit instead of tokens. Measured with
// whisper.cpp's tokenizer, hints run 2.9 (a sentence full of names) to 3.7
// (a plain list) characters per token, so 450 characters is at most about
// 155 tokens, which leaves room for that context. The backend still trims by
// tokens as a safety net (src-tauri/src/prompt_budget.rs), keeping the front.

/** The custom words limit, in characters. */
export const HINT_CHAR_LIMIT = 450;

/** Prompt tokens whisper.cpp reads. */
export const PROMPT_TOKEN_LIMIT = 223;

const PIECE = /[A-Za-z]+|\d+|[^\sA-Za-z\d]/g;

/** A safe (high) estimate of how many Whisper tokens `text` takes. Tuned
 * against whisper.cpp's tokenizer: never under, at most about 15% over. Keep
 * in step with `estimate_tokens` in prompt_budget.rs. */
export function estimateTokens(text: string): number {
  let tokens = 0;
  for (const piece of text.match(PIECE) ?? []) {
    if (/^[A-Za-z]+$/.test(piece)) {
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

/** The terms of a hint: comma, line or "and" separated. */
export function hintTerms(text: string): string[] {
  return text
    .split(/,|\n|;|\band\b/)
    .map((term) => term.replace(/^[\s.:]+|[\s.:]+$/g, ""))
    .filter((term) => term.length > 0);
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

/** The front of `text` that fits in `limit` characters, cut after a whole
 * term, for "Keep what fits". */
export function trimHintToChars(text: string, limit = HINT_CHAR_LIMIT): string {
  const trimmed = text.trim();
  if (trimmed.length <= limit) {
    return trimmed;
  }
  const head = trimmed.slice(0, limit + 1);
  const cut = Math.max(head.lastIndexOf(","), head.lastIndexOf(" "));
  return (cut > 0 ? head.slice(0, cut) : head.slice(0, limit)).replace(
    /[\s,;]+$/,
    "",
  );
}
