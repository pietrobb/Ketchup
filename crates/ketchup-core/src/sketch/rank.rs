use super::*;

// Relative to unit-length equation rows. Below this threshold the numerical
// evidence cannot establish another independent equation: never claim full rank.
const RANK_TOLERANCE: f64 = 1.0e-6;
const DIFFERENCE_STEP: f64 = 1.0e-5;

pub(super) fn analyze(
    entities: &[SketchEntity],
    constraints: &[SketchConstraint],
    layouts: &BTreeMap<SketchEntityId, VariableLayout>,
    equation_variables: &[Vec<usize>],
    equation_ranges: &[(SketchConstraintId, std::ops::Range<usize>)],
    variable_count: usize,
) -> Result<(SketchSolveStatus, Vec<SketchEntityId>), SketchError> {
    let indices = entities
        .iter()
        .enumerate()
        .map(|(index, entity)| (entity.id(), index))
        .collect::<BTreeMap<_, _>>();
    let origins = parameter_origins(entities);
    let scales = parameter_scales(entities);
    let mut jacobian = vec![vec![0.0; variable_count]; equation_variables.len()];
    let active_columns = equation_variables
        .iter()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    for column in active_columns {
        // Translate the whole problem for this column: tiny arc angle changes
        // must not disappear when its center is far from the world origin.
        let (local_entities, local_constraints) =
            translated_problem(entities, constraints, origins[column]);
        let parameters = pack_solver_parameters(&local_entities);
        let step = DIFFERENCE_STEP * scales[column];
        let mut positive_parameters = parameters.clone();
        let mut negative_parameters = parameters.clone();
        positive_parameters[column] += step;
        negative_parameters[column] -= step;
        let mut positive = local_entities.clone();
        let mut negative = local_entities;
        if !unpack_solver_parameters(&mut positive, &positive_parameters, &[column])
            || !unpack_solver_parameters(&mut negative, &negative_parameters, &[column])
        {
            return Err(SketchError::NonConvergent);
        }
        let positive = constraint_residuals(&positive, &indices, &local_constraints)?;
        let negative = constraint_residuals(&negative, &indices, &local_constraints)?;
        let delta = positive_parameters[column] - negative_parameters[column];
        for (row, variables) in equation_variables.iter().enumerate() {
            if variables.binary_search(&column).is_ok() {
                jacobian[row][column] = (positive[row] - negative[row]) / delta * scales[column];
            }
        }
    }
    // These equations constrain solver parameters directly. In particular,
    // differentiating an arc's atan2 roundtrip across +/-pi is not valid.
    for (constraint, (_, range)) in constraints.iter().zip(equation_ranges) {
        match constraint.kind {
            SketchConstraintKind::Projection { .. } | SketchConstraintKind::Radius { .. } => {
                for row in range.clone() {
                    let column = equation_variables[row][0];
                    jacobian[row].fill(0.0);
                    jacobian[row][column] = scales[column];
                }
            }
            _ => {}
        }
    }

    let mut basis: Vec<Vec<f64>> = Vec::new();
    for (constraint_id, range) in equation_ranges {
        for equation in range.clone() {
            let mut row = jacobian[equation].clone();
            let norm = dot_slice(&row, &row).sqrt();
            if !norm.is_finite() {
                return Err(SketchError::NonConvergent);
            }
            if norm == 0.0 {
                return Err(SketchError::OverConstrained(*constraint_id));
            }
            for value in &mut row {
                *value /= norm;
            }
            // Twice-reorthogonalized Gram-Schmidt produces an orthonormal row
            // space without squaring the Jacobian's condition number (J^T J).
            for _ in 0..2 {
                for vector in &basis {
                    let projection = dot_slice(&row, vector);
                    for (value, component) in row.iter_mut().zip(vector) {
                        *value -= projection * component;
                    }
                }
            }
            let norm = dot_slice(&row, &row).sqrt();
            if norm <= RANK_TOLERANCE {
                return Err(SketchError::OverConstrained(*constraint_id));
            }
            for value in &mut row {
                *value /= norm;
            }
            basis.push(row);
        }
    }
    let remaining_dof = variable_count - basis.len();
    let status = if remaining_dof == 0 {
        SketchSolveStatus::FullyConstrained
    } else {
        SketchSolveStatus::UnderConstrained { remaining_dof }
    };
    // diag(I - Q^T Q) is the squared nullspace component of each variable.
    // Unlike unmatched columns it includes dependent variables which move along
    // with free ones, e.g. both entities of an unanchored rigid pair.
    let unconstrained = layouts
        .iter()
        .filter_map(|(id, layout)| {
            layout
                .variable_range()
                .any(|column| {
                    1.0 - basis
                        .iter()
                        .map(|row| row[column] * row[column])
                        .sum::<f64>()
                        > RANK_TOLERANCE * RANK_TOLERANCE
                })
                .then_some(*id)
        })
        .collect();
    Ok((status, unconstrained))
}

