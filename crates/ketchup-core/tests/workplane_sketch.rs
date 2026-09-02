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
    MAX_SKETCH_CONSTRAINTS, MAX_SKETCH_ENTITIES, PrincipalPlane, SketchConstraint,
    SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId, SketchError,
    SketchPointKind, SketchPointRef, SketchSolveStatus, SketchSolverPolicy, SketchSpec,
    SolvedSketchRegionEdge, SolvedSketchRegionProfile, WorkplaneFrame, WorkplaneSpec,
    WorkplaneSupport, WorkplaneSupportHealth,
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
fn underconstrained_closed_geometry_produces_deterministic_regions() {
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [0.0, 0.0],
                end_mm: [10.0, 0.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [10.0, 0.0],
                end_mm: [10.0, 10.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(3),
                start_mm: [10.0, 10.0],
                end_mm: [0.0, 10.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [0.0, 10.0],
                end_mm: [0.0, 0.0],
            },
        ],
        constraints: Vec::new(),
    };

    assert_eq!(
        sketch.solve().unwrap().status,
        SketchSolveStatus::UnderConstrained { remaining_dof: 16 }
    );
    let first = sketch.solved_regions().unwrap();
    assert_eq!(first, sketch.solved_regions().unwrap());
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0].profile,
        SolvedSketchRegionProfile::Polyline(vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
        ])
    );
}

#[test]
fn region_identity_survives_edge_reversal_and_boundary_orientation_stays_contiguous() {
    let sketches = [
        SketchSpec {
            workplane: XY,
            entities: vec![
                SketchEntity::Line {
                    id: SketchEntityId(1),
                    start_mm: [-5.0, 0.0],
                    end_mm: [5.0, 0.0],
                },
                SketchEntity::Arc {
                    id: SketchEntityId(2),
                    start_mm: [-5.0, 0.0],
                    end_mm: [5.0, 0.0],
                    center_mm: [0.0, 0.0],
                    clockwise: true,
                },
            ],
            constraints: Vec::new(),
        },
        SketchSpec {
            workplane: XY,
            entities: vec![
                SketchEntity::Line {
                    id: SketchEntityId(1),
                    start_mm: [5.0, 0.0],
                    end_mm: [-5.0, 0.0],
                },
                SketchEntity::Arc {
                    id: SketchEntityId(2),
                    start_mm: [5.0, 0.0],
                    end_mm: [-5.0, 0.0],
                    center_mm: [0.0, 0.0],
                    clockwise: false,
                },
            ],
            constraints: Vec::new(),
        },
    ];

    let regions = sketches.map(|sketch| sketch.solved_regions().unwrap().remove(0));
    assert_eq!(regions[0].id, regions[1].id);
    assert_eq!(regions[0].entity_ids, regions[1].entity_ids);
    for region in regions {
        let SolvedSketchRegionProfile::Boundary(edges) = region.profile else {
            panic!("expected line/arc boundary");
        };
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].end_mm(), edges[1].start_mm());
        assert_eq!(edges[1].end_mm(), edges[0].start_mm());
    }
}

