//! A program applied to a document runs in the program worker: a program that
//! takes all the memory is refused with `memory_limit` and the session that
//! applied it (the window) keeps working (review PRG-M1). The checks of its
//! model run there too, under the same limits (review PRG-1).
use crate::operations_support::{BLOCK, apply_error, session};
use std::time::{Duration, Instant};

#[test]
fn a_program_past_the_memory_limit_is_refused_and_the_session_lives_on() {
    let _turn = crate::integration_support::file_turn();
    let error = apply_error("s = \"ab\" * 50000\nt = s.replace(\"\", s)\n");
    assert!(error.contains("GiB of memory and was stopped"), "{error}");
    session(BLOCK).unwrap();
}

/// 400 boxes that all overlap evaluate in a fraction of a second, but checking
/// their 79 800 pairs takes over two minutes in a debug build: the caller waits
/// no longer than the worker's 60 s deadline for a report.
#[test]
fn a_model_too_large_to_check_is_refused_at_the_deadline_and_the_session_lives_on() {
    let _turn = crate::integration_support::file_turn();
    let started = Instant::now();
    let error = apply_error(
        "for i in range(400):\n    box(\"b%d\" % i, (100, 100, 100), at=(0, i * 0.01, 0))\n",
    );
    assert!(
        error.contains("checking the program's model") && error.contains("was stopped"),
        "{error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(80),
        "{:?}",
        started.elapsed()
    );
    session(BLOCK).unwrap();
}
