//! What a connected AI wrote, listed in the Connect tab in place of a question
//! before each write (owner decision R1 of review 2026-11-19 og): the file's
//! path, when it was written and its size, with Open folder beside it.

use super::Response;
use crate::delivery::Deliver;
use eframe::egui;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::SystemTime;

/// The most writes the list keeps, the newest first. A write is every file one
/// answer named, so one set of more sheets than this does not push out every
/// write before it (review 2026-11-21 ac P3).
const KEPT: usize = 100;

/// The most entries of a written file's folder read for the files beside it a
/// write may change.
const READ_ENTRIES: usize = 4_096;

/// A file a client wrote or removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WrittenFile {
    pub(crate) path: PathBuf,
    /// The local time it was written, `HH:MM:SS`.
    pub(crate) at: String,
    /// Its size; none for a folder, such as a DXF set, or a file removed.
    pub(crate) bytes: Option<u64>,
    /// The write removed it, as an old sheet beyond a set's new last.
    pub(crate) removed: bool,
}

/// The files the window's clients wrote, one list per write, shared with the
/// threads that see their requests answered.
#[derive(Clone, Default)]
pub(crate) struct WriteLog(Arc<Mutex<VecDeque<Vec<WrittenFile>>>>);

impl WriteLog {
    /// The files written, the newest write first, each write's in the order
    /// its answer named them.
    pub(crate) fn files(&self) -> Vec<WrittenFile> {
        self.files_now().iter().flatten().cloned().collect()
    }

    /// The list; a thread that panicked holding it left it whole, so it is
    /// read on.
    fn files_now(&self) -> std::sync::MutexGuard<'_, VecDeque<Vec<WrittenFile>>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The sender a request writing `path` answers on in place of `reply`:
    /// the answer goes on to `reply` as it came, and when it is a success the
    /// files it says it wrote or removed are listed ([`answered_files`]), else
    /// `path` when the answer says it was saved or the file there is another
    /// than before, with the work-recovery checkpoint beside a file written.
    /// For a refusal, every file of [`stamps`] another than before or gone:
    /// an export that failed and could not put back what it wrote left them,
    /// but not the file the refusal says another program changed. A request
    /// answered never drops its `reply` unanswered.
    pub(super) fn watch(
        &self,
        path: PathBuf,
        reply: mpsc::SyncSender<Response>,
        context: egui::Context,
    ) -> mpsc::SyncSender<Response> {
        let (watched, answers) = mpsc::sync_channel::<Response>(1);
        let before = stamps(&path);
        let log = self.clone();
        let forward = reply.clone();
        let shown = path.display().to_string();
        let spawned = std::thread::Builder::new()
            .name("ketchup-write-log".to_owned())
            .spawn(move || {
                // No answer: the request was dropped, and so is `forward`.
                let Some(answer) = answers.iter().next() else {
                    return;
                };
                let (files, removed) = if answer.ok {
                    succeeded(&path, &before, answer.result.as_ref())
                } else {
                    refused(&path, &before, answer.result.as_ref())
                };
                if log.record_write(&files, &removed) {
                    context.request_repaint();
                }
                forward.deliver(answer);
            });
        match spawned {
            Ok(_) => watched,
            Err(error) => {
                // The request answers its client unlisted.
                ketchup_rejection::log!("{shown} is not listed when written: {error}");
                reply
            }
        }
    }

    /// Lists the file or folder at `path` as written now.
    #[cfg(test)]
    pub(crate) fn record(&self, path: &Path) {
        self.record_write(&[path.to_path_buf()], &[]);
    }

