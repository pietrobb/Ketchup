//! Draft analysis of exact faces against a pull direction, for `query kind=faces`.
//! Only planes, cylinders and cones are judged, from their exact evidence; any
//! other face is `not_analyzed` with its reason, never estimated.

use ketchup_geometry::linalg::{cross, dot, length, normalize};
use ketchup_model::exact_product::ExactBRepGraphFaceEvidence;
use ketchup_model::tolerance::DRAFT_VERTICAL_TOLERANCE_DEGREES;
use serde_json::{Value, json};

/// Smallest draft that counts as positive unless the query names one, in degrees.
pub const DEFAULT_MIN_DRAFT_DEGREES: f64 = 1.0;

/// A draft angle is measured from the pull direction's perpendicular plane: positive
/// when the face turns toward the pull, negative (an undercut) when it turns away.
#[derive(Clone, Copy, Debug)]
pub struct DraftQuery {
    direction: [f64; 3],
    min_draft_degrees: f64,
}

impl DraftQuery {
    /// `None` for a zero or non-finite direction, or a minimum draft that is not
    /// above the vertical tolerance and below 90 degrees.
    pub fn new(direction: [f64; 3], min_draft_degrees: Option<f64>) -> Option<Self> {
        let min_draft_degrees = min_draft_degrees.unwrap_or(DEFAULT_MIN_DRAFT_DEGREES);
        let valid =
            min_draft_degrees > DRAFT_VERTICAL_TOLERANCE_DEGREES && min_draft_degrees < 90.0;
        normalize(direction)
            .filter(|_| valid)
            .map(|direction| Self {
                direction,
                min_draft_degrees,
            })
    }

    pub fn summary(&self) -> Value {
        json!({"direction":self.direction,"min_draft_degrees":self.min_draft_degrees,
            "vertical_tolerance_degrees":DRAFT_VERTICAL_TOLERANCE_DEGREES,
            "coordinate_space":"definition_mm",
            "classes":["positive","insufficient","vertical","negative"]})
    }

    /// The draft of one face: the least and greatest angle over the face and its
    /// class, or `not_analyzed` with the reason.
    pub fn face(&self, evidence: &ExactBRepGraphFaceEvidence) -> Value {
        match (evidence.surface_kind.as_str(), evidence.cone) {
            ("plane", _) => self.analyzed("plane", self.angle(evidence.unit_normal), 0.0),
            ("cylinder", _) => match evidence.unit_axis_direction {
                // Every normal of a cylinder is square to its axis, so an axis at
                // `off` degrees from the pull bounds every draft by ±off.
                Some(axis) => self.revolution("cylinder", axis, |off| (0.0, off)),
                None => not_analyzed("cylinder", "unsupported_surface"),
            },
            // A cone's normal makes one angle with its axis, so along the pull the
            // draft is that angle everywhere on the face.
            (_, Some(cone)) => self.revolution("cone", cone.unit_axis_direction, |off| {
                (self.angle(cone.unit_normal), off)
            }),
            (surface, None) => not_analyzed(surface, "unsupported_surface"),
        }
    }

    fn revolution(
        &self,
        surface: &str,
        axis: [f64; 3],
        draft: impl FnOnce(f64) -> (f64, f64),
    ) -> Value {
        let off = length(cross(axis, self.direction))
            .min(1.0)
            .asin()
            .to_degrees();
        if off > DRAFT_VERTICAL_TOLERANCE_DEGREES {
            return not_analyzed(surface, "axis_not_along_direction");
        }
        let (angle, spread) = draft(off);
        self.analyzed(surface, angle, spread)
    }

    fn angle(&self, normal: [f64; 3]) -> f64 {
        dot(normal, self.direction)
            .clamp(-1.0, 1.0)
            .asin()
            .to_degrees()
    }

    fn analyzed(&self, surface: &str, angle: f64, spread: f64) -> Value {
        let (min, max) = (angle - spread, angle + spread);
        let class = if min >= -DRAFT_VERTICAL_TOLERANCE_DEGREES
            && max <= DRAFT_VERTICAL_TOLERANCE_DEGREES
        {
            "vertical"
        } else if angle < 0.0 {
            "negative"
        } else if angle < self.min_draft_degrees {
            "insufficient"
        } else {
            "positive"
        };
        json!({"status":"analyzed","surface":surface,
            "min_angle_degrees":min,"max_angle_degrees":max,"class":class})
    }
}

fn not_analyzed(surface: &str, reason: &str) -> Value {
    json!({"status":"not_analyzed","surface":surface,"reason":reason})
}
