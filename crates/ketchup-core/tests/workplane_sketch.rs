use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, FeatureKind,
};
use ketchup_core::exact_product::{
    BODY_SUBSHAPE_REF_SCHEMA_V1, BodySubshapeRef, ExactEdgeRole, ExactFaceRole,
    ExactFeatureChainRequest, ReferenceStability, build_box_render_package,
    canonical_reference_lineage_digest,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    PrincipalPlane, SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity,
    SketchEntityId, SketchError, SketchPointKind, SketchPointRef, SketchSolveStatus, SketchSpec,
    WorkplaneFrame, WorkplaneSpec, WorkplaneSupport, WorkplaneSupportHealth,
};
use ketchup_core::state_view::encode_semantic_state;

const DEFINITION: DefinitionId = DefinitionId(1);
const XY: FeatureId = FeatureId(10);
const OFFSET: FeatureId = FeatureId(11);
const SKETCH: FeatureId = FeatureId(12);

fn point(entity: u64, point: SketchPointKind) -> SketchPointRef {
    SketchPointRef {
        entity: SketchEntityId(entity),
        point,
    }
}

fn fully_constrained_circle(workplane: FeatureId) -> SketchSpec {
    SketchSpec {
        workplane,
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
                    point: point(1, SketchPointKind::Center),
                    position_mm: [10.0, 20.0],
                },
            },
        ],
    }
}

#[test]
fn principal_offset_and_fully_constrained_sketch_are_one_undoable_lossless_batch() {
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let offset = WorkplaneSpec {
        support: WorkplaneSupport::Offset {
            base: XY,
            distance: Dimension::from_decimal("25").unwrap(),
        },
        frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(25.0),
    };
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Constrained part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: XY,
                definition_id: DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Offset 25 mm".into(),
                kind: FeatureKind::Workplane(offset),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Circle sketch".into(),
                kind: FeatureKind::Sketch(fully_constrained_circle(OFFSET)),
            },
        ]))
        .unwrap();

    assert_eq!(document.visible_undo_steps(), 1);
    let committed = document.current();
    let FeatureKind::Sketch(spec) = committed.feature(SKETCH).unwrap().kind() else {
        panic!("expected canonical sketch");
    };
    assert_eq!(
        spec.solve().unwrap().status,
        SketchSolveStatus::FullyConstrained
    );
    let committed_digest = committed.canonical_digest();
    let bytes = persistence::save(&committed);
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
}

#[test]
fn line_arc_and_circle_keep_stable_ids_and_report_deterministic_remaining_dof() {
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [0.0, 0.0],
                end_mm: [10.0, 0.0],
            },
            SketchEntity::Arc {
                id: SketchEntityId(2),
                start_mm: [10.0, 0.0],
                end_mm: [0.0, 10.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
            SketchEntity::Circle {
                id: SketchEntityId(3),
                center_mm: [20.0, 20.0],
                radius_mm: 4.0,
            },
        ],
        constraints: vec![SketchConstraint {
            id: SketchConstraintId(1),
            kind: SketchConstraintKind::Horizontal {
                entity: SketchEntityId(1),
            },
        }],
    };

    let first = sketch.solve().unwrap();
    let second = sketch.solve().unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first.status,
        SketchSolveStatus::UnderConstrained { remaining_dof: 11 }
    );
    assert_eq!(
        sketch
            .entities
            .iter()
            .map(SketchEntity::id)
            .collect::<Vec<_>>(),
        vec![SketchEntityId(1), SketchEntityId(2), SketchEntityId(3)]
    );

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Mixed sketch".into(),
            },
            CanonicalCommand::CreateFeature {
                id: XY,
                definition_id: DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Mixed entities".into(),
                kind: FeatureKind::Sketch(sketch.clone()),
            },
        ]))
        .unwrap();
    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    let bytes = persistence::save(&committed);
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    let FeatureKind::Sketch(reopened_sketch) = reopened.feature(SKETCH).unwrap().kind() else {
        panic!("expected reopened sketch");
    };
    assert_eq!(&reopened_sketch.entities, &sketch.entities);
    assert_eq!(&reopened_sketch.constraints, &sketch.constraints);
    assert_eq!(reopened_sketch.solve().unwrap(), first);
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert_eq!(persistence::save(&reopened), bytes);
}

