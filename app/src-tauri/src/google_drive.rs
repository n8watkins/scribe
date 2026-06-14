//! Google Drive REST v3 helpers for the notes-sync feature. Everything here is
//! scoped to `drive.file` (app-created files only), so `files.list` only ever
//! sees folders/files this app made — which is exactly what lets us "find our
//! own folder" without broad Drive access.
//!
//! Content upload is done in two steps — create the file's metadata, then PATCH
//! its bytes via the media-upload endpoint — so we never have to assemble a
//! `multipart/related` body. The daily Markdown file is regenerated from the
//! local DB on each sync (read-modify-write with SQLite as the source of
//! truth), which keeps "Sync now" idempotent: syncing twice yields one file,
//! not duplicated entries.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{Datelike, Local, Timelike};
use serde::Serialize;
use serde_json::json;

use crate::error::CommandError;
use crate::transcript::Transcript;

const API_BASE: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";
const ROOT_FOLDER_NAME: &str = "Scribe Voice Notes";
/// Distinct root for the full-history transcript backup, kept separate from the
/// notes folder so the curated notes log and the raw transcript dump never mix.
const TRANSCRIPTS_ROOT_FOLDER_NAME: &str = "Scribe Transcripts";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    /// Number of note/transcript entries written across all daily files.
    pub synced_notes: u32,
    /// Number of distinct daily Markdown files created or updated.
    pub files_written: u32,
}

/// A handle to the Drive REST API for one access token. Base URLs are fields so
/// tests can point them at a local mock server.
pub struct Drive {
    client: reqwest::blocking::Client,
    token: String,
    api: String,
    upload: String,
}

