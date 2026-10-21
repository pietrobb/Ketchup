//! Where an underlay lies in the document: its placement, a [`Transform`] as an occurrence's
//! is, that turns about Z, scales uniformly and moves in the XY plane; and the two-point fit
//! that places it so two points picked on its lines land on two points of the model.

use std::fmt;

use ketchup_geometry::linalg::{cross2, dot2};

use super::{Underlay, UnderlayId};
use crate::document::{CanonicalCommand, CanonicalError, CommandBatch, Snapshot, Transform};
use crate::tolerance::PICK_MATCH_MM;

pub(super) fn is_identity(placement: &Transform) -> bool {
    *placement == Transform::identity()
}

/// Whether `placement` maps the XY plane onto itself, invertibly: what an underlay's
/// placement must do, as its lines lie in that plane.
pub(crate) fn is_planar(placement: &Transform) -> bool {
    let matrix = placement.matrix();
    Transform::from_matrix(*matrix).is_ok()
        && [matrix[2], matrix[6], matrix[8], matrix[9], matrix[11]] == [0.0; 5]
        && matrix[10] == 1.0
        && placement.inverse().is_some()
}

fn in_plane(transform: Transform, point_mm: [f64; 2]) -> [f64; 2] {
    let [x, y, _] = transform.transform_point([point_mm[0], point_mm[1], 0.0]);
    [x, y]
}

impl Underlay {
    /// Where the point `local_mm` of its lines lies in the document.
    #[must_use]
    pub fn to_document(&self, local_mm: [f64; 2]) -> [f64; 2] {
        in_plane(self.placement, local_mm)
    }

    /// The point of its lines that lies at `document_mm` in the document.
    #[must_use]
    pub fn to_local(&self, document_mm: [f64; 2]) -> [f64; 2] {
        let inverse = self
            .placement
            .inverse()
            .expect("a document keeps only invertible underlay placements");
        in_plane(inverse, document_mm)
    }

    /// The millimetres of the document one millimetre of its lines spans.
    #[must_use]
    pub fn scale(&self) -> f64 {
        self.placement.affine().determinant().abs().sqrt()
    }
}

/// A two-point fit: the underlay's new placement, how it turned and scaled, and by how much
/// the second picked point misses its target along the line from the first, as the
/// distances between the two pairs differ and the fit does not scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnderlayFit {
    pub placement: Transform,
    pub turn_deg: f64,
    pub scale: f64,
    pub length_mismatch_mm: f64,
}

/// Why a fit, move or turn does not place an underlay.
#[derive(Clone, Debug, PartialEq)]
pub enum UnderlayFitError {
    NotFound(UnderlayId),
    /// A coordinate that is not a finite number.
    NotFinite,
    /// The two points picked on the underlay are the same point: they give no direction.
    UnderlayPointsCoincide,
    /// The two target points of the model are the same point.
    ModelPointsCoincide,
    /// The fitted placement is no transform of the document.
    Placement(CanonicalError),
    /// The edit would lift or tilt the underlay out of the ground plane.
    OffPlane,
}

impl fmt::Display for UnderlayFitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(formatter, "the document has no underlay {}", id.0),
            Self::NotFinite => formatter.write_str("a picked point is not a finite number"),
            Self::UnderlayPointsCoincide => write!(
                formatter,
                "the two points picked on the underlay are less than {PICK_MATCH_MM} mm apart"
            ),
            Self::ModelPointsCoincide => write!(
                formatter,
                "the two target points of the model are less than {PICK_MATCH_MM} mm apart"
            ),
            Self::Placement(error) => write!(formatter, "the fitted placement is invalid: {error}"),
            Self::OffPlane => formatter.write_str(
                "an underlay stays in the ground plane: it moves along X and Y and turns about Z only",
            ),
        }
    }
}

impl std::error::Error for UnderlayFitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Placement(error) => Some(error),
            _ => None,
        }
    }
}

