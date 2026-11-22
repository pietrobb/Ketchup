//! A program applied to a document runs in the program worker: a program that
//! takes all the memory is refused with `memory_limit` and the session that
//! applied it (the window) keeps working (review PRG-M1). The checks of its
//! model run there too, under the same limits (review PRG-1).
use crate::operations_support::{BLOCK, session};
use ketchup_application::{DocumentSession, RuleProgramApplyError, SessionSettings};
use ketchup_model::document::RuleProgramSource;
use std::collections::BTreeMap;

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

/// A block of cubes 18 × 18 × 18, each touching its 26 neighbours, evaluates
/// in about 2.5 s but takes about 9 s more to check in a debug build (a release
/// build: 27 × 27 × 27, 6 s). Cubes that all overlap stop at a pair limit, and
/// a flat grid now checks only neighbours (review 2026-11-22 R1122ac-PG-1102,
/// capability review 1026). The caller waits no longer than
/// its deadline for a report. The report is no gate (review 2026-10-16 N-12,
/// AGENTS.md §3): the model is applied and its report says that its checks did
/// not finish.
#[test]
fn a_model_too_large_to_check_is_applied_with_an_unfinished_report_and_the_session_lives_on() {
    let _turn = crate::integration_support::file_turn();
    let (side, seconds) = if cfg!(debug_assertions) {
        (18, 5)
    } else {
        (27, 3)
    };
    let mut window = DocumentSession::new(SessionSettings::default());
    let applied = window
        .apply_rule_program_by(
            source(&format!(
                "for i in range({side}):\n    for j in range({side}):\n        for k in range({side}):\n            \
                 box(\"b%d_%d_%d\" % (i, j, k), (100, 100, 100), at=(i * 100, j * 100, k * 100))\n"
            )),
            false,
            ketchup_application::program_isolation::CheckDeadline::after(
                std::time::Duration::from_secs(seconds),
            ),
        )
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(applied.snapshot.occurrences().count(), side * side * side);
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
    session(BLOCK).unwrap();
}