    /// Lists one write: the files and folders it `wrote` and the files it
    /// `removed`, as now; false when it named none.
    fn record_write(&self, wrote: &[PathBuf], removed: &[PathBuf]) -> bool {
        if wrote.is_empty() && removed.is_empty() {
            return false;
        }
        let at = local_clock(
            SystemTime::now(),
            crate::app::local_time::utc_offset_seconds(),
        );
        let listed = |path: &PathBuf, removed: bool| WrittenFile {
            path: path.clone(),
            at: at.clone(),
            bytes: std::fs::metadata(path)
                .ok()
                .and_then(|metadata| metadata.is_file().then_some(metadata.len()))
                .filter(|_| !removed),
            removed,
        };
        let write = wrote
            .iter()
            .map(|path| listed(path, false))
            .chain(removed.iter().map(|path| listed(path, true)))
            .collect();
        let mut writes = self.files_now();
        writes.push_front(write);
        writes.truncate(KEPT);
        true
    }
}

/// What tells one write of `path` from another: when it was last modified and
/// its size; none when nothing is there.
fn stamp(path: &Path) -> Option<Stamp> {
    std::fs::metadata(path)
        .ok()
        .map(|metadata| (metadata.modified().ok(), metadata.len()))
}

type Stamp = (Option<SystemTime>, u64);

/// The [`stamp`] of `path` and of every file in its folder whose name starts as
/// the name of `path` without its extension does: a set's sheets and list, the
/// reports and companions beside an export, a document's work-recovery
/// checkpoint (R1122ac, review §4 P3: only the path asked for was compared). At
/// most [`READ_ENTRIES`] entries of the folder are read.
fn stamps(path: &Path) -> BTreeMap<PathBuf, Stamp> {
    let mut stamps = BTreeMap::new();
    if let Some(stamp) = stamp(path) {
        stamps.insert(path.to_path_buf(), stamp);
    }
    let (Some(folder), Some(stem)) = (path.parent(), path.file_stem()) else {
        return stamps;
    };
    let stem = stem.to_string_lossy();
    // A folder that does not read has no files beside the path to compare.
    let entries = std::fs::read_dir(folder).ok().into_iter().flatten();
    for entry in entries.take(READ_ENTRIES).flatten() {
        if entry.file_name().to_string_lossy().starts_with(&*stem)
            && let Ok(metadata) = entry.metadata()
            && metadata.is_file()
        {
            stamps.insert(entry.path(), (metadata.modified().ok(), metadata.len()));
        }
    }
    stamps
}

/// The files of [`stamps`] of `path` another than `before` or new, `path`
/// first, and those gone.
fn changed_since(path: &Path, before: &BTreeMap<PathBuf, Stamp>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let after = stamps(path);
    let mut written: Vec<PathBuf> = after
        .iter()
        .filter(|(file, stamp)| before.get(*file) != Some(stamp))
        .map(|(file, _)| file.clone())
        .collect();
    written.sort_by_key(|file| file != path);
    let removed = before
        .keys()
        .filter(|file| !after.contains_key(*file))
        .cloned()
        .collect();
    (written, removed)
}

/// The files a successful request writing `path` wrote and removed: those its
/// answer names, else `path` when it was saved or changed, and the
/// work-recovery checkpoint beside any of them written or removed since
/// `before`.
fn succeeded(
    path: &Path,
    before: &BTreeMap<PathBuf, Stamp>,
    result: Option<&Value>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let (mut files, mut removed) = answered_files(result);
    let (changed, gone) = changed_since(path, before);
    // A save of the same size within the clock of a FAT disk leaves the stamp
    // as it was, so the answer is believed.
    let saved = result.is_some_and(|result| result["saved"] == true);
    if files.is_empty() && (saved || changed.first().is_some_and(|file| file == path)) {
        files.push(path.to_path_buf());
    }
    let checkpoints = files
        .iter()
        .map(|file| {
            let mut checkpoint = file.clone().into_os_string();
            checkpoint.push(".work-recovery");
            PathBuf::from(checkpoint)
        })
        .collect::<Vec<_>>();
    files.extend(
        changed
            .into_iter()
            .filter(|file| checkpoints.contains(file)),
    );
    removed.extend(gone.into_iter().filter(|file| checkpoints.contains(file)));
    (files, removed)
}

