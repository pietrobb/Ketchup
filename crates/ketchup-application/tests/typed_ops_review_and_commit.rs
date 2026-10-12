//! Review-and-commit checks for the typed CAD operations that replaced the
//! workflow intents (tag, transform, delete, evaluator input, make unique,
//! profile points):
//! planning changes nothing, one commit is one Undo step, invalid input is
//! refused without a change and a stale proposal keeps the concurrent edit.

use super::document_session::part;
use ketchup_application::{DocumentSession, SessionError};
use ketchup_assistant::sidecar::*;
use ketchup_model::document::*;
use std::collections::BTreeSet;

const TAG: u64 = 14;
const NEW_TAG: u64 = 24;
const COLLECTION: CollectionId = CollectionId(17);
const NODE: u64 = 21;
const NEW_NODE: u64 = 25;

fn program(operations: Vec<AssistantCadEditOperation>) -> AssistantCadEditProgram {
    AssistantCadEditProgram { operations }
}

fn selected(id: OccurrenceId) -> AssistantCadEntitySelector {
    AssistantCadEntitySelector::Occurrences {
        occurrence_ids: vec![id.0],
    }
}

fn commit_commands(session: &mut DocumentSession, commands: Vec<CanonicalCommand>) {
    let proposal = session.plan_commands(CommandBatch::new(commands)).unwrap();
    session.apply_proposal(&proposal).unwrap();
}