#[test]
fn line_arc_boundary_is_oriented_and_open_or_branched_geometry_fails_closed() {
    let semicircle = SketchSpec {
        workplane: XY,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [-5.0, 0.0],
                end_mm: [5.0, 0.0],
            },
            SketchEntity::Arc {
                id: SketchEntityId(2),
                start_mm: [-5.0, 0.0],
                end_mm: [5.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: true,
            },
        ],
        constraints: Vec::new(),
    };
    let first = semicircle.solved_regions().unwrap();
    let second = semicircle.solved_regions().unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first[0].entity_ids,
        vec![SketchEntityId(1), SketchEntityId(2)]
    );
    assert_eq!(
        first[0].profile,
        SolvedSketchRegionProfile::Boundary(vec![
            SolvedSketchRegionEdge::Line {
                start_mm: [-5.0, 0.0],
                end_mm: [5.0, 0.0],
            },
            SolvedSketchRegionEdge::Arc {
                start_mm: [5.0, 0.0],
                end_mm: [-5.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
        ])
    );

    let open = SketchSpec {
        workplane: XY,
        entities: semicircle.entities[..1].to_vec(),
        constraints: Vec::new(),
    };
    assert_eq!(open.solved_regions(), Err(SketchError::OpenRegion));

    let mut branched_entities = semicircle.entities;
    branched_entities.push(SketchEntity::Line {
        id: SketchEntityId(3),
        start_mm: [5.0, 0.0],
        end_mm: [5.0, 5.0],
    });
    let branched = SketchSpec {
        workplane: XY,
        entities: branched_entities,
        constraints: Vec::new(),
    };
    assert_eq!(
        branched.solved_regions(),
        Err(SketchError::InvalidRegionIdentity)
    );
}

#[test]
fn zero_area_closed_boundary_fails_closed() {
    let degenerate = SketchSpec {
        workplane: XY,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [0.0, 0.0],
                end_mm: [1.0, 0.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [1.0, 0.0],
                end_mm: [2.0, 0.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(3),
                start_mm: [2.0, 0.0],
                end_mm: [0.0, 0.0],
            },
        ],
        constraints: Vec::new(),
    };

    assert_eq!(degenerate.solved_regions(), Err(SketchError::OpenRegion));
}

#[test]
fn entity_constraint_and_solver_dof_limits_fail_closed() {
    let line = |id| SketchEntity::Line {
        id: SketchEntityId(id),
        start_mm: [id as f64, 0.0],
        end_mm: [id as f64, 1.0],
    };
    let entity_limited = SketchSpec {
        workplane: XY,
        entities: vec![line(1); MAX_SKETCH_ENTITIES + 1],
        constraints: Vec::new(),
    };
    assert_eq!(entity_limited.solve(), Err(SketchError::ResourceLimit));

    let constraint_limited = SketchSpec {
        workplane: XY,
        entities: vec![line(1)],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Horizontal {
                    entity: SketchEntityId(1),
                },
            };
            MAX_SKETCH_CONSTRAINTS + 1
        ],
    };
    assert_eq!(constraint_limited.solve(), Err(SketchError::ResourceLimit));

    let solver_limited = SketchSpec {
        workplane: XY,
        entities: (1..=129).map(line).collect(),
        constraints: Vec::new(),
    };
    assert_eq!(solver_limited.solve(), Err(SketchError::ResourceLimit));
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
fn coupled_constraints_use_least_squares_and_nonconvergence_is_not_overconstraint() {
    let source = SketchEntity::Line {
        id: SketchEntityId(1),
        start_mm: [1.0, 2.0],
        end_mm: [8.0, 7.0],
    };
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![source.clone()],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Horizontal {
                    entity: SketchEntityId(1),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Distance {
                    a: point(1, SketchPointKind::Start),
                    b: point(1, SketchPointKind::End),
                    value: Dimension::from_decimal("10").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Start),
                    position_mm: [0.0, 0.0],
                },
            },
        ],
    };

    let first = sketch.solve_geometry().unwrap();
    let second = sketch.solve_geometry().unwrap();
    assert_eq!(first, second);
    assert_eq!(sketch.entities, vec![source.clone()]);
    assert_eq!(first.report.status, SketchSolveStatus::FullyConstrained);
    let SketchEntity::Line {
        start_mm, end_mm, ..
    } = &first.entities[0]
    else {
        panic!("expected solved line");
    };
    assert!(start_mm[0].abs() <= 1.0e-7 && start_mm[1].abs() <= 1.0e-7);
    assert!(((end_mm[0] - start_mm[0]).hypot(end_mm[1] - start_mm[1]) - 10.0).abs() <= 1.0e-7);
    assert!((start_mm[1] - end_mm[1]).abs() <= 1.0e-7);

    let bounded = SketchSolverPolicy {
        max_iterations: 1,
        initial_damping: 1.0e20,
        ..SketchSolverPolicy::default()
    };
    assert_eq!(
        sketch.solve_geometry_with_policy(bounded),
        Err(SketchError::NonConvergent)
    );
    assert_eq!(sketch.entities, vec![source]);
    assert_eq!(
        sketch.solve_with_policy(SketchSolverPolicy {
            max_iterations: 0,
            ..SketchSolverPolicy::default()
        }),
        Err(SketchError::InvalidSolverPolicy)
    );
}

