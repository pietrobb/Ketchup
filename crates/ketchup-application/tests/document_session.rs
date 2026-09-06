use ketchup_application::evaluation::*;
use ketchup_application::{
    AssistantValidationSelection, DocumentSession, SaveOptions, SessionError, SessionSettings,
    StructuralValidationScope, scoped_static_load_report,
};
use ketchup_core::{
    assistant_sidecar::*, document::*, exact_product::ExactResultRegistry,
    persistence::ContainerData,
};
use std::{collections::BTreeSet, time::Duration};
fn part() -> AssistantCadEditOperation {
    AssistantCadEditOperation::CreatePart {
        name: "Editable part".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [0.0, 0.0],
            radius_mm: 12.0,
        }],
        constraints: vec![AssistantSketchConstraint::Radius {
            id: 1,
            entity_id: 1,
            value_mm: 12.0,
        }],
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 30.0 },
        translation_mm: [5.0, 6.0, 7.0],
        rotation: None,
    }
}

fn program() -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![part()],
    }
}
fn worker_settings() -> SessionSettings {
    let path = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    SessionSettings {
        exact_worker_path: Some(path),
        evaluation_timeout: Duration::from_secs(30),
    }
}
#[test]
fn real_worker_session_save_open_and_read_only_reports() {
    let settings = worker_settings();
    let mut session = DocumentSession::new(settings.clone());
    let empty = session.snapshot();
    let proposal = session
        .plan_cad_program(&program(), &BTreeSet::new())
        .unwrap();
    assert_eq!(session.visible_undo_steps(), 0);
    assert_eq!(
        session.snapshot().canonical_digest(),
        empty.canonical_digest()
    );
    let snapshot = session.apply_proposal(&proposal).unwrap();
    let report = session.evaluate().unwrap();
    assert!(report.complete, "{report:?}");
    assert!(report.topology_complete, "{report:?}");
    assert_eq!(report.producers.len(), 1);
    assert_eq!(session.visible_undo_steps(), 1);
    assert_eq!(
        session.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    let validation = session.validators(&AssistantValidationSelection::only(&["collision"]));
    assert_eq!(validation["canonical_digest"], snapshot.canonical_digest());
    assert_eq!(session.visible_undo_steps(), 1);
    assert!(
        session
            .evaluate()
            .unwrap()
            .producers
            .iter()
            .all(|entry| entry.render == EvidenceStatus::Current)
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(session.save(&path, SaveOptions::default()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let mut reopened = DocumentSession::open(&path, settings).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert_eq!(reopened.visible_undo_steps(), 0);
    assert!(reopened.evaluate().unwrap().complete);
    assert_eq!(
        reopened.validators(&AssistantValidationSelection::only(&["collision"])),
        validation
    );
    session.undo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        empty.canonical_digest()
    );
    session.redo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    session.set_grounded(OccurrenceId(1), true).unwrap();
    assert!(session.snapshot().occurrence_is_grounded(OccurrenceId(1)));
    session.undo().unwrap();
    assert!(!session.snapshot().occurrence_is_grounded(OccurrenceId(1)));
}
#[test]
fn missing_worker_empty_coverage_and_invalid_inputs_are_honest() {
    let mut session = DocumentSession::new(SessionSettings {
        exact_worker_path: Some("nonexistent-worker".into()),
        ..SessionSettings::default()
    });
    let report = session.evaluate().unwrap();
    assert!(!report.complete);
    assert!(report.not_evaluated.is_some());
    let invalid = AssistantCadEditProgram { operations: vec![] };
    assert!(matches!(
        session.plan_cad_program(&invalid, &BTreeSet::new()),
        Err(SessionError::Planning(_))
    ));
    assert!(
        serde_json::from_str::<AssistantCadEditProgram>(
            r#"{"operations":[{"operation":"does_not_exist"}]}"#
        )
        .is_err()
    );
    session
        .apply_cad_program(&program(), &BTreeSet::new())
        .unwrap();
    let report = session.evaluate().unwrap();
    assert!(!report.complete);
    assert_eq!(report.producers.len(), 1);
    assert!(matches!(
        report.producers[0].render,
        EvidenceStatus::NotEvaluated { .. }
    ));
    let validation = session.validators(&AssistantValidationSelection::only(&["bogus"]));
    assert_eq!(validation["state"], "not_evaluated");
    assert_eq!(
        session.validators(&AssistantValidationSelection::only(&[]))["complete"],
        false
    );
    assert_eq!(session.visible_undo_steps(), 1);
}
#[test]
fn shared_poll_wait_cancel_and_stale_publication() {
    let settings = worker_settings();
    let mut document = DocumentStore::new();
    let batch = ketchup_application::plan_assistant_cad_edit_program(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let mut render = ExactResultRegistry::default();
    let mut topology = ExactResultRegistry::default();
    let task = start_exact_evaluation(
        document.current(),
        &ContainerData::default(),
        &render,
        &topology,
        settings.exact_worker_path.clone(),
        || {},
    );
    let products = task.wait(Duration::from_secs(30)).unwrap();
    let report =
        publish_exact_products(&mut document, &mut render, &mut topology, &task, products).unwrap();
    assert!(report.complete);
    let task = start_exact_evaluation(
        document.current(),
        &ContainerData::default(),
        &render,
        &topology,
        settings.exact_worker_path.clone(),
        || {},
    );
    let products = loop {
        match task.poll() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("{error}"),
        }
    };
    document.undo().unwrap();
    assert!(
        publish_exact_products(&mut document, &mut render, &mut topology, &task, products).is_err()
    );
    let task = start_exact_evaluation(
        document.current(),
        &ContainerData::default(),
        &render,
        &topology,
        settings.exact_worker_path,
        || {},
    );
    task.cancel();
    assert!(task.wait(Duration::from_secs(1)).is_err());
}

use ketchup_core::sketch::*;
fn rectangle_sketch(workplane: FeatureId, min_mm: [f64; 2], max_mm: [f64; 2]) -> SketchSpec {
    let corners = [
        min_mm,
        [max_mm[0], min_mm[1]],
        max_mm,
        [min_mm[0], max_mm[1]],
    ];
    let entities = (0..4)
        .map(|index| SketchEntity::Line {
            id: SketchEntityId(index as u64 + 1),
            start_mm: corners[index],
            end_mm: corners[(index + 1) % corners.len()],
        })
        .collect::<Vec<_>>();
    let mut constraints = Vec::new();
    for index in 0..4 {
        let entity = SketchEntityId(index as u64 + 1);
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 1),
            kind: SketchConstraintKind::FixedPoint {
                point: SketchPointRef {
                    entity,
                    point: SketchPointKind::Start,
                },
                position_mm: corners[index],
            },
        });
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 2),
            kind: SketchConstraintKind::FixedPoint {
                point: SketchPointRef {
                    entity,
                    point: SketchPointKind::End,
                },
                position_mm: corners[(index + 1) % corners.len()],
            },
        });
    }
    SketchSpec {
        workplane,
        entities,
        constraints,
    }
}

