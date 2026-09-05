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
        },
    }
}

pub(crate) fn assistant_sketch_entity(entity: &AssistantSketchEntity) -> SketchEntity {
    match entity {
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
    }
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
