//! Throwing an error away on purpose, and the log it goes to (review
//! 2026-11-04 r19 TST-160). An `is_err()` test with an empty block or an
//! irrefutable `let` of `Ok` and `Err` drop an error as silently as `let _ =`, and an
//! `eprintln!` of a window started without a console (`windows_subsystem`)
//! goes nowhere. [`dropped`] names why the error may go, and writes it with
//! its cause to the log: standard error, or the file a window gives
//! [`log_to_file`].

use ketchup_user_files::{UserFolder, append_private, create_private_folder};
use std::any::Any;
use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

/// Why an error may be thrown away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Whoever the value or reply was for stopped waiting: a receiver, a
    /// requester or a client that has gone.
    NobodyWaiting,
    /// The process, thread or lock acted on has ended already, or ends with
    /// this one.
    AlreadyEnded,
    /// A thread's panic, which the panic hook reported when it happened.
    PanicReported,
    /// The failure reaches the user another way: the status line, a reply,
    /// the next request.
    ReportedElsewhere,
    /// A cleanup that failed is done again the next time.
    RetriedLater,
    /// A value was set or registered already and stays; the new one is not
    /// needed.
    AlreadyHeld,
    /// Only a mark or a release the page makes anyway is missing.
    Cosmetic,
}

/// Throws away the error of `result`, if any, for `reason`, writing it to the
/// log.
pub fn dropped<T, E: fmt::Display>(result: Result<T, E>, reason: Reason) {
    if let Err(error) = result {
        log(format_args!("dropped ({reason:?}): {error}"));
    }
}

/// The panic a joined thread ended with, said by its message.
pub struct Panic(pub Box<dyn Any + Send>);

impl fmt::Display for Panic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = self
            .0
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| self.0.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("a panic without a message");
        write!(formatter, "the thread panicked: {message}")
    }
}

/// The value a cell or a register that holds one already gave back.
pub struct Held<T>(pub T);

impl<T> fmt::Display for Held<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a value is held already")
    }
}

/// Writes one line to the log: [`dropped`] and [`log!`](crate::log) use it. A log
/// file that grows past a megabyte is moved to `<file>.old` and begun again, while
/// the window runs (review 2026-11-06 P3: only when it opened).
pub fn log(line: fmt::Arguments<'_>) {
    let mut log = FILE.lock().unwrap_or_else(PoisonError::into_inner);
    let written = match log.as_mut() {
        Some(log) => {
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs());
            // Another window may log to the same file: what it wrote counts, and
            // when it moved the full log aside this window's file is the old one,
            // longer than full, so the line goes to the new log (review
            // 2026-11-06 r23 P3: it went on into `.old`).
            let len = log.file.metadata().map_or(0, |metadata| metadata.len());
            if len > log.full_at {
                match LogFile::open(&log.path, log.folder.as_ref()) {
                    Ok(begun) => *log = begun,
                    // Said in the full log, which is tried again a megabyte later.
                    Err(error) => {
                        log.full_at = len.saturating_add(LOG_LIMIT);
                        count_lost(writeln!(
                            log.file,
                            "[{seconds}] the full log could not be moved aside: {error}"
                        ));
                    }
                }
            }
            log.file
                .write_all(format!("[{seconds}] {line}\n").as_bytes())
        }
        None => writeln!(io::stderr(), "{line}"),
    };
    count_lost(written);
}

fn count_lost(written: io::Result<()>) {
    if written.is_err() {
        LOST.fetch_add(1, Ordering::Relaxed);
    }
}

/// The file the log goes to, if a window gave one; a window says where it is.
pub fn log_file() -> Option<PathBuf> {
    FILE.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|log| log.path.clone())
}

/// Lines the log could not take; nothing is left to say them to.
pub fn lost_lines() -> u64 {
    LOST.load(Ordering::Relaxed)
}

/// Sends the log to the end of `file` instead of standard error. A file
/// larger than a megabyte is first moved to `<file>.old`.
///
/// # Errors
///
/// The folder or the file cannot be made or opened; the log stays where it
/// was.
pub fn log_to_file(file: &Path) -> io::Result<()> {
    log_to(file, None)
}

/// [`log_to_file`] of `file` in the user's `folder`: under `sudo` the folder is
/// made and the file opened and moved aside as that user (review 2026-11-13
/// R1113-SEC-601: root did, through the user's links, appending to and closing
/// a file of root's).
///
/// # Errors
///
/// As [`log_to_file`], or the folder's user cannot be the one writing.
pub fn log_to_user_file(folder: &UserFolder, file: &str) -> io::Result<()> {
    log_to(&folder.path().join(file), Some(folder))
}

fn log_to(file: &Path, folder: Option<&UserFolder>) -> io::Result<()> {
    let opened = as_owner_of(folder, || {
        // The log names the documents a window opened: only its user reads it
        // (review 2026-11-06 P3: it was made readable to everyone).
        if let Some(folder) = file.parent() {
            create_private_folder(folder)?;
        }
        Ok(())
    })
    .and_then(|()| LogFile::open(file, folder))?;
    *FILE.lock().unwrap_or_else(PoisonError::into_inner) = Some(opened);
    Ok(())
}

/// `work` as the user of `folder`, or as this process is without one.
fn as_owner_of<T>(
    folder: Option<&UserFolder>,
    work: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    match folder {
        Some(folder) => folder.as_owner(work),
        None => work(),
    }
}

/// The open log file, where it is, the length past which it is moved aside and
/// the user's folder it is in.
struct LogFile {
    file: File,
    path: PathBuf,
    full_at: u64,
    folder: Option<UserFolder>,
}

impl LogFile {
    /// Opens `path` to append to, moving it to `<path>.old` first when it is larger
    /// than a megabyte; only its user may read it, and no link in its place is
    /// followed.
    fn open(path: &Path, folder: Option<&UserFolder>) -> io::Result<Self> {
        let file = as_owner_of(folder, || {
            if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.len() > LOG_LIMIT) {
                let mut old = path.as_os_str().to_owned();
                old.push(".old");
                match std::fs::rename(path, old) {
                    // Another window moved it aside first (review 2026-11-06 r23 P3:
                    // this window was left without a log).
                    Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                    _ => {}
                }
            }
            append_private(path)
        })?;
        Ok(Self {
            file,
            path: path.to_owned(),
            full_at: LOG_LIMIT,
            folder: folder.cloned(),
        })
    }
}

const LOG_LIMIT: u64 = 1 << 20;
static FILE: Mutex<Option<LogFile>> = Mutex::new(None);
static LOST: AtomicU64 = AtomicU64::new(0);

/// A window built for release has no console to read: its log goes to `file` in
/// the user's log folder `folder` (`%LOCALAPPDATA%\<folder>` on Windows,
/// `~/Library/Logs/<folder>` on macOS and `$XDG_STATE_HOME/<folder>` or
/// `~/.local/state/<folder>` elsewhere), or stays on standard error, which says why.
pub fn log_window_to(folder: &str, file: &str) {
    if cfg!(debug_assertions) {
        return;
    }
    if let Some(folder) = UserFolder::logs(folder)
        && let Err(error) = log_to_user_file(&folder, file)
    {
        log(format_args!(
            "the log {} could not be opened: {error}",
            folder.path().join(file).display()
        ));
    }
}

/// Writes a line to the log, formatted as `format!` does.
#[macro_export]
macro_rules! log {
    ($($argument:tt)*) => {
        $crate::dropped::log(format_args!($($argument)*))
    };
}
