//! POLYLINE (2D, 3D, polygon mesh and polyface) and LWPOLYLINE entities of a drawing:
//! their vertices, joined by lines or, where a vertex has a bulge, by arcs, as profiles.

use super::*;

pub(super) fn parse_polyline(
    record: &[DxfPair<'_>],
    pairs: &[DxfPair<'_>],
    index: &mut usize,
    ordinal: &mut usize,
    entity_count: &mut usize,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let profile_ordinal = *ordinal;
    let layer = parse_layer(record)?;
    let mirrored = ensure_planar_ocs(record)?;
    report_ignored_groups(
        "POLYLINE",
        record,
        &[
            5, 8, 10, 20, 30, 39, 66, 70, 71, 72, 73, 74, 75, 100, 210, 220, 230,
        ],
        diagnostics,
    )?;
    if parse_optional_i64(record, 66)?.is_some_and(|value| value != 1) {
        return Err(DxfImportError::MalformedPairs);
    }
    if parse_optional_number(record, 10)?.is_some_and(|value| value != 0.0)
        || parse_optional_number(record, 20)?.is_some_and(|value| value != 0.0)
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let flags = parse_optional_i64(record, 70)?.unwrap_or(0);
    if flags & !(1 | 8 | 16 | 32 | 64 | 128) != 0 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let polygon_mesh = flags & 16 != 0;
    let polyface = flags & 64 != 0;
    if (polygon_mesh && (polyface || flags & 8 != 0))
        || (polyface && flags & (1 | 8 | 16 | 32) != 0)
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let has_mesh_metadata = record.iter().any(|pair| matches!(pair.code, 71..=75));
    if (!polygon_mesh && !polyface && has_mesh_metadata)
        || (polyface && record.iter().any(|pair| matches!(pair.code, 73..=75)))
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let three_dimensional = flags & 8 != 0;
    // A 3D polyline and a mesh are in world coordinates; seen from below they are
    // not read (review 2026-10-24 UL-22 mirrors only the 2D one, in its own).
    if mirrored && (three_dimensional || polygon_mesh || polyface) {
        return Err(DxfImportError::NonPlanarGeometry);
    }
    if flags & 128 != 0 {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.polyline-generation-flag-ignored",
            Some(layer.clone()),
            1,
        )?;
    }
    let closed = flags & 1 != 0;
    let mut vertices = Vec::new();
    let mut polygon_mesh_vertices = Vec::new();
    let mut polyface_vertices = Vec::new();
    let mut polyface_faces = Vec::new();
    let mut saw_polyface_face = false;
    let mut saw_seqend = false;
    while *index < pairs.len() {
        if pairs[*index].code != 0 || pairs[*index].value.is_empty() {
            return Err(DxfImportError::MalformedPairs);
        }
        *ordinal += 1;
        bump_entity_count(entity_count, diagnostics.reading.max_entities)?;
        let kind = pairs[*index].value.to_ascii_uppercase();
        let start = *index + 1;
        *index = start;
        while *index < pairs.len() && pairs[*index].code != 0 {
            *index += 1;
        }
        let child = &pairs[start..*index];
        match kind.as_str() {
            "VERTEX" if polygon_mesh => {
                polygon_mesh_vertices.push(parse_polygon_mesh_vertex(child, &layer, diagnostics)?);
            }
            "VERTEX" if polyface => match parse_polyface_vertex(child, &layer, diagnostics)? {
                PolyfaceRecord::Coordinate(point) => {
                    if saw_polyface_face {
                        return Err(DxfImportError::MalformedPairs);
                    }
                    polyface_vertices.push(point);
                }
                PolyfaceRecord::Face { indices, hidden } => {
                    saw_polyface_face = true;
                    if hidden > 0 {
                        diagnostics.add(
                            ImportDiagnosticSeverity::Warning,
                            "dxf.polyface-invisible-edge-dropped",
                            Some(layer.clone()),
                            hidden,
                        )?;
                    }
                    polyface_faces.push(indices);
                }
            },
            "VERTEX" => {
                if vertices.len() > diagnostics.reading.max_segments {
                    return Err(DxfImportError::TooManySegments);
                }
                vertices.push(parse_polyline_vertex(
                    child,
                    &layer,
                    three_dimensional,
                    diagnostics,
                )?);
            }
            "SEQEND" => {
                ensure_matching_optional_layer(child, &layer)?;
                report_ignored_groups("SEQEND", child, &[5, 8, 100], diagnostics)?;
                saw_seqend = true;
                break;
            }
            _ => return Err(DxfImportError::MalformedPairs),
        }
    }
    if !saw_seqend {
        return Err(DxfImportError::MalformedPairs);
    }
    if polygon_mesh {
        return finish_polygon_mesh(
            record,
            layer,
            profile_ordinal,
            polygon_mesh_vertices,
            flags,
            units,
            diagnostics,
        );
    }
    if polyface {
        return finish_polyface(
            record,
            layer,
            profile_ordinal,
            polyface_vertices,
            polyface_faces,
            units,
            diagnostics,
        );
    }
    if vertices.len() < 2 {
        return Err(DxfImportError::MalformedPairs);
    }
    let mut profile =
        build_polyline_profile(layer, profile_ordinal, vertices, closed, units, diagnostics)?;
    if mirrored {
        profile.segments = profile.segments.into_iter().map(mirror_ocs).collect();
    }
    Ok(vec![profile])
}

fn parse_polygon_mesh_vertex(
    record: &[DxfPair<'_>],
    layer: &str,
    diagnostics: &mut DiagnosticCounts,
) -> Result<[f64; 2], DxfImportError> {
    ensure_matching_optional_layer(record, layer)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "VERTEX",
        record,
        &[5, 8, 10, 20, 30, 42, 70, 100, 210, 220, 230],
        diagnostics,
    )?;
    if parse_required_i64(record, 70)? != 64 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    if parse_optional_number(record, 42)?.is_some_and(|bulge| bulge != 0.0) {
        return Err(DxfImportError::InvalidBulge);
    }
    Ok([
        parse_required_number(record, 10)?,
        parse_required_number(record, 20)?,
    ])
}

fn finish_polygon_mesh(
    header: &[DxfPair<'_>],
    layer: String,
    ordinal: usize,
    native_vertices: Vec<[f64; 2]>,
    flags: i64,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    if parse_optional_i64(header, 73)?.unwrap_or(0) != 0
        || parse_optional_i64(header, 74)?.unwrap_or(0) != 0
        || parse_optional_i64(header, 75)?.unwrap_or(0) != 0
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let m_count = parse_required_i64(header, 71)?;
    let n_count = parse_required_i64(header, 72)?;
    if m_count < 2 || n_count < 2 {
        return Err(DxfImportError::MalformedPairs);
    }
    let m_count = usize::try_from(m_count)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let n_count = usize::try_from(n_count)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let vertex_count = m_count
        .checked_mul(n_count)
        .ok_or(DxfImportError::TooManyEntities)?;
    if vertex_count > diagnostics.reading.max_entities {
        return Err(DxfImportError::TooManyEntities);
    }
    if native_vertices.len() != vertex_count {
        return Err(DxfImportError::MalformedPairs);
    }

    let m_closed = flags & 1 != 0;
    let n_closed = flags & 32 != 0;
    if m_closed && n_closed {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let max_boundary_segments = if m_closed {
        m_count
    } else if n_closed {
        n_count
    } else {
        2_usize
            .checked_mul(
                m_count
                    .checked_sub(1)
                    .and_then(|m| n_count.checked_sub(1).and_then(|n| m.checked_add(n)))
                    .ok_or(DxfImportError::TooManySegments)?,
            )
            .ok_or(DxfImportError::TooManySegments)?
    };
    if max_boundary_segments > diagnostics.reading.max_segments {
        return Err(DxfImportError::TooManySegments);
    }

    let scale = units.millimetres_per_unit();
    let mut vertices = Vec::with_capacity(native_vertices.len());
    let mut unique_vertices = BTreeSet::new();
    for point in native_vertices {
        let point = [
            scale_coordinate(point[0], scale)?,
            scale_coordinate(point[1], scale)?,
        ];
        if !unique_vertices.insert(point_key(point)) {
            return Err(DxfImportError::AmbiguousGeometry);
        }
        vertices.push(point);
    }

    let m_steps = if m_closed { m_count } else { m_count - 1 };
    let n_steps = if n_closed { n_count } else { n_count - 1 };
    let mut faces = Vec::with_capacity(m_steps * n_steps);
    for m in 0..m_steps {
        let next_m = (m + 1) % m_count;
        for n in 0..n_steps {
            let next_n = (n + 1) % n_count;
            let face = vec![
                m * n_count + n,
                next_m * n_count + n,
                next_m * n_count + next_n,
                m * n_count + next_n,
            ];
            let points = face
                .iter()
                .map(|index| vertices[*index])
                .collect::<Vec<_>>();
            validate_mesh_face(&points)?;
            faces.push(face);
        }
    }
    build_mesh_boundary_profiles(
        layer,
        ordinal,
        vertices,
        faces,
        "dxf.polygon-mesh-surface-topology-dropped",
        "dxf.polygon-mesh-boundary-geometry",
        diagnostics,
    )
}

enum PolyfaceRecord {
    Coordinate([f64; 2]),
    Face { indices: Vec<usize>, hidden: u32 },
}

fn parse_polyface_vertex(
    record: &[DxfPair<'_>],
    layer: &str,
    diagnostics: &mut DiagnosticCounts,
) -> Result<PolyfaceRecord, DxfImportError> {
    ensure_matching_optional_layer(record, layer)?;
    report_ignored_groups(
        "VERTEX",
        record,
        &[5, 8, 10, 20, 30, 70, 71, 72, 73, 74, 100],
        diagnostics,
    )?;
    let flags = parse_required_i64(record, 70)?;
    match flags {
        64 | 192 => {
            ensure_planar(record)?;
            if record.iter().any(|pair| matches!(pair.code, 71..=74)) {
                return Err(DxfImportError::MalformedPairs);
            }
            Ok(PolyfaceRecord::Coordinate([
                parse_required_number(record, 10)?,
                parse_required_number(record, 20)?,
            ]))
        }
        128 => {
            if parse_optional_number(record, 10)?.is_some_and(|value| value != 0.0)
                || parse_optional_number(record, 20)?.is_some_and(|value| value != 0.0)
                || parse_optional_number(record, 30)?.is_some_and(|value| value != 0.0)
            {
                return Err(DxfImportError::AmbiguousGeometry);
            }
            let raw_indices = [71_i16, 72, 73, 74]
                .into_iter()
                .map(|code| parse_optional_i64(record, code))
                .collect::<Result<Vec<_>, _>>()?;
            let mut indices = Vec::with_capacity(4);
            let mut hidden = 0_u32;
            let mut ended = false;
            for raw_index in raw_indices {
                let raw_index = raw_index.unwrap_or(0);
                if raw_index == 0 {
                    ended = true;
                    continue;
                }
                if ended {
                    return Err(DxfImportError::MalformedPairs);
                }
                if raw_index < 0 {
                    hidden += 1;
                }
                let one_based = raw_index
                    .checked_abs()
                    .ok_or(DxfImportError::MalformedPairs)?;
                let one_based = usize::try_from(one_based)
                    .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
                indices.push(
                    one_based
                        .checked_sub(1)
                        .ok_or(DxfImportError::MalformedPairs)?,
                );
            }
            if !matches!(indices.len(), 3 | 4) {
                return Err(DxfImportError::MalformedPairs);
            }
            Ok(PolyfaceRecord::Face { indices, hidden })
        }
        _ => Err(DxfImportError::AmbiguousGeometry),
    }
}

fn finish_polyface(
    header: &[DxfPair<'_>],
    layer: String,
    ordinal: usize,
    native_vertices: Vec<[f64; 2]>,
    faces: Vec<Vec<usize>>,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let declared_vertices = parse_required_i64(header, 71)?;
    let declared_faces = parse_required_i64(header, 72)?;
    if declared_vertices < 3 || declared_faces < 1 {
        return Err(DxfImportError::MalformedPairs);
    }
    let declared_vertices = usize::try_from(declared_vertices)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let declared_faces = usize::try_from(declared_faces)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    if native_vertices.len() != declared_vertices || faces.len() != declared_faces {
        return Err(DxfImportError::MalformedPairs);
    }
    let scale = units.millimetres_per_unit();
    let mut vertices = Vec::with_capacity(native_vertices.len());
    let mut unique_vertices = BTreeSet::new();
    for point in native_vertices {
        let point = [
            scale_coordinate(point[0], scale)?,
            scale_coordinate(point[1], scale)?,
        ];
        if !unique_vertices.insert(point_key(point)) {
            return Err(DxfImportError::AmbiguousGeometry);
        }
        vertices.push(point);
    }
    for face in &faces {
        let mut unique_indices = BTreeSet::new();
        for index in face {
            if *index >= vertices.len() || !unique_indices.insert(*index) {
                return Err(DxfImportError::AmbiguousGeometry);
            }
        }
        let points = face
            .iter()
            .map(|index| vertices[*index])
            .collect::<Vec<_>>();
        validate_mesh_face(&points)?;
    }
    build_mesh_boundary_profiles(
        layer,
        ordinal,
        vertices,
        faces,
        "dxf.polyface-face-topology-dropped",
        "dxf.polyface-boundary-geometry",
        diagnostics,
    )
}

fn parse_polyline_vertex(
    record: &[DxfPair<'_>],
    layer: &str,
    three_dimensional: bool,
    diagnostics: &mut DiagnosticCounts,
) -> Result<PolylineVertex, DxfImportError> {
    ensure_matching_optional_layer(record, layer)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "VERTEX",
        record,
        &[5, 8, 10, 20, 30, 42, 70, 100, 210, 220, 230],
        diagnostics,
    )?;
    let flags = parse_optional_i64(record, 70)?.unwrap_or(0);
    if flags != if three_dimensional { 32 } else { 0 } {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let bulge = parse_optional_number(record, 42)?;
    if three_dimensional && bulge.is_some_and(|value| value != 0.0) {
        return Err(DxfImportError::InvalidBulge);
    }
    Ok(PolylineVertex {
        x: parse_required_number(record, 10)?,
        y: Some(parse_required_number(record, 20)?),
        bulge: if three_dimensional { None } else { bulge },
    })
}

#[derive(Clone, Copy)]
pub(super) struct PolylineVertex {
    pub(super) x: f64,
    pub(super) y: Option<f64>,
    pub(super) bulge: Option<f64>,
}

pub(super) fn parse_lwpolyline(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<OrderedProfile, DxfImportError> {
    let layer = parse_layer(record)?;
    let mirrored = ensure_planar_ocs(record)?;
    report_ignored_groups(
        "LWPOLYLINE",
        record,
        &[5, 8, 10, 20, 30, 38, 39, 42, 70, 90, 100, 210, 220, 230],
        diagnostics,
    )?;
    let declared_count = parse_required_i64(record, 90)?;
    if declared_count < 2 || declared_count as usize > diagnostics.reading.max_segments + 1 {
        return Err(DxfImportError::TooManySegments);
    }
    let flags = parse_optional_i64(record, 70)?.unwrap_or(0);
    if flags & !129 != 0 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    if flags & 128 != 0 {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.polyline-generation-flag-ignored",
            Some(layer.clone()),
            1,
        )?;
    }
    let closed = flags & 1 != 0;
    let mut vertices = Vec::new();
    for pair in record {
        match pair.code {
            10 => vertices.push(PolylineVertex {
                x: parse_number(pair.value)?,
                y: None,
                bulge: None,
            }),
            20 => {
                let vertex = vertices.last_mut().ok_or(DxfImportError::MalformedPairs)?;
                if vertex.y.replace(parse_number(pair.value)?).is_some() {
                    return Err(DxfImportError::MalformedPairs);
                }
            }
            42 => {
                let vertex = vertices.last_mut().ok_or(DxfImportError::MalformedPairs)?;
                if vertex.bulge.is_some() {
                    return Err(DxfImportError::MalformedPairs);
                }
                vertex.bulge = Some(parse_number(pair.value)?);
            }
            _ => {}
        }
    }
    if vertices.len() != declared_count as usize || vertices.iter().any(|vertex| vertex.y.is_none())
    {
        return Err(DxfImportError::MalformedPairs);
    }
    let mut profile = build_polyline_profile(layer, ordinal, vertices, closed, units, diagnostics)?;
    if mirrored {
        profile.segments = profile.segments.into_iter().map(mirror_ocs).collect();
    }
    Ok(profile)
}

pub(super) fn build_polyline_profile(
    layer: String,
    ordinal: usize,
    vertices: Vec<PolylineVertex>,
    closed: bool,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<OrderedProfile, DxfImportError> {
    let scale = units.millimetres_per_unit();
    let points = vertices
        .iter()
        .map(|vertex| {
            Ok([
                scale_coordinate(vertex.x, scale)?,
                scale_coordinate(vertex.y.expect("validated polyline Y"), scale)?,
            ])
        })
        .collect::<Result<Vec<_>, DxfImportError>>()?;
    let read = points.len();
    let (points, vertices) = without_repeated_vertices(points, vertices, closed);
    let removed = u32::try_from(read - points.len())
        .map_err(|_: std::num::TryFromIntError| DxfImportError::TooManySegments)?;
    if removed > 0 {
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.repeated-vertex-removed",
            Some(layer.clone()),
            removed,
        )?;
    }
    if points.len() < 2 {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let edge_count = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    if edge_count > diagnostics.reading.max_segments {
        return Err(DxfImportError::TooManySegments);
    }
    if !closed
        && vertices
            .last()
            .and_then(|vertex| vertex.bulge)
            .is_some_and(|bulge| bulge != 0.0)
    {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.open-polyline-terminal-bulge-ignored",
            Some(layer.clone()),
            1,
        )?;
    }
    let mut segments = Vec::with_capacity(edge_count);
    for index in 0..edge_count {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        ensure_distinct(start, end)?;
        segments.push(segment_from_bulge(
            start,
            end,
            vertices[index].bulge.unwrap_or(0.0),
        )?);
    }
    Ok(OrderedProfile {
        layer,
        ordinal,
        style: EntityStyle::default(),
        segments,
        closed,
    })
}

fn segment_from_bulge(
    start: [f64; 2],
    end: [f64; 2],
    bulge: f64,
) -> Result<ProfileSegment, DxfImportError> {
    if bulge == 0.0 {
        return Ok(ProfileSegment::Line {
            start_mm: start,
            end_mm: end,
        });
    }
    if !bulge.is_finite() {
        return Err(DxfImportError::InvalidBulge);
    }
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let chord_squared = dx * dx + dy * dy;
    if !chord_squared.is_finite()
        || chord_squared <= DXF_GEOMETRY_EPSILON_MM * DXF_GEOMETRY_EPSILON_MM
    {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let center_factor = (1.0 - bulge * bulge) / (4.0 * bulge);
    let center = [
        normalize_coordinate((start[0] + end[0]) * 0.5 - dy * center_factor)?,
        normalize_coordinate((start[1] + end[1]) * 0.5 + dx * center_factor)?,
    ];
    let radius_dx = start[0] - center[0];
    let radius_dy = start[1] - center[1];
    let radius_squared = radius_dx * radius_dx + radius_dy * radius_dy;
    if !radius_squared.is_finite()
        || radius_squared <= DXF_GEOMETRY_EPSILON_MM * DXF_GEOMETRY_EPSILON_MM
    {
        return Err(DxfImportError::InvalidBulge);
    }
    validate_arc_sweep_envelope(
        start,
        end,
        center,
        radius_squared.sqrt(),
        bulge < 0.0,
        bulge.abs() > 1.0,
    )?;
    Ok(ProfileSegment::CircularArc {
        start_mm: start,
        end_mm: end,
        center_mm: center,
        clockwise: bulge < 0.0,
    })
}
