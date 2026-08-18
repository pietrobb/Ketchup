use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureEvaluationState,
    FeatureId, FeatureKind, ProposalContext,
};
use ketchup_core::exact_product::{
    EXACT_POCKET_EVALUATOR_V1, ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest,
    ExactProductError, ExactResultRegistry,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadPocketOperation, PadSpec, PocketSpec, PrincipalPlane,
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId,
    SketchPointKind, SketchPointRef, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::collections::BTreeSet;
use std::sync::Arc;

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

#[test]
fn worker_evaluates_workplane_bound_pad_and_rejects_stale_extent_result() {
    let definition = DefinitionId(1);
    let plane = FeatureId(10);
    let sketch_id = FeatureId(11);
    let pad = FeatureId(12);
    let sketch = SketchSpec {
        workplane: plane,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [10.0, 20.0],
            radius_mm: 5.0,
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: Dimension::from_decimal("5").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: SketchPointRef {
                        entity: SketchEntityId(1),
                        point: SketchPointKind::Center,
                    },
                    position_mm: [10.0, 20.0],
                },
            },
        ],
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "YZ Pad".into(),
            },
            CanonicalCommand::CreateFeature {
                id: plane,
                definition_id: definition,
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id: definition,
                name: "Solved circle".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: definition,
                name: "Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("25").unwrap()),
                }),
            },
        ]))
        .unwrap();

    let source = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&source, definition).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    assert_eq!(package.bounds_mm, [[0.0, 5.0, 15.0], [25.0, 15.0, 25.0]]);
    assert!(package.is_current(&source));
    ExactResultRegistry::accept(&source, [Arc::new(package.clone().into())]).unwrap();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: pad,
                dimension: Dimension::from_decimal("30").unwrap(),
            },
        ]))
        .unwrap();
    assert!(matches!(
        ExactResultRegistry::accept(&document.current(), [Arc::new(package.into())]),
        Err(ExactProductError::StaleResult)
    ));
}

#[test]
fn worker_evaluates_face_supported_sketch_pocket_and_rejects_stale_depth_result() {
    let definition = DefinitionId(2);
    let base_plane = FeatureId(20);
    let base_sketch_id = FeatureId(21);
    let pad = FeatureId(22);
    let face_plane = FeatureId(23);
    let pocket_sketch_id = FeatureId(24);
    let pocket = FeatureId(25);
    let base_sketch = rectangle_sketch(base_plane, [10.0, 20.0], [110.0, 80.0]);
    let base_region = base_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
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
        ]))
        .unwrap();

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let pad_request =
        ExactFeatureChainRequest::from_snapshot(&document.current(), definition).unwrap();
    let pad_package = supervisor.evaluate_rectangle(&pad_request).unwrap();
    let top = pad_package.reference(ExactFaceRole::Top).unwrap().clone();
    for reference in pad_package.references {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }

    let pocket_sketch = rectangle_sketch(face_plane, [30.0, 20.0], [50.0, 35.0]);
    let pocket_region = pocket_sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
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
        ]))
        .unwrap();
    let proposal = document
        .plan_pad_pocket(
            pocket,
            definition,
            "Pocket",
            PadPocketOperation::Pocket(PocketSpec {
                target: pad,
                sketch: pocket_sketch_id,
                region: pocket_region,
                support: Box::new(top),
                direction: FeatureDirection::OppositeNormal,
                extent: FeatureExtent::Blind(Dimension::from_decimal("6").unwrap()),
            }),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    document.commit_verified_proposal(&proposal).unwrap();

    let source = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&source, definition).unwrap();
    assert_eq!(request.producer_feature_id(), pocket);
    assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    assert_eq!(package.bounds_mm, [[10.0, 20.0, 0.0], [110.0, 80.0, 18.0]]);
    assert_eq!(package.vertices.len(), 16);
    assert_eq!(package.triangles.len(), 28);
    for role in [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
        ExactFaceRole::PocketFloor,
        ExactFaceRole::PocketWest,
        ExactFaceRole::PocketEast,
        ExactFaceRole::PocketSouth,
        ExactFaceRole::PocketNorth,
    ] {
        let reference = package
            .reference(role)
            .expect("stable Pocket face reference");
        assert_eq!(reference.producer_feature_id, pocket);
        assert!(reference.has_valid_lineage());
    }
    ExactResultRegistry::accept(&source, [Arc::new(package.clone().into())]).unwrap();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: pocket,
                dimension: Dimension::from_decimal("7").unwrap(),
            },
        ]))
        .unwrap();
    assert!(matches!(
        ExactResultRegistry::accept(&document.current(), [Arc::new(package.into())]),
        Err(ExactProductError::StaleResult)
    ));
}

