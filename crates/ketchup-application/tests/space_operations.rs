//! `upsert_space`, `delete_space` and the same for clearance volumes: what an
//! AI declares through a CAD program commits as one Undo step and survives
//! save and reopen; what the document does not allow is refused before
//! anything changes.

use ketchup_application::{DocumentSession, SaveOptions, SessionError, SessionSettings};
use ketchup_assistant::catalog::cad_operation_catalog;
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation as Operation, AssistantCadEditProgram, AssistantClearanceOwner,
    AssistantClearanceSeverity, AssistantInstancePath,
};
use ketchup_model::document::InstancePath;
use ketchup_model::space::{ClearanceOwner, ClearanceSeverity, ClearanceVolumeId, SpaceId};
use std::collections::BTreeSet;

fn space(
    id: u64,
    purpose: &str,
    min_mm: [f64; 3],
    max_mm: [f64; 3],
    adjacent: &[u64],
) -> Operation {
    Operation::UpsertSpace {
        space_id: id,
        purpose: purpose.to_owned(),
        min_mm,
        max_mm,
        adjacent_to: adjacent.to_vec(),
        accessible_to: Vec::new(),
    }
}

fn kitchen(adjacent: &[u64]) -> Operation {
    space(70, "Kitchen", [0.0; 3], [3000.0, 2000.0, 2500.0], adjacent)
}

fn pantry(adjacent: &[u64]) -> Operation {
    space(
        75,
        "Pantry",
        [3000.0, 0.0, 0.0],
        [4000.0, 1000.0, 2500.0],
        adjacent,
    )
}

fn program(operations: Vec<Operation>) -> AssistantCadEditProgram {
    AssistantCadEditProgram { operations }
}

fn commit(session: &mut DocumentSession, operations: Vec<Operation>) {
    session
        .apply_cad_program(&program(operations), &BTreeSet::new())
        .unwrap();
}

#[test]
fn the_catalog_offers_space_operations() {
    let upsert = cad_operation_catalog(Some("upsert_space")).unwrap();
    let fields = upsert["fields"].as_str().unwrap();
    assert!(fields.contains("min_mm: [number; 3]"), "{fields}");
    assert!(fields.contains("adjacent_to?: [integer]"), "{fields}");
    assert!(upsert["description"].as_str().unwrap().contains("mutual"));
    assert!(cad_operation_catalog(Some("delete_space")).is_ok());
}

#[test]
fn spaces_commit_as_one_undo_step_and_survive_save_and_reopen() {
    let mut session = DocumentSession::default();
    let before = session.snapshot().canonical_digest();
    let proposal = session
        .plan_cad_program(
            &program(vec![kitchen(&[75]), pantry(&[70])]),
            &BTreeSet::new(),
        )
        .unwrap();
    assert_eq!(session.snapshot().canonical_digest(), before);
    session.apply_proposal(&proposal).unwrap();
    assert_eq!(session.visible_undo_steps(), 1);

    let snapshot = session.snapshot();
    let kitchen_space = snapshot.space(SpaceId(70)).unwrap();
    assert_eq!(kitchen_space.purpose(), "Kitchen");
    assert_eq!(kitchen_space.volume().max(), [3000.0, 2000.0, 2500.0]);
    assert_eq!(kitchen_space.adjacent_to(), [SpaceId(75)]);

    // Replacing a space keeps one entry; the pantry stays mutually adjacent.
    commit(
        &mut session,
        vec![space(
            70,
            "Kitchen and dining",
            [0.0; 3],
            [3500.0, 2000.0, 2500.0],
            &[75],
        )],
    );
    let saved = session.snapshot();
    assert_eq!(saved.spaces().count(), 2);
    assert_eq!(
        saved.space(SpaceId(70)).unwrap().purpose(),
        "Kitchen and dining"
    );

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("spaces.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    let reopened = reopened.snapshot();
    assert_eq!(
        reopened.spaces().cloned().collect::<Vec<_>>(),
        saved.spaces().cloned().collect::<Vec<_>>()
    );

    // Deleting needs the adjacency gone first; both changes go in one program.
    commit(
        &mut session,
        vec![
            space(
                70,
                "Kitchen and dining",
                [0.0; 3],
                [3500.0, 2000.0, 2500.0],
                &[],
            ),
            Operation::DeleteSpace { space_id: 75 },
        ],
    );
    assert!(session.snapshot().space(SpaceId(75)).is_none());
    session.undo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        saved.canonical_digest()
    );
}

fn clearance(id: u64, owner: AssistantClearanceOwner, max_z: f64) -> Operation {
    Operation::UpsertClearanceVolume {
        clearance_volume_id: id,
        reason: "Door swing".to_owned(),
        owner,
        min_mm: [0.0; 3],
        max_mm: [900.0, 900.0, max_z],
        severity: AssistantClearanceSeverity::Required,
        tolerance_mm: Some(0.5),
    }
}

fn of_space(space_id: u64) -> AssistantClearanceOwner {
    AssistantClearanceOwner::Space { space_id }
}

fn of_occurrence(root_occurrence_id: u64) -> AssistantClearanceOwner {
    AssistantClearanceOwner::Occurrence {
        instance_path: AssistantInstancePath {
            root_occurrence_id,
            steps: Vec::new(),
        },
    }
}