#[test]
fn unsatisfied_geometry_is_solved_deterministically_and_conflicts_fail_closed() {
    let line = SketchEntity::Line {
        id: SketchEntityId(1),
        start_mm: [0.0, 0.0],
        end_mm: [10.0, 5.0],
    };
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![line.clone()],
        constraints: vec![SketchConstraint {
            id: SketchConstraintId(1),
            kind: SketchConstraintKind::Horizontal {
                entity: SketchEntityId(1),
            },
        }],
    };
    let first = sketch.solve_geometry().unwrap();
    let second = sketch.solve_geometry().unwrap();
    assert_eq!(first, second);
    assert_eq!(sketch.entities, vec![line]);
    assert!(matches!(
        &first.entities[0],
        SketchEntity::Line {
            start_mm: [0.0, 2.5],
            end_mm: [10.0, 2.5],
            ..
        }
    ));

    let conflicting = SketchSpec {
        workplane: XY,
        entities: first.entities,
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Start),
                    position_mm: [0.0, 0.0],
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Start),
                    position_mm: [1.0, 0.0],
                },
            },
        ],
    };
    assert_eq!(
        conflicting.solve(),
        Err(SketchError::OverConstrained(SketchConstraintId(1)))
    );
}

#[test]
fn arc_rank_uses_center_radius_and_endpoint_angles_without_breaking_geometry() {
    let arc = SketchEntity::Arc {
        id: SketchEntityId(1),
        start_mm: [5.0, 0.0],
        end_mm: [0.0, 5.0],
        center_mm: [0.0, 0.0],
        clockwise: false,
    };
    let constraints = vec![
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
                point: point(1, SketchPointKind::Center),
                position_mm: [0.0, 0.0],
            },
        },
    ];
    let under = SketchSpec {
        workplane: XY,
        entities: vec![arc.clone()],
        constraints: constraints.clone(),
    };
    assert_eq!(
        under.solve().unwrap().status,
        SketchSolveStatus::UnderConstrained { remaining_dof: 2 }
    );

    let duplicate_radius = SketchSpec {
        workplane: XY,
        entities: vec![arc.clone()],
        constraints: vec![
            constraints[0].clone(),
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Distance {
                    a: point(1, SketchPointKind::Center),
                    b: point(1, SketchPointKind::Start),
                    value: Dimension::from_decimal("5.0").unwrap(),
                },
            },
        ],
    };
    assert_eq!(
        duplicate_radius.solve(),
        Err(SketchError::OverConstrained(SketchConstraintId(2)))
    );

    let fully = SketchSpec {
        workplane: XY,
        entities: vec![SketchEntity::Arc {
            id: SketchEntityId(1),
            start_mm: [8.0, 2.0],
            end_mm: [2.0, 8.0],
            center_mm: [2.0, 2.0],
            clockwise: false,
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
                    point: point(1, SketchPointKind::Start),
                    position_mm: [5.0, 0.0],
                },
            },
            SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::End),
                    position_mm: [0.0, 5.0],
                },
            },
        ],
    };
    let solved_fully = fully.solve_geometry().unwrap();
    assert_eq!(
        solved_fully.report.status,
        SketchSolveStatus::FullyConstrained
    );
    let SketchEntity::Arc {
        start_mm,
        end_mm,
        center_mm,
        ..
    } = &solved_fully.entities[0]
    else {
        panic!("expected solved arc");
    };
    assert_eq!(*start_mm, [5.0, 0.0]);
    assert_eq!(*end_mm, [0.0, 5.0]);
    assert!(center_mm[0].abs() < 1.0e-7 && center_mm[1].abs() < 1.0e-7);

    let partially_redundant = SketchSpec {
        workplane: XY,
        entities: vec![arc.clone()],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Center),
                    position_mm: [0.0, 0.0],
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Start),
                    position_mm: [5.0, 0.0],
                },
            },
            SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::End),
                    position_mm: [0.0, 5.0],
                },
            },
        ],
    };
    assert_eq!(
        partially_redundant.solve(),
        Err(SketchError::OverConstrained(SketchConstraintId(1)))
    );

    let moved = SketchSpec {
        workplane: XY,
        entities: vec![arc],
        constraints: vec![SketchConstraint {
            id: SketchConstraintId(1),
            kind: SketchConstraintKind::FixedPoint {
                point: point(1, SketchPointKind::Start),
                position_mm: [10.0, 0.0],
            },
        }],
    }
    .solve_geometry()
    .unwrap();
    let SketchEntity::Arc {
        start_mm,
        end_mm,
        center_mm,
        ..
    } = &moved.entities[0]
    else {
        panic!("expected solved arc");
    };
    assert_eq!(*start_mm, [10.0, 0.0]);
    assert!((10.0 - (end_mm[0] - center_mm[0]).hypot(end_mm[1] - center_mm[1])).abs() < 1.0e-7);

    for radial_constraint in [
        SketchConstraintKind::Radius {
            entity: SketchEntityId(1),
            value: Dimension::from_decimal("5").unwrap(),
        },
        SketchConstraintKind::Distance {
            a: point(1, SketchPointKind::Center),
            b: point(1, SketchPointKind::Start),
            value: Dimension::from_decimal("5").unwrap(),
        },
    ] {
        let radius_preserved = SketchSpec {
            workplane: XY,
            entities: vec![SketchEntity::Arc {
                id: SketchEntityId(1),
                start_mm: [5.0, 0.0],
                end_mm: [0.0, 5.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            }],
            constraints: vec![
                SketchConstraint {
                    id: SketchConstraintId(1),
                    kind: radial_constraint,
                },
                SketchConstraint {
                    id: SketchConstraintId(2),
                    kind: SketchConstraintKind::FixedPoint {
                        point: point(1, SketchPointKind::Start),
                        position_mm: [10.0, 0.0],
                    },
                },
            ],
        }
        .solve_geometry()
        .unwrap();
        let SketchEntity::Arc {
            start_mm,
            end_mm,
            center_mm,
            ..
        } = &radius_preserved.entities[0]
        else {
            panic!("expected solved arc");
        };
        assert_eq!(*start_mm, [10.0, 0.0]);
        assert_eq!(*center_mm, [5.0, 0.0]);
        assert_eq!(*end_mm, [5.0, 5.0]);
    }
}

