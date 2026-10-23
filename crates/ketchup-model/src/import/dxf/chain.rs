//! Loose lines and arcs chained into profiles, layer by layer: a repeated line, a run
//! that branches or does not chain, or a profile that repeats a segment of its layer is
//! left out as one group in the report, the rest kept.

use super::*;

fn arc_endpoint_buckets(segments: &[LooseSegment]) -> BTreeMap<[i64; 2], Vec<[f64; 2]>> {
    let mut arc_endpoints: BTreeMap<[i64; 2], Vec<[f64; 2]>> = BTreeMap::new();
    for segment in segments {
        if matches!(segment.segment, ProfileSegment::CircularArc { .. }) {
            for point in [segment.segment.start_mm(), segment.segment.end_mm()] {
                arc_endpoints
                    .entry(connectivity_bucket(point))
                    .or_default()
                    .push(point);
            }
        }
    }
    arc_endpoints
}

/// Snaps the ends of `segments` that lie within the tolerance of an arc end onto it, and
/// takes out, in their order, those whose ends it cannot snap unambiguously, each with why.
fn snap_lines_to_unambiguous_arc_endpoints(
    segments: &mut Vec<LooseSegment>,
) -> Vec<(LooseSegment, DxfImportError)> {
    // Arc ends come from a centre, a radius and angles, so two arcs with
    // different centres rarely meet bit for bit. Two ends within the tolerance
    // become one of them, the same one for both.
    let mut removed = Vec::new();
    let arc_endpoints = arc_endpoint_buckets(segments);
    let mut failed = BTreeMap::new();
    for (index, segment) in segments.iter_mut().enumerate() {
        if let ProfileSegment::CircularArc {
            start_mm, end_mm, ..
        } = &mut segment.segment
        {
            let snapped = shared_arc_endpoint(*start_mm, &arc_endpoints).and_then(|start| {
                let end = shared_arc_endpoint(*end_mm, &arc_endpoints)?;
                ensure_distinct(start, end)?;
                Ok((start, end))
            });
            match snapped {
                Ok((start, end)) => (*start_mm, *end_mm) = (start, end),
                Err(error) => {
                    failed.insert(index, error);
                }
            }
        }
    }
    take_out(segments, failed, &mut removed);
    let arc_endpoints = arc_endpoint_buckets(segments);
    let mut failed = BTreeMap::new();
    for (index, segment) in segments.iter_mut().enumerate() {
        let ProfileSegment::Line { start_mm, end_mm } = &mut segment.segment else {
            continue;
        };
        let snapped = unambiguous_arc_endpoint(*start_mm, &arc_endpoints).and_then(|start| {
            let end = unambiguous_arc_endpoint(*end_mm, &arc_endpoints)?;
            ensure_distinct(start, end)?;
            Ok((start, end))
        });
        match snapped {
            Ok((start, end)) => (*start_mm, *end_mm) = (start, end),
            Err(error) => {
                failed.insert(index, error);
            }
        }
    }
    take_out(segments, failed, &mut removed);
    removed
}

/// Moves the `segments` at the indices of `failed` to `removed`, each with its error.
fn take_out(
    segments: &mut Vec<LooseSegment>,
    mut failed: BTreeMap<usize, DxfImportError>,
    removed: &mut Vec<(LooseSegment, DxfImportError)>,
) {
    if failed.is_empty() {
        return;
    }
    let mut kept = Vec::with_capacity(segments.len() - failed.len());
    for (index, segment) in std::mem::take(segments).into_iter().enumerate() {
        match failed.remove(&index) {
            Some(error) => removed.push((segment, error)),
            None => kept.push(segment),
        }
    }
    *segments = kept;
}

/// The point an arc end shares with at most one other arc end near it.
fn shared_arc_endpoint(
    point: [f64; 2],
    arc_endpoints: &BTreeMap<[i64; 2], Vec<[f64; 2]>>,
) -> Result<[f64; 2], DxfImportError> {
    let near = nearby_arc_endpoints(point, arc_endpoints);
    if near.len() > 2 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    Ok(near.into_values().next().unwrap_or(point))
}

fn nearby_arc_endpoints(
    point: [f64; 2],
    arc_endpoints: &BTreeMap<[i64; 2], Vec<[f64; 2]>>,
) -> BTreeMap<[u64; 2], [f64; 2]> {
    let bucket = connectivity_bucket(point);
    let mut candidates = BTreeMap::new();
    for x_offset in -1..=1 {
        for y_offset in -1..=1 {
            let nearby = [bucket[0] + x_offset, bucket[1] + y_offset];
            for candidate in arc_endpoints.get(&nearby).into_iter().flatten() {
                if (point[0] - candidate[0]).hypot(point[1] - candidate[1])
                    <= DXF_GEOMETRY_EPSILON_MM
                {
                    candidates.insert(point_key(*candidate), *candidate);
                }
            }
        }
    }
    candidates
}