/// Two parts, a tag, a collection holding the first part and one evaluator input.
fn seeded() -> (DocumentSession, OccurrenceId, OccurrenceId) {
    let mut session = DocumentSession::default();
    session
        .apply_cad_program(&program(vec![part(), part()]), &BTreeSet::new())
        .unwrap();
    let ids: Vec<OccurrenceId> = session
        .snapshot()
        .occurrences()
        .map(Occurrence::id)
        .collect();
    let [collected, free] = ids[..] else {
        panic!("two seeded occurrences, got {ids:?}")
    };
    commit_commands(
        &mut session,
        vec![
            CanonicalCommand::CreateTag {
                id: TagId(TAG),
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateCollection {
                id: COLLECTION,
                name: "Selection set".to_owned(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![collected],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(NODE),
                name: "width".to_owned(),
                dimension: Dimension::new("600", 600.0).unwrap(),
                dependencies: Vec::new(),
            },
        ],
    );
    (session, collected, free)
}

type Check = Box<dyn Fn(&Snapshot)>;

#[test]
fn replacement_ops_plan_without_change_and_commit_one_undo_step() {
    let (_, collected, free) = seeded();
    let cases: Vec<(AssistantCadEditOperation, Check)> = vec![
        (
            AssistantCadEditOperation::CreateTag {
                tag_id: NEW_TAG,
                name: "Reviewed tag".to_owned(),
                visible: false,
            },
            Box::new(|snapshot| {
                let tag = snapshot.tag(TagId(NEW_TAG)).unwrap();
                assert_eq!(tag.name(), "Reviewed tag");
                assert!(!tag.visible());
            }),
        ),
        (
            AssistantCadEditOperation::SetTagVisibility {
                tag_id: TAG,
                visible: false,
            },
            Box::new(|snapshot| assert!(!snapshot.tag(TagId(TAG)).unwrap().visible())),
        ),
        (
            AssistantCadEditOperation::SetOccurrenceTag {
                selector: selected(free),
                tag_id: Some(TAG),
            },
            Box::new(move |snapshot| {
                assert_eq!(
                    snapshot.occurrence(free).unwrap().tags(),
                    &BTreeSet::from([TagId(TAG)])
                );
            }),
        ),
        (
            AssistantCadEditOperation::Transform {
                selector: selected(free),
                translation_mm: [10.0, -20.0, 30.5],
                rotation: None,
            },
            // part() places each occurrence at [5, 6, 7] mm.
            Box::new(move |snapshot| {
                let matrix = *snapshot.occurrence(free).unwrap().transform().matrix();
                assert_eq!([matrix[3], matrix[7], matrix[11]], [15.0, -14.0, 37.5]);
            }),
        ),
        (
            AssistantCadEditOperation::Delete {
                selector: selected(collected),
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            },
            Box::new(move |snapshot| {
                assert!(snapshot.occurrence(collected).is_none());
                assert!(snapshot.occurrence(free).is_some());
                let collection = snapshot.collection(COLLECTION).unwrap();
                assert_eq!(collection.occurrence_ids().count(), 0);
            }),
        ),
        (
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: NEW_NODE,
                name: "Reviewed depth".to_owned(),
                value: 42.5,
            },
            Box::new(|snapshot| {
                let node = snapshot.evaluator_node(NodeId(NEW_NODE)).unwrap();
                assert_eq!(node.name(), "Reviewed depth");
                assert_eq!(node.dimension().unwrap().millimetres(), 42.5);
            }),
        ),
    ];
    for (operation, check) in cases {
        let (mut session, ..) = seeded();
        let before = session.snapshot();
        let undo_before = session.visible_undo_steps();
        let proposal = session
            .plan_cad_program(&program(vec![operation.clone()]), &BTreeSet::new())
            .unwrap_or_else(|error| panic!("{operation:?}: {error:?}"));
        assert_eq!(session.snapshot().revision_id(), before.revision_id());
        assert_eq!(
            session.snapshot().canonical_digest(),
            before.canonical_digest(),
            "planning {operation:?} changed the document"
        );
        assert_eq!(session.visible_undo_steps(), undo_before);

        let committed = session.apply_proposal(&proposal).unwrap();
        check(&committed);
        assert_eq!(
            session.visible_undo_steps(),
            undo_before + 1,
            "{operation:?}"
        );
        session.undo().unwrap();
        assert_eq!(
            session.snapshot().canonical_digest(),
            before.canonical_digest(),
            "undo of {operation:?}"
        );
    }
}

#[test]
fn invalid_replacement_ops_are_refused_without_a_change() {
    let (session, collected, _) = seeded();
    let missing = OccurrenceId(999);
    let digest_before = session.snapshot().canonical_digest();
    let refused = |operation: AssistantCadEditOperation| {
        let error = session
            .plan_cad_program(&program(vec![operation.clone()]), &BTreeSet::new())
            .expect_err(&format!("{operation:?} must be refused"));
        assert!(
            matches!(error, SessionError::Planning(_) | SessionError::Prepare(_)),
            "{operation:?}: {error:?}"
        );
        assert_eq!(session.snapshot().canonical_digest(), digest_before);
        format!("{error:?}")
    };
    for (tag_id, name) in [(0, "Invalid ID"), (NEW_TAG, ""), (TAG, "Duplicate")] {
        refused(AssistantCadEditOperation::CreateTag {
            tag_id,
            name: name.to_owned(),
            visible: true,
        });
    }
    refused(AssistantCadEditOperation::SetTagVisibility {
        tag_id: 999,
        visible: false,
    });
    refused(AssistantCadEditOperation::SetOccurrenceTag {
        selector: selected(collected),
        tag_id: Some(999),
    });
    refused(AssistantCadEditOperation::SetOccurrenceTag {
        selector: selected(missing),
        tag_id: Some(TAG),
    });
    refused(AssistantCadEditOperation::Transform {
        selector: selected(missing),
        translation_mm: [1.0, 0.0, 0.0],
        rotation: None,
    });
    refused(AssistantCadEditOperation::Delete {
        selector: selected(missing),
        dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
    });
    let referenced = refused(AssistantCadEditOperation::Delete {
        selector: selected(collected),
        dependency_policy: AssistantCadDeletePolicy::RejectIfReferenced,
    });
    assert!(referenced.contains("cad_delete_referenced"), "{referenced}");
    for (node_id, name, value) in [
        (0, "Invalid ID", 1.0),
        (NEW_NODE, "", 1.0),
        (NODE, "Duplicate", 1.0),
        (NEW_NODE, "Not a number", f64::NAN),
    ] {
        refused(AssistantCadEditOperation::CreateEvaluatorInput {
            node_id,
            name: name.to_owned(),
            value,
        });
    }
    refused(AssistantCadEditOperation::MakeOccurrenceUnique {
        occurrence_id: missing.0,
    });
}

#[test]
fn stale_replacement_proposals_are_refused_and_keep_the_concurrent_edit() {
    let (_, collected, free) = seeded();
    let hide_free = CanonicalCommand::SetOccurrenceVisibility {
        id: free,
        visible: false,
    };
    let cases = [
        (
            AssistantCadEditOperation::CreateTag {
                tag_id: NEW_TAG,
                name: "Reviewed tag".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: TagId(NEW_TAG),
                name: "Concurrent tag".to_owned(),
                visible: false,
            },
        ),
        (
            AssistantCadEditOperation::SetTagVisibility {
                tag_id: TAG,
                visible: false,
            },
            CanonicalCommand::SetTagVisibility {
                id: TagId(TAG),
                visible: false,
            },
        ),
        (
            AssistantCadEditOperation::SetOccurrenceTag {
                selector: selected(free),
                tag_id: Some(TAG),
            },
            hide_free.clone(),
        ),
        (
            AssistantCadEditOperation::Transform {
                selector: selected(free),
                translation_mm: [10.0, 0.0, 0.0],
                rotation: None,
            },
            hide_free,
        ),
        (
            AssistantCadEditOperation::Delete {
                selector: selected(collected),
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![collected, free],
            },
        ),
        (
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: NEW_NODE,
                name: "Reviewed depth".to_owned(),
                value: 42.5,
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(NEW_NODE),
                name: "Concurrent depth".to_owned(),
                dimension: Dimension::new("7", 7.0).unwrap(),
                dependencies: Vec::new(),
            },
        ),
    ];
    for (operation, concurrent) in cases {
        let (mut session, ..) = seeded();
        let proposal = session
            .plan_cad_program(&program(vec![operation.clone()]), &BTreeSet::new())
            .unwrap();
        commit_commands(&mut session, vec![concurrent]);
        let changed = session.snapshot().canonical_digest();
        let undo_after_concurrent = session.visible_undo_steps();

        assert!(
            matches!(
                session.apply_proposal(&proposal),
                Err(SessionError::Commit(ProposalCommitError::Stale(_)))
            ),
            "{operation:?} committed over a concurrent edit"
        );
        assert_eq!(session.snapshot().canonical_digest(), changed);
        assert_eq!(session.visible_undo_steps(), undo_after_concurrent);
    }
}