#[test]
fn conflicting_dimensions_are_overconstrained_before_numerical_solving() {
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![SketchEntity::Line {
            id: SketchEntityId(1),
            start_mm: [0.0, 0.0],
            end_mm: [5.0, 0.0],
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Distance {
                    a: point(1, SketchPointKind::Start),
                    b: point(1, SketchPointKind::End),
                    value: Dimension::from_decimal("10").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Distance {
                    a: point(1, SketchPointKind::Start),
                    b: point(1, SketchPointKind::End),
                    value: Dimension::from_decimal("20").unwrap(),
                },
            },
        ],
    };
    assert_eq!(
        sketch.solve(),
        Err(SketchError::OverConstrained(SketchConstraintId(1)))
    );
}

#[test]
fn coincidence_partition_exposes_conflicting_dimensions() {
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [0.0, 0.0],
                end_mm: [10.0, 0.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [0.0, 1.0],
                end_mm: [0.0, 5.0],
            },
        ],
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
                kind: SketchConstraintKind::Distance {
                    a: point(1, SketchPointKind::Start),
                    b: point(1, SketchPointKind::End),
                    value: Dimension::from_decimal("10").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::Distance {
                    a: point(2, SketchPointKind::Start),
                    b: point(1, SketchPointKind::End),
                    value: Dimension::from_decimal("20").unwrap(),
                },
            },
        ],
    };
    assert_eq!(
        sketch.solve(),
        Err(SketchError::OverConstrained(SketchConstraintId(2)))
    );
}

#[test]
fn numerical_solving_preserves_disconnected_entities_bit_for_bit() {
    let circle = SketchEntity::Circle {
        id: SketchEntityId(2),
        center_mm: [50.0, 50.0],
        radius_mm: 5.0e-7,
    };
    let arc = SketchEntity::Arc {
        id: SketchEntityId(3),
        start_mm: [105.0, 100.0],
        end_mm: [100.0, 105.000_000_05],
        center_mm: [100.0, 100.0],
        clockwise: false,
    };
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [1.0, 2.0],
                end_mm: [8.0, 7.0],
            },
            circle.clone(),
            arc.clone(),
        ],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Horizontal {
                    entity: SketchEntityId(1),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Distance {
                    a: point(1, SketchPointKind::Start),
                    b: point(1, SketchPointKind::End),
                    value: Dimension::from_decimal("10").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Start),
                    position_mm: [0.0, 0.0],
                },
            },
        ],
    };
    let solved = sketch.solve_geometry().unwrap();
    assert_eq!(solved.entities[1], circle);
    assert_eq!(solved.entities[2], arc);
    assert_eq!(
        sketch.solve_with_policy(SketchSolverPolicy {
            tolerance_mm: 1.0,
            ..SketchSolverPolicy::default()
        }),
        Err(SketchError::InvalidSolverPolicy)
    );
}

#[test]
fn aliased_arc_radius_distance_is_one_dimensional_target() {
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![
            SketchEntity::Arc {
                id: SketchEntityId(1),
                start_mm: [5.0, 0.0],
                end_mm: [0.0, 5.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [0.0, 0.0],
                end_mm: [1.0, 1.0],
            },
        ],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Coincident {
                    a: point(1, SketchPointKind::Center),
                    b: point(2, SketchPointKind::Start),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: Dimension::from_decimal("5").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::Distance {
                    a: point(2, SketchPointKind::Start),
                    b: point(1, SketchPointKind::Start),
                    value: Dimension::from_decimal("6").unwrap(),
                },
            },
        ],
    };
    assert_eq!(
        sketch.solve(),
        Err(SketchError::OverConstrained(SketchConstraintId(2)))
    );
}

