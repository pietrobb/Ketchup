use std::collections::{BTreeMap, BTreeSet};

use crate::document::MeshBodySpec;

const MIN_CYLINDER_SIDES: usize = 6;
const UNAMBIGUOUS_CYLINDER_SIDES: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecognizedMeshKind {
    Box,
    Cylinder,
    LinearExtrusion,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshRecognitionResiduals {
    pub tolerance_mm: f64,
    pub max_layer_distance_mm: f64,
    pub max_pairing_distance_mm: f64,
    pub max_profile_distance_mm: f64,
    pub max_surface_distance_mm: f64,
}

impl MeshRecognitionResiduals {
    #[must_use]
    pub fn maximum_mm(self) -> f64 {
        self.max_layer_distance_mm
            .max(self.max_pairing_distance_mm)
            .max(self.max_profile_distance_mm)
            .max(self.max_surface_distance_mm)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoxRecognition {
    pub center_mm: [f64; 3],
    pub axes: [[f64; 3]; 3],
    pub dimensions_mm: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct CylinderRecognition {
    pub center_mm: [f64; 3],
    pub axis: [f64; 3],
    pub radius_mm: f64,
    pub height_mm: f64,
    pub tessellated_side_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinearExtrusionRecognition {
    pub base_origin_mm: [f64; 3],
    pub profile_basis: [[f64; 3]; 2],
    pub axis: [f64; 3],
    pub height_mm: f64,
    pub profile_mm: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MeshRecognitionCandidate {
    Box(BoxRecognition),
    Cylinder(CylinderRecognition),
    LinearExtrusion(LinearExtrusionRecognition),
}

impl MeshRecognitionCandidate {
    #[must_use]
    pub const fn kind(&self) -> RecognizedMeshKind {
        match self {
            Self::Box(_) => RecognizedMeshKind::Box,
            Self::Cylinder(_) => RecognizedMeshKind::Cylinder,
            Self::LinearExtrusion(_) => RecognizedMeshKind::LinearExtrusion,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MeshRecognition {
    NoMatch {
        reason: String,
    },
    Ambiguous {
        candidates: Vec<MeshRecognitionCandidate>,
        residuals: MeshRecognitionResiduals,
        reason: String,
    },
    Candidate {
        candidate: MeshRecognitionCandidate,
        residuals: MeshRecognitionResiduals,
    },
}

#[derive(Clone, Debug)]
struct PrismEvidence {
    extrusion_axis: usize,
    low: f64,
    high: f64,
    profile: Vec<[f64; 2]>,
    center_2d: [f64; 2],
    residuals: MeshRecognitionResiduals,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum ToleranceBucketCoordinate {
    Cell(i64),
    Exact(u64),
}

fn tolerance_bucket_coordinate(value: f64, tolerance_mm: f64) -> ToleranceBucketCoordinate {
    let scaled = value / tolerance_mm;
    if scaled.is_finite() && scaled > i64::MIN as f64 && scaled < i64::MAX as f64 {
        ToleranceBucketCoordinate::Cell(scaled.floor() as i64)
    } else {
        // Beyond 2^63 cells, adjacent finite f64 values are farther apart than the tolerance.
        ToleranceBucketCoordinate::Exact(if value == 0.0 { 0 } else { value.to_bits() })
    }
}

fn neighboring_bucket_coordinates(
    coordinate: ToleranceBucketCoordinate,
) -> [Option<ToleranceBucketCoordinate>; 3] {
    match coordinate {
        ToleranceBucketCoordinate::Cell(cell) => [
            cell.checked_sub(1).map(ToleranceBucketCoordinate::Cell),
            Some(coordinate),
            cell.checked_add(1).map(ToleranceBucketCoordinate::Cell),
        ],
        ToleranceBucketCoordinate::Exact(_) => [None, Some(coordinate), None],
    }
}

/// Examines a closed mesh without changing the document or the mesh.
///
/// The recognizer deliberately supports only Cartesian linear extrusions. A
/// caller must still explicitly confirm and independently verify any exact
/// conversion based on the returned candidate.
#[must_use]
pub fn recognize_mesh_body(mesh: &MeshBodySpec, tolerance_mm: f64) -> MeshRecognition {
    recognize_mesh_body_cancellable(mesh, tolerance_mm, || false)
}

#[must_use]
pub fn recognize_mesh_body_cancellable(
    mesh: &MeshBodySpec,
    tolerance_mm: f64,
    cancelled: impl Fn() -> bool,
) -> MeshRecognition {
    if cancelled() || !tolerance_mm.is_finite() || tolerance_mm <= 0.0 {
        return MeshRecognition::NoMatch {
            reason: "recognition tolerance must be finite and positive".to_owned(),
        };
    }
    if mesh.vertices_mm.len() < 4
        || mesh.triangles.len() < 4
        || mesh
            .vertices_mm
            .iter()
            .flatten()
            .any(|value| cancelled() || !value.is_finite())
        || mesh
            .triangles
            .iter()
            .flatten()
            .any(|index| cancelled() || *index as usize >= mesh.vertices_mm.len())
        || !closed_oriented_triangle_manifold(mesh, &cancelled)
    {
        return MeshRecognition::NoMatch {
            reason: "mesh is not a finite indexed triangle body".to_owned(),
        };
    }

    let mut prisms = (0..3)
        .filter_map(|axis| prism_evidence(mesh, axis, tolerance_mm, &cancelled))
        .collect::<Vec<_>>();
    prisms.sort_by(|left, right| {
        left.residuals
            .maximum_mm()
            .total_cmp(&right.residuals.maximum_mm())
            .then_with(|| left.extrusion_axis.cmp(&right.extrusion_axis))
    });
    if prisms.is_empty() {
        return MeshRecognition::NoMatch {
            reason: "mesh does not contain two paired planar layers within tolerance".to_owned(),
        };
    }

    let mut boxes = prisms
        .iter()
        .filter_map(|evidence| box_candidate(evidence, tolerance_mm))
        .collect::<Vec<_>>();
    boxes.sort_by(|left, right| {
        left.1
            .maximum_mm()
            .total_cmp(&right.1.maximum_mm())
            .then_with(|| candidate_dimensions(&left.0).total_cmp(&candidate_dimensions(&right.0)))
    });
    if let Some((candidate, residuals)) = boxes.first().cloned() {
        return MeshRecognition::Candidate {
            candidate,
            residuals,
        };
    }

    let mut cylinders = prisms
        .iter()
        .filter_map(|evidence| cylinder_candidate(evidence, tolerance_mm, &cancelled))
        .collect::<Vec<_>>();
    cylinders.sort_by(|left, right| left.1.maximum_mm().total_cmp(&right.1.maximum_mm()));
    if let Some((candidate, residuals)) = cylinders.first().cloned() {
        let MeshRecognitionCandidate::Cylinder(cylinder) = &candidate else {
            unreachable!("cylinder_candidate always returns a cylinder")
        };
        if cylinder.tessellated_side_count >= UNAMBIGUOUS_CYLINDER_SIDES {
            return MeshRecognition::Candidate {
                candidate,
                residuals,
            };
        }
        return MeshRecognition::Ambiguous {
            candidates: vec![candidate, extrusion_candidate(&prisms[0])],
            residuals,
            reason:
                "low polygon count cannot distinguish a cylinder from a regular polygon extrusion"
                    .to_owned(),
        };
    }

    if prisms.len() == 1 {
        let evidence = &prisms[0];
        return MeshRecognition::Candidate {
            candidate: extrusion_candidate(evidence),
            residuals: evidence.residuals,
        };
    }

    MeshRecognition::Ambiguous {
        candidates: prisms.iter().map(extrusion_candidate).collect(),
        residuals: prisms[0].residuals,
        reason: "multiple Cartesian extrusion axes satisfy the requested tolerance".to_owned(),
    }
}

fn candidate_dimensions(candidate: &MeshRecognitionCandidate) -> f64 {
    match candidate {
        MeshRecognitionCandidate::Box(value) => value.dimensions_mm.into_iter().product(),
        _ => 0.0,
    }
}

fn closed_oriented_triangle_manifold(mesh: &MeshBodySpec, cancelled: &dyn Fn() -> bool) -> bool {
    let mut edges = BTreeMap::<(u32, u32), (usize, i32)>::new();
    let mut triangles = BTreeSet::new();
    for [a, b, c] in &mesh.triangles {
        if cancelled() || a == b || b == c || a == c {
            return false;
        }
        let mut canonical = [*a, *b, *c];
        canonical.sort_unstable();
        if !triangles.insert(canonical) {
            return false;
        }
        for (from, to) in [(*a, *b), (*b, *c), (*c, *a)] {
            let key = (from.min(to), from.max(to));
            let direction = if from < to { 1 } else { -1 };
            let entry = edges.entry(key).or_default();
            entry.0 += 1;
            entry.1 += direction;
        }
    }
    if !edges
        .values()
        .all(|(count, direction)| *count == 2 && *direction == 0)
    {
        return false;
    }
    let mut adjacency = BTreeMap::<u32, Vec<u32>>::new();
    for (a, b) in edges.keys() {
        if cancelled() {
            return false;
        }
        adjacency.entry(*a).or_default().push(*b);
        adjacency.entry(*b).or_default().push(*a);
    }
    let Some(start) = adjacency.keys().next().copied() else {
        return false;
    };
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(vertex) = pending.pop() {
        if cancelled() {
            return false;
        }
        if visited.insert(vertex) {
            pending.extend(adjacency.get(&vertex).into_iter().flatten().copied());
        }
    }
    visited.len() == mesh.vertices_mm.len()
}

fn prism_evidence(
    mesh: &MeshBodySpec,
    axis: usize,
    tolerance_mm: f64,
    cancelled: &dyn Fn() -> bool,
) -> Option<PrismEvidence> {
    let low = mesh
        .vertices_mm
        .iter()
        .map(|point| point[axis])
        .min_by(f64::total_cmp)?;
    let high = mesh
        .vertices_mm
        .iter()
        .map(|point| point[axis])
        .max_by(f64::total_cmp)?;
    if high - low <= tolerance_mm {
        return None;
    }

    let mut layer_is_low = Vec::with_capacity(mesh.vertices_mm.len());
    let mut max_layer_distance: f64 = 0.0;
    for point in &mesh.vertices_mm {
        if cancelled() {
            return None;
        }
        let low_distance = (point[axis] - low).abs();
        let high_distance = (point[axis] - high).abs();
        let distance = low_distance.min(high_distance);
        if distance > tolerance_mm {
            return None;
        }
        max_layer_distance = max_layer_distance.max(distance);
        layer_is_low.push(low_distance <= high_distance);
    }

    let [u_axis, v_axis] = profile_axes(axis);
    let low_boundary = canonical_cycle(
        cap_boundary(mesh, &layer_is_low, true, cancelled)?,
        mesh,
        u_axis,
        v_axis,
        cancelled,
    )?;
    let high_boundary = canonical_cycle(
        cap_boundary(mesh, &layer_is_low, false, cancelled)?,
        mesh,
        u_axis,
        v_axis,
        cancelled,
    )?;
    if low_boundary.len() != high_boundary.len() || low_boundary.len() < 3 {
        return None;
    }
    let bucket = |point: [f64; 3]| {
        [
            tolerance_bucket_coordinate(point[u_axis], tolerance_mm),
            tolerance_bucket_coordinate(point[v_axis], tolerance_mm),
        ]
    };
    let mut high_buckets = BTreeMap::<[ToleranceBucketCoordinate; 2], Vec<u32>>::new();
    for high_index in &high_boundary {
        if cancelled() {
            return None;
        }
        high_buckets
            .entry(bucket(mesh.vertices_mm[*high_index as usize]))
            .or_default()
            .push(*high_index);
    }
    let mut pairing = BTreeMap::new();
    let mut used_high = BTreeSet::new();
    let mut max_pairing_distance: f64 = 0.0;
    for low_index in &low_boundary {
        if cancelled() {
            return None;
        }
        let low_point = mesh.vertices_mm[*low_index as usize];
        let low_bucket = bucket(low_point);
        let mut matched = None;
        for u in neighboring_bucket_coordinates(low_bucket[0])
            .into_iter()
            .flatten()
        {
            for v in neighboring_bucket_coordinates(low_bucket[1])
                .into_iter()
                .flatten()
            {
                for high_index in high_buckets.get(&[u, v]).into_iter().flatten() {
                    if cancelled() {
                        return None;
                    }
                    let high_point = mesh.vertices_mm[*high_index as usize];
                    let distance = ((low_point[u_axis] - high_point[u_axis]).powi(2)
                        + (low_point[v_axis] - high_point[v_axis]).powi(2))
                    .sqrt();
                    if distance <= tolerance_mm
                        && matched.replace((*high_index, distance)).is_some()
                    {
                        return None;
                    }
                }
            }
        }
        let (matched_index, distance) = matched?;
        if !used_high.insert(matched_index) {
            return None;
        }
        pairing.insert(*low_index, matched_index);
        max_pairing_distance = max_pairing_distance.max(distance);
    }
    if used_high.len() != high_boundary.len()
        || !side_band_matches(mesh, &layer_is_low, &low_boundary, &pairing, cancelled)
    {
        return None;
    }

    let profile = low_boundary
        .iter()
        .map(|index| {
            let point = mesh.vertices_mm[*index as usize];
            [point[u_axis], point[v_axis]]
        })
        .collect::<Vec<_>>();
    let center_2d = profile.iter().fold([0.0, 0.0], |mut center, point| {
        center[0] += point[0] / profile.len() as f64;
        center[1] += point[1] / profile.len() as f64;
        center
    });
    Some(PrismEvidence {
        extrusion_axis: axis,
        low,
        high,
        profile,
        center_2d,
        residuals: MeshRecognitionResiduals {
            tolerance_mm,
            max_layer_distance_mm: max_layer_distance,
            max_pairing_distance_mm: max_pairing_distance,
            max_profile_distance_mm: 0.0,
            max_surface_distance_mm: 0.0,
        },
    })
}

fn cap_boundary(
    mesh: &MeshBodySpec,
    layer_is_low: &[bool],
    low: bool,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<u32>> {
    let mut edge_counts = BTreeMap::<(u32, u32), usize>::new();
    for triangle in &mesh.triangles {
        if cancelled() {
            return None;
        }
        if triangle
            .iter()
            .all(|index| layer_is_low[*index as usize] == low)
        {
            for [a, b] in [
                [triangle[0], triangle[1]],
                [triangle[1], triangle[2]],
                [triangle[2], triangle[0]],
            ] {
                *edge_counts.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
    }
    let boundary_edges = edge_counts
        .into_iter()
        .filter_map(|(edge, count)| (count == 1).then_some(edge))
        .collect::<Vec<_>>();
    if boundary_edges.len() < 3 {
        return None;
    }
    let mut adjacency = BTreeMap::<u32, Vec<u32>>::new();
    for (a, b) in boundary_edges {
        if cancelled() {
            return None;
        }
        adjacency.entry(a).or_default().push(b);
        adjacency.entry(b).or_default().push(a);
    }
    if adjacency.values().any(|neighbors| neighbors.len() != 2) {
        return None;
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_unstable();
    }
    let start = *adjacency.keys().next()?;
    let mut cycle = vec![start];
    let mut visited = BTreeSet::from([start]);
    let mut previous = None;
    let mut current = start;
    loop {
        if cancelled() {
            return None;
        }
        let neighbors = adjacency.get(&current)?;
        let next = neighbors
            .iter()
            .copied()
            .find(|neighbor| Some(*neighbor) != previous)?;
        if next == start {
            break;
        }
        if cycle.len() >= adjacency.len() || !visited.insert(next) {
            return None;
        }
        cycle.push(next);
        previous = Some(current);
        current = next;
    }
    (cycle.len() == adjacency.len()).then_some(cycle)
}

fn canonical_cycle(
    cycle: Vec<u32>,
    mesh: &MeshBodySpec,
    u_axis: usize,
    v_axis: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<u32>> {
    let point = |index: u32| {
        let vertex = mesh.vertices_mm[index as usize];
        [vertex[u_axis], vertex[v_axis]]
    };
    let mut seen = BTreeSet::new();
    for index in &cycle {
        if cancelled() {
            return None;
        }
        let value = point(*index);
        if !seen.insert((value[0].to_bits(), value[1].to_bits())) {
            return None;
        }
    }
    let start = cycle
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| compare_2d(point(**left), point(**right)))?
        .0;
    let forward = (0..cycle.len())
        .map(|offset| cycle[(start + offset) % cycle.len()])
        .collect::<Vec<_>>();
    let reverse = (0..cycle.len())
        .map(|offset| cycle[(start + cycle.len() - offset) % cycle.len()])
        .collect::<Vec<_>>();
    let order = forward
        .iter()
        .zip(&reverse)
        .map(|(left, right)| compare_2d(point(*left), point(*right)))
        .find(|order| !order.is_eq())
        .unwrap_or(std::cmp::Ordering::Equal);
    Some(if order.is_gt() { reverse } else { forward })
}

fn compare_2d(left: [f64; 2], right: [f64; 2]) -> std::cmp::Ordering {
    left[0]
        .total_cmp(&right[0])
        .then_with(|| left[1].total_cmp(&right[1]))
}

fn side_band_matches(
    mesh: &MeshBodySpec,
    layer_is_low: &[bool],
    low_boundary: &[u32],
    pairing: &BTreeMap<u32, u32>,
    cancelled: &dyn Fn() -> bool,
) -> bool {
    let mut expected = Vec::<([u32; 4], usize)>::with_capacity(low_boundary.len());
    let mut edge_to_quad = BTreeMap::<(u32, u32), usize>::new();
    for index in 0..low_boundary.len() {
        if cancelled() {
            return false;
        }
        let low_a = low_boundary[index];
        let low_b = low_boundary[(index + 1) % low_boundary.len()];
        let Some(high_a) = pairing.get(&low_a).copied() else {
            return false;
        };
        let Some(high_b) = pairing.get(&low_b).copied() else {
            return false;
        };
        for (a, b) in [(low_a, low_b), (high_a, high_b)] {
            if edge_to_quad.insert((a.min(b), a.max(b)), index).is_some() {
                return false;
            }
        }
        let mut quad = [low_a, low_b, high_a, high_b];
        quad.sort_unstable();
        expected.push((quad, 0));
    }

    let mut mixed_triangle_count = 0;
    for triangle in &mesh.triangles {
        if cancelled() {
            return false;
        }
        let low_count = triangle
            .iter()
            .filter(|index| layer_is_low[**index as usize])
            .count();
        if low_count == 0 || low_count == 3 {
            continue;
        }
        mixed_triangle_count += 1;
        let same_layer_edge = [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ]
        .into_iter()
        .find(|(a, b)| layer_is_low[*a as usize] == layer_is_low[*b as usize]);
        let Some((a, b)) = same_layer_edge else {
            return false;
        };
        let Some(matched) = edge_to_quad.get(&(a.min(b), a.max(b))).copied() else {
            return false;
        };
        if !triangle
            .iter()
            .all(|vertex| expected[matched].0.contains(vertex))
        {
            return false;
        }
        expected[matched].1 += 1;
    }
    mixed_triangle_count == low_boundary.len() * 2 && expected.iter().all(|(_, count)| *count == 2)
}

fn box_candidate(
    evidence: &PrismEvidence,
    tolerance_mm: f64,
) -> Option<(MeshRecognitionCandidate, MeshRecognitionResiduals)> {
    if evidence.profile.len() != 4 {
        return None;
    }
    let p = &evidence.profile;
    let edge_u = subtract_2d(p[1], p[0]);
    let edge_v = subtract_2d(p[3], p[0]);
    let width = length_2d(edge_u);
    let depth = length_2d(edge_v);
    if width <= tolerance_mm || depth <= tolerance_mm {
        return None;
    }
    let expected_opposite = [p[1][0] + edge_v[0], p[1][1] + edge_v[1]];
    let closure_residual = distance_2d(p[2], expected_opposite);
    let perpendicular_residual = dot_2d(edge_u, edge_v).abs() / width.max(depth);
    let profile_residual = closure_residual.max(perpendicular_residual);
    if profile_residual > tolerance_mm {
        return None;
    }
    let [u_axis, v_axis] = profile_axes(evidence.extrusion_axis);
    let mut first_axis = [0.0; 3];
    first_axis[u_axis] = edge_u[0] / width;
    first_axis[v_axis] = edge_u[1] / width;
    let mut second_axis = [0.0; 3];
    second_axis[u_axis] = edge_v[0] / depth;
    second_axis[v_axis] = edge_v[1] / depth;
    let extrusion_axis = cardinal_axis(evidence.extrusion_axis);
    let axis_center = (evidence.low + evidence.high) * 0.5;
    let center_mm = combine_point(evidence.extrusion_axis, evidence.center_2d, axis_center);
    let mut residuals = evidence.residuals;
    residuals.max_profile_distance_mm = profile_residual;
    Some((
        MeshRecognitionCandidate::Box(BoxRecognition {
            center_mm,
            axes: [first_axis, second_axis, extrusion_axis],
            dimensions_mm: [width, depth, evidence.high - evidence.low],
        }),
        residuals,
    ))
}

fn cylinder_candidate(
    evidence: &PrismEvidence,
    tolerance_mm: f64,
    cancelled: &dyn Fn() -> bool,
) -> Option<(MeshRecognitionCandidate, MeshRecognitionResiduals)> {
    let side_count = evidence.profile.len();
    if side_count < MIN_CYLINDER_SIDES {
        return None;
    }
    let radii = evidence
        .profile
        .iter()
        .map(|point| distance_2d(*point, evidence.center_2d))
        .collect::<Vec<_>>();
    let radius = radii.iter().sum::<f64>() / radii.len() as f64;
    if radius <= tolerance_mm {
        return None;
    }
    let radial_residual = radii
        .iter()
        .map(|value| (value - radius).abs())
        .max_by(f64::total_cmp)?;
    let expected_angle = std::f64::consts::TAU / side_count as f64;
    let mut angular_residual: f64 = 0.0;
    let mut tessellation_residual: f64 = 0.0;
    for index in 0..evidence.profile.len() {
        if cancelled() {
            return None;
        }
        let first = subtract_2d(evidence.profile[index], evidence.center_2d);
        let second = subtract_2d(
            evidence.profile[(index + 1) % evidence.profile.len()],
            evidence.center_2d,
        );
        let angle = (dot_2d(first, second) / (length_2d(first) * length_2d(second)))
            .clamp(-1.0, 1.0)
            .acos();
        angular_residual = angular_residual.max((angle - expected_angle).abs() * radius);
        let edge = subtract_2d(second, first);
        let projection = (-dot_2d(first, edge) / dot_2d(edge, edge)).clamp(0.0, 1.0);
        let nearest = [
            first[0] + projection * edge[0],
            first[1] + projection * edge[1],
        ];
        tessellation_residual = tessellation_residual.max((radius - length_2d(nearest)).abs());
    }
    let profile_residual = radial_residual.max(angular_residual);
    let surface_residual = profile_residual.max(tessellation_residual);
    if surface_residual > tolerance_mm {
        return None;
    }
    let axis_center = (evidence.low + evidence.high) * 0.5;
    let center_mm = combine_point(evidence.extrusion_axis, evidence.center_2d, axis_center);
    let mut residuals = evidence.residuals;
    residuals.max_profile_distance_mm = profile_residual;
    residuals.max_surface_distance_mm = surface_residual;
    Some((
        MeshRecognitionCandidate::Cylinder(CylinderRecognition {
            center_mm,
            axis: cardinal_axis(evidence.extrusion_axis),
            radius_mm: radius,
            height_mm: evidence.high - evidence.low,
            tessellated_side_count: side_count,
        }),
        residuals,
    ))
}

fn extrusion_candidate(evidence: &PrismEvidence) -> MeshRecognitionCandidate {
    let [u_axis, v_axis] = profile_axes(evidence.extrusion_axis);
    let mut basis_u = [0.0; 3];
    basis_u[u_axis] = 1.0;
    let mut basis_v = [0.0; 3];
    basis_v[v_axis] = 1.0;
    MeshRecognitionCandidate::LinearExtrusion(LinearExtrusionRecognition {
        base_origin_mm: combine_point(evidence.extrusion_axis, evidence.center_2d, evidence.low),
        profile_basis: [basis_u, basis_v],
        axis: cardinal_axis(evidence.extrusion_axis),
        height_mm: evidence.high - evidence.low,
        profile_mm: evidence
            .profile
            .iter()
            .map(|point| {
                [
                    point[0] - evidence.center_2d[0],
                    point[1] - evidence.center_2d[1],
                ]
            })
            .collect(),
    })
}

fn profile_axes(extrusion_axis: usize) -> [usize; 2] {
    match extrusion_axis {
        0 => [1, 2],
        1 => [0, 2],
        2 => [0, 1],
        _ => unreachable!("axis is generated from 0..3"),
    }
}

fn cardinal_axis(axis: usize) -> [f64; 3] {
    let mut result = [0.0; 3];
    result[axis] = 1.0;
    result
}

fn combine_point(axis: usize, profile: [f64; 2], axis_value: f64) -> [f64; 3] {
    let [u_axis, v_axis] = profile_axes(axis);
    let mut point = [0.0; 3];
    point[axis] = axis_value;
    point[u_axis] = profile[0];
    point[v_axis] = profile[1];
    point
}

fn subtract_2d(left: [f64; 2], right: [f64; 2]) -> [f64; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

fn dot_2d(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[0] + left[1] * right[1]
}

fn length_2d(value: [f64; 2]) -> f64 {
    dot_2d(value, value).sqrt()
}

fn distance_2d(left: [f64; 2], right: [f64; 2]) -> f64 {
    length_2d(subtract_2d(left, right))
}
