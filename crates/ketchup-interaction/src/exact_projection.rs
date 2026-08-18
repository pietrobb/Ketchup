#![forbid(unsafe_code)]

use crate::spatial::{
    SPATIAL_INDEX_V1, SnapshotBinding, SpatialBounds, SpatialIndex, SpatialQueryError,
    SpatialQueryStats,
};
use crate::{Ray, Vec3};
use ketchup_core::document::{DefinitionId, InstancePath, Snapshot, Transform};
use ketchup_core::exact_product::{AssemblySelectionTarget, ExactBodyPackage, ExactResultRegistry};
use std::collections::BTreeSet;
use std::sync::Arc;

const RAY_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Debug, PartialEq)]
pub struct DurableExactHit {
    pub target: AssemblySelectionTarget,
    pub position_mm: Vec3,
    pub ray_distance_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactSurfaceHit {
    pub definition_id: DefinitionId,
    pub instance_path: InstancePath,
    pub durable_target: Option<AssemblySelectionTarget>,
    pub position_mm: Vec3,
    pub outward_normal: Vec3,
    pub ray_distance_mm: f64,
}

#[derive(Clone, Debug)]
struct ExactOccurrence {
    instance_path: InstancePath,
    transform: Transform,
    package: Arc<ExactBodyPackage>,
}

#[derive(Clone, Copy, Debug)]
struct PhysicalTriangleHit<'a> {
    occurrence: &'a ExactOccurrence,
    triangle_index: usize,
    ray_distance_mm: f64,
}

#[derive(Clone, Debug)]
pub struct ExactInteractionProjection {
    binding: SnapshotBinding,
    occurrences: Vec<ExactOccurrence>,
    occurrence_paths: BTreeSet<InstancePath>,
    spatial_index: SpatialIndex,
}

impl ExactInteractionProjection {
    #[must_use]
    pub fn from_snapshot(snapshot: &Snapshot, results: &ExactResultRegistry) -> Self {
        Self::from_snapshot_where(snapshot, results, |_| true)
    }

