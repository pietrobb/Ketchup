use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId,
    SketchError, SketchPointKind, SketchPointRef, SketchSolveStatus, SketchSpec,
};

fn point(entity: u64, point: SketchPointKind) -> SketchPointRef {
    SketchPointRef {
        entity: SketchEntityId(entity),
        point,
    }
}

fn constraint(id: u64, kind: SketchConstraintKind) -> SketchConstraint {
    SketchConstraint {
        id: SketchConstraintId(id),
        kind,
    }
}

fn horizontal_pair(length: f64, scale: f64, offset: f64) -> SketchSpec {
    SketchSpec {
        workplane: FeatureId(1),
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [offset, offset],
                end_mm: [offset + 10.0 * scale, offset],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [offset, offset + 2.0 * scale],
                end_mm: [offset + length * scale, offset + 2.0 * scale],
            },
        ],
        constraints: vec![
            constraint(
                1,
                SketchConstraintKind::Horizontal {
                    entity: SketchEntityId(1),
                },
            ),
            constraint(
                2,
                SketchConstraintKind::Horizontal {
                    entity: SketchEntityId(2),
                },
            ),
            constraint(
                3,
                SketchConstraintKind::Parallel {
                    a: SketchEntityId(1),
                    b: SketchEntityId(2),
                },
            ),
            constraint(
                4,
                SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Start),
                    position_mm: [offset, offset],
                },
            ),
            constraint(
                5,
                SketchConstraintKind::FixedPoint {
                    point: point(2, SketchPointKind::Start),
                    position_mm: [offset, offset + 2.0 * scale],
                },
            ),
            constraint(
                6,
                SketchConstraintKind::Distance {
                    a: point(1, SketchPointKind::Start),
                    b: point(1, SketchPointKind::End),
                    value: Dimension::from_decimal((10.0 * scale).to_string()).unwrap(),
                },
            ),
        ],
    }
}

#[test]
fn algebraic_dependency_never_claims_fully_constrained() {
    for length in [10.0, 25.0] {
        for (scale, offset) in [(1.0, 0.0), (0.001, 0.0), (1000.0, 0.0), (1.0, 900_000.0)] {
            let sketch = horizontal_pair(length, scale, offset);
            assert!(
                matches!(sketch.solve(), Err(SketchError::OverConstrained(_))),
                "length={length}, scale={scale}, offset={offset}: {:?}",
                sketch.solve()
            );
            // Removing the redundant parallel relation leaves exactly B's length free.
            let mut repaired = sketch;
            repaired.constraints.remove(2);
            let solution = repaired.solve_geometry().unwrap();
            assert_eq!(
                solution.report.status,
                SketchSolveStatus::UnderConstrained { remaining_dof: 1 }
            );
            assert_eq!(
                solution.report.unconstrained_entity_ids,
                vec![SketchEntityId(2)]
            );
            assert_eq!(solution.entities, repaired.entities);
        }
    }
}

#[test]
fn nullspace_marks_all_entities_in_a_free_rigid_pair() {
    let mut sketch = horizontal_pair(10.0, 1.0, 0.0);
    sketch.constraints = vec![
        constraint(
            1,
            SketchConstraintKind::Coincident {
                a: point(1, SketchPointKind::Start),
                b: point(2, SketchPointKind::Start),
            },
        ),
        constraint(
            2,
            SketchConstraintKind::Coincident {
                a: point(1, SketchPointKind::End),
                b: point(2, SketchPointKind::End),
            },
        ),
        constraint(
            3,
            SketchConstraintKind::Distance {
                a: point(1, SketchPointKind::Start),
                b: point(1, SketchPointKind::End),
                value: Dimension::from_decimal("10").unwrap(),
            },
        ),
        constraint(
            4,
            SketchConstraintKind::Horizontal {
                entity: SketchEntityId(1),
            },
        ),
    ];
    let report = sketch.solve().unwrap();
    assert_eq!(
        report.status,
        SketchSolveStatus::UnderConstrained { remaining_dof: 2 }
    );
    assert_eq!(
        report.unconstrained_entity_ids,
        vec![SketchEntityId(1), SketchEntityId(2)]
    );
}

#[test]
fn adding_the_missing_dimension_really_fixes_both_lines() {
    for (scale, offset) in [(0.001, 900_000.0), (1.0, 0.0), (1000.0, 0.0)] {
        let mut sketch = horizontal_pair(25.0, scale, offset);
        sketch.constraints.remove(2);
        sketch.constraints.push(constraint(
            7,
            SketchConstraintKind::Distance {
                a: point(2, SketchPointKind::Start),
                b: point(2, SketchPointKind::End),
                value: Dimension::from_decimal((25.0 * scale).to_string()).unwrap(),
            },
        ));
        let solution = sketch.solve_geometry().unwrap();
        assert_eq!(solution.report.status, SketchSolveStatus::FullyConstrained);
        assert!(solution.report.unconstrained_entity_ids.is_empty());
        assert_eq!(solution.entities, sketch.entities);
    }
}

