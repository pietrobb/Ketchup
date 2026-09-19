#![forbid(unsafe_code)]

use crate::spatial::{
    SPATIAL_INDEX_V1, SnapshotBinding, SpatialIndex, SpatialQueryError, SpatialQueryStats, cross,
    ray_triangle_distance, transform_point, transformed_bounds,
};
use crate::{Ray, Vec3};
use ketchup_core::document::{
    DefinitionId, FeatureId, FeatureKind, InstancePath, ProfileSegment, Snapshot, Transform,
};
use ketchup_core::sketch::SolvedSketchRegionProfile;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const NORMAL_EPSILON: f64 = 1.0e-12;

pub type CanonicalPlanarProfileMesh = (FeatureId, Vec<[f64; 3]>, Vec<[u32; 3]>);

#[derive(Clone, Debug, PartialEq)]
pub struct MeshSurfaceHit {
    pub definition_id: DefinitionId,
    pub feature_id: FeatureId,
    pub instance_path: InstancePath,
    pub triangle_index: usize,
    pub position_mm: Vec3,
    pub outward_normal: Vec3,
    pub ray_distance_mm: f64,
}

#[derive(Clone, Debug)]
struct MeshGeometry {
    definition_id: DefinitionId,
    feature_id: FeatureId,
    vertices_mm: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    bounds_mm: [[f64; 3]; 2],
}

#[derive(Clone, Debug)]
struct MeshOccurrence {
    instance_path: InstancePath,
    transform: Transform,
    geometry: Arc<MeshGeometry>,
}

#[derive(Clone, Copy, Debug)]
struct PhysicalMeshHit<'a> {
    occurrence: &'a MeshOccurrence,
    triangle_index: usize,
    ray_distance_mm: f64,
}

#[derive(Clone, Debug)]
pub struct MeshInteractionProjection {
    binding: SnapshotBinding,
    occurrences: Vec<MeshOccurrence>,
    occurrence_paths: BTreeSet<InstancePath>,
    spatial_index: SpatialIndex,
}

