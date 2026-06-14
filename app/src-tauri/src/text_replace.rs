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
/// Implemented manually (no regex / no extra crate). Matching is done on the
/// lowercased haystack so the comparison is case-insensitive, while the
/// original `text` supplies the surrounding characters that are kept verbatim.
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
fn replace_word_ci(haystack: &str, needle: &str, replacement: &str) -> String {
    let haystack_lower = haystack.to_lowercase();
    let needle_lower = needle.to_lowercase();
    let needle_len = needle_lower.len();
    if needle_len == 0 {
        return haystack.to_string();
    }

    let mut out = String::with_capacity(haystack.len());
    // Byte cursor into the ORIGINAL haystack; lowercasing here is ASCII-stable
    // for the boundaries we care about, and we only ever copy original bytes.
    let mut cursor = 0usize;

    while let Some(rel) = haystack_lower[cursor..].find(&needle_lower) {
        let start = cursor + rel;
        let end = start + needle_len;

        // Boundary check against the lowercased haystack: the char immediately
        // before `start` and immediately after `end` must each be the
        // string edge or a non-alphanumeric.
        let before_ok = match haystack_lower[..start].chars().next_back() {
            None => true,
            Some(ch) => !ch.is_alphanumeric(),
        };
        let after_ok = match haystack_lower[end..].chars().next() {
            None => true,
            Some(ch) => !ch.is_alphanumeric(),
        };

        if before_ok && after_ok {
            out.push_str(&haystack[cursor..start]);
            out.push_str(replacement);
            cursor = end;
        } else {
            // Not a whole-word match: keep the text up to and including the
            // first char of this candidate, then resume the search after it so
            // overlapping candidates are still found.
            let next = start + next_char_len(&haystack_lower[start..]);
            out.push_str(&haystack[cursor..next]);
            cursor = next;
        }
    }

    out.push_str(&haystack[cursor..]);
    out
}

/// Byte length of the first char of `s` (1 when `s` is empty, to guarantee
/// forward progress in the scan loop).
fn next_char_len(s: &str) -> usize {
    s.chars().next().map(|c| c.len_utf8()).unwrap_or(1)
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
}
