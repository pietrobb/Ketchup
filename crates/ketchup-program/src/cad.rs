//! Converts an evaluated model into canonical part creation requests that
//! the application planner turns into one undoable command batch.
//! The operations carry only the part's origin; the planner places each
//! occurrence with the part's exact frame (`Part::transform_matrix`).

use crate::model::{Part, ProgramModel, ProgramOperation, ProgramPartBody, ProgramProfileSegment};
use ketchup_assistant::sidecar::{
    AssistantAxisSpec, AssistantCadEditOperation, AssistantCadPartFeature, AssistantPartHole,
    AssistantPartPocket, AssistantPrincipalPlane, AssistantSketchEntity, AssistantWorkplaneSpec,
};

/// Sketch entities of a closed program profile, numbered from 1.
#[must_use]
pub fn profile_entities(segments: &[ProgramProfileSegment]) -> Vec<AssistantSketchEntity> {
    segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let id = u64::try_from(index + 1).expect("bounded program profile");
            if let Some([control_1_mm, control_2_mm]) = segment.bezier {
                return AssistantSketchEntity::CubicBezier {
                    id,
                    start_mm: segment.start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm: segment.end_mm,
                };
            }
            match segment.arc {
                None => AssistantSketchEntity::Line {
                    id,
                    start_mm: segment.start_mm,
                    end_mm: segment.end_mm,
                },
                Some(arc) => AssistantSketchEntity::Arc {
                    id,
                    start_mm: segment.start_mm,
                    end_mm: segment.end_mm,
                    center_mm: arc.center_mm,
                    clockwise: arc.clockwise,
                },
            }
        })
        .collect()
}

/// A part's body, plus the holes and pockets it drills itself
/// (`Part::panel_machining`); the planner subtracts its other operations in
/// program order.
pub fn part(part: &Part) -> AssistantCadEditOperation {
    let drilled = &part.operations[..part.panel_machining()];
    let holes = drilled
        .iter()
        .filter_map(|operation| match operation {
            ProgramOperation::Hole(hole) => Some(AssistantPartHole {
                id: hole.id.clone(),
                entry_local_mm: hole.entry_mm,
                inward_unit_local: hole.inward,
                diameter_mm: hole.diameter_mm,
                depth_mm: hole.depth_mm,
                through: hole.through,
            }),
            _ => None,
        })
        .collect();
    let pockets = drilled
        .iter()
        .filter_map(|operation| match operation {
            ProgramOperation::Pocket(pocket) => {
                let (min, max) = pocket.local_box();
                Some(AssistantPartPocket {
                    id: pocket.id.clone(),
                    min_local_mm: min,
                    max_local_mm: max,
                    inward_unit_local: pocket.inward,
                })
            }
            _ => None,
        })
        .collect();
    let (segments, feature) = match &part.body {
        ProgramPartBody::Extrusion {
            segments,
            distance_mm,
            ..
        } => (
            segments,
            AssistantCadPartFeature::Extrusion {
                distance_mm: *distance_mm,
            },
        ),
        ProgramPartBody::Revolve {
            segments,
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
        } => (
            segments,
            AssistantCadPartFeature::Revolve {
                axis: AssistantAxisSpec::TwoPoints {
                    start_mm: [axis_start_mm[0], axis_start_mm[1], 0.0],
                    end_mm: [axis_end_mm[0], axis_end_mm[1], 0.0],
                },
                angle_degrees: *angle_degrees,
            },
        ),
        // The assistant schema has no swept or lofted part: the planner
        // replaces this profile and one-millimetre base with the real body
        // (`plan_rule_part_batch`).
        ProgramPartBody::Sweep { segments, .. } => (
            segments,
            AssistantCadPartFeature::Extrusion { distance_mm: 1.0 },
        ),
        ProgramPartBody::Loft { sections } => (
            &sections[0].segments,
            AssistantCadPartFeature::Extrusion { distance_mm: 1.0 },
        ),
    };
    AssistantCadEditOperation::CreatePart {
        name: part.name.clone(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: profile_entities(segments),
        constraints: Vec::new(),
        feature,
        holes,
        pockets,
        translation_mm: part.at_mm,
        rotation: None,
    }
}

/// One independent part creation operation per part, in program order.
#[must_use]
pub fn part_operations(model: &ProgramModel) -> Vec<AssistantCadEditOperation> {
    model.parts.iter().map(part).collect()
}