#[test]
fn tiny_translated_arc_uses_non_prefix_active_columns() {
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [0.0, 0.0],
                end_mm: [1.0, 1.0],
            },
            SketchEntity::Arc {
                id: SketchEntityId(2),
                start_mm: [999_998.999_999_8, 999_999.000_000_5],
                end_mm: [999_998.999_999_3, 999_999.0],
                center_mm: [999_998.999_999_8, 999_999.0],
                clockwise: false,
            },
        ],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(2, SketchPointKind::Start),
                    position_mm: [999_999.000_000_5, 999_999.0],
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(2, SketchPointKind::End),
                    position_mm: [999_999.0, 999_999.000_000_5],
                },
            },
        ],
    };
    let solved = sketch.solve_geometry().unwrap();
    assert_eq!(solved.entities[0], sketch.entities[0]);
    let SketchEntity::Arc {
        start_mm, end_mm, ..
    } = solved.entities[1]
    else {
        panic!("expected solved arc");
    };
    assert!((start_mm[0] - 999_999.000_000_5).abs() <= 1.0e-7);
    assert!((start_mm[1] - 999_999.0).abs() <= 1.0e-7);
    assert!((end_mm[0] - 999_999.0).abs() <= 1.0e-7);
    assert!((end_mm[1] - 999_999.000_000_5).abs() <= 1.0e-7);
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
fn full_general_constraint_vocabulary_solves_geometric_invariants() {
    let line = |id, start_mm, end_mm| SketchEntity::Line {
        id: SketchEntityId(id),
        start_mm,
        end_mm,
    };
    let circle = |id, center_mm, radius_mm| SketchEntity::Circle {
        id: SketchEntityId(id),
        center_mm,
        radius_mm,
    };
    let fixed_line = |constraint_id, entity, start_mm, end_mm| {
        vec![
            SketchConstraint {
                id: SketchConstraintId(constraint_id),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(entity, SketchPointKind::Start),
                    position_mm: start_mm,
                },
            },
            SketchConstraint {
                id: SketchConstraintId(constraint_id + 1),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(entity, SketchPointKind::End),
                    position_mm: end_mm,
                },
            },
        ]
    };
    let solve = |entities, constraints| {
        SketchSpec {
            workplane: XY,
            entities,
            constraints,
        }
        .solve_geometry()
        .unwrap()
        .entities
    };
    let solved_line = |entities: &[SketchEntity], id| {
        let SketchEntity::Line {
            start_mm, end_mm, ..
        } = entities
            .iter()
            .find(|entity| entity.id() == SketchEntityId(id))
            .unwrap()
        else {
            panic!("expected line");
        };
        (*start_mm, *end_mm)
    };
    let direction = |line: ([f64; 2], [f64; 2])| {
        let delta = [line.1[0] - line.0[0], line.1[1] - line.0[1]];
        let length = delta[0].hypot(delta[1]);
        [delta[0] / length, delta[1] / length]
    };
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    let dot = |a: [f64; 2], b: [f64; 2]| a[0] * b[0] + a[1] * b[1];

    for (kind, expected_dot) in [
        (
            SketchConstraintKind::Parallel {
                a: SketchEntityId(1),
                b: SketchEntityId(2),
            },
            Some(1.0),
        ),
        (
            SketchConstraintKind::Perpendicular {
                a: SketchEntityId(1),
                b: SketchEntityId(2),
            },
            Some(0.0),
        ),
        (
            SketchConstraintKind::Angle {
                a: SketchEntityId(1),
                b: SketchEntityId(2),
                angle_degrees: 60.0,
            },
            Some(0.5),
        ),
        (
            SketchConstraintKind::Collinear {
                a: SketchEntityId(1),
                b: SketchEntityId(2),
            },
            None,
        ),
    ] {
        let mut constraints = fixed_line(1, 1, [0.0, 0.0], [10.0, 0.0]);
        constraints.push(SketchConstraint {
            id: SketchConstraintId(3),
            kind,
        });
        let entities = solve(
            vec![
                line(1, [0.0, 0.0], [10.0, 0.0]),
                line(2, [1.0, 4.0], [8.0, 7.0]),
            ],
            constraints,
        );
        let a = direction(solved_line(&entities, 1));
        let b_line = solved_line(&entities, 2);
        let b = direction(b_line);
        if let Some(expected) = expected_dot {
            if expected == 1.0 {
                assert!(cross(a, b).abs() <= 1.0e-7);
            } else {
                assert!((dot(a, b) - expected).abs() <= 1.0e-7);
            }
        } else {
            assert!(cross(a, b).abs() <= 1.0e-7);
            assert!(b_line.0[1].abs() <= 1.0e-7);
        }
    }

    let mut equal_constraints = fixed_line(1, 1, [0.0, 0.0], [10.0, 0.0]);
    equal_constraints.push(SketchConstraint {
        id: SketchConstraintId(3),
        kind: SketchConstraintKind::Equal {
            a: SketchEntityId(1),
            b: SketchEntityId(2),
        },
    });
    let equal = solve(
        vec![
            line(1, [0.0, 0.0], [10.0, 0.0]),
            line(2, [0.0, 5.0], [4.0, 7.0]),
        ],
        equal_constraints,
    );
    let second = solved_line(&equal, 2);
    assert!(((second.1[0] - second.0[0]).hypot(second.1[1] - second.0[1]) - 10.0).abs() <= 1.0e-7);

    let tangent = solve(
        vec![
            line(1, [-10.0, 0.0], [10.0, 0.0]),
            circle(2, [0.0, 5.0], 3.0),
        ],
        vec![
            fixed_line(1, 1, [-10.0, 0.0], [10.0, 0.0]),
            vec![
                SketchConstraint {
                    id: SketchConstraintId(3),
                    kind: SketchConstraintKind::FixedPoint {
                        point: point(2, SketchPointKind::Center),
                        position_mm: [0.0, 5.0],
                    },
                },
                SketchConstraint {
                    id: SketchConstraintId(4),
                    kind: SketchConstraintKind::Tangent {
                        a: SketchEntityId(1),
                        b: SketchEntityId(2),
                    },
                },
            ],
        ]
        .concat(),
    );
    let SketchEntity::Circle { radius_mm, .. } = &tangent[1] else {
        panic!("expected circle");
    };
    assert!((*radius_mm - 5.0).abs() <= 1.0e-7);

    let concentric = solve(
        vec![circle(1, [0.0, 0.0], 5.0), circle(2, [3.0, 4.0], 2.0)],
        vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Center),
                    position_mm: [0.0, 0.0],
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Concentric {
                    a: SketchEntityId(1),
                    b: SketchEntityId(2),
                },
            },
        ],
    );
    let SketchEntity::Circle { center_mm, .. } = &concentric[1] else {
        panic!("expected circle");
    };
    assert!(center_mm[0].abs() <= 1.0e-7 && center_mm[1].abs() <= 1.0e-7);

    let symmetric = solve(
        vec![
            line(1, [-10.0, 0.0], [10.0, 0.0]),
            line(2, [-4.0, 2.0], [-5.0, 5.0]),
            line(3, [3.0, -1.0], [5.0, -5.0]),
        ],
        vec![
            fixed_line(1, 1, [-10.0, 0.0], [10.0, 0.0]),
            vec![SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::Symmetric {
                    a: point(2, SketchPointKind::Start),
                    b: point(3, SketchPointKind::Start),
                    axis: SketchEntityId(1),
                },
            }],
        ]
        .concat(),
    );
    let a = solved_line(&symmetric, 2).0;
    let b = solved_line(&symmetric, 3).0;
    assert!((a[0] - b[0]).abs() <= 1.0e-7);
    assert!((a[1] + b[1]).abs() <= 1.0e-7);

    let midpoint = solve(
        vec![
            line(1, [0.0, 0.0], [10.0, 0.0]),
            line(2, [2.0, 3.0], [2.0, 8.0]),
        ],
        vec![
            fixed_line(1, 1, [0.0, 0.0], [10.0, 0.0]),
            vec![SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::Midpoint {
                    point: point(2, SketchPointKind::Start),
                    line: SketchEntityId(1),
                },
            }],
        ]
        .concat(),
    );
    let midpoint_value = solved_line(&midpoint, 2).0;
    assert!((midpoint_value[0] - 5.0).abs() <= 1.0e-7 && midpoint_value[1].abs() <= 1.0e-7);

    let on_curve = solve(
        vec![
            line(1, [0.0, 0.0], [10.0, 0.0]),
            line(2, [2.0, 3.0], [2.0, 8.0]),
        ],
        vec![
            fixed_line(1, 1, [0.0, 0.0], [10.0, 0.0]),
            vec![SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::PointOnCurve {
                    point: point(2, SketchPointKind::Start),
                    curve: SketchEntityId(1),
                },
            }],
        ]
        .concat(),
    );
    assert!(solved_line(&on_curve, 2).0[1].abs() <= 1.0e-7);
}