impl Drive {
    pub fn new(access_token: impl Into<String>) -> Result<Self, CommandError> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .user_agent(concat!("Scribe/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|error| failure(error.to_string()))?,
            token: access_token.into(),
            api: API_BASE.to_string(),
            upload: UPLOAD_BASE.to_string(),
        })
    }

    /// Writes each note into its day's `YYYY-MM/YYYY-MM-DD.md` file, creating
    /// the root and month folders as needed. The daily file is rebuilt from the
    /// supplied notes (caller passes the full set for each day from the DB).
    pub fn sync_notes(&self, notes: &[Transcript]) -> Result<SyncReport, CommandError> {
        let root = self.ensure_folder(None, ROOT_FOLDER_NAME)?;

        // Group by the note's *local* calendar day so the file names match what
        // the owner sees on his clock, not UTC.
        let mut by_day: BTreeMap<String, Vec<&Transcript>> = BTreeMap::new();
        for note in notes {
            let day = note
                .created_at
                .with_timezone(&Local)
                .format("%Y-%m-%d")
                .to_string();
            by_day.entry(day).or_default().push(note);
        }

        let mut files_written = 0_u32;
        let mut synced_notes = 0_u32;
        for (day, mut day_notes) in by_day {
            day_notes.sort_by_key(|note| note.created_at);
            let content = render_daily(&day, &day_notes);
            self.write_into_month(&root, &day, &format!("{day}.md"), &content)?;

            files_written += 1;
            synced_notes += day_notes.len() as u32;
        }

        Ok(SyncReport { synced_notes, files_written })
    }

    /// Backs up *every* supplied transcript into a DISTINCT root folder
    /// ("Scribe Transcripts"), grouped by `YYYY-MM` month folders with one
    /// daily Markdown file per local calendar day — mirroring `sync_notes` but
    /// for the full-history dump rather than the curated notes log. The daily
    /// file is rebuilt from the supplied transcripts each run, so it is
    /// idempotent: syncing twice yields one file per day, not duplicates.
    pub fn sync_all_transcripts(
        &self,
        transcripts: &[Transcript],
    ) -> Result<SyncReport, CommandError> {
        let root = self.ensure_folder(None, TRANSCRIPTS_ROOT_FOLDER_NAME)?;

        // Group by the transcript's *local* calendar day so file names match
        // the owner's clock, not UTC — same convention as the notes sync.
        let mut by_day: BTreeMap<String, Vec<&Transcript>> = BTreeMap::new();
        for transcript in transcripts {
            let day = transcript
                .created_at
                .with_timezone(&Local)
                .format("%Y-%m-%d")
                .to_string();
            by_day.entry(day).or_default().push(transcript);
        }

        let mut files_written = 0_u32;
        let mut synced_notes = 0_u32;
        for (day, mut day_items) in by_day {
            day_items.sort_by_key(|item| item.created_at);
            let content = render_transcripts_daily(&day, &day_items);
            self.write_into_month(&root, &day, &format!("{day}.md"), &content)?;

            files_written += 1;
            synced_notes += day_items.len() as u32;
        }

        Ok(SyncReport { synced_notes, files_written })
    }

    /// Ensures the `YYYY-MM` month folder under `root`, find-or-creates
    /// `file_name` inside it, and uploads `content`. Shared by the daily sync
    /// and the end-of-day organize pass.
    fn write_into_month(
        &self,
        root: &str,
        day: &str,
        file_name: &str,
        content: &str,
    ) -> Result<(), CommandError> {
        let month = &day[..7]; // YYYY-MM
        let month_folder = self.ensure_folder(Some(root), month)?;
        let file_id = match self.find_file(&month_folder, file_name)? {
            Some(id) => id,
            None => self.create_file(&month_folder, file_name)?,
        };
        self.upload_text(&file_id, content)
    }

    /// Writes the end-of-day organized Markdown into the day's month folder as
    /// `{day}-organized.md`, alongside the raw `{day}.md` daily log.
    pub fn write_organized(&self, day: &str, content: &str) -> Result<(), CommandError> {
        let root = self.ensure_folder(None, ROOT_FOLDER_NAME)?;
        self.write_into_month(&root, day, &format!("{day}-organized.md"), content)
    }

    /// Finds the folder named `name` (optionally under `parent`), creating it if
    /// absent. Returns its file id.
    fn ensure_folder(&self, parent: Option<&str>, name: &str) -> Result<String, CommandError> {
        if let Some(id) = self.find(parent, name, Some(FOLDER_MIME))? {
            return Ok(id);
        }
        self.create(parent, name, Some(FOLDER_MIME))
    }

    fn find_file(&self, parent: &str, name: &str) -> Result<Option<String>, CommandError> {
        self.find(Some(parent), name, None)
    }

    fn create_file(&self, parent: &str, name: &str) -> Result<String, CommandError> {
        self.create(Some(parent), name, None)
    }

    fn find(
        &self,
        parent: Option<&str>,
        name: &str,
        mime: Option<&str>,
    ) -> Result<Option<String>, CommandError> {
        let mut clauses = vec![
            format!("name = '{}'", escape_query_literal(name)),
            "trashed = false".to_string(),
        ];
        if let Some(mime) = mime {
            clauses.push(format!("mimeType = '{mime}'"));
        }
        if let Some(parent) = parent {
            clauses.push(format!("'{}' in parents", escape_query_literal(parent)));
        }
        let q = clauses.join(" and ");
        let url = format!(
            "{}/files?q={}&fields=files(id,name)&spaces=drive&pageSize=10",
            self.api,
            encode(&q)
        );

        let json = self.get_json(&url)?;
        Ok(json
            .get("files")
            .and_then(|files| files.as_array())
            .and_then(|files| files.first())
            .and_then(|file| file.get("id"))
            .and_then(|id| id.as_str())
            .map(ToOwned::to_owned))
    }

    fn create(
        &self,
        parent: Option<&str>,
        name: &str,
        mime: Option<&str>,
    ) -> Result<String, CommandError> {
        let mut metadata = json!({ "name": name });
        if let Some(mime) = mime {
            metadata["mimeType"] = json!(mime);
        }
        if let Some(parent) = parent {
            metadata["parents"] = json!([parent]);
        }

        let url = format!("{}/files?fields=id", self.api);
        let response = self
            .client
            .post(&url)
            .timeout(TIMEOUT)
            .header("Authorization", self.bearer())
            .header("Content-Type", "application/json")
            .body(metadata.to_string())
            .send()
            .map_err(|error| failure(format!("Could not reach Google Drive. {error}")))?;
        let json = read_json(response, "create a Drive file")?;
        json.get("id")
            .and_then(|id| id.as_str())
            .map(ToOwned::to_owned)
            .ok_or_else(|| failure("Google Drive did not return a file id."))
    }

    fn upload_text(&self, file_id: &str, content: &str) -> Result<(), CommandError> {
        let url = format!("{}/files/{}?uploadType=media", self.upload, file_id);
        let response = self
            .client
            .patch(&url)
            .timeout(TIMEOUT)
            .header("Authorization", self.bearer())
            .header("Content-Type", "text/markdown; charset=utf-8")
            .body(content.to_string())
            .send()
            .map_err(|error| failure(format!("Could not upload to Google Drive. {error}")))?;
        // We only need success; the returned metadata is ignored.
        read_json(response, "upload Drive file content").map(|_| ())
    }

    fn get_json(&self, url: &str) -> Result<serde_json::Value, CommandError> {
        let response = self
            .client
            .get(url)
            .timeout(TIMEOUT)
            .header("Authorization", self.bearer())
            .send()
            .map_err(|error| failure(format!("Could not reach Google Drive. {error}")))?;
        read_json(response, "query Google Drive")
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }

    #[cfg(test)]
    fn with_base(access_token: &str, base: &str) -> Self {
        Self {
            client: reqwest::blocking::Client::new(),
            token: access_token.to_string(),
            api: base.to_string(),
            upload: base.to_string(),
        }
    }
}

