use super::*;

pub(super) fn constraint_entity_ids(kind: &SketchConstraintKind) -> Vec<SketchEntityId> {
    let mut ids = match kind {
        SketchConstraintKind::Horizontal { entity }
        | SketchConstraintKind::Vertical { entity }
        | SketchConstraintKind::Radius { entity, .. }
        | SketchConstraintKind::Projection { entity, .. }
        | SketchConstraintKind::Construction { entity } => vec![*entity],
        SketchConstraintKind::Coincident { a, b } | SketchConstraintKind::Distance { a, b, .. } => {
            vec![a.entity, b.entity]
        }
        SketchConstraintKind::FixedPoint { point, .. } => vec![point.entity],
        SketchConstraintKind::Parallel { a, b }
        | SketchConstraintKind::Perpendicular { a, b }
        | SketchConstraintKind::Tangent { a, b }
        | SketchConstraintKind::Angle { a, b, .. }
        | SketchConstraintKind::Equal { a, b }
        | SketchConstraintKind::Concentric { a, b }
        | SketchConstraintKind::Collinear { a, b } => vec![*a, *b],
        SketchConstraintKind::Symmetric { a, b, axis } => vec![a.entity, b.entity, *axis],
        SketchConstraintKind::Midpoint { point, line } => vec![point.entity, *line],
        SketchConstraintKind::PointOnCurve { point, curve } => vec![point.entity, *curve],
    };
    ids.sort_unstable();
    ids.dedup();
    ids
}