const PROFILE: FeatureId = FeatureId(31);
const EXTRUSION: FeatureId = FeatureId(32);
const TRIANGLE: [[f64; 2]; 3] = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];

/// `set_feature_parameter` with the whole outline, built from wire JSON.
fn set_points(feature: FeatureId, path: &str, points_mm: &[[f64; 2]]) -> AssistantCadEditProgram {
    serde_json::from_value(serde_json::json!({"operations": [{
        "operation": "set_feature_parameter", "feature_id": feature.0,
        "parameter_path": path, "value_type": "length", "value": points_mm,
    }]}))
    .unwrap()
}

fn outline(snapshot: &Snapshot) -> Vec<[f64; 2]> {
    let feature = snapshot.feature(PROFILE).unwrap();
    feature.kind().polygon_points().unwrap()
}

#[test]
fn profile_points_replace_the_outline_in_one_undo_step_and_refuse_bad_input() {
    let mut session = DocumentSession::default();
    commit_commands(
        &mut session,
        vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(30),
                name: "Box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DefinitionId(30),
                name: "Outline".to_owned(),
                kind: FeatureKind::polygon(&TRIANGLE),
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DefinitionId(30),
                name: "Extrusion".to_owned(),
                kind: FeatureKind::extrusion(PROFILE, Dimension::new("20", 20.0).unwrap()),
            },
        ],
    );
    let digest_before = session.snapshot().canonical_digest();
    let undo_before = session.visible_undo_steps();
    // A fourth corner, and corners on zero that a single length parameter cannot reach.
    let rectangle = [[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]];
    let proposal = session
        .plan_cad_program(&set_points(PROFILE, "points", &rectangle), &BTreeSet::new())
        .unwrap();
    assert_eq!(session.snapshot().canonical_digest(), digest_before);
    assert_eq!(session.visible_undo_steps(), undo_before);
    assert_eq!(
        outline(&session.apply_proposal(&proposal).unwrap()),
        rectangle
    );
    assert_eq!(session.visible_undo_steps(), undo_before + 1);
    session.undo().unwrap();
    assert_eq!(session.snapshot().canonical_digest(), digest_before);

    let triangle = [[0.0, 0.0], [12.0, 0.0], [0.0, 8.0]];
    let crossed = [[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]];
    for (feature, path, points_mm) in [
        (PROFILE, "points", &[[0.0, 0.0], [1.0, 0.0]][..]),
        (PROFILE, "points", &crossed[..]),
        (
            PROFILE,
            "points",
            &[[0.0, 0.0], [12.0, 0.0], [12.0, 0.0]][..],
        ),
        (PROFILE, "segments.0.end.x", &triangle[..]),
        (EXTRUSION, "points", &triangle[..]),
        (FeatureId(999), "points", &triangle[..]),
    ] {
        let program = set_points(feature, path, points_mm);
        let error = session
            .plan_cad_program(&program, &BTreeSet::new())
            .expect_err(&format!(
                "{path} {points_mm:?} on {feature:?} must be refused"
            ));
        assert!(
            matches!(
                error,
                SessionError::Planning(_) | SessionError::Prepare(_) | SessionError::Canonical(_)
            ),
            "{error:?}"
        );
        assert_eq!(session.snapshot().canonical_digest(), digest_before);
    }

    let proposal = session
        .plan_cad_program(&set_points(PROFILE, "points", &triangle), &BTreeSet::new())
        .unwrap();
    let concurrent = vec![[0.0, 0.0], [11.0, 0.0], [0.0, 9.0]];
    commit_commands(
        &mut session,
        vec![CanonicalCommand::SetProfilePoints {
            id: PROFILE,
            points_mm: concurrent.clone(),
        }],
    );
    assert!(matches!(
        session.apply_proposal(&proposal),
        Err(SessionError::Commit(ProposalCommitError::Stale(_)))
    ));
    assert_eq!(outline(&session.snapshot()), concurrent);
}