#[test]
fn parallel_dependency_is_detected_in_every_constraint_order() {
    let sketch = horizontal_pair(25.0, 1.0, 0.0);
    for rotation in 0..sketch.constraints.len() {
        let mut reordered = sketch.clone();
        reordered.constraints.rotate_left(rotation);
        for (index, constraint) in reordered.constraints.iter_mut().enumerate() {
            constraint.id = SketchConstraintId(index as u64 + 1);
        }
        assert!(matches!(
            reordered.solve(),
            Err(SketchError::OverConstrained(_))
        ));
    }
}

#[test]
fn rank_uses_the_solved_orientation_and_regular_parallel_angle_limits() {
    for angle_degrees in [0.0_f64, 45.0, 90.0, 180.0] {
        let mut sketch = horizontal_pair(10.0, 1.0, 0.0);
        if angle_degrees == 180.0 {
            sketch.entities[1] = SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [0.0, 2.0],
                end_mm: [-10.0, 2.0],
            };
        } else if angle_degrees != 0.0 {
            sketch.entities[1] = SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [0.0, 2.0],
                end_mm: [5.0, 7.0],
            };
        }
        sketch.constraints.retain(|c| !matches!(c.id.0, 2 | 3));
        sketch.constraints.push(constraint(
            7,
            SketchConstraintKind::Distance {
                a: point(2, SketchPointKind::Start),
                b: point(2, SketchPointKind::End),
                value: Dimension::from_decimal("10").unwrap(),
            },
        ));
        let relation = if angle_degrees == 0.0 || angle_degrees == 180.0 {
            SketchConstraintKind::Parallel {
                a: SketchEntityId(1),
                b: SketchEntityId(2),
            }
        } else {
            SketchConstraintKind::Angle {
                a: SketchEntityId(1),
                b: SketchEntityId(2),
                angle_degrees,
            }
        };
        sketch.constraints.push(constraint(8, relation));
        let solution = sketch.solve_geometry().unwrap();
        assert_eq!(
            solution.report.status,
            SketchSolveStatus::FullyConstrained,
            "angle={angle_degrees}"
        );
        assert!(solution.report.unconstrained_entity_ids.is_empty());
        let SketchEntity::Line {
            start_mm, end_mm, ..
        } = solution.entities[1]
        else {
            panic!("line");
        };
        let length = (end_mm[0] - start_mm[0]).hypot(end_mm[1] - start_mm[1]);
        assert!((length - 10.0).abs() < 1.0e-7);
        assert!(
            ((end_mm[0] - start_mm[0]) / length - angle_degrees.to_radians().cos()).abs() < 1.0e-7
        );
    }
}

#[test]
fn degenerate_distance_dependency_cannot_prove_full_rank() {
    let sketch = SketchSpec {
        workplane: FeatureId(1),
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [0.0, 0.0],
                end_mm: [10.0, 0.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [5.0, 0.0],
                end_mm: [5.0, 10.0],
            },
        ],
        constraints: vec![
            constraint(
                1,
                SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Start),
                    position_mm: [0.0, 0.0],
                },
            ),
            constraint(
                2,
                SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::End),
                    position_mm: [10.0, 0.0],
                },
            ),
            constraint(
                3,
                SketchConstraintKind::Distance {
                    a: point(1, SketchPointKind::Start),
                    b: point(2, SketchPointKind::Start),
                    value: Dimension::from_decimal("5").unwrap(),
                },
            ),
            constraint(
                4,
                SketchConstraintKind::Distance {
                    a: point(1, SketchPointKind::End),
                    b: point(2, SketchPointKind::Start),
                    value: Dimension::from_decimal("5").unwrap(),
                },
            ),
        ],
    };
    assert!(matches!(
        sketch.solve(),
        Err(SketchError::OverConstrained(_))
    ));
}

#[test]
fn redundant_sketch_fails_atomically_and_repaired_sketch_roundtrips_with_undo() {
    let mut store = DocumentStore::new();
    let before = store.current().canonical_digest();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Part".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(1),
            definition_id: DefinitionId(1),
            name: "XY".into(),
            kind: FeatureKind::Workplane(ketchup_core::sketch::WorkplaneSpec::principal(
                ketchup_core::sketch::PrincipalPlane::Xy,
            )),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(2),
            definition_id: DefinitionId(1),
            name: "Sketch".into(),
            kind: FeatureKind::Sketch(horizontal_pair(25.0, 1.0, 0.0)),
        },
    ];
    assert!(
        store
            .apply_batch(&CommandBatch::new(commands.clone()))
            .is_err()
    );
    assert_eq!(store.current().canonical_digest(), before);
    assert_eq!(store.visible_undo_steps(), 0);
    let CanonicalCommand::CreateFeature {
        kind: FeatureKind::Sketch(ref mut spec),
        ..
    } = commands[2]
    else {
        panic!("sketch");
    };
    spec.constraints.remove(2);
    let expected = spec.solve().unwrap();
    store.apply_batch(&CommandBatch::new(commands)).unwrap();
    let after = store.current().canonical_digest();
    let bytes = persistence::save(&store.current());
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), after);
    let FeatureKind::Sketch(spec) = reopened.feature(FeatureId(2)).unwrap().kind() else {
        panic!("sketch");
    };
    assert_eq!(spec.solve().unwrap(), expected);
    assert_eq!(store.undo().unwrap().canonical_digest(), before);
    assert_eq!(store.redo().unwrap().canonical_digest(), after);
}