fn parameter_origins(entities: &[SketchEntity]) -> Vec<[f64; 2]> {
    entities
        .iter()
        .flat_map(|entity| {
            let origin = match entity {
                SketchEntity::Line { start_mm, .. }
                | SketchEntity::CubicBezier { start_mm, .. } => *start_mm,
                SketchEntity::Arc { center_mm, .. } | SketchEntity::Circle { center_mm, .. } => {
                    *center_mm
                }
            };
            std::iter::repeat_n(origin, entity.degrees_of_freedom())
        })
        .collect()
}

fn translated_problem(
    entities: &[SketchEntity],
    constraints: &[SketchConstraint],
    origin: [f64; 2],
) -> (Vec<SketchEntity>, Vec<SketchConstraint>) {
    let shift = |point: &mut [f64; 2]| {
        point[0] -= origin[0];
        point[1] -= origin[1];
    };
    let translate = |entity: &mut SketchEntity| match entity {
        SketchEntity::Line {
            start_mm, end_mm, ..
        } => {
            shift(start_mm);
            shift(end_mm);
        }
        SketchEntity::Circle { center_mm, .. } => shift(center_mm),
        SketchEntity::Arc {
            start_mm,
            end_mm,
            center_mm,
            ..
        } => {
            shift(start_mm);
            shift(end_mm);
            shift(center_mm);
        }
        SketchEntity::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } => {
            shift(start_mm);
            shift(control_1_mm);
            shift(control_2_mm);
            shift(end_mm);
        }
    };
    let mut entities = entities.to_vec();
    for entity in &mut entities {
        translate(entity);
    }
    let mut constraints = constraints.to_vec();
    for constraint in &mut constraints {
        match &mut constraint.kind {
            SketchConstraintKind::FixedPoint { position_mm, .. } => shift(position_mm),
            SketchConstraintKind::Projection { target, .. } => translate(target),
            _ => {}
        }
    }
    (entities, constraints)
}

fn parameter_scales(entities: &[SketchEntity]) -> Vec<f64> {
    let mut scales = Vec::new();
    for entity in entities {
        match entity {
            SketchEntity::Line {
                start_mm, end_mm, ..
            } => {
                scales.extend([distance2(*start_mm, *end_mm); 4]);
            }
            SketchEntity::Circle { radius_mm, .. } => scales.extend([*radius_mm; 3]),
            SketchEntity::Arc {
                start_mm,
                center_mm,
                ..
            } => {
                let radius = distance2(*start_mm, *center_mm);
                scales.extend([radius, radius, radius, 1.0, 1.0]);
            }
            SketchEntity::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
                ..
            } => {
                let size = distance2(*start_mm, *control_1_mm)
                    .max(distance2(*control_1_mm, *control_2_mm))
                    .max(distance2(*control_2_mm, *end_mm));
                scales.extend([size; 8]);
            }
        }
    }
    scales
}