fn read_json(
    response: reqwest::blocking::Response,
    action: &str,
) -> Result<serde_json::Value, CommandError> {
    let status = response.status();
    let text = response
        .text()
        .map_err(|error| failure(format!("Could not read the Drive response. {error}")))?;
    if !status.is_success() {
        return Err(failure(format!(
            "Google Drive returned HTTP {status} trying to {action}. {}",
            truncate(&text, 300)
        )));
    }
    if text.trim().is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_str(&text)
        .map_err(|error| failure(format!("Could not parse the Drive response. {error}")))
}

/// Renders one day's notes into the daily Markdown file.
fn render_daily(day: &str, notes: &[&Transcript]) -> String {
    // Friendly heading, e.g. "# June 12, 2026" (falls back to the raw day).
    let heading = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|date| format!("{} {}, {}", date.format("%B"), date.day(), date.year()))
        .unwrap_or_else(|_| day.to_string());
    let mut out = format!("# {heading}\n\n");
    for note in notes {
        out.push_str(&format!("**{}**\n\n", format_time(note.created_at)));
        out.push_str(note.text.trim());
        out.push('\n');
        // Show a summary only when the note has actually been analyzed — no
        // empty "Summary: —" clutter. Rendered as a blockquote so it reads as
        // a set-apart note, handling multi-line LLM output cleanly.
        if let Some(summary) = note
            .analysis
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            out.push('\n');
            for line in summary.lines() {
                out.push_str(&format!("> {line}\n"));
            }
        }
        out.push('\n');
    }
    out
}

/// Renders one day's transcripts into the daily Markdown file for the
/// full-history backup. Like `render_daily` but plainer: a timestamped entry
/// per transcript with its text (no analysis blockquote — the transcript dump
/// is the raw record, the notes log carries the summaries).
fn render_transcripts_daily(day: &str, transcripts: &[&Transcript]) -> String {
    let heading = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|date| format!("{} {}, {}", date.format("%B"), date.day(), date.year()))
        .unwrap_or_else(|_| day.to_string());
    let mut out = format!("# {heading}\n\n");
    for transcript in transcripts {
        out.push_str(&format!("**{}**\n\n", format_time(transcript.created_at)));
        out.push_str(transcript.text.trim());
        out.push('\n');
        out.push('\n');
    }
    out
}

/// 12-hour local time like "3:51 PM" (no leading zero on the hour).
fn format_time(at: chrono::DateTime<chrono::Utc>) -> String {
    let local = at.with_timezone(&Local);
    let (hour12, meridiem) = match local.hour() {
        0 => (12, "AM"),
        hour @ 1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        hour => (hour - 12, "PM"),
    };
    format!("{}:{:02} {}", hour12, local.minute(), meridiem)
}