pub(super) fn evaluate_constraint(
    constraint: &SketchConstraint,
    entities: &BTreeMap<SketchEntityId, &SketchEntity>,
) -> Result<(Vec<u8>, usize), SketchError> {
    let point = |reference: SketchPointRef| {
        entities
            .get(&reference.entity)
            .and_then(|entity| entity.point(reference.point))
            .ok_or(SketchError::InvalidConstraintReference(constraint.id))
    };
    let mut signature = vec![0];
    let equations = match &constraint.kind {
        SketchConstraintKind::Horizontal { entity } | SketchConstraintKind::Vertical { entity } => {
            let tag = if matches!(constraint.kind, SketchConstraintKind::Horizontal { .. }) {
                1
            } else {
                2
            };
            signature[0] = tag;
            signature.extend_from_slice(&entity.0.to_le_bytes());
            if !matches!(
                entities.get(entity).copied(),
                Some(SketchEntity::Line { .. })
            ) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            1
        }
        SketchConstraintKind::Coincident { a, b } => {
            point(*a)?;
            point(*b)?;
            if a == b {
                return Err(SketchError::OverConstrained(constraint.id));
            }
            let (first, second) = canonical_point_pair(*a, *b);
            signature[0] = 3;
            push_point_ref(&mut signature, first);
            push_point_ref(&mut signature, second);
            2
        }
        SketchConstraintKind::Distance { a, b, value } => {
            point(*a)?;
            point(*b)?;
            if a == b {
                return Err(SketchError::OverConstrained(constraint.id));
            }
            let expected = valid_positive_dimension(value)?;
            if let Some(entity) = arc_radius_entity(*a, *b).filter(|entity| {
                matches!(
                    entities.get(entity).copied(),
                    Some(SketchEntity::Arc { .. })
                )
            }) {
                signature[0] = 5;
                signature.extend_from_slice(&entity.0.to_le_bytes());
            } else {
                let (first, second) = canonical_point_pair(*a, *b);
                signature[0] = 4;
                push_point_ref(&mut signature, first);
                push_point_ref(&mut signature, second);
            }
            signature.extend_from_slice(&expected.to_bits().to_le_bytes());
            1
        }
        SketchConstraintKind::Radius { entity, value } => {
            let expected = valid_positive_dimension(value)?;
            signature[0] = 5;
            signature.extend_from_slice(&entity.0.to_le_bytes());
            signature.extend_from_slice(&expected.to_bits().to_le_bytes());
            if !matches!(
                entities.get(entity).copied(),
                Some(SketchEntity::Arc { .. } | SketchEntity::Circle { .. })
            ) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            1
        }
        SketchConstraintKind::FixedPoint {
            point: reference,
            position_mm,
        } => {
            point(*reference)?;
            if position_mm
                .iter()
                .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
            {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 6;
            push_point_ref(&mut signature, *reference);
            signature.extend_from_slice(&position_mm[0].to_bits().to_le_bytes());
            signature.extend_from_slice(&position_mm[1].to_bits().to_le_bytes());
            2
        }
        SketchConstraintKind::Parallel { a, b }
        | SketchConstraintKind::Perpendicular { a, b }
        | SketchConstraintKind::Collinear { a, b } => {
            if a == b
                || !matches!(entities.get(a).copied(), Some(SketchEntity::Line { .. }))
                || !matches!(entities.get(b).copied(), Some(SketchEntity::Line { .. }))
            {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = match constraint.kind {
                SketchConstraintKind::Parallel { .. } => 7,
                SketchConstraintKind::Perpendicular { .. } => 8,
                SketchConstraintKind::Collinear { .. } => 14,
                _ => unreachable!(),
            };
            let (first, second) = canonical_entity_pair(*a, *b);
            signature.extend_from_slice(&first.0.to_le_bytes());
            signature.extend_from_slice(&second.0.to_le_bytes());
            if matches!(constraint.kind, SketchConstraintKind::Collinear { .. }) {
                2
            } else {
                1
            }
        }
        SketchConstraintKind::Tangent { a, b } => {
            if a == b || !supports_tangent(entities.get(a).copied(), entities.get(b).copied()) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 9;
            let (first, second) = canonical_entity_pair(*a, *b);
            signature.extend_from_slice(&first.0.to_le_bytes());
            signature.extend_from_slice(&second.0.to_le_bytes());
            1
        }
        SketchConstraintKind::Angle {
            a,
            b,
            angle_degrees,
        } => {
            if a == b
                || !matches!(entities.get(a).copied(), Some(SketchEntity::Line { .. }))
                || !matches!(entities.get(b).copied(), Some(SketchEntity::Line { .. }))
                || !valid_angle_degrees(*angle_degrees)
            {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 10;
            let (first, second) = canonical_entity_pair(*a, *b);
            signature.extend_from_slice(&first.0.to_le_bytes());
            signature.extend_from_slice(&second.0.to_le_bytes());
            signature.extend_from_slice(&angle_degrees.to_bits().to_le_bytes());
            1
        }
        SketchConstraintKind::Equal { a, b } => {
            if a == b || !supports_equal(entities.get(a).copied(), entities.get(b).copied()) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 11;
            let (first, second) = canonical_entity_pair(*a, *b);
            signature.extend_from_slice(&first.0.to_le_bytes());
            signature.extend_from_slice(&second.0.to_le_bytes());
            1
        }
        SketchConstraintKind::Symmetric { a, b, axis } => {
            point(*a)?;
            point(*b)?;
            if a == b || !matches!(entities.get(axis).copied(), Some(SketchEntity::Line { .. })) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 12;
            let (first, second) = canonical_point_pair(*a, *b);
            push_point_ref(&mut signature, first);
            push_point_ref(&mut signature, second);
            signature.extend_from_slice(&axis.0.to_le_bytes());
            2
        }
        SketchConstraintKind::Concentric { a, b } => {
            if a == b
                || !is_circular_entity(entities.get(a).copied())
                || !is_circular_entity(entities.get(b).copied())
            {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 13;
            let (first, second) = canonical_entity_pair(*a, *b);
            signature.extend_from_slice(&first.0.to_le_bytes());
            signature.extend_from_slice(&second.0.to_le_bytes());
            2
        }
        SketchConstraintKind::Midpoint {
            point: reference,
            line,
        } => {
            point(*reference)?;
            if reference.entity == *line
                || !matches!(entities.get(line).copied(), Some(SketchEntity::Line { .. }))
            {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 15;
            push_point_ref(&mut signature, *reference);
            signature.extend_from_slice(&line.0.to_le_bytes());
            2
        }
        SketchConstraintKind::PointOnCurve {
            point: reference,
            curve,
        } => {
            point(*reference)?;
            if reference.entity == *curve || !is_curve_entity(entities.get(curve).copied()) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 16;
            push_point_ref(&mut signature, *reference);
            signature.extend_from_slice(&curve.0.to_le_bytes());
            1
        }
        SketchConstraintKind::Projection {
            entity,
            source_feature,
            source_entity,
            target,
        } => {
            let Some(actual) = entities.get(entity).copied() else {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            };
            if source_feature.0 == 0
                || source_entity.0 == 0
                || target.id() != *entity
                || actual.id() != *entity
                || actual != target.as_ref()
            {
                return Err(SketchError::InvalidProjectionSource);
            }
            target.validate()?;
            signature[0] = 17;
            signature.extend_from_slice(&entity.0.to_le_bytes());
            signature.extend_from_slice(&source_feature.0.to_le_bytes());
            signature.extend_from_slice(&source_entity.0.to_le_bytes());
            actual.degrees_of_freedom()
        }
        SketchConstraintKind::Construction { entity } => {
            if !entities.contains_key(entity) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 18;
            signature.extend_from_slice(&entity.0.to_le_bytes());
            0
        }
    };
    Ok((signature, equations))
}

pub(super) fn dimensional_constraint_target(
    constraint: &SketchConstraint,
    coincidence_parents: &BTreeMap<SketchPointRef, SketchPointRef>,
    entities: &BTreeMap<SketchEntityId, &SketchEntity>,
) -> Result<(Vec<u8>, Vec<u8>), SketchError> {
    let mut target = Vec::new();
    let mut value = Vec::new();
    match &constraint.kind {
        SketchConstraintKind::Distance {
            a,
            b,
            value: dimension,
        } => {
            let a = coincidence_root(coincidence_parents, *a);
            let b = coincidence_root(coincidence_parents, *b);
            if a == b {
                return Err(SketchError::OverConstrained(constraint.id));
            }
            let radial_entity = entities.iter().find_map(|(entity_id, entity)| {
                if !matches!(entity, SketchEntity::Arc { .. }) {
                    return None;
                }
                [SketchPointKind::Start, SketchPointKind::End]
                    .into_iter()
                    .any(|endpoint| {
                        let center = coincidence_root(
                            coincidence_parents,
                            SketchPointRef {
                                entity: *entity_id,
                                point: SketchPointKind::Center,
                            },
                        );
                        let endpoint = coincidence_root(
                            coincidence_parents,
                            SketchPointRef {
                                entity: *entity_id,
                                point: endpoint,
                            },
                        );
                        canonical_point_pair(center, endpoint) == canonical_point_pair(a, b)
                    })
                    .then_some(*entity_id)
            });
            if let Some(entity) = radial_entity {
                target.push(1);
                target.extend_from_slice(&entity.0.to_le_bytes());
            } else {
                target.push(2);
                let (first, second) = canonical_point_pair(a, b);
                push_point_ref(&mut target, first);
                push_point_ref(&mut target, second);
            }
            value.extend_from_slice(&dimension.millimetres().to_bits().to_le_bytes());
        }
        SketchConstraintKind::Radius {
            entity,
            value: dimension,
        } => {
            target.push(1);
            target.extend_from_slice(&entity.0.to_le_bytes());
            value.extend_from_slice(&dimension.millimetres().to_bits().to_le_bytes());
        }
        SketchConstraintKind::Angle {
            a,
            b,
            angle_degrees,
        } => {
            target.push(4);
            let (first, second) = canonical_entity_pair(*a, *b);
            target.extend_from_slice(&first.0.to_le_bytes());
            target.extend_from_slice(&second.0.to_le_bytes());
            value.extend_from_slice(&angle_degrees.to_bits().to_le_bytes());
        }
        SketchConstraintKind::FixedPoint { point, position_mm } => {
            target.push(3);
            push_point_ref(&mut target, coincidence_root(coincidence_parents, *point));
            value.extend_from_slice(&position_mm[0].to_bits().to_le_bytes());
            value.extend_from_slice(&position_mm[1].to_bits().to_le_bytes());
        }
        SketchConstraintKind::Horizontal { .. }
        | SketchConstraintKind::Vertical { .. }
        | SketchConstraintKind::Coincident { .. }
        | SketchConstraintKind::Parallel { .. }
        | SketchConstraintKind::Perpendicular { .. }
        | SketchConstraintKind::Tangent { .. }
        | SketchConstraintKind::Equal { .. }
        | SketchConstraintKind::Symmetric { .. }
        | SketchConstraintKind::Concentric { .. }
        | SketchConstraintKind::Collinear { .. }
        | SketchConstraintKind::Midpoint { .. }
        | SketchConstraintKind::PointOnCurve { .. }
        | SketchConstraintKind::Projection { .. }
        | SketchConstraintKind::Construction { .. } => {
            unreachable!("only dimensional constraints are collected")
        }
    }
    Ok((target, value))
}

#[derive(Clone, Copy)]
pub(super) enum VariableLayout {
    Line { base: usize },
    Arc { base: usize },
    Circle { base: usize },
    CubicBezier { base: usize },
}

impl VariableLayout {
    pub(super) fn variable_range(self) -> std::ops::Range<usize> {
        match self {
            Self::Line { base } => base..base + 4,
            Self::Arc { base } => base..base + 5,
            Self::Circle { base } => base..base + 3,
            Self::CubicBezier { base } => base..base + 8,
        }
    }
}

pub(super) fn variable_layouts(
    entities: &[SketchEntity],
) -> Result<(BTreeMap<SketchEntityId, VariableLayout>, usize), SketchError> {
    let mut layouts = BTreeMap::new();
    let mut next = 0usize;
    for entity in entities {
        let layout = match entity {
            SketchEntity::Line { .. } => VariableLayout::Line { base: next },
            SketchEntity::Arc { .. } => VariableLayout::Arc { base: next },
            SketchEntity::Circle { .. } => VariableLayout::Circle { base: next },
            SketchEntity::CubicBezier { .. } => VariableLayout::CubicBezier { base: next },
        };
        next = next
            .checked_add(entity.degrees_of_freedom())
            .ok_or(SketchError::ResourceLimit)?;
        if next > MAX_SKETCH_SOLVER_DOF {
            return Err(SketchError::ResourceLimit);
        }
        layouts.insert(entity.id(), layout);
    }
    Ok((layouts, next))
}

pub(super) fn constraint_variable_equations(
    constraint: &SketchConstraint,
    layouts: &BTreeMap<SketchEntityId, VariableLayout>,
) -> Result<Vec<Vec<usize>>, SketchError> {
    let point_variables = |reference: SketchPointRef, axis: usize| {
        let layout = layouts
            .get(&reference.entity)
            .ok_or(SketchError::InvalidConstraintReference(constraint.id))?;
        let variables = match (*layout, reference.point, axis) {
            (VariableLayout::Line { base }, SketchPointKind::Start, 0) => vec![base],
            (VariableLayout::Line { base }, SketchPointKind::Start, 1) => vec![base + 1],
            (VariableLayout::Line { base }, SketchPointKind::End, 0) => vec![base + 2],
            (VariableLayout::Line { base }, SketchPointKind::End, 1) => vec![base + 3],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Start, 0) => vec![base],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Start, 1) => vec![base + 1],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Control1, 0) => vec![base + 2],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Control1, 1) => vec![base + 3],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Control2, 0) => vec![base + 4],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Control2, 1) => vec![base + 5],
            (VariableLayout::CubicBezier { base }, SketchPointKind::End, 0) => vec![base + 6],
            (VariableLayout::CubicBezier { base }, SketchPointKind::End, 1) => vec![base + 7],
            (VariableLayout::Arc { base }, SketchPointKind::Center, 0)
            | (VariableLayout::Circle { base }, SketchPointKind::Center, 0) => vec![base],
            (VariableLayout::Arc { base }, SketchPointKind::Center, 1)
            | (VariableLayout::Circle { base }, SketchPointKind::Center, 1) => vec![base + 1],
            (VariableLayout::Arc { base }, SketchPointKind::Start, 0) => {
                vec![base, base + 2, base + 3]
            }
            (VariableLayout::Arc { base }, SketchPointKind::Start, 1) => {
                vec![base + 1, base + 2, base + 3]
            }
            (VariableLayout::Arc { base }, SketchPointKind::End, 0) => {
                vec![base, base + 2, base + 4]
            }
            (VariableLayout::Arc { base }, SketchPointKind::End, 1) => {
                vec![base + 1, base + 2, base + 4]
            }
            _ => return Err(SketchError::InvalidConstraintReference(constraint.id)),
        };
        Ok(variables)
    };
    let entity_variables = |entity: SketchEntityId| {
        let (base, width) = match layouts
            .get(&entity)
            .ok_or(SketchError::InvalidConstraintReference(constraint.id))?
        {
            VariableLayout::Line { base } => (*base, 4),
            VariableLayout::Arc { base } => (*base, 5),
            VariableLayout::Circle { base } => (*base, 3),
            VariableLayout::CubicBezier { base } => (*base, 8),
        };
        Ok((base..base + width).collect::<Vec<_>>())
    };
    let union = |mut a: Vec<usize>, b: Vec<usize>| {
        a.extend(b);
        a.sort_unstable();
        a.dedup();
        a
    };
    match &constraint.kind {
        SketchConstraintKind::Horizontal { entity } => match layouts.get(entity) {
            Some(VariableLayout::Line { base }) => Ok(vec![vec![base + 1, base + 3]]),
            _ => Err(SketchError::InvalidConstraintReference(constraint.id)),
        },
        SketchConstraintKind::Vertical { entity } => match layouts.get(entity) {
            Some(VariableLayout::Line { base }) => Ok(vec![vec![*base, base + 2]]),
            _ => Err(SketchError::InvalidConstraintReference(constraint.id)),
        },
        SketchConstraintKind::Coincident { a, b } => Ok(vec![
            union(point_variables(*a, 0)?, point_variables(*b, 0)?),
            union(point_variables(*a, 1)?, point_variables(*b, 1)?),
        ]),
        SketchConstraintKind::Distance { a, b, .. } => {
            if let Some(entity) = arc_radius_entity(*a, *b)
                && let Some(VariableLayout::Arc { base }) = layouts.get(&entity)
            {
                return Ok(vec![vec![base + 2]]);
            }
            Ok(vec![union(
                union(point_variables(*a, 0)?, point_variables(*a, 1)?),
                union(point_variables(*b, 0)?, point_variables(*b, 1)?),
            )])
        }
        SketchConstraintKind::Radius { entity, .. } => match layouts.get(entity) {
            Some(VariableLayout::Arc { base } | VariableLayout::Circle { base }) => {
                Ok(vec![vec![base + 2]])
            }
            _ => Err(SketchError::InvalidConstraintReference(constraint.id)),
        },
        SketchConstraintKind::FixedPoint { point, .. } => Ok(vec![
            point_variables(*point, 0)?,
            point_variables(*point, 1)?,
        ]),
        SketchConstraintKind::Parallel { a, b }
        | SketchConstraintKind::Perpendicular { a, b }
        | SketchConstraintKind::Tangent { a, b }
        | SketchConstraintKind::Angle { a, b, .. }
        | SketchConstraintKind::Equal { a, b } => {
            Ok(vec![union(entity_variables(*a)?, entity_variables(*b)?)])
        }
        SketchConstraintKind::Symmetric { a, b, axis } => {
            let variables = union(
                union(
                    union(point_variables(*a, 0)?, point_variables(*a, 1)?),
                    union(point_variables(*b, 0)?, point_variables(*b, 1)?),
                ),
                entity_variables(*axis)?,
            );
            Ok(vec![variables.clone(), variables])
        }
        SketchConstraintKind::Concentric { a, b } => Ok(vec![
            union(
                point_variables(
                    SketchPointRef {
                        entity: *a,
                        point: SketchPointKind::Center,
                    },
                    0,
                )?,
                point_variables(
                    SketchPointRef {
                        entity: *b,
                        point: SketchPointKind::Center,
                    },
                    0,
                )?,
            ),
            union(
                point_variables(
                    SketchPointRef {
                        entity: *a,
                        point: SketchPointKind::Center,
                    },
                    1,
                )?,
                point_variables(
                    SketchPointRef {
                        entity: *b,
                        point: SketchPointKind::Center,
                    },
                    1,
                )?,
            ),
        ]),
        SketchConstraintKind::Collinear { a, b } => {
            let variables = union(entity_variables(*a)?, entity_variables(*b)?);
            Ok(vec![variables.clone(), variables])
        }
        SketchConstraintKind::Midpoint { point, line } => {
            let variables = entity_variables(*line)?;
            Ok(vec![
                union(point_variables(*point, 0)?, variables.clone()),
                union(point_variables(*point, 1)?, variables),
            ])
        }
        SketchConstraintKind::PointOnCurve { point, curve } => Ok(vec![union(
            union(point_variables(*point, 0)?, point_variables(*point, 1)?),
            entity_variables(*curve)?,
        )]),
        SketchConstraintKind::Projection { entity, .. } => Ok(entity_variables(*entity)?
            .into_iter()
            .map(|variable| vec![variable])
            .collect()),
        SketchConstraintKind::Construction { .. } => Ok(Vec::new()),
    }
}

