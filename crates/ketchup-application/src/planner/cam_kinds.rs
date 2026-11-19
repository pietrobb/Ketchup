//! The CAM kinds a client names, as the model keeps them.

use ketchup_assistant::sidecar::{AssistantCamToolKind, AssistantCamWorkOffset};
use ketchup_model::cam::{CamToolKind, CamWorkOffset};

pub(super) fn tool_kind(kind: AssistantCamToolKind) -> CamToolKind {
    match kind {
        AssistantCamToolKind::FlatEndMill => CamToolKind::FlatEndMill,
        AssistantCamToolKind::BallEndMill => CamToolKind::BallEndMill,
        AssistantCamToolKind::Drill => CamToolKind::Drill,
    }
}

pub(super) fn work_offset(offset: AssistantCamWorkOffset) -> CamWorkOffset {
    match offset {
        AssistantCamWorkOffset::G54 => CamWorkOffset::G54,
        AssistantCamWorkOffset::G55 => CamWorkOffset::G55,
        AssistantCamWorkOffset::G56 => CamWorkOffset::G56,
        AssistantCamWorkOffset::G57 => CamWorkOffset::G57,
        AssistantCamWorkOffset::G58 => CamWorkOffset::G58,
        AssistantCamWorkOffset::G59 => CamWorkOffset::G59,
    }
}
