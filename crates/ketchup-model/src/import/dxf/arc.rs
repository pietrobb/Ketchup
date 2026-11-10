//! ARC and CIRCLE entities of a drawing: an arc as a loose segment, a circle and an arc of a
//! whole turn as a closed profile.

use super::*;

pub(super) fn whole_turn_arc(record: &[DxfPair<'_>]) -> Result<bool, DxfImportError> {
    Ok(crate::underlay::drawn::whole_turn(
        parse_required_number(record, 50)?,
        parse_required_number(record, 51)?,
    ))
}

/// A CIRCLE, or an ARC of a whole turn (`entity`), as its closed profile.
pub(super) fn parse_circle(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
    entity: &str,
) -> Result<OrderedProfile, DxfImportError> {
    let layer = parse_layer(record)?;
    let mirrored = ensure_planar_ocs(record)?;
    let accepted: &[i16] = if entity == "ARC" {
        &[5, 8, 10, 20, 30, 40, 50, 51, 100, 210, 220, 230]
    } else {
        &[5, 8, 10, 20, 30, 39, 40, 100, 210, 220, 230]
    };
    report_ignored_groups(entity, record, accepted, diagnostics)?;
    let scale = units.millimetres_per_unit();
    let center = parse_xy(record, 10, 20, scale)?;
    let radius = scale_coordinate(parse_required_number(record, 40)?, scale)?;
    if radius <= DXF_GEOMETRY_EPSILON_MM {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let right = [normalize_coordinate(center[0] + radius)?, center[1]];
    let left = [normalize_coordinate(center[0] - radius)?, center[1]];
    normalize_coordinate(center[1] + radius)?;
    normalize_coordinate(center[1] - radius)?;
    ensure_distinct(right, left)?;
    let segments = vec![
        ProfileSegment::CircularArc {
            start_mm: right,
            end_mm: left,
            center_mm: center,
            clockwise: false,
        },
        ProfileSegment::CircularArc {
            start_mm: left,
            end_mm: right,
            center_mm: center,
            clockwise: false,
        },
    ];
    Ok(OrderedProfile {
        layer,
        ordinal,
        style: EntityStyle::default(),
        segments: if mirrored {
            segments.into_iter().map(mirror_ocs).collect()
        } else {
            segments
        },
        closed: true,
    })
}

pub(super) fn parse_arc(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<LooseSegment, DxfImportError> {
    let layer = parse_layer(record)?;
    let mirrored = ensure_planar_ocs(record)?;
    report_ignored_groups(
        "ARC",
        record,
        &[5, 8, 10, 20, 30, 40, 50, 51, 100, 210, 220, 230],
        diagnostics,
    )?;
    let scale = units.millimetres_per_unit();
    let center = parse_xy(record, 10, 20, scale)?;
    let radius = scale_coordinate(parse_required_number(record, 40)?, scale)?;
    if radius <= DXF_GEOMETRY_EPSILON_MM {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let start_angle = parse_required_number(record, 50)?.rem_euclid(360.0);
    let end_angle = parse_required_number(record, 51)?.rem_euclid(360.0);
    let sweep = (end_angle - start_angle).rem_euclid(360.0);
    if sweep <= f64::EPSILON || (360.0 - sweep) <= f64::EPSILON {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let (start_sin, start_cos) = deterministic_sin_cos_degrees(start_angle);
    let (end_sin, end_cos) = deterministic_sin_cos_degrees(end_angle);
    let start = [
        normalize_coordinate(center[0] + radius * start_cos)?,
        normalize_coordinate(center[1] + radius * start_sin)?,
    ];
    let end = [
        normalize_coordinate(center[0] + radius * end_cos)?,
        normalize_coordinate(center[1] + radius * end_sin)?,
    ];
    validate_arc_sweep_envelope(start, end, center, radius, false, sweep > 180.0)?;
    ensure_distinct(start, end)?;
    let segment = ProfileSegment::CircularArc {
        start_mm: start,
        end_mm: end,
        center_mm: center,
        clockwise: false,
    };
    Ok(LooseSegment {
        layer,
        ordinal: 0,
        style: EntityStyle::default(),
        segment: if mirrored {
            mirror_ocs(segment)
        } else {
            segment
        },
    })
}