#[test]
fn branched_pocket_recompute_preserves_unrelated_pad_and_root_failure_identity() {
    const DEFINITION: DefinitionId = DefinitionId(3);
    const BASE_PLANE: FeatureId = FeatureId(30);
    const BASE_SKETCH: FeatureId = FeatureId(31);
    const BASE_PAD: FeatureId = FeatureId(32);
    const FACE_PLANE: FeatureId = FeatureId(33);
    const POCKET_SKETCH: FeatureId = FeatureId(34);
    const POCKET: FeatureId = FeatureId(35);
    const OTHER_PLANE: FeatureId = FeatureId(40);
    const OTHER_SKETCH: FeatureId = FeatureId(41);
    const OTHER_PAD: FeatureId = FeatureId(42);

    let base_sketch = rectangle_sketch(BASE_PLANE, [0.0, 0.0], [100.0, 60.0]);
    let base_region = base_sketch.solved_regions().unwrap()[0].id;
    let other_sketch = rectangle_sketch(OTHER_PLANE, [0.0, 0.0], [30.0, 15.0]);
    let other_region = other_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Branched exact part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PLANE,
                definition_id: DEFINITION,
                name: "Base plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_SKETCH,
                definition_id: DEFINITION,
                name: "Base sketch".into(),
                kind: FeatureKind::Sketch(base_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PAD,
                definition_id: DEFINITION,
                name: "Base pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: BASE_SKETCH,
                    region: base_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("18").unwrap()),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OTHER_PLANE,
                definition_id: DEFINITION,
                name: "Other plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: OTHER_SKETCH,
                definition_id: DEFINITION,
                name: "Other sketch".into(),
                kind: FeatureKind::Sketch(other_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: OTHER_PAD,
                definition_id: DEFINITION,
                name: "Other pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: OTHER_SKETCH,
                    region: other_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("12").unwrap()),
                }),
            },
        ]))
        .unwrap();

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_request = ExactFeatureChainRequest::from_snapshot_for_producer(
        &document.current(),
        DEFINITION,
        BASE_PAD,
    )
    .unwrap();
    let base_package = supervisor.evaluate_rectangle(&base_request).unwrap();
    let top = base_package.reference(ExactFaceRole::Top).unwrap().clone();
    for reference in base_package.references {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }

    let pocket_sketch = rectangle_sketch(FACE_PLANE, [20.0, 15.0], [40.0, 30.0]);
    let pocket_region = pocket_sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FACE_PLANE,
                definition_id: DEFINITION,
                name: "Pad top".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: WorkplaneFrame {
                        origin_mm: [0.0, 0.0, 18.0],
                        x_axis: [1.0, 0.0, 0.0],
                        y_axis: [0.0, 1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                }),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET_SKETCH,
                definition_id: DEFINITION,
                name: "Pocket sketch".into(),
                kind: FeatureKind::Sketch(pocket_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Pocket".into(),
                kind: FeatureKind::SketchPocket(PocketSpec {
                    target: BASE_PAD,
                    sketch: POCKET_SKETCH,
                    region: pocket_region,
                    support: Box::new(top.clone()),
                    direction: FeatureDirection::OppositeNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("6").unwrap()),
                }),
            },
        ]))
        .unwrap();

    let source = document.current();
    let pocket_request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&source, DEFINITION, POCKET).unwrap();
    let other_request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&source, DEFINITION, OTHER_PAD)
            .unwrap();
    let pocket_package = Arc::new(ExactBodyPackage::from(
        supervisor.evaluate_rectangle(&pocket_request).unwrap(),
    ));
    let other_package = Arc::new(ExactBodyPackage::from(
        supervisor.evaluate_rectangle(&other_request).unwrap(),
    ));
    let registry = ExactResultRegistry::accept(
        &source,
        [Arc::clone(&pocket_package), Arc::clone(&other_package)],
    )
    .unwrap();
    assert_eq!(registry.len(), 2);

    let source_bytes = persistence::save(&source);
    let reopened = persistence::load(&source_bytes).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        source.canonical_digest()
    );
    assert_eq!(persistence::save(&reopened.snapshot()), source_bytes);

    let revision = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: POCKET,
                dimension: Dimension::from_decimal("7").unwrap(),
            },
        ]))
        .unwrap();
    assert_eq!(revision.dirty_features(), &BTreeSet::from([POCKET]));
    assert_eq!(
        revision.feature_states()[&POCKET],
        FeatureEvaluationState::Stale
    );
    assert_eq!(
        revision.feature_states()[&OTHER_PAD],
        FeatureEvaluationState::Current
    );

    let failed = document
        .current()
        .feature_dependency_graph()
        .unwrap()
        .evaluation_states(&BTreeSet::new(), &BTreeSet::from([BASE_SKETCH]));
    assert_eq!(
        failed[&POCKET],
        FeatureEvaluationState::Error {
            failed_at: BASE_SKETCH
        }
    );
    assert_eq!(failed[&OTHER_PAD], FeatureEvaluationState::Current);

    let changed = document.current();
    assert!(matches!(
        ExactResultRegistry::accept(&changed, [pocket_package]),
        Err(ExactProductError::StaleResult)
    ));
    let mut carried = ExactResultRegistry::carried_forward(&changed, &registry);
    assert_eq!(carried.len(), 1);
    assert_eq!(
        carried.values().next().unwrap().producer_feature_id(),
        OTHER_PAD
    );
    let changed_request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&changed, DEFINITION, POCKET).unwrap();
    carried
        .insert_current(
            &changed,
            Arc::new(ExactBodyPackage::from(
                supervisor.evaluate_rectangle(&changed_request).unwrap(),
            )),
        )
        .unwrap();
    assert_eq!(carried.len(), 2);

    let changed_bytes = persistence::save(&changed);
    let changed_reopened = persistence::load(&changed_bytes).unwrap();
    assert_eq!(
        changed_reopened.snapshot().canonical_digest(),
        changed.canonical_digest()
    );
    assert_eq!(
        persistence::save(&changed_reopened.snapshot()),
        changed_bytes
    );
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        source.canonical_digest()
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        changed.canonical_digest()
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: BASE_PAD,
                dimension: Dimension::from_decimal("20").unwrap(),
            },
        ]))
        .unwrap();
    let stale = document.current();
    let FeatureKind::Workplane(stale_plane) = stale.feature(FACE_PLANE).unwrap().kind() else {
        panic!("expected face-supported workplane");
    };
    assert!(matches!(
        stale_plane.support,
        WorkplaneSupport::PlanarFace {
            health: WorkplaneSupportHealth::Stale,
            ..
        }
    ));
    assert!(
        ExactFeatureChainRequest::from_snapshot_for_producer(&stale, DEFINITION, POCKET).is_err()
    );

    let changed_base_request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&stale, DEFINITION, BASE_PAD).unwrap();
    let changed_base = supervisor
        .evaluate_rectangle(&changed_base_request)
        .unwrap();
    let changed_top = changed_base.reference(ExactFaceRole::Top).unwrap().clone();
    assert_eq!(changed_top.lineage_digest, top.lineage_digest);
    assert_ne!(
        changed_top.corroborating_geometry_fingerprint,
        top.corroborating_geometry_fingerprint
    );

    let mut incompatible = changed_base.clone();
    incompatible.identity.backend.push_str("-alternate");
    for reference in &mut incompatible.references {
        reference.backend = incompatible.identity.backend.clone();
    }
    let ambiguous = ExactResultRegistry::accept(
        &stale,
        [
            Arc::new(ExactBodyPackage::from(changed_base.clone())),
            Arc::new(ExactBodyPackage::from(incompatible)),
        ],
    )
    .unwrap();
    document
        .register_exact_reference_evidence(&ambiguous)
        .unwrap();
    let ambiguous_snapshot = document.current();
    let FeatureKind::Workplane(ambiguous_plane) =
        ambiguous_snapshot.feature(FACE_PLANE).unwrap().kind()
    else {
        panic!("expected face-supported workplane");
    };
    assert!(matches!(
        ambiguous_plane.support,
        WorkplaneSupport::PlanarFace {
            health: WorkplaneSupportHealth::Ambiguous,
            ..
        }
    ));

    document
        .register_exact_reference_evidence(&ExactResultRegistry::default())
        .unwrap();
    let lost_snapshot = document.current();
    let FeatureKind::Workplane(lost_plane) = lost_snapshot.feature(FACE_PLANE).unwrap().kind()
    else {
        panic!("expected face-supported workplane");
    };
    assert!(matches!(
        lost_plane.support,
        WorkplaneSupport::PlanarFace {
            health: WorkplaneSupportHealth::Lost,
            ..
        }
    ));

    let resolved =
        ExactResultRegistry::accept(&stale, [Arc::new(ExactBodyPackage::from(changed_base))])
            .unwrap();
    document
        .register_exact_reference_evidence(&resolved)
        .unwrap();
    let rebound = document.current();
    assert_eq!(rebound.canonical_digest(), stale.canonical_digest());
    let FeatureKind::Workplane(rebound_plane) = rebound.feature(FACE_PLANE).unwrap().kind() else {
        panic!("expected face-supported workplane");
    };
    let WorkplaneSupport::PlanarFace {
        reference: rebound_reference,
        health,
    } = &rebound_plane.support
    else {
        panic!("expected planar-face support");
    };
    assert_eq!(*health, WorkplaneSupportHealth::Resolved);
    assert_eq!(rebound_reference.as_ref(), &changed_top);
    let FeatureKind::SketchPocket(rebound_pocket) = rebound.feature(POCKET).unwrap().kind() else {
        panic!("expected Pocket");
    };
    assert_eq!(rebound_pocket.support.as_ref(), &changed_top);
    ExactFeatureChainRequest::from_snapshot_for_producer(&rebound, DEFINITION, POCKET).unwrap();

    let rebound_bytes = persistence::save(&rebound);
    let reopened_rebound = persistence::load(&rebound_bytes).unwrap().snapshot();
    assert_eq!(
        reopened_rebound.canonical_digest(),
        rebound.canonical_digest()
    );
    let FeatureKind::Workplane(reopened_plane) =
        reopened_rebound.feature(FACE_PLANE).unwrap().kind()
    else {
        panic!("expected reopened face-supported workplane");
    };
    assert!(matches!(
        reopened_plane.support,
        WorkplaneSupport::PlanarFace {
            health: WorkplaneSupportHealth::Resolved,
            ..
        }
    ));
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        changed.canonical_digest()
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        rebound.canonical_digest()
    );
}
