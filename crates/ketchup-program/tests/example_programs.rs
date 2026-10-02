//! Every example program is a golden test: its whole evaluation report (parts,
//! issues, relations, bill of materials) is checked into
//! `examples/programs/<name>.report.json`. A change in the evaluator that moves
//! any example shows up as a reviewed diff of that file instead of a hand-written
//! assertion. Regenerate with `KETCHUP_UPDATE_GOLDEN=1 cargo test -p ketchup-program`.
use ketchup_program::run;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn examples() -> Vec<PathBuf> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/programs");
    let mut programs = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "star")
        })
        .collect::<Vec<_>>();
    programs.sort();
    programs
}

fn report_json(program: &Path) -> String {
    let source = std::fs::read_to_string(program).unwrap();
    let name = program.file_name().unwrap().to_string_lossy();
    let (_, report) =
        run(&name, &source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert!(
        report.ok,
        "{name} must evaluate without errors: {:#?}",
        report.issues
    );
    serde_json::to_string_pretty(&report).unwrap() + "\n"
}

#[test]
fn every_example_program_matches_its_golden_report() {
    let programs = examples();
    assert!(programs.len() >= 3, "the example programs are missing");
    let update = ketchup_test_env::update_golden();
    let mut changed = Vec::new();
    for program in programs {
        let golden = program.with_extension("report.json");
        let actual = report_json(&program);
        if update {
            std::fs::write(&golden, &actual).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&golden)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if expected != actual {
            changed.push(golden.display().to_string());
        }
    }
    assert!(
        changed.is_empty(),
        "example reports changed; review and regenerate with KETCHUP_UPDATE_GOLDEN=1: {changed:#?}"
    );
}
