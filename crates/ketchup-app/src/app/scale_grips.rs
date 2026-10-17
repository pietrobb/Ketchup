//! Scale grips on the selection's world box, and what a value typed for Scale
//! asks for.
//!
//! A grip sits at a corner, an edge midpoint or a face centre of the box. It
//! stretches every world axis it is off-centre in: a corner all three, an
//! edge two, a face one. The pivot is the opposite grip, or the box centre
//! while Ctrl is held.

use crate::drawing_plane::DRAWING_SNAP_PIXELS;
use crate::length_input::{FactorOrLength, parse_factor_or_length};
use crate::*;

/// What a value typed into the Scale box asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ScaleInput {
    /// One factor for the axes the gesture or the axis lock drives (`1,5`).
    Factor(f64),
    /// One factor per world axis, typed `x;y;z` so a decimal comma stays free.
    PerAxis([f64; 3]),
    /// The size the one driven axis should reach, typed with a unit (`150 mm`).
    Length(f64),
}

impl ScaleInput {
    /// Read with the one length parser: arithmetic everywhere, and a value
    /// without a unit is a factor (`150` is 150×, `150 mm` a size).
    pub(crate) fn parse(text: &str) -> Option<Self> {
        if text.contains(';') {
            let factors: Vec<f64> = text
                .split(';')
                .map(|part| match parse_factor_or_length(part)? {
                    FactorOrLength::Factor(factor) => Some(factor),
                    FactorOrLength::LengthMm(_) => None,
                })
                .collect::<Option<_>>()?;
            return <[f64; 3]>::try_from(factors).ok().map(Self::PerAxis);
        }
        Some(match parse_factor_or_length(text)? {
            FactorOrLength::Factor(factor) => Self::Factor(factor),
            FactorOrLength::LengthMm(length_mm) => Self::Length(length_mm),
        })
    }

    /// The world factors for a gesture that drives `drives` on a selection
    /// `extent_mm` wide, or the catalog key naming why there are none.
    pub(crate) fn factors(
        self,
        drives: [bool; 3],
        extent_mm: [f64; 3],
    ) -> Result<[f64; 3], &'static str> {
        let factors = self.correction_factors(drives, extent_mm)?;
        if scale_is_meaningful(factors) {
            Ok(factors)
        } else {
            Err("digest-scale-invalid-factor")
        }
    }

    /// As [`Self::factors`], but the identity too: correcting the last Scale
    /// to `1` takes the part back to its own size.
    pub(crate) fn correction_factors(
        self,
        drives: [bool; 3],
        extent_mm: [f64; 3],
    ) -> Result<[f64; 3], &'static str> {
        let factors = match self {
            Self::Factor(factor) => drives.map(|driven| if driven { factor } else { 1.0 }),
            Self::PerAxis(factors) => factors,
            Self::Length(length_mm) => {
                let mut driven = (0..3).filter(|axis| drives[*axis]);
                let (Some(axis), None) = (driven.next(), driven.next()) else {
                    return Err("digest-scale-length-needs-one-axis");
                };
                let mut factors = [1.0; 3];
                factors[axis] = length_mm / extent_mm[axis];
                factors
            }
        };
        if factors.into_iter().all(scale_factor_in_range) {
            Ok(factors)
        } else {
            Err("digest-scale-invalid-factor")
        }
    }
}

/// The world axes a lock lets Scale stretch: one, or all three when free.
pub(crate) fn axis_drives(axis: Option<Axis>) -> [bool; 3] {
    match axis {
        Some(Axis::X) => [true, false, false],
        Some(Axis::Y) => [false, true, false],
        Some(Axis::Z) => [false, false, true],
        None => [true; 3],
    }
}

/// One grip: which side of the box centre it sits on along each world axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScaleGrip {
    side: [i8; 3],
}

impl ScaleGrip {
    /// The grips of `bounds`, leaving out those that would stretch an axis
    /// along which the box has no size.
    pub(crate) fn all(bounds: [Vec3; 2]) -> impl Iterator<Item = Self> {
        let extent = (bounds[1] - bounds[0]).to_array();
        (0..27)
            .map(|index| Self {
                side: [index % 3 - 1, index / 3 % 3 - 1, index / 9 - 1],
            })
            .filter(move |grip| {
                grip.side != [0; 3]
                    && (0..3)
                        .all(|axis| grip.side[axis] == 0 || extent[axis] >= limits::MIN_LENGTH_MM)
            })
    }

    pub(crate) fn drives(self) -> [bool; 3] {
        self.side.map(|side| side != 0)
    }

    pub(crate) fn point(self, bounds: [Vec3; 2]) -> Vec3 {
        Self::at(bounds, self.side.map(f64::from))
    }

