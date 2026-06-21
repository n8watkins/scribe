//! Pause-aware filler suppression.
//!
//! Whisper hands us each word with millisecond start/end times (see the
//! timestamped parse in `whisper.rs`). A word from the user's filler list
//! ("um", "uh", …) is removed only when it is bracketed by a real silence — a
//! hesitation — and kept when it sits tight against its neighbours (fluent
//! speech like "**oh** no" or "**like** this"). The silence that qualifies is
//! the user's `filler_pause_threshold_ms`.
//!
//! This module is pure and timing-only so it is fully unit-testable off-Windows;
//! the (Windows-gated) timestamp capture lives in `whisper.rs`.

/// One word with its Whisper timing, in milliseconds from the clip start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedWord {
    /// The word as transcribed, keeping any trailing punctuation ("store.",
    /// "um,"). Matching against the filler list normalizes this.
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl TimedWord {
    pub fn new(text: impl Into<String>, start_ms: i64, end_ms: i64) -> Self {
        Self {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }
}

/// Lowercase + strip everything but letters/digits, so "Um," / "uh." match the
/// list entries "um" / "uh".
fn normalize_word(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Removes filler words that are flanked by a pause >= `threshold_ms`, returning
/// the kept words joined by single spaces. `filler_words` is matched
/// case/punctuation-insensitively; an empty list (or no qualifying pauses) leaves
/// the text untouched. A filler at the very start or end of the clip is treated
/// as having a pause on the missing side (a leading "Um, …" is a hesitation), so
/// it is removed too.
///
/// The caller feeds the result through `normalize_transcript_text`, which tidies
/// the spacing/punctuation left behind.
pub fn suppress_fillers(words: &[TimedWord], filler_words: &[String], threshold_ms: i64) -> String {
    if words.is_empty() {
        return String::new();
    }
    // Pre-normalize the list once.
    let fillers: Vec<String> = filler_words
        .iter()
        .map(|w| normalize_word(w))
        .filter(|w| !w.is_empty())
        .collect();

    let last = words.len() - 1;
    let mut kept: Vec<&str> = Vec::with_capacity(words.len());
    for (i, word) in words.iter().enumerate() {
        let normalized = normalize_word(&word.text);
        let is_filler = !normalized.is_empty() && fillers.iter().any(|f| *f == normalized);
        if is_filler {
            // Only existing neighbours count — a *missing* neighbour is NOT a
            // pause, or "oh no" (oh at the start, tight after) would lose its
            // "oh". Remove when the largest gap to a real neighbour meets the
            // threshold, or when the filler stands completely alone ("Um.").
            let gap_before = (i != 0).then(|| word.start_ms - words[i - 1].end_ms);
            let gap_after = (i != last).then(|| words[i + 1].start_ms - word.end_ms);
            let remove = match gap_before.into_iter().chain(gap_after).max() {
                Some(max_gap) => max_gap >= threshold_ms,
                None => true, // a lone filler utterance is just a hesitation
            };
            if remove {
                continue;
            }
        }
        kept.push(word.text.as_str());
    }
    kept.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fillers() -> Vec<String> {
        ["um", "uh", "er", "hmm", "like", "so", "oh"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn removes_filler_flanked_by_a_pause() {
        // "I went <pause> um <pause> to the store" -> the um is a hesitation.
        let words = vec![
            TimedWord::new("I", 0, 100),
            TimedWord::new("went", 110, 400),
            TimedWord::new("um", 900, 1050), // 500ms gap before, 450ms after
            TimedWord::new("to", 1500, 1600),
            TimedWord::new("the", 1610, 1700),
            TimedWord::new("store", 1710, 2000),
        ];
        assert_eq!(suppress_fillers(&words, &fillers(), 300), "I went to the store");
    }

    #[test]
    fn keeps_filler_word_used_fluently() {
        // "oh no" spoken tightly -> "oh" is real, not a hesitation.
        let words = vec![
            TimedWord::new("oh", 0, 150),
            TimedWord::new("no", 160, 400), // only 10ms gap
        ];
        assert_eq!(suppress_fillers(&words, &fillers(), 300), "oh no");
        // "I want it like this" with no gaps -> "like" stays.
        let fluent = vec![
            TimedWord::new("I", 0, 80),
            TimedWord::new("want", 90, 300),
            TimedWord::new("it", 310, 430),
            TimedWord::new("like", 440, 600),
            TimedWord::new("this", 610, 850),
        ];
        assert_eq!(suppress_fillers(&fluent, &fillers(), 300), "I want it like this");
    }

    #[test]
    fn removes_edge_filler_with_a_gap_but_not_a_tight_one() {
        // Leading "um" with a 400ms gap after, trailing "uh" with a 500ms gap
        // before -> both are hesitations and go.
        let words = vec![
            TimedWord::new("um", 0, 200),
            TimedWord::new("okay", 600, 900),
            TimedWord::new("uh", 1400, 1600),
        ];
        assert_eq!(suppress_fillers(&words, &fillers(), 300), "okay");
        // A leading "um" tight against the next word is kept (no hesitation).
        let tight = vec![
            TimedWord::new("um", 0, 200),
            TimedWord::new("okay", 230, 500),
        ];
        assert_eq!(suppress_fillers(&tight, &fillers(), 300), "um okay");
    }

    #[test]
    fn removes_a_lone_filler_utterance() {
        assert_eq!(suppress_fillers(&[TimedWord::new("Um.", 0, 300)], &fillers(), 300), "");
    }

    #[test]
    fn matches_ignoring_case_and_punctuation() {
        let words = vec![
            TimedWord::new("Well", 0, 200),
            TimedWord::new("Um,", 800, 1000), // 600ms gap before
            TimedWord::new("yes", 1500, 1800),
        ];
        assert_eq!(suppress_fillers(&words, &fillers(), 300), "Well yes");
    }

    #[test]
    fn empty_filler_list_is_a_noop() {
        let words = vec![
            TimedWord::new("um", 0, 200),
            TimedWord::new("hello", 900, 1200),
        ];
        assert_eq!(suppress_fillers(&words, &[], 300), "um hello");
    }

    #[test]
    fn threshold_controls_aggressiveness() {
        // 250ms gap on each side: removed at a 200ms threshold, kept at 300ms.
        let words = vec![
            TimedWord::new("yes", 0, 200),
            TimedWord::new("um", 450, 650),
            TimedWord::new("no", 900, 1100),
        ];
        assert_eq!(suppress_fillers(&words, &fillers(), 200), "yes no");
        assert_eq!(suppress_fillers(&words, &fillers(), 300), "yes um no");
    }
}
