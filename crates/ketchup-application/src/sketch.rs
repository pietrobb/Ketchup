use ketchup_core::assistant_sidecar::{
    AssistantPrincipalPlane, AssistantSketchConstraint, AssistantSketchEntity,
    AssistantSketchPointKind, AssistantSketchPointRef,
};
use ketchup_core::document::{CanonicalError, Dimension};
use ketchup_core::sketch::{
    PrincipalPlane, SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity,
    SketchEntityId, SketchPointKind, SketchPointRef,
};

pub(crate) fn assistant_principal_plane(plane: AssistantPrincipalPlane) -> PrincipalPlane {
    match plane {
        AssistantPrincipalPlane::Xy => PrincipalPlane::Xy,
        AssistantPrincipalPlane::Yz => PrincipalPlane::Yz,
        AssistantPrincipalPlane::Xz => PrincipalPlane::Xz,
    }
}

fn assistant_sketch_point_ref(point: AssistantSketchPointRef) -> SketchPointRef {
    SketchPointRef {
        entity: SketchEntityId(point.entity_id),
        point: match point.point {
            AssistantSketchPointKind::Start => SketchPointKind::Start,
            AssistantSketchPointKind::End => SketchPointKind::End,
            AssistantSketchPointKind::Center => SketchPointKind::Center,
            AssistantSketchPointKind::Control1 => SketchPointKind::Control1,
            AssistantSketchPointKind::Control2 => SketchPointKind::Control2,
        },
    }
}

fn transformed_profile_entity(
    entity: SketchEntity,
    id: u64,
    translation_mm: [f64; 2],
    rotation_degrees: f64,
    uniform_scale: f64,
) -> SketchEntity {
    let angle = rotation_degrees.to_radians();
    let (sin, cos) = angle.sin_cos();
    let point = |point: [f64; 2]| {
        let x = uniform_scale * point[0];
        let y = uniform_scale * point[1];
        [
            translation_mm[0] + cos * x - sin * y,
            translation_mm[1] + sin * x + cos * y,
        ]
    };
    let id = SketchEntityId(id);
    match entity {
        SketchEntity::Line {
            start_mm, end_mm, ..
        } => SketchEntity::Line {
            id,
            start_mm: point(start_mm),
            end_mm: point(end_mm),
        },
        SketchEntity::Arc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
            ..
        } => SketchEntity::Arc {
            id,
            start_mm: point(start_mm),
            end_mm: point(end_mm),
            center_mm: point(center_mm),
            clockwise,
        },
        SketchEntity::Circle {
            center_mm,
            radius_mm,
            ..
        } => SketchEntity::Circle {
            id,
            center_mm: point(center_mm),
            radius_mm: uniform_scale * radius_mm,
        },
        SketchEntity::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } => SketchEntity::CubicBezier {
            id,
            start_mm: point(start_mm),
            control_1_mm: point(control_1_mm),
            control_2_mm: point(control_2_mm),
            end_mm: point(end_mm),
        },
    }
}