fn apply_session_batch(
    session: &mut DocumentSession,
    batch: &CommandBatch,
) -> Result<Snapshot, SessionError> {
    let proposal = session.plan_commands(batch.clone())?;
    session.apply_proposal(&proposal)
}
#[test]
fn real_worker_face_supported_pocket_keeps_intermediate_and_roundtrips() {
    let definition = DefinitionId(2);
    let base_plane = FeatureId(20);
    let base_sketch_id = FeatureId(21);
    let pad = FeatureId(22);
    let face_plane = FeatureId(23);
    let pocket_sketch_id = FeatureId(24);
    let pocket = FeatureId(25);
    let base_sketch = rectangle_sketch(base_plane, [10.0, 20.0], [110.0, 80.0]);
    let base_region = base_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentSession::new(worker_settings());
    apply_session_batch(
        &mut document,
        &CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Pad Pocket".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_plane,
                definition_id: definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: base_sketch_id,
                definition_id: definition,
                name: "Base rectangle".into(),
                kind: FeatureKind::Sketch(base_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: definition,
                name: "Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: base_sketch_id,
                    region: base_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("18").unwrap()),
                }),
            },
        ]),
    )
    .unwrap();

    apply_session_batch(
        &mut document,
        &CommandBatch::new(vec![CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(1),
            definition_id: definition,
            name: "Pad".into(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }]),
    )
    .unwrap();
    assert!(document.evaluate().unwrap().complete);
    let package = document.exact_results().values().next().unwrap();
    let ketchup_core::exact_product::ExactBodyPackage::Rectangle(pad_package) = package.as_ref()
    else {
        panic!("expected rectangle fast path")
    };
    let top = pad_package
        .reference(ketchup_core::exact_product::ExactFaceRole::Top)
        .unwrap()
        .clone();
    let pocket_sketch = rectangle_sketch(face_plane, [30.0, 20.0], [50.0, 35.0]);
    let pocket_region = pocket_sketch.solved_regions().unwrap()[0].id;
    apply_session_batch(
        &mut document,
        &CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: face_plane,
                definition_id: definition,
                name: "Pad top".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: WorkplaneFrame {
                        origin_mm: [10.0, 20.0, 18.0],
                        x_axis: [1.0, 0.0, 0.0],
                        y_axis: [0.0, 1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                }),
            },
            CanonicalCommand::CreateFeature {
                id: pocket_sketch_id,
                definition_id: definition,
                name: "Pocket rectangle".into(),
                kind: FeatureKind::Sketch(pocket_sketch),
            },
        ]),
    )
    .unwrap();
    apply_session_batch(
        &mut document,
        &CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: pocket,
            definition_id: definition,
            name: "Pocket".into(),
            kind: FeatureKind::SketchPocket(PocketSpec {
                target: pad,
                sketch: pocket_sketch_id,
                region: pocket_region,
                support: Box::new(top),
                direction: FeatureDirection::OppositeNormal,
                extent: FeatureExtent::Blind(Dimension::from_decimal("6").unwrap()),
            }),
        }]),
    )
    .unwrap();
    let before = document.snapshot();
    let undo = document.visible_undo_steps();
    let report = document.evaluate().unwrap();
    assert!(report.complete, "{report:?}");
    assert!(
        report
            .producers
            .iter()
            .any(|entry| entry.key.feature_id == pocket)
    );
    assert!(
        report
            .producers
            .iter()
            .any(|entry| entry.key.feature_id == pad)
    );
    assert_eq!(document.visible_undo_steps(), undo);
    assert_eq!(
        document.snapshot().canonical_digest(),
        before.canonical_digest()
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pocket.ketchup");
    document.save(&path, SaveOptions::default()).unwrap();
    let mut reopened = DocumentSession::open(&path, worker_settings()).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        before.canonical_digest()
    );
    assert!(reopened.evaluate().unwrap().complete);
}

