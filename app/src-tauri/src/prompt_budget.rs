//! Fits the context hint (and live transcription's running context) into
//! the prompt tokens whisper.cpp actually reads.
//!
//! whisper.cpp keeps only the LAST `n_text_ctx / 2 - 1` = 223 prompt tokens
//! (src/whisper.cpp in v1.9.1, `max_prompt_ctx` and `n_take1`), so a long hint
//! silently loses its first terms, the ones people put first because they
//! matter most. Scribe trims instead, keeping the front of the hint, and in
//! live transcription gives the running context only the room that is left,
//! so the hint is never pushed out.
//!
//! Token counts are a safe estimate tuned against whisper.cpp's tokenizer;
//! keep it in step with `estimateTokens` in src/lib/promptBudget.ts.

/// Prompt tokens whisper.cpp reads.
pub const PROMPT_TOKEN_LIMIT: usize = 223;
/// Characters of the running transcript live transcription offers as
/// context when there is room.
pub const CONTEXT_MAX_CHARS: usize = 200;

/// A safe (high) estimate of the Whisper tokens in `text`.
pub fn estimate_tokens(text: &str) -> usize {
    let mut tokens = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_alphabetic() {
            let mut word = String::from(c);
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphabetic() {
                    word.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            let lower = word.chars().all(|ch| ch.is_ascii_lowercase());
            tokens += if lower && word.len() <= 8 {
                1
            } else {
                word.len().div_ceil(3).max(1)
            };
        } else if c.is_ascii_digit() {
            let mut len: usize = 1;
            while chars.peek().is_some_and(|next| next.is_ascii_digit()) {
                chars.next();
                len += 1;
            }
            tokens += len.div_ceil(3);
        } else if !c.is_whitespace() {
            tokens += 1;
        }
    }
    tokens
}

/// The front of `hint` that fits in `budget` tokens, cut after a whole term
/// (at a comma, a line break or a space), so whisper.cpp never has to drop
/// the start of it.
pub fn fit_front(hint: &str, budget: usize) -> &str {
    let hint = hint.trim();
    if estimate_tokens(hint) <= budget {
        return hint;
    }
    let mut best = 0;
    for (index, c) in hint.char_indices() {
        if matches!(c, ',' | '\n' | ';' | ' ') {
            if estimate_tokens(&hint[..index]) > budget {
                break;
            }
            best = index;
        }
    }
    hint[..best].trim_end_matches([',', ';', ' ', '\n'])
}

/// The prompt for one live transcription segment: the hint (trimmed to fit)
/// first, then as much of the end of the running transcript as the remaining
/// room allows, at most [`CONTEXT_MAX_CHARS`]. The context stays next to the
/// new audio, where it helps continuity most, and it can never push the
/// hint out because it only gets the tokens the hint left.
pub fn prompt_with_context(hint: &str, accumulated: &str) -> String {
    let hint = fit_front(hint, PROMPT_TOKEN_LIMIT);
    let room = PROMPT_TOKEN_LIMIT
        .saturating_sub(estimate_tokens(hint))
        // The space between them.
        .saturating_sub(1);
    let context = context_tail(accumulated.trim(), room);
    match (hint.is_empty(), context.is_empty()) {
        (true, true) => String::new(),
        (false, true) => hint.to_string(),
        (true, false) => context.to_string(),
        (false, false) => format!("{} {}", hint, context),
    }
}

/// The longest end of `text`, at most [`CONTEXT_MAX_CHARS`] characters and
/// `budget` tokens, starting at a word where the text has spaces.
fn context_tail(text: &str, budget: usize) -> &str {
    if budget == 0 {
        return "";
    }
    let start = text
        .char_indices()
        .rev()
        .nth(CONTEXT_MAX_CHARS - 1)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let mut tail = &text[start..];
    if start > 0 {
        // Don't start mid-word.
        if let Some((_, rest)) = tail.split_once(' ') {
            if !rest.is_empty() {
                tail = rest;
            }
        }
    }
    while estimate_tokens(tail) > budget {
        tail = match tail.split_once(' ') {
            Some((_, rest)) => rest,
            // No spaces (Chinese, Japanese): drop a character at a time.
            None => {
                let mut chars = tail.char_indices();
                chars.next();
                chars.next().map(|(index, _)| &tail[index..]).unwrap_or("")
            }
        };
    }
    tail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_like_the_whisper_tokenizer_or_a_little_higher() {
        // Real counts from whisper.cpp v1.9.1 with base.en: 129 and 137.
        let focused = "Notes on n8builds, Appturnity and T-Hub, with Claude Code";
        assert!(estimate_tokens(focused) >= 15);
        assert_eq!(estimate_tokens("the repo"), 2);
        assert_eq!(estimate_tokens("Supabase"), 3);
        assert_eq!(estimate_tokens("whisper.cpp"), 3);
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn a_short_hint_is_kept_whole() {
        assert_eq!(fit_front("  Tauri, Claude  ", 223), "Tauri, Claude");
    }

    #[test]
    fn a_long_hint_keeps_its_front() {
        let terms: Vec<String> = (0..200).map(|i| format!("Term{i}")).collect();
        let hint = terms.join(", ");
        let fitted = fit_front(&hint, PROMPT_TOKEN_LIMIT);
        assert!(fitted.starts_with("Term0, Term1, Term2"));
        assert!(estimate_tokens(fitted) <= PROMPT_TOKEN_LIMIT);
        assert!(!fitted.ends_with(','));
        assert!(!fitted.contains("Term199"));
    }

    #[test]
    fn context_follows_the_hint_and_never_pushes_it_out() {
        let terms: Vec<String> = (0..40).map(|i| format!("Term{i}")).collect();
        let hint = terms.join(", ");
        let said = "and then we deployed it to the staging server ".repeat(10);
        let prompt = prompt_with_context(&hint, &said);
        assert!(prompt.starts_with("Term0, "));
        assert!(prompt.contains("Term39"));
        assert!(estimate_tokens(&prompt) <= PROMPT_TOKEN_LIMIT);
    }

    #[test]
    fn context_gets_up_to_200_characters_when_the_hint_is_short() {
        let said = "word ".repeat(100);
        let prompt = prompt_with_context("Tauri", &said);
        let context = prompt.strip_prefix("Tauri ").unwrap();
        assert!(context.len() <= CONTEXT_MAX_CHARS);
        assert!(context.len() > 150);
    }

    #[test]
    fn no_room_means_no_context() {
        let terms: Vec<String> = (0..200).map(|i| format!("Term{i}")).collect();
        let prompt = prompt_with_context(&terms.join(", "), "we said things");
        assert!(!prompt.contains("we said things"));
    }

    #[test]
    fn text_without_spaces_keeps_its_last_200_characters() {
        let prompt = prompt_with_context("", &"\u{e9}".repeat(300));
        assert_eq!(prompt.chars().count(), 200);
    }

    #[test]
    fn either_part_alone() {
        assert_eq!(prompt_with_context("", ""), "");
        assert_eq!(prompt_with_context("Tauri", ""), "Tauri");
        assert_eq!(prompt_with_context("", "hello there"), "hello there");
    }
}