pub(super) fn structural_constraint_matching(
    equations: &[Vec<usize>],
    variable_count: usize,
    skipped: Option<std::ops::Range<usize>>,
) -> Vec<Option<usize>> {
    let mut variable_owner = vec![None; variable_count];
    for equation in 0..equations.len() {
        if skipped
            .as_ref()
            .is_some_and(|range| range.contains(&equation))
        {
            continue;
        }
        let mut visited = vec![false; variable_count];
        augment_constraint_rank(equation, equations, &mut variable_owner, &mut visited);
    }
    variable_owner
}

pub(super) fn structural_constraint_rank(
    equations: &[Vec<usize>],
    variable_count: usize,
    skipped: Option<std::ops::Range<usize>>,
) -> usize {
    structural_constraint_matching(equations, variable_count, skipped)
        .into_iter()
        .flatten()
        .count()
}

pub(super) fn augment_constraint_rank(
    equation: usize,
    equations: &[Vec<usize>],
    variable_owner: &mut [Option<usize>],
    visited: &mut [bool],
) -> bool {
    for variable in &equations[equation] {
        if visited[*variable] {
            continue;
        }
        visited[*variable] = true;
        let can_assign = match variable_owner[*variable] {
            None => true,
            Some(owner) => augment_constraint_rank(owner, equations, variable_owner, visited),
        };
        if can_assign {
            variable_owner[*variable] = Some(equation);
            return true;
        }
    }
    false
}

