//! Deterministic "say X -> get Y" text replacements applied to the final
//! transcript after Whisper. Unlike the vocabulary prompt (a soft recognition
//! bias passed to whisper.cpp via `--prompt`), this layer is a literal
//! find-and-replace the user controls exactly.

use serde::{Deserialize, Serialize};

/// One user-defined replacement: whenever `from` is spoken (matched
/// case-insensitively on word boundaries), the transcript text is rewritten to
/// `to` verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextReplacement {
    pub from: String,
    pub to: String,
}

/// Applies every replacement to `text` and returns the rewritten string.
///
/// Semantics:
/// - **Case-insensitive**: `from` matches regardless of letter case.
/// - **Word-boundary-aware**: a match counts only when the character before the
///   match start is the string start or a non-alphanumeric, and the character
///   after the match end is the string end or non-alphanumeric. So
///   `from = "no"` rewrites the standalone word `no` but never the `no` inside
///   `nope`.
/// - **Longest-`from`-first**: replacements are applied in order of descending
///   `from` length, so a longer phrase wins over a shorter one it contains
///   (e.g. `"my email"` is replaced before `"email"`).
/// - Entries whose `from` is empty or whitespace-only are skipped.
/// - Each replacement is applied to the *result* of the previous one, and all
///   matching occurrences are replaced.
///
/// Implemented manually (no regex / no extra crate). The scan runs over the
/// ORIGINAL text and compares case-insensitively per character, so it never
/// mixes byte offsets between a lowercased copy and the original (which would be
/// unsound: `char::to_lowercase` is not length-preserving, e.g. `İ`).
pub fn apply(text: &str, replacements: &[TextReplacement]) -> String {
    // Longest `from` first; skip empty/whitespace-only entries. Sorting a
    // clone of the references keeps the caller's Vec order untouched.
    let mut ordered: Vec<&TextReplacement> = replacements
        .iter()
        .filter(|r| !r.from.trim().is_empty())
        .collect();
    ordered.sort_by(|a, b| b.from.chars().count().cmp(&a.from.chars().count()));

    let mut result = text.to_string();
    for replacement in ordered {
        result = replace_word_ci(&result, &replacement.from, &replacement.to);
    }
    result
}

/// Replaces all case-insensitive, word-boundary-delimited occurrences of
/// `needle` in `haystack` with `replacement`. `needle` is assumed non-empty
/// (callers filter empties out).
///
/// The scan walks the ORIGINAL `haystack` one char at a time. At every position
/// it tries to match `needle` char-by-char (lowercasing both sides per char)
/// and, on a content match, verifies the word boundaries using
/// `char::is_alphanumeric` (Unicode-aware): the char just before the match start
/// and just after the match end must each be a string edge or non-alphanumeric.
/// On a whole-word match it appends `replacement` verbatim and jumps past the
/// matched region; otherwise it copies the single current char and advances by
/// one char. Everything operates on the original chars and the chars themselves
/// are pushed onto the output, so no byte offset is ever taken from a lowercased
/// copy (which would be unsound — `to_lowercase` can change a char's byte width).
fn replace_word_ci(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }

    let mut out = String::with_capacity(haystack.len());
    // The original chars, in order. Indexing by char position lets us look at
    // the preceding char for the left-boundary check without ever slicing the
    // string by a byte offset derived from a lowercased copy.
    let chars: Vec<char> = haystack.chars().collect();
    let needle_chars: Vec<char> = needle.chars().collect();

    let mut i = 0usize; // current char index into `chars`
    while i < chars.len() {
        if let Some(match_end_char_idx) = match_needle_at(&chars, i, &needle_chars) {
            // Left boundary: the char immediately before the match start.
            let before_ok = match i.checked_sub(1) {
                None => true,
                Some(prev) => !chars[prev].is_alphanumeric(),
            };
            // Right boundary: the char immediately after the match end.
            let after_ok = match chars.get(match_end_char_idx) {
                None => true,
                Some(ch) => !ch.is_alphanumeric(),
            };

            if before_ok && after_ok {
                out.push_str(replacement);
                i = match_end_char_idx; // jump past the matched run
                continue;
            }
        }

        // No whole-word match here: copy the single current char and advance.
        out.push(chars[i]);
        i += 1;
    }

    out
}

