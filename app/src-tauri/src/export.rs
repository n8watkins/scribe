//! Local export of transcripts to Markdown, CSV, or JSON. These are pure
//! renderers over a slice of `Transcript`: no I/O, no settings — the
//! `export_transcripts` command (commands.rs) fetches the rows, picks one of
//! these, and writes the result through the native save-file dialog. Keeping
//! them pure makes the formatting trivially unit-testable.

use crate::transcript::Transcript;

/// Renders the transcripts as a single Markdown document: an H1 title, then one
/// section per transcript with a local-time heading, a small metadata line, the
/// text, and (for notes that have one) the analysis as a blockquote.
pub fn to_markdown(transcripts: &[Transcript]) -> String {
    let mut out = String::from("# Scribe export\n\n");
    out.push_str(&format!(
        "_{} transcript{} exported._\n",
        transcripts.len(),
        if transcripts.len() == 1 { "" } else { "s" }
    ));

    for transcript in transcripts {
        out.push_str("\n---\n\n");
        let kind = if transcript.is_note { "Note" } else { "Dictation" };
        out.push_str(&format!(
            "## {} — {}\n\n",
            kind,
            format_local(transcript.created_at)
        ));
        out.push_str(transcript.text.trim());
        out.push('\n');

        if let Some(analysis) = transcript
            .analysis
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            out.push('\n');
            for line in analysis.lines() {
                out.push_str(&format!("> {line}\n"));
            }
        }
        out.push('\n');
    }
    out
}

/// Renders the transcripts as RFC-4180 CSV with a header row. Every field is
/// run through `csv_field`, which quotes and escapes any value containing a
/// comma, double-quote, or newline, so the output round-trips through any
/// spreadsheet importer.
pub fn to_csv(transcripts: &[Transcript]) -> String {
    let mut out = String::new();
    // RFC 4180 uses CRLF line endings throughout, including after the header.
    out.push_str("id,created_at,is_note,word_count,character_count,language,text,analysis\r\n");
    for transcript in transcripts {
        let row = [
            transcript.id.as_str(),
            transcript.created_at.to_rfc3339().as_str(),
            if transcript.is_note { "true" } else { "false" },
            transcript.word_count.to_string().as_str(),
            transcript.character_count.to_string().as_str(),
            transcript.language.as_deref().unwrap_or(""),
            transcript.text.as_str(),
            transcript.analysis.as_deref().unwrap_or(""),
        ]
        .iter()
        .map(|field| csv_field(field))
        .collect::<Vec<_>>()
        .join(",");
        out.push_str(&row);
        // RFC 4180 uses CRLF line endings; spreadsheet apps expect them.
        out.push_str("\r\n");
    }
    out
}

/// Renders the transcripts as pretty-printed JSON — the full `Transcript`
/// shape (camelCase, matching the rest of the wire format), so a JSON export
/// is a lossless dump of the rows.
pub fn to_json(transcripts: &[Transcript]) -> String {
    // The Transcript derive is infallible to serialize (no maps with non-string
    // keys, no custom errors), so this never fails in practice; fall back to an
    // empty array rather than panicking if it somehow does.
    serde_json::to_string_pretty(transcripts).unwrap_or_else(|_| "[]".to_string())
}

