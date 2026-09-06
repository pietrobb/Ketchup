use crate::spatial::{SnapshotBinding, SpatialBounds, SpatialIndex, SpatialQueryStats};
use crate::{InteractionError, InteractionScene, SharedBoxGeometry, Vec3};
use ketchup_core::document::{
    DefinitionId, DocumentId, FeatureId, FeatureKind, GroupId, InstancePath, OccurrenceId,
    ProfileSegment, SceneOccurrence, SceneQueryBudgetExceeded, Snapshot, Transform,
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

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedBoundsQuery {
    pub occurrence_indices: Vec<usize>,
    pub stats: SpatialQueryStats,
    pub unbounded_occurrences: usize,
}

pub struct InteractionProjection {
    binding: SnapshotBinding,
    document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    status: ProjectionStatus,
    occurrences: Vec<ProjectedOccurrence>,
    spatial_index: SpatialIndex,
    unbounded_occurrences: usize,
}

pub struct CanonicalInteractionProjection;

impl CanonicalInteractionProjection {
    #[must_use]
    pub fn from_snapshot(snapshot: &Snapshot) -> InteractionProjection {
        Self::from_scene_occurrences(snapshot, snapshot.scene_query())
    }

    pub fn from_snapshot_bounded(
        snapshot: &Snapshot,
        max_occurrences: usize,
        max_path_steps: usize,
        max_text_bytes: usize,
    ) -> Result<InteractionProjection, SceneQueryBudgetExceeded> {
        snapshot
            .scene_query_bounded(max_occurrences, max_path_steps, max_text_bytes)
            .map(|occurrences| Self::from_scene_occurrences(snapshot, occurrences))
    }

    fn from_scene_occurrences(
        snapshot: &Snapshot,
        scene_occurrences: Vec<SceneOccurrence>,
    ) -> InteractionProjection {
        let mut boxes = BTreeMap::new();
        let occurrences: Vec<ProjectedOccurrence> = scene_occurrences
            .into_iter()
            .map(|scene_occurrence| {
                let (profile_feature_id, extrusion_feature_id, local_box) = *boxes
                    .entry(scene_occurrence.definition_id)
                    .or_insert_with(|| canonical_box(snapshot, scene_occurrence.definition_id));
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
        let spatial_index = SpatialIndex::build(occurrences.iter().enumerate().filter_map(
            |(index, occurrence)| {
                occurrence.box_proxy.map(|bounds| {
                    (
                        index,
                        SpatialBounds::from_origin_size(bounds.origin_mm, bounds.size_mm),
                    )
                })
            },
        ));
        let unbounded_occurrences = occurrences
            .iter()
            .filter(|occurrence| occurrence.box_proxy.is_none())
            .count();
        InteractionProjection {
            binding: SnapshotBinding::from_snapshot(snapshot),
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            status: ProjectionStatus::ProxyIncomplete,
            occurrences,
            spatial_index,
            unbounded_occurrences,
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
    pub fn query_world_bounds(&self, min_mm: Vec3, max_mm: Vec3) -> ProjectedBoundsQuery {
        let (occurrence_indices, stats) = self
            .spatial_index
            .query_bounds_with_stats(SpatialBounds::from_min_max(min_mm, max_mm));
        ProjectedBoundsQuery {
            occurrence_indices,
            stats,
            unbounded_occurrences: self.unbounded_occurrences,
        }
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
        let mut scene = InteractionScene::new(self.binding.clone());
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
        scene.rebuild_spatial_index();
        Ok(scene)
    }
}

/// Whether a definition carries an operation whose result no axis-aligned box
/// can stand in for.
///
/// A subtractive or free-form operation removes or reshapes material, so a
/// filled proxy would hide the very geometry that matters — a through hole
/// would read as solid to render, pick and export alike. Such definitions are
/// answered by the exact pipeline or by a canonical mesh body, never by a box.
#[must_use]
pub fn definition_requires_evaluated_geometry(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> bool {
    let Some(definition) = snapshot.definition(definition_id) else {
        return false;
    };
    definition.feature_ids().iter().any(|feature_id| {
        snapshot.feature(*feature_id).is_none_or(|feature| {
            !matches!(
                feature.kind(),
                FeatureKind::Workplane(_)
                    | FeatureKind::Sketch(_)
                    | FeatureKind::Profile { .. }
                    | FeatureKind::SegmentProfile { .. }
                    | FeatureKind::Extrusion { .. }
            )
        })
    })
}

fn canonical_box(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> (Option<FeatureId>, Option<FeatureId>, Option<ProjectedBox>) {
    let Some(definition) = snapshot.definition(definition_id) else {
        return (None, None, None);
    };
    if definition_requires_evaluated_geometry(snapshot, definition_id) {
        return (None, None, None);
    }
    let mut profile = None;
    let mut extrusion = None;
    for feature_id in definition.feature_ids() {
        let Some(feature) = snapshot.feature(*feature_id) else {
            return (None, None, None);
        };
        match feature.kind() {
            FeatureKind::Profile { .. } | FeatureKind::SegmentProfile { .. } => {
                profile.get_or_insert(*feature_id);
            }
            FeatureKind::Extrusion {
                profile: source,
                height,
            } => {
                if extrusion
                    .replace((*feature_id, *source, height.millimetres()))
                    .is_some()
                {
                    return (None, None, None);
                }
            }
            FeatureKind::Workplane(_) | FeatureKind::Sketch(_) => {}
            _ => return (None, None, None),
        }
    }
    let (profile_id, extrusion_id, height) = match extrusion {
        None if definition.feature_ids().len() == 1 => {
            let Some(profile_id) = profile else {
                return (None, None, None);
            };
            (profile_id, None, 0.0)
        }
        Some((extrusion_id, profile_id, height)) => (profile_id, Some(extrusion_id), height),
        None => return (None, None, None),
    };
    let Some(profile) = snapshot.feature(profile_id) else {
        return (None, None, None);
    };
    if profile.definition_id() != definition_id {
        return (None, None, None);
    }
    let bounds = match profile.kind() {
        FeatureKind::Profile { points_mm } => profile_bounds(points_mm),
        FeatureKind::SegmentProfile { segments, .. } => segment_profile_bounds(segments),
        _ => return (None, None, None),
    };
    let Some((min_x, min_y, width, depth)) = bounds else {
        return (Some(profile_id), extrusion_id, None);
    };
    if !height.is_finite() {
        return (Some(profile_id), extrusion_id, None);
    }
    (
        Some(profile_id),
        extrusion_id,
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

fn segment_profile_bounds(segments: &[ProfileSegment]) -> Option<(f64, f64, f64, f64)> {
    let mut points = Vec::new();
    for segment in segments {
        match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                points.extend([*start_mm, *end_mm]);
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                ..
            } => {
                let radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
                if !radius.is_finite() {
                    return None;
                }
                points.extend([
                    *start_mm,
                    *end_mm,
                    [center_mm[0] - radius, center_mm[1] - radius],
                    [center_mm[0] + radius, center_mm[1] + radius],
                ]);
            }
            ProfileSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => {
                points.extend([*start_mm, *control_1_mm, *control_2_mm, *end_mm]);
            }
        }
    }
    if let Some(bounds) = profile_bounds(&points) {
        return Some(bounds);
    }
    let first = points.first()?;
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (first[0], first[0], first[1], first[1]);
    for point in &points[1..] {
        min_x = min_x.min(point[0]);
        max_x = max_x.max(point[0]);
        min_y = min_y.min(point[1]);
        max_y = max_y.max(point[1]);
    }
    const MINIMUM_PROXY_SPAN_MM: f64 = 1.0e-9;
    let mut width = max_x - min_x;
    let mut depth = max_y - min_y;
    if width == 0.0 && depth > 0.0 {
        min_x -= MINIMUM_PROXY_SPAN_MM * 0.5;
        width = MINIMUM_PROXY_SPAN_MM;
    } else if depth == 0.0 && width > 0.0 {
        min_y -= MINIMUM_PROXY_SPAN_MM * 0.5;
        depth = MINIMUM_PROXY_SPAN_MM;
    }
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
