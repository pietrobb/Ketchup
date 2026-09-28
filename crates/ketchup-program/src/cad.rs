//! Converts an evaluated model into canonical panel creation requests that
//! the application planner turns into one undoable command batch.
//! The operations carry only the part's origin; the planner places each
//! occurrence with the part's exact frame (`Part::transform_matrix`).

use crate::model::{Part, ProgramModel, ProgramPartBody};
use ketchup_core::assistant_sidecar::{
    AssistantAxisSpec, AssistantCadEditOperation, AssistantCadPartFeature, AssistantPanelHole,
    AssistantPanelPocket, AssistantPrincipalPlane, AssistantSketchEntity, AssistantWorkplaneSpec,
};

fn panel(part: &Part) -> AssistantCadEditOperation {
    let holes = part
        .holes
        .iter()
        .map(|hole| AssistantPanelHole {
            id: hole.id.clone(),
            entry_local_mm: hole.face.local_point(part.size_mm, hole.u_mm, hole.v_mm),
            inward_unit_local: hole.face.inward(),
            diameter_mm: hole.diameter_mm,
            depth_mm: hole.depth_mm,
        })
        .collect();
    let pockets = part
        .pockets
        .iter()
        .map(|pocket| {
            let (min, max) = pocket.local_box(part.size_mm);
            AssistantPanelPocket {
                id: pocket.id.clone(),
                min_local_mm: min,
                max_local_mm: max,
                inward_unit_local: pocket.face.inward(),
            }
        })
        .collect();
    AssistantCadEditOperation::CreatePanel {
        name: part.name.clone(),
        dimensions_mm: part.size_mm,
        holes,
        pockets,
        translation_mm: part.at_mm,
        rotation: None,
    }
}

pub fn part(part: &Part) -> AssistantCadEditOperation {
    let (segments, feature) = match &part.body {
        ProgramPartBody::Panel => return panel(part),
        ProgramPartBody::Extrusion {
            segments,
            distance_mm,
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
    };
    AssistantCadEditOperation::CreatePart {
        name: part.name.clone(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: segments
            .iter()
            .enumerate()
            .map(|(index, segment)| AssistantSketchEntity::Line {
                id: u64::try_from(index + 1).expect("bounded program profile"),
                start_mm: segment.start_mm,
                end_mm: segment.end_mm,
            })
            .collect(),
        constraints: Vec::new(),
        feature,
        translation_mm: part.at_mm,
        rotation: None,
    }
}

/// One independent part creation operation per part, in program order.
#[must_use]
pub fn part_operations(model: &ProgramModel) -> Vec<AssistantCadEditOperation> {
    model.parts.iter().map(part).collect()
}