pub(crate) fn assistant_sketch_entities(entity: &AssistantSketchEntity) -> Vec<SketchEntity> {
    if let AssistantSketchEntity::ProfileCopies {
        source_entities,
        copies,
    } = entity
    {
        let source = source_entities
            .iter()
            .flat_map(assistant_sketch_entities)
            .collect::<Vec<_>>();
        return copies
            .iter()
            .flat_map(|copy| {
                source
                    .iter()
                    .cloned()
                    .zip(copy.entity_ids.iter().copied())
                    .map(|(entity, id)| {
                        transformed_profile_entity(
                            entity,
                            id,
                            copy.translation_mm,
                            copy.rotation_degrees,
                            copy.uniform_scale,
                        )
                    })
            })
            .collect();
    }

    if let AssistantSketchEntity::Ellipse {
        segment_ids,
        center_mm,
        radius_x_mm,
        radius_y_mm,
        rotation_degrees,
        maximum_deviation_mm: _,
    } = entity
    {
        let angle = rotation_degrees.to_radians();
        let (sin, cos) = angle.sin_cos();
        let point = |local: [f64; 2]| {
            [
                center_mm[0] + cos * local[0] - sin * local[1],
                center_mm[1] + sin * local[0] + cos * local[1],
            ]
        };
        let kappa = 4.0 * (2.0_f64.sqrt() - 1.0) / 3.0;
        let rx = *radius_x_mm;
        let ry = *radius_y_mm;
        let points = [
            [[rx, 0.0], [rx, kappa * ry], [kappa * rx, ry], [0.0, ry]],
            [[0.0, ry], [-kappa * rx, ry], [-rx, kappa * ry], [-rx, 0.0]],
            [
                [-rx, 0.0],
                [-rx, -kappa * ry],
                [-kappa * rx, -ry],
                [0.0, -ry],
            ],
            [[0.0, -ry], [kappa * rx, -ry], [rx, -kappa * ry], [rx, 0.0]],
        ];
        return points
            .into_iter()
            .zip(segment_ids)
            .map(|(points, id)| SketchEntity::CubicBezier {
                id: SketchEntityId(*id),
                start_mm: point(points[0]),
                control_1_mm: point(points[1]),
                control_2_mm: point(points[2]),
                end_mm: point(points[3]),
            })
            .collect();
    }

    if let AssistantSketchEntity::RoundedRectangle {
        segment_ids,
        center_mm,
        width_mm,
        height_mm,
        corner_radius_mm,
        rotation_degrees,
    } = entity
    {
        let angle = rotation_degrees.to_radians();
        let (sin, cos) = angle.sin_cos();
        let point = |local: [f64; 2]| {
            [
                center_mm[0] + cos * local[0] - sin * local[1],
                center_mm[1] + sin * local[0] + cos * local[1],
            ]
        };
        let half_width = 0.5 * width_mm;
        let half_height = 0.5 * height_mm;
        let radius = *corner_radius_mm;
        let [
            bottom,
            bottom_right,
            right,
            top_right,
            top,
            top_left,
            left,
            bottom_left,
        ] = *segment_ids;
        return vec![
            SketchEntity::Line {
                id: SketchEntityId(bottom),
                start_mm: point([-half_width + radius, -half_height]),
                end_mm: point([half_width - radius, -half_height]),
            },
            SketchEntity::Arc {
                id: SketchEntityId(bottom_right),
                start_mm: point([half_width - radius, -half_height]),
                end_mm: point([half_width, -half_height + radius]),
                center_mm: point([half_width - radius, -half_height + radius]),
                clockwise: false,
            },
            SketchEntity::Line {
                id: SketchEntityId(right),
                start_mm: point([half_width, -half_height + radius]),
                end_mm: point([half_width, half_height - radius]),
            },
            SketchEntity::Arc {
                id: SketchEntityId(top_right),
                start_mm: point([half_width, half_height - radius]),
                end_mm: point([half_width - radius, half_height]),
                center_mm: point([half_width - radius, half_height - radius]),
                clockwise: false,
            },
            SketchEntity::Line {
                id: SketchEntityId(top),
                start_mm: point([half_width - radius, half_height]),
                end_mm: point([-half_width + radius, half_height]),
            },
            SketchEntity::Arc {
                id: SketchEntityId(top_left),
                start_mm: point([-half_width + radius, half_height]),
                end_mm: point([-half_width, half_height - radius]),
                center_mm: point([-half_width + radius, half_height - radius]),
                clockwise: false,
            },
            SketchEntity::Line {
                id: SketchEntityId(left),
                start_mm: point([-half_width, half_height - radius]),
                end_mm: point([-half_width, -half_height + radius]),
            },
            SketchEntity::Arc {
                id: SketchEntityId(bottom_left),
                start_mm: point([-half_width, -half_height + radius]),
                end_mm: point([-half_width + radius, -half_height]),
                center_mm: point([-half_width + radius, -half_height + radius]),
                clockwise: false,
            },
        ];
    }

    vec![match entity {
        AssistantSketchEntity::Line {
            id,
            start_mm,
            end_mm,
        } => SketchEntity::Line {
            id: SketchEntityId(*id),
            start_mm: *start_mm,
            end_mm: *end_mm,
        },
        AssistantSketchEntity::Arc {
            id,
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => SketchEntity::Arc {
            id: SketchEntityId(*id),
            start_mm: *start_mm,
            end_mm: *end_mm,
            center_mm: *center_mm,
            clockwise: *clockwise,
        },
        AssistantSketchEntity::Circle {
            id,
            center_mm,
            radius_mm,
        } => SketchEntity::Circle {
            id: SketchEntityId(*id),
            center_mm: *center_mm,
            radius_mm: *radius_mm,
        },
        AssistantSketchEntity::CubicBezier {
            id,
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => SketchEntity::CubicBezier {
            id: SketchEntityId(*id),
            start_mm: *start_mm,
            control_1_mm: *control_1_mm,
            control_2_mm: *control_2_mm,
            end_mm: *end_mm,
        },
        AssistantSketchEntity::Ellipse { .. } => unreachable!("ellipse handled above"),
        AssistantSketchEntity::RoundedRectangle { .. } => {
            unreachable!("rounded rectangle handled above")
        }
        AssistantSketchEntity::ProfileCopies { .. } => {
            unreachable!("profile copies handled above")
        }
    }]
}

pub(crate) fn assistant_sketch_constraint(
    constraint: &AssistantSketchConstraint,
) -> Result<SketchConstraint, CanonicalError> {
    let (id, kind) = match constraint {
        AssistantSketchConstraint::Horizontal { id, entity_id } => (
            *id,
            SketchConstraintKind::Horizontal {
                entity: SketchEntityId(*entity_id),
            },
        ),
        AssistantSketchConstraint::Vertical { id, entity_id } => (
            *id,
            SketchConstraintKind::Vertical {
                entity: SketchEntityId(*entity_id),
            },
        ),
        AssistantSketchConstraint::Coincident { id, a, b } => (
            *id,
            SketchConstraintKind::Coincident {
                a: assistant_sketch_point_ref(*a),
                b: assistant_sketch_point_ref(*b),
            },
        ),
        AssistantSketchConstraint::Distance { id, a, b, value_mm } => (
            *id,
            SketchConstraintKind::Distance {
                a: assistant_sketch_point_ref(*a),
                b: assistant_sketch_point_ref(*b),
                value: Dimension::new(value_mm.to_string(), *value_mm)?,
            },
        ),
        AssistantSketchConstraint::Radius {
            id,
            entity_id,
            value_mm,
        } => (
            *id,
            SketchConstraintKind::Radius {
                entity: SketchEntityId(*entity_id),
                value: Dimension::new(value_mm.to_string(), *value_mm)?,
            },
        ),
        AssistantSketchConstraint::FixedPoint {
            id,
            point,
            position_mm,
        } => (
            *id,
            SketchConstraintKind::FixedPoint {
                point: assistant_sketch_point_ref(*point),
                position_mm: *position_mm,
            },
        ),
        AssistantSketchConstraint::Parallel {
            id,
            a_entity_id,
            b_entity_id,
        } => (
            *id,
            SketchConstraintKind::Parallel {
                a: SketchEntityId(*a_entity_id),
                b: SketchEntityId(*b_entity_id),
            },
        ),
        AssistantSketchConstraint::Perpendicular {
            id,
            a_entity_id,
            b_entity_id,
        } => (
            *id,
            SketchConstraintKind::Perpendicular {
                a: SketchEntityId(*a_entity_id),
                b: SketchEntityId(*b_entity_id),
            },
        ),
        AssistantSketchConstraint::Tangent {
            id,
            a_entity_id,
            b_entity_id,
        } => (
            *id,
            SketchConstraintKind::Tangent {
                a: SketchEntityId(*a_entity_id),
                b: SketchEntityId(*b_entity_id),
            },
        ),
        AssistantSketchConstraint::Angle {
            id,
            a_entity_id,
            b_entity_id,
            angle_degrees,
        } => (
            *id,
            SketchConstraintKind::Angle {
                a: SketchEntityId(*a_entity_id),
                b: SketchEntityId(*b_entity_id),
                angle_degrees: *angle_degrees,
            },
        ),
        AssistantSketchConstraint::Equal {
            id,
            a_entity_id,
            b_entity_id,
        } => (
            *id,
            SketchConstraintKind::Equal {
                a: SketchEntityId(*a_entity_id),
                b: SketchEntityId(*b_entity_id),
            },
        ),
        AssistantSketchConstraint::Symmetric {
            id,
            a,
            b,
            axis_entity_id,
        } => (
            *id,
            SketchConstraintKind::Symmetric {
                a: assistant_sketch_point_ref(*a),
                b: assistant_sketch_point_ref(*b),
                axis: SketchEntityId(*axis_entity_id),
            },
        ),
        AssistantSketchConstraint::Concentric {
            id,
            a_entity_id,
            b_entity_id,
        } => (
            *id,
            SketchConstraintKind::Concentric {
                a: SketchEntityId(*a_entity_id),
                b: SketchEntityId(*b_entity_id),
            },
        ),
        AssistantSketchConstraint::Collinear {
            id,
            a_entity_id,
            b_entity_id,
        } => (
            *id,
            SketchConstraintKind::Collinear {
                a: SketchEntityId(*a_entity_id),
                b: SketchEntityId(*b_entity_id),
            },
        ),
        AssistantSketchConstraint::Midpoint {
            id,
            point,
            line_entity_id,
        } => (
            *id,
            SketchConstraintKind::Midpoint {
                point: assistant_sketch_point_ref(*point),
                line: SketchEntityId(*line_entity_id),
            },
        ),
        AssistantSketchConstraint::PointOnCurve {
            id,
            point,
            curve_entity_id,
        } => (
            *id,
            SketchConstraintKind::PointOnCurve {
                point: assistant_sketch_point_ref(*point),
                curve: SketchEntityId(*curve_entity_id),
            },
        ),
    };
    Ok(SketchConstraint {
        id: SketchConstraintId(id),
        kind,
    })
}
