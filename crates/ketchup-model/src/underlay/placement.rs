//! Where an underlay lies in the document: its placement, a [`Transform`] as an occurrence's
//! is, that turns about Z, scales uniformly and moves in the XY plane; and the two-point fit
//! that places it so two points picked on its lines land on two points of the model.

use std::fmt;

use ketchup_geometry::linalg::{cross2, dot2};

use super::{Underlay, UnderlayId};
use crate::document::{CanonicalCommand, CanonicalError, CommandBatch, Snapshot, Transform};
use crate::tolerance::{MAX_COORDINATE_MM, PICK_MATCH_MM, UNDERLAY_MAX_SCALE};

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

/// Whether `placement` scales by at most [`UNDERLAY_MAX_SCALE`] either way and keeps the
/// origin of the lines within the model's ±[`MAX_COORDINATE_MM`]: what the document holds
/// of an underlay (review 2026-10-24 MD-24), and so what a fit or move may plan.
pub(crate) fn within_bounds(placement: &Transform) -> bool {
    let matrix = placement.matrix();
    (1.0 / UNDERLAY_MAX_SCALE..=UNDERLAY_MAX_SCALE).contains(&placement_scale(placement))
        && [matrix[3], matrix[7]]
            .iter()
            .all(|value| value.abs() <= MAX_COORDINATE_MM)
}

fn placement_scale(placement: &Transform) -> f64 {
    placement.affine().determinant().abs().sqrt()
}

/// `placement` when it is [`within_bounds`].
fn in_range(placement: Transform) -> Result<Transform, UnderlayFitError> {
    if within_bounds(&placement) {
        Ok(placement)
    } else {
        let matrix = placement.matrix();
        Err(UnderlayFitError::OutOfRange {
            scale: placement_scale(&placement),
            origin_mm: [matrix[3], matrix[7]],
        })
    }
}

fn in_plane(transform: Transform, point_mm: [f64; 2]) -> [f64; 2] {
    let [x, y, _] = transform.transform_point([point_mm[0], point_mm[1], 0.0]);
    [x, y]
}

impl Underlay {
    /// Puts back what a file holds of it that the document no longer takes, and says what
    /// in English, one clause each: a placement out of range or off the plane goes to the
    /// origin unscaled, a view out of range to the ground plane, and edits the document
    /// cannot hold are dropped, the drawing shown as read. Out of range is no damage, so
    /// the file opens (review 2026-10-26 MD-43); a batch is still refused.
    pub(crate) fn reset_what_the_document_cannot_hold(&mut self) -> Vec<&'static str> {
        let mut reset = Vec::new();
        if !is_planar(&self.placement) || !within_bounds(&self.placement) {
            self.placement = Transform::identity();
            reset.push("it was placed out of range, so it lies at the origin unscaled");
        }
        if !self.view.is_valid() {
            self.view = super::view::UnderlayView::default();
            reset.push("its view lay out of range, so it stands in the ground plane");
        }
        if !self.edits.is_valid() {
            self.edits = super::edits::UnderlayEdits::default();
            reset.push("its drawing edits could not be kept, so it shows the drawing as read");
        }
        reset
    }

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

    /// The map of the plane from its lines to the document.
    #[must_use]
    pub fn plane_map(&self) -> super::objects::Affine {
        let matrix = self.placement.matrix();
        [
            [matrix[0], matrix[1], matrix[3]],
            [matrix[4], matrix[5], matrix[7]],
        ]
    }

    /// The millimetres of the document one millimetre of its lines spans.
    #[must_use]
    pub fn scale(&self) -> f64 {
        placement_scale(&self.placement)
    }
}

/// The first plan moved from its drawing's coordinates; a drawing standing as a view shows
/// no part of the national grid (review 2026-10-29 UL-60).
fn first_moved(snapshot: &Snapshot) -> Option<&Underlay> {
    snapshot
        .underlays()
        .filter(|underlay| underlay.view.plane == super::view::ViewPlane::Top)
        .filter(|underlay| underlay.offset_mm() != [0.0; 2])
        .min_by_key(|underlay| underlay.id.0)
}

/// What the first underlay of `snapshot` moved from its drawing's coordinates was moved
/// by: the offset a drawing read next moves by when it can
/// ([`super::UnderlayLines::move_near_origin`]), so drawings of one national grid lie on
/// each other however far apart they are (review 2026-10-26 UL-41).
#[must_use]
pub fn geo_origin(snapshot: &Snapshot) -> Option<[f64; 2]> {
    first_moved(snapshot).map(Underlay::offset_mm)
}

