//! The openings of a blank: the holes of its outline, cut or drilled through it.

use super::*;

/// The cuts through a blank of the openings its outline has, each with the
/// extent it spans: each hole of a region extruded as the stock, cut along the
/// whole extrusion. A round hole is drilled, as a machine makes it, to the
/// depth of the blank: its interval is the extrusion, a machine depth, not
/// the cutter of a through-all hole running past the part (review 2026-11-04
/// r19 P3).
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
            let (machining, [low, high]) = match hole {
                ExactBRepPlanarLoop::Circle {
                    center_bits,
                    radius_bits,
                } => {
                    let center_mm = center_bits.map(f64::from_bits);
                    let radius = f64::from_bits(*radius_bits);
                    let drill = GeneralMachiningGeometry::CircularDrill {
                        frame,
                        center_mm,
                        diameter_mm: radius * 2.0,
                        through: false,
                        start_mm,
                        end_mm,
                    };
                    (
                        drill,
                        [
                            center_mm.map(|at| at - radius),
                            center_mm.map(|at| at + radius),
                        ],
                    )
                }
                ExactBRepPlanarLoop::Boundary { segments } => {
                    let segments = machining_segments(segments)?;
                    let extent = profile_bounds(&segments)?;
                    let cut = GeneralMachiningGeometry::ProfileCut {
                        frame,
                        segments,
                        start_mm,
                        end_mm,
                    };
                    (cut, extent)
                }
            };
            Some((
                machining,
                opening_extent(frame, [low, high], start_mm, end_mm)?,
            ))
        })
        .collect()
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
