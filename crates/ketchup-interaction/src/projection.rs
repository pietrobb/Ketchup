use crate::{InteractionError, InteractionScene, SharedBoxGeometry, Vec3};
use ketchup_core::document::{
    DefinitionId, DocumentId, FeatureId, FeatureKind, GroupId, InstancePath, OccurrenceId,
    Snapshot, Transform,
};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const INTERACTION_PROJECTION_V1: &str = "ketchup.interaction-projection.v1";
pub const PROXY_EVALUATOR_V1: &str = "ketchup.proxy-box-evaluator.v1";
pub const PROXY_BACKEND_V1: &str = "ketchup.interaction.cpu-aabb.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionStatus {
    ProxyIncomplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectedBodyRef {
    pub definition_id: DefinitionId,
    pub profile_feature_id: Option<FeatureId>,
    pub extrusion_feature_id: Option<FeatureId>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectedBox {
    pub origin_mm: Vec3,
    pub size_mm: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedOccurrence {
    pub occurrence_id: OccurrenceId,
    pub instance_path: InstancePath,
    pub body: ProjectedBodyRef,
    pub occurrence_name: String,
    pub definition_name: String,
    pub canonical_world_transform: Transform,
    pub parent: Option<GroupId>,
    pub visible: bool,
    pub shared_occurrence_count: usize,
    pub local_box: Option<ProjectedBox>,
    pub box_proxy: Option<ProjectedBox>,
}

pub struct InteractionProjection {
    document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    status: ProjectionStatus,
    occurrences: Vec<ProjectedOccurrence>,
}

pub struct CanonicalInteractionProjection;

impl CanonicalInteractionProjection {
    #[must_use]
    pub fn from_snapshot(snapshot: &Snapshot) -> InteractionProjection {
        let occurrences = snapshot
            .scene_query()
            .into_iter()
            .map(|scene_occurrence| {
                let (profile_feature_id, extrusion_feature_id, local_box) =
                    canonical_box(snapshot, scene_occurrence.definition_id);
                let box_proxy = local_box
                    .and_then(|local_box| transformed_aabb(scene_occurrence.transform, local_box));
                ProjectedOccurrence {
                    occurrence_id: scene_occurrence.occurrence_id,
                    instance_path: scene_occurrence.instance_path,
                    body: ProjectedBodyRef {
                        definition_id: scene_occurrence.definition_id,
                        profile_feature_id,
                        extrusion_feature_id,
                    },
                    occurrence_name: scene_occurrence.occurrence_name,
                    definition_name: scene_occurrence.definition_name,
                    canonical_world_transform: scene_occurrence.transform,
                    parent: scene_occurrence.parent,
                    visible: scene_occurrence.visible,
                    shared_occurrence_count: scene_occurrence.shared_occurrence_count,
                    local_box,
                    box_proxy,
                }
            })
            .collect();
        InteractionProjection {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            status: ProjectionStatus::ProxyIncomplete,
            occurrences,
        }
    }
}

impl InteractionProjection {
    #[must_use]
    pub const fn schema(&self) -> &'static str {
        INTERACTION_PROJECTION_V1
    }

    #[must_use]
    pub const fn evaluator(&self) -> &'static str {
        PROXY_EVALUATOR_V1
    }

    #[must_use]
    pub const fn backend(&self) -> &'static str {
        PROXY_BACKEND_V1
    }

    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }

    #[must_use]
    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }

    #[must_use]
    pub const fn status(&self) -> ProjectionStatus {
        self.status
    }

    #[must_use]
    pub fn occurrences(&self) -> &[ProjectedOccurrence] {
        &self.occurrences
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id()
            && self.source_revision == snapshot.revision_id()
            && self.source_digest == snapshot.canonical_digest()
    }

    pub fn scene(&self) -> Result<InteractionScene, InteractionError> {
        self.scene_with_box_overrides_where(&BTreeMap::new(), |_| true)
    }

    pub fn scene_where(
        &self,
        include: impl Fn(&ProjectedOccurrence) -> bool,
    ) -> Result<InteractionScene, InteractionError> {
        self.scene_with_box_overrides_where(&BTreeMap::new(), include)
    }

    pub fn scene_with_box_overrides_where(
        &self,
        overrides: &BTreeMap<InstancePath, ProjectedBox>,
        include: impl Fn(&ProjectedOccurrence) -> bool,
    ) -> Result<InteractionScene, InteractionError> {
        let mut scene = InteractionScene::new();
        let mut geometries =
            BTreeMap::<(DefinitionId, u64, u64, u64), Arc<SharedBoxGeometry>>::new();
        for occurrence in self
            .occurrences
            .iter()
            .filter(|occurrence| occurrence.visible && include(occurrence))
        {
            let Some(box_proxy) = overrides
                .get(&occurrence.instance_path)
                .copied()
                .or(occurrence.box_proxy)
            else {
                continue;
            };
            let key = (
                occurrence.body.definition_id,
                box_proxy.size_mm.x.to_bits(),
                box_proxy.size_mm.y.to_bits(),
                box_proxy.size_mm.z.to_bits(),
            );
            let geometry = if let Some(geometry) = geometries.get(&key) {
                Arc::clone(geometry)
            } else {
                let geometry = Arc::new(SharedBoxGeometry::new(box_proxy.size_mm)?);
                geometries.insert(key, Arc::clone(&geometry));
                geometry
            };
            scene.add_occurrence(
                occurrence.instance_path.clone(),
                occurrence.body.definition_id,
                box_proxy.origin_mm,
                geometry,
            )?;
        }
        Ok(scene)
    }
}

