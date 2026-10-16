//! A program applied to a document runs in the program worker: a program that
//! takes all the memory is refused with `memory_limit` and the session that
//! applied it (the window) keeps working (review PRG-M1). The checks of its
//! model run there too, under the same limits (review PRG-1).
use crate::operations_support::{BLOCK, session};
use ketchup_application::{DocumentSession, RuleProgramApplyError, SessionSettings};
use ketchup_model::document::RuleProgramSource;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// The interpreter's heap bound or the worker's process limit may stop it first;
/// either way the code is `memory_limit` and the same session applies the next program.
#[test]
fn a_program_past_the_memory_limit_is_refused_and_the_session_lives_on() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let program = source("s = \"ab\" * 50000\nt = s.replace(\"\", s)\n");
    let Err(error) = session.apply_rule_program(program, false) else {
        panic!("the program must be refused");
    };
    let RuleProgramApplyError::Program { code, .. } = &error else {
        panic!("{error}");
    };
    assert_eq!(code, "memory_limit", "{error}");
    if let Err(error) = session.apply_rule_program(source(BLOCK), false) {
        panic!("{error}");
    }
}

/// Review 2026-10-15 N-7: a boolean keeps a copy of its tool with the tool's
/// own booleans, so a chain of parts each cut by the one before nests one level
/// per part. The model at the depth bound comes back from the worker; one
/// level deeper is refused by name, not as a worker frame that does not decode.
#[test]
fn a_boolean_chain_past_the_depth_bound_is_refused_by_name() {
    let _turn = crate::integration_support::file_turn();
    let chain = |depth: usize| {
        source(&format!(
            "box(\"p0\", (10, 10, 10))\nfor i in range(1, {}):\n    \
             box(\"p%d\" % i, (10, 10, 10), at = (20 * i, 0, 0))\n    \
             subtract(\"p%d\" % i, \"p%d\" % (i - 1))\n",
            depth + 1
        ))
    };
    let bound = ketchup_program::eval::MAX_BOOLEAN_DEPTH;
    let mut session = DocumentSession::new(SessionSettings::default());
    if let Err(error) = session.apply_rule_program(chain(bound), false) {
        panic!("{error}");
    }
    let Err(error) = session.apply_rule_program(chain(bound + 1), false) else {
        panic!("the chain past the bound must be refused");
    };
    let message = error.to_string();
    assert!(
        message.contains("nest") && message.contains(&bound.to_string()),
        "{message}"
    );
}

fn source(program: &str) -> RuleProgramSource {
    RuleProgramSource {
        variants: Default::default(),
        file_name: "operations.star".to_owned(),
        source: program.to_owned(),
        overrides: BTreeMap::new(),
    }
}

/// 400 boxes that all overlap evaluate in a fraction of a second, but checking
/// their 79 800 pairs takes over two minutes in a debug build: the caller waits
/// no longer than the worker's 60 s deadline for a report. The report is no
/// gate (review 2026-10-16 N-12, AGENTS.md §3): the model is applied and its
/// report says that its checks did not finish.
#[test]
fn a_model_too_large_to_check_is_applied_with_an_unfinished_report_and_the_session_lives_on() {
    let _turn = crate::integration_support::file_turn();
    let started = Instant::now();
    let mut window = DocumentSession::new(SessionSettings::default());
    let applied = window
        .apply_rule_program(
            source("for i in range(400):\n    box(\"b%d\" % i, (100, 100, 100), at=(0, i * 0.01, 0))\n"),
            false,
        )
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(applied.snapshot.occurrences().count(), 400);
    let unfinished = applied
        .report
        .issues
        .iter()
        .filter(|issue| issue.kind == ketchup_program::CHECKS_UNVERIFIED)
        .map(|issue| issue.message.as_str())
        .collect::<Vec<_>>();
    assert!(
        matches!(unfinished.as_slice(), [message] if message.contains("checking the program's model") && message.contains("was stopped")),
        "{:?}",
        applied.report.issues
    );
    assert_eq!(applied.report.issues.len(), 1);
    assert!(applied.report.relations.is_empty());
    assert_eq!(applied.report.bom, ketchup_program::bom(&applied.model));
    assert!(
        started.elapsed() < Duration::from_secs(80),
        "{:?}",
        started.elapsed()
    );
    session(BLOCK).unwrap();
}
