//! Throwing an error away on purpose, and the log it goes to (review
//! 2026-11-04 r19 TST-160). An `is_err()` test with an empty block or an
//! irrefutable `let` of `Ok` and `Err` drop an error as silently as `let _ =`, and an
//! `eprintln!` of a window started without a console (`windows_subsystem`)
//! goes nowhere. [`dropped`] names why the error may go, and writes it with
//! its cause to the log: standard error, or the file a window gives
//! [`log_to_file`].

use std::any::Any;
use std::fmt;
use std::fs::{File, OpenOptions};
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

/// Writes one line to the log: [`dropped`] and [`log!`](crate::log) use it.
pub fn log(line: fmt::Arguments<'_>) {
    let mut file = FILE.lock().unwrap_or_else(PoisonError::into_inner);
    let written = match file.as_mut() {
        Some(file) => {
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs());
            writeln!(file, "[{seconds}] {line}")
        }
        None => writeln!(io::stderr(), "{line}"),
    };
    if written.is_err() {
        LOST.fetch_add(1, Ordering::Relaxed);
    }
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
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    if std::fs::metadata(file).is_ok_and(|metadata| metadata.len() > LOG_LIMIT) {
        let mut old = file.as_os_str().to_owned();
        old.push(".old");
        std::fs::rename(file, old)?;
    }
    let opened = OpenOptions::new().create(true).append(true).open(file)?;
    *FILE.lock().unwrap_or_else(PoisonError::into_inner) = Some(opened);
    Ok(())
}

/// Where a window keeps its log: `%LOCALAPPDATA%\<folder>\<file>` on
/// Windows, `~/Library/Logs/<folder>/<file>` on macOS and
/// `$XDG_STATE_HOME/<folder>/<file>` or `~/.local/state/<folder>/<file>`
/// elsewhere; none when the platform names no such folder.
pub fn user_log_file(folder: &str, file: &str) -> Option<PathBuf> {
    let var = |name| std::env::var_os(name).map(PathBuf::from);
    let base = if cfg!(windows) {
        var("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library").join("Logs"))
    } else {
        var("XDG_STATE_HOME")
            .filter(|base| base.is_absolute())
            .or_else(|| var("HOME").map(|home| home.join(".local").join("state")))
    };
    base.filter(|base| base.is_absolute())
        .map(|base| base.join(folder).join(file))
}

const LOG_LIMIT: u64 = 1 << 20;
static FILE: Mutex<Option<File>> = Mutex::new(None);
static LOST: AtomicU64 = AtomicU64::new(0);

/// A window built for release has no console to read: its log goes to
/// [`user_log_file`] of `folder`, or stays on standard error, which says why.
pub fn log_window_to(folder: &str, file: &str) {
    if cfg!(debug_assertions) {
        return;
    }
    if let Some(path) = user_log_file(folder, file)
        && let Err(error) = log_to_file(&path)
    {
        log(format_args!(
            "the log {} could not be opened: {error}",
            path.display()
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