fn canonical_box(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> (Option<FeatureId>, Option<FeatureId>, Option<ProjectedBox>) {
    let Some(definition) = snapshot.definition(definition_id) else {
        return (None, None, None);
    };
    let extrusions = definition
        .feature_ids()
        .iter()
        .filter_map(|feature_id| {
            let feature = snapshot.feature(*feature_id)?;
            match feature.kind() {
                FeatureKind::Extrusion { profile, height } => {
                    Some((*feature_id, *profile, height.millimetres()))
                }
                FeatureKind::Profile { .. } | FeatureKind::ThroughCut { .. } => None,
            }
        })
        .collect::<Vec<_>>();
    let [(extrusion_id, profile_id, height)] = extrusions.as_slice() else {
        return (None, None, None);
    };
    let Some(profile) = snapshot.feature(*profile_id) else {
        return (None, None, None);
    };
    if profile.definition_id() != definition_id {
        return (None, None, None);
    }
    let FeatureKind::Profile { points_mm } = profile.kind() else {
        return (None, None, None);
    };
    let Some((min_x, min_y, width, depth)) = profile_bounds(points_mm) else {
        return (Some(*profile_id), Some(*extrusion_id), None);
    };
    if !height.is_finite() || *height == 0.0 {
        return (Some(*profile_id), Some(*extrusion_id), None);
    }
    (
        Some(*profile_id),
        Some(*extrusion_id),
        Some(ProjectedBox {
            origin_mm: Vec3::new(min_x, min_y, height.min(0.0)),
            size_mm: Vec3::new(width, depth, height.abs()),
        }),
    )
}

fn profile_bounds(points: &[[f64; 2]]) -> Option<(f64, f64, f64, f64)> {
    let first = points.first()?;
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (first[0], first[0], first[1], first[1]);
    for point in &points[1..] {
        min_x = min_x.min(point[0]);
        max_x = max_x.max(point[0]);
        min_y = min_y.min(point[1]);
        max_y = max_y.max(point[1]);
    }
    let (width, depth) = (max_x - min_x, max_y - min_y);
    (min_x.is_finite()
        && min_y.is_finite()
        && width.is_finite()
        && width > 0.0
        && depth.is_finite()
        && depth > 0.0)
        .then_some((min_x, min_y, width, depth))
}

fn transformed_aabb(transform: Transform, local_box: ProjectedBox) -> Option<ProjectedBox> {
    let corners = box_corners(local_box).map(|point| transform_point(transform, point));
    let min = Vec3::new(
        corners.iter().map(|point| point.x).min_by(f64::total_cmp)?,
        corners.iter().map(|point| point.y).min_by(f64::total_cmp)?,
        corners.iter().map(|point| point.z).min_by(f64::total_cmp)?,
    );
    let max = Vec3::new(
        corners.iter().map(|point| point.x).max_by(f64::total_cmp)?,
        corners.iter().map(|point| point.y).max_by(f64::total_cmp)?,
        corners.iter().map(|point| point.z).max_by(f64::total_cmp)?,
    );
    Some(ProjectedBox {
        origin_mm: min,
        size_mm: max - min,
    })
}

fn transform_point(transform: Transform, point: Vec3) -> Vec3 {
    let matrix = transform.matrix();
    Vec3::new(
        matrix[0] * point.x + matrix[1] * point.y + matrix[2] * point.z + matrix[3],
        matrix[4] * point.x + matrix[5] * point.y + matrix[6] * point.z + matrix[7],
        matrix[8] * point.x + matrix[9] * point.y + matrix[10] * point.z + matrix[11],
    )
}

fn box_corners(local_box: ProjectedBox) -> [Vec3; 8] {
    let origin = local_box.origin_mm;
    let size = local_box.size_mm;
    [
        origin,
        origin + Vec3::new(size.x, 0.0, 0.0),
        origin + Vec3::new(0.0, size.y, 0.0),
        origin + Vec3::new(size.x, size.y, 0.0),
        origin + Vec3::new(0.0, 0.0, size.z),
        origin + Vec3::new(size.x, 0.0, size.z),
        origin + Vec3::new(0.0, size.y, size.z),
        origin + size,
    ]
}
