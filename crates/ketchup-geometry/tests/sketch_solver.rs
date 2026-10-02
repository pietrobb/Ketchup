//! The sketch solver on its own: degrees of freedom, convergence and
//! over-constrained sketches, without a document around it.

use ketchup_geometry::dimension::Dimension;
use ketchup_geometry::id::FeatureId;
use ketchup_geometry::sketch::{
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchDiagnosticStatus,
    SketchEntity, SketchEntityId, SketchError, SketchPointKind, SketchPointRef, SketchSolveStatus,
    SketchSolverPolicy, SketchSpec,
};
use ketchup_geometry::tolerance::APPROXIMATION;

fn line(id: u64, start_mm: [f64; 2], end_mm: [f64; 2]) -> SketchEntity {
    SketchEntity::Line {
        id: SketchEntityId(id),
        start_mm,
        end_mm,
    }
}

fn point(entity: u64, point: SketchPointKind) -> SketchPointRef {
    SketchPointRef {
        entity: SketchEntityId(entity),
        point,
    }
}

fn start(entity: u64) -> SketchPointRef {
    point(entity, SketchPointKind::Start)
}

fn end(entity: u64) -> SketchPointRef {
    point(entity, SketchPointKind::End)
}

fn distance(a: SketchPointRef, b: SketchPointRef, millimetres: f64) -> SketchConstraintKind {
    SketchConstraintKind::Distance {
        a,
        b,
        value: Dimension::new(millimetres.to_string(), millimetres).unwrap(),
    }
}

/// A sketch whose constraints get ids 1, 2, ... in the given order.
fn sketch(entities: Vec<SketchEntity>, constraints: Vec<SketchConstraintKind>) -> SketchSpec {
    SketchSpec {
        workplane: FeatureId(1),
        entities,
        constraints: constraints
            .into_iter()
            .zip(1..)
            .map(|(kind, id)| SketchConstraint {
                id: SketchConstraintId(id),
                kind,
            })
            .collect(),
    }
}

/// A 100 x 50 rectangle anchored at `origin`, drawn roughly: every corner is
/// off by up to 2 mm, so the solver has to move all of it.
fn rough_rectangle(origin: [f64; 2]) -> SketchSpec {
    let at = |x: f64, y: f64| [origin[0] + x, origin[1] + y];
    sketch(
        vec![
            line(1, at(0.0, 0.0), at(98.0, 1.0)),
            line(2, at(99.0, 2.0), at(101.0, 49.0)),
            line(3, at(100.0, 51.0), at(2.0, 50.0)),
            line(4, at(-1.0, 49.0), at(1.0, 1.0)),
        ],
        vec![
            SketchConstraintKind::Coincident {
                a: end(1),
                b: start(2),
            },
            SketchConstraintKind::Coincident {
                a: end(2),
                b: start(3),
            },
            SketchConstraintKind::Coincident {
                a: end(3),
                b: start(4),
            },
            SketchConstraintKind::Coincident {
                a: end(4),
                b: start(1),
            },
            SketchConstraintKind::Horizontal {
                entity: SketchEntityId(1),
            },
            SketchConstraintKind::Vertical {
                entity: SketchEntityId(2),
            },
            SketchConstraintKind::Horizontal {
                entity: SketchEntityId(3),
            },
            SketchConstraintKind::Vertical {
                entity: SketchEntityId(4),
            },
            SketchConstraintKind::FixedPoint {
                point: start(1),
                position_mm: origin,
            },
            distance(start(1), end(1), 100.0),
            distance(start(2), end(2), 50.0),
        ],
    )
}

fn assert_line(entity: &SketchEntity, expected_start: [f64; 2], expected_end: [f64; 2]) {
    let SketchEntity::Line {
        start_mm, end_mm, ..
    } = entity
    else {
        panic!("expected a line, got {entity:?}");
    };
    for (actual, expected) in [(start_mm, expected_start), (end_mm, expected_end)] {
        assert!(
            (actual[0] - expected[0]).abs() <= APPROXIMATION
                && (actual[1] - expected[1]).abs() <= APPROXIMATION,
            "{entity:?} should run {expected_start:?} -> {expected_end:?}"
        );
    }
}