/// Where an underlay moved by `offset_mm` from its drawing's own coordinates lies: by the
/// first underlay of `snapshot` that was moved too, and by the difference of the two
/// offsets from it, so that drawings in one national grid lie on each other wherever the
/// median of each moved it. Identity for a drawing not moved, the first one moved, or one
/// so far from the first that it would leave the model.
pub(super) fn geo_placement(snapshot: &Snapshot, offset_mm: [f64; 2]) -> Transform {
    let Some(first) = first_moved(snapshot) else {
        return Transform::identity();
    };
    if offset_mm == [0.0; 2] {
        return Transform::identity();
    }
    let [x, y] = [0, 1].map(|axis| offset_mm[axis] - first.offset_mm()[axis]);
    Transform::from_translation(x, y, 0.0)
        .ok()
        .and_then(|apart| in_range(first.placement.compose(apart)).ok())
        .unwrap_or_else(Transform::identity)
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
    /// The edit would lift or tilt the underlay out of the plane it stands on.
    OffPlane,
    /// The placement would scale the underlay more than [`UNDERLAY_MAX_SCALE`] either way
    /// or put its origin outside the model's ±[`MAX_COORDINATE_MM`].
    OutOfRange {
        scale: f64,
        origin_mm: [f64; 2],
    },
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
                "an underlay stays in the plane it stands on, the ground plane for a plan: \
                 it moves along that plane and turns about the axis it is seen along only",
            ),
            Self::OutOfRange { scale, origin_mm } => write!(
                formatter,
                "the placement would scale the underlay by {scale} and put its origin at \
                 ({}, {}) mm; an underlay scales by at most {UNDERLAY_MAX_SCALE} either way \
                 and its origin stays within ±{MAX_COORDINATE_MM} mm, so check the unit of \
                 the points",
                origin_mm[0], origin_mm[1]
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
        placement: in_range(edit.compose(placement))?,
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

/// The turn by `angle_degrees` about the world axis `axis` (0, 1 or 2) through
/// `centre_mm`, built entry by entry so that a drawing standing across that axis keeps its
/// placement exactly planar: the axis's row and column are exactly those of the identity.
pub fn axis_turn(
    centre_mm: [f64; 3],
    axis: usize,
    angle_degrees: f64,
) -> Result<Transform, CanonicalError> {
    if axis == 2 {
        return planar_turn([centre_mm[0], centre_mm[1]], angle_degrees);
    }
    let (sin, cos) = angle_degrees.to_radians().sin_cos();
    // The two other axes, in the order a right-handed turn about `axis` takes them.
    let (i, j) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut matrix = [0.0; 16];
    matrix[axis * 4 + axis] = 1.0;
    matrix[15] = 1.0;
    matrix[i * 4 + i] = cos;
    matrix[i * 4 + j] = -sin;
    matrix[j * 4 + i] = sin;
    matrix[j * 4 + j] = cos;
    matrix[i * 4 + 3] = centre_mm[i] - (cos * centre_mm[i] - sin * centre_mm[j]);
    matrix[j * 4 + 3] = centre_mm[j] - (sin * centre_mm[i] + cos * centre_mm[j]);
    Transform::from_matrix(matrix)
}

impl Underlay {
    /// The world axis (0, 1 or 2) it is seen along, across its plane.
    #[must_use]
    pub fn seen_along(&self) -> usize {
        self.view.plane.axes().2
    }

    /// The part of the world move `delta_mm` that lies in its plane: a point picked off the
    /// plane moves it to where that point lies on it.
    #[must_use]
    pub fn in_its_plane(&self, mut delta_mm: [f64; 3]) -> [f64; 3] {
        delta_mm[self.seen_along()] = 0.0;
        delta_mm
    }
}

/// One batch, so one Undo step, that moves the underlay `id` by `edit`, a move or turn of
/// the document, in the plane it stands on, a plan's or a view's (review 2026-10-29 UO-69);
/// an edit that would lift or tilt it out of that plane is refused.
pub fn plan_underlay_edit(
    snapshot: &Snapshot,
    id: UnderlayId,
    edit: Transform,
) -> Result<CommandBatch, UnderlayFitError> {
    let underlay = snapshot
        .underlay(id)
        .ok_or(UnderlayFitError::NotFound(id))?;
    let view = underlay.view.transform();
    let inverse = view.inverse().expect("a view's map is rigid");
    let placement = inverse
        .compose(edit)
        .compose(view)
        .compose(underlay.placement);
    if !is_planar(&placement) {
        return Err(UnderlayFitError::OffPlane);
    }
    Ok(CommandBatch::new(vec![CanonicalCommand::UpsertUnderlay(
        Underlay {
            placement: in_range(placement)?,
            ..underlay.clone()
        },
    )]))
}
