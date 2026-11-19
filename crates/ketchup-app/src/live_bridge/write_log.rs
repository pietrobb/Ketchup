//! What a connected AI wrote, listed in the Connect tab in place of a question
//! before each write (owner decision R1 of review 2026-11-19 og): the file's
//! path, when it was written and its size, with Open folder beside it.

use super::Response;
use crate::delivery::Deliver;
use eframe::egui;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::SystemTime;

/// The most files the list keeps, the newest first.
const KEPT: usize = 100;

/// A file a client wrote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WrittenFile {
    pub(crate) path: PathBuf,
    /// The local time it was written, `HH:MM:SS`.
    pub(crate) at: String,
    /// Its size; none for a folder, such as a DXF set.
    pub(crate) bytes: Option<u64>,
}

/// The files the window's clients wrote, shared with the threads that see
/// their requests answered.
#[derive(Clone, Default)]
pub(crate) struct WriteLog(Arc<Mutex<VecDeque<WrittenFile>>>);

impl WriteLog {
    /// The files written, the newest first.
    pub(crate) fn files(&self) -> Vec<WrittenFile> {
        self.files_now().iter().cloned().collect()
    }

    /// The list; a thread that panicked holding it left it whole, so it is
    /// read on.
    fn files_now(&self) -> std::sync::MutexGuard<'_, VecDeque<WrittenFile>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The sender a request writing `path` answers on in place of `reply`:
    /// the answer goes on to `reply` as it came, and when it is a success that
    /// left a file at `path` other than the one there before, the file is
    /// listed. A request answered never drops its `reply` unanswered.
    pub(super) fn watch(
        &self,
        path: PathBuf,
        reply: mpsc::SyncSender<Response>,
        context: egui::Context,
    ) -> mpsc::SyncSender<Response> {
        let (watched, answers) = mpsc::sync_channel::<Response>(1);
        let before = stamp(&path);
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
                if answer.ok && stamp(&path).is_some_and(|after| Some(after) != before) {
                    log.record(&path);
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
    pub(crate) fn record(&self, path: &Path) {
        let bytes = std::fs::metadata(path)
            .ok()
            .and_then(|metadata| metadata.is_file().then_some(metadata.len()));
        let file = WrittenFile {
            path: path.to_path_buf(),
            at: local_clock(
                SystemTime::now(),
                crate::app::local_time::utc_offset_seconds(),
            ),
            bytes,
        };
        let mut files = self.files_now();
        files.push_front(file);
        files.truncate(KEPT);
    }
}

/// What tells one write of `path` from another: when it was last modified and
/// its size; none when nothing is there.
fn stamp(path: &Path) -> Option<(Option<SystemTime>, u64)> {
    std::fs::metadata(path)
        .ok()
        .map(|metadata| (metadata.modified().ok(), metadata.len()))
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

    /// The answer reaches the client as it came; only a success that left a
    /// new file is listed, a refusal or a file untouched is not.
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
        assert!(answer(false, Some(b"refused")).is_empty(), "a refusal");
        let files = answer(true, Some(b"written 123"));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, path);
        assert_eq!(files[0].bytes, Some(11));
        assert_eq!(answer(true, None).len(), 1, "the file untouched");
    }
}
