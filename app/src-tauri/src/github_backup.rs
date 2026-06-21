//! GitHub Contents-API backup for the notes-sync feature. Per local calendar
//! day, the daily Markdown file is regenerated from the local DB on each sync
//! (SQLite is the source of truth), so "Sync now" is idempotent — syncing twice
//! yields one file, not duplicated entries.
//!
//! The Contents PUT to UPDATE an existing file requires its current blob `sha`.
//! So each daily write is GET-contents (to
//! learn the sha; 404 = new file, omit sha) then PUT-contents (base64 body +
//! sha). The GET is ONLY for the sha — we never read or concatenate remote
//! content; the whole day is re-rendered from the DB.

use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;
use chrono::{Datelike, Local, Timelike};
use serde::Serialize;
use serde_json::json;

use crate::error::CommandError;
use crate::transcript::Transcript;

const API_BASE: &str = "https://api.github.com";
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    /// Number of note/transcript entries written across all daily files.
    pub synced_notes: u32,
    /// Number of distinct daily Markdown files created or updated.
    pub files_written: u32,
}

/// A handle to the GitHub Contents API for one access token and one repo. The
/// base URL is a field so tests can point it at a local mock server.
pub struct GitHubBackup {
    client: reqwest::blocking::Client,
    token: String,
    owner: String,
    repo: String,
    api: String,
}