/// Quotes and escapes a single CSV field per RFC 4180: a field is wrapped in
/// double quotes when it contains a comma, double-quote, or CR/LF, and any
/// embedded double-quote is doubled.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Local-time stamp like "2026-06-12 18:30" for human-readable headings.
fn format_local(at: chrono::DateTime<chrono::Utc>) -> String {
    at.with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    /// A dictation transcript pinned to a known instant.
    fn dictation(id: &str, text: &str) -> Transcript {
        let mut t = Transcript::new_last_buffer(text, Some(1000), None, Some("en".into())).unwrap();
        t.id = id.to_string();
        t.created_at = Utc.with_ymd_and_hms(2026, 6, 12, 18, 30, 0).unwrap();
        t
    }

    /// A note with an analysis summary.
    fn note(id: &str, text: &str, analysis: &str) -> Transcript {
        let mut t = dictation(id, text);
        t.is_note = true;
        t.analysis = Some(analysis.to_string());
        t
    }

    /// A row whose text exercises every CSV-escaping case at once: a comma, a
    /// double-quote, and a newline.
    fn gnarly() -> Transcript {
        dictation("tx_gnarly", "he said \"hi, there\"\nthen left")
    }

    #[test]
    fn markdown_includes_text_kind_and_analysis() {
        let rows = vec![
            dictation("tx_1", "buy milk"),
            note("tx_2", "call Sam", "Summary: a call"),
        ];
        let md = to_markdown(&rows);

        assert!(md.starts_with("# Scribe export"));
        assert!(md.contains("2 transcripts exported"));
        assert!(md.contains("## Dictation"));
        assert!(md.contains("buy milk"));
        // The note is labeled and its analysis renders as a blockquote.
        assert!(md.contains("## Note"));
        assert!(md.contains("call Sam"));
        assert!(md.contains("> Summary: a call"));
    }

    #[test]
    fn markdown_singular_count_for_one_row() {
        let md = to_markdown(&[dictation("tx_1", "solo")]);
        assert!(md.contains("1 transcript exported."));
        assert!(!md.contains("transcripts exported"));
    }

    #[test]
    fn csv_has_header_and_quotes_special_characters() {
        let rows = vec![
            note("tx_note", "a note", "did stuff"),
            gnarly(),
        ];
        let csv = to_csv(&rows);

        let mut lines = csv.split("\r\n");
        assert_eq!(
            lines.next().unwrap(),
            "id,created_at,is_note,word_count,character_count,language,text,analysis"
        );

        // The note row: is_note true, plain (un-quoted) simple fields.
        let note_line = lines.next().unwrap();
        assert!(note_line.starts_with("tx_note,"));
        assert!(note_line.contains(",true,"));
        assert!(note_line.contains("a note"));
        assert!(note_line.contains("did stuff"));

        // The gnarly row's text field is quoted, its inner quotes doubled, and
        // its embedded newline is preserved inside the quotes.
        let gnarly_line = lines.next().unwrap();
        assert!(gnarly_line.contains(",false,"));
        assert!(gnarly_line.contains("\"he said \"\"hi, there\"\"\nthen left\""));
    }

    #[test]
    fn csv_field_escaping_matches_rfc_4180() {
        // A plain field is left bare.
        assert_eq!(csv_field("plain"), "plain");
        // A comma forces quoting.
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        // A double-quote forces quoting and is itself doubled.
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        // A newline (and a bare CR) forces quoting, preserved inside the quotes.
        assert_eq!(csv_field("line1\nline2"), "\"line1\nline2\"");
        assert_eq!(csv_field("line1\rline2"), "\"line1\rline2\"");
        // Empty stays empty (an absent optional column).
        assert_eq!(csv_field(""), "");
    }

    #[test]
    fn json_is_pretty_and_lossless() {
        let rows = vec![note("tx_2", "call Sam", "Summary: a call"), gnarly()];
        let json = to_json(&rows);

        // Pretty-printed (indented, multi-line).
        assert!(json.contains("\n  "));
        // Round-trips back to the same transcripts.
        let parsed: Vec<Transcript> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rows);
        // camelCase field names (matches the rest of the wire format).
        assert!(json.contains("\"isNote\""));
        assert!(json.contains("\"createdAt\""));
    }

    #[test]
    fn empty_input_renders_each_format_cleanly() {
        assert!(to_markdown(&[]).contains("0 transcripts exported"));
        // CSV is just the header row.
        assert_eq!(
            to_csv(&[]),
            "id,created_at,is_note,word_count,character_count,language,text,analysis\r\n"
        );
        assert_eq!(to_json(&[]), "[]");
    }
}
