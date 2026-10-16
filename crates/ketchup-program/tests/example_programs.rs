//! Every example program is a golden test. Its whole evaluation report (parts,
//! issues, relations, bill of materials) is computed here and compared with
//! `examples/programs/<name>.expected.json`: short lists stay verbatim, a list
//! longer than [`VERBATIM_ITEMS`] keeps its length and its count per `kind`. A
//! change in the evaluator that moves an example shows up as a readable diff of
//! that file, without committing megabytes of generated report (review REPO-1).
//! Regenerate with `KETCHUP_UPDATE_GOLDEN=1 cargo test -p ketchup-program`; set
//! `KETCHUP_TEST_REPORT_DIR` as well to get the full reports for comparison.
use ketchup_program::run;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Lists up to this long are kept whole in the expected report.
const VERBATIM_ITEMS: usize = 24;
/// An expected report larger than this has kept a generated report in the repository.
const EXPECTED_MAX_BYTES: usize = 32 * 1024;

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

fn report(program: &Path) -> Value {
    let source = std::fs::read_to_string(program).unwrap();
    let name = program.file_name().unwrap().to_string_lossy();
    let (_, report) =
        run(&name, &source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert!(
        report.ok,
        "{name} must evaluate without errors: {:#?}",
        report.issues
    );
    serde_json::to_value(&report).unwrap()
}

fn condense(value: &Value) -> Value {
    match value {
        Value::Array(items) if items.len() > VERBATIM_ITEMS => {
            let mut kinds = BTreeMap::<&str, usize>::new();
            for kind in items.iter().filter_map(|item| item.get("kind")?.as_str()) {
                *kinds.entry(kind).or_default() += 1;
            }
            let mut condensed = Map::new();
            condensed.insert("count".into(), json!(items.len()));
            if !kinds.is_empty() {
                condensed.insert("kinds".into(), json!(kinds));
            }
            Value::Object(condensed)
        }
        Value::Array(items) => Value::Array(items.iter().map(condense).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), condense(field)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap() + "\n"
}

#[test]
fn every_example_program_matches_its_expected_report() {
    let programs = examples();
    assert!(programs.len() >= 3, "the example programs are missing");
    let update = ketchup_test_env::update_golden();
    let mut changed = Vec::new();
    for program in programs {
        let report = report(&program);
        let stem = program.file_stem().unwrap().to_string_lossy().into_owned();
        if let Some(path) = ketchup_test_env::report_path(&format!("{stem}.report.json")) {
            std::fs::write(path, pretty(&report)).unwrap();
        }
        let actual = pretty(&condense(&report));
        assert!(
            actual.len() <= EXPECTED_MAX_BYTES,
            "{stem}: expected report of {} bytes keeps too much of the generated report",
            actual.len()
        );
        let golden = program.with_extension("expected.json");
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

#[test]
fn a_long_list_keeps_its_count_and_kinds() {
    let items = (0..=VERBATIM_ITEMS)
        .map(|index| json!({ "kind": if index % 2 == 0 { "contact" } else { "gap" } }))
        .collect::<Vec<_>>();
    let short = json!({ "relations": items[..VERBATIM_ITEMS] });
    assert_eq!(condense(&short), short);
    assert_eq!(
        condense(&json!({ "relations": items })),
        json!({ "relations": { "count": VERBATIM_ITEMS + 1, "kinds": { "contact": 13, "gap": 12 } } })
    );
    let plain = (0..=VERBATIM_ITEMS).collect::<Vec<_>>();
    assert_eq!(
        condense(&json!({ "values": plain })),
        json!({ "values": { "count": VERBATIM_ITEMS + 1 } })
    );
}