impl GitHubBackup {
    /// Builds a backup handle for `owner/name`. Errors when `repo_slug` is not
    /// exactly "owner/name".
    pub fn new(token: impl Into<String>, repo_slug: &str) -> Result<Self, CommandError> {
        let mut parts = repo_slug.split('/');
        let owner = parts.next().unwrap_or("").trim().to_string();
        let repo = parts.next().unwrap_or("").trim().to_string();
        if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
            return Err(failure(format!(
                "The GitHub repository must be in \"owner/name\" form, got \"{repo_slug}\"."
            )));
        }
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .user_agent(concat!("Scribe/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|error| failure(error.to_string()))?,
            token: token.into(),
            owner,
            repo,
            api: API_BASE.to_string(),
        })
    }

    /// Writes each note into its day's `YYYY-MM/YYYY-MM-DD.md` file at the repo
    /// root. The daily file is rebuilt from the supplied notes (caller passes
    /// the full set for each day from the DB).
    pub fn sync_notes(&self, notes: &[Transcript]) -> Result<SyncReport, CommandError> {
        self.ensure_repo()?;

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
            let path = format!("{}/{}.md", &day[..7], day);
            self.put_day_file(&path, &content)?;

            files_written += 1;
            synced_notes += day_notes.len() as u32;
        }

        Ok(SyncReport { synced_notes, files_written })
    }

    /// Backs up *every* supplied transcript into a DISTINCT `transcripts/`
    /// top-level folder, grouped by `YYYY-MM` with one daily Markdown file per
    /// local calendar day — mirroring `sync_notes` but for the full-history dump
    /// rather than the curated notes log. The daily file is rebuilt from the
    /// supplied transcripts each run, so it is idempotent.
    pub fn sync_all_transcripts(
        &self,
        transcripts: &[Transcript],
    ) -> Result<SyncReport, CommandError> {
        self.ensure_repo()?;

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
            let path = format!("transcripts/{}/{}.md", &day[..7], day);
            self.put_day_file(&path, &content)?;

            files_written += 1;
            synced_notes += day_items.len() as u32;
        }

        Ok(SyncReport { synced_notes, files_written })
    }

    /// Ensures the target repo exists. GET the repo; 200 → Ok. On 404, create it
    /// (private, auto-init) but ONLY when `owner` is the authenticated user —
    /// otherwise we'd land it in the wrong account, so we return a clear error
    /// asking the user to create `owner/name` manually.
    pub fn ensure_repo(&self) -> Result<(), CommandError> {
        let url = format!("{}/repos/{}/{}", self.api, self.owner, self.repo);
        let (status, _) = self.get_json(&url)?;
        if status.is_success() {
            return Ok(());
        }
        if status.as_u16() != 404 {
            return Err(failure(format!(
                "GitHub returned HTTP {status} checking the repository {}/{}.",
                self.owner, self.repo
            )));
        }

        // 404: create it under the authenticated user, if that's the owner.
        let login = self.fetch_login()?;
        if !login.eq_ignore_ascii_case(&self.owner) {
            return Err(failure(format!(
                "The repository {}/{} does not exist and could not be created automatically \
                 (it is owned by \"{}\", not your account \"{}\"). Create it on GitHub first.",
                self.owner, self.repo, self.owner, login
            )));
        }

        let create_url = format!("{}/user/repos", self.api);
        let body = json!({ "name": self.repo, "private": true, "auto_init": true });
        let response = self
            .client
            .post(&create_url)
            .timeout(TIMEOUT)
            .header("Authorization", self.bearer())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .body(body.to_string())
            .send()
            .map_err(|error| failure(format!("Could not reach GitHub. {error}")))?;
        let status = response.status();
        let text = response
            .text()
            .map_err(|error| failure(format!("Could not read GitHub's response. {error}")))?;
        if !status.is_success() {
            return Err(failure(format!(
                "GitHub returned HTTP {status} creating the repository {}/{}. {}",
                self.owner,
                self.repo,
                truncate(&text, 300)
            )));
        }
        Ok(())
    }

    /// GET the file's blob sha (None on 404), then PUT the new content (base64,
    /// with the sha when updating). On 409/422 (sha conflict / mismatch), re-GET
    /// the sha and retry the PUT once.
    fn put_day_file(&self, path: &str, content: &str) -> Result<(), CommandError> {
        let sha = self.get_sha(path)?;
        match self.put_contents(path, content, sha.as_deref())? {
            PutOutcome::Ok => Ok(()),
            PutOutcome::Conflict => {
                // A concurrent write changed the blob; re-fetch the sha and try
                // once more.
                let sha = self.get_sha(path)?;
                match self.put_contents(path, content, sha.as_deref())? {
                    PutOutcome::Ok => Ok(()),
                    PutOutcome::Conflict => Err(failure(format!(
                        "GitHub kept rejecting the write to {path} (sha conflict). Try again."
                    ))),
                }
            }
        }
    }

    /// GET `contents/{path}`; returns Some(sha) on 200, None on 404, error
    /// otherwise.
    fn get_sha(&self, path: &str) -> Result<Option<String>, CommandError> {
        let url = self.contents_url(path);
        let (status, json) = self.get_json(&url)?;
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(failure(format!(
                "GitHub returned HTTP {status} reading {path}."
            )));
        }
        Ok(json
            .get("sha")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned))
    }

    /// PUT `contents/{path}` with base64 content (+ sha when updating). Returns
    /// `Conflict` on 409/422 so the caller can re-fetch the sha and retry.
    fn put_contents(
        &self,
        path: &str,
        content: &str,
        sha: Option<&str>,
    ) -> Result<PutOutcome, CommandError> {
        let url = self.contents_url(path);
        let mut body = json!({
            "message": format!("scribe: notes {}", file_day(path)),
            "content": base64::engine::general_purpose::STANDARD.encode(content.as_bytes()),
        });
        if let Some(sha) = sha {
            body["sha"] = json!(sha);
        }
        let response = self
            .client
            .put(&url)
            .timeout(TIMEOUT)
            .header("Authorization", self.bearer())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .body(body.to_string())
            .send()
            .map_err(|error| failure(format!("Could not write to GitHub. {error}")))?;
        let status = response.status();
        if status.is_success() {
            return Ok(PutOutcome::Ok);
        }
        if status.as_u16() == 409 || status.as_u16() == 422 {
            return Ok(PutOutcome::Conflict);
        }
        if status.as_u16() == 401 {
            return Err(unauthorized());
        }
        let text = response
            .text()
            .map_err(|error| failure(format!("Could not read GitHub's response. {error}")))?;
        Err(failure(format!(
            "GitHub returned HTTP {status} writing {path}. {}",
            truncate(&text, 300)
        )))
    }

    fn fetch_login(&self) -> Result<String, CommandError> {
        let url = format!("{}/user", self.api);
        let (status, json) = self.get_json(&url)?;
        if !status.is_success() {
            return Err(failure(format!(
                "GitHub returned HTTP {status} reading the authenticated account."
            )));
        }
        Ok(json
            .get("login")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string())
    }

    /// GET a URL with the GitHub headers, returning (status, parsed-json). A
    /// non-2xx status is NOT an error here so callers can inspect 404/409/422.
    fn get_json(&self, url: &str) -> Result<(reqwest::StatusCode, serde_json::Value), CommandError> {
        let response = self
            .client
            .get(url)
            .timeout(TIMEOUT)
            .header("Authorization", self.bearer())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .map_err(|error| failure(format!("Could not reach GitHub. {error}")))?;
        let status = response.status();
        if status.as_u16() == 401 {
            return Err(unauthorized());
        }
        let text = response
            .text()
            .map_err(|error| failure(format!("Could not read GitHub's response. {error}")))?;
        let json = if text.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
        };
        Ok((status, json))
    }

    fn contents_url(&self, path: &str) -> String {
        format!(
            "{}/repos/{}/{}/contents/{}",
            self.api,
            self.owner,
            self.repo,
            encode_path(path)
        )
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }

    #[cfg(test)]
    fn with_base(token: &str, owner: &str, repo: &str, base: &str) -> Self {
        Self {
            client: reqwest::blocking::Client::new(),
            token: token.to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            api: base.to_string(),
        }
    }
}

