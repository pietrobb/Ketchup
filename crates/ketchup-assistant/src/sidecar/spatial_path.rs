//! Assistant spatial line, arc and cubic segments in canonical model form.

use super::AssistantSpatialPathSegment;
use ketchup_model::document::SpatialPathSegment;

impl AssistantSpatialPathSegment {
    #[must_use]
    pub fn canonical(&self) -> SpatialPathSegment {
        match self {
            Self::Line { start_mm, end_mm } => SpatialPathSegment::Line {
                start_mm: *start_mm,
                end_mm: *end_mm,
            },
            Self::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                normal,
                clockwise,
            } => SpatialPathSegment::CircularArc {
                start_mm: *start_mm,
                end_mm: *end_mm,
                center_mm: *center_mm,
                normal: *normal,
                clockwise: *clockwise,
            },
            Self::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => SpatialPathSegment::CubicBezier {
                start_mm: *start_mm,
                control_1_mm: *control_1_mm,
                control_2_mm: *control_2_mm,
                end_mm: *end_mm,
            },
        }
    }
}
