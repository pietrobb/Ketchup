#![forbid(unsafe_code)]

use crate::spatial::{
    SPATIAL_INDEX_V1, SnapshotBinding, SpatialIndex, SpatialQueryError, SpatialQueryStats, cross,
    ray_triangle_distance, transform_point, transformed_bounds,
};
use crate::{Ray, Vec3};
use ketchup_core::document::{
    DefinitionId, FeatureId, FeatureKind, InstancePath, Snapshot, Transform,
};
use std::collections::BTreeMap;
use std::sync::Arc;

const NORMAL_EPSILON: f64 = 1.0e-12;

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
    spatial_index: SpatialIndex,
}

impl MeshInteractionProjection {
    #[must_use]
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        let geometries = snapshot
            .definitions()
            .filter_map(|definition| {
                let [feature_id] = definition.feature_ids() else {
                    return None;
                };
                let FeatureKind::MeshBody(spec) = snapshot.feature(*feature_id)?.kind() else {
                    return None;
                };
                let bounds_mm = mesh_bounds(&spec.vertices_mm)?;
                Some((
                    definition.id(),
                    Arc::new(MeshGeometry {
                        definition_id: definition.id(),
                        feature_id: *feature_id,
                        vertices_mm: spec.vertices_mm.clone(),
                        triangles: spec.triangles.clone(),
                        bounds_mm,
                    }),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let occurrences = snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .filter_map(|occurrence| {
                Some(MeshOccurrence {
                    instance_path: occurrence.instance_path,
                    transform: occurrence.transform,
                    geometry: Arc::clone(geometries.get(&occurrence.definition_id)?),
                })
            })
            .collect::<Vec<_>>();
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
    pub fn exact_surface_pick_with_stats(
        &self,
        ray: Ray,
    ) -> (Option<MeshSurfaceHit>, SpatialQueryStats) {
        let (candidate_indices, stats) = self.spatial_index.query_ray(ray);
        let hit = candidate_indices
            .into_iter()
            .filter_map(|index| hit_occurrence(ray, &self.occurrences[index]))
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

fn hit_occurrence<'a>(ray: Ray, occurrence: &'a MeshOccurrence) -> Option<PhysicalMeshHit<'a>> {
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
                ray_distance_mm: ray_triangle_distance(ray, first, second, third)?,
            })
        })
        .min_by(|left, right| {
            left.ray_distance_mm
                .total_cmp(&right.ray_distance_mm)
                .then_with(|| left.triangle_index.cmp(&right.triangle_index))
        })
}