/// If `needle_chars` matches the chars of `chars` starting at char index
/// `start` (compared case-insensitively, per char), returns the char index one
/// past the match; otherwise `None`. Per-char lowercasing on both sides keeps
/// the comparison case-insensitive without allocating a lowercased haystack.
fn match_needle_at(chars: &[char], start: usize, needle_chars: &[char]) -> Option<usize> {
    if start + needle_chars.len() > chars.len() {
        return None;
    }
    for (offset, &needle_ch) in needle_chars.iter().enumerate() {
        if !chars_eq_ci(chars[start + offset], needle_ch) {
            return None;
        }
    }
    Some(start + needle_chars.len())
}

/// Case-insensitive single-char comparison. Lowercasing a `char` can expand to
/// multiple chars (e.g. `İ`), so compare the full lowercase iterators rather
/// than assuming a 1:1 mapping.
fn chars_eq_ci(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    a.to_lowercase().eq(b.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(from: &str, to: &str) -> TextReplacement {
        TextReplacement {
            from: from.to_string(),
            to: to.to_string(),
        }
    }

    #[test]
    fn replaces_standalone_word() {
        assert_eq!(
            apply("I don't no", &[rep("no", "know")]),
            "I don't know"
        );
    }

    #[test]
    fn does_not_replace_inside_word() {
        // "no" inside "nope" must be left alone.
        assert_eq!(apply("nope", &[rep("no", "know")]), "nope");
    }

    #[test]
    fn is_case_insensitive() {
        assert_eq!(apply("NO", &[rep("no", "know")]), "know");
    }

    #[test]
    fn longest_from_wins() {
        // The longer "my email" must be used over the shorter "email".
        assert_eq!(
            apply(
                "send my email",
                &[rep("my email", "a@b.com"), rep("email", "mail")]
            ),
            "send a@b.com"
        );
    }

    #[test]
    fn empty_from_is_ignored() {
        assert_eq!(apply("hello", &[rep("", "X"), rep("  ", "Y")]), "hello");
    }

    #[test]
    fn empty_list_returns_input_unchanged() {
        assert_eq!(apply("unchanged text", &[]), "unchanged text");
    }

    #[test]
    fn replaces_all_occurrences() {
        assert_eq!(
            apply("no no no", &[rep("no", "know")]),
            "know know know"
        );
    }

    #[test]
    fn replacement_is_verbatim_and_keeps_punctuation() {
        // Punctuation is a boundary; the "to" is inserted verbatim.
        assert_eq!(
            apply("clawed, clawed.", &[rep("clawed", "Claude")]),
            "Claude, Claude."
        );
    }

    #[test]
    fn chains_replacements_on_previous_result() {
        // First "a" -> "b" everywhere, then "b" -> "c": the second sees the
        // output of the first, so a standalone "a" becomes "c".
        assert_eq!(apply("a", &[rep("a", "b"), rep("b", "c")]), "c");
    }

    #[test]
    fn unicode_uppercase_before_match_does_not_panic() {
        // `İ` (U+0130) lowercases to TWO chars, so the old "find offsets in a
        // lowercased copy, slice the original" approach produced invalid byte
        // offsets and panicked / garbled the output. The char-aware scan must
        // handle it: the leading `İ` is a word boundary, so "no" is replaced.
        assert_eq!(apply("İ no", &[rep("no", "know")]), "İ know");
    }

    #[test]
    fn multibyte_from_is_matched_on_word_boundaries() {
        // A multibyte `from` ("café") is matched case-insensitively as a whole
        // word and left alone inside a larger word ("cafés" is not "café").
        assert_eq!(
            apply("CAFÉ time", &[rep("café", "coffee")]),
            "coffee time"
        );
        assert_eq!(apply("cafés", &[rep("café", "coffee")]), "cafés");
    }

    #[test]
    fn multibyte_to_is_inserted_verbatim() {
        // A multibyte replacement is appended byte-for-byte.
        assert_eq!(apply("euro sign", &[rep("euro", "€")]), "€ sign");
    }

    #[test]
    fn unicode_text_around_match_is_preserved() {
        // Non-replaced multibyte text on both sides keeps its exact casing/bytes.
        assert_eq!(
            apply("naïve no façade", &[rep("no", "know")]),
            "naïve know façade"
        );
    }
}
