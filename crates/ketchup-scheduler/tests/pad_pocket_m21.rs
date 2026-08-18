use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureEvaluationState, FeatureId, FeatureKind, MultiBodyBooleanPlan,
    NewBodyFeaturePlan, ProposalContext, ToolBodyPolicy,
};
use ketchup_core::exact_product::{
    EXACT_BOOLEAN_UNION_EVALUATOR_V1, EXACT_POCKET_EVALUATOR_V1, ExactBodyPackage, ExactFaceRole,
    ExactFeatureChainRequest, ExactProductError, ExactResultRegistry,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadPocketOperation, PadSpec, PocketSpec, PrincipalPlane,
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId,
    SketchPointKind, SketchPointRef, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use ketchup_scheduler::{ExactWorkerClient, ExactWorkerSupervisor};
use std::collections::{BTreeMap, BTreeSet};
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
    const BASE_BODY: BodyId = BodyId(1);
    const OTHER_BODY: BodyId = BodyId(2);
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
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: OTHER_BODY,
                name: "Other body".into(),
                visible: true,
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
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: OTHER_BODY,
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
    let requests = ExactFeatureChainRequest::terminal_body_requests(&source, DEFINITION).unwrap();
    assert_eq!(
        requests.keys().copied().collect::<Vec<_>>(),
        vec![BASE_BODY, OTHER_BODY]
    );
    assert_eq!(requests[&BASE_BODY].producer_feature_id(), POCKET);
    assert_eq!(requests[&OTHER_BODY].producer_feature_id(), OTHER_PAD);
    let pocket_request = &requests[&BASE_BODY];
    let other_request = &requests[&OTHER_BODY];
    let pocket_package = Arc::new(ExactBodyPackage::from(
        supervisor.evaluate_rectangle(pocket_request).unwrap(),
    ));
    let other_package = Arc::new(ExactBodyPackage::from(
        supervisor.evaluate_rectangle(other_request).unwrap(),
    ));
    let registry = ExactResultRegistry::publish_body_results(
        &source,
        &ExactResultRegistry::default(),
        [Arc::clone(&other_package), Arc::clone(&pocket_package)],
    )
    .unwrap();
    let permuted_registry = ExactResultRegistry::publish_body_results(
        &source,
        &ExactResultRegistry::default(),
        [Arc::clone(&pocket_package), Arc::clone(&other_package)],
    )
    .unwrap();
    let ordered_results = |registry: &ExactResultRegistry| {
        registry
            .body_values(&source)
            .unwrap()
            .into_iter()
            .map(|(key, package)| {
                (
                    key,
                    package.result_key().result_fingerprint.clone(),
                    package.references()[0].lineage_digest.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ordered_results(&registry),
        ordered_results(&permuted_registry)
    );
    assert_eq!(registry.len(), 2);
    let body_values = registry.body_values(&source).unwrap();
    assert_eq!(
        body_values
            .keys()
            .map(|key| (key.body_id, key.producer_feature_id))
            .collect::<Vec<_>>(),
        vec![(BASE_BODY, POCKET), (OTHER_BODY, OTHER_PAD)]
    );
    assert!(Arc::ptr_eq(
        registry
            .get_body(&source, DEFINITION, BASE_BODY)
            .unwrap()
            .unwrap(),
        &pocket_package
    ));
    let registry_stamp = registry.contents_stamp();
    let base_request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&source, DEFINITION, BASE_PAD)
            .unwrap();
    let non_terminal = Arc::new(ExactBodyPackage::from(
        supervisor.evaluate_rectangle(&base_request).unwrap(),
    ));
    assert!(matches!(
        ExactResultRegistry::publish_body_results(&source, &registry, [non_terminal]),
        Err(ExactProductError::NonTerminalBodyResult {
            definition_id: DEFINITION,
            body_id: BASE_BODY,
            producer_feature_id: BASE_PAD
        })
    ));
    assert!(matches!(
        ExactResultRegistry::publish_body_results(
            &source,
            &ExactResultRegistry::default(),
            [Arc::clone(&pocket_package), Arc::clone(&pocket_package)]
        ),
        Err(ExactProductError::ConflictingBodyPublication {
            definition_id: DEFINITION,
            body_id: BASE_BODY
        })
    ));
    assert_eq!(registry.contents_stamp(), registry_stamp);

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
        ExactResultRegistry::publish_body_results(
            &changed,
            &registry,
            [Arc::clone(&pocket_package)]
        ),
        Err(ExactProductError::StaleResult)
    ));
    assert_eq!(registry.contents_stamp(), registry_stamp);
    let carried_only = ExactResultRegistry::carried_forward(&changed, &registry);
    assert_eq!(carried_only.len(), 1);
    assert_eq!(
        carried_only.values().next().unwrap().producer_feature_id(),
        OTHER_PAD
    );
    let changed_request =
        ExactFeatureChainRequest::from_snapshot_for_body(&changed, DEFINITION, BASE_BODY).unwrap();
    let changed_package = Arc::new(ExactBodyPackage::from(
        supervisor.evaluate_rectangle(&changed_request).unwrap(),
    ));
    let carried = ExactResultRegistry::publish_body_results(
        &changed,
        &registry,
        [Arc::clone(&changed_package)],
    )
    .unwrap();
    assert_eq!(carried.len(), 2);
    assert!(Arc::ptr_eq(
        carried
            .get_body(&changed, DEFINITION, BASE_BODY)
            .unwrap()
            .unwrap(),
        &changed_package
    ));
    let carried_other = carried
        .get_body(&changed, DEFINITION, OTHER_BODY)
        .unwrap()
        .unwrap();
    assert_eq!(
        carried_other.result_key().result_fingerprint,
        other_package.result_key().result_fingerprint
    );
    assert_eq!(
        carried_other.references()[0].lineage_digest,
        other_package.references()[0].lineage_digest
    );
    assert_eq!(
        carried_other.references()[0].corroborating_geometry_fingerprint,
        other_package.references()[0].corroborating_geometry_fingerprint
    );
    let last_valid_registry_stamp = carried.contents_stamp();
    let last_valid_body_outputs = carried
        .values()
        .map(|package| {
            (
                package.producer_feature_id(),
                package.result_key().result_fingerprint.clone(),
                package.references()[0].lineage_digest.clone(),
            )
        })
        .collect::<Vec<_>>();

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
    let before_ambiguous = (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    );
    document
        .register_exact_reference_evidence(&ambiguous)
        .unwrap();
    assert_eq!(
        (
            document.current().revision_id(),
            document.current().canonical_digest(),
            document.visible_undo_steps(),
            document.visible_redo_steps(),
        ),
        before_ambiguous
    );
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
    assert!(
        ExactFeatureChainRequest::from_snapshot_for_body(
            &ambiguous_snapshot,
            DEFINITION,
            BASE_BODY
        )
        .is_err()
    );
    assert_eq!(carried.contents_stamp(), last_valid_registry_stamp);
    assert_eq!(
        carried
            .values()
            .map(|package| {
                (
                    package.producer_feature_id(),
                    package.result_key().result_fingerprint.clone(),
                    package.references()[0].lineage_digest.clone(),
                )
            })
            .collect::<Vec<_>>(),
        last_valid_body_outputs
    );
    assert_eq!(
        ExactResultRegistry::carried_forward(&ambiguous_snapshot, &carried)
            .values()
            .map(|package| package.producer_feature_id())
            .collect::<Vec<_>>(),
        vec![OTHER_PAD]
    );

    let before_lost = (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    );
    document
        .register_exact_reference_evidence(&ExactResultRegistry::default())
        .unwrap();
    assert_eq!(
        (
            document.current().revision_id(),
            document.current().canonical_digest(),
            document.visible_undo_steps(),
            document.visible_redo_steps(),
        ),
        before_lost
    );
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
    assert!(
        ExactFeatureChainRequest::from_snapshot_for_body(&lost_snapshot, DEFINITION, BASE_BODY)
            .is_err()
    );
    assert_eq!(carried.contents_stamp(), last_valid_registry_stamp);
    assert_eq!(
        carried
            .values()
            .map(|package| {
                (
                    package.producer_feature_id(),
                    package.result_key().result_fingerprint.clone(),
                    package.references()[0].lineage_digest.clone(),
                )
            })
            .collect::<Vec<_>>(),
        last_valid_body_outputs
    );
    assert_eq!(
        ExactResultRegistry::carried_forward(&lost_snapshot, &carried)
            .values()
            .map(|package| package.producer_feature_id())
            .collect::<Vec<_>>(),
        vec![OTHER_PAD]
    );

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

#[test]
fn reviewed_multibody_authoring_drives_exact_union_and_preserves_failed_evaluation_state() {
    const DEFINITION: DefinitionId = DefinitionId(4);
    const BASE_BODY: BodyId = BodyId(1);
    const TOOL_BODY: BodyId = BodyId(2);
    const BASE_PROFILE: FeatureId = FeatureId(50);
    const BASE_EXTRUSION: FeatureId = FeatureId(51);
    const TOOL_PROFILE: FeatureId = FeatureId(52);
    const TOOL_EXTRUSION: FeatureId = FeatureId(53);
    const UNION: FeatureId = FeatureId(54);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Reviewed multi-body exact part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PROFILE,
                definition_id: DEFINITION,
                name: "Base profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: BASE_EXTRUSION,
                definition_id: DEFINITION,
                name: "Base extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: BASE_PROFILE,
                    height: Dimension::from_decimal("5").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_PROFILE,
                definition_id: DEFINITION,
                name: "Tool profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 10.0], [0.0, 10.0]],
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    let stamp = |document: &DocumentStore| {
        (
            document.current().revision_id(),
            document.current().canonical_digest(),
            document.visible_undo_steps(),
            document.visible_redo_steps(),
        )
    };

    let before_body = stamp(&document);
    let create_body = document
        .plan_new_body_feature(
            NewBodyFeaturePlan {
                definition_id: DEFINITION,
                body_id: TOOL_BODY,
                body_name: "Tool body".into(),
                feature_id: TOOL_EXTRUSION,
                feature_name: "Tool extrusion".into(),
                feature_kind: FeatureKind::Extrusion {
                    profile: TOOL_PROFILE,
                    height: Dimension::from_decimal("5").unwrap(),
                },
            },
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    assert_eq!(stamp(&document), before_body);
    document.commit_proposal(&create_body).unwrap();
    assert_eq!(document.visible_undo_steps(), before_body.2 + 1);
    let two_body_snapshot = document.current();
    assert_eq!(
        ExactFeatureChainRequest::terminal_body_requests(&two_body_snapshot, DEFINITION)
            .unwrap()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![BASE_BODY, TOOL_BODY]
    );

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_request =
        ExactFeatureChainRequest::from_snapshot_for_body(&two_body_snapshot, DEFINITION, BASE_BODY)
            .unwrap();
    let tool_request =
        ExactFeatureChainRequest::from_snapshot_for_body(&two_body_snapshot, DEFINITION, TOOL_BODY)
            .unwrap();
    let base_package = Arc::new(ExactBodyPackage::from(
        supervisor.evaluate_rectangle(&base_request).unwrap(),
    ));
    let tool_package = Arc::new(ExactBodyPackage::from(
        supervisor.evaluate_rectangle(&tool_request).unwrap(),
    ));
    let registry = ExactResultRegistry::publish_body_results(
        &two_body_snapshot,
        &ExactResultRegistry::default(),
        [Arc::clone(&base_package), Arc::clone(&tool_package)],
    )
    .unwrap();

    let edit_tool = document
        .prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::SetFeatureDimension {
                id: TOOL_EXTRUSION,
                dimension: Dimension::from_decimal("6").unwrap(),
            }]),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    let before_edit = stamp(&document);
    let edited_revision = document.commit_proposal(&edit_tool).unwrap();
    assert_eq!(
        edited_revision.dirty_features(),
        &BTreeSet::from([TOOL_EXTRUSION])
    );
    assert_eq!(
        edited_revision.feature_states()[&BASE_EXTRUSION],
        FeatureEvaluationState::Current
    );
    assert_eq!(document.visible_undo_steps(), before_edit.2 + 1);
    let edited_snapshot = document.current();
    let carried_base = ExactResultRegistry::carried_forward(&edited_snapshot, &registry);
    assert_eq!(carried_base.len(), 1);
    assert_eq!(
        carried_base.values().next().unwrap().producer_feature_id(),
        BASE_EXTRUSION
    );
    let edited_tool_request =
        ExactFeatureChainRequest::from_snapshot_for_body(&edited_snapshot, DEFINITION, TOOL_BODY)
            .unwrap();
    let edited_tool_package = Arc::new(ExactBodyPackage::from(
        supervisor.evaluate_rectangle(&edited_tool_request).unwrap(),
    ));
    let edited_registry = ExactResultRegistry::publish_body_results(
        &edited_snapshot,
        &registry,
        [Arc::clone(&edited_tool_package)],
    )
    .unwrap();
    let carried_base_package = edited_registry
        .get_body(&edited_snapshot, DEFINITION, BASE_BODY)
        .unwrap()
        .unwrap();
    assert_eq!(
        carried_base_package.result_key().result_fingerprint,
        base_package.result_key().result_fingerprint
    );
    assert_eq!(
        carried_base_package.references()[0].lineage_digest,
        base_package.references()[0].lineage_digest
    );
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        two_body_snapshot.canonical_digest()
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        edited_snapshot.canonical_digest()
    );
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        two_body_snapshot.canonical_digest()
    );

    let before_union = stamp(&document);
    let union = document
        .plan_multibody_boolean(
            MultiBodyBooleanPlan {
                definition_id: DEFINITION,
                operation: BooleanOperation::Union,
                target_body_id: BASE_BODY,
                target_feature_id: BASE_EXTRUSION,
                tool_body_id: TOOL_BODY,
                tool_feature_id: TOOL_EXTRUSION,
                result_feature_id: UNION,
                result_feature_name: "Reviewed union".into(),
                tool_policy: ToolBodyPolicy::Preserve,
            },
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    assert_eq!(stamp(&document), before_union);
    document.commit_proposal(&union).unwrap();
    assert_eq!(document.visible_undo_steps(), before_union.2 + 1);
    let union_snapshot = document.current();
    let requests =
        ExactFeatureChainRequest::terminal_body_requests(&union_snapshot, DEFINITION).unwrap();
    assert_eq!(
        requests.keys().copied().collect::<Vec<_>>(),
        vec![BASE_BODY, TOOL_BODY]
    );
    assert_eq!(requests[&BASE_BODY].producer_feature_id(), UNION);
    assert_eq!(
        requests[&BASE_BODY].evaluator(),
        EXACT_BOOLEAN_UNION_EVALUATOR_V1
    );
    assert_eq!(requests[&TOOL_BODY].producer_feature_id(), TOOL_EXTRUSION);

    let union_package = supervisor
        .evaluate_rectangle(&requests[&BASE_BODY])
        .unwrap();
    let repeated_union = supervisor
        .evaluate_rectangle(&requests[&BASE_BODY])
        .unwrap();
    assert_eq!(union_package.identity, repeated_union.identity);
    assert_eq!(union_package.references, repeated_union.references);
    assert_eq!(
        union_package.bounds_mm,
        [[0.0, 0.0, 0.0], [30.0, 10.0, 5.0]]
    );
    assert!(union_package.references.iter().all(|reference| {
        reference.has_valid_lineage()
            && reference.producer_feature_id == UNION
            && reference.matches_request(&requests[&BASE_BODY])
    }));
    let mut edge_use = BTreeMap::<(u32, u32), usize>::new();
    let mut signed_volume_mm3 = 0.0;
    for triangle in &union_package.triangles {
        let [a, b, c] = triangle
            .vertex_indices
            .map(|index| union_package.vertices[index as usize].position_mm);
        signed_volume_mm3 += (a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
        for edge in [
            (triangle.vertex_indices[0], triangle.vertex_indices[1]),
            (triangle.vertex_indices[1], triangle.vertex_indices[2]),
            (triangle.vertex_indices[2], triangle.vertex_indices[0]),
        ] {
            *edge_use
                .entry((edge.0.min(edge.1), edge.0.max(edge.1)))
                .or_default() += 1;
        }
    }
    assert!((signed_volume_mm3.abs() - 1_500.0).abs() < 1.0e-6);
    assert!(edge_use.values().all(|count| *count == 2));

    let union_registry = ExactResultRegistry::publish_body_results(
        &union_snapshot,
        &registry,
        [Arc::new(ExactBodyPackage::from(union_package))],
    )
    .unwrap();
    let preserved_tool = union_registry
        .get_body(&union_snapshot, DEFINITION, TOOL_BODY)
        .unwrap()
        .unwrap();
    assert_eq!(
        preserved_tool.result_key().result_fingerprint,
        tool_package.result_key().result_fingerprint
    );
    assert_eq!(
        preserved_tool.references()[0].lineage_digest,
        tool_package.references()[0].lineage_digest
    );

    let union_digest = union_snapshot.canonical_digest();
    let union_bytes = persistence::save(&union_snapshot);
    let reopened = persistence::load(&union_bytes).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), union_digest);
    assert_eq!(persistence::save(&reopened), union_bytes);
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        two_body_snapshot.canonical_digest()
    );
    assert_eq!(document.redo().unwrap().canonical_digest(), union_digest);

    let before_failure = stamp(&document);
    let registry_before_failure = union_registry.contents_stamp();
    let mut failed_worker =
        ExactWorkerClient::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    failed_worker.crash().unwrap();
    assert!(
        failed_worker
            .extrude_rectangle_request(&requests[&BASE_BODY])
            .is_err()
    );
    assert_eq!(stamp(&document), before_failure);
    assert_eq!(union_registry.contents_stamp(), registry_before_failure);
}