/// The files a refused request writing `path` left another than `before` or
/// removed, but not the one its refusal says another program changed.
fn refused(
    path: &Path,
    before: &BTreeMap<PathBuf, Stamp>,
    result: Option<&Value>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let another = result
        .and_then(|result| result["details"]["changed_by_another"].as_str())
        .map(PathBuf::from);
    let (mut files, mut removed) = changed_since(path, before);
    files.retain(|file| Some(file) != another.as_ref());
    removed.retain(|file| Some(file) != another.as_ref());
    (files, removed)
}

/// The files a successful answer says its request wrote, as the exporter
/// named them (review 2026-11-20 ac P3: the log knew only the path asked for,
/// so a DXF set, its list, the reports and companions beside an export and a
/// variant's takeoff went unlisted): a document saved where `save_path` says,
/// and for an export its `files` (a set's sheets) or else its `path`, with
/// `set_list`, `loss_report`, `support_report`, `companions`, `cut_list_path`
/// and `takeoff_path`; then the files it says it removed, a set's
/// `removed_files`.
fn answered_files(result: Option<&Value>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let Some(result) = result else {
        return (Vec::new(), Vec::new());
    };
    let named = |key: &str| result.get(key).and_then(Value::as_str).map(PathBuf::from);
    let listed = |key: &str| {
        result
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
    };
    let mut files = Vec::new();
    let mut removed = Vec::new();
    if result["saved"] == true {
        files.extend(named("save_path"));
    }
    // `exported` is true, or how many variants were.
    let exported = &result["exported"];
    if *exported == true || exported.as_u64().is_some_and(|count| count > 0) {
        removed.extend(listed("removed_files"));
        if result.get("files").is_some() {
            files.extend(listed("files"));
        } else {
            files.extend(named("path"));
        }
        for key in [
            "set_list",
            "loss_report",
            "support_report",
            "cut_list_path",
            "takeoff_path",
        ] {
            files.extend(named(key));
        }
        files.extend(listed("companions"));
    }
    (files, removed)
}

/// `time` on a clock `offset` seconds ahead of UTC, `HH:MM:SS`.
fn local_clock(time: SystemTime, offset: i64) -> String {
    let utc = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0));
    let seconds = (utc + offset).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// `bytes` as a person reads a file's size.
