//! Bought parts a program reads from STEP files stay the program's: applying
//! it places each file's exact body where the parameters put it, reads each
//! file once and keeps the document owned by the program.
use ketchup_application::program_imports::{ImportFiles, NoFiles};
use ketchup_application::program_isolation::CheckDeadline;
use ketchup_application::{DocumentSession, RuleProgramApplyError};
use ketchup_model::document::{FeatureKind, RuleProgramSource, Snapshot, Transform};
use ketchup_model::import::{ImportLengthUnit, StepImportEvidence};
use ketchup_program::imported::ImportedBody;
use std::collections::BTreeMap;

const WARDROBE: &str = "door_h = param('door_h', 2000)\n\
    for (i, z) in enumerate([100, door_h - 100]):\n\
    \x20   purchased('hinge %d' % i, 'hettich/9237903.stp', '9237903', 'Hettich',\n\
    \x20             at = (18, 0, z), z = (-1, 0, 0), x = (0, 0, 1))\n\
    board('door', (600, 18, door_h))\n";

fn source(door_h: f64) -> RuleProgramSource {
    RuleProgramSource {
        variants: Default::default(),
        file_name: "wardrobe.star".into(),
        source: WARDROBE.into(),
        overrides: BTreeMap::from([("door_h".into(), door_h)]),
    }
}

/// A folder holding one hinge file, counting how often it is read.
#[derive(Default)]
struct Folder {
    reads: BTreeMap<String, usize>,
    broken: bool,
}

fn hinge_evidence() -> StepImportEvidence {
    StepImportEvidence {
        source_sha256: [7; 32],
        source_byte_len: 4_096,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "0123456789abcdef".to_owned(),
        body_kind: ketchup_model::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 2_200.0,
        volume_mm3: 6_000.0,
        bounds_mm: [[-10.0, -25.0, 0.0], [10.0, 25.0, 12.0]],
        backend: "occt-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    }
}

impl ImportFiles for Folder {
    fn read(
        &mut self,
        file: &str,
        _part: Option<&str>,
    ) -> Result<Option<ImportedBody>, ketchup_rejection::Rejection> {
        *self.reads.entry(file.to_owned()).or_default() += 1;
        if self.broken {
            return Err(ketchup_application::program_imports::unreadable(
                "the exact kernel found no solid in it",
            ));
        }
        assert_eq!(file, "hettich/9237903.stp");
        Ok(Some(ImportedBody {
            file_name: "9237903.stp".to_owned(),
            part_index: None,
            evidence: hinge_evidence(),
            placement: None,
        }))
    }
}

fn apply(
    session: &mut DocumentSession,
    door_h: f64,
    files: &mut dyn ImportFiles,
) -> Result<Snapshot, RuleProgramApplyError> {
    session
        .apply_rule_program_reading(source(door_h), false, files, CheckDeadline::start())
        .map(|applied| applied.snapshot)
}

fn hinge(snapshot: &Snapshot, name: &str) -> (ketchup_model::document::OccurrenceId, Transform) {
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap_or_else(|| panic!("{name} not placed"));
    let definition = snapshot.definition(occurrence.definition_id()).unwrap();
    let [feature] = definition.feature_ids() else {
        panic!("{name}: {:?}", definition.feature_ids());
    };
    let FeatureKind::ImportedExactBody(spec) = snapshot.feature(*feature).unwrap().kind() else {
        panic!("{name} is not the file's body");
    };
    assert_eq!(spec.source_sha256, [7; 32]);
    (occurrence.id(), occurrence.transform())
}

#[test]
fn the_program_places_each_bought_part_and_keeps_the_document() {
    let mut session = DocumentSession::default();
    let mut folder = Folder::default();
    let snapshot = apply(&mut session, 2000.0, &mut folder).unwrap();
    // Two hinges of one file: read once, one receipt.
    assert_eq!(folder.reads["hettich/9237903.stp"], 1);
    assert_eq!(snapshot.import_receipts().count(), 1);
    let receipt = snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.source_name(), "9237903.stp");
    let (top, at_2000) = hinge(&snapshot, "hinge 1");
    hinge(&snapshot, "hinge 0");
    assert_eq!(at_2000.matrix()[11], 1900.0);
    assert!(session.rule_program().is_some());

    // A taller door moves the upper hinge; the occurrence stays and so does
    // the program, the file read from the folder again only once.
    let snapshot = apply(&mut session, 2400.0, &mut folder).unwrap();
    assert_eq!(folder.reads["hettich/9237903.stp"], 2);
    let (moved, at_2400) = hinge(&snapshot, "hinge 1");
    assert_eq!(moved, top);
    assert_eq!(at_2400.matrix()[11], 2300.0);
    assert_eq!(snapshot.import_receipts().count(), 1);
    assert!(session.rule_program().is_some());

    // A host without the folder (a parameter set over the headless protocol)
    // takes the body the document already holds.
    let snapshot = apply(&mut session, 2200.0, &mut NoFiles).unwrap();
    assert_eq!(hinge(&snapshot, "hinge 1").1.matrix()[11], 2100.0);
    assert!(session.rule_program().is_some());
}

#[test]
fn a_file_that_cannot_be_read_says_which_part_and_why() {
    let mut session = DocumentSession::default();
    let error = apply(
        &mut session,
        2000.0,
        &mut Folder {
            broken: true,
            ..Folder::default()
        },
    )
    .err()
    .expect("a broken file is refused");
    assert_eq!(error.code(), "program_import_failed");
    let message = error.to_string();
    assert!(
        message.contains("hinge 0")
            && message.contains("hettich/9237903.stp")
            && message.contains("found no solid"),
        "{message}"
    );
    assert!(session.snapshot().definitions().next().is_none());
    // Nothing to read the file from and nothing in the document either.
    let error = apply(&mut session, 2000.0, &mut NoFiles)
        .err()
        .expect("an unread file is refused");
    assert!(
        error
            .to_string()
            .contains("apply the program from its file"),
        "{error}"
    );
}