enum PutOutcome {
    Ok,
    Conflict,
}

/// The day component of a daily file path (the file stem), used in the commit
/// message. E.g. `2026-06/2026-06-12.md` → `2026-06-12`.
fn file_day(path: &str) -> String {
    path.rsplit('/')
        .next()
        .and_then(|name| name.strip_suffix(".md"))
        .unwrap_or(path)
        .to_string()
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

/// Percent-encodes a Contents-API path, keeping '/' as the segment separator
/// but escaping reserved chars within each segment (unreserved set passes
/// through).
fn encode_path(path: &str) -> String {
    path.split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
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
    CommandError::new("github_sync_failed", message)
}

/// HTTP 401 from GitHub means the stored token was revoked or expired. Distinct
/// code so the caller can prompt a reconnect (and clear the dead token) instead
/// of showing a raw "HTTP 401".
fn unauthorized() -> CommandError {
    CommandError::new(
        "github_unauthorized",
        "Your GitHub connection expired or was revoked. Reconnect GitHub in Settings → Sync.",
    )
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

    /// Sequential HTTP mock: serves `(status, body)` pairs in order, capturing
    /// each request. Mirrors the sequential-mock harness in note_analysis.rs.
    fn mock_server(responses: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
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
                    "HTTP/1.1 {} STATUS\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
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
    fn put_day_file_creates_new_file_with_base64_content() {
        // GET contents → 404 (new file), then PUT (no sha, create). We hit
        // put_day_file directly to avoid the ensure_repo() GET.
        let (base, handle) = mock_server(vec![
            (404, serde_json::json!({ "message": "Not Found" }).to_string()),
            (201, serde_json::json!({ "content": { "sha": "newsha" } }).to_string()),
        ]);

        let backup = GitHubBackup::with_base("tok", "alice", "scribe-notes", &base);
        backup
            .put_day_file("2026-06/2026-06-12.md", "# June 12, 2026\n\nhello\n")
            .unwrap();

        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 2, "GET-then-PUT");
        assert!(requests.iter().all(|r| r.contains("Bearer tok")));
        assert!(requests[0]
            .starts_with("GET /repos/alice/scribe-notes/contents/2026-06/2026-06-12.md"));
        assert!(requests[1]
            .starts_with("PUT /repos/alice/scribe-notes/contents/2026-06/2026-06-12.md"));
        // The PUT body carries base64 of the content and, since the file is new,
        // NO sha field.
        let expected_b64 =
            base64::engine::general_purpose::STANDARD.encode("# June 12, 2026\n\nhello\n".as_bytes());
        assert!(requests[1].contains(&expected_b64));
        assert!(!requests[1].contains("\"sha\""));
        // Header names are lowercased on the wire by reqwest; match lowercased.
        assert!(requests[1]
            .to_ascii_lowercase()
            .contains("x-github-api-version: 2022-11-28"));
    }

    #[test]
    fn put_day_file_sends_sha_when_updating() {
        // GET contents → 200 with an existing sha, then PUT carries that sha.
        let (base, handle) = mock_server(vec![
            (200, serde_json::json!({ "sha": "oldsha123" }).to_string()),
            (200, serde_json::json!({ "content": { "sha": "newsha" } }).to_string()),
        ]);

        let backup = GitHubBackup::with_base("tok", "alice", "scribe-notes", &base);
        backup
            .put_day_file("2026-06/2026-06-12.md", "updated\n")
            .unwrap();

        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("PUT /repos/alice/scribe-notes/contents/"));
        assert!(requests[1].contains("oldsha123"), "update PUT carries the sha");
    }

    #[test]
    fn sync_notes_ensures_repo_then_writes_path() {
        // ensure_repo GET (200), then GET-contents (404) + PUT for the one day.
        let (base, handle) = mock_server(vec![
            (200, serde_json::json!({ "id": 1, "full_name": "alice/scribe-notes" }).to_string()),
            (404, serde_json::json!({ "message": "Not Found" }).to_string()),
            (201, serde_json::json!({ "content": { "sha": "s" } }).to_string()),
        ]);

        let at = Utc.with_ymd_and_hms(2026, 6, 12, 18, 30, 0).unwrap();
        let notes = vec![note("tx_1", "buy milk and call Sam", Some("Summary: errands"), at)];

        let backup = GitHubBackup::with_base("tok", "alice", "scribe-notes", &base);
        let report = backup.sync_notes(&notes).unwrap();
        assert_eq!(report, SyncReport { synced_notes: 1, files_written: 1 });

        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].starts_with("GET /repos/alice/scribe-notes "));
        // The day's file is written under its YYYY-MM month folder at repo root.
        assert!(requests[1].contains("/contents/2026-06/2026-06-12.md"));
        // The PUT body carries the rendered note text + summary blockquote
        // (base64), so decode and check.
        let body_start = requests[2].find("\r\n\r\n").unwrap() + 4;
        let body = &requests[2][body_start..];
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        let content_b64 = json["content"].as_str().unwrap();
        let decoded = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(content_b64)
                .unwrap(),
        )
        .unwrap();
        assert!(decoded.contains("buy milk and call Sam"));
        assert!(decoded.contains("> Summary: errands"));
    }

    #[test]
    fn sync_all_transcripts_uses_transcripts_path() {
        let (base, handle) = mock_server(vec![
            (200, serde_json::json!({ "id": 1 }).to_string()),
            (404, serde_json::json!({ "message": "Not Found" }).to_string()),
            (201, serde_json::json!({ "content": { "sha": "s" } }).to_string()),
        ]);

        let at = Utc.with_ymd_and_hms(2026, 6, 12, 18, 30, 0).unwrap();
        let mut tx = note("tx_1", "draft the quarterly update", Some("ignore me"), at);
        tx.is_note = false;
        let backup = GitHubBackup::with_base("tok", "alice", "scribe-notes", &base);
        let report = backup.sync_all_transcripts(&[tx]).unwrap();
        assert_eq!(report.files_written, 1);

        let requests = handle.join().unwrap();
        assert!(requests[1].contains("/contents/transcripts/2026-06/2026-06-12.md"));
    }

    #[test]
    fn new_rejects_bad_repo_slug() {
        assert!(GitHubBackup::new("tok", "noslash").is_err());
        assert!(GitHubBackup::new("tok", "a/b/c").is_err());
        assert!(GitHubBackup::new("tok", "owner/name").is_ok());
    }

    #[test]
    fn sync_report_serializes_camel_case() {
        let report = SyncReport { synced_notes: 3, files_written: 2 };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["syncedNotes"], 3);
        assert_eq!(json["filesWritten"], 2);
    }
}
