//! The openings of a blank: the holes of its outline, cut or drilled through it.

use super::*;

/// The cuts through a blank of the openings its outline has, each with the
/// extent it spans: each hole of a region extruded as the stock, cut along the
/// whole extrusion, a machine depth, not the cutter of a through-all hole
/// running past the part (review 2026-11-04 r19 P3). A round hole is its two
/// half arcs: whether it is drilled is for the tools a format is set up with
/// ([`as_tooled`]), not for its shape, as no drill makes the Ø450 opening
/// of a sink (review 2026-11-06 MF-181).
pub(super) fn opening_cuts(
    profile: &ExactBRepProfile,
    interval: ExactBRepLinearInterval,
) -> Option<Vec<(GeneralMachiningGeometry, PieceDimensions)>> {
    let ExactBRepPlanarGeometry::Region { holes, .. } = &profile.geometry else {
        return Some(Vec::new());
    };
    let frame = machining_frame(profile, interval.direction())?;
    let (start_mm, end_mm) = (interval.start_mm(), interval.end_mm());
    holes
        .iter()
        .map(|hole| {
            let segments = machining_segments(&loop_segments(hole))?;
            let [low, high] = match hole {
                ExactBRepPlanarLoop::Circle {
                    center_bits,
                    radius_bits,
                } => {
                    let center_mm = center_bits.map(f64::from_bits);
                    let radius = f64::from_bits(*radius_bits);
                    [
                        center_mm.map(|at| at - radius),
                        center_mm.map(|at| at + radius),
                    ]
                }
                ExactBRepPlanarLoop::Boundary { .. } => profile_bounds(&segments)?,
            };
            let cut = GeneralMachiningGeometry::ProfileCut {
                frame,
                segments,
                start_mm,
                end_mm,
            };
            Some((cut, opening_extent(frame, [low, high], start_mm, end_mm)?))
        })
        .collect()
}

/// The drilling a format set up with drills up to `largest_drill_mm` makes of
/// a round cut, or the cut a drilling larger than that drill is milled as:
/// its two half arcs (review 2026-11-06 MF-181). Any other machining is left
/// as it is.
pub(super) fn as_tooled(
    machining: &GeneralMachiningGeometry,
    largest_drill_mm: Option<f64>,
) -> Option<GeneralMachiningGeometry> {
    let drills = |diameter_mm: f64| largest_drill_mm.is_some_and(|largest| diameter_mm <= largest);
    match *machining {
        GeneralMachiningGeometry::ProfileCut {
            frame,
            start_mm,
            end_mm,
            ..
        } => {
            let (center_mm, diameter_mm) = round_contour(machining)?;
            drills(diameter_mm).then_some(GeneralMachiningGeometry::CircularDrill {
                frame,
                center_mm,
                diameter_mm,
                through: false,
                start_mm,
                end_mm,
            })
        }
        GeneralMachiningGeometry::CircularDrill {
            frame,
            center_mm,
            diameter_mm,
            through: false,
            start_mm,
            end_mm,
        } if largest_drill_mm.is_some() && !drills(diameter_mm) => {
            let radius_bits = (diameter_mm / 2.0).to_bits();
            let segments =
                machining_segments(&half_arcs(center_mm.map(f64::to_bits), radius_bits))?;
            Some(GeneralMachiningGeometry::ProfileCut {
                frame,
                segments,
                start_mm,
                end_mm,
            })
        }
        _ => None,
    }
}

/// The center and diameter of a cut whose contour is one circle, as two half
/// arcs about one center.
pub(super) fn round_contour(machining: &GeneralMachiningGeometry) -> Option<([f64; 2], f64)> {
    let GeneralMachiningGeometry::ProfileCut { segments, .. } = machining else {
        return None;
    };
    let [
        GeneralMachiningSegment::CircularArc {
            start_mm: first_start,
            end_mm: first_end,
            center_mm,
            clockwise,
        },
        GeneralMachiningSegment::CircularArc {
            start_mm: second_start,
            end_mm: second_end,
            center_mm: second_center,
            clockwise: second_clockwise,
        },
    ] = segments.as_slice()
    else {
        return None;
    };
    let distance = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    let radius = distance(*first_start, *center_mm);
    let close = |a: f64, b: f64| (a - b).abs() <= ROUNDING * radius.max(1.0);
    let round = center_mm == second_center
        && clockwise == second_clockwise
        && first_end == second_start
        && second_end == first_start
        && close(distance(*first_start, *first_end), 2.0 * radius)
        && close(distance(*first_end, *center_mm), radius);
    (round && radius > 0.0).then_some((*center_mm, 2.0 * radius))
}

/// The extent in the definition of the part of `frame` over the rectangle
/// `[low, high]` from `start_mm` to `end_mm` along its normal.
fn opening_extent(
    frame: GeneralMachiningFrame,
    [low, high]: [[f64; 2]; 2],
    start_mm: f64,
    end_mm: f64,
) -> Option<PieceDimensions> {
    let mut minimum = [f64::INFINITY; 3];
    let mut maximum = [f64::NEG_INFINITY; 3];
    for corner in 0..8_u8 {
        let pick =
            |bit: u8, first: f64, last: f64| if corner >> bit & 1 == 1 { last } else { first };
        let (x, y, along) = (
            pick(0, low[0], high[0]),
            pick(1, low[1], high[1]),
            pick(2, start_mm, end_mm),
        );
        for axis in 0..3 {
            let at = frame.origin_mm[axis]
                + frame.x_axis[axis] * x
                + frame.y_axis[axis] * y
                + frame.normal[axis] * along;
            minimum[axis] = minimum[axis].min(at);
            maximum[axis] = maximum[axis].max(at);
        }
    }
    piece_dimensions_from_bounds([minimum, maximum])
}

pub(super) fn loop_segments(planar_loop: &ExactBRepPlanarLoop) -> Vec<ExactBRepPlanarSegment> {
    match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => segments.clone(),
        ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        } => half_arcs(*center_bits, *radius_bits),
    }
}

/// A circle as its two half arcs, east to west and back.
pub(super) fn half_arcs(center_bits: [u64; 2], radius_bits: u64) -> Vec<ExactBRepPlanarSegment> {
    let [x, y] = center_bits.map(f64::from_bits);
    let radius = f64::from_bits(radius_bits);
    let [east, west] = [[x + radius, y], [x - radius, y]].map(|point| point.map(f64::to_bits));
    [(east, west), (west, east)]
        .into_iter()
        .map(
            |(start_bits, end_bits)| ExactBRepPlanarSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise: false,
            },
        )
        .collect()
}
