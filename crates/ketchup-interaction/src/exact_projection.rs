#![forbid(unsafe_code)]

use crate::spatial::{
    SPATIAL_INDEX_V1, SnapshotBinding, SpatialBounds, SpatialIndex, SpatialQueryError,
    SpatialQueryStats,
};
use crate::{Ray, Vec3};
use ketchup_core::document::{
    CommandBatch, DefinitionId, DocumentStore, FeatureId, InstancePath, Proposal,
    ProposalCommitError, ProposalContext, ProposalDiffEntry, ProposalPrepareError, Revision,
    Snapshot, Transform,
};
use ketchup_core::exact_product::{AssemblySelectionTarget, ExactBodyPackage, ExactResultRegistry};
use ketchup_core::topology::{
    TopologicalElementKind, TopologicalElementRef, TopologicalReferenceQuarantineReason,
    TopologicalReferenceResolution,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

const RAY_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopologicalPickLocator {
    pub instance_path: InstancePath,
    pub producer_feature_id: FeatureId,
    pub kind: TopologicalElementKind,
    pub ordinal: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopologicalSelectionTarget {
    pub instance_path: InstancePath,
    pub reference: TopologicalElementRef,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotBoundTopologicalSelection {
    binding: SnapshotBinding,
    target: TopologicalSelectionTarget,
}

impl SnapshotBoundTopologicalSelection {
    #[must_use]
    pub const fn target(&self) -> &TopologicalSelectionTarget {
        &self.target
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.binding.is_current(snapshot)
    }

    pub fn resolve_current(
        &self,
        snapshot: &Snapshot,
        results: &ExactResultRegistry,
    ) -> Result<TopologicalSelectionTarget, TopologicalPickError> {
        if !self.binding.is_current(snapshot) {
            return Err(TopologicalPickError::StaleSnapshot);
        }
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| occurrence.instance_path == self.target.instance_path)
            .ok_or(TopologicalPickError::OccurrenceUnavailable)?;
        if !occurrence.visible || occurrence.definition_id != self.target.reference.definition_id {
            return Err(TopologicalPickError::OccurrenceUnavailable);
        }
        match results.resolve_topological_reference(snapshot, &self.target.reference) {
            TopologicalReferenceResolution::Resolved { reference } => {
                Ok(TopologicalSelectionTarget {
                    instance_path: self.target.instance_path.clone(),
                    reference: *reference,
                })
            }
            TopologicalReferenceResolution::Ambiguous { candidate_count } => {
                Err(TopologicalPickError::Ambiguous { candidate_count })
            }
            TopologicalReferenceResolution::Lost => Err(TopologicalPickError::Lost),
            TopologicalReferenceResolution::Quarantined { reason } => {
                Err(TopologicalPickError::Quarantined { reason })
            }
        }
    }

    pub fn prepare_proposal(
        &self,
        document: &DocumentStore,
        results: &ExactResultRegistry,
        batch: CommandBatch,
        context: ProposalContext,
    ) -> Result<TopologicalSelectionProposal, TopologicalSelectionProposalError> {
        self.resolve_current(&document.current(), results)
            .map_err(TopologicalSelectionProposalError::Selection)?;
        let proposal = document
            .prepare_proposal_with_context(batch, context)
            .map_err(TopologicalSelectionProposalError::Prepare)?;
        Ok(TopologicalSelectionProposal {
            selection: self.clone(),
            proposal,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologicalSelectionProposal {
    selection: SnapshotBoundTopologicalSelection,
    proposal: Proposal,
}

impl TopologicalSelectionProposal {
    #[must_use]
    pub const fn selection(&self) -> &SnapshotBoundTopologicalSelection {
        &self.selection
    }

    #[must_use]
    pub const fn provenance_revision(&self) -> u64 {
        self.proposal.provenance_revision()
    }

    #[must_use]
    pub fn provenance_digest(&self) -> &str {
        self.proposal.provenance_digest()
    }

    #[must_use]
    pub fn authoritative_diff(&self) -> &[ProposalDiffEntry] {
        self.proposal.authoritative_diff()
    }

    #[must_use]
    pub fn intended_result_digest(&self) -> &str {
        self.proposal.intended_result_digest()
    }

    pub fn commit(
        &self,
        document: &mut DocumentStore,
        results: &ExactResultRegistry,
    ) -> Result<Arc<Revision>, TopologicalSelectionProposalError> {
        let snapshot = document.current();
        self.selection
            .resolve_current(&snapshot, results)
            .map_err(TopologicalSelectionProposalError::Selection)?;
        if self.proposal.document_id() != snapshot.document_id()
            || self.proposal.provenance_revision() != snapshot.revision_id()
            || self.proposal.provenance_digest() != snapshot.canonical_digest()
        {
            return Err(TopologicalSelectionProposalError::StaleProposal);
        }
        document
            .commit_proposal(&self.proposal)
            .map_err(TopologicalSelectionProposalError::Commit)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopologicalPickError {
    StaleSnapshot,
    OccurrenceUnavailable,
    ResultUnavailable,
    Lost,
    Ambiguous {
        candidate_count: usize,
    },
    Quarantined {
        reason: TopologicalReferenceQuarantineReason,
    },
}

impl fmt::Display for TopologicalPickError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleSnapshot => {
                formatter.write_str("topological pick is bound to a stale snapshot")
            }
            Self::OccurrenceUnavailable => formatter.write_str("picked occurrence is unavailable"),
            Self::ResultUnavailable => formatter.write_str("picked exact element is unavailable"),
            Self::Lost => formatter.write_str("topological reference is lost"),
            Self::Ambiguous { candidate_count } => {
                write!(
                    formatter,
                    "topological reference has {candidate_count} candidates"
                )
            }
            Self::Quarantined { reason } => {
                write!(
                    formatter,
                    "topological reference is quarantined: {reason:?}"
                )
            }
        }
    }
}

impl std::error::Error for TopologicalPickError {}

#[derive(Debug)]
pub enum TopologicalSelectionProposalError {
    Selection(TopologicalPickError),
    Prepare(ProposalPrepareError),
    StaleProposal,
    Commit(ProposalCommitError),
}

impl fmt::Display for TopologicalSelectionProposalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selection(error) => write!(formatter, "selection rejected: {error}"),
            Self::Prepare(error) => write!(formatter, "proposal preparation failed: {error}"),
            Self::StaleProposal => formatter.write_str("selection proposal is stale"),
            Self::Commit(error) => write!(formatter, "proposal commit failed: {error}"),
        }
    }
}

impl std::error::Error for TopologicalSelectionProposalError {}

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
    pub topological_target: Option<SnapshotBoundTopologicalSelection>,
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
        let mut render_packages = BTreeMap::<DefinitionId, Vec<&Arc<ExactBodyPackage>>>::new();
        for package in results.render_values(snapshot) {
            render_packages
                .entry(package.definition_id())
                .or_default()
                .push(package);
        }
        let mut occurrences = Vec::new();
        for occurrence in snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .filter(|occurrence| include(&occurrence.instance_path))
        {
            for package in render_packages
                .get(&occurrence.definition_id)
                .into_iter()
                .flat_map(|packages| packages.iter())
            {
                occurrences.push(ExactOccurrence {
                    instance_path: occurrence.instance_path.clone(),
                    transform: occurrence.transform,
                    package: Arc::clone(package),
                });
            }
        }
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

    pub fn topological_pick_current(
        &self,
        snapshot: &Snapshot,
        results: &ExactResultRegistry,
        locator: &TopologicalPickLocator,
    ) -> Result<SnapshotBoundTopologicalSelection, TopologicalPickError> {
        if !self.binding.is_current(snapshot) {
            return Err(TopologicalPickError::StaleSnapshot);
        }
        let mut matches = self.occurrences.iter().filter(|occurrence| {
            occurrence.instance_path == locator.instance_path
                && occurrence.package.producer_feature_id() == locator.producer_feature_id
        });
        let occurrence = matches
            .next()
            .ok_or(TopologicalPickError::ResultUnavailable)?;
        if matches.next().is_some() {
            return Err(TopologicalPickError::ResultUnavailable);
        }
        let reference = occurrence
            .package
            .topological_reference(locator.kind, locator.ordinal)
            .cloned()
            .ok_or(TopologicalPickError::ResultUnavailable)?;
        let selection = SnapshotBoundTopologicalSelection {
            binding: self.binding.clone(),
            target: TopologicalSelectionTarget {
                instance_path: occurrence.instance_path.clone(),
                reference,
            },
        };
        selection.resolve_current(snapshot, results)?;
        Ok(selection)
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
        let topological_target = hit
            .occurrence
            .package
            .topological_reference_for_triangle(hit.triangle_index)
            .cloned()
            .map(|reference| SnapshotBoundTopologicalSelection {
                binding: self.binding.clone(),
                target: TopologicalSelectionTarget {
                    instance_path: hit.occurrence.instance_path.clone(),
                    reference,
                },
            });
        (
            Some(ExactSurfaceHit {
                definition_id: hit.occurrence.package.definition_id(),
                instance_path: hit.occurrence.instance_path.clone(),
                durable_target,
                topological_target,
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
    use ketchup_core::exact_brep_graph::ExactBRepGraph;
    use ketchup_core::exact_product::{
        ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence, ExactFaceRole,
        ExactFeatureChainRequest, ExactProductError, ExactRenderPackage, ImportedExactPackage,
        build_box_render_package, canonical_reference_lineage_digest,
    };
    use ketchup_core::import::{
        ImportLengthUnit, StepImportEvidence, StepImportMesh, StepMeshTriangle, plan_step_import,
    };
    use ketchup_core::persistence;

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

    #[test]
    fn generated_face_edge_vertex_picks_survive_unrelated_recompute_without_retargeting() {
        let mut document = through_cut_document();
        let snapshot = document.current();
        let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, CUT).unwrap();
        let mesh = StepImportMesh {
            vertices_mm: vec![[0.0, 0.0, 10.0], [10.0, 0.0, 10.0], [0.0, 10.0, 10.0]],
            triangles: vec![StepMeshTriangle {
                vertex_indices: [0, 1, 2],
                face_ordinal: 0,
            }],
        };
        let package = Arc::new(ExactBodyPackage::Graph(
            ExactBRepGraphPackage::from_worker_evidence(
                &graph,
                ExactBRepGraphWorkerEvidence {
                    exact_input_digest: "generated-headless-input".into(),
                    result_fingerprint: "generated-headless-result".into(),
                    volume_mm3: 1_000.0,
                    area_mm2: 0.0,
                    topology_counts: [4, 6, 4, 1, 1],
                    wire_count: None,
                    bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
                    backend: "occt-generated-headless.v1".into(),
                    tolerance: "1e-7-mm".into(),
                },
                &mesh,
            )
            .unwrap(),
        ));
        let producer_feature_id = package.producer_feature_id();
        let results = ExactResultRegistry::accept(&snapshot, [package]).unwrap();
        let projection = ExactInteractionProjection::from_snapshot(&snapshot, &results);
        let instance_path = InstancePath::root(OCCURRENCE);
        let selections = [
            TopologicalElementKind::Face,
            TopologicalElementKind::Edge,
            TopologicalElementKind::Vertex,
        ]
        .map(|kind| {
            projection
                .topological_pick_current(
                    &snapshot,
                    &results,
                    &TopologicalPickLocator {
                        instance_path: instance_path.clone(),
                        producer_feature_id,
                        kind,
                        ordinal: 0,
                    },
                )
                .unwrap()
        });
        let face_hit = projection
            .exact_surface_pick_current(
                &snapshot,
                Ray::new(Vec3::new(1.0, 1.0, 20.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            face_hit.topological_target.unwrap().target().reference,
            selections[0].target().reference
        );

        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: OCCURRENCE,
                    transform: Transform::from_translation(2.0, 0.0, 0.0).unwrap(),
                },
            ]))
            .unwrap();
        let recomputed = document.current();
        assert_eq!(
            selections[0].resolve_current(&recomputed, &results),
            Err(TopologicalPickError::StaleSnapshot)
        );
        let recomputed_results = ExactResultRegistry::carried_forward(&recomputed, &results);
        let recomputed_projection =
            ExactInteractionProjection::from_snapshot(&recomputed, &recomputed_results);
        for (selection, kind) in selections.iter().zip([
            TopologicalElementKind::Face,
            TopologicalElementKind::Edge,
            TopologicalElementKind::Vertex,
        ]) {
            let repicked = recomputed_projection
                .topological_pick_current(
                    &recomputed,
                    &recomputed_results,
                    &TopologicalPickLocator {
                        instance_path: instance_path.clone(),
                        producer_feature_id,
                        kind,
                        ordinal: 0,
                    },
                )
                .unwrap();
            assert_eq!(repicked.target().reference, selection.target().reference);
        }

        let undone = document.undo().unwrap();
        let undo_results = ExactResultRegistry::carried_forward(&undone, &recomputed_results);
        for selection in &selections {
            assert_eq!(
                selection.resolve_current(&undone, &undo_results).unwrap(),
                selection.target().clone()
            );
        }
    }

    #[test]
    fn imported_face_edge_vertex_picks_are_snapshot_bound_for_manual_and_proposal_flows() {
        let source = b"headless topological selection fixture";
        let evidence = StepImportEvidence {
            source_unit: ImportLengthUnit::Millimetre,
            result_fingerprint: "headless-selection-result".into(),
            solid_count: 1,
            topology_counts: [4, 6, 4, 1, 1],
            volume_mm3: 1.0,
            bounds_mm: [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
            backend: "occt-headless-selection.v1".into(),
            tolerance: "1e-7-mm".into(),
        };
        let mesh = StepImportMesh {
            vertices_mm: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            triangles: vec![StepMeshTriangle {
                vertex_indices: [0, 1, 2],
                face_ordinal: 0,
            }],
        };
        let mut document = DocumentStore::new();
        document
            .apply_batch(
                &plan_step_import(&document.current(), source, "selection.step", &evidence)
                    .unwrap(),
            )
            .unwrap();
        let snapshot = document.current();
        let scene = snapshot.scene_query();
        let [scene_occurrence] = scene.as_slice() else {
            panic!("import must publish one visible occurrence");
        };
        let imported = ImportedExactPackage::from_snapshot(
            &snapshot,
            scene_occurrence.definition_id,
            source.to_vec(),
            &mesh,
        )
        .unwrap();
        let package = Arc::new(ExactBodyPackage::Imported(imported.clone()));
        let producer_feature_id = package.producer_feature_id();
        let results = ExactResultRegistry::accept(&snapshot, [package]).unwrap();
        let projection = ExactInteractionProjection::from_snapshot(&snapshot, &results);

        let selections = [
            TopologicalElementKind::Face,
            TopologicalElementKind::Edge,
            TopologicalElementKind::Vertex,
        ]
        .map(|kind| {
            projection
                .topological_pick_current(
                    &snapshot,
                    &results,
                    &TopologicalPickLocator {
                        instance_path: scene_occurrence.instance_path.clone(),
                        producer_feature_id,
                        kind,
                        ordinal: 0,
                    },
                )
                .unwrap()
        });
        for (selection, kind) in selections.iter().zip([
            TopologicalElementKind::Face,
            TopologicalElementKind::Edge,
            TopologicalElementKind::Vertex,
        ]) {
            assert_eq!(selection.target().reference.kind, kind);
            assert!(selection.target().reference.has_valid_lineage());
            assert_eq!(
                selection.resolve_current(&snapshot, &results).unwrap(),
                selection.target().clone()
            );
        }

        let selected_edge = &selections[1];
        assert_eq!(
            selected_edge.resolve_current(&snapshot, &ExactResultRegistry::default()),
            Err(TopologicalPickError::Lost)
        );
        let mut duplicated = imported.clone();
        duplicated
            .topological_references
            .push(selected_edge.target().reference.clone());
        let ambiguous_results = ExactResultRegistry::accept(
            &snapshot,
            [Arc::new(ExactBodyPackage::Imported(duplicated))],
        )
        .unwrap();
        assert_eq!(
            selected_edge.resolve_current(&snapshot, &ambiguous_results),
            Err(TopologicalPickError::Ambiguous { candidate_count: 2 })
        );
        let mut ordinal_neighbor_only = imported.clone();
        ordinal_neighbor_only
            .topological_references
            .retain(|reference| reference != &selected_edge.target().reference);
        let ordinal_neighbor_results = ExactResultRegistry::accept(
            &snapshot,
            [Arc::new(ExactBodyPackage::Imported(ordinal_neighbor_only))],
        )
        .unwrap();
        assert_eq!(
            selected_edge.resolve_current(&snapshot, &ordinal_neighbor_results),
            Err(TopologicalPickError::Lost)
        );

        let ray = Ray::new(Vec3::new(0.25, 0.25, 2.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
        let face_hit = projection
            .exact_surface_pick_current(&snapshot, ray)
            .unwrap()
            .unwrap();
        let ray_selection = face_hit
            .topological_target
            .expect("face ordinal must map to a role-neutral reference");
        assert_eq!(
            ray_selection.target().reference,
            selections[0].target().reference
        );

        let proposal = selections[1]
            .prepare_proposal(
                &document,
                &results,
                CommandBatch::new(vec![CanonicalCommand::SetOccurrenceTransform {
                    id: scene_occurrence.occurrence_id,
                    transform: Transform::from_translation(2.0, 0.0, 0.0).unwrap(),
                }]),
                ProposalContext::local_assistant_model(),
            )
            .unwrap();
        assert_eq!(proposal.provenance_revision(), snapshot.revision_id());
        proposal.commit(&mut document, &results).unwrap();
        assert_eq!(
            selections[1].resolve_current(&document.current(), &results),
            Err(TopologicalPickError::StaleSnapshot)
        );
        assert_eq!(
            projection.topological_pick_current(
                &document.current(),
                &results,
                &TopologicalPickLocator {
                    instance_path: scene_occurrence.instance_path.clone(),
                    producer_feature_id,
                    kind: TopologicalElementKind::Vertex,
                    ordinal: 0,
                },
            ),
            Err(TopologicalPickError::StaleSnapshot)
        );

        let changed = document.current();
        let changed_results = ExactResultRegistry::carried_forward(&changed, &results);
        let undone = document.undo().unwrap();
        let undo_results = ExactResultRegistry::carried_forward(&undone, &changed_results);
        for selection in &selections {
            assert_eq!(
                selection.resolve_current(&undone, &undo_results).unwrap(),
                selection.target().clone()
            );
        }

        let mut container = persistence::ContainerData::default();
        container.insert_import_blob(source.to_vec()).unwrap();
        let saved = persistence::save_container(&undone, &container).unwrap();
        let reopened = persistence::load(&saved).unwrap();
        let reopened_snapshot = reopened.snapshot();
        assert_eq!(
            persistence::save_container(&reopened_snapshot, reopened.container_data()).unwrap(),
            saved
        );
        let reopened_results =
            ExactResultRegistry::carried_forward(&reopened_snapshot, &undo_results);
        for selection in &selections {
            assert_eq!(
                selection
                    .resolve_current(&reopened_snapshot, &reopened_results)
                    .unwrap(),
                selection.target().clone()
            );
        }

        let redone = document.redo().unwrap();
        let redo_results = ExactResultRegistry::carried_forward(&redone, &undo_results);
        for selection in &selections {
            assert_eq!(
                selection.resolve_current(&redone, &redo_results),
                Err(TopologicalPickError::StaleSnapshot)
            );
        }
    }
}