/// Escapes a value embedded in a Drive `q` string literal (single quotes and
/// backslashes), per the Drive query grammar.
fn escape_query_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Percent-encodes a query-string value (unreserved set passes through).
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn failure(message: impl Into<String>) -> CommandError {
    CommandError::new("drive_sync_failed", message)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Sequential HTTP mock: serves `responses` in order, capturing each
    /// request (request line + body). Mirrors the harness in note_analysis.rs.
    fn mock_server(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 65536];
                let mut request = Vec::new();
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some(headers_end) = text.find("\r\n\r\n") {
                        let content_length = text
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= headers_end + 4 + content_length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                let payload = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
                stream.write_all(payload.as_bytes()).unwrap();
            }
            requests
        });
        (base, handle)
    }

    fn note(id: &str, text: &str, analysis: Option<&str>, at: chrono::DateTime<Utc>) -> Transcript {
        let mut t = Transcript::new_last_buffer(text, Some(1000), None, Some("en".into())).unwrap();
        t.id = id.to_string();
        t.created_at = at;
        t.is_note = true;
        t.analysis = analysis.map(ToOwned::to_owned);
        t
    }

    #[test]
    fn sync_creates_folders_then_writes_daily_file() {
        // Everything is missing, so: find+create root, find+create month,
        // find(empty)+create daily, then upload its content (7 calls).
        let (base, handle) = mock_server(vec![
            json!({ "files": [] }).to_string(),       // find root
            json!({ "id": "root1" }).to_string(),     // create root
            json!({ "files": [] }).to_string(),       // find month
            json!({ "id": "month1" }).to_string(),    // create month
            json!({ "files": [] }).to_string(),       // find daily
            json!({ "id": "daily1" }).to_string(),    // create daily
            json!({ "id": "daily1" }).to_string(),    // upload content
        ]);

        let at = Utc.with_ymd_and_hms(2026, 6, 12, 18, 30, 0).unwrap();
        let notes = vec![note("tx_1", "buy milk and call Sam", Some("Summary: errands"), at)];

        let drive = Drive::with_base("tok", &base);
        let report = drive.sync_notes(&notes).unwrap();

        assert_eq!(report.files_written, 1);
        assert_eq!(report.synced_notes, 1);

        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 7);
        // Every request carried the bearer token (reqwest lowercases header
        // names on the wire, so match the case-preserved value).
        assert!(requests.iter().all(|r| r.contains("Bearer tok")));
        // The first call is a folder query for the root app folder.
        assert!(requests[0].starts_with("GET /files?q="));
        assert!(requests[0].contains(encode("Scribe Voice Notes").as_str()));
        // The upload PATCH carried the rendered note text + summary blockquote.
        assert!(requests[6].starts_with("PATCH /files/daily1?uploadType=media"));
        assert!(requests[6].contains("buy milk and call Sam"));
        assert!(requests[6].contains("> Summary: errands"));
    }

    #[test]
    fn sync_reuses_existing_folders_and_file() {
        // Folders + daily file already exist: each find returns an id, so the
        // only write is the content PATCH (4 calls total).
        let (base, handle) = mock_server(vec![
            json!({ "files": [{ "id": "root1", "name": "Scribe Voice Notes" }] }).to_string(),
            json!({ "files": [{ "id": "month1", "name": "2026-06" }] }).to_string(),
            json!({ "files": [{ "id": "daily1", "name": "2026-06-12.md" }] }).to_string(),
            json!({ "id": "daily1" }).to_string(),
        ]);

        let at = Utc.with_ymd_and_hms(2026, 6, 12, 18, 30, 0).unwrap();
        let notes = vec![note("tx_1", "no summary here", None, at)];

        let drive = Drive::with_base("tok", &base);
        let report = drive.sync_notes(&notes).unwrap();
        assert_eq!(report, SyncReport { synced_notes: 1, files_written: 1 });

        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 4);
        // An un-analyzed note has no summary blockquote at all (no clutter).
        assert!(requests[3].contains("no summary here"));
        assert!(!requests[3].contains("> "));
    }

    #[test]
    fn sync_all_transcripts_creates_distinct_root_then_writes_daily_file() {
        // Same shape as the notes test, but into the "Scribe Transcripts" root:
        // find+create root, find+create month, find(empty)+create daily, upload.
        let (base, handle) = mock_server(vec![
            json!({ "files": [] }).to_string(),    // find root
            json!({ "id": "root1" }).to_string(),  // create root
            json!({ "files": [] }).to_string(),    // find month
            json!({ "id": "month1" }).to_string(), // create month
            json!({ "files": [] }).to_string(),    // find daily
            json!({ "id": "daily1" }).to_string(), // create daily
            json!({ "id": "daily1" }).to_string(), // upload content
        ]);

        let at = Utc.with_ymd_and_hms(2026, 6, 12, 18, 30, 0).unwrap();
        // A dictation transcript (not a note); its analysis must NOT be rendered.
        let mut tx = note("tx_1", "draft the quarterly update", Some("Summary: ignore me"), at);
        tx.is_note = false;
        let transcripts = vec![tx];

        let drive = Drive::with_base("tok", &base);
        let report = drive.sync_all_transcripts(&transcripts).unwrap();

        assert_eq!(report.files_written, 1);
        assert_eq!(report.synced_notes, 1);

        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 7);
        assert!(requests.iter().all(|r| r.contains("Bearer tok")));
        // The first call queries for the DISTINCT transcripts root folder, not
        // the notes folder.
        assert!(requests[0].starts_with("GET /files?q="));
        assert!(requests[0].contains(encode("Scribe Transcripts").as_str()));
        assert!(!requests[0].contains(encode("Scribe Voice Notes").as_str()));
        // The upload PATCH carried the transcript text but not the analysis
        // (the raw dump has no summary blockquote).
        assert!(requests[6].starts_with("PATCH /files/daily1?uploadType=media"));
        assert!(requests[6].contains("draft the quarterly update"));
        assert!(!requests[6].contains("Summary: ignore me"));
    }

    #[test]
    fn render_daily_orders_and_separates_entries() {
        let early = Utc.with_ymd_and_hms(2026, 6, 12, 8, 0, 0).unwrap();
        let late = Utc.with_ymd_and_hms(2026, 6, 12, 9, 0, 0).unwrap();
        // Pass out of order; render sorts by time only via sync_notes, so test
        // render directly with the intended order.
        let a = note("a", "first", None, early);
        let b = note("b", "second", None, late);
        let rendered = render_daily("2026-06-12", &[&a, &b]);
        assert!(rendered.starts_with("# June 12, 2026"));
        let first = rendered.find("first").unwrap();
        let second = rendered.find("second").unwrap();
        assert!(first < second, "entries in chronological order");
        // No empty summaries for un-analyzed notes.
        assert!(!rendered.contains("> "));
    }
}