impl MeshInteractionProjection {
    #[must_use]
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self::from_snapshot_where(snapshot, |_| true)
    }

    #[must_use]
    pub fn from_snapshot_where(
        snapshot: &Snapshot,
        include: impl Fn(&InstancePath) -> bool,
    ) -> Self {
        let geometries = snapshot
            .definitions()
            .filter_map(|definition| {
                let canonical_mesh = match definition.feature_ids() {
                    [feature_id] => match snapshot.feature(*feature_id)?.kind() {
                        FeatureKind::MeshBody(spec) => Some((
                            *feature_id,
                            spec.vertices_mm.clone(),
                            spec.triangles.clone(),
                        )),
                        _ => None,
                    },
                    _ => None,
                };
                let (feature_id, vertices_mm, triangles) = canonical_mesh.or_else(|| {
                    crate::profile_surface::canonical_definition_surface(snapshot, definition.id())
                })?;
                let bounds_mm = mesh_bounds(&vertices_mm)?;
                Some((
                    definition.id(),
                    Arc::new(MeshGeometry {
                        definition_id: definition.id(),
                        feature_id,
                        vertices_mm,
                        triangles,
                        bounds_mm,
                    }),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let occurrences = snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .filter(|occurrence| include(&occurrence.instance_path))
            .filter_map(|occurrence| {
                Some(MeshOccurrence {
                    instance_path: occurrence.instance_path,
                    transform: occurrence.transform,
                    geometry: Arc::clone(geometries.get(&occurrence.definition_id)?),
                })
            })
            .collect::<Vec<_>>();
        let occurrence_paths = occurrences
            .iter()
            .map(|occurrence| occurrence.instance_path.clone())
            .collect();
        let spatial_index =
            SpatialIndex::build(occurrences.iter().enumerate().map(|(index, occurrence)| {
                (
                    index,
                    transformed_bounds(occurrence.transform, occurrence.geometry.bounds_mm),
                )
            }));
        Self {
            binding: SnapshotBinding::from_snapshot(snapshot),
            occurrences,
            occurrence_paths,
            spatial_index,
        }
    }

    #[must_use]
    pub const fn spatial_index_schema(&self) -> &'static str {
        SPATIAL_INDEX_V1
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.binding.is_current(snapshot)
    }

    #[must_use]
    pub fn occurrence_count(&self) -> usize {
        self.occurrences.len()
    }

    #[must_use]
    pub fn contains_occurrence(&self, instance_path: &InstancePath) -> bool {
        self.occurrence_paths.contains(instance_path)
    }

    #[must_use]
    pub fn shared_geometry_count(&self) -> usize {
        let mut pointers = self
            .occurrences
            .iter()
            .map(|occurrence| Arc::as_ptr(&occurrence.geometry))
            .collect::<Vec<_>>();
        pointers.sort_unstable();
        pointers.dedup();
        pointers.len()
    }

    #[must_use]
    pub fn exact_surface_pick(&self, ray: Ray) -> Option<MeshSurfaceHit> {
        self.exact_surface_pick_with_stats(ray).0
    }

    #[must_use]
    pub fn surface_pick_with_tolerance(
        &self,
        ray: Ray,
        tolerance_mm: f64,
    ) -> Option<MeshSurfaceHit> {
        self.surface_pick_with_tolerance_with_stats(ray, tolerance_mm)
            .0
    }

    #[must_use]
    pub fn exact_surface_pick_with_stats(
        &self,
        ray: Ray,
    ) -> (Option<MeshSurfaceHit>, SpatialQueryStats) {
        self.surface_pick_with_tolerance_with_stats(ray, 0.0)
    }

    fn surface_pick_with_tolerance_with_stats(
        &self,
        ray: Ray,
        tolerance_mm: f64,
    ) -> (Option<MeshSurfaceHit>, SpatialQueryStats) {
        let tolerance_mm = if tolerance_mm.is_finite() {
            tolerance_mm.max(0.0)
        } else {
            0.0
        };
        let (candidate_indices, stats) = self
            .spatial_index
            .query_ray_with_tolerance(ray, tolerance_mm);
        let hit = candidate_indices
            .into_iter()
            .filter_map(|index| hit_occurrence(ray, &self.occurrences[index], tolerance_mm))
            .min_by(|left, right| {
                left.ray_distance_mm
                    .total_cmp(&right.ray_distance_mm)
                    .then_with(|| {
                        left.occurrence
                            .instance_path
                            .cmp(&right.occurrence.instance_path)
                    })
                    .then_with(|| left.triangle_index.cmp(&right.triangle_index))
            });
        let Some(hit) = hit else {
            return (None, stats);
        };
        let triangle = hit.occurrence.geometry.triangles[hit.triangle_index];
        let [first, second, third] = triangle.map(|index| {
            let position = hit.occurrence.geometry.vertices_mm[index as usize];
            transform_point(
                hit.occurrence.transform,
                Vec3::new(position[0], position[1], position[2]),
            )
        });
        let normal = cross(second - first, third - first);
        let normal_length = normal.length();
        if normal_length <= NORMAL_EPSILON {
            return (None, stats);
        }
        (
            Some(MeshSurfaceHit {
                definition_id: hit.occurrence.geometry.definition_id,
                feature_id: hit.occurrence.geometry.feature_id,
                instance_path: hit.occurrence.instance_path.clone(),
                triangle_index: hit.triangle_index,
                position_mm: ray.at(hit.ray_distance_mm),
                outward_normal: normal * (1.0 / normal_length),
                ray_distance_mm: hit.ray_distance_mm,
            }),
            stats,
        )
    }

    pub fn exact_surface_picks(&self, ray: Ray) -> Vec<MeshSurfaceHit> {
        let (candidates, _) = self.spatial_index.query_ray_with_tolerance(ray, 0.0);
        let mut hits = candidates
            .into_iter()
            .filter_map(|index| {
                let hit = hit_occurrence(ray, &self.occurrences[index], 0.0)?;
                let triangle = hit.occurrence.geometry.triangles[hit.triangle_index];
                let [a, b, c] = triangle.map(|index| {
                    let p = hit.occurrence.geometry.vertices_mm[index as usize];
                    transform_point(hit.occurrence.transform, Vec3::new(p[0], p[1], p[2]))
                });
                let normal = cross(b - a, c - a);
                let length = normal.length();
                (length > NORMAL_EPSILON).then(|| MeshSurfaceHit {
                    definition_id: hit.occurrence.geometry.definition_id,
                    feature_id: hit.occurrence.geometry.feature_id,
                    instance_path: hit.occurrence.instance_path.clone(),
                    triangle_index: hit.triangle_index,
                    position_mm: ray.at(hit.ray_distance_mm),
                    outward_normal: normal * (1.0 / length),
                    ray_distance_mm: hit.ray_distance_mm,
                })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|a, b| {
            a.ray_distance_mm
                .total_cmp(&b.ray_distance_mm)
                .then_with(|| a.instance_path.cmp(&b.instance_path))
                .then_with(|| a.triangle_index.cmp(&b.triangle_index))
        });
        hits
    }

    pub fn exact_surface_pick_current(
        &self,
        snapshot: &Snapshot,
        ray: Ray,
    ) -> Result<Option<MeshSurfaceHit>, SpatialQueryError> {
        if !self.is_current(snapshot) {
            return Err(SpatialQueryError::StaleProjection);
        }
        Ok(self.exact_surface_pick(ray))
    }
}

pub fn canonical_planar_profile_mesh(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> Option<CanonicalPlanarProfileMesh> {
    let definition = snapshot.definition(definition_id)?;
    let feature_id = *definition.feature_ids().last()?;
    canonical_profile_feature_mesh(snapshot, feature_id)
}

pub fn canonical_profile_feature_mesh(
    snapshot: &Snapshot,
    feature_id: FeatureId,
) -> Option<CanonicalPlanarProfileMesh> {
    let feature = snapshot.feature(feature_id)?;
    match feature.kind() {
        FeatureKind::SegmentProfile {
            segments,
            closed: true,
        } => {
            let (positions, triangles) = segment_profile_mesh(segments)?;
            Some((feature_id, positions, triangles))
        }
        FeatureKind::Sketch(_) => canonical_sketch_feature_mesh(snapshot, feature_id),
        _ => None,
    }
}

pub fn canonical_sketch_profile_mesh(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> Option<CanonicalPlanarProfileMesh> {
    let definition = snapshot.definition(definition_id)?;
    let sketch_feature_id = *definition.feature_ids().last()?;
    canonical_sketch_feature_mesh(snapshot, sketch_feature_id)
}

fn canonical_sketch_feature_mesh(
    snapshot: &Snapshot,
    sketch_feature_id: FeatureId,
) -> Option<CanonicalPlanarProfileMesh> {
    let feature = snapshot.feature(sketch_feature_id)?;
    let definition_id = feature.definition_id();
    let FeatureKind::Sketch(sketch) = feature.kind() else {
        return None;
    };
    let workplane = snapshot.feature(sketch.workplane)?;
    if workplane.definition_id() != definition_id {
        return None;
    }
    let FeatureKind::Workplane(workplane) = workplane.kind() else {
        return None;
    };
    let [region] = sketch.solved_regions().ok()?.try_into().ok()?;
    if !region.holes.is_empty() {
        return None;
    }
    let SolvedSketchRegionProfile::Polyline(points) = region.outer else {
        return None;
    };
    let triangles = oriented_triangles(&points)?;
    let frame = workplane.frame;
    let vertices = points
        .into_iter()
        .map(|point| {
            [
                frame.origin_mm[0] + frame.x_axis[0] * point[0] + frame.y_axis[0] * point[1],
                frame.origin_mm[1] + frame.x_axis[1] * point[0] + frame.y_axis[1] * point[1],
                frame.origin_mm[2] + frame.x_axis[2] * point[0] + frame.y_axis[2] * point[1],
            ]
        })
        .collect();
    Some((sketch_feature_id, vertices, triangles))
}

pub fn segment_profile_mesh(
    segments: &[ProfileSegment],
) -> Option<crate::profile_surface::SurfaceMesh> {
    let points = segment_profile_boundary(segments)?;
    let triangles = oriented_triangles(&points)?;
    Some((
        points
            .into_iter()
            .map(|point| [point[0], point[1], 0.0])
            .collect(),
        triangles,
    ))
}

fn segment_profile_boundary(segments: &[ProfileSegment]) -> Option<Vec<[f64; 2]>> {
    let mut boundary = Vec::new();
    for segment in segments {
        let sampled = match segment {
            ProfileSegment::Line { start_mm, end_mm } => vec![*start_mm, *end_mm],
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => sample_arc(*start_mm, *end_mm, *center_mm, *clockwise)?,
            ProfileSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => (0..=32)
                .map(|step| {
                    let t = f64::from(step) / 32.0;
                    let inverse = 1.0 - t;
                    [
                        inverse.powi(3) * start_mm[0]
                            + 3.0 * inverse.powi(2) * t * control_1_mm[0]
                            + 3.0 * inverse * t.powi(2) * control_2_mm[0]
                            + t.powi(3) * end_mm[0],
                        inverse.powi(3) * start_mm[1]
                            + 3.0 * inverse.powi(2) * t * control_1_mm[1]
                            + 3.0 * inverse * t.powi(2) * control_2_mm[1]
                            + t.powi(3) * end_mm[1],
                    ]
                })
                .collect(),
        };
        if boundary.is_empty() {
            boundary.extend(sampled);
        } else {
            if !same_point(*boundary.last()?, sampled[0]) {
                return None;
            }
            boundary.extend(sampled.into_iter().skip(1));
        }
    }
    if boundary.len() < 4 || !same_point(boundary[0], *boundary.last()?) {
        return None;
    }
    boundary.pop();
    boundary.dedup_by(|left, right| same_point(*left, *right));
    Some(boundary)
}

fn sample_arc(
    start_mm: [f64; 2],
    end_mm: [f64; 2],
    center_mm: [f64; 2],
    clockwise: bool,
) -> Option<Vec<[f64; 2]>> {
    let start_radius = [start_mm[0] - center_mm[0], start_mm[1] - center_mm[1]];
    let end_radius = [end_mm[0] - center_mm[0], end_mm[1] - center_mm[1]];
    let radius = start_radius[0].hypot(start_radius[1]);
    let end_length = end_radius[0].hypot(end_radius[1]);
    if !radius.is_finite()
        || radius <= 1.0e-9
        || (radius - end_length).abs() > 1.0e-8 * radius.max(end_length).max(1.0)
    {
        return None;
    }
    let start_angle = start_radius[1].atan2(start_radius[0]);
    let end_angle = end_radius[1].atan2(end_radius[0]);
    let mut sweep = end_angle - start_angle;
    if clockwise {
        while sweep >= 0.0 {
            sweep -= std::f64::consts::TAU;
        }
    } else {
        while sweep <= 0.0 {
            sweep += std::f64::consts::TAU;
        }
    }
    let steps = ((sweep.abs() / std::f64::consts::TAU * 64.0 - 1.0e-9).ceil() as usize).max(1);
    Some(
        (0..=steps)
            .map(|step| {
                let angle = start_angle + sweep * step as f64 / steps as f64;
                [
                    center_mm[0] + radius * angle.cos(),
                    center_mm[1] + radius * angle.sin(),
                ]
            })
            .collect(),
    )
}

fn same_point(left: [f64; 2], right: [f64; 2]) -> bool {
    (left[0] - right[0]).abs() <= 1.0e-8 && (left[1] - right[1]).abs() <= 1.0e-8
}

fn oriented_triangles(points: &[[f64; 2]]) -> Option<Vec<[u32; 3]>> {
    let mut triangles = triangulate_polygon(points)?;
    if polygon_area(points) < 0.0 {
        for triangle in &mut triangles {
            triangle.swap(1, 2);
        }
    }
    Some(triangles)
}

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
        .sum::<f64>()
        * 0.5
}

fn triangulate_polygon(points: &[[f64; 2]]) -> Option<Vec<[u32; 3]>> {
    let orientation = polygon_area(points).signum();
    if points.len() < 3 || orientation == 0.0 {
        return None;
    }
    let mut remaining = (0..points.len()).collect::<Vec<_>>();
    let mut triangles = Vec::with_capacity(points.len() - 2);
    while remaining.len() > 3 {
        let mut ear = None;
        for index in 0..remaining.len() {
            let previous = remaining[(index + remaining.len() - 1) % remaining.len()];
            let current = remaining[index];
            let next = remaining[(index + 1) % remaining.len()];
            if triangle_cross(points[previous], points[current], points[next]) * orientation
                <= 1.0e-12
            {
                continue;
            }
            if remaining.iter().copied().any(|candidate| {
                candidate != previous
                    && candidate != current
                    && candidate != next
                    && point_in_triangle(
                        points[candidate],
                        points[previous],
                        points[current],
                        points[next],
                    )
            }) {
                continue;
            }
            ear = Some((index, [previous as u32, current as u32, next as u32]));
            break;
        }
        let (index, triangle) = ear?;
        triangles.push(triangle);
        remaining.remove(index);
    }
    triangles.push([
        remaining[0] as u32,
        remaining[1] as u32,
        remaining[2] as u32,
    ]);
    Some(triangles)
}

fn triangle_cross(left: [f64; 2], middle: [f64; 2], right: [f64; 2]) -> f64 {
    (middle[0] - left[0]) * (right[1] - left[1]) - (middle[1] - left[1]) * (right[0] - left[0])
}

fn point_in_triangle(point: [f64; 2], first: [f64; 2], second: [f64; 2], third: [f64; 2]) -> bool {
    let crosses = [
        triangle_cross(first, second, point),
        triangle_cross(second, third, point),
        triangle_cross(third, first, point),
    ];
    let has_negative = crosses.iter().any(|value| *value < -1.0e-12);
    let has_positive = crosses.iter().any(|value| *value > 1.0e-12);
    !(has_negative && has_positive)
}

fn mesh_bounds(vertices: &[[f64; 3]]) -> Option<[[f64; 3]; 2]> {
    let first = *vertices.first()?;
    let mut minimum = first;
    let mut maximum = first;
    for vertex in &vertices[1..] {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(vertex[axis]);
            maximum[axis] = maximum[axis].max(vertex[axis]);
        }
    }
    Some([minimum, maximum])
}

fn hit_occurrence<'a>(
    ray: Ray,
    occurrence: &'a MeshOccurrence,
    tolerance_mm: f64,
) -> Option<PhysicalMeshHit<'a>> {
    occurrence
        .geometry
        .triangles
        .iter()
        .enumerate()
        .filter_map(|(triangle_index, triangle)| {
            let [first, second, third] = triangle.map(|index| {
                let position = occurrence.geometry.vertices_mm[index as usize];
                transform_point(
                    occurrence.transform,
                    Vec3::new(position[0], position[1], position[2]),
                )
            });
            Some(PhysicalMeshHit {
                occurrence,
                triangle_index,
                ray_distance_mm: ray_triangle_distance_with_tolerance(
                    ray,
                    first,
                    second,
                    third,
                    tolerance_mm,
                )?,
            })
        })
        .min_by(|left, right| {
            left.ray_distance_mm
                .total_cmp(&right.ray_distance_mm)
                .then_with(|| left.triangle_index.cmp(&right.triangle_index))
        })
}

fn ray_triangle_distance_with_tolerance(
    ray: Ray,
    first: Vec3,
    second: Vec3,
    third: Vec3,
    tolerance_mm: f64,
) -> Option<f64> {
    if let Some(distance) = ray_triangle_distance(ray, first, second, third) {
        return Some(distance);
    }
    if tolerance_mm <= 0.0 {
        return None;
    }
    let normal = cross(second - first, third - first);
    let denominator = dot(normal, ray.direction);
    if denominator.abs() <= NORMAL_EPSILON {
        return None;
    }
    let distance = dot(normal, first - ray.origin) / denominator;
    if distance < 0.0 || !distance.is_finite() {
        return None;
    }
    let point = ray.at(distance);
    let edge_distance = [
        point_segment_distance(point, first, second),
        point_segment_distance(point, second, third),
        point_segment_distance(point, third, first),
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min);
    (edge_distance <= tolerance_mm).then_some(distance)
}

fn point_segment_distance(point: Vec3, start: Vec3, end: Vec3) -> f64 {
    let segment = end - start;
    let length_squared = dot(segment, segment);
    if length_squared <= NORMAL_EPSILON {
        return point.distance(start);
    }
    let parameter = (dot(point - start, segment) / length_squared).clamp(0.0, 1.0);
    point.distance(start + segment * parameter)
}

fn dot(left: Vec3, right: Vec3) -> f64 {
    left.x * right.x + left.y * right.y + left.z * right.z
}