#[test]
fn arc_constraints_reject_the_excluded_supporting_circle_segment() {
    let arc = SketchEntity::Arc {
        id: SketchEntityId(1),
        start_mm: [5.0, 0.0],
        end_mm: [-5.0, 0.0],
        center_mm: [0.0, 0.0],
        clockwise: false,
    };
    let bounded = SketchSolverPolicy {
        max_iterations: 1,
        initial_damping: 1.0e20,
        ..SketchSolverPolicy::default()
    };
    let point_on_excluded_half = SketchSpec {
        workplane: XY,
        entities: vec![
            arc.clone(),
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [0.0, -5.0],
                end_mm: [0.0, -8.0],
            },
        ],
        constraints: vec![SketchConstraint {
            id: SketchConstraintId(1),
            kind: SketchConstraintKind::PointOnCurve {
                point: point(2, SketchPointKind::Start),
                curve: SketchEntityId(1),
            },
        }],
    };
    assert_eq!(
        point_on_excluded_half.solve_geometry_with_policy(bounded),
        Err(SketchError::NonConvergent)
    );

    let tangent_on_excluded_half = SketchSpec {
        workplane: XY,
        entities: vec![
            arc,
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [-10.0, -5.0],
                end_mm: [10.0, -5.0],
            },
        ],
        constraints: vec![SketchConstraint {
            id: SketchConstraintId(1),
            kind: SketchConstraintKind::Tangent {
                a: SketchEntityId(1),
                b: SketchEntityId(2),
            },
        }],
    };
    assert_eq!(
        tangent_on_excluded_half.solve_geometry_with_policy(bounded),
        Err(SketchError::NonConvergent)
    );
}

