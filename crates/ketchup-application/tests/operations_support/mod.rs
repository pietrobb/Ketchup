//! Builds a rule program in a fresh document and evaluates a part's exact
//! solid, as the window does.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::f64::consts::PI;

use ketchup_application::evaluation::exact_worker_candidates;
use ketchup_application::{DocumentSession, SessionSettings};
use ketchup_model::document::RuleProgramSource;
use ketchup_model::exact_brep_graph::ExactBRepGraph;
use ketchup_model::exact_product::ExactBRepGraphPackage;
use ketchup_scheduler::ExactWorkerSupervisor;

pub const A: f64 = 100.0;
pub const B: f64 = 60.0;
pub const C: f64 = 40.0;
pub const BLOCK: &str = "block = box(\"block\", [100, 60, 40])\n";

pub fn worker() -> ExactWorkerSupervisor {
    let executable = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    ExactWorkerSupervisor::spawn(executable).unwrap()
}

/// Applies `program` to a new document.
pub fn session(program: &str) -> Result<DocumentSession, String> {
    let mut session = DocumentSession::new(SessionSettings::default());
    session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "operations.star".to_owned(),
                source: program.to_owned(),
                overrides: BTreeMap::new(),
            },
            false,
        )
        .map_err(|error| format!("apply: {error}"))?;
    Ok(session)
}

/// Why `program` cannot be applied.
pub fn apply_error(program: &str) -> String {
    match session(program) {
        Ok(_) => panic!("the program must be refused:\n{program}"),
        Err(error) => error,
    }
}

/// Names of the parts `program` builds.
pub fn part_names(program: &str) -> Vec<String> {
    let session = session(program).unwrap_or_else(|error| panic!("{error}"));
    let snapshot = session.snapshot();
    let mut names: Vec<_> = snapshot
        .occurrences()
        .map(|occurrence| occurrence.name().to_owned())
        .collect();
    names.sort();
    names
}

/// Builds `program` in a new document and evaluates the exact solid of `part`.
pub fn solid(
    worker: &mut ExactWorkerSupervisor,
    program: &str,
    part: &str,
) -> Result<ExactBRepGraphPackage, String> {
    let session = session(program)?;
    let snapshot = session.snapshot();
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == part)
        .unwrap_or_else(|| panic!("no part {part:?}"));
    let definition = snapshot.definition(occurrence.definition_id()).unwrap();
    let graph = ExactBRepGraph::from_snapshot(
        &snapshot,
        occurrence.definition_id(),
        *definition.feature_ids().last().unwrap(),
    )
    .map_err(|error| format!("graph: {error:?}"))?;
    worker
        .evaluate_exact_brep_graph(&graph)
        .map_err(|error| error.to_string())
}

/// Volume of one closed solid with a single shell.
pub fn volume(worker: &mut ExactWorkerSupervisor, program: &str, part: &str) -> f64 {
    let package = solid(worker, program, part).unwrap_or_else(|error| panic!("{error}\n{program}"));
    let [_, _, _, shells, solids] = package.topology_counts;
    assert_eq!((shells, solids), (1, 1), "{program}");
    package.volume_mm3
}

pub fn face_count(worker: &mut ExactWorkerSupervisor, program: &str, part: &str) -> u32 {
    solid(worker, program, part).unwrap().topology_counts[2]
}

pub fn assert_volume(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-6 * expected.max(1.0),
        "volume {actual} != {expected}"
    );
}

/// Material a fillet of radius `r` removes along a straight edge of length `l`.
pub fn fillet_loss(r: f64, l: f64) -> f64 {
    (1.0 - PI / 4.0) * r * r * l
}