pub(crate) fn shown_size(bytes: u64) -> String {
    const KIB: u64 = 1_024;
    // Tenths of `unit`, rounded half up, in integers.
    let scaled = |unit: u64| {
        let tenths = (u128::from(bytes) * 10 + u128::from(unit) / 2) / u128::from(unit);
        format!("{}.{}", tenths / 10, tenths % 10)
    };
    match bytes {
        0..KIB => format!("{bytes} B"),
        KIB..1_048_576 => format!("{} kB", scaled(KIB)),
        _ => format!("{} MB", scaled(KIB * KIB)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_is_the_users_and_the_size_is_readable() {
        let noon =
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86_400 * 3 + 12 * 3_600 + 5);
        assert_eq!(local_clock(noon, 3_600), "13:00:05");
        assert_eq!(local_clock(noon, -13 * 3_600), "23:00:05");
        assert_eq!(shown_size(512), "512 B");
        assert_eq!(shown_size(2_048), "2.0 kB");
        assert_eq!(shown_size(1_100), "1.1 kB");
        assert_eq!(shown_size(3 * 1_048_576 / 2), "1.5 MB");
    }

    /// The answer reaches the client as it came; an answer that left a new
    /// file is listed, a success or a refusal alike, a file untouched is not.
    #[test]
    fn a_request_answered_is_listed_only_when_it_wrote_its_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("house.dxf");
        let log = WriteLog::default();
        let answer = |ok: bool, write: Option<&[u8]>| {
            let (reply, client) = mpsc::sync_channel(1);
            let watched = log.watch(path.clone(), reply, egui::Context::default());
            if let Some(bytes) = write {
                std::fs::write(&path, bytes).unwrap();
            }
            let mut response = Response::error(7, "export_failed");
            response.ok = ok;
            watched.deliver(response);
            // The file is listed before the answer goes on.
            let delivered = client.recv().unwrap();
            assert_eq!((delivered.id, delivered.ok), (7, ok));
            log.files()
        };
        assert!(answer(false, None).is_empty(), "a refusal");
        let files = answer(true, Some(b"written 123"));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, path);
        assert_eq!(files[0].bytes, Some(11));
        assert_eq!(answer(true, None).len(), 1, "the file untouched");
        // A failed export that could not put back the file it changed is
        // listed: the file there is not the one before.
        let files = answer(false, Some(b"left half written"));
        assert_eq!(files.len(), 2);
        assert_eq!(
            (files[0].path.as_path(), files[0].bytes),
            (path.as_path(), Some(17))
        );
    }

    /// Every file an answer says was written is listed, the first named
    /// first: a DXF set's sheets and its list (the path asked for is no file
    /// of a set), the reports and companions beside an export, a variant
    /// export's two tables and a document saved by apply_and_verify.
    #[test]
    fn the_files_an_answer_names_are_listed_not_only_the_path_asked_for() {
        let directory = tempfile::tempdir().unwrap();
        let log = WriteLog::default();
        let answered = |path: &str, result: serde_json::Value| {
            let (reply, client) = mpsc::sync_channel(1);
            let watched = log.watch(directory.path().join(path), reply, egui::Context::default());
            let mut response = Response::error(1, "unused");
            response.ok = true;
            response.result = Some(result);
            watched.deliver(response);
            client.recv().unwrap();
            let files = log.files();
            log.files_now().clear();
            files
                .iter()
                .map(|file| file.path.display().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            answered(
                "plan.dxf",
                serde_json::json!({"exported": true, "path": "plan.dxf",
                    "files": ["plan-01.dxf", "plan-02.dxf"], "set_list": "plan.sheets.txt"}),
            ),
            ["plan-01.dxf", "plan-02.dxf", "plan.sheets.txt"]
        );
        assert_eq!(
            answered(
                "x.btlx",
                serde_json::json!({"exported": true, "path": "x.btlx", "loss_report": null,
                    "support_report": "x.btlx.support.txt", "companions": ["x.drawings.svg"]}),
            ),
            ["x.btlx", "x.btlx.support.txt", "x.drawings.svg"]
        );
        assert_eq!(
            answered(
                "cabinet.csv",
                serde_json::json!({"exported": 2, "cut_list_path": "cabinet.csv",
                    "takeoff_path": "cabinet.takeoff.csv"}),
            ),
            ["cabinet.csv", "cabinet.takeoff.csv"]
        );
        assert_eq!(
            answered(
                "a.ketchup",
                serde_json::json!({"saved": true, "save_path": "a.ketchup"}),
            ),
            ["a.ketchup"]
        );
        assert!(
            answered(
                "v.csv",
                serde_json::json!({"exported": 0, "takeoff_path": "v.csv"})
            )
            .is_empty(),
            "no variant exported"
        );
    }

    /// The list keeps the last writes, not the last files: one set of more
    /// sheets than it keeps does not push out every write before it, and the
    /// old sheets the set removed are listed as removed (review 2026-11-21 ac
    /// P3).
    #[test]
    fn a_large_set_keeps_the_writes_before_it_and_lists_what_it_removed() {
        let directory = tempfile::tempdir().unwrap();
        let log = WriteLog::default();
        log.record(&directory.path().join("before.step"));
        let sheets = (1..=KEPT + 20)
            .map(|sheet| format!("plan-{sheet:03}.dxf"))
            .collect::<Vec<_>>();
        let (reply, client) = mpsc::sync_channel(1);
        let watched = log.watch(
            directory.path().join("plan.dxf"),
            reply,
            egui::Context::default(),
        );
        let mut response = Response::error(1, "unused");
        response.ok = true;
        response.result = Some(serde_json::json!({"exported": true, "path": "plan.dxf",
            "files": sheets, "removed_files": ["plan-999.dxf"]}));
        watched.deliver(response);
        client.recv().unwrap();
        let files = log.files();
        assert_eq!(files.len(), KEPT + 20 + 2);
        assert_eq!(files[0].path, Path::new("plan-001.dxf"));
        let removed = &files[KEPT + 20];
        assert_eq!(
            (removed.path.as_path(), removed.removed),
            (Path::new("plan-999.dxf"), true)
        );
        assert!(!files[0].removed);
        assert_eq!(files[KEPT + 21].path, directory.path().join("before.step"));
    }

    /// R1122ac, review §4 P3: a refused export lists every file beside its path
    /// it left another or removed (a set's sheets and list, a report), not only
    /// the path asked for, and not the file the refusal says another program
    /// changed; a save lists the work-recovery checkpoint it removed.
    #[test]
    fn a_refusal_lists_the_files_beside_its_path_and_not_another_programs() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path();
        let path = folder.join("plan.dxf");
        for name in ["plan-01.dxf", "plan-02.dxf", "other.dxf"] {
            std::fs::write(folder.join(name), b"before").unwrap();
        }
        let log = WriteLog::default();
        let answer = |ok: bool, result: Value, write: &dyn Fn()| {
            log.files_now().clear();
            let (reply, client) = mpsc::sync_channel(1);
            let watched = log.watch(path.clone(), reply, egui::Context::default());
            write();
            let mut response = Response::error(1, "export_failed");
            response.ok = ok;
            response.result = Some(result);
            watched.deliver(response);
            client.recv().unwrap();
            log.files()
                .iter()
                .map(|file| (file.path.clone(), file.removed))
                .collect::<Vec<_>>()
        };
        let failed = serde_json::json!({"code": "export_failed"});
        let files = answer(false, failed, &|| {
            std::fs::write(folder.join("plan-01.dxf"), b"half written").unwrap();
            std::fs::write(folder.join("plan.dxf.sheets.txt"), b"list").unwrap();
            std::fs::remove_file(folder.join("plan-02.dxf")).unwrap();
            std::fs::write(folder.join("other.dxf"), b"another program's").unwrap();
        });
        assert_eq!(
            files,
            [
                (folder.join("plan-01.dxf"), false),
                (folder.join("plan.dxf.sheets.txt"), false),
                (folder.join("plan-02.dxf"), true),
            ]
        );
        let another = serde_json::json!({"code": "export_failed",
            "details": {"changed_by_another": path.display().to_string()}});
        let files = answer(false, another, &|| {
            std::fs::write(&path, b"theirs").unwrap()
        });
        assert!(files.is_empty(), "{files:?}");
        let document = folder.join("plan.ketchup");
        let checkpoint = folder.join("plan.ketchup.work-recovery");
        std::fs::write(&checkpoint, b"steps").unwrap();
        let saved = serde_json::json!({"saved": true, "save_path": document.display().to_string()});
        let files = answer(true, saved, &|| {
            std::fs::write(&document, b"saved").unwrap();
            std::fs::remove_file(&checkpoint).unwrap();
        });
        assert_eq!(files, [(document.clone(), false), (checkpoint, true)]);
    }

    /// A save the answer reports is listed even when the file's stamp did not
    /// change, as a save of the same size within a FAT disk's two-second clock.
    #[test]
    fn a_save_is_listed_when_the_stamp_stays_the_same() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a.ketchup");
        std::fs::write(&path, b"same").unwrap();
        let log = WriteLog::default();
        let (reply, client) = mpsc::sync_channel(1);
        let watched = log.watch(path.clone(), reply, egui::Context::default());
        let mut response = Response::error(1, "unused");
        response.ok = true;
        response.result = Some(serde_json::json!({"saved": true, "dirty": false}));
        watched.deliver(response);
        client.recv().unwrap();
        let files = log.files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, path);
    }
}