#[test]
fn every_free_entity_keeps_its_own_degrees_of_freedom() {
    let entities = [
        (line(1, [0.0, 0.0], [10.0, 0.0]), 4),
        (
            SketchEntity::Circle {
                id: SketchEntityId(1),
                center_mm: [0.0, 0.0],
                radius_mm: 5.0,
            },
            3,
        ),
        (
            SketchEntity::Arc {
                id: SketchEntityId(1),
                start_mm: [5.0, 0.0],
                end_mm: [0.0, 5.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
            5,
        ),
        (
            SketchEntity::CubicBezier {
                id: SketchEntityId(1),
                start_mm: [0.0, 0.0],
                control_1_mm: [3.0, 4.0],
                control_2_mm: [7.0, 4.0],
                end_mm: [10.0, 0.0],
            },
            8,
        ),
    ];
    for (entity, dof) in entities {
        let report = sketch(vec![entity.clone()], Vec::new()).solve().unwrap();
        assert_eq!(
            report.status,
            SketchSolveStatus::UnderConstrained { remaining_dof: dof },
            "{entity:?}"
        );
        assert_eq!(report.unconstrained_entity_ids, vec![SketchEntityId(1)]);
        assert_eq!(report.equation_count, 0);
    }
}

#[test]
fn each_constraint_removes_the_freedom_it_fixes() {
    let free = || line(1, [0.0, 0.0], [10.0, 1.0]);
    let steps = [
        (vec![], 4),
        (
            vec![SketchConstraintKind::Horizontal {
                entity: SketchEntityId(1),
            }],
            3,
        ),
        (
            vec![
                SketchConstraintKind::Horizontal {
                    entity: SketchEntityId(1),
                },
                SketchConstraintKind::FixedPoint {
                    point: start(1),
                    position_mm: [0.0, 0.0],
                },
            ],
            1,
        ),
    ];
    for (constraints, dof) in steps {
        let report = sketch(vec![free()], constraints).solve().unwrap();
        assert_eq!(
            report.status,
            SketchSolveStatus::UnderConstrained { remaining_dof: dof }
        );
    }
    let fixed = sketch(
        vec![free()],
        vec![
            SketchConstraintKind::Horizontal {
                entity: SketchEntityId(1),
            },
            SketchConstraintKind::FixedPoint {
                point: start(1),
                position_mm: [0.0, 0.0],
            },
            distance(start(1), end(1), 25.0),
        ],
    );
    let solution = fixed.solve_geometry().unwrap();
    assert_eq!(solution.report.status, SketchSolveStatus::FullyConstrained);
    assert!(solution.report.unconstrained_entity_ids.is_empty());
    assert_line(&solution.entities[0], [0.0, 0.0], [25.0, 0.0]);
}

#[test]
fn only_the_entity_left_free_is_reported_unconstrained() {
    let report = sketch(
        vec![
            line(1, [0.0, 0.0], [10.0, 0.0]),
            line(2, [20.0, 5.0], [30.0, 8.0]),
        ],
        vec![
            SketchConstraintKind::FixedPoint {
                point: start(1),
                position_mm: [0.0, 0.0],
            },
            SketchConstraintKind::FixedPoint {
                point: end(1),
                position_mm: [10.0, 0.0],
            },
        ],
    )
    .solve()
    .unwrap();
    assert_eq!(
        report.status,
        SketchSolveStatus::UnderConstrained { remaining_dof: 4 }
    );
    assert_eq!(report.unconstrained_entity_ids, vec![SketchEntityId(2)]);
}

#[test]
fn a_roughly_drawn_rectangle_converges_to_its_dimensions() {
    let solution = rough_rectangle([0.0, 0.0]).solve_geometry().unwrap();
    assert_eq!(solution.report.status, SketchSolveStatus::FullyConstrained);
    assert_eq!(solution.report.equation_count, 16);
    assert_line(&solution.entities[0], [0.0, 0.0], [100.0, 0.0]);
    assert_line(&solution.entities[1], [100.0, 0.0], [100.0, 50.0]);
    assert_line(&solution.entities[2], [100.0, 50.0], [0.0, 50.0]);
    assert_line(&solution.entities[3], [0.0, 50.0], [0.0, 0.0]);
}

#[test]
fn moving_the_whole_sketch_moves_the_solution_and_nothing_else() {
    let origin = [5_000.0, -3_000.0];
    let solution = rough_rectangle(origin).solve_geometry().unwrap();
    assert_eq!(solution.report.status, SketchSolveStatus::FullyConstrained);
    let at = |x: f64, y: f64| [origin[0] + x, origin[1] + y];
    assert_line(&solution.entities[0], at(0.0, 0.0), at(100.0, 0.0));
    assert_line(&solution.entities[1], at(100.0, 0.0), at(100.0, 50.0));
    assert_line(&solution.entities[2], at(100.0, 50.0), at(0.0, 50.0));
    assert_line(&solution.entities[3], at(0.0, 50.0), at(0.0, 0.0));
}

#[test]
fn the_solver_stops_within_its_iteration_budget() {
    let policy = SketchSolverPolicy {
        max_iterations: 1,
        ..SketchSolverPolicy::default()
    };
    assert_eq!(
        rough_rectangle([0.0, 0.0]).solve_with_policy(policy),
        Err(SketchError::NonConvergent)
    );
    let no_budget = SketchSolverPolicy {
        max_iterations: 0,
        ..SketchSolverPolicy::default()
    };
    assert_eq!(
        rough_rectangle([0.0, 0.0]).solve_with_policy(no_budget),
        Err(SketchError::InvalidSolverPolicy)
    );
}

#[test]
fn a_redundant_constraint_is_named_as_the_conflict() {
    // The rectangle is fully constrained by its 11 constraints; a 12th that
    // fixes an already fixed corner adds equations without freedom to remove.
    let mut over = rough_rectangle([0.0, 0.0]);
    over.constraints.push(SketchConstraint {
        id: SketchConstraintId(12),
        kind: SketchConstraintKind::FixedPoint {
            point: end(3),
            position_mm: [0.0, 50.0],
        },
    });
    assert!(
        matches!(over.solve(), Err(SketchError::OverConstrained(_))),
        "{:?}",
        over.solve()
    );
    let diagnosis = over.diagnose().unwrap();
    assert_eq!(diagnosis.status, SketchDiagnosticStatus::Conflicting);
    assert_eq!(diagnosis.constraint_ids.len(), 1);
}

#[test]
fn closing_a_cycle_of_coincident_points_is_over_constrained() {
    let over = sketch(
        vec![
            line(1, [0.0, 0.0], [10.0, 0.0]),
            line(2, [10.0, 0.0], [10.0, 10.0]),
            line(3, [10.0, 10.0], [10.0, 0.0]),
        ],
        vec![
            SketchConstraintKind::Coincident {
                a: end(1),
                b: start(2),
            },
            SketchConstraintKind::Coincident {
                a: start(2),
                b: end(3),
            },
            SketchConstraintKind::Coincident {
                a: end(3),
                b: end(1),
            },
        ],
    );
    assert_eq!(
        over.solve(),
        Err(SketchError::OverConstrained(SketchConstraintId(3)))
    );
    let diagnosis = over.diagnose().unwrap();
    assert_eq!(diagnosis.status, SketchDiagnosticStatus::Conflicting);
    assert_eq!(diagnosis.constraint_ids, vec![SketchConstraintId(3)]);
    assert_eq!(
        diagnosis.entity_ids,
        vec![SketchEntityId(1), SketchEntityId(3)]
    );
}

#[test]
fn two_different_lengths_for_one_distance_conflict() {
    let over = sketch(
        vec![line(1, [0.0, 0.0], [10.0, 0.0])],
        vec![
            distance(start(1), end(1), 10.0),
            distance(start(1), end(1), 12.0),
        ],
    );
    assert!(
        matches!(over.solve(), Err(SketchError::OverConstrained(_))),
        "{:?}",
        over.solve()
    );
}

#[test]
fn entities_and_constraints_must_be_in_id_order() {
    let unordered = sketch(
        vec![
            line(2, [0.0, 0.0], [10.0, 0.0]),
            line(1, [0.0, 5.0], [10.0, 5.0]),
        ],
        Vec::new(),
    );
    assert_eq!(unordered.solve(), Err(SketchError::EntitiesNotCanonical));
    let mut constraints = sketch(
        vec![line(1, [0.0, 0.0], [10.0, 1.0])],
        vec![
            SketchConstraintKind::Horizontal {
                entity: SketchEntityId(1),
            },
            SketchConstraintKind::FixedPoint {
                point: start(1),
                position_mm: [0.0, 0.0],
            },
        ],
    );
    constraints.constraints.reverse();
    assert_eq!(
        constraints.solve(),
        Err(SketchError::ConstraintsNotCanonical)
    );
}
