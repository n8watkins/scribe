//! Replacements live in one plain text file, `replacements.txt` in Scribe's
//! data folder: one `heard -> written` per line, `#` comments and blank lines
//! ignored. The file is the source of truth; the app reads it, appends to it
//! and removes single lines from it, and picks up the user's own edits.
//!
//! Concurrency: every app write re-reads the file under a lock, changes only
//! its own lines, and replaces the file atomically (write a temp file, then
//! rename over the original), so a reader never sees half a file and a user's
//! saved edit is never mixed into a partial write. If the user's editor saves
//! an older copy after an app write, the editor's copy wins, as with any file
//! two programs edit. A background watcher reloads after any change.
//!
//! Format guide for people: docs/REPLACEMENTS.md.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::text_replace::TextReplacement;

pub const FILE_NAME: &str = "replacements.txt";

const HEADER: &str = "# Scribe replacements: one \"heard -> written\" per line.\n\
# Matching ignores case and only matches whole words; longer phrases are applied first.\n\
# Lines starting with # are comments. Format guide: Words > Replacements > Format.\n";

/// How often the watcher checks the file for outside edits.
const WATCH_INTERVAL: Duration = Duration::from_secs(2);

/// What the file holds right now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementsInfo {
    pub path: String,
    pub rows: Vec<TextReplacement>,
    /// 1-based numbers of lines that aren't comments, blank or `heard -> written`.
    pub skipped_lines: Vec<usize>,
}

/// What an append did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddReport {
    pub added: usize,
    pub duplicates: usize,
    pub info: ReplacementsInfo,
}

/// One parsed line: a replacement, a line to ignore, or a line that can't be read.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    Entry(TextReplacement),
    Ignore,
    Invalid,
}

const ARROWS: [&str; 3] = ["->", "=>", "\u{2192}"];