pub(super) fn solve_constraints(
    entities: &mut [SketchEntity],
    constraints: &[SketchConstraint],
    policy: SketchSolverPolicy,
) -> Result<(), SketchError> {
    let indices = entities
        .iter()
        .enumerate()
        .map(|(index, entity)| (entity.id(), index))
        .collect::<BTreeMap<_, _>>();

    // Preserve the established branch choice and exact results for simple constraints.
    for constraint in constraints {
        project_constraint(entities, &indices, constraint, constraints)?;
    }
    let mut residual = constraint_residuals(entities, &indices, constraints)?;
    if residuals_converged(&residual, policy.tolerance_mm) {
        return Ok(());
    }

    let (layouts, variable_count) = variable_layouts(entities)?;
    let equation_variables = constraints
        .iter()
        .map(|constraint| constraint_variable_equations(constraint, &layouts))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if equation_variables.len() != residual.len() {
        return Err(SketchError::NonConvergent);
    }
    let active_columns = equation_variables
        .iter()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let active_indices = active_columns
        .iter()
        .enumerate()
        .map(|(active, original)| (*original, active))
        .collect::<BTreeMap<_, _>>();
    let reduced_equation_variables = equation_variables
        .iter()
        .map(|variables| {
            variables
                .iter()
                .map(|variable| active_indices[variable])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let equation_nonzeros = reduced_equation_variables
        .iter()
        .map(Vec::len)
        .sum::<usize>();
    let finite_difference_work = active_columns
        .len()
        .checked_mul(residual.len())
        .and_then(|work| work.checked_mul(2))
        .ok_or(SketchError::ResourceLimit)?;
    let conjugate_gradient_work = active_columns
        .len()
        .checked_mul(equation_nonzeros)
        .and_then(|work| work.checked_mul(2))
        .ok_or(SketchError::ResourceLimit)?;
    let numerical_work = finite_difference_work
        .checked_add(conjugate_gradient_work)
        .and_then(|work| work.checked_mul(usize::from(policy.max_iterations)))
        .ok_or(SketchError::ResourceLimit)?;
    if numerical_work > MAX_SKETCH_NUMERICAL_EVALUATIONS {
        return Err(SketchError::ResourceLimit);
    }

    let mut parameters = pack_solver_parameters(entities);
    if parameters.len() != variable_count {
        return Err(SketchError::NonConvergent);
    }
    let mut objective = residual_objective(&residual);
    let mut damping = policy.initial_damping;

    for _ in 0..policy.max_iterations {
        let jacobian = numerical_sketch_jacobian(
            SketchJacobianContext {
                entities,
                indices: &indices,
                constraints,
                active_columns: &active_columns,
                equation_variables: &reduced_equation_variables,
                policy,
            },
            &parameters,
            &residual,
        )?;
        let Some(step) =
            sketch_least_squares_step(&jacobian, &reduced_equation_variables, &residual, damping)
        else {
            damping *= 10.0;
            if !damping.is_finite() {
                break;
            }
            continue;
        };
        let mut candidate_parameters = parameters.clone();
        for (column, delta) in active_columns.iter().zip(step) {
            candidate_parameters[*column] += delta;
        }
        let mut candidate_entities = entities.to_vec();
        if !unpack_solver_parameters(
            &mut candidate_entities,
            &candidate_parameters,
            &active_columns,
        ) {
            damping *= 10.0;
            continue;
        }
        let candidate_residual = constraint_residuals(&candidate_entities, &indices, constraints)?;
        let candidate_objective = residual_objective(&candidate_residual);
        if candidate_objective.is_finite() && candidate_objective < objective {
            entities.clone_from_slice(&candidate_entities);
            parameters = candidate_parameters;
            residual = candidate_residual;
            objective = candidate_objective;
            if residuals_converged(&residual, policy.tolerance_mm) {
                return Ok(());
            }
            damping = (damping * 0.25).max(f64::EPSILON);
        } else {
            damping *= 10.0;
            if !damping.is_finite() {
                break;
            }
        }
    }

    Err(SketchError::NonConvergent)
}

pub(super) fn pack_solver_parameters(entities: &[SketchEntity]) -> Vec<f64> {
    let mut parameters = Vec::new();
    for entity in entities {
        match entity {
            SketchEntity::Line {
                start_mm, end_mm, ..
            } => parameters.extend([start_mm[0], start_mm[1], end_mm[0], end_mm[1]]),
            SketchEntity::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            } => {
                let radius = distance2(*start_mm, *center_mm);
                parameters.extend([
                    center_mm[0],
                    center_mm[1],
                    radius,
                    (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]),
                    (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]),
                ]);
            }
            SketchEntity::Circle {
                center_mm,
                radius_mm,
                ..
            } => parameters.extend([center_mm[0], center_mm[1], *radius_mm]),
            SketchEntity::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
                ..
            } => parameters.extend([
                start_mm[0],
                start_mm[1],
                control_1_mm[0],
                control_1_mm[1],
                control_2_mm[0],
                control_2_mm[1],
                end_mm[0],
                end_mm[1],
            ]),
        }
    }
    parameters
}

pub(super) fn unpack_solver_parameters(
    entities: &mut [SketchEntity],
    parameters: &[f64],
    active_columns: &[usize],
) -> bool {
    if parameters.iter().any(|value| !value.is_finite()) {
        return false;
    }
    let mut offset = 0;
    for entity in entities {
        let width = entity.degrees_of_freedom();
        let active = active_columns
            .iter()
            .any(|column| (offset..offset + width).contains(column));
        let Some(values) = parameters.get(offset..offset + width) else {
            return false;
        };
        if active {
            match entity {
                SketchEntity::Line {
                    start_mm, end_mm, ..
                } => {
                    *start_mm = [values[0], values[1]];
                    *end_mm = [values[2], values[3]];
                }
                SketchEntity::Arc {
                    start_mm,
                    end_mm,
                    center_mm,
                    ..
                } => {
                    let radius = values[2];
                    if radius <= EPSILON_MM {
                        return false;
                    }
                    *center_mm = [values[0], values[1]];
                    *start_mm = [
                        values[0] + radius * values[3].cos(),
                        values[1] + radius * values[3].sin(),
                    ];
                    *end_mm = [
                        values[0] + radius * values[4].cos(),
                        values[1] + radius * values[4].sin(),
                    ];
                }
                SketchEntity::Circle {
                    center_mm,
                    radius_mm,
                    ..
                } => {
                    if values[2] <= EPSILON_MM {
                        return false;
                    }
                    *center_mm = [values[0], values[1]];
                    *radius_mm = values[2];
                }
                SketchEntity::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                    ..
                } => {
                    *start_mm = [values[0], values[1]];
                    *control_1_mm = [values[2], values[3]];
                    *control_2_mm = [values[4], values[5]];
                    *end_mm = [values[6], values[7]];
                }
            }
        }
        offset += width;
    }
    offset == parameters.len()
}

pub(super) fn solver_parameter_step(
    entities: &[SketchEntity],
    column: usize,
    base_step: f64,
) -> f64 {
    let mut offset = 0;
    for entity in entities {
        let width = entity.degrees_of_freedom();
        if (offset..offset + width).contains(&column) {
            if let SketchEntity::Arc {
                center_mm,
                start_mm,
                ..
            } = entity
            {
                let local = column - offset;
                let coordinate_scale = center_mm[0].abs().max(center_mm[1].abs()).max(1.0);
                let representable_mm = 64.0 * f64::EPSILON * coordinate_scale;
                if local == 2 {
                    return base_step.max(representable_mm);
                }
                if matches!(local, 3 | 4) {
                    let radius = distance2(*start_mm, *center_mm).max(EPSILON_MM);
                    return base_step.max(representable_mm / radius).min(0.25);
                }
            }
            return base_step;
        }
        offset += width;
    }
    base_step
}

pub(super) struct SketchJacobianContext<'a> {
    pub(super) entities: &'a [SketchEntity],
    pub(super) indices: &'a BTreeMap<SketchEntityId, usize>,
    pub(super) constraints: &'a [SketchConstraint],
    pub(super) active_columns: &'a [usize],
    pub(super) equation_variables: &'a [Vec<usize>],
    pub(super) policy: SketchSolverPolicy,
}

pub(super) fn numerical_sketch_jacobian(
    context: SketchJacobianContext<'_>,
    parameters: &[f64],
    baseline: &[f64],
) -> Result<Vec<Vec<f64>>, SketchError> {
    let mut jacobian = vec![vec![0.0; context.active_columns.len()]; baseline.len()];
    for (active_column, column) in context.active_columns.iter().copied().enumerate() {
        let step = solver_parameter_step(
            context.entities,
            column,
            context.policy.finite_difference_step,
        );
        let mut positive_parameters = parameters.to_vec();
        positive_parameters[column] += step;
        let mut positive_entities = context.entities.to_vec();
        let positive =
            unpack_solver_parameters(&mut positive_entities, &positive_parameters, &[column])
                .then(|| {
                    constraint_residuals(&positive_entities, context.indices, context.constraints)
                })
                .transpose()?;

        let mut negative_parameters = parameters.to_vec();
        negative_parameters[column] -= step;
        let mut negative_entities = context.entities.to_vec();
        let negative =
            unpack_solver_parameters(&mut negative_entities, &negative_parameters, &[column])
                .then(|| {
                    constraint_residuals(&negative_entities, context.indices, context.constraints)
                })
                .transpose()?;

        for row in 0..baseline.len() {
            if context.equation_variables[row]
                .binary_search(&active_column)
                .is_err()
            {
                continue;
            }
            jacobian[row][active_column] = match (&positive, &negative) {
                (Some(positive), Some(negative)) => (positive[row] - negative[row]) / (2.0 * step),
                (Some(positive), None) => (positive[row] - baseline[row]) / step,
                (None, Some(negative)) => (baseline[row] - negative[row]) / step,
                (None, None) => return Err(SketchError::NonConvergent),
            };
        }
    }
    Ok(jacobian)
}