    #[must_use]
    pub fn from_snapshot_where(
        snapshot: &Snapshot,
        results: &ExactResultRegistry,
        include: impl Fn(&InstancePath) -> bool,
    ) -> Self {
        let render_packages = results.render_by_definition(snapshot);
        let occurrences = snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .filter(|occurrence| include(&occurrence.instance_path))
            .filter_map(|occurrence| {
                let package = Arc::clone(render_packages.get(&occurrence.definition_id)?);
                if !package.is_current(snapshot)
                    || package.definition_id() != occurrence.definition_id
                {
                    return None;
                }
                Some(ExactOccurrence {
                    instance_path: occurrence.instance_path,
                    transform: occurrence.transform,
                    package,
                })
            })
            .collect::<Vec<_>>();
        let occurrence_paths = occurrences
            .iter()
            .map(|occurrence| occurrence.instance_path.clone())
            .collect();
        let spatial_index =
            SpatialIndex::build(occurrences.iter().enumerate().map(|(index, occurrence)| {
                let bounds =
                    transformed_bounds(occurrence.transform, occurrence.package.bounds_mm());
                (
                    index,
                    SpatialBounds::from_min_max(
                        Vec3::new(bounds[0][0], bounds[0][1], bounds[0][2]),
                        Vec3::new(bounds[1][0], bounds[1][1], bounds[1][2]),
                    ),
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
    pub fn exact_surface_pick(&self, ray: Ray) -> Option<ExactSurfaceHit> {
        self.exact_surface_pick_with_stats(ray).0
    }

    #[must_use]
    pub fn exact_surface_pick_with_stats(
        &self,
        ray: Ray,
    ) -> (Option<ExactSurfaceHit>, SpatialQueryStats) {
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
        let triangle = &hit.occurrence.package.triangles()[hit.triangle_index];
        let [first, second, third] = triangle.vertex_indices.map(|index| {
            let position = hit.occurrence.package.vertices()[index as usize].position_mm;
            transform_point(
                hit.occurrence.transform,
                Vec3::new(position[0], position[1], position[2]),
            )
        });
        let normal = cross(second - first, third - first);
        let normal_length = normal.length();
        if normal_length <= RAY_EPSILON {
            return (None, stats);
        }
        let outward_normal = Vec3::new(
            normal.x / normal_length,
            normal.y / normal_length,
            normal.z / normal_length,
        );
        let durable_target =
            triangle.face_role.and_then(|role| {
                hit.occurrence.package.reference(role).cloned().map(|body| {
                    AssemblySelectionTarget {
                        instance_path: hit.occurrence.instance_path.clone(),
                        body,
                    }
                })
            });
        (
            Some(ExactSurfaceHit {
                definition_id: hit.occurrence.package.definition_id(),
                instance_path: hit.occurrence.instance_path.clone(),
                durable_target,
                position_mm: ray.at(hit.ray_distance_mm),
                outward_normal,
                ray_distance_mm: hit.ray_distance_mm,
            }),
            stats,
        )
    }

    pub fn exact_surface_pick_current(
        &self,
        snapshot: &Snapshot,
        ray: Ray,
    ) -> Result<Option<ExactSurfaceHit>, SpatialQueryError> {
        if !self.is_current(snapshot) {
            return Err(SpatialQueryError::StaleProjection);
        }
        Ok(self.exact_surface_pick(ray))
    }

    #[must_use]
    pub fn exact_pick(&self, ray: Ray) -> Option<DurableExactHit> {
        let hit = self.exact_surface_pick(ray)?;
        Some(DurableExactHit {
            target: hit.durable_target?,
            position_mm: hit.position_mm,
            ray_distance_mm: hit.ray_distance_mm,
        })
    }
}

fn hit_occurrence<'a>(
    ray: Ray,
    occurrence: &'a ExactOccurrence,
) -> Option<PhysicalTriangleHit<'a>> {
    if !ray_intersects_bounds(
        ray,
        transformed_bounds(occurrence.transform, occurrence.package.bounds_mm()),
    ) {
        return None;
    }
    occurrence
        .package
        .triangles()
        .iter()
        .enumerate()
        .filter_map(|(triangle_index, triangle)| {
            let [first, second, third] = triangle.vertex_indices.map(|index| {
                let position = occurrence.package.vertices()[index as usize].position_mm;
                transform_point(
                    occurrence.transform,
                    Vec3::new(position[0], position[1], position[2]),
                )
            });
            let ray_distance_mm = ray_triangle_distance(ray, first, second, third)?;
            Some(PhysicalTriangleHit {
                occurrence,
                triangle_index,
                ray_distance_mm,
            })
        })
        .min_by(|left, right| {
            left.ray_distance_mm
                .total_cmp(&right.ray_distance_mm)
                .then_with(|| left.triangle_index.cmp(&right.triangle_index))
        })
}

fn transformed_bounds(transform: Transform, bounds: [[f64; 3]; 2]) -> [[f64; 3]; 2] {
    let corners = [
        [bounds[0][0], bounds[0][1], bounds[0][2]],
        [bounds[1][0], bounds[0][1], bounds[0][2]],
        [bounds[0][0], bounds[1][1], bounds[0][2]],
        [bounds[1][0], bounds[1][1], bounds[0][2]],
        [bounds[0][0], bounds[0][1], bounds[1][2]],
        [bounds[1][0], bounds[0][1], bounds[1][2]],
        [bounds[0][0], bounds[1][1], bounds[1][2]],
        [bounds[1][0], bounds[1][1], bounds[1][2]],
    ]
    .map(|point| transform_point(transform, Vec3::new(point[0], point[1], point[2])));
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for point in corners {
        for (axis, value) in [point.x, point.y, point.z].into_iter().enumerate() {
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    [min, max]
}

fn transform_point(transform: Transform, point: Vec3) -> Vec3 {
    let matrix = transform.matrix();
    Vec3::new(
        matrix[0] * point.x + matrix[1] * point.y + matrix[2] * point.z + matrix[3],
        matrix[4] * point.x + matrix[5] * point.y + matrix[6] * point.z + matrix[7],
        matrix[8] * point.x + matrix[9] * point.y + matrix[10] * point.z + matrix[11],
    )
}

fn ray_intersects_bounds(ray: Ray, bounds: [[f64; 3]; 2]) -> bool {
    let origins = [ray.origin.x, ray.origin.y, ray.origin.z];
    let directions = [ray.direction.x, ray.direction.y, ray.direction.z];
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::INFINITY;

    for axis in 0..3 {
        if directions[axis].abs() <= RAY_EPSILON {
            if origins[axis] < bounds[0][axis] || origins[axis] > bounds[1][axis] {
                return false;
            }
            continue;
        }
        let first = (bounds[0][axis] - origins[axis]) / directions[axis];
        let second = (bounds[1][axis] - origins[axis]) / directions[axis];
        near = near.max(first.min(second));
        far = far.min(first.max(second));
        if far < near {
            return false;
        }
    }
    far >= 0.0
}

fn ray_triangle_distance(ray: Ray, first: Vec3, second: Vec3, third: Vec3) -> Option<f64> {
    let first_edge = second - first;
    let second_edge = third - first;
    let determinant_vector = cross(ray.direction, second_edge);
    let determinant = dot(first_edge, determinant_vector);
    if determinant.abs() <= RAY_EPSILON {
        return None;
    }
    let inverse_determinant = 1.0 / determinant;
    let origin_offset = ray.origin - first;
    let first_weight = dot(origin_offset, determinant_vector) * inverse_determinant;
    if !(-RAY_EPSILON..=1.0 + RAY_EPSILON).contains(&first_weight) {
        return None;
    }
    let weight_vector = cross(origin_offset, first_edge);
    let second_weight = dot(ray.direction, weight_vector) * inverse_determinant;
    if second_weight < -RAY_EPSILON || first_weight + second_weight > 1.0 + RAY_EPSILON {
        return None;
    }
    let distance = dot(second_edge, weight_vector) * inverse_determinant;
    (distance >= 0.0 && distance.is_finite()).then_some(distance)
}

fn dot(left: Vec3, right: Vec3) -> f64 {
    left.x * right.x + left.y * right.y + left.z * right.z
}

fn cross(left: Vec3, right: Vec3) -> Vec3 {
    Vec3::new(
        left.y * right.z - left.z * right.y,
        left.z * right.x - left.x * right.z,
        left.x * right.y - left.y * right.x,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_core::document::{
        CanonicalCommand, CommandBatch, Dimension, DocumentStore, FeatureId, FeatureKind,
        OccurrenceId, Transform,
    };
    use ketchup_core::exact_product::{
        ExactFaceRole, ExactFeatureChainRequest, ExactProductError, ExactRenderPackage,
        build_box_render_package, canonical_reference_lineage_digest,
    };

    const DEFINITION: DefinitionId = DefinitionId(1);
    const EXTRUSION: FeatureId = FeatureId(2);
    const CUT: FeatureId = FeatureId(4);
    const OCCURRENCE: OccurrenceId = OccurrenceId(1);

    fn through_cut_document() -> DocumentStore {
        let mut store = DocumentStore::new();
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DEFINITION,
                    name: "Cut box".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(1),
                    definition_id: DEFINITION,
                    name: "Outer profile".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: EXTRUSION,
                    definition_id: DEFINITION,
                    name: "Extrusion".to_owned(),
                    kind: FeatureKind::Extrusion {
                        profile: FeatureId(1),
                        height: Dimension::from_decimal("10").unwrap(),
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: DEFINITION,
                    name: "Cut profile".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: CUT,
                    definition_id: DEFINITION,
                    name: "Through cut".to_owned(),
                    kind: FeatureKind::ThroughCut {
                        target: EXTRUSION,
                        profile: FeatureId(3),
                    },
                },
                CanonicalCommand::CreateOccurrence {
                    id: OCCURRENCE,
                    definition_id: DEFINITION,
                    name: "Cut box occurrence".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        store
    }

    fn render_package(snapshot: &Snapshot) -> ExactRenderPackage {
        let request = ExactFeatureChainRequest::from_snapshot(snapshot, DEFINITION).unwrap();
        let evidence = [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
            ExactFaceRole::CutWest,
            ExactFaceRole::CutEast,
            ExactFaceRole::CutSouth,
            ExactFaceRole::CutNorth,
        ]
        .map(|role| {
            (
                role,
                canonical_reference_lineage_digest(
                    request.document_id,
                    request.producer_feature_id(),
                    role.semantic_role(),
                    role.source_element_id(),
                    "planar_face",
                ),
                format!("geometry:{role:?}"),
            )
        });
        build_box_render_package(
            &request,
            "exact-input".to_owned(),
            "result".to_owned(),
            "test-backend".to_owned(),
            "test-tolerance".to_owned(),
            [[0.0; 3], request.dimensions_mm()],
            evidence,
        )
        .unwrap()
    }

    fn projection(snapshot: &Snapshot, package: ExactRenderPackage) -> ExactInteractionProjection {
        let results = ExactResultRegistry::accept(snapshot, [Arc::new(package.into())]).unwrap();
        ExactInteractionProjection::from_snapshot(snapshot, &results)
    }

    #[test]
    fn registry_indexes_current_results_by_complete_derived_identity() {
        let store = through_cut_document();
        let snapshot = store.current();
        let package: Arc<ExactBodyPackage> = Arc::new(render_package(&snapshot).into());
        let key = package.result_key();
        let mut results = ExactResultRegistry::accept(&snapshot, [Arc::clone(&package)]).unwrap();

        assert_eq!(key.definition_id, DEFINITION);
        assert_eq!(key.producer_feature_id, CUT);
        assert!(Arc::ptr_eq(results.get_result(&key).unwrap(), &package));
        assert!(Arc::ptr_eq(results.get(&DEFINITION).unwrap(), &package));
        assert_eq!(
            results.insert_current(&snapshot, package),
            Err(ExactProductError::DuplicateResult {
                definition_id: DEFINITION,
                producer_feature_id: CUT,
            })
        );
    }

    #[test]
    fn vertical_ray_through_hole_misses_physical_mesh() {
        let store = through_cut_document();
        let snapshot = store.current();
        let projection = projection(&snapshot, render_package(&snapshot));
        let ray = Ray::new(Vec3::new(5.0, 5.0, 20.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();

        assert_eq!(projection.occurrence_count(), 1);
        assert!(projection.exact_surface_pick(ray).is_none());
        assert!(projection.exact_pick(ray).is_none());
    }

    #[test]
    fn every_through_cut_wall_maps_to_its_durable_reference() {
        let store = through_cut_document();
        let snapshot = store.current();
        let projection = projection(&snapshot, render_package(&snapshot));

        for (role, direction) in [
            (ExactFaceRole::CutWest, Vec3::new(-1.0, 0.0, 0.0)),
            (ExactFaceRole::CutEast, Vec3::new(1.0, 0.0, 0.0)),
            (ExactFaceRole::CutSouth, Vec3::new(0.0, -1.0, 0.0)),
            (ExactFaceRole::CutNorth, Vec3::new(0.0, 1.0, 0.0)),
        ] {
            let ray = Ray::new(Vec3::new(5.0, 5.0, 5.0), direction).unwrap();
            let hit = projection
                .exact_pick(ray)
                .unwrap_or_else(|| panic!("cut wall {role:?} was not picked"));
            assert_eq!(hit.target.body.role(), Some(role));
            assert!(hit.target.body.has_valid_lineage());
            assert_eq!(hit.target.instance_path, InstancePath::root(OCCURRENCE));
        }
    }

    #[test]
    fn unreferenced_front_triangle_occludes_referenced_triangles_behind_it() {
        let store = through_cut_document();
        let snapshot = store.current();
        let projection = projection(&snapshot, render_package(&snapshot));
        let ray = Ray::new(Vec3::new(-5.0, 2.0, 5.0), Vec3::new(1.0, 0.0, -0.5)).unwrap();
        let physical_hit = hit_occurrence(ray, &projection.occurrences[0]).unwrap();

        assert_eq!(
            physical_hit.occurrence.package.triangles()[physical_hit.triangle_index].face_role,
            None
        );
        assert!(
            physical_hit
                .occurrence
                .package
                .triangles()
                .iter()
                .any(|triangle| {
                    let [first, second, third] = triangle.vertex_indices.map(|index| {
                        let position =
                            physical_hit.occurrence.package.vertices()[index as usize].position_mm;
                        Vec3::new(position[0], position[1], position[2])
                    });
                    triangle.face_role.is_some()
                        && ray_triangle_distance(ray, first, second, third)
                            .is_some_and(|distance| distance > physical_hit.ray_distance_mm)
                })
        );
        let surface_hit = projection
            .exact_surface_pick(ray)
            .expect("the unreferenced front triangle is still a physical surface");
        assert!(surface_hit.durable_target.is_none());
        assert_eq!(surface_hit.instance_path, InstancePath::root(OCCURRENCE));
        assert!(surface_hit.outward_normal.x < -0.999);
        assert!(projection.exact_pick(ray).is_none());
    }

    #[test]
    fn stale_packages_are_rejected_and_affine_transforms_remain_pickable() {
        let mut store = through_cut_document();
        let initial_snapshot = store.current();
        let initial_package = render_package(&initial_snapshot);
        let current = projection(&initial_snapshot, initial_package.clone());
        let instance_path = InstancePath::root(OCCURRENCE);
        assert!(current.contains_occurrence(&instance_path));

        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: EXTRUSION,
                    dimension: Dimension::from_decimal("12").unwrap(),
                },
            ]))
            .unwrap();
        let changed_snapshot = store.current();
        assert!(matches!(
            ExactResultRegistry::accept(&changed_snapshot, [Arc::new(initial_package.into())]),
            Err(ExactProductError::StaleResult)
        ));

        let rotation = Transform::from_matrix([
            0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ])
        .unwrap();
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: OCCURRENCE,
                    transform: rotation,
                },
            ]))
            .unwrap();
        let rotated_snapshot = store.current();
        let rotated = projection(&rotated_snapshot, render_package(&rotated_snapshot));
        assert_eq!(rotated.occurrence_count(), 1);
        assert!(rotated.contains_occurrence(&instance_path));
        let hole_ray = Ray::new(Vec3::new(-5.0, 5.0, 20.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
        assert!(rotated.exact_surface_pick(hole_ray).is_none());
        let wall_ray = Ray::new(Vec3::new(-5.0, 5.0, 5.0), Vec3::new(0.0, -1.0, 0.0)).unwrap();
        let wall = rotated.exact_pick(wall_ray).unwrap();
        assert_eq!(wall.target.body.role(), Some(ExactFaceRole::CutWest));
        assert!((wall.position_mm.y - 4.0).abs() <= 1.0e-9);
        assert!(wall.position_mm.x.abs() > 4.999 && wall.position_mm.x.abs() < 5.001);
    }
}