#[test]
fn algebraically_overlapping_relations_are_rejected_as_redundant() {
    let entities = vec![
        SketchEntity::Line {
            id: SketchEntityId(1),
            start_mm: [0.0, 0.0],
            end_mm: [10.0, 0.0],
        },
        SketchEntity::Line {
            id: SketchEntityId(2),
            start_mm: [0.0, 2.0],
            end_mm: [5.0, 2.0],
        },
    ];
    for constraints in [
        vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Parallel {
                    a: SketchEntityId(1),
                    b: SketchEntityId(2),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Collinear {
                    a: SketchEntityId(2),
                    b: SketchEntityId(1),
                },
            },
        ],
        vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Perpendicular {
                    a: SketchEntityId(1),
                    b: SketchEntityId(2),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Angle {
                    a: SketchEntityId(2),
                    b: SketchEntityId(1),
                    angle_degrees: 90.0,
                },
            },
        ],
    ] {
        let sketch = SketchSpec {
            workplane: XY,
            entities: entities.clone(),
            constraints,
        };
        assert_eq!(
            sketch.solve(),
            Err(SketchError::OverConstrained(SketchConstraintId(2)))
        );
    }
}

#[test]
fn coincident_equal_circles_are_not_misclassified_as_tangent() {
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![
            SketchEntity::Circle {
                id: SketchEntityId(1),
                center_mm: [0.0, 0.0],
                radius_mm: 5.0,
            },
            SketchEntity::Circle {
                id: SketchEntityId(2),
                center_mm: [0.0, 0.0],
                radius_mm: 5.0,
            },
        ],
        constraints: vec![SketchConstraint {
            id: SketchConstraintId(1),
            kind: SketchConstraintKind::Tangent {
                a: SketchEntityId(1),
                b: SketchEntityId(2),
            },
        }],
    };

    assert_eq!(sketch.solve(), Err(SketchError::NonConvergent));
}