pub(super) fn sketch_least_squares_step(
    jacobian: &[Vec<f64>],
    equation_variables: &[Vec<usize>],
    residual: &[f64],
    damping: f64,
) -> Option<Vec<f64>> {
    let columns = jacobian.first().map_or(0, Vec::len);
    if columns == 0 {
        return Some(Vec::new());
    }
    let mut right = vec![0.0; columns];
    let mut diagonal = vec![0.0; columns];
    for (row, variables) in equation_variables.iter().enumerate() {
        for &column in variables {
            let derivative = jacobian[row][column];
            right[column] -= derivative * residual[row];
            diagonal[column] += derivative * derivative;
        }
    }
    let damping_diagonal = diagonal
        .iter()
        .map(|value| damping * value.abs().max(1.0))
        .collect::<Vec<_>>();
    for (value, damping) in diagonal.iter_mut().zip(&damping_diagonal) {
        *value += damping;
    }
    if diagonal
        .iter()
        .any(|value| !value.is_finite() || *value <= f64::EPSILON)
    {
        return None;
    }

    let mut solution = vec![0.0; columns];
    let mut remainder = right.clone();
    let mut preconditioned = remainder
        .iter()
        .zip(&diagonal)
        .map(|(value, diagonal)| value / diagonal)
        .collect::<Vec<_>>();
    let mut direction = preconditioned.clone();
    let mut product = dot_slice(&remainder, &preconditioned);
    let initial_norm = remainder
        .iter()
        .fold(0.0_f64, |norm, value| norm.max(value.abs()));
    if initial_norm <= f64::EPSILON {
        return Some(solution);
    }

    for _ in 0..columns {
        let applied =
            apply_sketch_normal(jacobian, equation_variables, &damping_diagonal, &direction);
        let denominator = dot_slice(&direction, &applied);
        if !denominator.is_finite() {
            return None;
        }
        if denominator <= f64::EPSILON {
            break;
        }
        let alpha = product / denominator;
        if !alpha.is_finite() {
            return None;
        }
        for index in 0..columns {
            solution[index] += alpha * direction[index];
            remainder[index] -= alpha * applied[index];
        }
        let norm = remainder
            .iter()
            .fold(0.0_f64, |norm, value| norm.max(value.abs()));
        if norm <= NEGLIGIBLE * initial_norm.max(1.0) {
            break;
        }
        preconditioned = remainder
            .iter()
            .zip(&diagonal)
            .map(|(value, diagonal)| value / diagonal)
            .collect();
        let next_product = dot_slice(&remainder, &preconditioned);
        if !next_product.is_finite() {
            return None;
        }
        if product.abs() <= f64::EPSILON {
            break;
        }
        let beta = next_product / product;
        for index in 0..columns {
            direction[index] = preconditioned[index] + beta * direction[index];
        }
        product = next_product;
    }
    solution
        .iter()
        .all(|value| value.is_finite())
        .then_some(solution)
}

pub(super) fn apply_sketch_normal(
    jacobian: &[Vec<f64>],
    equation_variables: &[Vec<usize>],
    damping_diagonal: &[f64],
    vector: &[f64],
) -> Vec<f64> {
    let mut result = vector
        .iter()
        .zip(damping_diagonal)
        .map(|(value, damping)| value * damping)
        .collect::<Vec<_>>();
    for (row, variables) in equation_variables.iter().enumerate() {
        let projected = variables
            .iter()
            .map(|column| jacobian[row][*column] * vector[*column])
            .sum::<f64>();
        for &column in variables {
            result[column] += jacobian[row][column] * projected;
        }
    }
    result
}

