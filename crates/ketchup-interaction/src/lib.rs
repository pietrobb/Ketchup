#![forbid(unsafe_code)]

pub mod projection;

use ketchup_core::adapters::{AdapterError, UiAction, UiAdapter};
use ketchup_core::document::{
    DefinitionId, DocumentStore, NodeId, Proposal, ProposalCommitError, Revision, Snapshot,
};
use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Add, Mul, Sub};
use std::sync::Arc;

const RAY_EPSILON: f64 = 1.0e-12;
const SNAP_EPSILON: f64 = 1.0e-9;
const BOX_EDGE_ENDPOINTS: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    #[must_use]
    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    #[must_use]
    pub fn distance(self, other: Self) -> f64 {
        (self - other).length()
    }

    fn component(self, axis: Axis) -> f64 {
        match axis {
            Axis::X => self.x,
            Axis::Y => self.y,
            Axis::Z => self.z,
        }
    }
}

impl Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

impl Ray {
    pub fn new(origin: Vec3, direction: Vec3) -> Result<Self, InteractionError> {
        let length = direction.length();
        if !length.is_finite()
            || length <= RAY_EPSILON
            || !origin.x.is_finite()
            || !origin.y.is_finite()
            || !origin.z.is_finite()
        {
            return Err(InteractionError::InvalidRay);
        }
        Ok(Self {
            origin,
            direction: direction * (1.0 / length),
        })
    }

