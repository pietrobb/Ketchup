//! Entities flat in the plan: an extrusion of ±Z, and what one of -Z, as AutoCAD's
//! Mirror leaves it, does to the points of an object coordinate system.
use super::*;

/// How far an extrusion direction may be off ±Z and still be it: drawing programs
/// leave the rounding of a unit vector in it (review 2026-10-24 UL-22).
const EXTRUSION_TOLERANCE: f64 = ROUNDING;

/// The sign of the Z of an entity flat in the plan: no height and an extrusion of
/// ±Z.
pub(super) fn extrusion_sign(record: &[DxfPair<'_>]) -> Result<f64, DxfImportError> {
    let mut sign = 1.0;
    for pair in record {
        let off = match pair.code {
            30 | 31 | 32 | 33 | 38 | 39 => parse_number(pair.value)? != 0.0,
            210 | 220 => parse_number(pair.value)?.abs() > EXTRUSION_TOLERANCE,
            230 => {
                let z = parse_number(pair.value)?;
                sign = z.signum();
                (z.abs() - 1.0).abs() > EXTRUSION_TOLERANCE
            }
            _ => false,
        };
        if off {
            return Err(DxfImportError::NonPlanarGeometry);
        }
    }
    Ok(sign)
}

pub(super) fn ensure_planar(record: &[DxfPair<'_>]) -> Result<(), DxfImportError> {
    if extrusion_sign(record)? < 0.0 {
        return Err(DxfImportError::NonPlanarGeometry);
    }
    Ok(())
}

/// Like `ensure_planar` for an entity whose points are in world coordinates (a LINE,
/// an ELLIPSE, a SPLINE, an MTEXT): its extrusion only turns its thickness, so -Z
/// changes none of its points.
pub(super) fn ensure_planar_wcs(record: &[DxfPair<'_>]) -> Result<(), DxfImportError> {
    extrusion_sign(record).map(|_| ())
}

/// Like `ensure_planar` for an entity whose points are in its object coordinate
/// system: extrusion (0, 0, -1), which AutoCAD's Mirror leaves on arcs, circles,
/// polylines and texts, is accepted. Returns whether the entity is so mirrored.
pub(super) fn ensure_planar_ocs(record: &[DxfPair<'_>]) -> Result<bool, DxfImportError> {
    Ok(extrusion_sign(record)? < 0.0)
}

/// A segment from an object coordinate system with extrusion (0, 0, -1) in world
/// coordinates: the arbitrary axis algorithm turns its x axis to world -x.
pub(super) fn mirror_ocs(segment: ProfileSegment) -> ProfileSegment {
    // `0.0 - x` and not `-x`, so that 0 stays +0 and joins bit for bit.
    let flip = |point: [f64; 2]| [0.0 - point[0], point[1]];
    match segment {
        ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
            start_mm: flip(start_mm),
            end_mm: flip(end_mm),
        },
        ProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => ProfileSegment::CircularArc {
            start_mm: flip(start_mm),
            end_mm: flip(end_mm),
            center_mm: flip(center_mm),
            clockwise: !clockwise,
        },
        ProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => ProfileSegment::CubicBezier {
            start_mm: flip(start_mm),
            control_1_mm: flip(control_1_mm),
            control_2_mm: flip(control_2_mm),
            end_mm: flip(end_mm),
        },
        ProfileSegment::Spline { points_mm } => ProfileSegment::Spline {
            points_mm: points_mm.into_iter().map(flip).collect(),
        },
    }
}
