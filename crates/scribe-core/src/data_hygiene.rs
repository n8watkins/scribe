//! Keeps Scribe's folders from collecting data nobody can see or delete.
//!
//! - The LocalDictate to Scribe rename copied the old data folder but left the
//!   database pointing into the old one; [`repair_rebrand_paths`] repoints
//!   those references at the identical copies and the old folder is removed
//!   once nothing refers to it.
//! - [`sweep_orphan_clips`] deletes clips no transcript points at.
//! - Temp recordings, "open in editor" copies and failed recordings are
//!   cleaned up by age, at startup and when the user clears history.
//!
//! Everything here is best-effort: a file that can't be removed is logged and
//! skipped, never an error that blocks startup.

use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};


use crate::{
    commands::BackendState,
    core::Core,
    db::Database,
    error::CommandError,
    host::Host,
};

/// Temp recordings and segment WAVs older than this are leftovers from a
/// crash or an interrupted session.
const STALE_TEMP_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// A clip newer than this is never treated as orphaned: its transcript may be
/// in the middle of being saved while the sweep runs.
const ORPHAN_GRACE: Duration = Duration::from_secs(60 * 60);

/// The folders Scribe writes to.
#[derive(Debug, Clone)]
pub struct DataDirs {
    /// `%APPDATA%\<identifier>`: database, clips, models, failed recordings.
    pub data_dir: PathBuf,
    /// `%LOCALAPPDATA%\<identifier>`: temp recordings and editor copies.
    pub cache_dir: PathBuf,
}

impl DataDirs {
    pub fn clips(&self) -> PathBuf {
        self.data_dir.join("clips")
    }

    pub fn failed_recordings(&self) -> PathBuf {
        self.data_dir.join("failed")
    }

    pub fn temp_recordings(&self) -> PathBuf {
        self.cache_dir.join("recordings")
    }

    /// Where "Open in editor" writes a transcript's text for the editor.
    pub fn editor_copies(&self) -> PathBuf {
        self.cache_dir.join("scribe-export")
    }

    pub fn editor_copy(&self, transcript_id: &str) -> PathBuf {
        self.editor_copies().join(format!("{}.txt", transcript_id))
    }
}

/// What a startup maintenance pass removed, for the log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub files: usize,
    pub bytes: u64,
}

impl SweepReport {
    fn add(&mut self, other: SweepReport) {
        self.files += other.files;
        self.bytes += other.bytes;
    }
}

/// For each stored path inside `old_dir`, the same relative path inside
/// `new_dir` when a byte-identical copy exists there. Returns `(old, new)`
/// pairs; paths without an identical copy are left out, so they keep pointing
/// at the old folder and keep it from being deleted.
pub fn plan_rebrand_repoints(
    stored: &[String],
    old_dir: &Path,
    new_dir: &Path,
) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut repoints = Vec::new();
    for path in stored {
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(relative) = relative_to(Path::new(path), old_dir) else {
            continue;
        };
        let candidate = new_dir.join(&relative);
        if files_identical(Path::new(path), &candidate) {
            repoints.push((path.clone(), candidate.to_string_lossy().into_owned()));
        }
    }
    repoints
}

/// True when any stored path still lives inside `dir`.
pub fn any_inside(stored: &[String], dir: &Path) -> bool {
    stored
        .iter()
        .any(|path| relative_to(Path::new(path), dir).is_some())
}