pub(super) fn dot_slice(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

pub(super) fn residuals_converged(residuals: &[f64], tolerance: f64) -> bool {
    residuals
        .iter()
        .all(|residual| residual.is_finite() && residual.abs() <= tolerance)
}

pub(super) fn residual_objective(residuals: &[f64]) -> f64 {
    residuals.iter().map(|residual| residual * residual).sum()
}

pub(super) fn project_constraint(
    entities: &mut [SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    constraint: &SketchConstraint,
    constraints: &[SketchConstraint],
) -> Result<(), SketchError> {
    match &constraint.kind {
        SketchConstraintKind::Horizontal { entity } => {
            let SketchEntity::Line {
                start_mm, end_mm, ..
            } = entity_mut(entities, indices, *entity, constraint.id)?
            else {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            };
            let y = (start_mm[1] + end_mm[1]) * 0.5;
            start_mm[1] = y;
            end_mm[1] = y;
        }
        SketchConstraintKind::Vertical { entity } => {
            let SketchEntity::Line {
                start_mm, end_mm, ..
            } = entity_mut(entities, indices, *entity, constraint.id)?
            else {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            };
            let x = (start_mm[0] + end_mm[0]) * 0.5;
            start_mm[0] = x;
            end_mm[0] = x;
        }
        SketchConstraintKind::Coincident { a, b } => {
            let a_position = solved_point(entities, indices, *a, constraint.id)?;
            let b_position = solved_point(entities, indices, *b, constraint.id)?;
            let midpoint = [
                (a_position[0] + b_position[0]) * 0.5,
                (a_position[1] + b_position[1]) * 0.5,
            ];
            set_solved_point(entities, indices, *a, midpoint, constraint.id)?;
            set_solved_point(entities, indices, *b, midpoint, constraint.id)?;
        }
        SketchConstraintKind::Distance { a, b, value } => {
            let expected = valid_positive_dimension(value)?;
            if let Some(entity) = arc_radius_entity(*a, *b) {
                let SketchEntity::Arc {
                    start_mm,
                    end_mm,
                    center_mm,
                    ..
                } = entity_mut(entities, indices, entity, constraint.id)?
                else {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                };
                *start_mm = point_at_radius(*start_mm, *center_mm, expected, [1.0, 0.0]);
                *end_mm = point_at_radius(*end_mm, *center_mm, expected, [0.0, 1.0]);
                return Ok(());
            }
            let a_position = solved_point(entities, indices, *a, constraint.id)?;
            let b_position = solved_point(entities, indices, *b, constraint.id)?;
            let delta = [b_position[0] - a_position[0], b_position[1] - a_position[1]];
            let length = delta[0].hypot(delta[1]);
            let direction = if length <= EPSILON_MM {
                [1.0, 0.0]
            } else {
                [delta[0] / length, delta[1] / length]
            };
            let correction = (expected - length) * 0.5;
            set_solved_point(
                entities,
                indices,
                *a,
                [
                    a_position[0] - direction[0] * correction,
                    a_position[1] - direction[1] * correction,
                ],
                constraint.id,
            )?;
            set_solved_point(
                entities,
                indices,
                *b,
                [
                    b_position[0] + direction[0] * correction,
                    b_position[1] + direction[1] * correction,
                ],
                constraint.id,
            )?;
        }
        SketchConstraintKind::Radius { entity, value } => {
            let expected = valid_positive_dimension(value)?;
            match entity_mut(entities, indices, *entity, constraint.id)? {
                SketchEntity::Circle { radius_mm, .. } => *radius_mm = expected,
                SketchEntity::Arc {
                    start_mm,
                    end_mm,
                    center_mm,
                    ..
                } => {
                    *start_mm = point_at_radius(*start_mm, *center_mm, expected, [1.0, 0.0]);
                    *end_mm = point_at_radius(*end_mm, *center_mm, expected, [0.0, 1.0]);
                }
                SketchEntity::Line { .. } | SketchEntity::CubicBezier { .. } => {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                }
            }
        }
        SketchConstraintKind::FixedPoint { point, position_mm } => {
            set_fixed_point(
                entities,
                indices,
                *point,
                *position_mm,
                constraint.id,
                constraints,
            )?;
        }
        SketchConstraintKind::Projection { entity, target, .. } => {
            *entity_mut(entities, indices, *entity, constraint.id)? = target.as_ref().clone();
        }
        SketchConstraintKind::Construction { .. } => {}
        SketchConstraintKind::Parallel { .. }
        | SketchConstraintKind::Perpendicular { .. }
        | SketchConstraintKind::Tangent { .. }
        | SketchConstraintKind::Angle { .. }
        | SketchConstraintKind::Equal { .. }
        | SketchConstraintKind::Symmetric { .. }
        | SketchConstraintKind::Concentric { .. }
        | SketchConstraintKind::Collinear { .. }
        | SketchConstraintKind::Midpoint { .. }
        | SketchConstraintKind::PointOnCurve { .. } => {}
    }
    Ok(())
}

pub(super) fn line_frame(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    id: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<([f64; 2], [f64; 2], f64), SketchError> {
    let SketchEntity::Line {
        start_mm, end_mm, ..
    } = entity_ref(entities, indices, id, constraint_id)?
    else {
        return Err(SketchError::InvalidConstraintReference(constraint_id));
    };
    let delta = [end_mm[0] - start_mm[0], end_mm[1] - start_mm[1]];
    let length = delta[0].hypot(delta[1]);
    if length <= EPSILON_MM {
        return Err(SketchError::InvalidConstraintReference(constraint_id));
    }
    Ok((*start_mm, [delta[0] / length, delta[1] / length], length))
}

pub(super) fn circular_geometry(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    id: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<([f64; 2], f64), SketchError> {
    match entity_ref(entities, indices, id, constraint_id)? {
        SketchEntity::Arc {
            start_mm,
            center_mm,
            ..
        } => Ok((*center_mm, distance2(*start_mm, *center_mm))),
        SketchEntity::Circle {
            center_mm,
            radius_mm,
            ..
        } => Ok((*center_mm, *radius_mm)),
        SketchEntity::Line { .. } | SketchEntity::CubicBezier { .. } => {
            Err(SketchError::InvalidConstraintReference(constraint_id))
        }
    }
}

pub(super) fn line_point_distance(point: [f64; 2], origin: [f64; 2], direction: [f64; 2]) -> f64 {
    (point[0] - origin[0]) * direction[1] - (point[1] - origin[1]) * direction[0]
}

pub(super) fn arc_endpoint_penalty(entity: &SketchEntity, point: [f64; 2]) -> f64 {
    let SketchEntity::Arc {
        start_mm,
        end_mm,
        center_mm,
        clockwise,
        ..
    } = entity
    else {
        return 0.0;
    };
    let radius = distance2(*start_mm, *center_mm);
    let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
    let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
    let point_angle = (point[1] - center_mm[1]).atan2(point[0] - center_mm[0]);
    let (sweep, travel) = if *clockwise {
        (
            (start_angle - end_angle).rem_euclid(std::f64::consts::TAU),
            (start_angle - point_angle).rem_euclid(std::f64::consts::TAU),
        )
    } else {
        (
            (end_angle - start_angle).rem_euclid(std::f64::consts::TAU),
            (point_angle - start_angle).rem_euclid(std::f64::consts::TAU),
        )
    };
    if travel <= sweep + EPSILON_MM / radius {
        0.0
    } else {
        distance2(point, *start_mm).min(distance2(point, *end_mm))
    }
}

pub(super) fn bounded_arc_residual(base: f64, penalty: f64) -> f64 {
    if penalty <= EPSILON_MM {
        base
    } else {
        base.abs().hypot(penalty)
    }
}

pub(super) fn tangent_residual(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    a: SketchEntityId,
    b: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<f64, SketchError> {
    match (
        entity_ref(entities, indices, a, constraint_id)?,
        entity_ref(entities, indices, b, constraint_id)?,
    ) {
        (
            SketchEntity::Line { .. },
            circular @ (SketchEntity::Arc { .. } | SketchEntity::Circle { .. }),
        ) => {
            let (origin, direction, _) = line_frame(entities, indices, a, constraint_id)?;
            let (center, radius) = circular_geometry(entities, indices, b, constraint_id)?;
            let signed_distance = line_point_distance(center, origin, direction);
            let contact = [
                center[0] - signed_distance * direction[1],
                center[1] + signed_distance * direction[0],
            ];
            Ok(bounded_arc_residual(
                signed_distance.abs() - radius,
                arc_endpoint_penalty(circular, contact),
            ))
        }
        (SketchEntity::Arc { .. } | SketchEntity::Circle { .. }, SketchEntity::Line { .. }) => {
            tangent_residual(entities, indices, b, a, constraint_id)
        }
        (
            SketchEntity::Arc { .. } | SketchEntity::Circle { .. },
            SketchEntity::Arc { .. } | SketchEntity::Circle { .. },
        ) => {
            let (a_center, a_radius) = circular_geometry(entities, indices, a, constraint_id)?;
            let (b_center, b_radius) = circular_geometry(entities, indices, b, constraint_id)?;
            let center_distance = distance2(a_center, b_center);
            let external = center_distance - (a_radius + b_radius);
            let radius_difference = (a_radius - b_radius).abs();
            let internal = center_distance - radius_difference;
            if center_distance <= EPSILON_MM {
                return Ok(if radius_difference <= EPSILON_MM {
                    external
                } else {
                    internal
                });
            }
            let external_branch = external.abs() <= internal.abs();
            let direction = [
                (b_center[0] - a_center[0]) / center_distance,
                (b_center[1] - a_center[1]) / center_distance,
            ];
            let (a_contact, b_contact, base) = if external_branch {
                (
                    [
                        a_center[0] + direction[0] * a_radius,
                        a_center[1] + direction[1] * a_radius,
                    ],
                    [
                        b_center[0] - direction[0] * b_radius,
                        b_center[1] - direction[1] * b_radius,
                    ],
                    external,
                )
            } else if a_radius >= b_radius {
                (
                    [
                        a_center[0] + direction[0] * a_radius,
                        a_center[1] + direction[1] * a_radius,
                    ],
                    [
                        b_center[0] + direction[0] * b_radius,
                        b_center[1] + direction[1] * b_radius,
                    ],
                    internal,
                )
            } else {
                (
                    [
                        a_center[0] - direction[0] * a_radius,
                        a_center[1] - direction[1] * a_radius,
                    ],
                    [
                        b_center[0] - direction[0] * b_radius,
                        b_center[1] - direction[1] * b_radius,
                    ],
                    internal,
                )
            };
            let penalty =
                arc_endpoint_penalty(entity_ref(entities, indices, a, constraint_id)?, a_contact)
                    .hypot(arc_endpoint_penalty(
                        entity_ref(entities, indices, b, constraint_id)?,
                        b_contact,
                    ));
            Ok(bounded_arc_residual(base, penalty))
        }
        (SketchEntity::Line { .. }, SketchEntity::Line { .. })
        | (SketchEntity::CubicBezier { .. }, _)
        | (_, SketchEntity::CubicBezier { .. }) => {
            Err(SketchError::InvalidConstraintReference(constraint_id))
        }
    }
}

pub(super) fn entity_measure(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    id: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<f64, SketchError> {
    match entity_ref(entities, indices, id, constraint_id)? {
        SketchEntity::Line {
            start_mm, end_mm, ..
        } => Ok(distance2(*start_mm, *end_mm)),
        SketchEntity::Arc {
            start_mm,
            center_mm,
            ..
        } => Ok(distance2(*start_mm, *center_mm)),
        SketchEntity::Circle { radius_mm, .. } => Ok(*radius_mm),
        SketchEntity::CubicBezier { .. } => {
            Err(SketchError::InvalidConstraintReference(constraint_id))
        }
    }
}

pub(super) fn point_on_curve_residual(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    point: [f64; 2],
    curve: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<f64, SketchError> {
    match entity_ref(entities, indices, curve, constraint_id)? {
        SketchEntity::Line { .. } => {
            let (origin, direction, _) = line_frame(entities, indices, curve, constraint_id)?;
            Ok(line_point_distance(point, origin, direction))
        }
        circular @ (SketchEntity::Arc { .. } | SketchEntity::Circle { .. }) => {
            let (center, radius) = circular_geometry(entities, indices, curve, constraint_id)?;
            Ok(bounded_arc_residual(
                distance2(point, center) - radius,
                arc_endpoint_penalty(circular, point),
            ))
        }
        SketchEntity::CubicBezier { .. } => {
            Err(SketchError::InvalidConstraintReference(constraint_id))
        }
    }
}

pub(super) fn constraint_residuals(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    constraints: &[SketchConstraint],
) -> Result<Vec<f64>, SketchError> {
    let mut residuals = Vec::new();
    for constraint in constraints {
        match &constraint.kind {
            SketchConstraintKind::Horizontal { entity } => {
                let SketchEntity::Line {
                    start_mm, end_mm, ..
                } = entity_ref(entities, indices, *entity, constraint.id)?
                else {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                };
                residuals.push(end_mm[1] - start_mm[1]);
            }
            SketchConstraintKind::Vertical { entity } => {
                let SketchEntity::Line {
                    start_mm, end_mm, ..
                } = entity_ref(entities, indices, *entity, constraint.id)?
                else {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                };
                residuals.push(end_mm[0] - start_mm[0]);
            }
            SketchConstraintKind::Coincident { a, b } => {
                let a = solved_point(entities, indices, *a, constraint.id)?;
                let b = solved_point(entities, indices, *b, constraint.id)?;
                residuals.extend([a[0] - b[0], a[1] - b[1]]);
            }
            SketchConstraintKind::Distance { a, b, value } => {
                residuals.push(
                    distance2(
                        solved_point(entities, indices, *a, constraint.id)?,
                        solved_point(entities, indices, *b, constraint.id)?,
                    ) - valid_positive_dimension(value)?,
                );
            }
            SketchConstraintKind::Radius { entity, value } => {
                let actual = match entity_ref(entities, indices, *entity, constraint.id)? {
                    SketchEntity::Arc {
                        start_mm,
                        center_mm,
                        ..
                    } => distance2(*start_mm, *center_mm),
                    SketchEntity::Circle { radius_mm, .. } => *radius_mm,
                    SketchEntity::Line { .. } | SketchEntity::CubicBezier { .. } => {
                        return Err(SketchError::InvalidConstraintReference(constraint.id));
                    }
                };
                residuals.push(actual - valid_positive_dimension(value)?);
            }
            SketchConstraintKind::FixedPoint { point, position_mm } => {
                let actual = solved_point(entities, indices, *point, constraint.id)?;
                residuals.extend([actual[0] - position_mm[0], actual[1] - position_mm[1]]);
            }
            SketchConstraintKind::Parallel { a, b } => {
                let (_, a_direction, _) = line_frame(entities, indices, *a, constraint.id)?;
                let (_, b_direction, _) = line_frame(entities, indices, *b, constraint.id)?;
                residuals.push(a_direction[0] * b_direction[1] - a_direction[1] * b_direction[0]);
            }
            SketchConstraintKind::Perpendicular { a, b } => {
                let (_, a_direction, _) = line_frame(entities, indices, *a, constraint.id)?;
                let (_, b_direction, _) = line_frame(entities, indices, *b, constraint.id)?;
                residuals.push(a_direction[0] * b_direction[0] + a_direction[1] * b_direction[1]);
            }
            SketchConstraintKind::Tangent { a, b } => {
                residuals.push(tangent_residual(entities, indices, *a, *b, constraint.id)?);
            }
            SketchConstraintKind::Angle {
                a,
                b,
                angle_degrees,
            } => {
                let (_, a_direction, _) = line_frame(entities, indices, *a, constraint.id)?;
                let (_, b_direction, _) = line_frame(entities, indices, *b, constraint.id)?;
                residuals.push(
                    a_direction[0] * b_direction[0] + a_direction[1] * b_direction[1]
                        - angle_degrees.to_radians().cos(),
                );
            }
            SketchConstraintKind::Equal { a, b } => {
                residuals.push(
                    entity_measure(entities, indices, *a, constraint.id)?
                        - entity_measure(entities, indices, *b, constraint.id)?,
                );
            }
            SketchConstraintKind::Symmetric { a, b, axis } => {
                let a = solved_point(entities, indices, *a, constraint.id)?;
                let b = solved_point(entities, indices, *b, constraint.id)?;
                let (origin, direction, _) = line_frame(entities, indices, *axis, constraint.id)?;
                let midpoint = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                residuals.extend([
                    line_point_distance(midpoint, origin, direction),
                    (b[0] - a[0]) * direction[0] + (b[1] - a[1]) * direction[1],
                ]);
            }
            SketchConstraintKind::Concentric { a, b } => {
                let (a_center, _) = circular_geometry(entities, indices, *a, constraint.id)?;
                let (b_center, _) = circular_geometry(entities, indices, *b, constraint.id)?;
                residuals.extend([a_center[0] - b_center[0], a_center[1] - b_center[1]]);
            }
            SketchConstraintKind::Collinear { a, b } => {
                let (a_origin, a_direction, _) = line_frame(entities, indices, *a, constraint.id)?;
                let (b_origin, b_direction, _) = line_frame(entities, indices, *b, constraint.id)?;
                residuals.extend([
                    a_direction[0] * b_direction[1] - a_direction[1] * b_direction[0],
                    line_point_distance(b_origin, a_origin, a_direction),
                ]);
            }
            SketchConstraintKind::Midpoint { point, line } => {
                let actual = solved_point(entities, indices, *point, constraint.id)?;
                let SketchEntity::Line {
                    start_mm, end_mm, ..
                } = entity_ref(entities, indices, *line, constraint.id)?
                else {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                };
                residuals.extend([
                    actual[0] - (start_mm[0] + end_mm[0]) * 0.5,
                    actual[1] - (start_mm[1] + end_mm[1]) * 0.5,
                ]);
            }
            SketchConstraintKind::PointOnCurve { point, curve } => {
                residuals.push(point_on_curve_residual(
                    entities,
                    indices,
                    solved_point(entities, indices, *point, constraint.id)?,
                    *curve,
                    constraint.id,
                )?);
            }
            SketchConstraintKind::Projection { entity, target, .. } => {
                let actual = pack_solver_parameters(std::slice::from_ref(entity_ref(
                    entities,
                    indices,
                    *entity,
                    constraint.id,
                )?));
                let expected = pack_solver_parameters(std::slice::from_ref(target.as_ref()));
                if actual.len() != expected.len() {
                    return Err(SketchError::InvalidProjectionSource);
                }
                residuals.extend(
                    actual
                        .into_iter()
                        .zip(expected)
                        .map(|(actual, expected)| actual - expected),
                );
            }
            SketchConstraintKind::Construction { .. } => {}
        }
    }
    Ok(residuals)
}

pub(super) fn entity_ref<'a>(
    entities: &'a [SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    id: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<&'a SketchEntity, SketchError> {
    indices
        .get(&id)
        .and_then(|index| entities.get(*index))
        .ok_or(SketchError::InvalidConstraintReference(constraint_id))
}

pub(super) fn entity_mut<'a>(
    entities: &'a mut [SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    id: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<&'a mut SketchEntity, SketchError> {
    indices
        .get(&id)
        .and_then(|index| entities.get_mut(*index))
        .ok_or(SketchError::InvalidConstraintReference(constraint_id))
}

pub(super) fn solved_point(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    reference: SketchPointRef,
    constraint_id: SketchConstraintId,
) -> Result<[f64; 2], SketchError> {
    entity_ref(entities, indices, reference.entity, constraint_id)?
        .point(reference.point)
        .ok_or(SketchError::InvalidConstraintReference(constraint_id))
}

pub(super) fn set_fixed_point(
    entities: &mut [SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    reference: SketchPointRef,
    position: [f64; 2],
    constraint_id: SketchConstraintId,
    constraints: &[SketchConstraint],
) -> Result<(), SketchError> {
    let preserves_arc_radius = matches!(
        reference.point,
        SketchPointKind::Start | SketchPointKind::End
    ) && constraints.iter().any(|constraint| match &constraint.kind {
        SketchConstraintKind::Radius { entity, .. } => *entity == reference.entity,
        SketchConstraintKind::Distance { a, b, .. } => {
            arc_radius_entity(*a, *b) == Some(reference.entity)
        }
        _ => false,
    });
    if preserves_arc_radius {
        let opposite_point = match reference.point {
            SketchPointKind::Start => SketchPointKind::End,
            SketchPointKind::End => SketchPointKind::Start,
            _ => unreachable!("only arc endpoints preserve an arc radius"),
        };
        let opposite_position = constraints
            .iter()
            .find_map(|constraint| match &constraint.kind {
                SketchConstraintKind::FixedPoint { point, position_mm }
                    if point.entity == reference.entity && point.point == opposite_point =>
                {
                    Some(*position_mm)
                }
                _ => None,
            });
        let constrained_radius = constraints
            .iter()
            .find_map(|constraint| match &constraint.kind {
                SketchConstraintKind::Radius { entity, value } if *entity == reference.entity => {
                    Some(value)
                }
                SketchConstraintKind::Distance { a, b, value }
                    if arc_radius_entity(*a, *b) == Some(reference.entity) =>
                {
                    Some(value)
                }
                _ => None,
            });
        let SketchEntity::Arc {
            start_mm,
            end_mm,
            center_mm,
            ..
        } = entity_mut(entities, indices, reference.entity, constraint_id)?
        else {
            return Err(SketchError::InvalidConstraintReference(constraint_id));
        };
        if let (Some(opposite_position), Some(radius)) = (opposite_position, constrained_radius) {
            let radius = valid_positive_dimension(radius)?;
            let (start, end) = match reference.point {
                SketchPointKind::Start => (position, opposite_position),
                SketchPointKind::End => (opposite_position, position),
                _ => unreachable!("only arc endpoints preserve an arc radius"),
            };
            let chord = [end[0] - start[0], end[1] - start[1]];
            let chord_length = chord[0].hypot(chord[1]);
            if chord_length <= EPSILON_MM || chord_length > 2.0 * radius + EPSILON_MM {
                return Err(SketchError::OverConstrained(constraint_id));
            }
            let midpoint = [(start[0] + end[0]) * 0.5, (start[1] + end[1]) * 0.5];
            let height = (radius * radius - (chord_length * 0.5).powi(2))
                .max(0.0)
                .sqrt();
            let normal = [-chord[1] / chord_length, chord[0] / chord_length];
            let centers = [
                [
                    midpoint[0] + normal[0] * height,
                    midpoint[1] + normal[1] * height,
                ],
                [
                    midpoint[0] - normal[0] * height,
                    midpoint[1] - normal[1] * height,
                ],
            ];
            let chosen = if distance2(centers[0], *center_mm) <= distance2(centers[1], *center_mm) {
                centers[0]
            } else {
                centers[1]
            };
            *start_mm = start;
            *end_mm = end;
            *center_mm = chosen;
            return Ok(());
        }
        let current = match reference.point {
            SketchPointKind::Start => *start_mm,
            SketchPointKind::End => *end_mm,
            _ => unreachable!("only arc endpoints preserve an arc radius"),
        };
        let translation = [position[0] - current[0], position[1] - current[1]];
        for point in [start_mm, end_mm, center_mm] {
            point[0] += translation[0];
            point[1] += translation[1];
        }
        return Ok(());
    }
    set_solved_point(entities, indices, reference, position, constraint_id)
}

pub(super) fn set_solved_point(
    entities: &mut [SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    reference: SketchPointRef,
    position: [f64; 2],
    constraint_id: SketchConstraintId,
) -> Result<(), SketchError> {
    match (
        entity_mut(entities, indices, reference.entity, constraint_id)?,
        reference.point,
    ) {
        (SketchEntity::Line { start_mm, .. }, SketchPointKind::Start)
        | (SketchEntity::CubicBezier { start_mm, .. }, SketchPointKind::Start) => {
            *start_mm = position;
        }
        (SketchEntity::Line { end_mm, .. }, SketchPointKind::End)
        | (SketchEntity::CubicBezier { end_mm, .. }, SketchPointKind::End) => {
            *end_mm = position;
        }
        (SketchEntity::CubicBezier { control_1_mm, .. }, SketchPointKind::Control1) => {
            *control_1_mm = position;
        }
        (SketchEntity::CubicBezier { control_2_mm, .. }, SketchPointKind::Control2) => {
            *control_2_mm = position;
        }
        (SketchEntity::Circle { center_mm, .. }, SketchPointKind::Center) => {
            *center_mm = position;
        }
        (
            SketchEntity::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            },
            SketchPointKind::Center,
        ) => {
            let translation = [position[0] - center_mm[0], position[1] - center_mm[1]];
            for point in [start_mm, end_mm] {
                point[0] += translation[0];
                point[1] += translation[1];
            }
            *center_mm = position;
        }
        (
            SketchEntity::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            },
            SketchPointKind::Start,
        ) => {
            let radius = distance2(position, *center_mm);
            if radius <= EPSILON_MM {
                return Err(SketchError::InvalidConstraintReference(constraint_id));
            }
            *start_mm = position;
            *end_mm = point_at_radius(*end_mm, *center_mm, radius, [0.0, 1.0]);
        }
        (
            SketchEntity::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            },
            SketchPointKind::End,
        ) => {
            let radius = distance2(position, *center_mm);
            if radius <= EPSILON_MM {
                return Err(SketchError::InvalidConstraintReference(constraint_id));
            }
            *end_mm = position;
            *start_mm = point_at_radius(*start_mm, *center_mm, radius, [1.0, 0.0]);
        }
        _ => return Err(SketchError::InvalidConstraintReference(constraint_id)),
    }
    Ok(())
}

pub(super) fn point_at_radius(
    point: [f64; 2],
    center: [f64; 2],
    radius: f64,
    fallback_direction: [f64; 2],
) -> [f64; 2] {
    let delta = [point[0] - center[0], point[1] - center[1]];
    let length = delta[0].hypot(delta[1]);
    let direction = if length <= EPSILON_MM {
        fallback_direction
    } else {
        [delta[0] / length, delta[1] / length]
    };
    [
        center[0] + direction[0] * radius,
        center[1] + direction[1] * radius,
    ]
}

pub(super) fn valid_positive_dimension(value: &Dimension) -> Result<f64, SketchError> {
    let canonical = Dimension::new(value.source_token(), value.millimetres())
        .map_err(SketchError::Dimension)?;
    if canonical.millimetres() <= EPSILON_MM {
        return Err(SketchError::InvalidDimension);
    }
    Ok(canonical.millimetres())
}

pub(super) fn arc_radius_entity(a: SketchPointRef, b: SketchPointRef) -> Option<SketchEntityId> {
    if a.entity != b.entity {
        return None;
    }
    matches!(
        (a.point, b.point),
        (
            SketchPointKind::Center,
            SketchPointKind::Start | SketchPointKind::End
        ) | (
            SketchPointKind::Start | SketchPointKind::End,
            SketchPointKind::Center
        )
    )
    .then_some(a.entity)
}

pub(super) fn coincidence_root(
    parents: &BTreeMap<SketchPointRef, SketchPointRef>,
    mut point: SketchPointRef,
) -> SketchPointRef {
    while let Some(parent) = parents.get(&point) {
        point = *parent;
    }
    point
}

pub(super) fn canonical_point_pair(
    a: SketchPointRef,
    b: SketchPointRef,
) -> (SketchPointRef, SketchPointRef) {
    if a <= b { (a, b) } else { (b, a) }
}

pub(super) fn canonical_entity_pair(
    a: SketchEntityId,
    b: SketchEntityId,
) -> (SketchEntityId, SketchEntityId) {
    if a <= b { (a, b) } else { (b, a) }
}

pub(super) fn overlapping_relation_signature(
    kind: &SketchConstraintKind,
) -> Option<(u8, SketchEntityId, SketchEntityId)> {
    let (relation, a, b) = match kind {
        SketchConstraintKind::Parallel { a, b } | SketchConstraintKind::Collinear { a, b } => {
            (1, *a, *b)
        }
        SketchConstraintKind::Perpendicular { a, b }
        | SketchConstraintKind::Angle {
            a,
            b,
            angle_degrees: 90.0,
        } => (2, *a, *b),
        _ => return None,
    };
    let (a, b) = canonical_entity_pair(a, b);
    Some((relation, a, b))
}

pub(super) fn is_circular_entity(entity: Option<&SketchEntity>) -> bool {
    matches!(
        entity,
        Some(SketchEntity::Arc { .. } | SketchEntity::Circle { .. })
    )
}

pub(super) fn is_curve_entity(entity: Option<&SketchEntity>) -> bool {
    matches!(
        entity,
        Some(SketchEntity::Line { .. } | SketchEntity::Arc { .. } | SketchEntity::Circle { .. })
    )
}

pub(super) fn supports_tangent(a: Option<&SketchEntity>, b: Option<&SketchEntity>) -> bool {
    is_curve_entity(a) && is_curve_entity(b) && (is_circular_entity(a) || is_circular_entity(b))
}

pub(super) fn supports_equal(a: Option<&SketchEntity>, b: Option<&SketchEntity>) -> bool {
    matches!(
        (a, b),
        (
            Some(SketchEntity::Line { .. }),
            Some(SketchEntity::Line { .. })
        )
    ) || (is_circular_entity(a) && is_circular_entity(b))
}

pub(super) fn valid_angle_degrees(angle_degrees: f64) -> bool {
    angle_degrees.is_finite() && angle_degrees > 0.0 && angle_degrees < 180.0
}

pub(super) fn push_point_ref(bytes: &mut Vec<u8>, reference: SketchPointRef) {
    bytes.extend_from_slice(&reference.entity.0.to_le_bytes());
    bytes.push(match reference.point {
        SketchPointKind::Start => 1,
        SketchPointKind::End => 2,
        SketchPointKind::Center => 3,
        SketchPointKind::Control1 => 4,
        SketchPointKind::Control2 => 5,
    });
}

pub(super) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(super) fn distance2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

pub(super) fn distance3(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