/// The placement that moves the underlay placed by `placement` so that the document points
/// `from_mm`, two points of its lines where they are drawn now, land on `to_mm`: the first
/// exactly, the second on the line from it to its target, turned about Z. Only when
/// `scaled` does the fit also scale the underlay so that the second lands exactly.
pub fn fit_two_points(
    placement: Transform,
    from_mm: [[f64; 2]; 2],
    to_mm: [[f64; 2]; 2],
    scaled: bool,
) -> Result<UnderlayFit, UnderlayFitError> {
    if !from_mm
        .iter()
        .chain(&to_mm)
        .flatten()
        .all(|value| value.is_finite())
    {
        return Err(UnderlayFitError::NotFinite);
    }
    let along = |points: [[f64; 2]; 2]| [points[1][0] - points[0][0], points[1][1] - points[0][1]];
    let (from, to) = (along(from_mm), along(to_mm));
    let (from_length, to_length) = (from[0].hypot(from[1]), to[0].hypot(to[1]));
    if from_length < PICK_MATCH_MM {
        return Err(UnderlayFitError::UnderlayPointsCoincide);
    }
    if to_length < PICK_MATCH_MM {
        return Err(UnderlayFitError::ModelPointsCoincide);
    }
    // `to` divided by `from` as complex numbers: the turn, times the scale.
    let squared = from_length * from_length;
    let mut cos = dot2(from, to) / squared;
    let mut sin = cross2(from, to) / squared;
    let scale = if scaled {
        cos.hypot(sin)
    } else {
        let length = cos.hypot(sin);
        cos /= length;
        sin /= length;
        1.0
    };
    let edit = turn_onto(from_mm[0], to_mm[0], cos, sin).map_err(UnderlayFitError::Placement)?;
    Ok(UnderlayFit {
        placement: edit.compose(placement),
        turn_deg: sin.atan2(cos).to_degrees(),
        scale,
        length_mismatch_mm: to_length - scale * from_length,
    })
}

/// Moves `from` to the origin, turns it there by the angle of `cos` and `sin` (scaling by
/// their length) and moves it onto `to`.
fn turn_onto(
    from: [f64; 2],
    to: [f64; 2],
    cos: f64,
    sin: f64,
) -> Result<Transform, CanonicalError> {
    let turn = Transform::from_matrix([
        cos, -sin, 0.0, 0.0, sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])?;
    Ok(Transform::from_translation(to[0], to[1], 0.0)?
        .compose(turn)
        .compose(Transform::from_translation(-from[0], -from[1], 0.0)?))
}

/// The turn by `angle_degrees` about the vertical line through `centre_mm`, built in the
/// plane so it keeps an underlay's placement exactly planar.
pub fn planar_turn(centre_mm: [f64; 2], angle_degrees: f64) -> Result<Transform, CanonicalError> {
    let (sin, cos) = angle_degrees.to_radians().sin_cos();
    turn_onto(centre_mm, centre_mm, cos, sin)
}

/// One batch, so one Undo step, that places the underlay `id` by [`fit_two_points`].
pub fn plan_underlay_fit(
    snapshot: &Snapshot,
    id: UnderlayId,
    from_mm: [[f64; 2]; 2],
    to_mm: [[f64; 2]; 2],
    scaled: bool,
) -> Result<(CommandBatch, UnderlayFit), UnderlayFitError> {
    let underlay = snapshot
        .underlay(id)
        .ok_or(UnderlayFitError::NotFound(id))?;
    let fit = fit_two_points(underlay.placement, from_mm, to_mm, scaled)?;
    let batch = CommandBatch::new(vec![CanonicalCommand::UpsertUnderlay(Underlay {
        placement: fit.placement,
        ..underlay.clone()
    })]);
    Ok((batch, fit))
}

/// One batch, so one Undo step, that moves the underlay `id` by `edit`, a move or turn of
/// the document; an edit that would take it out of the ground plane is refused.
pub fn plan_underlay_edit(
    snapshot: &Snapshot,
    id: UnderlayId,
    edit: Transform,
) -> Result<CommandBatch, UnderlayFitError> {
    let underlay = snapshot
        .underlay(id)
        .ok_or(UnderlayFitError::NotFound(id))?;
    let placement = edit.compose(underlay.placement);
    if !is_planar(&placement) {
        return Err(UnderlayFitError::OffPlane);
    }
    Ok(CommandBatch::new(vec![CanonicalCommand::UpsertUnderlay(
        Underlay {
            placement,
            ..underlay.clone()
        },
    )]))
}