fn unambiguous_arc_endpoint(
    point: [f64; 2],
    arc_endpoints: &BTreeMap<[i64; 2], Vec<[f64; 2]>>,
) -> Result<[f64; 2], DxfImportError> {
    let candidates = nearby_arc_endpoints(point, arc_endpoints);
    match candidates.len() {
        0 => Ok(point),
        1 => Ok(*candidates.values().next().expect("one endpoint candidate")),
        _ => Err(DxfImportError::AmbiguousGeometry),
    }
}

fn connectivity_bucket(point: [f64; 2]) -> [i64; 2] {
    [
        (point[0] / DXF_GEOMETRY_EPSILON_MM).floor() as i64,
        (point[1] / DXF_GEOMETRY_EPSILON_MM).floor() as i64,
    ]
}

/// Where a drawing or block reports what it leaves out of its profiles: the handles of its
/// entities by ordinal, its report, and the name of the block (whose coordinates the
/// report then gives) or none for the drawing. Without one, the first ambiguity refuses
/// the whole reading, as within one hatch.
type LeaveOut<'a> = Option<(
    &'a BTreeMap<usize, String>,
    &'a mut DiagnosticCounts,
    Option<&'a str>,
)>;

/// Leaves out one group of segments of `layer`, each drawn by the entity of its ordinal,
/// as one report entry: `kind` of `error`, the layer, how many lines and the rectangle
/// around them, so the group can be found in the drawing, with the handles of the first
/// entities. Refuses with `error` when there is nowhere to report it.
fn leave_out<'s>(
    report: &mut LeaveOut<'_>,
    layer: &str,
    kind: &str,
    drawn: impl IntoIterator<Item = (usize, &'s ProfileSegment)>,
    error: DxfImportError,
) -> Result<(), DxfImportError> {
    let drawn = drawn.into_iter().collect::<Vec<_>>();
    if drawn.is_empty() {
        return Ok(());
    }
    let Some((handles, diagnostics, block)) = report else {
        return Err(error);
    };
    let [min, max] = drawn.iter().fold(
        [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
        |[min, max], (_, segment)| {
            let [low, high] = segment.bounds_mm();
            [
                [min[0].min(low[0]), min[1].min(low[1])],
                [max[0].max(high[0]), max[1].max(high[1])],
            ]
        },
    );
    let lines = match drawn.len() {
        1 => "1 line".to_owned(),
        count => format!("{count} lines"),
    };
    let within = block.map_or(String::new(), |name| format!(" of block {name}"));
    let what = format!(
        "{}: {kind}, layer {layer}, {lines} within x {}..{}, y {}..{} mm{within}",
        error.reason(),
        report_mm(min[0]),
        report_mm(max[0]),
        report_mm(min[1]),
        report_mm(max[1]),
    );
    let mut ordinals = Vec::new();
    for (ordinal, _) in drawn {
        if !ordinals.contains(&ordinal) {
            ordinals.push(ordinal);
        }
    }
    for ordinal in ordinals {
        diagnostics.skip(
            "dxf.profile-ambiguous",
            what.clone(),
            handles
                .get(&ordinal)
                .map(|handle| ("handles", handle.as_str())),
            Some(error.clone()),
        )?;
    }
    Ok(())
}

/// `value` in millimetres to a thousandth, without trailing zeros.
fn report_mm(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0" } else { text }.to_owned()
}

/// The profiles `loose` chains into, layer by layer. With a `report`, a line it cannot
/// place unambiguously is left out and counted: a repeated line alone, and every line of
/// a run that branches or does not chain; without one, it refuses them all.
pub(super) fn assemble_loose_segments(
    loose: Vec<LooseSegment>,
    max_segments: usize,
    mut report: LeaveOut<'_>,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let mut by_layer: BTreeMap<String, Vec<LooseSegment>> = BTreeMap::new();
    for segment in loose {
        by_layer
            .entry(segment.layer.clone())
            .or_default()
            .push(segment);
    }
    let mut profiles = Vec::new();
    for (layer, mut segments) in by_layer {
        for (segment, error) in snap_lines_to_unambiguous_arc_endpoints(&mut segments) {
            leave_out(
                &mut report,
                &layer,
                "an end near several arc ends",
                [(segment.ordinal, &segment.segment)],
                error,
            )?;
        }
        let mut duplicate_lines = BTreeSet::new();
        let mut repeated = Vec::new();
        segments.retain(|segment| {
            if let ProfileSegment::Line { .. } = segment.segment {
                let start = point_key(segment.segment.start_mm());
                let end = point_key(segment.segment.end_mm());
                let key = if start <= end {
                    (start, end)
                } else {
                    (end, start)
                };
                if !duplicate_lines.insert(key) {
                    repeated.push((segment.ordinal, segment.segment.clone()));
                    return false;
                }
            }
            true
        });
        for (ordinal, segment) in &repeated {
            leave_out(
                &mut report,
                &layer,
                "a repeated line",
                [(*ordinal, segment)],
                DxfImportError::AmbiguousGeometry,
            )?;
        }
        let mut endpoints: BTreeMap<[u64; 2], Vec<usize>> = BTreeMap::new();
        for (index, segment) in segments.iter().enumerate() {
            endpoints
                .entry(point_key(segment.segment.start_mm()))
                .or_default()
                .push(index);
            endpoints
                .entry(point_key(segment.segment.end_mm()))
                .or_default()
                .push(index);
        }
        let mut unused = (0..segments.len()).collect::<BTreeSet<_>>();
        // Seeds in ordinal order, and open ends looked up among the
        // component's own points: scanning every segment per profile made a
        // file of many separate segments quadratic.
        let mut seeds = (0..segments.len()).collect::<Vec<_>>();
        seeds.sort_by_key(|index| segments[*index].ordinal);
        for seed in seeds {
            if !unused.contains(&seed) {
                continue;
            }
            let mut component = BTreeSet::new();
            let mut queue = VecDeque::from([seed]);
            while let Some(index) = queue.pop_front() {
                if !component.insert(index) {
                    continue;
                }
                for point in [
                    segments[index].segment.start_mm(),
                    segments[index].segment.end_mm(),
                ] {
                    if let Some(incident) = endpoints.get(&point_key(point)) {
                        queue.extend(incident.iter().copied());
                    }
                }
            }
            for index in &component {
                unused.remove(index);
            }
            let ordinal = component
                .iter()
                .map(|index| segments[*index].ordinal)
                .min()
                .expect("non-empty component");
            let (ordered, closed) =
                match chain_component(&segments, &endpoints, &component, seed, max_segments) {
                    Ok(chained) => chained,
                    Err((error, detail)) => {
                        leave_out(
                            &mut report,
                            &layer,
                            detail,
                            component
                                .iter()
                                .map(|index| (segments[*index].ordinal, &segments[*index].segment)),
                            error,
                        )?;
                        continue;
                    }
                };
            profiles.push(OrderedProfile {
                layer: layer.clone(),
                ordinal,
                style: EntityStyle::default(),
                segments: ordered,
                closed,
            });
        }
    }
    Ok(profiles)
}

/// The segments of `component`, a connected run of `segments` that `endpoints` indexes by
/// end, in order from its smaller open end (or from the start of `seed` when it closes),
/// and whether it closes; or why they do not chain into one profile.
fn chain_component(
    segments: &[LooseSegment],
    endpoints: &BTreeMap<[u64; 2], Vec<usize>>,
    component: &BTreeSet<usize>,
    seed: usize,
    max_segments: usize,
) -> Result<(Vec<ProfileSegment>, bool), (DxfImportError, &'static str)> {
    let ambiguous = |detail| (DxfImportError::AmbiguousGeometry, detail);
    let component_points = component
        .iter()
        .flat_map(|index| {
            [
                point_key(segments[*index].segment.start_mm()),
                point_key(segments[*index].segment.end_mm()),
            ]
        })
        .collect::<BTreeSet<_>>();
    let degree = |point: &[u64; 2]| {
        endpoints[point]
            .iter()
            .filter(|index| component.contains(index))
            .count()
    };
    if component_points.iter().any(|point| degree(point) > 2) {
        return Err(ambiguous("a T-junction (lines that branch)"));
    }
    if component.len() > max_segments {
        return Err((
            DxfImportError::TooManySegments,
            "a run of too many segments",
        ));
    }
    let degree_one = component_points
        .into_iter()
        .filter(|point| degree(point) == 1)
        .collect::<Vec<_>>();
    let closed = match degree_one.len() {
        0 => true,
        2 => false,
        _ => return Err(ambiguous("lines that do not chain")),
    };
    let start_point = if closed {
        point_key(segments[seed].segment.start_mm())
    } else {
        *degree_one.iter().min().expect("two open endpoints")
    };
    let mut unused = component.clone();
    let mut current = start_point;
    let mut ordered = Vec::with_capacity(component.len());
    while !unused.is_empty() {
        let Some(index) = endpoints
            .get(&current)
            .into_iter()
            .flatten()
            .filter(|index| unused.contains(index))
            .copied()
            .min_by_key(|index| segments[*index].ordinal)
        else {
            return Err(ambiguous("lines that do not chain"));
        };
        let segment = &segments[index].segment;
        let oriented = if point_key(segment.start_mm()) == current {
            segment.clone()
        } else {
            reverse_segment(segment)
        };
        current = point_key(oriented.end_mm());
        ordered.push(oriented);
        unused.remove(&index);
    }
    if (closed && current != start_point) || (!closed && !degree_one.contains(&current)) {
        return Err(ambiguous("lines that do not chain"));
    }
    Ok((ordered, closed))
}

/// `profiles` without each that repeats a segment of its layer, of an earlier profile or
/// of its own, left out and counted in `report` (refused without one).
pub(super) fn without_duplicate_segments(
    profiles: Vec<OrderedProfile>,
    mut report: LeaveOut<'_>,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let mut seen = BTreeSet::new();
    let mut kept = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let mut own = BTreeSet::new();
        let repeats = profile.segments.iter().any(|segment| {
            let key = (profile.layer.clone(), undirected_segment_key(segment));
            seen.contains(&key) || !own.insert(key)
        });
        if repeats {
            leave_out(
                &mut report,
                &profile.layer,
                "a segment shared with another profile of its layer",
                profile
                    .segments
                    .iter()
                    .map(|segment| (profile.ordinal, segment)),
                DxfImportError::AmbiguousGeometry,
            )?;
            continue;
        }
        seen.extend(own);
        kept.push(profile);
    }
    Ok(kept)
}