/// Repoints the database's references from the pre-rename folder to the
/// copies in the current data folder, then deletes the old folder when the
/// copy step had finished (`migration_complete`) and nothing refers to it any
/// more. Safe to run on every launch.
pub fn repair_rebrand_paths(
    state: &BackendState,
    old_dir: &Path,
    new_dir: &Path,
    migration_complete: bool,
) -> Result<(), CommandError> {
    if !old_dir.is_dir() {
        return Ok(());
    }
    // Compare files without holding the database lock: model files are large.
    let stored = state.db()?.stored_file_paths()?;
    let repoints = plan_rebrand_repoints(&stored, old_dir, new_dir);
    if !repoints.is_empty() {
        let db = state.db()?;
        let changed = repoints
            .iter()
            .map(|(old, new)| db.replace_stored_path(old, new))
            .sum::<Result<usize, _>>()?;
        log::info!(
            "Repointed {} references from {} to the copies in {}",
            changed,
            old_dir.display(),
            new_dir.display()
        );
    }

    let stored = state.db()?.stored_file_paths()?;
    if any_inside(&stored, old_dir) {
        log::warn!(
            "Keeping the old data folder {}: Scribe still uses files in it",
            old_dir.display()
        );
        return Ok(());
    }
    if !migration_complete {
        return Ok(());
    }
    match fs::remove_dir_all(old_dir) {
        Ok(()) => log::info!(
            "Removed the old data folder {}; its contents were copied to {}",
            old_dir.display(),
            new_dir.display()
        ),
        Err(error) => log::warn!(
            "Could not remove the old data folder {}: {}",
            old_dir.display(),
            error
        ),
    }
    Ok(())
}

/// Deletes WAV files in `clips_dir` whose file name no stored clip path
/// uses, skipping files modified after `older_than` (a clip being saved right
/// now is not an orphan yet).
pub fn sweep_orphan_clips(
    clips_dir: &Path,
    referenced: &[String],
    older_than: SystemTime,
) -> SweepReport {
    let names: HashSet<String> = referenced
        .iter()
        .filter_map(|path| file_name(path))
        .collect();
    remove_files_where(clips_dir, false, |path, modified| {
        modified < older_than
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".wav") && !names.contains(name))
    })
}

/// Deletes every file under `dir` (recursively) last modified before
/// `older_than`.
pub fn remove_files_older_than(dir: &Path, older_than: SystemTime) -> SweepReport {
    remove_files_where(dir, true, |_, modified| modified < older_than)
}

/// Deletes every file directly inside `dir`.
pub fn clear_files(dir: &Path) -> SweepReport {
    remove_files_where(dir, false, |_, _| true)
}

/// Runs once per launch on a background thread: rebrand repair, failed
/// recordings past the retention window, orphaned clips, stale temp
/// recordings and leftover editor copies.
pub fn run_startup_maintenance_with(
    state: &BackendState,
    pre_rebrand_dir: Option<PathBuf>,
    migration_complete: bool,
) {
    let dirs = state.dirs().clone();

    if let Some(old_dir) = pre_rebrand_dir {
        if let Err(error) =
            repair_rebrand_paths(state, &old_dir, &dirs.data_dir, migration_complete)
        {
            log::warn!("Could not repair pre-rename paths: {}", error.message);
        }
    }

    let now = SystemTime::now();
    let mut removed = SweepReport::default();

    match state.db().and_then(|db| db.get_settings()) {
        Ok(settings) => {
            if let Some(days) = settings.history_retention_days {
                let cutoff = now - Duration::from_secs(u64::from(days) * 24 * 60 * 60);
                removed.add(remove_files_older_than(&dirs.failed_recordings(), cutoff));
            }
        }
        Err(error) => log::warn!("Skipping failed-recording retention: {}", error.message),
    }

    match state.db().and_then(|db| db.referenced_clip_paths()) {
        Ok(referenced) => {
            removed.add(sweep_orphan_clips(
                &dirs.clips(),
                &referenced,
                now - ORPHAN_GRACE,
            ));
        }
        Err(error) => log::warn!("Skipping the orphaned-clip sweep: {}", error.message),
    }

    removed.add(remove_files_older_than(
        &dirs.temp_recordings(),
        now - STALE_TEMP_AGE,
    ));
    removed.add(clear_files(&dirs.editor_copies()));

    if removed.files > 0 {
        log::info!(
            "Startup cleanup removed {} files ({} MB)",
            removed.files,
            removed.bytes / 1_000_000
        );
    }
}

/// Clears everything "Clear history" promises beyond the transcript rows:
/// the Last Transcript Buffer, failed recordings and editor copies, then
/// compacts the database.
pub fn clear_history_leftovers(db: &Database, dirs: &DataDirs) -> Result<(), CommandError> {
    db.clear_last_transcript()?;
    clear_files(&dirs.failed_recordings());
    clear_files(&dirs.editor_copies());
    db.vacuum()
}

