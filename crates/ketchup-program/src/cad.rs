//! Converts an evaluated model into canonical panel creation requests that
//! the application planner turns into one undoable command batch.

use crate::model::{Part, ProgramModel};
use ketchup_core::assistant_sidecar::{
    AssistantCadEditOperation, AssistantPanelHole, AssistantPanelPocket,
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

/// One `create_panel` operation per part, in program order.
#[must_use]
pub fn panel_operations(model: &ProgramModel) -> Vec<AssistantCadEditOperation> {
    model.parts.iter().map(panel).collect()
}
