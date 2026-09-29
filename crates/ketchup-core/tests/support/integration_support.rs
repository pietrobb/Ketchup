//! Shared by every crate's single integration test binary (`tests/integration.rs`,
//! `autotests = false`): the check that no `tests/*.rs` file is left out of it, and
//! the file turns that keep one process from mixing tests written for their own.
// Each crate uses only part of this module.
#![allow(dead_code)]

use std::cell::Cell;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::{collections::BTreeSet, fs, path::Path};

/// libtest name of `function` declared in the module whose `module_path!()` is
/// given; test names omit the leading crate segment.
pub fn test_name(module_path: &str, function: &str) -> String {
    match module_path.split_once("::") {
        Some((_, module)) => format!("{module}::{function}"),
        None => function.to_owned(),
    }
}

pub fn assert_every_test_file_is_registered(tests_dir: &Path, root_source: &str) {
    let declared: BTreeSet<&str> = root_source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("mod ")?.strip_suffix(';'))
        .collect();
    let unregistered: Vec<String> = fs::read_dir(tests_dir)
        .expect("read the crate's tests directory")
        .map(|entry| entry.expect("read tests directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .filter_map(|path| Some(path.file_stem()?.to_str()?.to_owned()))
        .filter(|name| name != "integration" && !declared.contains(name.as_str()))
        .collect();
    assert!(
        unregistered.is_empty(),
        "declare these files as `mod <name>;` in tests/integration.rs: {unregistered:?}"
    );
}

/// All integration tests of a crate share one process, but many were written for
/// one process per test file: exact worker and GPU deadlines, process-tree memory.
/// A test that drives the app or starts external processes takes its file's turn.
/// Tests of one file run together; a test of another file waits until they are done. libtest starts tests
/// in name order and names begin with the file's module, so a file's tests have all
/// started before the next file's. Threads that are not libtest test threads, and
/// threads already holding a turn, never wait.
struct FileTurns {
    file: Option<String>,
    holders: usize,
}

static FILE_TURNS: Mutex<FileTurns> = Mutex::new(FileTurns {
    file: None,
    holders: 0,
});
static FILE_TURN_RELEASED: Condvar = Condvar::new();

thread_local! {
    static TURNS_HELD: Cell<usize> = const { Cell::new(0) };
}

pub struct FileTurn(());

impl Drop for FileTurn {
    fn drop(&mut self) {
        let mut turns = FILE_TURNS.lock().unwrap_or_else(PoisonError::into_inner);
        turns.holders -= 1;
        if turns.holders == 0 {
            turns.file = None;
        }
        TURNS_HELD.set(TURNS_HELD.get() - 1);
        FILE_TURN_RELEASED.notify_all();
    }
}

/// Hold for as long as the test runs; the app test harness's `Shell` takes one itself.
pub fn file_turn() -> FileTurn {
    let thread = std::thread::current();
    let file = thread
        .name()
        .and_then(|test| test.rsplit_once("::"))
        .map(|(file, _)| file.to_owned());
    let mut turns = FILE_TURNS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(file) = file {
        if TURNS_HELD.get() == 0 {
            while turns.file.as_ref().is_some_and(|current| *current != file) {
                turns = wait(turns);
            }
        }
        turns.file.get_or_insert(file);
    }
    turns.holders += 1;
    TURNS_HELD.set(TURNS_HELD.get() + 1);
    FileTurn(())
}

fn wait(turns: MutexGuard<'static, FileTurns>) -> MutexGuard<'static, FileTurns> {
    FILE_TURN_RELEASED
        .wait(turns)
        .unwrap_or_else(PoisonError::into_inner)
}
