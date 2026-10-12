//! `upsert_joint` and `delete_joint`: a joint an AI declares between two
//! placed parts, ordinary or built by a Starlark program, commits as one Undo
//! step, survives save and reopen as MCP `inspect query kind=joints` reports
//! it, disappears when deleted and comes back with Undo; what does not resolve
//! is refused before anything changes.

use ketchup_application::model_query::{EntityKind, ModelQuery};
use ketchup_application::{DocumentSession, SaveOptions, SessionError, SessionSettings};
use ketchup_assistant::catalog::cad_operation_catalog;
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation as Operation, AssistantCadEditProgram, AssistantInstancePath,
};
use ketchup_model::document::{OccurrenceId, RuleProgramSource, Snapshot};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// A stile and a rail whose end sits 8 mm deep in it.
const SOURCE: &str = "box('stile',(18,100,400))\nbox('rail',(300,100,18),at=(10,0,200))\n";

fn program_session() -> DocumentSession {
    let mut session = DocumentSession::default();
    session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "frame.star".into(),
                source: SOURCE.into(),
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
    session
}

fn program(operations: Vec<Operation>) -> AssistantCadEditProgram {
    AssistantCadEditProgram { operations }
}

fn commit(session: &mut DocumentSession, operations: Vec<Operation>) {
    session
        .apply_cad_program(&program(operations), &BTreeSet::new())
        .unwrap();
}

fn part(snapshot: &Snapshot, name: &str) -> OccurrenceId {
    snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap_or_else(|| panic!("no part {name}"))
        .id()
}

fn path(id: OccurrenceId) -> AssistantInstancePath {
    AssistantInstancePath {
        root_occurrence_id: id.0,
        steps: Vec::new(),
    }
}

fn joint(id: u64, a: u64, b: u64, min_mm: [f64; 3], max_mm: [f64; 3]) -> Operation {
    Operation::UpsertJoint {
        joint_id: id,
        part_a: path(OccurrenceId(a)),
        part_b: path(OccurrenceId(b)),
        min_mm,
        max_mm,
    }
}

/// The joint as MCP `inspect query kind=joints` reports it.
fn reported(snapshot: &Snapshot, id: u64) -> Option<Value> {
    ModelQuery::default()
        .detail(snapshot, EntityKind::Joints, id)
        .ok()
        .map(|detail| detail["item"].clone())
}

#[test]
fn the_catalog_offers_joint_operations() {
    let upsert = cad_operation_catalog(Some("upsert_joint")).unwrap();
    let fields = upsert["fields"].as_str().unwrap();
    assert!(fields.contains("part_a"), "{fields}");
    assert!(fields.contains("min_mm"), "{fields}");
    assert!(cad_operation_catalog(Some("delete_joint")).is_ok());
}

#[test]
fn joints_of_program_and_ordinary_parts_survive_reopen_and_undo_restores_a_deleted_one() {
    let mut session = program_session();
    commit(&mut session, vec![super::document_session::part()]);
    let snapshot = session.snapshot();
    let (stile, rail, ordinary) = (
        part(&snapshot, "stile").0,
        part(&snapshot, "rail").0,
        part(&snapshot, "Editable part").0,
    );
    let before = snapshot.canonical_digest();
    let undo = session.visible_undo_steps();

    commit(
        &mut session,
        vec![
            joint(1, stile, rail, [10., 0., 200.], [18., 100., 218.]),
            joint(2, ordinary, stile, [0., 0., 0.], [17., 18., 37.]),
        ],
    );
    assert_eq!(session.visible_undo_steps(), undo + 1);
    let created = session.snapshot();
    let tenon = reported(&created, 1).expect("the program-part joint is reported");
    assert_eq!(tenon["participant_a"]["kind"], "instance", "{tenon:#}");
    assert_eq!(tenon["participant_a"]["root_occurrence_id"], stile);
    assert_eq!(tenon["participant_a"]["resolved"], true);
    assert_eq!(tenon["participant_b"]["root_occurrence_id"], rail);
    assert_eq!(
        tenon["volume"],
        json!({"min_mm":[10.0,0.0,200.0],"max_mm":[18.0,100.0,218.0],"coordinate_space":"world_mm"})
    );
    let mixed = reported(&created, 2).expect("the ordinary-part joint is reported");
    assert_eq!(mixed["participant_a"]["root_occurrence_id"], ordinary);
    assert_eq!(mixed["participant_b"]["root_occurrence_id"], stile);

    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("joints.ketchup");
    session.save(&file, SaveOptions::default()).unwrap();
    let reopened = DocumentSession::open(&file, SessionSettings::default()).unwrap();
    assert_eq!(reported(&reopened.snapshot(), 1), Some(tenon.clone()));
    assert_eq!(reported(&reopened.snapshot(), 2), Some(mixed));

    commit(&mut session, vec![Operation::DeleteJoint { joint_id: 1 }]);
    assert!(reported(&session.snapshot(), 1).is_none());
    session.undo().unwrap();
    assert_eq!(reported(&session.snapshot(), 1), Some(tenon));
    assert_eq!(
        session.snapshot().canonical_digest(),
        created.canonical_digest()
    );
    session.undo().unwrap();
    assert_eq!(session.snapshot().canonical_digest(), before);
    assert_eq!(session.snapshot().joints().count(), 0);
}

#[test]
fn joints_that_do_not_resolve_are_refused_without_a_change() {
    let mut session = program_session();
    let snapshot = session.snapshot();
    let (stile, rail) = (part(&snapshot, "stile").0, part(&snapshot, "rail").0);
    commit(
        &mut session,
        vec![joint(1, stile, rail, [10., 0., 200.], [18., 100., 218.])],
    );
    let digest = session.snapshot().canonical_digest();
    let undo = session.visible_undo_steps();
    let refused = |operation: Operation| {
        let error = session
            .plan_cad_program(&program(vec![operation.clone()]), &BTreeSet::new())
            .expect_err(&format!("{operation:?} must be refused"));
        assert!(
            matches!(error, SessionError::Planning(_) | SessionError::Prepare(_)),
            "{operation:?}: {error:?}"
        );
        assert_eq!(session.snapshot().canonical_digest(), digest);
        assert_eq!(session.visible_undo_steps(), undo);
    };
    let volume = ([10., 0., 200.], [18., 100., 218.]);
    // A part that is not placed, a part joined to itself, the reserved id.
    refused(joint(2, stile, 999, volume.0, volume.1));
    refused(joint(2, stile, stile, volume.0, volume.1));
    refused(joint(0, stile, rail, volume.0, volume.1));
    // No volume, an inverted one, a coordinate that is not a number.
    refused(joint(2, stile, rail, volume.0, volume.0));
    refused(joint(2, stile, rail, volume.1, volume.0));
    refused(joint(2, stile, rail, [f64::NAN, 0., 200.], volume.1));
    // A joint that does not exist.
    refused(Operation::DeleteJoint { joint_id: 9 });
}
