//! Every example program is a golden test. Its whole evaluation report (parts,
//! issues, relations, bill of materials) is computed here, and only a digest of
//! it is checked into `examples/programs/<name>.digest.json`: short lists stay
//! verbatim, a list longer than [`VERBATIM_ITEMS`] becomes its length, its count
//! per `kind` and the SHA-256 of its JSON. A change in the evaluator that moves
//! any example still shows up as a reviewed diff of that file, without
//! committing megabytes of generated report on every change (review REPO-1).
//! Regenerate with `KETCHUP_UPDATE_GOLDEN=1 cargo test -p ketchup-program`; set
//! `KETCHUP_TEST_REPORT_DIR` as well to get the full reports for comparison.
use ketchup_program::run;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Lists up to this long are kept whole in the digest.
const VERBATIM_ITEMS: usize = 24;
/// A digest larger than this has kept a generated report in the repository.
const DIGEST_MAX_BYTES: usize = 32 * 1024;

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

fn digest(value: &Value) -> Value {
    match value {
        Value::Array(items) if items.len() > VERBATIM_ITEMS => {
            let mut kinds = BTreeMap::<&str, usize>::new();
            for kind in items.iter().filter_map(|item| item.get("kind")?.as_str()) {
                *kinds.entry(kind).or_default() += 1;
            }
            let hash = Sha256::digest(serde_json::to_vec(value).unwrap());
            let mut condensed = Map::new();
            condensed.insert("count".into(), json!(items.len()));
            if !kinds.is_empty() {
                condensed.insert("kinds".into(), json!(kinds));
            }
            condensed.insert(
                "sha256".into(),
                json!(
                    hash.iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                ),
            );
            Value::Object(condensed)
        }
        Value::Array(items) => Value::Array(items.iter().map(digest).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), digest(field)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap() + "\n"
}

#[test]
fn every_example_program_matches_its_golden_digest() {
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
        let actual = pretty(&digest(&report));
        assert!(
            actual.len() <= DIGEST_MAX_BYTES,
            "{stem}: digest of {} bytes keeps too much of the generated report",
            actual.len()
        );
        let golden = program.with_extension("digest.json");
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
fn a_long_list_is_digested_by_length_kind_and_hash() {
    let items = (0..=VERBATIM_ITEMS)
        .map(|index| json!({ "kind": if index % 2 == 0 { "contact" } else { "gap" } }))
        .collect::<Vec<_>>();
    let short = json!({ "relations": items[..VERBATIM_ITEMS] });
    assert_eq!(digest(&short), short);
    let long = digest(&json!({ "relations": items }));
    assert_eq!(long["relations"]["count"], json!(VERBATIM_ITEMS + 1));
    assert_eq!(
        long["relations"]["kinds"],
        json!({ "contact": 13, "gap": 12 })
    );
    assert_eq!(long["relations"]["sha256"].as_str().unwrap().len(), 64);
    let mut moved = items.clone();
    moved[0] = json!({ "kind": "contact", "area_mm2": 1.0 });
    assert_ne!(
        digest(&json!({ "relations": moved }))["relations"]["sha256"],
        long["relations"]["sha256"],
        "a change inside a digested list changes its hash"
    );
}