#[test]
fn clearance_volumes_of_a_space_or_a_part_survive_save_reopen_and_undo() {
    let operations = cad_operation_catalog(Some("upsert_clearance_volume")).unwrap();
    assert!(operations["fields"].as_str().unwrap().contains("owner"));
    assert!(cad_operation_catalog(Some("delete_clearance_volume")).is_ok());

    let mut session = DocumentSession::default();
    commit(&mut session, vec![super::document_session::part()]);
    let part = session.snapshot().occurrences().next().unwrap().id();
    let empty = session.snapshot().canonical_digest();
    // The owning space is declared in the same program.
    commit(
        &mut session,
        vec![
            kitchen(&[]),
            clearance(90, of_space(70), 2000.0),
            clearance(91, of_occurrence(part.0), 1000.0),
        ],
    );
    let saved = session.snapshot();
    let door = saved.clearance_volume(ClearanceVolumeId(90)).unwrap();
    assert_eq!(door.owner(), &ClearanceOwner::Space(SpaceId(70)));
    assert_eq!(door.severity(), ClearanceSeverity::Required);
    assert_eq!(door.tolerance().linear_mm(), 0.5);
    assert_eq!(door.volume().max(), [900.0, 900.0, 2000.0]);
    assert_eq!(
        saved
            .clearance_volume(ClearanceVolumeId(91))
            .unwrap()
            .owner(),
        &ClearanceOwner::Occurrence(InstancePath::root(part))
    );

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("clearances.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(
        reopened
            .snapshot()
            .clearance_volumes()
            .cloned()
            .collect::<Vec<_>>(),
        saved.clearance_volumes().cloned().collect::<Vec<_>>()
    );

    commit(
        &mut session,
        vec![Operation::DeleteClearanceVolume {
            clearance_volume_id: 90,
        }],
    );
    assert!(
        session
            .snapshot()
            .clearance_volume(ClearanceVolumeId(90))
            .is_none()
    );
    session.undo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        saved.canonical_digest()
    );
    session.undo().unwrap();
    assert_eq!(session.snapshot().canonical_digest(), empty);
}

#[test]
fn clearance_volumes_the_document_does_not_allow_are_refused_without_a_change() {
    let mut session = DocumentSession::default();
    commit(&mut session, vec![kitchen(&[])]);
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
    // No volume; owners that do not exist.
    refused(clearance(90, of_space(70), 0.0));
    refused(clearance(90, of_space(99), 2000.0));
    refused(clearance(90, of_occurrence(42), 2000.0));
    // A negative or non-finite tolerance, an empty reason.
    for tolerance_mm in [-1.0, f64::NAN] {
        let mut operation = clearance(90, of_space(70), 2000.0);
        if let Operation::UpsertClearanceVolume {
            tolerance_mm: tolerance,
            ..
        } = &mut operation
        {
            *tolerance = Some(tolerance_mm);
        }
        refused(operation);
    }
    let mut unnamed = clearance(90, of_space(70), 2000.0);
    if let Operation::UpsertClearanceVolume { reason, .. } = &mut unnamed {
        reason.clear();
    }
    refused(unnamed);
    // One that does not exist.
    refused(Operation::DeleteClearanceVolume {
        clearance_volume_id: 99,
    });
}

#[test]
fn spaces_the_document_does_not_allow_are_refused_without_a_change() {
    let mut session = DocumentSession::default();
    commit(&mut session, vec![kitchen(&[75]), pantry(&[70])]);
    let digest = session.snapshot().canonical_digest();
    let undo = session.visible_undo_steps();
    let refused = |operations: Vec<Operation>| {
        let error = session
            .plan_cad_program(&program(operations.clone()), &BTreeSet::new())
            .expect_err(&format!("{operations:?} must be refused"));
        assert!(
            matches!(error, SessionError::Planning(_) | SessionError::Prepare(_)),
            "{operations:?}: {error:?}"
        );
        assert_eq!(session.snapshot().canonical_digest(), digest);
        assert_eq!(session.visible_undo_steps(), undo);
    };
    // No volume, an inverted box, a non-finite corner.
    refused(vec![space(80, "Flat", [0.0; 3], [100.0, 100.0, 0.0], &[])]);
    refused(vec![space(80, "Inverted", [100.0; 3], [0.0; 3], &[])]);
    refused(vec![space(
        80,
        "Broken",
        [f64::NAN, 0.0, 0.0],
        [1.0; 3],
        &[],
    )]);
    // Reserved id, empty purpose, adjacent to itself.
    refused(vec![space(0, "Zero", [0.0; 3], [1.0; 3], &[])]);
    refused(vec![space(80, "  ", [0.0; 3], [1.0; 3], &[])]);
    refused(vec![space(80, "Self", [0.0; 3], [1.0; 3], &[80])]);
    // One-sided adjacency and a missing neighbour.
    refused(vec![space(80, "Hall", [0.0; 3], [1.0; 3], &[70])]);
    refused(vec![space(80, "Hall", [0.0; 3], [1.0; 3], &[99])]);
    // A space another space still lists, and one that does not exist.
    refused(vec![Operation::DeleteSpace { space_id: 75 }]);
    refused(vec![Operation::DeleteSpace { space_id: 99 }]);
}