fn remove_files_where(
    dir: &Path,
    recursive: bool,
    mut matches: impl FnMut(&Path, SystemTime) -> bool,
) -> SweepReport {
    remove_files_matching(dir, recursive, &mut matches)
}

fn remove_files_matching(
    dir: &Path,
    recursive: bool,
    matches: &mut dyn FnMut(&Path, SystemTime) -> bool,
) -> SweepReport {
    let mut report = SweepReport::default();
    let Ok(entries) = fs::read_dir(dir) else {
        return report;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            if recursive {
                report.add(remove_files_matching(&path, true, matches));
            }
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if !matches(&path, modified) {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => {
                report.files += 1;
                report.bytes += metadata.len();
            }
            Err(error) => log::warn!("Could not remove {}: {}", path.display(), error),
        }
    }
    report
}

/// `path` relative to `dir`, compared case-insensitively because Windows
/// paths are, and stored paths may differ from the current folder in case.
fn relative_to(path: &Path, dir: &Path) -> Option<PathBuf> {
    if let Ok(relative) = path.strip_prefix(dir) {
        return (!relative.as_os_str().is_empty()).then(|| relative.to_path_buf());
    }
    let path_text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let dir_text = dir.to_string_lossy().replace('/', "\\").to_lowercase();
    let dir_text = dir_text.trim_end_matches('\\');
    let rest = path_text.strip_prefix(dir_text)?.strip_prefix('\\')?;
    if rest.is_empty() {
        return None;
    }
    // Take the relative part from the original string to keep its case.
    let original = path.to_string_lossy();
    let start = original.len() - rest.len();
    Some(PathBuf::from_iter(original[start..].split(['\\', '/'])))
}