    #[must_use]
    pub fn at(self, distance: f64) -> Vec3 {
        self.origin + self.direction * distance
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Side {
    Minimum,
    Maximum,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ElementId {
    Face {
        axis: Axis,
        side: Side,
    },
    Edge(u8),
    EdgeMidpoint(u8),
    Endpoint(u8),
    Intersection {
        primary_edge: u8,
        other_occurrence_id: u64,
        other_edge: u8,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionFilter {
    Face,
    Edge,
    Point,
    Any,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SelectionId {
    pub definition_id: DefinitionId,
    pub occurrence_id: u64,
    pub element: ElementId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapKind {
    Endpoint,
    Intersection,
    Midpoint,
    Face,
}

impl SnapKind {
    const fn rank(self) -> u8 {
        match self {
            Self::Endpoint => 0,
            Self::Intersection => 1,
            Self::Midpoint => 2,
            Self::Face => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapResult {
    pub kind: SnapKind,
    pub reference: SelectionId,
    pub position_mm: Vec3,
    pub distance_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactHit {
    pub reference: SelectionId,
    pub position_mm: Vec3,
    pub ray_distance_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PickResult {
    pub primary: ExactHit,
    pub overlapping: Vec<ExactHit>,
    pub snap: SnapResult,
}

#[derive(Debug, PartialEq)]
pub struct SharedBoxGeometry {
    size_mm: Vec3,
    endpoints: [Vec3; 8],
    edge_midpoints: [Vec3; 12],
}

impl SharedBoxGeometry {
    pub fn new(size_mm: Vec3) -> Result<Self, InteractionError> {
        if !size_mm.x.is_finite()
            || !size_mm.y.is_finite()
            || !size_mm.z.is_finite()
            || size_mm.x <= 0.0
            || size_mm.y <= 0.0
            || size_mm.z <= 0.0
        {
            return Err(InteractionError::InvalidBox);
        }
        let endpoints = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(size_mm.x, 0.0, 0.0),
            Vec3::new(0.0, size_mm.y, 0.0),
            Vec3::new(size_mm.x, size_mm.y, 0.0),
            Vec3::new(0.0, 0.0, size_mm.z),
            Vec3::new(size_mm.x, 0.0, size_mm.z),
            Vec3::new(0.0, size_mm.y, size_mm.z),
            size_mm,
        ];
        let edge_midpoints = BOX_EDGE_ENDPOINTS.map(|(left, right)| {
            let left = endpoints[left];
            let right = endpoints[right];
            (left + right) * 0.5
        });
        Ok(Self {
            size_mm,
            endpoints,
            edge_midpoints,
        })
    }

    #[must_use]
    pub const fn size_mm(&self) -> Vec3 {
        self.size_mm
    }
}

#[derive(Clone, Debug)]
pub struct Occurrence {
    pub id: u64,
    pub definition_id: DefinitionId,
    pub origin_mm: Vec3,
    geometry: Arc<SharedBoxGeometry>,
}

impl Occurrence {
    #[must_use]
    pub fn geometry(&self) -> &Arc<SharedBoxGeometry> {
        &self.geometry
    }
}

#[derive(Default)]
pub struct InteractionScene {
    occurrences: Vec<Occurrence>,
}

impl InteractionScene {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            occurrences: Vec::new(),
        }
    }

    pub(crate) fn add_occurrence(
        &mut self,
        id: u64,
        definition_id: DefinitionId,
        origin_mm: Vec3,
        geometry: Arc<SharedBoxGeometry>,
    ) -> Result<(), InteractionError> {
        if id == 0 || self.occurrences.iter().any(|item| item.id == id) {
            return Err(InteractionError::DuplicateOccurrence);
        }
        if !origin_mm.x.is_finite() || !origin_mm.y.is_finite() || !origin_mm.z.is_finite() {
            return Err(InteractionError::InvalidBox);
        }
        self.occurrences.push(Occurrence {
            id,
            definition_id,
            origin_mm,
            geometry,
        });
        Ok(())
    }

    #[must_use]
    pub fn occurrence_count(&self) -> usize {
        self.occurrences.len()
    }

    #[must_use]
    pub fn authoritative_geometry_count(&self) -> usize {
        let mut unique = Vec::<*const SharedBoxGeometry>::new();
        for occurrence in &self.occurrences {
            let pointer = Arc::as_ptr(&occurrence.geometry);
            if !unique.contains(&pointer) {
                unique.push(pointer);
            }
        }
        unique.len()
    }

    pub fn exact_pick(&self, ray: Ray, snap_tolerance_mm: f64) -> Option<PickResult> {
        self.exact_pick_filtered(ray, snap_tolerance_mm, SelectionFilter::Face)
    }

    pub fn exact_pick_filtered(
        &self,
        ray: Ray,
        snap_tolerance_mm: f64,
        filter: SelectionFilter,
    ) -> Option<PickResult> {
        if !snap_tolerance_mm.is_finite() || snap_tolerance_mm < 0.0 {
            return None;
        }
        let mut hits = self
            .occurrences
            .iter()
            .filter_map(|occurrence| {
                let mut hit = hit_occurrence(ray, occurrence)?;
                let face = hit.reference.element;
                hit.reference.element =
                    filtered_element(occurrence, hit.position_mm, snap_tolerance_mm, filter, face)?;
                Some(hit)
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            left.ray_distance_mm
                .total_cmp(&right.ray_distance_mm)
                .then_with(|| left.reference.cmp(&right.reference))
        });
        let primary = *hits.first()?;
        let occurrence = self
            .occurrences
            .iter()
            .find(|item| item.id == primary.reference.occurrence_id)?;
        let local_snap = resolve_snap(primary, occurrence, snap_tolerance_mm);
        let snap = intersection_snap(primary, &self.occurrences, snap_tolerance_mm)
            .filter(|candidate| better_snap(candidate, &local_snap))
            .unwrap_or(local_snap);
        Some(PickResult {
            primary,
            overlapping: hits,
            snap,
        })
    }
}

fn filtered_element(
    occurrence: &Occurrence,
    world_position: Vec3,
    tolerance: f64,
    filter: SelectionFilter,
    face: ElementId,
) -> Option<ElementId> {
    let local_position = world_position - occurrence.origin_mm;
    let nearest_endpoint = occurrence
        .geometry
        .endpoints
        .iter()
        .enumerate()
        .map(|(index, point)| (index as u8, local_position.distance(*point)))
        .min_by(|left, right| left.1.total_cmp(&right.1));
    let nearest_edge = BOX_EDGE_ENDPOINTS
        .iter()
        .enumerate()
        .map(|(index, (left, right))| {
            (
                index as u8,
                point_segment_distance(
                    local_position,
                    occurrence.geometry.endpoints[*left],
                    occurrence.geometry.endpoints[*right],
                ),
            )
        })
        .min_by(|left, right| left.1.total_cmp(&right.1));
    let endpoint = nearest_endpoint
        .filter(|candidate| candidate.1 <= tolerance + SNAP_EPSILON)
        .map(|candidate| ElementId::Endpoint(candidate.0));
    let edge = nearest_edge
        .filter(|candidate| candidate.1 <= tolerance + SNAP_EPSILON)
        .map(|candidate| ElementId::Edge(candidate.0));
    match filter {
        SelectionFilter::Face => Some(face),
        SelectionFilter::Edge => edge,
        SelectionFilter::Point => endpoint,
        SelectionFilter::Any => endpoint.or(edge).or(Some(face)),
    }
}

fn point_segment_distance(point: Vec3, start: Vec3, end: Vec3) -> f64 {
    let segment = end - start;
    let from_start = point - start;
    let squared_length = segment.x * segment.x + segment.y * segment.y + segment.z * segment.z;
    let projection =
        (from_start.x * segment.x + from_start.y * segment.y + from_start.z * segment.z)
            / squared_length;
    let closest = start + segment * projection.clamp(0.0, 1.0);
    point.distance(closest)
}

fn hit_occurrence(ray: Ray, occurrence: &Occurrence) -> Option<ExactHit> {
    let local_origin = ray.origin - occurrence.origin_mm;
    let size = occurrence.geometry.size_mm;
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::INFINITY;
    let mut near_face = (Axis::X, Side::Minimum);
    let mut far_face = (Axis::X, Side::Maximum);

    for axis in [Axis::X, Axis::Y, Axis::Z] {
        let origin = local_origin.component(axis);
        let direction = ray.direction.component(axis);
        let maximum = size.component(axis);
        if direction.abs() <= RAY_EPSILON {
            if origin < 0.0 || origin > maximum {
                return None;
            }
            continue;
        }
        let first = -origin / direction;
        let second = (maximum - origin) / direction;
        let (axis_near, axis_far, axis_near_side, axis_far_side) = if first <= second {
            (first, second, Side::Minimum, Side::Maximum)
        } else {
            (second, first, Side::Maximum, Side::Minimum)
        };
        if axis_near > near {
            near = axis_near;
            near_face = (axis, axis_near_side);
        }
        if axis_far < far {
            far = axis_far;
            far_face = (axis, axis_far_side);
        }
        if far < near {
            return None;
        }
    }

    let (distance, face) = if near >= 0.0 {
        (near, near_face)
    } else if far >= 0.0 {
        (far, far_face)
    } else {
        return None;
    };
    Some(ExactHit {
        reference: SelectionId {
            definition_id: occurrence.definition_id,
            occurrence_id: occurrence.id,
            element: ElementId::Face {
                axis: face.0,
                side: face.1,
            },
        },
        position_mm: ray.at(distance),
        ray_distance_mm: distance,
    })
}

fn resolve_snap(primary: ExactHit, occurrence: &Occurrence, tolerance: f64) -> SnapResult {
    let endpoint = occurrence
        .geometry
        .endpoints
        .iter()
        .enumerate()
        .map(|(index, point)| {
            snap_candidate(
                primary,
                occurrence,
                *point,
                ElementId::Endpoint(index as u8),
                SnapKind::Endpoint,
            )
        });
    let midpoint = occurrence
        .geometry
        .edge_midpoints
        .iter()
        .enumerate()
        .map(|(index, point)| {
            snap_candidate(
                primary,
                occurrence,
                *point,
                ElementId::EdgeMidpoint(index as u8),
                SnapKind::Midpoint,
            )
        });
    endpoint
        .chain(midpoint)
        .filter(|candidate| candidate.distance_mm <= tolerance + SNAP_EPSILON)
        .min_by(|left, right| {
            left.distance_mm
                .total_cmp(&right.distance_mm)
                .then_with(|| left.kind.rank().cmp(&right.kind.rank()))
                .then_with(|| left.reference.cmp(&right.reference))
        })
        .unwrap_or(SnapResult {
            kind: SnapKind::Face,
            reference: primary.reference,
            position_mm: primary.position_mm,
            distance_mm: 0.0,
        })
}

fn intersection_snap(
    primary: ExactHit,
    occurrences: &[Occurrence],
    tolerance: f64,
) -> Option<SnapResult> {
    let primary_occurrence = occurrences
        .iter()
        .find(|occurrence| occurrence.id == primary.reference.occurrence_id)?;
    for (primary_edge, (left, right)) in BOX_EDGE_ENDPOINTS.iter().enumerate() {
        let primary_start =
            primary_occurrence.origin_mm + primary_occurrence.geometry.endpoints[*left];
        let primary_end =
            primary_occurrence.origin_mm + primary_occurrence.geometry.endpoints[*right];
        if point_segment_distance(primary.position_mm, primary_start, primary_end)
            > tolerance + SNAP_EPSILON
        {
            continue;
        }
        for other in occurrences
            .iter()
            .filter(|occurrence| occurrence.id != primary_occurrence.id)
        {
            for (other_edge, (other_left, other_right)) in BOX_EDGE_ENDPOINTS.iter().enumerate() {
                let other_start = other.origin_mm + other.geometry.endpoints[*other_left];
                let other_end = other.origin_mm + other.geometry.endpoints[*other_right];
                if point_segment_distance(primary.position_mm, other_start, other_end)
                    <= tolerance + SNAP_EPSILON
                    && !segments_parallel(primary_end - primary_start, other_end - other_start)
                {
                    return Some(SnapResult {
                        kind: SnapKind::Intersection,
                        reference: SelectionId {
                            definition_id: primary.reference.definition_id,
                            occurrence_id: primary.reference.occurrence_id,
                            element: ElementId::Intersection {
                                primary_edge: primary_edge as u8,
                                other_occurrence_id: other.id,
                                other_edge: other_edge as u8,
                            },
                        },
                        position_mm: primary.position_mm,
                        distance_mm: 0.0,
                    });
                }
            }
        }
    }
    None
}

fn segments_parallel(left: Vec3, right: Vec3) -> bool {
    let cross = Vec3::new(
        left.y * right.z - left.z * right.y,
        left.z * right.x - left.x * right.z,
        left.x * right.y - left.y * right.x,
    );
    cross.length() <= SNAP_EPSILON
}

fn better_snap(candidate: &SnapResult, current: &SnapResult) -> bool {
    candidate.distance_mm < current.distance_mm - SNAP_EPSILON
        || ((candidate.distance_mm - current.distance_mm).abs() <= SNAP_EPSILON
            && candidate.kind.rank() < current.kind.rank())
}

fn snap_candidate(
    primary: ExactHit,
    occurrence: &Occurrence,
    local_position: Vec3,
    element: ElementId,
    kind: SnapKind,
) -> SnapResult {
    let position_mm = occurrence.origin_mm + local_position;
    SnapResult {
        kind,
        reference: SelectionId {
            definition_id: occurrence.definition_id,
            occurrence_id: occurrence.id,
            element,
        },
        position_mm,
        distance_mm: primary.position_mm.distance(position_mm),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionDigest {
    pub localization_key: &'static str,
    pub arguments: BTreeMap<&'static str, String>,
    pub command_digest: String,
}

impl ActionDigest {
    #[must_use]
    pub fn render(&self, catalog: &LocaleCatalog) -> String {
        catalog.format(self.localization_key, &self.arguments)
    }
}

#[derive(Clone, Debug)]
pub struct SmartPushPullPlan {
    proposal: Proposal,
    action_digest: ActionDigest,
}

impl SmartPushPullPlan {
    #[must_use]
    pub const fn action_digest(&self) -> &ActionDigest {
        &self.action_digest
    }
}

#[derive(Clone, Debug)]
pub enum SmartPushPullOutcome {
    Ready(SmartPushPullPlan),
    NeedsChoice { candidates: Vec<NodeId> },
}

pub fn plan_smart_push_pull(
    store: &DocumentStore,
    candidates: &[NodeId],
    new_height_text: impl Into<String>,
) -> Result<SmartPushPullOutcome, InteractionError> {
    if candidates.len() != 1 {
        return Ok(SmartPushPullOutcome::NeedsChoice {
            candidates: candidates.to_vec(),
        });
    }
    let target = candidates[0];
    let snapshot = store.current();
    let node = snapshot
        .node(target)
        .ok_or(InteractionError::NodeNotFound(target))?;
    let new_height_text = new_height_text.into();
    let batch = UiAdapter::canonicalize(UiAction::SetDimension {
        target,
        value_text: new_height_text.clone(),
    })?;
    let mut arguments = BTreeMap::new();
    arguments.insert("feature", node.name().to_owned());
    arguments.insert("from", node.dimension().source_token().to_owned());
    arguments.insert("to", new_height_text);
    let action_digest = ActionDigest {
        localization_key: "action-smart-push-pull-height",
        command_digest: batch.digest(),
        arguments,
    };
    Ok(SmartPushPullOutcome::Ready(SmartPushPullPlan {
        proposal: store.prepare_proposal(batch),
        action_digest,
    }))
}

pub struct PreviewSession {
    generation: u64,
    plan: SmartPushPullPlan,
    cancelled: bool,
}

impl PreviewSession {
    #[must_use]
    pub const fn new(generation: u64, plan: SmartPushPullPlan) -> Self {
        Self {
            generation,
            plan,
            cancelled: false,
        }
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn action_digest(&self) -> &ActionDigest {
        &self.plan.action_digest
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    pub fn confirm(self, store: &mut DocumentStore) -> Result<CommittedPreview, PreviewError> {
        if self.cancelled {
            return Err(PreviewError::Cancelled);
        }
        let revision = store.commit_proposal(&self.plan.proposal)?;
        if revision.batch_digest() != self.plan.action_digest.command_digest {
            return Err(PreviewError::DigestMismatch);
        }
        Ok(CommittedPreview {
            revision,
            action_digest: self.plan.action_digest,
        })
    }
}

pub struct CommittedPreview {
    pub revision: Arc<Revision>,
    pub action_digest: ActionDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocaleCatalog {
    messages: BTreeMap<String, String>,
}

impl LocaleCatalog {
    pub fn parse(resource: &str) -> Result<Self, InteractionError> {
        let mut messages = BTreeMap::new();
        for raw_line in resource.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or(InteractionError::InvalidLocaleResource)?;
            let key = key.trim();
            if key.is_empty()
                || messages
                    .insert(key.to_owned(), value.trim().to_owned())
                    .is_some()
            {
                return Err(InteractionError::InvalidLocaleResource);
            }
        }
        Ok(Self { messages })
    }

    #[must_use]
    pub fn english() -> Self {
        Self::parse(include_str!("../../../locales/en-US.ftl"))
            .expect("the embedded English locale must be valid")
    }

    #[must_use]
    pub fn text(&self, key: &str) -> String {
        self.messages
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("[{key}]"))
    }

    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.messages.contains_key(key)
    }

    #[must_use]
    pub fn format(&self, key: &str, arguments: &BTreeMap<&str, String>) -> String {
        let mut value = self
            .messages
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("[{key}]"));
        for (name, replacement) in arguments {
            value = value.replace(&format!("{{ ${name} }}"), replacement);
        }
        value
    }
}

#[derive(Debug, PartialEq)]
pub enum InteractionError {
    InvalidRay,
    InvalidBox,
    DuplicateOccurrence,
    NodeNotFound(NodeId),
    InvalidLocaleResource,
    Adapter(AdapterError),
}

impl fmt::Display for InteractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRay => formatter.write_str("ray must have finite origin and direction"),
            Self::InvalidBox => formatter.write_str("box geometry must be finite and positive"),
            Self::DuplicateOccurrence => {
                formatter.write_str("occurrence ID must be unique and nonzero")
            }
            Self::NodeNotFound(id) => write!(formatter, "node {} does not exist", id.0),
            Self::InvalidLocaleResource => formatter.write_str("locale resource is invalid"),
            Self::Adapter(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for InteractionError {}

impl From<AdapterError> for InteractionError {
    fn from(error: AdapterError) -> Self {
        Self::Adapter(error)
    }
}

#[derive(Debug, PartialEq)]
pub enum PreviewError {
    Cancelled,
    Stale(ProposalCommitError),
    DigestMismatch,
}

impl fmt::Display for PreviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("preview was cancelled"),
            Self::Stale(error) => error.fmt(formatter),
            Self::DigestMismatch => {
                formatter.write_str("preview and committed command digests differ")
            }
        }
    }
}

impl std::error::Error for PreviewError {}

impl From<ProposalCommitError> for PreviewError {
    fn from(error: ProposalCommitError) -> Self {
        Self::Stale(error)
    }
}

#[must_use]
pub fn snapshot_revision(snapshot: &Snapshot) -> u64 {
    snapshot.revision_id()
}