#[test]
fn typed_cad_program_commits_static_metadata_atomically() {
    let mut session = DocumentSession::default();
    session
        .apply_cad_program(
            &AssistantCadEditProgram {
                operations: vec![part(), part()],
            },
            &BTreeSet::new(),
        )
        .unwrap();
    let geometry = session.snapshot();
    let metadata = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::UpsertClassificationDimension {
                dimension_id: 1,
                name: "ketchup.validator-role.v1".into(),
                categories: vec![
                    AssistantCadClassificationCategory {
                        id: 1,
                        name: "physics.static.load:case-a".into(),
                    },
                    AssistantCadClassificationCategory {
                        id: 2,
                        name: "physics.static.support:case-a".into(),
                    },
                ],
            },
            AssistantCadEditOperation::SetOccurrenceClassification {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: vec![1],
                },
                dimension_id: 1,
                category_id: Some(1),
            },
            AssistantCadEditOperation::SetOccurrenceClassification {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: vec![2],
                },
                dimension_id: 1,
                category_id: Some(2),
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 1,
                name: "physics.gravity_x_m_s2".into(),
                value: 0.0,
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 2,
                name: "physics.gravity_y_m_s2".into(),
                value: 0.0,
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 3,
                name: "physics.gravity_z_m_s2".into(),
                value: -9.81,
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 4,
                name: "physics.mass_kg.occurrence.1".into(),
                value: 100.0,
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 5,
                name: "physics.applied_load_n.occurrence.1".into(),
                value: 200.0,
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 6,
                name: "physics.support_capacity_n.occurrence.2".into(),
                value: 2_000.0,
            },
        ],
    };

    let proposal = session
        .plan_cad_program(&metadata, &BTreeSet::new())
        .unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        geometry.canonical_digest()
    );
    assert_eq!(session.visible_undo_steps(), 1);
    let committed = session.apply_proposal(&proposal).unwrap();
    assert_eq!(session.visible_undo_steps(), 2);
    assert_eq!(
        committed.occurrence_classification(OccurrenceId(1), ClassificationDimensionId(1)),
        Some(ClassificationCategoryId(1))
    );
    let scope = StructuralValidationScope::bind(&committed, [OccurrenceId(1)]);
    let report = scoped_static_load_report(&committed, &scope, || false);
    assert_eq!(report["state"], "passed", "{report:#}");
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["coverage"]["checked_load_occurrence_count"], 1);

    session.undo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        geometry.canonical_digest()
    );
}
