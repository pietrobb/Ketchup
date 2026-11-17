//! Assistant spatial line, arc and cubic segments in canonical model form.

use super::AssistantSpatialPathSegment;
use crate::request_invalid::AssistantRequestInvalid;
use ketchup_model::document::{SpatialPathSegment, is_valid_spatial_sweep_path};
use ketchup_tolerance::limits;

pub fn validated_spatial_path_segments(
    segments: &[AssistantSpatialPathSegment],
    tolerance_mm: f64,
) -> Result<Vec<SpatialPathSegment>, AssistantRequestInvalid> {
    if !(1..=limits::PATH_SEGMENTS).contains(&segments.len()) {
        return Err(AssistantRequestInvalid::invalid(
            "spatial path segment count",
        ));
    }
    let segments = segments
        .iter()
        .map(AssistantSpatialPathSegment::canonical)
        .collect::<Vec<_>>();
    if !is_valid_spatial_sweep_path(&segments, tolerance_mm) {
        return Err(AssistantRequestInvalid::invalid("spatial path"));
    }
    Ok(segments)
}

/// Whether a boundary surface has 2 to 64 loop curves and at most 64 inner
/// curves of a network; the model checks their geometry.
pub(super) fn network_counts_fit(
    segments: &[AssistantSpatialPathSegment],
    inner: &[AssistantSpatialPathSegment],
) -> bool {
    (2..=limits::PATH_SEGMENTS).contains(&segments.len()) && inner.len() <= limits::PATH_SEGMENTS
}

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