#[test]
fn full_general_constraint_vocabulary_is_lossless_across_save_open() {
    let line = |id, start_mm, end_mm| SketchEntity::Line {
        id: SketchEntityId(id),
        start_mm,
        end_mm,
    };
    let circle = |id, center_mm, radius_mm| SketchEntity::Circle {
        id: SketchEntityId(id),
        center_mm,
        radius_mm,
    };
    let sketch = SketchSpec {
        workplane: XY,
        entities: vec![
            line(1, [0.0, 0.0], [10.0, 0.0]),
            line(2, [0.0, 2.0], [5.0, 2.0]),
            line(3, [20.0, 0.0], [30.0, 0.0]),
            line(4, [25.0, -5.0], [25.0, 5.0]),
            line(5, [40.0, 0.0], [60.0, 0.0]),
            circle(6, [50.0, 5.0], 5.0),
            line(7, [70.0, 0.0], [80.0, 0.0]),
            line(8, [70.0, 0.0], [75.0, 5.0 * 3.0_f64.sqrt()]),
            line(9, [90.0, 0.0], [95.0, 0.0]),
            line(10, [90.0, 2.0], [93.0, 6.0]),
            line(11, [100.0, 0.0], [120.0, 0.0]),
            line(12, [108.0, 3.0], [108.0, 5.0]),
            line(13, [108.0, -3.0], [108.0, -5.0]),
            circle(14, [130.0, 0.0], 5.0),
            circle(15, [130.0, 0.0], 2.0),
            line(16, [140.0, 0.0], [150.0, 0.0]),
            line(17, [155.0, 0.0], [160.0, 0.0]),
            line(18, [170.0, 0.0], [180.0, 0.0]),
            line(19, [175.0, 0.0], [175.0, 5.0]),
            circle(20, [190.0, 0.0], 5.0),
            line(21, [195.0, 0.0], [200.0, 4.0]),
        ],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Parallel {
                    a: SketchEntityId(1),
                    b: SketchEntityId(2),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Perpendicular {
                    a: SketchEntityId(3),
                    b: SketchEntityId(4),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::Tangent {
                    a: SketchEntityId(5),
                    b: SketchEntityId(6),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(4),
                kind: SketchConstraintKind::Angle {
                    a: SketchEntityId(7),
                    b: SketchEntityId(8),
                    angle_degrees: 60.0,
                },
            },
            SketchConstraint {
                id: SketchConstraintId(5),
                kind: SketchConstraintKind::Equal {
                    a: SketchEntityId(9),
                    b: SketchEntityId(10),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(6),
                kind: SketchConstraintKind::Symmetric {
                    a: point(12, SketchPointKind::Start),
                    b: point(13, SketchPointKind::Start),
                    axis: SketchEntityId(11),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(7),
                kind: SketchConstraintKind::Concentric {
                    a: SketchEntityId(14),
                    b: SketchEntityId(15),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(8),
                kind: SketchConstraintKind::Collinear {
                    a: SketchEntityId(16),
                    b: SketchEntityId(17),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(9),
                kind: SketchConstraintKind::Midpoint {
                    point: point(19, SketchPointKind::Start),
                    line: SketchEntityId(18),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(10),
                kind: SketchConstraintKind::PointOnCurve {
                    point: point(21, SketchPointKind::Start),
                    curve: SketchEntityId(20),
                },
            },
        ],
    };
    let expected_constraints = sketch.constraints.clone();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "General constraints".into(),
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
                name: "Full constraint vocabulary".into(),
                kind: FeatureKind::Sketch(sketch),
            },
        ]))
        .unwrap();

    let committed = document.current();
    let digest = committed.canonical_digest();
    let bytes = persistence::save(&committed);
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    let FeatureKind::Sketch(reopened_sketch) = reopened.feature(SKETCH).unwrap().kind() else {
        panic!("expected reopened sketch");
    };
    assert_eq!(reopened_sketch.constraints, expected_constraints);
    assert_eq!(reopened.canonical_digest(), digest);
    assert_eq!(persistence::save(&reopened), bytes);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetSketchConstraintDimension {
                id: SKETCH,
                constraint_id: SketchConstraintId(4),
                dimension: Dimension::from_decimal("45").unwrap(),
            },
        ]))
        .unwrap();
    let edited = document.current();
    let FeatureKind::Sketch(edited_sketch) = edited.feature(SKETCH).unwrap().kind() else {
        panic!("expected edited sketch");
    };
    assert!(matches!(
        edited_sketch.constraints[3].kind,
        SketchConstraintKind::Angle {
            angle_degrees: 45.0,
            ..
        }
    ));
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