#[test]
fn transitive_coincidence_is_rejected_as_redundant() {
    let sketch = SketchSpec {
        workplane: XY,
        entities: (1..=3)
            .map(|id| SketchEntity::Line {
                id: SketchEntityId(id),
                start_mm: [0.0, 0.0],
                end_mm: [id as f64, 1.0],
            })
            .collect(),
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Coincident {
                    a: point(1, SketchPointKind::Start),
                    b: point(2, SketchPointKind::Start),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Coincident {
                    a: point(2, SketchPointKind::Start),
                    b: point(3, SketchPointKind::Start),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::Coincident {
                    a: point(1, SketchPointKind::Start),
                    b: point(3, SketchPointKind::Start),
                },
            },
        ],
    };
    assert_eq!(
        sketch.solve(),
        Err(SketchError::OverConstrained(SketchConstraintId(3)))
    );
}

#[test]
fn all_principal_planes_and_one_resolved_planar_face_support_are_canonical() {
    let mut document = DocumentStore::new();
    let document_id = document.current().document_id();
    let profile = FeatureId(20);
    let extrusion = FeatureId(21);
    let yz = FeatureId(22);
    let xz = FeatureId(23);
    let face_plane = FeatureId(24);
    let role = ExactFaceRole::Top;

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Supported sketch".into(),
            },
            CanonicalCommand::CreateFeature {
                id: XY,
                definition_id: DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: yz,
                definition_id: DEFINITION,
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: xz,
                definition_id: DEFINITION,
                name: "XZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xz)),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: DEFINITION,
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: extrusion,
                definition_id: DEFINITION,
                name: "Extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
        ]))
        .unwrap();
    let request = ExactFeatureChainRequest::from_snapshot(&document.current(), DEFINITION).unwrap();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                document_id,
                extrusion,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}"),
        )
    });
    let package = build_box_render_package(
        &request,
        "exact-input".into(),
        "result".into(),
        "occt".into(),
        "r0".into(),
        [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        evidence,
    )
    .unwrap();
    let reference = package.reference(role).unwrap().clone();
    document
        .register_exact_reference_evidence(reference.clone())
        .unwrap();
    let edge_role = ExactEdgeRole::NorthEastVertical;
    let edge_reference = BodySubshapeRef {
        schema: BODY_SUBSHAPE_REF_SCHEMA_V1.to_owned(),
        document_id,
        definition_id: DEFINITION,
        profile_feature_id: profile,
        producer_feature_id: extrusion,
        semantic_role: edge_role.semantic_role().to_owned(),
        source_element_id: edge_role.source_element_id().to_owned(),
        expected_type: edge_role.expected_type().to_owned(),
        expected_cardinality: 1,
        stability: ReferenceStability::Guaranteed,
        canonical_input_digest: request.canonical_input_digest.clone(),
        exact_input_digest: package.identity.exact_input_digest.clone(),
        result_fingerprint: package.identity.result_fingerprint.clone(),
        evaluator: package.identity.evaluator.clone(),
        backend: package.identity.backend.clone(),
        tolerance: package.identity.tolerance.clone(),
        lineage_digest: canonical_reference_lineage_digest(
            document_id,
            extrusion,
            edge_role.semantic_role(),
            edge_role.source_element_id(),
            edge_role.expected_type(),
        ),
        corroborating_geometry_fingerprint: "edge-geometry".to_owned(),
    };
    document
        .register_exact_reference_evidence(edge_reference.clone())
        .unwrap();
    let edge_reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(
        edge_reopened
            .snapshot()
            .exact_reference_by_lineage(&edge_reference.lineage_digest),
        Some(&edge_reference)
    );
    let before_wrong_frame = document.current();
    let wrong_frame =
        document.apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: face_plane,
            definition_id: DEFINITION,
            name: "Wrong top face".into(),
            kind: FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::PlanarFace {
                    reference: Box::new(reference.clone()),
                    health: WorkplaneSupportHealth::Resolved,
                },
                frame: WorkplaneFrame::principal(PrincipalPlane::Xy),
            }),
        }]));
    assert!(matches!(
        wrong_frame,
        Err(CanonicalError::Sketch(
            SketchError::InvalidPlanarFaceSupport
        ))
    ));
    assert_eq!(
        document.current().canonical_digest(),
        before_wrong_frame.canonical_digest()
    );
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: face_plane,
            definition_id: DEFINITION,
            name: "Top face".into(),
            kind: FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::PlanarFace {
                    reference: Box::new(reference.clone()),
                    health: WorkplaneSupportHealth::Resolved,
                },
                frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(10.0),
            }),
        }]))
        .unwrap();
    let anchored = document.current();
    let mut conflicting_evidence = reference.clone();
    conflicting_evidence.result_fingerprint = "conflicting-result".into();
    assert!(
        document
            .register_exact_reference_evidence(conflicting_evidence)
            .is_err()
    );
    assert_eq!(
        document.current().canonical_digest(),
        anchored.canonical_digest()
    );

    let snapshot = document.current();
    for (id, plane) in [
        (XY, PrincipalPlane::Xy),
        (yz, PrincipalPlane::Yz),
        (xz, PrincipalPlane::Xz),
    ] {
        assert_eq!(
            snapshot.feature(id).unwrap().kind(),
            &FeatureKind::Workplane(WorkplaneSpec::principal(plane))
        );
    }
    let state = encode_semantic_state(&snapshot);
    let complete = state.complete_v1();
    assert!(state.agent_v1().contains(&format!(
        "kind:workplane,definition:{},support:planar_face:producer:{},role:{:?},health:Resolved",
        DEFINITION.0, extrusion.0, reference.semantic_role
    )));
    for expected in [
        format!(
            "feature.{}.support.document_id={}",
            face_plane.0, document_id.0
        ),
        format!(
            "feature.{}.support.definition_id={}",
            face_plane.0, DEFINITION.0
        ),
        format!(
            "feature.{}.support.profile_feature_id={}",
            face_plane.0, profile.0
        ),
        format!(
            "feature.{}.support.producer_feature_id={}",
            face_plane.0, extrusion.0
        ),
        format!(
            "feature.{}.support.semantic_role={:?}",
            face_plane.0, reference.semantic_role
        ),
        format!(
            "feature.{}.support.source_element_id={:?}",
            face_plane.0, reference.source_element_id
        ),
        format!(
            "feature.{}.support.expected_type={:?}",
            face_plane.0, reference.expected_type
        ),
        format!("feature.{}.support.expected_cardinality=1", face_plane.0),
        format!("feature.{}.support.stability=Guaranteed", face_plane.0),
        format!(
            "feature.{}.support.canonical_input_digest={:?}",
            face_plane.0, reference.canonical_input_digest
        ),
        format!(
            "feature.{}.support.exact_input_digest={:?}",
            face_plane.0, reference.exact_input_digest
        ),
        format!(
            "feature.{}.support.result_fingerprint={:?}",
            face_plane.0, reference.result_fingerprint
        ),
        format!(
            "feature.{}.support.evaluator={:?}",
            face_plane.0, reference.evaluator
        ),
        format!(
            "feature.{}.support.backend={:?}",
            face_plane.0, reference.backend
        ),
        format!(
            "feature.{}.support.tolerance={:?}",
            face_plane.0, reference.tolerance
        ),
        format!(
            "feature.{}.support.lineage_digest={:?}",
            face_plane.0, reference.lineage_digest
        ),
        format!(
            "feature.{}.support.geometry_fingerprint={:?}",
            face_plane.0, reference.corroborating_geometry_fingerprint
        ),
        format!("feature.{}.support.health=Resolved", face_plane.0),
    ] {
        assert!(
            complete.contains(&expected),
            "missing StateView line: {expected}"
        );
    }
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: extrusion,
                dimension: Dimension::from_decimal("12").unwrap(),
            },
        ]))
        .unwrap();
    let recomputed = document.current();
    let FeatureKind::Workplane(recomputed_plane) = recomputed.feature(face_plane).unwrap().kind()
    else {
        panic!("expected face workplane");
    };
    assert_eq!(recomputed_plane.frame.origin_mm, [0.0, 0.0, 12.0]);
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        recomputed.canonical_digest()
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: profile,
                points_mm: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 10.0], [0.0, 10.0]],
            },
        ]))
        .unwrap();
    let upstream_mutation = document.current();
    let FeatureKind::Workplane(upstream_plane) =
        upstream_mutation.feature(face_plane).unwrap().kind()
    else {
        panic!("expected face workplane");
    };
    let WorkplaneSupport::PlanarFace {
        reference: upstream_reference,
        health,
    } = &upstream_plane.support
    else {
        panic!("expected planar-face support");
    };
    assert_eq!(*health, WorkplaneSupportHealth::Stale);
    assert_eq!(upstream_reference.lineage_digest, reference.lineage_digest);
    assert!(upstream_mutation.revision_id() > recomputed.revision_id());

    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(matches!(
        reopened.snapshot().feature(face_plane).unwrap().kind(),
        FeatureKind::Workplane(WorkplaneSpec {
            support: WorkplaneSupport::PlanarFace {
                health: WorkplaneSupportHealth::Resolved,
                ..
            },
            ..
        })
    ));
}

