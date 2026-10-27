//! Where a drawing stands in space. A plan lies in the ground plane; a front, back, left or
//! right view stands on a vertical plane, the drawing's X across it as a viewer looking at
//! that side sees it and the drawing's Y up. The underlay's placement (a turn, scale and
//! move in its own plane) aligns the drawing first; its view then sets it up, `at_mm` along
//! the plane's normal: a plan's height, a view's distance from the origin.

use crate::document::{CanonicalCommand, CommandBatch, Snapshot, Transform};
use crate::tolerance::MAX_COORDINATE_MM;
use ketchup_geometry::linalg::cross;

use super::{Underlay, UnderlayId};

/// The side a drawing shows.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ViewPlane {
    /// Seen from above: X right, Y away.
    #[default]
    Top,
    /// Seen from −Y: X right, Z up.
    Front,
    /// Seen from +Y: −X right, Z up.
    Back,
    /// Seen from −X: −Y right, Z up.
    Left,
    /// Seen from +X: Y right, Z up.
    Right,
}

impl ViewPlane {
    pub const ALL: [Self; 5] = [Self::Top, Self::Front, Self::Back, Self::Left, Self::Right];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Front => "front",
            Self::Back => "back",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|plane| plane.name() == name)
    }

    /// The world directions of the drawing's X and Y, and the world axis (0, 1 or 2) a
    /// drawing on this plane is seen along.
    #[must_use]
    pub const fn axes(self) -> ([f64; 3], [f64; 3], usize) {
        match self {
            Self::Top => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 2),
            Self::Front => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], 1),
            Self::Back => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], 1),
            Self::Left => ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], 0),
            Self::Right => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 0),
        }
    }
}

/// The plane an underlay's drawing stands on.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UnderlayView {
    pub plane: ViewPlane,
    /// Where the plane lies along the axis it is seen along, in millimetres.
    pub at_mm: f64,
}

impl UnderlayView {
    /// A plan in the ground plane: what every drawing is until it is set up.
    #[must_use]
    pub fn is_ground(&self) -> bool {
        *self == Self::default()
    }

    /// Whether the document can hold it: a finite position within the model's range.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.at_mm.is_finite() && self.at_mm.abs() <= MAX_COORDINATE_MM
    }

    /// The map of the drawing's plane, as its placement leaves it, onto the view's plane:
    /// rigid and right-handed, the drawing's +Z towards the viewer.
    #[must_use]
    pub fn transform(&self) -> Transform {
        let (x, y, axis) = self.plane.axes();
        let z = cross(x, y);
        let mut origin = [0.0; 3];
        origin[axis] = self.at_mm;
        let row = |i: usize| [x[i], y[i], z[i], origin[i]];
        let [a, b, c] = [row(0), row(1), row(2)];
        Transform::from_matrix([
            a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3], c[0], c[1], c[2], c[3], 0.0, 0.0, 0.0,
            1.0,
        ])
        .expect("a view's map is finite")
    }
}

impl Underlay {
    /// Where its lines lie in the world: the placement, then the view.
    #[must_use]
    pub fn world_transform(&self) -> Transform {
        if self.view.is_ground() {
            self.placement
        } else {
            self.view.transform().compose(self.placement)
        }
    }

    /// Whether it lies in the ground plane, where the window's tools pick, snap and draw.
    #[must_use]
    pub fn lies_flat(&self) -> bool {
        self.view.is_ground()
    }
}

/// Why a view does not set up an underlay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnderlayViewError {
    NotFound(UnderlayId),
    OutOfRange(f64),
}

impl std::fmt::Display for UnderlayViewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(formatter, "the document has no underlay {}", id.0),
            Self::OutOfRange(at) => write!(
                formatter,
                "a view stands at most {MAX_COORDINATE_MM} mm from the origin, not {at} mm"
            ),
        }
    }
}

impl std::error::Error for UnderlayViewError {}

/// One batch, so one Undo step, that sets the underlay `id` up as `view`.
pub fn plan_underlay_view(
    snapshot: &Snapshot,
    id: UnderlayId,
    view: UnderlayView,
) -> Result<CommandBatch, UnderlayViewError> {
    let underlay = snapshot
        .underlay(id)
        .ok_or(UnderlayViewError::NotFound(id))?;
    if !view.is_valid() {
        return Err(UnderlayViewError::OutOfRange(view.at_mm));
    }
    Ok(CommandBatch::new(vec![CanonicalCommand::UpsertUnderlay(
        Underlay {
            view,
            ..underlay.clone()
        },
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_view_puts_the_drawing_x_right_and_y_up_as_its_viewer_sees_it() {
        let at = |plane, point: [f64; 2]| {
            UnderlayView { plane, at_mm: 50.0 }
                .transform()
                .transform_point([point[0], point[1], 0.0])
        };
        assert_eq!(at(ViewPlane::Top, [3.0, 4.0]), [3.0, 4.0, 50.0]);
        assert_eq!(at(ViewPlane::Front, [3.0, 4.0]), [3.0, 50.0, 4.0]);
        assert_eq!(at(ViewPlane::Back, [3.0, 4.0]), [-3.0, 50.0, 4.0]);
        assert_eq!(at(ViewPlane::Left, [3.0, 4.0]), [50.0, -3.0, 4.0]);
        assert_eq!(at(ViewPlane::Right, [3.0, 4.0]), [50.0, 3.0, 4.0]);
        for plane in ViewPlane::ALL {
            let view = UnderlayView { plane, at_mm: 0.0 };
            assert!(view.transform().is_rigid(), "{plane:?}");
            assert!(view.transform().affine().determinant() > 0.0, "{plane:?}");
            assert_eq!(ViewPlane::from_name(plane.name()), Some(plane));
        }
    }

    #[test]
    fn a_stood_view_is_saved_and_read_back_and_a_plan_writes_nothing() {
        let view = UnderlayView {
            plane: ViewPlane::Left,
            at_mm: -120.5,
        };
        let written = serde_json::to_string(&view).unwrap();
        assert_eq!(
            serde_json::from_str::<UnderlayView>(&written).unwrap(),
            view
        );
        assert!(UnderlayView::default().is_ground());
        assert!(!view.is_ground());
        assert!(
            !UnderlayView {
                plane: ViewPlane::Top,
                at_mm: f64::NAN
            }
            .is_valid()
        );
    }
}