fn undirected_segment_key(segment: &ProfileSegment) -> UndirectedSegmentKey {
    let start = duplicate_point_key(segment.start_mm());
    let end = duplicate_point_key(segment.end_mm());
    match segment {
        ProfileSegment::Line { .. } => {
            if start <= end {
                UndirectedSegmentKey::Line(start, end)
            } else {
                UndirectedSegmentKey::Line(end, start)
            }
        }
        ProfileSegment::CircularArc {
            center_mm,
            clockwise,
            ..
        } => {
            if start <= end {
                UndirectedSegmentKey::CircularArc {
                    start,
                    end,
                    center: duplicate_point_key(*center_mm),
                    clockwise: *clockwise,
                }
            } else {
                UndirectedSegmentKey::CircularArc {
                    start: end,
                    end: start,
                    center: duplicate_point_key(*center_mm),
                    clockwise: !clockwise,
                }
            }
        }
        ProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            ..
        } => {
            if start <= end {
                UndirectedSegmentKey::CubicBezier {
                    start,
                    control_1: duplicate_point_key(*control_1_mm),
                    control_2: duplicate_point_key(*control_2_mm),
                    end,
                }
            } else {
                UndirectedSegmentKey::CubicBezier {
                    start: end,
                    control_1: duplicate_point_key(*control_2_mm),
                    control_2: duplicate_point_key(*control_1_mm),
                    end: start,
                }
            }
        }
        ProfileSegment::Spline { points_mm } => {
            let mut points = points_mm
                .iter()
                .map(|point| duplicate_point_key(*point))
                .collect::<Vec<_>>();
            if start > end {
                points.reverse();
            }
            UndirectedSegmentKey::Spline(points)
        }
    }
}

pub(super) fn point_key(point: [f64; 2]) -> [u64; 2] {
    [point[0].to_bits(), point[1].to_bits()]
}

fn duplicate_point_key(point: [f64; 2]) -> [i64; 2] {
    const DUPLICATE_GRID_MM: f64 = ACCUMULATED_ROUNDING;
    [
        (point[0] / DUPLICATE_GRID_MM).round() as i64,
        (point[1] / DUPLICATE_GRID_MM).round() as i64,
    ]
}

fn reverse_segment(segment: &ProfileSegment) -> ProfileSegment {
    match segment {
        ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
            start_mm: *end_mm,
            end_mm: *start_mm,
        },
        ProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => ProfileSegment::CircularArc {
            start_mm: *end_mm,
            end_mm: *start_mm,
            center_mm: *center_mm,
            clockwise: !clockwise,
        },
        ProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => ProfileSegment::CubicBezier {
            start_mm: *end_mm,
            control_1_mm: *control_2_mm,
            control_2_mm: *control_1_mm,
            end_mm: *start_mm,
        },
        ProfileSegment::Spline { points_mm } => ProfileSegment::Spline {
            points_mm: points_mm.iter().rev().copied().collect(),
        },
    }
}