fn file_name(path: &str) -> Option<String> {
    path.rsplit(['\\', '/'])
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// Same size and same bytes. Streams in chunks so large model files never
/// load into memory at once.
fn files_identical(a: &Path, b: &Path) -> bool {
    let (Ok(meta_a), Ok(meta_b)) = (fs::metadata(a), fs::metadata(b)) else {
        return false;
    };
    if !meta_a.is_file() || !meta_b.is_file() || meta_a.len() != meta_b.len() {
        return false;
    }
    let (Ok(mut file_a), Ok(mut file_b)) = (fs::File::open(a), fs::File::open(b)) else {
        return false;
    };
    let mut buffer_a = vec![0_u8; 1 << 20];
    let mut buffer_b = vec![0_u8; 1 << 20];
    loop {
        let Ok(read) = file_a.read(&mut buffer_a) else {
            return false;
        };
        if read == 0 {
            return true;
        }
        if file_b.read_exact(&mut buffer_b[..read]).is_err() || buffer_a[..read] != buffer_b[..read]
        {
            return false;
        }
    }
}

/// The directory Scribe treats as its data root: the user-chosen `data_dir`
/// when set, otherwise the OS app-data directory. Existing data is never moved
/// when this changes; only future data lands in the new location.
pub fn effective_data_dir_with(core: &Core) -> PathBuf {
    effective_data_dir_from(core.backend())
}

/// The user-chosen data folder stored in `state`'s settings when set (and
/// not blank), otherwise the app data folder `state` writes into
/// (`DataDirs::data_dir`, the folder Tauri resolved at startup).
fn effective_data_dir_from(state: &BackendState) -> PathBuf {
    if let Some(custom) = state
        .db()
        .ok()
        .and_then(|db| db.get_settings().ok())
        .and_then(|settings| settings.data_dir)
    {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    state.dirs().data_dir.clone()
}

/// The directory tauri-plugin-log writes rotating log files to (TargetKind::
/// LogDir resolves to app_log_dir()). Unlike the data/models folders this is
/// always the OS log dir and is not affected by the custom data_dir setting.
/// `DataDirs` has no log folder, so this is the host's (`Paths::log_dir`).
pub fn logs_dir_with(host: &dyn Host) -> PathBuf {
    host.paths().log_dir().to_path_buf()
}

/// The folder where audio from a dictation whose transcription failed is
/// quarantined so it isn't lost. This deliberately mirrors the location used by
/// dictation::quarantine_failed_recording — the OS app-data dir's `failed/`
/// subfolder, *not* the custom data_dir — so "Open" always lands on the real
/// quarantine folder rather than an empty one. It is the folder the backend's
/// `DataDirs` name, which "Clear history" and retention delete from.
pub fn failed_recordings_dir_with(core: &Core) -> PathBuf {
    core.backend().dirs().failed_recordings()
}

/// Creates `dir` when missing and opens it in the system file manager.
pub fn open_folder_with(host: &dyn Host, dir: PathBuf) -> Result<(), CommandError> {
    std::fs::create_dir_all(&dir).map_err(|error| {
        CommandError::new(
            "open_folder_failed",
            format!("Could not create folder {}. {}", dir.display(), error),
        )
    })?;

    host.open_path(&dir).map_err(|error| {
        CommandError::new(
            "open_folder_failed",
            format!("Could not open folder {}. {}", dir.display(), error),
        )
    })
}

/// Writes transcript `id`'s text to its "open in editor" copy (in the
/// backend's `DataDirs`, which "Clear history" and the startup clean-up
/// delete from) and opens it with the system's default text app (the
/// `open_transcript_externally` command).
pub fn open_transcript_externally_with(core: &Core, id: &str) -> Result<(), CommandError> {
    open_editor_copy(core.backend(), core.host().as_ref(), id)
}

/// The body of [`open_transcript_externally_with`], over the parts it uses.
fn open_editor_copy(state: &BackendState, host: &dyn Host, id: &str) -> Result<(), CommandError> {
    let transcript = state
        .db()?
        .get_transcript_by_id(id)?
        .ok_or_else(|| CommandError::new("transcript_not_found", "Transcript was not found."))?;

    let export_dir = state.dirs().editor_copies();
    fs::create_dir_all(&export_dir).map_err(|error| {
        CommandError::new(
            "transcript_export_failed",
            format!(
                "Could not create export folder {}. {}",
                export_dir.display(),
                error
            ),
        )
    })?;

    let path = state.dirs().editor_copy(id);
    fs::write(&path, transcript.text.as_bytes()).map_err(|error| {
        CommandError::new(
            "transcript_export_failed",
            format!(
                "Could not write transcript file {}. {}",
                path.display(),
                error
            ),
        )
    })?;

    host.open_path(&path).map_err(|error| {
        CommandError::new(
            "transcript_open_failed",
            format!(
                "Could not open transcript file {}. {}",
                path.display(),
                error
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Transcript;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn state_with(db: Database, root: &Path) -> BackendState {
        BackendState::new(
            db,
            root.join("cache").join("recordings"),
            DataDirs {
                data_dir: root.join("data"),
                cache_dir: root.join("cache"),
            },
        )
    }

    fn insert_with_audio(db: &Database, text: &str, audio: &str) -> Transcript {
        let mut transcript = Transcript::new_last_buffer(text, None, None, None).unwrap();
        transcript.audio_path = Some(audio.to_string());
        db.save_last_transcript(&transcript).unwrap();
        transcript
    }

    #[test]
    fn plan_only_repoints_identical_copies() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("com.natkins.localdictate");
        let new = root.path().join("com.natkins.scribe");
        write(&old.join("clips/a.wav"), b"same");
        write(&new.join("clips/a.wav"), b"same");
        write(&old.join("clips/b.wav"), b"old");
        write(&new.join("clips/b.wav"), b"new");
        write(&old.join("clips/c.wav"), b"only old");
        let stored = vec![
            old.join("clips/a.wav").to_string_lossy().into_owned(),
            old.join("clips/b.wav").to_string_lossy().into_owned(),
            old.join("clips/c.wav").to_string_lossy().into_owned(),
            new.join("clips/a.wav").to_string_lossy().into_owned(),
        ];

        let plan = plan_rebrand_repoints(&stored, &old, &new);

        assert_eq!(
            plan,
            vec![(
                old.join("clips/a.wav").to_string_lossy().into_owned(),
                new.join("clips/a.wav").to_string_lossy().into_owned()
            )]
        );
    }

    #[test]
    fn relative_to_matches_windows_paths_case_insensitively() {
        let dir = Path::new(r"C:\Users\you\AppData\Roaming\com.natkins.localdictate");
        let path =
            Path::new(r"c:\users\you\appdata\roaming\COM.NATKINS.LOCALDICTATE\clips\Tx_1.wav");
        assert_eq!(
            relative_to(path, dir),
            Some(PathBuf::from_iter(["clips", "Tx_1.wav"]))
        );
        assert_eq!(relative_to(dir, dir), None);
        assert_eq!(
            relative_to(
                Path::new(r"C:\Users\you\AppData\Roaming\com.natkins.localdictate2\x.wav"),
                dir
            ),
            None
        );
    }

    #[test]
    fn repair_repoints_models_clips_and_buffer_then_removes_the_old_folder() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("com.natkins.localdictate");
        let new = root.path().join("data");
        let old_model = old.join("models/ggml-base.en.bin");
        let old_clip = old.join("clips/tx_1.wav");
        write(&old_model, b"model bytes");
        write(&new.join("models/ggml-base.en.bin"), b"model bytes");
        write(&old_clip, b"clip bytes");
        write(&new.join("clips/tx_1.wav"), b"clip bytes");
        write(&old.join("localdictate.sqlite3"), b"old db");

        let db = Database::in_memory().unwrap();
        db.upsert_model_record(&crate::models::ModelRecord {
            id: "base.en".to_string(),
            name: "Base English".to_string(),
            filename: "ggml-base.en.bin".to_string(),
            local_path: Some(old_model.to_string_lossy().into_owned()),
            size_bytes: Some(11),
            status: crate::models::ModelStatus::Selected,
            checksum: None,
            selected: true,
            downloaded_at: None,
        })
        .unwrap();
        insert_with_audio(&db, "a dictation", &old_clip.to_string_lossy());
        let state = state_with(db, root.path());

        repair_rebrand_paths(&state, &old, &new, true).unwrap();

        let stored = state.db().unwrap().stored_file_paths().unwrap();
        assert!(!any_inside(&stored, &old));
        assert!(stored
            .iter()
            .any(|path| path == &new.join("models/ggml-base.en.bin").to_string_lossy()));
        let buffer = state.db().unwrap().get_last_transcript().unwrap().unwrap();
        assert_eq!(
            buffer.audio_path.as_deref(),
            Some(new.join("clips/tx_1.wav").to_string_lossy().as_ref())
        );
        assert!(!old.exists(), "old folder is removed once unreferenced");

        // A second run is a no-op.
        repair_rebrand_paths(&state, &old, &new, true).unwrap();
    }

    #[test]
    fn repair_keeps_the_old_folder_while_anything_still_points_into_it() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("com.natkins.localdictate");
        let new = root.path().join("data");
        let old_clip = old.join("clips/tx_1.wav");
        write(&old_clip, b"only here");
        let db = Database::in_memory().unwrap();
        insert_with_audio(&db, "a dictation", &old_clip.to_string_lossy());
        let state = state_with(db, root.path());

        repair_rebrand_paths(&state, &old, &new, true).unwrap();

        assert!(old_clip.exists());
    }

    #[test]
    fn repair_waits_for_the_copy_step_to_finish_before_deleting() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("com.natkins.localdictate");
        write(&old.join("localdictate.sqlite3"), b"old db");
        let state = state_with(Database::in_memory().unwrap(), root.path());

        repair_rebrand_paths(&state, &old, &root.path().join("data"), false).unwrap();

        assert!(old.exists());
    }

    #[test]
    fn orphan_sweep_keeps_referenced_and_recent_clips() {
        let root = tempfile::tempdir().unwrap();
        let clips = root.path().join("clips");
        write(&clips.join("kept.wav"), b"1");
        write(&clips.join("orphan.wav"), b"22");
        write(&clips.join("notes.txt"), b"not a clip");
        let referenced = vec![r"C:\elsewhere\clips\kept.wav".to_string()];

        // Everything is newer than the cutoff: nothing goes.
        let none = sweep_orphan_clips(&clips, &referenced, SystemTime::UNIX_EPOCH);
        assert_eq!(none, SweepReport::default());

        let future = SystemTime::now() + Duration::from_secs(60);
        let report = sweep_orphan_clips(&clips, &referenced, future);
        assert_eq!(report, SweepReport { files: 1, bytes: 2 });
        assert!(clips.join("kept.wav").exists());
        assert!(!clips.join("orphan.wav").exists());
        assert!(clips.join("notes.txt").exists());
    }

    #[test]
    fn age_sweep_recurses_and_respects_the_cutoff() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("recordings");
        write(&dir.join("a.wav"), b"a");
        write(&dir.join("session/seg-1.wav"), b"b");

        assert_eq!(
            remove_files_older_than(&dir, SystemTime::UNIX_EPOCH),
            SweepReport::default()
        );
        let report = remove_files_older_than(&dir, SystemTime::now() + Duration::from_secs(60));
        assert_eq!(report.files, 2);
        assert!(!dir.join("session/seg-1.wav").exists());
        // A missing folder is fine.
        assert_eq!(
            remove_files_older_than(&root.path().join("missing"), SystemTime::now()),
            SweepReport::default()
        );
    }

    #[test]
    fn clear_history_leftovers_empties_buffer_failed_and_editor_copies() {
        let root = tempfile::tempdir().unwrap();
        let dirs = DataDirs {
            data_dir: root.path().join("data"),
            cache_dir: root.path().join("cache"),
        };
        write(&dirs.failed_recordings().join("s1.wav"), b"x");
        write(&dirs.editor_copy("t1"), b"text");
        let db = Database::in_memory().unwrap();
        db.save_last_transcript(
            &Transcript::new_last_buffer("buffered", None, None, None).unwrap(),
        )
        .unwrap();

        clear_history_leftovers(&db, &dirs).unwrap();

        assert!(db.get_last_transcript().unwrap().is_none());
        assert!(!dirs.failed_recordings().join("s1.wav").exists());
        assert!(!dirs.editor_copy("t1").exists());
    }

    #[test]
    fn files_identical_compares_content() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        let c = root.path().join("c");
        write(&a, &vec![7_u8; 3 << 20]);
        write(&b, &vec![7_u8; 3 << 20]);
        let mut different = vec![7_u8; 3 << 20];
        *different.last_mut().unwrap() = 8;
        write(&c, &different);
        assert!(files_identical(&a, &b));
        assert!(!files_identical(&a, &c));
        assert!(!files_identical(&a, &root.path().join("missing")));
    }

    fn host_in(root: &Path) -> crate::host::RecordingHost {
        crate::host::RecordingHost::new(
            crate::paths::Paths::from_parts(
                root.join("app-data"),
                root.join("app-cache"),
                root.join("app-logs"),
                None,
                None,
            ),
            crate::host::Flavor::Stable,
        )
    }

    fn choose_data_dir(state: &BackendState, dir: &str) {
        let mut settings = state.db().unwrap().get_settings().unwrap();
        settings.data_dir = Some(dir.into());
        state.db().unwrap().save_settings(&settings).unwrap();
    }

    /// Writes `path` and dates it `age` ago.
    fn write_aged(path: &Path, age: Duration) {
        write(path, b"x");
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    #[test]
    fn the_data_folder_is_the_chosen_one_when_set_else_the_backends_data_folder() {
        let root = tempfile::tempdir().unwrap();
        let state = state_with(Database::in_memory().unwrap(), root.path());
        let app_data = root.path().join("data");
        assert_eq!(state.dirs().data_dir, app_data);
        assert_eq!(effective_data_dir_from(&state), app_data);

        for blank in ["", "   "] {
            choose_data_dir(&state, blank);
            assert_eq!(effective_data_dir_from(&state), app_data);
        }

        let chosen = root.path().join("chosen");
        choose_data_dir(&state, &format!(" {} ", chosen.display()));
        assert_eq!(effective_data_dir_from(&state), chosen);
    }

    /// The logs folder is the host's, and the failed-recordings folder is the
    /// backend's (where dictation writes and Clear history deletes); neither
    /// follows the data folder the user chose.
    #[test]
    fn logs_and_failed_recordings_ignore_the_chosen_data_folder() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let state = state_with(Database::in_memory().unwrap(), root.path());
        let chosen = root.path().join("chosen");
        choose_data_dir(&state, &chosen.to_string_lossy());
        assert_eq!(effective_data_dir_from(&state), chosen);

        assert_eq!(logs_dir_with(&host), root.path().join("app-logs"));
        // What failed_recordings_dir_with(core) answers.
        let failed = state.dirs().failed_recordings();
        assert_eq!(failed, root.path().join("data").join("failed"));
        assert!(!failed.starts_with(&chosen));
    }

    #[test]
    fn startup_maintenance_clears_editor_copies_and_stale_temp_recordings() {
        let root = tempfile::tempdir().unwrap();
        let state = state_with(Database::in_memory().unwrap(), root.path());
        let dirs = state.dirs().clone();
        write(&dirs.editor_copy("t1"), b"text");
        let stale = dirs.temp_recordings().join("old.wav");
        write_aged(&stale, STALE_TEMP_AGE + Duration::from_secs(60));
        let fresh = dirs.temp_recordings().join("new.wav");
        write(&fresh, b"x");

        run_startup_maintenance_with(&state, None, false);

        assert!(!dirs.editor_copy("t1").exists());
        assert!(!stale.exists());
        assert!(fresh.exists());
    }

    /// Failed recordings past the history retention window and clips no
    /// transcript points at go; recent ones and referenced clips stay.
    #[test]
    fn startup_maintenance_applies_retention_and_sweeps_orphaned_clips() {
        let root = tempfile::tempdir().unwrap();
        let state = state_with(Database::in_memory().unwrap(), root.path());
        let dirs = state.dirs().clone();
        let mut settings = state.db().unwrap().get_settings().unwrap();
        settings.history_retention_days = Some(7);
        state.db().unwrap().save_settings(&settings).unwrap();

        let old_failed = dirs.failed_recordings().join("old.wav");
        write_aged(&old_failed, DAY * 8);
        let new_failed = dirs.failed_recordings().join("new.wav");
        write_aged(&new_failed, DAY * 6);

        let referenced = dirs.clips().join("kept.wav");
        write_aged(&referenced, DAY * 2);
        insert_with_audio(&state.db().unwrap(), "kept", &referenced.to_string_lossy());
        let orphan = dirs.clips().join("orphan.wav");
        write_aged(&orphan, DAY * 2);
        let recent_orphan = dirs.clips().join("saving.wav");
        write(&recent_orphan, b"x");

        run_startup_maintenance_with(&state, None, false);

        assert!(!old_failed.exists());
        assert!(new_failed.exists());
        assert!(referenced.exists());
        assert!(!orphan.exists());
        assert!(recent_orphan.exists(), "inside the grace period");
    }

    /// A data folder the user chose is theirs: the startup pass never
    /// touches it, however old or unreferenced its files are.
    #[test]
    fn startup_maintenance_leaves_a_chosen_data_folder_alone() {
        let root = tempfile::tempdir().unwrap();
        let state = state_with(Database::in_memory().unwrap(), root.path());
        let chosen = root.path().join("chosen");
        let mut settings = state.db().unwrap().get_settings().unwrap();
        settings.data_dir = Some(chosen.to_string_lossy().into_owned());
        settings.history_retention_days = Some(7);
        state.db().unwrap().save_settings(&settings).unwrap();
        let files = [
            chosen.join("clips").join("orphan.wav"),
            chosen.join("failed").join("old.wav"),
            chosen.join("recordings").join("old.wav"),
            chosen.join("scribe-export").join("t1.txt"),
            chosen.join("notes.txt"),
        ];
        for file in &files {
            write_aged(file, DAY * 30);
        }

        run_startup_maintenance_with(&state, None, false);

        for file in &files {
            assert!(file.exists(), "{} was removed", file.display());
        }
    }

    #[test]
    fn open_folder_creates_the_folder_then_opens_it_through_the_host() {
        use crate::host::{Failing, HostCall};

        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let dir = root.path().join("logs").join("nested");
        open_folder_with(&host, dir.clone()).unwrap();
        assert!(dir.is_dir());
        assert_eq!(host.calls(), [HostCall::OpenPath(dir.clone())]);
        assert!(host.events().is_empty());

        host.fail(Failing::OpenPath);
        let error = open_folder_with(&host, dir.clone()).unwrap_err();
        assert_eq!(error.code, "open_folder_failed");
        assert_eq!(
            error.message,
            format!(
                "Could not open folder {}. {}",
                dir.display(),
                Failing::OpenPath.message()
            )
        );

        // A folder that cannot be created is never handed to the host.
        let blocker = root.path().join("a-file");
        fs::write(&blocker, b"x").unwrap();
        let error = open_folder_with(&host, blocker.join("below")).unwrap_err();
        assert_eq!(error.code, "open_folder_failed");
        assert!(error.message.starts_with("Could not create folder"));
        assert_eq!(host.calls().len(), 2);
    }

    /// "Open in editor" writes the copy into the backend's editor copies
    /// (never the chosen data folder), then opens it through the host once.
    #[test]
    fn opening_a_transcript_writes_its_editor_copy_then_opens_it_through_the_host() {
        use crate::host::{Failing, HostCall};

        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let state = state_with(Database::in_memory().unwrap(), root.path());
        choose_data_dir(&state, &root.path().join("chosen").to_string_lossy());
        let transcript = insert_with_audio(&state.db().unwrap(), "hello there", "clip.wav");
        let copy = state.dirs().editor_copy(&transcript.id);
        assert!(copy.starts_with(root.path().join("cache")));

        open_editor_copy(&state, &host, &transcript.id).unwrap();
        assert_eq!(fs::read_to_string(&copy).unwrap(), "hello there");
        assert_eq!(host.calls(), [HostCall::OpenPath(copy.clone())]);
        assert!(host.events().is_empty());

        host.fail(Failing::OpenPath);
        let error = open_editor_copy(&state, &host, &transcript.id).unwrap_err();
        assert_eq!(error.code, "transcript_open_failed");
        assert_eq!(
            error.message,
            format!(
                "Could not open transcript file {}. {}",
                copy.display(),
                Failing::OpenPath.message()
            )
        );
        assert_eq!(host.calls().len(), 2);
        assert!(host.events().is_empty());
    }

    /// A transcript that is missing, or whose copy cannot be written, is
    /// never handed to the host.
    #[test]
    fn a_transcript_that_cannot_be_copied_is_never_opened() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let state = state_with(Database::in_memory().unwrap(), root.path());

        let error = open_editor_copy(&state, &host, "no-such-id").unwrap_err();
        assert_eq!(error.code, "transcript_not_found");
        assert_eq!(error.message, "Transcript was not found.");

        let transcript = insert_with_audio(&state.db().unwrap(), "hello", "clip.wav");
        let copy = state.dirs().editor_copy(&transcript.id);
        fs::create_dir_all(&copy).unwrap();
        let error = open_editor_copy(&state, &host, &transcript.id).unwrap_err();
        assert_eq!(error.code, "transcript_export_failed");
        assert!(error.message.starts_with(&format!(
            "Could not write transcript file {}.",
            copy.display()
        )));
        fs::remove_dir(&copy).unwrap();

        let copies = state.dirs().editor_copies();
        fs::remove_dir(&copies).unwrap();
        write(&copies, b"a file where the folder goes");
        let error = open_editor_copy(&state, &host, &transcript.id).unwrap_err();
        assert_eq!(error.code, "transcript_export_failed");
        assert!(error.message.starts_with(&format!(
            "Could not create export folder {}.",
            copies.display()
        )));

        let poisoner = state.clone();
        let _ = std::thread::spawn(move || {
            let _db = poisoner.db().unwrap();
            panic!("poison the database lock");
        })
        .join();
        let error = open_editor_copy(&state, &host, &transcript.id).unwrap_err();
        assert_eq!(error.code, "database_lock_poisoned");

        assert!(host.calls().is_empty());
        assert!(host.events().is_empty());
    }
}