#[test]
fn symmetric_constraints_are_canonical_and_tautological_coincidence_is_rejected() {
    let entities = vec![
        SketchEntity::Line {
            id: SketchEntityId(1),
            start_mm: [0.0, 0.0],
            end_mm: [10.0, 0.0],
        },
        SketchEntity::Line {
            id: SketchEntityId(2),
            start_mm: [10.0, 0.0],
            end_mm: [20.0, 0.0],
        },
    ];
    let a = point(1, SketchPointKind::End);
    let b = point(2, SketchPointKind::Start);
    let c = point(1, SketchPointKind::Start);
    for kinds in [
        vec![
            SketchConstraintKind::Coincident { a, b },
            SketchConstraintKind::Coincident { a: b, b: a },
        ],
        vec![
            SketchConstraintKind::Distance {
                a: c,
                b,
                value: Dimension::from_decimal("10").unwrap(),
            },
            SketchConstraintKind::Distance {
                a: b,
                b: c,
                value: Dimension::from_decimal("10.0").unwrap(),
            },
        ],
    ] {
        let sketch = SketchSpec {
            workplane: XY,
            entities: entities.clone(),
            constraints: kinds
                .into_iter()
                .enumerate()
                .map(|(index, kind)| SketchConstraint {
                    id: SketchConstraintId(index as u64 + 1),
                    kind,
                })
                .collect(),
        };
        assert_eq!(
            sketch.solve(),
            Err(SketchError::OverConstrained(SketchConstraintId(2)))
        );
    }

    let tautology = SketchSpec {
        workplane: XY,
        entities,
        constraints: vec![SketchConstraint {
            id: SketchConstraintId(1),
            kind: SketchConstraintKind::Coincident { a, b: a },
        }],
    };
    assert_eq!(
        tautology.solve(),
        Err(SketchError::OverConstrained(SketchConstraintId(1)))
    );
}