fn strip_quotes(text: &str) -> &str {
    text.trim()
        .trim_matches(|c| {
            matches!(
                c,
                '"' | '\'' | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}' | '`'
            )
        })
        .trim()
}

fn parse_line(raw: &str) -> Line {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
        return Line::Ignore;
    }
    let found = ARROWS
        .iter()
        .filter_map(|arrow| line.find(arrow).map(|index| (index, arrow.len())))
        .min_by_key(|(index, _)| *index);
    let Some((index, len)) = found else {
        return Line::Invalid;
    };
    let from = strip_quotes(&line[..index]);
    let to = strip_quotes(&line[index + len..]);
    if from.is_empty() || to.is_empty() {
        return Line::Invalid;
    }
    Line::Entry(TextReplacement {
        from: from.to_string(),
        to: to.to_string(),
    })
}

/// The replacements in `text`, in file order, and the numbers of lines that
/// couldn't be read.
pub fn parse(text: &str) -> (Vec<TextReplacement>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        match parse_line(raw) {
            Line::Entry(row) => rows.push(row),
            Line::Ignore => {}
            Line::Invalid => skipped.push(index + 1),
        }
    }
    (rows, skipped)
}

fn format_row(row: &TextReplacement) -> String {
    format!("{} -> {}", row.from.trim(), row.to.trim())
}

/// A new file: the header, then one line per replacement.
pub fn format_file(rows: &[TextReplacement]) -> String {
    let mut text = String::from(HEADER);
    text.push('\n');
    for row in rows
        .iter()
        .filter(|row| !row.from.trim().is_empty() && !row.to.trim().is_empty())
    {
        text.push_str(&format_row(row));
        text.push('\n');
    }
    text
}

fn same_heard(a: &TextReplacement, b: &TextReplacement) -> bool {
    a.from.trim().to_lowercase() == b.from.trim().to_lowercase()
}

/// `text` with `rows` appended, skipping any whose heard phrase is already
/// there. Comments and the user's own lines are kept as they are.
pub fn append_rows(text: &str, rows: &[TextReplacement]) -> (String, usize, usize) {
    let (mut existing, _) = parse(text);
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    let (mut added, mut duplicates) = (0, 0);
    for row in rows {
        if row.from.trim().is_empty() || row.to.trim().is_empty() {
            continue;
        }
        if existing.iter().any(|have| same_heard(have, row)) {
            duplicates += 1;
            continue;
        }
        out.push_str(&format_row(row));
        out.push('\n');
        existing.push(row.clone());
        added += 1;
    }
    (out, added, duplicates)
}

/// `text` without the first line that holds `row`, or None when no line does.
pub fn remove_row(text: &str, row: &TextReplacement) -> Option<String> {
    let mut removed = false;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if !removed {
            if let Line::Entry(entry) = parse_line(line) {
                if same_heard(&entry, row) && entry.to.trim() == row.to.trim() {
                    removed = true;
                    continue;
                }
            }
        }
        out.push_str(line);
    }
    removed.then_some(out)
}

/// Replaces `path` with `text` atomically: a temp file next to it, flushed,
/// then renamed over it.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let temp = path.with_extension("txt.tmp");
    {
        let mut file = fs::File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&temp, path)
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

struct Loaded {
    info: ReplacementsInfo,
    stamp: Option<(SystemTime, u64)>,
}

/// The loaded file, reloaded when it changes on disk.
pub struct ReplacementStore {
    path: PathBuf,
    loaded: Mutex<Loaded>,
}

impl ReplacementStore {
    /// Opens `path`, creating it from `legacy` (the list Scribe kept in its
    /// settings before) the first time, with a JSON backup of that list next
    /// to it.
    pub fn open(path: PathBuf, legacy: &[TextReplacement]) -> Self {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            if !legacy.is_empty() {
                let backup = path.with_file_name("replacements-backup-from-settings.json");
                if let Ok(json) = serde_json::to_string_pretty(legacy) {
                    let _ = fs::write(backup, json);
                }
            }
            if let Err(error) = write_atomic(&path, &format_file(legacy)) {
                log::warn!("Could not create {}: {}", path.display(), error);
            }
        }
        let store = Self {
            path,
            loaded: Mutex::new(Loaded {
                info: ReplacementsInfo::default(),
                stamp: None,
            }),
        };
        store.reload();
        store
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Loaded> {
        self.loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn read(&self) -> String {
        fs::read_to_string(&self.path).unwrap_or_default()
    }

    fn info_for(&self, text: &str) -> ReplacementsInfo {
        let (rows, skipped_lines) = parse(text);
        ReplacementsInfo {
            path: self.path.to_string_lossy().into_owned(),
            rows,
            skipped_lines,
        }
    }

    /// Reads the file again; returns true when it changed since the last read.
    pub fn reload(&self) -> bool {
        let now = stamp(&self.path);
        let mut loaded = self.lock();
        if loaded.stamp.is_some() && loaded.stamp == now {
            return false;
        }
        let text = self.read();
        loaded.info = self.info_for(&text);
        loaded.stamp = now;
        true
    }

    /// The current replacements, after picking up any outside edit.
    pub fn current(&self) -> ReplacementsInfo {
        self.reload();
        self.lock().info.clone()
    }

    /// Appends rows whose heard phrase isn't in the file yet.
    pub fn add(&self, rows: &[TextReplacement]) -> std::io::Result<AddReport> {
        let mut loaded = self.lock();
        let text = self.read();
        let (next, added, duplicates) = append_rows(&text, rows);
        if added > 0 {
            write_atomic(&self.path, &next)?;
        }
        loaded.info = self.info_for(&next);
        loaded.stamp = stamp(&self.path);
        Ok(AddReport {
            added,
            duplicates,
            info: loaded.info.clone(),
        })
    }

    /// Removes the first line holding `row`.
    pub fn remove(&self, row: &TextReplacement) -> std::io::Result<ReplacementsInfo> {
        let mut loaded = self.lock();
        let text = self.read();
        if let Some(next) = remove_row(&text, row) {
            write_atomic(&self.path, &next)?;
            loaded.info = self.info_for(&next);
            loaded.stamp = stamp(&self.path);
        }
        Ok(loaded.info.clone())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Checks the file every couple of seconds and calls `on_change` with the
    /// new contents after an outside edit.
    pub fn watch(self: &Arc<Self>, on_change: impl Fn(&ReplacementsInfo) + Send + 'static) {
        let store = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name("replacements-watch".to_string())
            .spawn(move || loop {
                std::thread::sleep(WATCH_INTERVAL);
                if store.reload() {
                    on_change(&store.lock().info);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(from: &str, to: &str) -> TextReplacement {
        TextReplacement {
            from: from.into(),
            to: to.into(),
        }
    }

    #[test]
    fn reads_entries_and_reports_bad_lines() {
        let text = "# comment\n\nnate builds -> n8builds\nnot a rule\nclawed => Claude\n \"cloud md\" \u{2192} CLAUDE.md\n -> nothing\n";
        let (rows, skipped) = parse(text);
        assert_eq!(
            rows,
            vec![
                row("nate builds", "n8builds"),
                row("clawed", "Claude"),
                row("cloud md", "CLAUDE.md")
            ]
        );
        assert_eq!(skipped, vec![4, 7]);
    }

    #[test]
    fn a_new_file_keeps_the_order_and_reads_back() {
        let rows = vec![row("b", "B"), row("a", "A"), row(" ", "x")];
        let (back, skipped) = parse(&format_file(&rows));
        assert_eq!(back, vec![row("b", "B"), row("a", "A")]);
        assert!(skipped.is_empty());
    }

    #[test]
    fn appending_skips_duplicates_and_keeps_comments() {
        let text = "# mine\nclawed -> Claude";
        let (next, added, duplicates) = append_rows(
            text,
            &[row("Clawed", "Claude"), row("superbase", "Supabase")],
        );
        assert_eq!((added, duplicates), (1, 1));
        assert_eq!(next, "# mine\nclawed -> Claude\nsuperbase -> Supabase\n");
    }

    #[test]
    fn removing_drops_only_the_first_matching_line() {
        let text = "a -> A\n# keep\nb -> B\na -> A\n";
        assert_eq!(
            remove_row(text, &row("A", "A")).unwrap(),
            "# keep\nb -> B\na -> A\n"
        );
        assert_eq!(remove_row(text, &row("zz", "Z")), None);
    }

    #[test]
    fn the_store_migrates_appends_removes_and_sees_outside_edits() {
        let dir = std::env::temp_dir().join(format!("scribe-repl-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join(FILE_NAME);
        let store = ReplacementStore::open(path.clone(), &[row("natebuilds", "n8builds")]);
        assert_eq!(store.current().rows, vec![row("natebuilds", "n8builds")]);
        assert!(dir.join("replacements-backup-from-settings.json").exists());

        let report = store
            .add(&[row("clawed", "Claude"), row("NateBuilds", "x")])
            .unwrap();
        assert_eq!((report.added, report.duplicates), (1, 1));

        // The user edits the file in an editor.
        std::thread::sleep(Duration::from_millis(20));
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("oops no arrow\nversal -> Vercel\n");
        fs::write(&path, text).unwrap();
        let info = store.current();
        assert_eq!(info.rows.len(), 3);
        assert_eq!(info.skipped_lines.len(), 1);

        let info = store.remove(&row("clawed", "Claude")).unwrap();
        assert_eq!(
            info.rows,
            vec![row("natebuilds", "n8builds"), row("versal", "Vercel")]
        );
        assert!(!dir.join("replacements.txt.tmp").exists());

        // Opening again never overwrites the user's file.
        let again = ReplacementStore::open(path.clone(), &[row("other", "x")]);
        assert_eq!(again.current().rows.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }
}