    /// The pivot: the opposite grip, or the box centre for `about_centre`.
    pub(crate) fn pivot(self, bounds: [Vec3; 2], about_centre: bool) -> Vec3 {
        Self::at(
            bounds,
            self.side
                .map(|side| if about_centre { 0.0 } else { -f64::from(side) }),
        )
    }

    fn at(bounds: [Vec3; 2], side: [f64; 3]) -> Vec3 {
        let centre = ((bounds[0] + bounds[1]) * 0.5).to_array();
        let half = ((bounds[1] - bounds[0]) * 0.5).to_array();
        Vec3::from(std::array::from_fn(|axis| {
            centre[axis] + side[axis] * half[axis]
        }))
    }
}

impl KetchupApp {
    /// The world box of everything a Scale now would stretch.
    pub(crate) fn scale_bounds(&self) -> Option<[Vec3; 2]> {
        let snapshot = self.document.current();
        self.rotation_bounds_for(&scale_applies(
            &snapshot,
            self.selected_instance_paths(),
            self.selection.selected_group,
        ))
    }

    /// The grip under `pointer`, within the reach of a drawing snap.
    pub(crate) fn scale_grip_at(
        &self,
        pointer: Pos2,
        rect: Rect,
    ) -> Option<(ScaleGrip, [Vec3; 2])> {
        if self.active_tool != ActiveTool::Scale || self.scale_session().is_some() {
            return None;
        }
        let bounds = self.scale_bounds()?;
        ScaleGrip::all(bounds)
            .map(|grip| {
                (
                    grip,
                    pointer.distance(self.project(grip.point(bounds), rect)),
                )
            })
            .filter(|(_, distance)| *distance <= DRAWING_SNAP_PIXELS)
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(grip, _)| (grip, bounds))
    }

    /// Follows `pointer` with an open Scale gesture and reports its factors,
    /// leaving the value box alone while it is being `typing` into.
    pub(crate) fn advance_scale_session(
        &mut self,
        phase: ToolSessionPhase,
        mut drag: ScaleDrag,
        pointer: Pos2,
        typing: bool,
    ) {
        Self::advance_scale(&mut drag, pointer);
        let mut arguments = self.scale_digest_arguments(drag.factors);
        self.set_scale_session(phase, drag);
        self.digest = self.catalog.format("digest-scale-live", &arguments);
        if !typing && let Some(factor) = arguments.remove("factor") {
            self.value_box.input = factor;
        }
    }

    /// Paints the grips while Scale waits for a gesture, and the pivot while
    /// one is under way.
    pub(crate) fn paint_scale_grips(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        hover: Option<Pos2>,
    ) {
        if self.active_tool != ActiveTool::Scale {
            return;
        }
        let palette = self.palette();
        if let Some((drag, _)) = self.scale_session() {
            painter.circle_filled(drag.centre_screen, 4.0, palette.accent);
            return;
        }
        let Some(bounds) = self.scale_bounds() else {
            return;
        };
        let hovered = hover
            .and_then(|pointer| self.scale_grip_at(pointer, rect))
            .map(|(grip, _)| grip);
        for grip in ScaleGrip::all(bounds) {
            let centre = self.project(grip.point(bounds), rect);
            let square = Rect::from_center_size(centre, Vec2::splat(7.0));
            let fill = if hovered == Some(grip) {
                palette.accent
            } else {
                Color32::WHITE
            };
            painter.rect_filled(square, 1.0, fill);
            painter.rect_stroke(
                square,
                1.0,
                Stroke::new(1.2_f32, palette.accent),
                egui::StrokeKind::Middle,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScaleInput;

    /// Scale reads its value with the one length parser: a value without a unit is a
    /// factor, arithmetic included, and one with a unit the size to reach (UI-6).
    #[test]
    fn a_typed_scale_is_a_factor_or_a_length_with_arithmetic() {
        let parse = ScaleInput::parse;
        assert_eq!(parse("1,5"), Some(ScaleInput::Factor(1.5)));
        assert_eq!(parse("150"), Some(ScaleInput::Factor(150.0)));
        assert_eq!(parse("3/4"), Some(ScaleInput::Factor(0.75)));
        assert_eq!(parse("1 + 0,5"), Some(ScaleInput::Factor(1.5)));
        assert_eq!(parse("150 mm"), Some(ScaleInput::Length(150.0)));
        assert_eq!(parse("1m - 2*18mm"), Some(ScaleInput::Length(964.0)));
        assert_eq!(parse("60cm/2"), Some(ScaleInput::Length(300.0)));
        assert_eq!(
            parse("2*1,5; 1; 1/2"),
            Some(ScaleInput::PerAxis([3.0, 1.0, 0.5]))
        );
        for refused in ["", "abc", "1m*1m", "100mm;1;1", "1;2", "1,2,3", "2/0"] {
            assert_eq!(parse(refused), None, "{refused}");
        }
    }
}