#[test]
fn invalid_cycles_constraints_and_stale_face_support_fail_without_mutation() {
    let mut document = DocumentStore::new();
    let before = document.current();
    let cycle = document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Invalid".into(),
        },
        CanonicalCommand::CreateFeature {
            id: XY,
            definition_id: DEFINITION,
            name: "Offset A".into(),
            kind: FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::Offset {
                    base: OFFSET,
                    distance: Dimension::from_decimal("1").unwrap(),
                },
                frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(1.0),
            }),
        },
        CanonicalCommand::CreateFeature {
            id: OFFSET,
            definition_id: DEFINITION,
            name: "Offset B".into(),
            kind: FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::Offset {
                    base: XY,
                    distance: Dimension::from_decimal("1").unwrap(),
                },
                frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(2.0),
            }),
        },
    ]));
    assert!(matches!(
        cycle,
        Err(CanonicalError::Sketch(SketchError::WorkplaneCycle(_)))
    ));
    assert_eq!(document.current().revision_id(), before.revision_id());
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 0);

    let redundant = SketchSpec {
        workplane: XY,
        entities: vec![SketchEntity::Line {
            id: SketchEntityId(1),
            start_mm: [0.0, 0.0],
            end_mm: [10.0, 0.0],
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Horizontal {
                    entity: SketchEntityId(1),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Horizontal {
                    entity: SketchEntityId(1),
                },
            },
        ],
    };
    assert_eq!(
        redundant.solve(),
        Err(SketchError::OverConstrained(SketchConstraintId(2)))
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Redo-preserving seed".into(),
            },
        ]))
        .unwrap();
    document.undo().unwrap();
    let before_refusal = document.current();
    let revision_count = document.revision_count();
    let undo_steps = document.visible_undo_steps();
    let redo_steps = document.visible_redo_steps();
    let refused = document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Invalid sketch".into(),
        },
        CanonicalCommand::CreateFeature {
            id: XY,
            definition_id: DEFINITION,
            name: "XY".into(),
            kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
        },
        CanonicalCommand::CreateFeature {
            id: SKETCH,
            definition_id: DEFINITION,
            name: "Redundant".into(),
            kind: FeatureKind::Sketch(redundant),
        },
    ]));
    assert!(matches!(
        refused,
        Err(CanonicalError::Sketch(SketchError::OverConstrained(
            SketchConstraintId(2)
        )))
    ));
    assert_eq!(
        document.current().revision_id(),
        before_refusal.revision_id()
    );
    assert_eq!(
        document.current().canonical_digest(),
        before_refusal.canonical_digest()
    );
    assert_eq!(document.revision_count(), revision_count);
    assert_eq!(document.visible_undo_steps(), undo_steps);
    assert_eq!(document.visible_redo_steps(), redo_steps);

    let stale_face = WorkplaneSpec {
        support: WorkplaneSupport::PlanarFace {
            reference: Box::new(BodySubshapeRef {
                schema: BODY_SUBSHAPE_REF_SCHEMA_V1.into(),
                document_id: before.document_id(),
                definition_id: DEFINITION,
                profile_feature_id: XY,
                producer_feature_id: OFFSET,
                semantic_role: "top".into(),
                source_element_id: "face:top".into(),
                expected_type: "face".into(),
                expected_cardinality: 1,
                stability: ReferenceStability::Guaranteed,
                canonical_input_digest: "canonical".into(),
                exact_input_digest: "exact".into(),
                result_fingerprint: "result".into(),
                evaluator: "evaluator".into(),
                backend: "backend".into(),
                tolerance: "tolerance".into(),
                lineage_digest: "lineage".into(),
                corroborating_geometry_fingerprint: "geometry".into(),
            }),
            health: WorkplaneSupportHealth::Stale,
        },
        frame: WorkplaneFrame::principal(PrincipalPlane::Xy),
    };
    assert_eq!(
        stale_face.validate_local(),
        Err(SketchError::UnresolvedWorkplaneSupport(
            WorkplaneSupportHealth::Stale
        ))
    );
}
