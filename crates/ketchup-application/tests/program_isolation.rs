//! A program applied to a document runs in the program worker: a program that
//! takes all the memory is refused with `memory_limit` and the session that
//! applied it (the window) keeps working (review PRG-M1).
use crate::operations_support::{BLOCK, apply_error, session};

#[test]
fn a_program_past_the_memory_limit_is_refused_and_the_session_lives_on() {
    let _turn = crate::integration_support::file_turn();
    let error = apply_error("s = \"ab\" * 50000\nt = s.replace(\"\", s)\n");
    assert!(error.contains("GiB of memory and was stopped"), "{error}");
    session(BLOCK).unwrap();
}