#[test]
fn make_occurrence_unique_gives_a_shared_part_its_own_definition_in_one_undo_step() {
    let mut session = DocumentSession::default();
    session
        .apply_cad_program(&program(vec![part()]), &BTreeSet::new())
        .unwrap();
    let source = session.snapshot().occurrences().next().unwrap().id();
    session
        .apply_cad_program(
            &program(vec![AssistantCadEditOperation::Copy {
                selector: selected(source),
                translation_mm: [100.0, 0.0, 0.0],
            }]),
            &BTreeSet::new(),
        )
        .unwrap();
    let snapshot = session.snapshot();
    let shared = snapshot.occurrence(source).unwrap().definition_id();
    let copy = snapshot
        .occurrences()
        .map(Occurrence::id)
        .find(|id| *id != source)
        .unwrap();
    assert_eq!(snapshot.occurrence(copy).unwrap().definition_id(), shared);
    let shared_features = snapshot.definition(shared).unwrap().feature_ids().len();
    let digest_before = snapshot.canonical_digest();
    let undo_before = session.visible_undo_steps();
    let unique = program(vec![AssistantCadEditOperation::MakeOccurrenceUnique {
        occurrence_id: copy.0,
    }]);

    let proposal = session.plan_cad_program(&unique, &BTreeSet::new()).unwrap();
    assert_eq!(session.snapshot().canonical_digest(), digest_before);
    assert_eq!(session.visible_undo_steps(), undo_before);
    let committed = session.apply_proposal(&proposal).unwrap();
    let own = committed.occurrence(copy).unwrap().definition_id();
    assert_ne!(own, shared);
    assert_eq!(
        committed.occurrence(source).unwrap().definition_id(),
        shared
    );
    assert_eq!(
        committed.definition(own).unwrap().feature_ids().len(),
        shared_features
    );
    assert_eq!(
        committed.definition(shared).unwrap().feature_ids().len(),
        shared_features
    );
    assert_eq!(session.visible_undo_steps(), undo_before + 1);
    session.undo().unwrap();
    assert_eq!(session.snapshot().canonical_digest(), digest_before);

    let proposal = session.plan_cad_program(&unique, &BTreeSet::new()).unwrap();
    commit_commands(
        &mut session,
        vec![CanonicalCommand::SetOccurrenceVisibility {
            id: copy,
            visible: false,
        }],
    );
    let changed = session.snapshot().canonical_digest();
    assert!(matches!(
        session.apply_proposal(&proposal),
        Err(SessionError::Commit(ProposalCommitError::Stale(_)))
    ));
    assert_eq!(session.snapshot().canonical_digest(), changed);
}
