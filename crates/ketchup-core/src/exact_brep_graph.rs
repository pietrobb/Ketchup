use crate::document::{
    BooleanOperation, DefinitionId, EdgeFinishKind, FeatureId, FeatureKind, LoftSection,
    ProfileSegment, Snapshot, Transform,
};
use crate::sketch::{
    FeatureDirection, FeatureExtent, FeatureExtentEnd, SketchRegionId, SolvedSketchRegion,
    SolvedSketchRegionEdge, SolvedSketchRegionProfile, WorkplaneFrame,
};
use crate::topology::{TopologicalElementKind, TopologicalElementRef};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const EXACT_BREP_GRAPH_SCHEMA_V4: &str = "ketchup.exact-brep-graph.v4";
pub const MAX_EXACT_BREP_GRAPH_PROFILES: usize = 1_024;
pub const MAX_EXACT_BREP_GRAPH_NODES: usize = 1_024;
pub const MAX_EXACT_BREP_GRAPH_SEGMENTS: usize = 16_384;
pub const MAX_EXACT_BREP_LOFT_SECTIONS: usize = 16;
pub const MAX_EXACT_BREP_LOFT_CONTROL_POINTS: usize = 64;
pub const MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS: usize = 64;
pub const MAX_EXACT_BREP_REGION_HOLES: usize = 64;
pub const MAX_EXACT_BREP_REGION_SEGMENTS: usize = 4_096;
pub const MAX_EXACT_BREP_GRAPH_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EXACT_BREP_TOPOLOGY_SELECTORS: usize = 64;
const MAX_ABS_MM: f64 = 1_000_000.0;
const MIN_LENGTH_MM: f64 = 1.0e-7;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ExactBRepProfileId(pub u32);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ExactBRepNodeId(pub u32);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepLinearInterval {
    pub direction_bits: [u64; 3],
    pub start_bits: u64,
    pub end_bits: u64,
}

impl ExactBRepLinearInterval {
    #[must_use]
    pub fn direction(self) -> [f64; 3] {
        self.direction_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn start_mm(self) -> f64 {
        f64::from_bits(self.start_bits)
    }

    #[must_use]
    pub fn end_mm(self) -> f64 {
        f64::from_bits(self.end_bits)
    }

    #[must_use]
    pub fn length_mm(self) -> f64 {
        self.end_mm() - self.start_mm()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactBRepBooleanOperation {
    Cut,
    Union,
    Intersect,
    Split,
}

impl From<BooleanOperation> for ExactBRepBooleanOperation {
    fn from(operation: BooleanOperation) -> Self {
        match operation {
            BooleanOperation::Cut => Self::Cut,
            BooleanOperation::Union => Self::Union,
            BooleanOperation::Intersect => Self::Intersect,
            BooleanOperation::Split => Self::Split,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactBRepTopologyKind {
    Face,
    Edge,
}

impl ExactBRepTopologyKind {
    const fn element_kind(self) -> TopologicalElementKind {
        match self {
            Self::Face => TopologicalElementKind::Face,
            Self::Edge => TopologicalElementKind::Edge,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepTopologySelector {
    pub kind: ExactBRepTopologyKind,
    pub reference_bytes: Vec<u8>,
}

impl ExactBRepTopologySelector {
    pub fn reference(&self) -> Result<TopologicalElementRef, ExactBRepGraphError> {
        let reference = TopologicalElementRef::from_bytes(&self.reference_bytes)
            .map_err(|_| ExactBRepGraphError::InvalidTopologySelector)?;
        if reference.kind != self.kind.element_kind() {
            return Err(ExactBRepGraphError::InvalidTopologySelector);
        }
        Ok(reference)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactBRepEdgeFinishKind {
    Fillet,
    Chamfer,
}

impl From<EdgeFinishKind> for ExactBRepEdgeFinishKind {
    fn from(kind: EdgeFinishKind) -> Self {
        match kind {
            EdgeFinishKind::Fillet => Self::Fillet,
            EdgeFinishKind::Chamfer => Self::Chamfer,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExactBRepPlanarSegment {
    Line {
        start_bits: [u64; 2],
        end_bits: [u64; 2],
    },
    CircularArc {
        start_bits: [u64; 2],
        end_bits: [u64; 2],
        center_bits: [u64; 2],
        clockwise: bool,
    },
    CubicBezier {
        start_bits: [u64; 2],
        control_1_bits: [u64; 2],
        control_2_bits: [u64; 2],
        end_bits: [u64; 2],
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExactBRepPlanarLoop {
    Boundary {
        segments: Vec<ExactBRepPlanarSegment>,
    },
    Circle {
        center_bits: [u64; 2],
        radius_bits: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExactBRepPlanarGeometry {
    Boundary {
        closed: bool,
        segments: Vec<ExactBRepPlanarSegment>,
    },
    Circle {
        center_bits: [u64; 2],
        radius_bits: u64,
    },
    Spline {
        control_point_bits: Vec<[u64; 2]>,
    },
    Region {
        outer: ExactBRepPlanarLoop,
        holes: Vec<ExactBRepPlanarLoop>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepProfile {
    pub id: ExactBRepProfileId,
    pub source_feature_id: u64,
    pub region_id: Option<u64>,
    pub frame_bits: [u64; 12],
    pub geometry: ExactBRepPlanarGeometry,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepLoftSection {
    pub profile: ExactBRepProfileId,
    pub elevation_bits: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExactBRepOperation {
    Extrude {
        profile: ExactBRepProfileId,
        distance_bits: u64,
        interval: ExactBRepLinearInterval,
    },
    ProfileCut {
        target: ExactBRepNodeId,
        profile: ExactBRepProfileId,
        depth_bits: Option<u64>,
        interval: ExactBRepLinearInterval,
        support_lineage_digest: Option<String>,
    },
    Boolean {
        operation: ExactBRepBooleanOperation,
        target: ExactBRepNodeId,
        tool: ExactBRepNodeId,
    },
    Shell {
        target: ExactBRepNodeId,
        removed_faces: Vec<ExactBRepTopologySelector>,
        thickness_bits: u64,
    },
    EdgeFinish {
        target: ExactBRepNodeId,
        edges: Vec<ExactBRepTopologySelector>,
        kind: ExactBRepEdgeFinishKind,
        amount_bits: u64,
    },
    FaceOffset {
        target: ExactBRepNodeId,
        face: ExactBRepTopologySelector,
        distance_bits: u64,
    },
    Revolve {
        profile: ExactBRepProfileId,
        axis_start_bits: [u64; 2],
        axis_end_bits: [u64; 2],
        angle_degrees_bits: u64,
    },
    Sweep {
        profile: ExactBRepProfileId,
        path: ExactBRepProfileId,
    },
    Loft {
        sections: Vec<ExactBRepLoftSection>,
    },
    ImportedExact {
        source_sha256: [u8; 32],
        source_byte_len: u64,
        result_fingerprint: String,
    },
    RigidTransform {
        target: ExactBRepNodeId,
        matrix_bits: [u64; 16],
    },
}

impl ExactBRepOperation {
    fn dependencies(&self) -> Vec<ExactBRepNodeId> {
        match self {
            Self::ProfileCut { target, .. }
            | Self::Shell { target, .. }
            | Self::EdgeFinish { target, .. }
            | Self::FaceOffset { target, .. }
            | Self::RigidTransform { target, .. } => vec![*target],
            Self::Boolean { target, tool, .. } => vec![*target, *tool],
            Self::Extrude { .. }
            | Self::Revolve { .. }
            | Self::Sweep { .. }
            | Self::Loft { .. }
            | Self::ImportedExact { .. } => Vec::new(),
        }
    }

    fn profile_ids(&self) -> Vec<ExactBRepProfileId> {
        match self {
            Self::Extrude { profile, .. }
            | Self::ProfileCut { profile, .. }
            | Self::Revolve { profile, .. } => vec![*profile],
            Self::Sweep { profile, path } => vec![*profile, *path],
            Self::Loft { sections } => sections.iter().map(|section| section.profile).collect(),
            Self::Boolean { .. }
            | Self::Shell { .. }
            | Self::EdgeFinish { .. }
            | Self::FaceOffset { .. }
            | Self::ImportedExact { .. }
            | Self::RigidTransform { .. } => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepNode {
    pub id: ExactBRepNodeId,
    pub source_feature_id: u64,
    pub operation: ExactBRepOperation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepGraph {
    pub schema: String,
    pub document_id: u64,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: u64,
    pub producer_feature_id: u64,
    pub profiles: Vec<ExactBRepProfile>,
    pub nodes: Vec<ExactBRepNode>,
    pub graph_digest: String,
    pub canonical_input_digest: String,
}

#[derive(Serialize)]
struct GraphDigestPayload<'a> {
    schema: &'a str,
    definition_id: u64,
    producer_feature_id: u64,
    profiles: &'a [ExactBRepProfile],
    nodes: &'a [ExactBRepNode],
}

impl ExactBRepGraph {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
    ) -> Result<Self, ExactBRepGraphError> {
        snapshot
            .feature_dependency_graph()
            .map_err(|_| ExactBRepGraphError::InvalidDependencyGraph)?;
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactBRepGraphError::DefinitionNotFound(definition_id))?;
        if !definition.feature_ids().contains(&producer_feature_id) {
            return Err(ExactBRepGraphError::FeatureNotFound(producer_feature_id));
        }
        let mut compiler = GraphCompiler::new(snapshot, definition_id);
        compiler.compile_body(producer_feature_id)?;
        let source_digest = snapshot.canonical_digest();
        let mut graph = Self {
            schema: EXACT_BREP_GRAPH_SCHEMA_V4.to_owned(),
            document_id: snapshot.document_id().0,
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id: definition_id.0,
            producer_feature_id: producer_feature_id.0,
            profiles: compiler.profiles,
            nodes: compiler.nodes,
            graph_digest: String::new(),
            canonical_input_digest: String::new(),
        };
        graph.graph_digest = graph.compute_graph_digest()?;
        graph.canonical_input_digest = graph.compute_canonical_input_digest();
        graph.validate()?;
        Ok(graph)
    }

    pub fn producer_bounds_mm(&self) -> Result<Option<[[f64; 3]; 2]>, ExactBRepGraphError> {
        self.validate()?;
        let mut node_bounds = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            node_bounds.push(operation_bounds(
                &node.operation,
                &self.profiles,
                &node_bounds,
            )?);
        }
        Ok(node_bounds.last().copied().flatten())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, ExactBRepGraphError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| ExactBRepGraphError::Serialization(error.to_string()))?;
        if bytes.len() > MAX_EXACT_BREP_GRAPH_BYTES {
            return Err(ExactBRepGraphError::ResourceLimit);
        }
        Ok(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ExactBRepGraphError> {
        if bytes.len() > MAX_EXACT_BREP_GRAPH_BYTES {
            return Err(ExactBRepGraphError::ResourceLimit);
        }
        let graph: Self = serde_json::from_slice(bytes)
            .map_err(|error| ExactBRepGraphError::Serialization(error.to_string()))?;
        graph.validate()?;
        Ok(graph)
    }

    pub fn validate(&self) -> Result<(), ExactBRepGraphError> {
        if self.schema != EXACT_BREP_GRAPH_SCHEMA_V4
            || self.document_id == 0
            || self.definition_id == 0
            || self.producer_feature_id == 0
            || self.source_digest.is_empty()
            || self.profiles.len() > MAX_EXACT_BREP_GRAPH_PROFILES
            || self.nodes.is_empty()
            || self.nodes.len() > MAX_EXACT_BREP_GRAPH_NODES
        {
            return Err(ExactBRepGraphError::InvalidGraph);
        }
        let mut segment_count = 0_usize;
        for (index, profile) in self.profiles.iter().enumerate() {
            if profile.id != ExactBRepProfileId(index as u32)
                || profile.source_feature_id == 0
                || !valid_frame(profile.frame_bits)
            {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            segment_count = segment_count
                .checked_add(validate_geometry(&profile.geometry)?)
                .ok_or(ExactBRepGraphError::ResourceLimit)?;
        }
        if segment_count > MAX_EXACT_BREP_GRAPH_SEGMENTS {
            return Err(ExactBRepGraphError::ResourceLimit);
        }
        let mut source_features = BTreeSet::new();
        let mut imported_source_nodes = 0_usize;
        let mut imported_source_bytes = 0_u64;
        for (index, node) in self.nodes.iter().enumerate() {
            let id = ExactBRepNodeId(index as u32);
            if let ExactBRepOperation::ImportedExact {
                source_byte_len, ..
            } = node.operation
            {
                imported_source_nodes += 1;
                imported_source_bytes = imported_source_bytes
                    .checked_add(source_byte_len)
                    .ok_or(ExactBRepGraphError::ResourceLimit)?;
                if imported_source_nodes > 64 || imported_source_bytes > 128 * 1024 * 1024 {
                    return Err(ExactBRepGraphError::ResourceLimit);
                }
            }
            if node.id != id
                || node.source_feature_id == 0
                || !source_features.insert(node.source_feature_id)
                || node
                    .operation
                    .dependencies()
                    .iter()
                    .any(|dependency| dependency.0 >= id.0)
                || node
                    .operation
                    .profile_ids()
                    .iter()
                    .any(|profile| profile.0 as usize >= self.profiles.len())
                || !valid_operation_profiles(&node.operation, &self.profiles)
                || !valid_operation(
                    &node.operation,
                    self.document_id,
                    self.definition_id,
                    &self.nodes[..index],
                )
            {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
        }
        if self.nodes.last().map(|node| node.source_feature_id) != Some(self.producer_feature_id)
            || self.graph_digest != self.compute_graph_digest()?
            || self.canonical_input_digest != self.compute_canonical_input_digest()
        {
            return Err(ExactBRepGraphError::DigestMismatch);
        }
        Ok(())
    }

    fn compute_graph_digest(&self) -> Result<String, ExactBRepGraphError> {
        let payload = GraphDigestPayload {
            schema: &self.schema,
            definition_id: self.definition_id,
            producer_feature_id: self.producer_feature_id,
            profiles: &self.profiles,
            nodes: &self.nodes,
        };
        let bytes = serde_json::to_vec(&payload)
            .map_err(|error| ExactBRepGraphError::Serialization(error.to_string()))?;
        Ok(digest(&bytes))
    }

    fn compute_canonical_input_digest(&self) -> String {
        digest(
            format!(
                "{}:{}:{}:{}:{}:{}:{}",
                self.schema,
                self.document_id,
                self.source_revision,
                self.source_digest,
                self.definition_id,
                self.producer_feature_id,
                self.graph_digest,
            )
            .as_bytes(),
        )
    }
}

struct GraphCompiler<'a> {
    snapshot: &'a Snapshot,
    definition_id: DefinitionId,
    profiles: Vec<ExactBRepProfile>,
    nodes: Vec<ExactBRepNode>,
    node_bounds: Vec<Option<[[f64; 3]; 2]>>,
    compiled_bodies: BTreeMap<FeatureId, ExactBRepNodeId>,
    profile_ids: BTreeMap<(FeatureId, Option<SketchRegionId>, [u64; 12]), ExactBRepProfileId>,
    visiting: BTreeSet<FeatureId>,
}

impl<'a> GraphCompiler<'a> {
    fn new(snapshot: &'a Snapshot, definition_id: DefinitionId) -> Self {
        Self {
            snapshot,
            definition_id,
            profiles: Vec::new(),
            nodes: Vec::new(),
            node_bounds: Vec::new(),
            compiled_bodies: BTreeMap::new(),
            profile_ids: BTreeMap::new(),
            visiting: BTreeSet::new(),
        }
    }

    fn compile_body(
        &mut self,
        feature_id: FeatureId,
    ) -> Result<ExactBRepNodeId, ExactBRepGraphError> {
        if let Some(id) = self.compiled_bodies.get(&feature_id) {
            return Ok(*id);
        }
        if !self.visiting.insert(feature_id) {
            return Err(ExactBRepGraphError::DependencyCycle(feature_id));
        }
        let feature = self
            .snapshot
            .feature(feature_id)
            .filter(|feature| feature.definition_id() == self.definition_id)
            .ok_or(ExactBRepGraphError::FeatureNotFound(feature_id))?;
        if self.snapshot.feature_is_suppressed(feature_id) {
            return Err(ExactBRepGraphError::SuppressedFeature(feature_id));
        }
        let operation = match feature.kind() {
            FeatureKind::Extrusion { profile, height } => {
                let profile = self.compile_profile(*profile, None, identity_frame())?;
                let interval = linear_interval([0.0, 0.0, 1.0], 0.0, height.millimetres())?;
                ExactBRepOperation::Extrude {
                    profile,
                    distance_bits: positive_distance(interval.length_mm())?,
                    interval,
                }
            }
            FeatureKind::Pad(spec) => {
                let (profile, origin_mm, direction) =
                    self.compile_sketch_profile(spec.sketch, spec.region, spec.direction)?;
                let interval = self.resolve_extent(origin_mm, direction, &spec.extent, None)?;
                ExactBRepOperation::Extrude {
                    profile,
                    distance_bits: positive_distance(interval.length_mm())?,
                    interval,
                }
            }
            FeatureKind::SketchPocket(spec) => {
                if !matches!(spec.extent, FeatureExtent::Blind(_))
                    && self
                        .snapshot
                        .exact_reference_by_lineage(&spec.support.lineage_digest)
                        != Some(spec.support.as_ref())
                {
                    return Err(ExactBRepGraphError::UnresolvedExtent);
                }
                let target = self.compile_body(spec.target)?;
                let target_bounds = self.node_bounds[target.0 as usize];
                let (profile, origin_mm, direction) =
                    self.compile_sketch_profile(spec.sketch, spec.region, spec.direction)?;
                let interval =
                    self.resolve_extent(origin_mm, direction, &spec.extent, target_bounds)?;
                ExactBRepOperation::ProfileCut {
                    target,
                    profile,
                    depth_bits: Some(positive_distance(interval.length_mm())?),
                    interval,
                    support_lineage_digest: Some(spec.support.lineage_digest.clone()),
                }
            }
            FeatureKind::ThroughCut { target, profile } => {
                let target = self.compile_body(*target)?;
                let target_bounds = self.node_bounds[target.0 as usize]
                    .ok_or(ExactBRepGraphError::UnresolvedExtent)?;
                ExactBRepOperation::ProfileCut {
                    target,
                    profile: self.compile_profile(*profile, None, identity_frame())?,
                    depth_bits: None,
                    interval: through_all_interval(
                        [0.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0],
                        target_bounds,
                    )?,
                    support_lineage_digest: None,
                }
            }
            FeatureKind::Pocket {
                target,
                profile,
                depth,
            } => ExactBRepOperation::ProfileCut {
                target: self.compile_body(*target)?,
                profile: self.compile_profile(*profile, None, identity_frame())?,
                depth_bits: Some(positive_distance(depth.millimetres())?),
                interval: linear_interval([0.0, 0.0, 1.0], 0.0, depth.millimetres())?,
                support_lineage_digest: None,
            },
            FeatureKind::Boolean {
                operation,
                target,
                tool,
            } => ExactBRepOperation::Boolean {
                operation: (*operation).into(),
                target: self.compile_body(*target)?,
                tool: self.compile_body(*tool)?,
            },
            FeatureKind::TopologyShell {
                target,
                removed_faces,
                thickness,
            } => ExactBRepOperation::Shell {
                target: self.compile_body(*target)?,
                removed_faces: topology_selectors(
                    removed_faces,
                    TopologicalElementKind::Face,
                    *target,
                )?,
                thickness_bits: positive_distance(thickness.millimetres())?,
            },
            FeatureKind::TopologyEdgeFinish {
                target,
                edges,
                kind,
                amount,
            } => ExactBRepOperation::EdgeFinish {
                target: self.compile_body(*target)?,
                edges: topology_selectors(edges, TopologicalElementKind::Edge, *target)?,
                kind: (*kind).into(),
                amount_bits: positive_distance(amount.millimetres())?,
            },
            FeatureKind::TopologyFaceOffset {
                target,
                face,
                distance,
            } => ExactBRepOperation::FaceOffset {
                target: self.compile_body(*target)?,
                face: topology_selectors(
                    std::slice::from_ref(face),
                    TopologicalElementKind::Face,
                    *target,
                )?
                .pop()
                .ok_or(ExactBRepGraphError::InvalidTopologySelector)?,
                distance_bits: signed_distance(distance.millimetres())?,
            },
            FeatureKind::Revolve {
                profile,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } => {
                let compiled_profile = match self
                    .snapshot
                    .feature(*profile)
                    .map(|feature| feature.kind())
                {
                    Some(FeatureKind::Sketch(sketch)) => {
                        let regions = sketch
                            .solved_regions()
                            .map_err(|_| ExactBRepGraphError::UnsupportedProfile(*profile))?;
                        let [region] = regions.as_slice() else {
                            return Err(ExactBRepGraphError::UnsupportedProfile(*profile));
                        };
                        self.compile_sketch_profile(
                            *profile,
                            region.id,
                            FeatureDirection::AlongNormal,
                        )?
                        .0
                    }
                    _ => self.compile_profile(*profile, None, identity_frame())?,
                };
                ExactBRepOperation::Revolve {
                    profile: compiled_profile,
                    axis_start_bits: axis_start_mm.map(f64::to_bits),
                    axis_end_bits: axis_end_mm.map(f64::to_bits),
                    angle_degrees_bits: angle_degrees.to_bits(),
                }
            }
            FeatureKind::Sweep { profile, path } => ExactBRepOperation::Sweep {
                profile: self.compile_profile(*profile, None, identity_frame())?,
                path: self.compile_profile(*path, None, identity_frame())?,
            },
            FeatureKind::Loft { sections } => ExactBRepOperation::Loft {
                sections: self.compile_loft_sections(sections)?,
            },
            FeatureKind::ImportedExactBody(spec) => ExactBRepOperation::ImportedExact {
                source_sha256: spec.source_sha256,
                source_byte_len: spec.source_byte_len,
                result_fingerprint: spec.result_fingerprint.clone(),
            },
            FeatureKind::RigidTransform { target, transform } => {
                ExactBRepOperation::RigidTransform {
                    target: self.compile_body(*target)?,
                    matrix_bits: transform.matrix().map(f64::to_bits),
                }
            }
            _ => return Err(ExactBRepGraphError::UnsupportedFeature(feature_id)),
        };
        let bounds = operation_bounds(&operation, &self.profiles, &self.node_bounds)?;
        let id = ExactBRepNodeId(
            self.nodes
                .len()
                .try_into()
                .map_err(|_| ExactBRepGraphError::ResourceLimit)?,
        );
        if self.nodes.len() >= MAX_EXACT_BREP_GRAPH_NODES {
            return Err(ExactBRepGraphError::ResourceLimit);
        }
        self.nodes.push(ExactBRepNode {
            id,
            source_feature_id: feature_id.0,
            operation,
        });
        self.node_bounds.push(bounds);
        self.visiting.remove(&feature_id);
        self.compiled_bodies.insert(feature_id, id);
        Ok(id)
    }

    fn compile_loft_sections(
        &mut self,
        sections: &[LoftSection],
    ) -> Result<Vec<ExactBRepLoftSection>, ExactBRepGraphError> {
        if !(2..=MAX_EXACT_BREP_LOFT_SECTIONS).contains(&sections.len()) {
            return Err(ExactBRepGraphError::InvalidParameter);
        }
        sections
            .iter()
            .map(|section| {
                Ok(ExactBRepLoftSection {
                    profile: self.compile_profile(section.profile, None, identity_frame())?,
                    elevation_bits: finite_coordinate(section.elevation_mm)?.to_bits(),
                })
            })
            .collect()
    }

    fn compile_sketch_profile(
        &mut self,
        sketch_id: FeatureId,
        region_id: SketchRegionId,
        direction: FeatureDirection,
    ) -> Result<(ExactBRepProfileId, [f64; 3], [f64; 3]), ExactBRepGraphError> {
        let sketch = self
            .snapshot
            .feature(sketch_id)
            .and_then(|feature| match feature.kind() {
                FeatureKind::Sketch(spec) => Some(spec),
                _ => None,
            })
            .ok_or(ExactBRepGraphError::UnsupportedProfile(sketch_id))?;
        let workplane = self
            .snapshot
            .feature(sketch.workplane)
            .and_then(|feature| match feature.kind() {
                FeatureKind::Workplane(spec) => Some(spec),
                _ => None,
            })
            .ok_or(ExactBRepGraphError::UnsupportedProfile(sketch_id))?;
        let direction = direction
            .vector(workplane.frame.normal)
            .ok_or(ExactBRepGraphError::InvalidParameter)?;
        let frame = frame_bits(workplane.frame, direction);
        Ok((
            self.compile_profile(sketch_id, Some(region_id), frame)?,
            workplane.frame.origin_mm,
            direction,
        ))
    }

    fn resolve_extent(
        &self,
        origin_mm: [f64; 3],
        direction: [f64; 3],
        extent: &FeatureExtent,
        target_bounds: Option<[[f64; 3]; 2]>,
    ) -> Result<ExactBRepLinearInterval, ExactBRepGraphError> {
        match extent {
            FeatureExtent::Blind(distance) => {
                linear_interval(direction, 0.0, distance.millimetres())
            }
            FeatureExtent::ThroughAll => through_all_interval(
                origin_mm,
                direction,
                target_bounds.ok_or(ExactBRepGraphError::UnresolvedExtent)?,
            ),
            FeatureExtent::UpToFace(reference) => linear_interval(
                direction,
                0.0,
                self.resolve_face_distance(origin_mm, direction, reference)?,
            ),
            FeatureExtent::Symmetric(distance) => {
                let half = distance.millimetres() * 0.5;
                linear_interval(direction, -half, half)
            }
            FeatureExtent::Bidirectional { along, opposite } => {
                let along = self.resolve_extent_end(origin_mm, direction, along, target_bounds)?;
                let opposite = self.resolve_extent_end(
                    origin_mm,
                    direction.map(|component| -component),
                    opposite,
                    target_bounds,
                )?;
                linear_interval(direction, -opposite, along)
            }
        }
    }

    fn resolve_extent_end(
        &self,
        origin_mm: [f64; 3],
        direction: [f64; 3],
        end: &FeatureExtentEnd,
        target_bounds: Option<[[f64; 3]; 2]>,
    ) -> Result<f64, ExactBRepGraphError> {
        match end {
            FeatureExtentEnd::Blind(distance) => Ok(distance.millimetres()),
            FeatureExtentEnd::ThroughAll => through_all_distance(
                origin_mm,
                direction,
                target_bounds.ok_or(ExactBRepGraphError::UnresolvedExtent)?,
            ),
            FeatureExtentEnd::UpToFace(reference) => {
                self.resolve_face_distance(origin_mm, direction, reference)
            }
        }
    }

    fn resolve_face_distance(
        &self,
        origin_mm: [f64; 3],
        direction: [f64; 3],
        reference: &crate::exact_product::BodySubshapeRef,
    ) -> Result<f64, ExactBRepGraphError> {
        let frame = self
            .snapshot
            .resolved_planar_face_workplane_frame(reference)
            .ok_or(ExactBRepGraphError::UnresolvedExtent)?;
        let denominator = dot(direction, frame.normal);
        if denominator.abs() <= MIN_LENGTH_MM {
            return Err(ExactBRepGraphError::AmbiguousExtent);
        }
        let distance = dot(subtract(frame.origin_mm, origin_mm), frame.normal) / denominator;
        if !distance.is_finite() || distance <= MIN_LENGTH_MM || distance > MAX_ABS_MM {
            return Err(ExactBRepGraphError::UnresolvedExtent);
        }
        let request = crate::exact_product::ExactFeatureChainRequest::from_snapshot_for_producer(
            self.snapshot,
            reference.definition_id,
            reference.producer_feature_id,
        )
        .map_err(|_| ExactBRepGraphError::UnresolvedExtent)?;
        if !reference.matches_durable_request_identity(&request) {
            return Err(ExactBRepGraphError::UnresolvedExtent);
        }
        let intersection = [0, 1, 2].map(|axis| origin_mm[axis] + direction[axis] * distance);
        let bounds = request.expected_bounds_mm();
        let tolerance = 1.0e-7;
        if (0..3).any(|axis| {
            intersection[axis] < bounds[0][axis] - tolerance
                || intersection[axis] > bounds[1][axis] + tolerance
        }) {
            return Err(ExactBRepGraphError::UnresolvedExtent);
        }
        Ok(distance)
    }

    fn compile_profile(
        &mut self,
        feature_id: FeatureId,
        region_id: Option<SketchRegionId>,
        frame_bits: [u64; 12],
    ) -> Result<ExactBRepProfileId, ExactBRepGraphError> {
        let key = (feature_id, region_id, frame_bits);
        if let Some(id) = self.profile_ids.get(&key) {
            return Ok(*id);
        }
        if self.profiles.len() >= MAX_EXACT_BREP_GRAPH_PROFILES {
            return Err(ExactBRepGraphError::ResourceLimit);
        }
        let feature = self
            .snapshot
            .feature(feature_id)
            .filter(|feature| feature.definition_id() == self.definition_id)
            .ok_or(ExactBRepGraphError::FeatureNotFound(feature_id))?;
        if self.snapshot.feature_is_suppressed(feature_id) {
            return Err(ExactBRepGraphError::SuppressedFeature(feature_id));
        }
        let geometry = match (feature.kind(), region_id) {
            (FeatureKind::Profile { points_mm }, None) => polygon_geometry(points_mm)?,
            (FeatureKind::SegmentProfile { segments, closed }, None) => {
                boundary_geometry(segments, *closed)?
            }
            (FeatureKind::SplineProfile { control_points_mm }, None) => {
                ExactBRepPlanarGeometry::Spline {
                    control_point_bits: point_bits(control_points_mm)?,
                }
            }
            (FeatureKind::Sketch(sketch), Some(region_id)) => {
                let region = sketch
                    .solved_regions()
                    .map_err(|_| ExactBRepGraphError::UnsupportedProfile(feature_id))?
                    .into_iter()
                    .find(|region| region.id == region_id)
                    .ok_or(ExactBRepGraphError::UnsupportedProfile(feature_id))?;
                solved_geometry(&region)?
            }
            _ => return Err(ExactBRepGraphError::UnsupportedProfile(feature_id)),
        };
        let id = ExactBRepProfileId(
            self.profiles
                .len()
                .try_into()
                .map_err(|_| ExactBRepGraphError::ResourceLimit)?,
        );
        self.profiles.push(ExactBRepProfile {
            id,
            source_feature_id: feature_id.0,
            region_id: region_id.map(|id| id.0),
            frame_bits,
            geometry,
        });
        self.profile_ids.insert(key, id);
        Ok(id)
    }
}

fn topology_selectors(
    references: &[TopologicalElementRef],
    expected_kind: TopologicalElementKind,
    target_feature_id: FeatureId,
) -> Result<Vec<ExactBRepTopologySelector>, ExactBRepGraphError> {
    if references.is_empty() || references.len() > MAX_EXACT_BREP_TOPOLOGY_SELECTORS {
        return Err(ExactBRepGraphError::InvalidTopologySelector);
    }
    references
        .iter()
        .map(|reference| {
            if reference.kind != expected_kind
                || reference.producer_feature_id != target_feature_id
                || !reference.has_valid_lineage()
            {
                return Err(ExactBRepGraphError::InvalidTopologySelector);
            }
            Ok(ExactBRepTopologySelector {
                kind: match expected_kind {
                    TopologicalElementKind::Face => ExactBRepTopologyKind::Face,
                    TopologicalElementKind::Edge => ExactBRepTopologyKind::Edge,
                    TopologicalElementKind::Vertex => {
                        return Err(ExactBRepGraphError::InvalidTopologySelector);
                    }
                },
                reference_bytes: reference
                    .to_bytes()
                    .map_err(|_| ExactBRepGraphError::InvalidTopologySelector)?,
            })
        })
        .collect()
}

fn solved_geometry(
    region: &SolvedSketchRegion,
) -> Result<ExactBRepPlanarGeometry, ExactBRepGraphError> {
    if region.holes.is_empty() {
        return match solved_loop(&region.outer)? {
            ExactBRepPlanarLoop::Boundary { segments } => Ok(ExactBRepPlanarGeometry::Boundary {
                closed: true,
                segments,
            }),
            ExactBRepPlanarLoop::Circle {
                center_bits,
                radius_bits,
            } => Ok(ExactBRepPlanarGeometry::Circle {
                center_bits,
                radius_bits,
            }),
        };
    }
    Ok(ExactBRepPlanarGeometry::Region {
        outer: solved_loop(&region.outer)?,
        holes: region
            .holes
            .iter()
            .map(solved_loop)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn solved_loop(
    profile: &SolvedSketchRegionProfile,
) -> Result<ExactBRepPlanarLoop, ExactBRepGraphError> {
    match profile {
        SolvedSketchRegionProfile::Polyline(points) => {
            let ExactBRepPlanarGeometry::Boundary { segments, .. } = polygon_geometry(points)?
            else {
                unreachable!()
            };
            Ok(ExactBRepPlanarLoop::Boundary { segments })
        }
        SolvedSketchRegionProfile::Boundary(edges) => Ok(ExactBRepPlanarLoop::Boundary {
            segments: edges
                .iter()
                .map(|edge| match edge {
                    SolvedSketchRegionEdge::Line { start_mm, end_mm } => {
                        profile_segment(*start_mm, *end_mm, None, false)
                    }
                    SolvedSketchRegionEdge::Arc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    } => profile_segment(*start_mm, *end_mm, Some(*center_mm), *clockwise),
                    SolvedSketchRegionEdge::CubicBezier {
                        start_mm,
                        control_1_mm,
                        control_2_mm,
                        end_mm,
                    } => cubic_bezier_segment(*start_mm, *control_1_mm, *control_2_mm, *end_mm),
                })
                .collect::<Result<Vec<_>, _>>()?,
        }),
        SolvedSketchRegionProfile::Circle {
            center_mm,
            radius_mm,
        } => Ok(ExactBRepPlanarLoop::Circle {
            center_bits: valid_point(*center_mm)?.map(f64::to_bits),
            radius_bits: positive_distance(*radius_mm)?,
        }),
    }
}

fn polygon_geometry(points: &[[f64; 2]]) -> Result<ExactBRepPlanarGeometry, ExactBRepGraphError> {
    if points.len() < 3 {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    let segments = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(start, end)| profile_segment(*start, *end, None, false))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExactBRepPlanarGeometry::Boundary {
        closed: true,
        segments,
    })
}

fn boundary_geometry(
    segments: &[ProfileSegment],
    closed: bool,
) -> Result<ExactBRepPlanarGeometry, ExactBRepGraphError> {
    if segments.is_empty() {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    let segments = segments
        .iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                profile_segment(*start_mm, *end_mm, None, false)
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => profile_segment(*start_mm, *end_mm, Some(*center_mm), *clockwise),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExactBRepPlanarGeometry::Boundary { closed, segments })
}

fn profile_segment(
    start_mm: [f64; 2],
    end_mm: [f64; 2],
    center_mm: Option<[f64; 2]>,
    clockwise: bool,
) -> Result<ExactBRepPlanarSegment, ExactBRepGraphError> {
    let start_bits = valid_point(start_mm)?.map(f64::to_bits);
    let end_bits = valid_point(end_mm)?.map(f64::to_bits);
    if start_bits == end_bits {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    Ok(if let Some(center_mm) = center_mm {
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits: valid_point(center_mm)?.map(f64::to_bits),
            clockwise,
        }
    } else {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        }
    })
}

fn cubic_bezier_segment(
    start_mm: [f64; 2],
    control_1_mm: [f64; 2],
    control_2_mm: [f64; 2],
    end_mm: [f64; 2],
) -> Result<ExactBRepPlanarSegment, ExactBRepGraphError> {
    let start = valid_point(start_mm)?;
    let end = valid_point(end_mm)?;
    if (start[0] - end[0]).hypot(start[1] - end[1]) <= MIN_LENGTH_MM {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    Ok(ExactBRepPlanarSegment::CubicBezier {
        start_bits: start.map(f64::to_bits),
        control_1_bits: valid_point(control_1_mm)?.map(f64::to_bits),
        control_2_bits: valid_point(control_2_mm)?.map(f64::to_bits),
        end_bits: end.map(f64::to_bits),
    })
}

fn point_bits(points: &[[f64; 2]]) -> Result<Vec<[u64; 2]>, ExactBRepGraphError> {
    if points.len() < 2 {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    points
        .iter()
        .map(|point| Ok(valid_point(*point)?.map(f64::to_bits)))
        .collect()
}

fn identity_frame() -> [u64; 12] {
    frame_bits(
        WorkplaneFrame::principal(crate::sketch::PrincipalPlane::Xy),
        [0.0, 0.0, 1.0],
    )
}

fn frame_bits(frame: WorkplaneFrame, direction: [f64; 3]) -> [u64; 12] {
    [
        frame.origin_mm[0],
        frame.origin_mm[1],
        frame.origin_mm[2],
        frame.x_axis[0],
        frame.x_axis[1],
        frame.x_axis[2],
        frame.y_axis[0],
        frame.y_axis[1],
        frame.y_axis[2],
        direction[0],
        direction[1],
        direction[2],
    ]
    .map(f64::to_bits)
}

fn linear_interval(
    direction: [f64; 3],
    start_mm: f64,
    end_mm: f64,
) -> Result<ExactBRepLinearInterval, ExactBRepGraphError> {
    let length = direction[0].hypot(direction[1]).hypot(direction[2]);
    if direction.iter().any(|component| !component.is_finite())
        || (length - 1.0).abs() > 1.0e-9
        || !start_mm.is_finite()
        || !end_mm.is_finite()
        || start_mm.abs() > MAX_ABS_MM
        || end_mm.abs() > MAX_ABS_MM
        || end_mm - start_mm <= MIN_LENGTH_MM
    {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    Ok(ExactBRepLinearInterval {
        direction_bits: direction.map(canonical_bits),
        start_bits: canonical_bits(start_mm),
        end_bits: canonical_bits(end_mm),
    })
}

fn through_all_interval(
    origin_mm: [f64; 3],
    direction: [f64; 3],
    target_bounds: [[f64; 3]; 2],
) -> Result<ExactBRepLinearInterval, ExactBRepGraphError> {
    let [minimum, maximum] = projected_bounds(origin_mm, direction, target_bounds)?;
    linear_interval(direction, minimum - 1.0, maximum + 1.0)
}

fn through_all_distance(
    origin_mm: [f64; 3],
    direction: [f64; 3],
    target_bounds: [[f64; 3]; 2],
) -> Result<f64, ExactBRepGraphError> {
    let [_minimum, maximum] = projected_bounds(origin_mm, direction, target_bounds)?;
    let distance = maximum + 1.0;
    if distance > MAX_ABS_MM {
        return Err(ExactBRepGraphError::ResourceLimit);
    }
    Ok(distance)
}

fn projected_bounds(
    origin_mm: [f64; 3],
    direction: [f64; 3],
    target_bounds: [[f64; 3]; 2],
) -> Result<[f64; 2], ExactBRepGraphError> {
    if !valid_bounds(target_bounds) {
        return Err(ExactBRepGraphError::UnresolvedExtent);
    }
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for x in [target_bounds[0][0], target_bounds[1][0]] {
        for y in [target_bounds[0][1], target_bounds[1][1]] {
            for z in [target_bounds[0][2], target_bounds[1][2]] {
                let projection = dot(subtract([x, y, z], origin_mm), direction);
                minimum = minimum.min(projection);
                maximum = maximum.max(projection);
            }
        }
    }
    if !minimum.is_finite()
        || !maximum.is_finite()
        || maximum <= MIN_LENGTH_MM
        || minimum - 1.0 < -MAX_ABS_MM
        || maximum + 1.0 > MAX_ABS_MM
    {
        return Err(ExactBRepGraphError::UnresolvedExtent);
    }
    Ok([minimum, maximum])
}

fn operation_bounds(
    operation: &ExactBRepOperation,
    profiles: &[ExactBRepProfile],
    node_bounds: &[Option<[[f64; 3]; 2]>],
) -> Result<Option<[[f64; 3]; 2]>, ExactBRepGraphError> {
    match operation {
        ExactBRepOperation::Extrude {
            profile, interval, ..
        } => swept_profile_bounds(&profiles[profile.0 as usize], *interval).map(Some),
        ExactBRepOperation::ProfileCut { target, .. }
        | ExactBRepOperation::Shell { target, .. }
        | ExactBRepOperation::EdgeFinish { target, .. }
        | ExactBRepOperation::FaceOffset { target, .. } => Ok(node_bounds[target.0 as usize]),
        ExactBRepOperation::RigidTransform {
            target,
            matrix_bits,
        } => node_bounds[target.0 as usize]
            .map(|bounds| transform_bounds(bounds, matrix_bits.map(f64::from_bits)))
            .transpose(),
        ExactBRepOperation::Boolean {
            operation,
            target,
            tool,
        } => {
            let target = node_bounds[target.0 as usize];
            let tool = node_bounds[tool.0 as usize];
            Ok(match operation {
                ExactBRepBooleanOperation::Cut => target,
                ExactBRepBooleanOperation::Union => target
                    .zip(tool)
                    .map(|(target, tool)| bounds_union(target, tool)),
                ExactBRepBooleanOperation::Intersect => target
                    .zip(tool)
                    .and_then(|(target, tool)| bounds_intersection(target, tool)),
                ExactBRepBooleanOperation::Split => target
                    .zip(tool)
                    .and_then(|(target, tool)| bounds_intersection(target, tool).map(|_| target)),
            })
        }
        ExactBRepOperation::Revolve { .. }
        | ExactBRepOperation::Sweep { .. }
        | ExactBRepOperation::Loft { .. }
        | ExactBRepOperation::ImportedExact { .. } => Ok(None),
    }
}

fn transform_bounds(
    bounds: [[f64; 3]; 2],
    matrix: [f64; 16],
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    let mut transformed = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for x in [bounds[0][0], bounds[1][0]] {
        for y in [bounds[0][1], bounds[1][1]] {
            for z in [bounds[0][2], bounds[1][2]] {
                let point = [
                    matrix[0] * x + matrix[1] * y + matrix[2] * z + matrix[3],
                    matrix[4] * x + matrix[5] * y + matrix[6] * z + matrix[7],
                    matrix[8] * x + matrix[9] * y + matrix[10] * z + matrix[11],
                ];
                for axis in 0..3 {
                    transformed[0][axis] = transformed[0][axis].min(point[axis]);
                    transformed[1][axis] = transformed[1][axis].max(point[axis]);
                }
            }
        }
    }
    if valid_bounds(transformed) {
        Ok(transformed)
    } else {
        Err(ExactBRepGraphError::ResourceLimit)
    }
}

fn swept_profile_bounds(
    profile: &ExactBRepProfile,
    interval: ExactBRepLinearInterval,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    let [[min_x, min_y], [max_x, max_y]] = planar_geometry_bounds(&profile.geometry)?;
    let frame = profile.frame_bits.map(f64::from_bits);
    let origin = [frame[0], frame[1], frame[2]];
    let x_axis = [frame[3], frame[4], frame[5]];
    let y_axis = [frame[6], frame[7], frame[8]];
    let direction = interval.direction();
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for x in [min_x, max_x] {
        for y in [min_y, max_y] {
            for distance in [interval.start_mm(), interval.end_mm()] {
                let point = [0, 1, 2].map(|axis| {
                    origin[axis] + x_axis[axis] * x + y_axis[axis] * y + direction[axis] * distance
                });
                for axis in 0..3 {
                    bounds[0][axis] = bounds[0][axis].min(point[axis]);
                    bounds[1][axis] = bounds[1][axis].max(point[axis]);
                }
            }
        }
    }
    valid_bounds(bounds)
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

fn planar_geometry_bounds(
    geometry: &ExactBRepPlanarGeometry,
) -> Result<[[f64; 2]; 2], ExactBRepGraphError> {
    let mut bounds = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
    let mut include = |point: [f64; 2]| {
        for axis in 0..2 {
            bounds[0][axis] = bounds[0][axis].min(point[axis]);
            bounds[1][axis] = bounds[1][axis].max(point[axis]);
        }
    };
    match geometry {
        ExactBRepPlanarGeometry::Boundary { segments, .. } => {
            for segment in segments {
                match segment {
                    ExactBRepPlanarSegment::Line {
                        start_bits,
                        end_bits,
                    } => {
                        include(start_bits.map(f64::from_bits));
                        include(end_bits.map(f64::from_bits));
                    }
                    ExactBRepPlanarSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        ..
                    } => {
                        let start = start_bits.map(f64::from_bits);
                        let end = end_bits.map(f64::from_bits);
                        let center = center_bits.map(f64::from_bits);
                        let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                        include(start);
                        include(end);
                        include([center[0] - radius, center[1] - radius]);
                        include([center[0] + radius, center[1] + radius]);
                    }
                    ExactBRepPlanarSegment::CubicBezier {
                        start_bits,
                        control_1_bits,
                        control_2_bits,
                        end_bits,
                    } => {
                        include(start_bits.map(f64::from_bits));
                        include(control_1_bits.map(f64::from_bits));
                        include(control_2_bits.map(f64::from_bits));
                        include(end_bits.map(f64::from_bits));
                    }
                }
            }
        }
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => {
            let center = center_bits.map(f64::from_bits);
            let radius = f64::from_bits(*radius_bits);
            include([center[0] - radius, center[1] - radius]);
            include([center[0] + radius, center[1] + radius]);
        }
        ExactBRepPlanarGeometry::Spline { control_point_bits } => {
            for point in control_point_bits {
                include(point.map(f64::from_bits));
            }
        }
        ExactBRepPlanarGeometry::Region { outer, .. } => {
            let loop_bounds = planar_geometry_bounds(&loop_geometry(outer))?;
            include(loop_bounds[0]);
            include(loop_bounds[1]);
        }
    }
    if bounds.iter().flatten().all(|value| value.is_finite())
        && (0..2).all(|axis| bounds[0][axis] < bounds[1][axis])
    {
        Ok(bounds)
    } else {
        Err(ExactBRepGraphError::InvalidParameter)
    }
}

fn bounds_union(left: [[f64; 3]; 2], right: [[f64; 3]; 2]) -> [[f64; 3]; 2] {
    [
        [0, 1, 2].map(|axis| left[0][axis].min(right[0][axis])),
        [0, 1, 2].map(|axis| left[1][axis].max(right[1][axis])),
    ]
}

fn bounds_intersection(left: [[f64; 3]; 2], right: [[f64; 3]; 2]) -> Option<[[f64; 3]; 2]> {
    let bounds = [
        [0, 1, 2].map(|axis| left[0][axis].max(right[0][axis])),
        [0, 1, 2].map(|axis| left[1][axis].min(right[1][axis])),
    ];
    valid_bounds(bounds).then_some(bounds)
}

fn valid_bounds(bounds: [[f64; 3]; 2]) -> bool {
    bounds
        .iter()
        .flatten()
        .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
        && (0..3).all(|axis| bounds[0][axis] < bounds[1][axis])
}

fn canonical_bits(value: f64) -> u64 {
    if value == 0.0 {
        0.0_f64.to_bits()
    } else {
        value.to_bits()
    }
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn finite_coordinate(value: f64) -> Result<f64, ExactBRepGraphError> {
    if value.is_finite() && value.abs() <= MAX_ABS_MM {
        Ok(value)
    } else {
        Err(ExactBRepGraphError::InvalidParameter)
    }
}

fn valid_point(point: [f64; 2]) -> Result<[f64; 2], ExactBRepGraphError> {
    Ok([finite_coordinate(point[0])?, finite_coordinate(point[1])?])
}

fn positive_distance(value: f64) -> Result<u64, ExactBRepGraphError> {
    if value.is_finite() && value > MIN_LENGTH_MM && value <= MAX_ABS_MM {
        Ok(value.to_bits())
    } else {
        Err(ExactBRepGraphError::InvalidParameter)
    }
}

fn signed_distance(value: f64) -> Result<u64, ExactBRepGraphError> {
    if value.is_finite() && value.abs() > MIN_LENGTH_MM && value.abs() <= MAX_ABS_MM {
        Ok(value.to_bits())
    } else {
        Err(ExactBRepGraphError::InvalidParameter)
    }
}

fn valid_frame(bits: [u64; 12]) -> bool {
    bits.map(f64::from_bits)
        .into_iter()
        .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
}

fn loop_geometry(planar_loop: &ExactBRepPlanarLoop) -> ExactBRepPlanarGeometry {
    match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments: segments.clone(),
        },
        ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        } => ExactBRepPlanarGeometry::Circle {
            center_bits: *center_bits,
            radius_bits: *radius_bits,
        },
    }
}

fn validate_geometry(geometry: &ExactBRepPlanarGeometry) -> Result<usize, ExactBRepGraphError> {
    let valid_bits = |bits: [u64; 2]| {
        bits.map(f64::from_bits)
            .into_iter()
            .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
    };
    match geometry {
        ExactBRepPlanarGeometry::Boundary { closed, segments } => {
            if segments.len() > MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS {
                return Err(ExactBRepGraphError::ResourceLimit);
            }
            if segments.is_empty() || *closed && segments.len() < 2 {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            let endpoints = segments
                .iter()
                .map(|segment| match segment {
                    ExactBRepPlanarSegment::Line {
                        start_bits,
                        end_bits,
                    } if valid_bits(*start_bits)
                        && valid_bits(*end_bits)
                        && start_bits != end_bits =>
                    {
                        Ok((*start_bits, *end_bits))
                    }
                    ExactBRepPlanarSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        ..
                    } if valid_bits(*start_bits)
                        && valid_bits(*end_bits)
                        && valid_bits(*center_bits)
                        && start_bits != end_bits =>
                    {
                        let start = start_bits.map(f64::from_bits);
                        let end = end_bits.map(f64::from_bits);
                        let center = center_bits.map(f64::from_bits);
                        let start_radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                        let end_radius = (end[0] - center[0]).hypot(end[1] - center[1]);
                        let tolerance = start_radius.max(end_radius).max(1.0) * 1.0e-9;
                        if start_radius <= MIN_LENGTH_MM
                            || (start_radius - end_radius).abs() > tolerance
                        {
                            Err(ExactBRepGraphError::InvalidGraph)
                        } else {
                            Ok((*start_bits, *end_bits))
                        }
                    }
                    ExactBRepPlanarSegment::CubicBezier {
                        start_bits,
                        control_1_bits,
                        control_2_bits,
                        end_bits,
                    } if valid_bits(*start_bits)
                        && valid_bits(*control_1_bits)
                        && valid_bits(*control_2_bits)
                        && valid_bits(*end_bits) =>
                    {
                        let start = start_bits.map(f64::from_bits);
                        let end = end_bits.map(f64::from_bits);
                        if (start[0] - end[0]).hypot(start[1] - end[1]) <= MIN_LENGTH_MM {
                            Err(ExactBRepGraphError::InvalidGraph)
                        } else {
                            Ok((*start_bits, *end_bits))
                        }
                    }
                    _ => Err(ExactBRepGraphError::InvalidGraph),
                })
                .collect::<Result<Vec<_>, _>>()?;
            if endpoints.windows(2).any(|pair| pair[0].1 != pair[1].0)
                || (*closed && endpoints.last().unwrap().1 != endpoints[0].0)
            {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            Ok(segments.len())
        }
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => {
            let radius = f64::from_bits(*radius_bits);
            if !valid_bits(*center_bits)
                || !radius.is_finite()
                || radius <= MIN_LENGTH_MM
                || radius > MAX_ABS_MM
            {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            Ok(1)
        }
        ExactBRepPlanarGeometry::Spline { control_point_bits } => {
            if control_point_bits.len() < 4
                || control_point_bits.iter().any(|point| !valid_bits(*point))
            {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            Ok(control_point_bits.len())
        }
        ExactBRepPlanarGeometry::Region { outer, holes } => {
            if holes.is_empty() {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            if holes.len() > MAX_EXACT_BREP_REGION_HOLES {
                return Err(ExactBRepGraphError::ResourceLimit);
            }
            let segment_count = holes.iter().try_fold(
                validate_geometry(&loop_geometry(outer))?,
                |segment_count, hole| {
                    segment_count
                        .checked_add(validate_geometry(&loop_geometry(hole))?)
                        .ok_or(ExactBRepGraphError::ResourceLimit)
                },
            )?;
            if segment_count > MAX_EXACT_BREP_REGION_SEGMENTS {
                return Err(ExactBRepGraphError::ResourceLimit);
            }
            Ok(segment_count)
        }
    }
}

fn valid_linear_interval(interval: ExactBRepLinearInterval) -> bool {
    let direction = interval.direction();
    let length = direction[0].hypot(direction[1]).hypot(direction[2]);
    let start = interval.start_mm();
    let end = interval.end_mm();
    direction.iter().all(|component| component.is_finite())
        && (length - 1.0).abs() <= 1.0e-9
        && start.is_finite()
        && end.is_finite()
        && start.abs() <= MAX_ABS_MM
        && end.abs() <= MAX_ABS_MM
        && end - start > MIN_LENGTH_MM
}

fn valid_topology_selectors(
    selectors: &[ExactBRepTopologySelector],
    expected_kind: ExactBRepTopologyKind,
    document_id: u64,
    definition_id: u64,
    target: ExactBRepNodeId,
    prior_nodes: &[ExactBRepNode],
) -> bool {
    let Some(target_node) = prior_nodes.get(target.0 as usize) else {
        return false;
    };
    if selectors.is_empty() || selectors.len() > MAX_EXACT_BREP_TOPOLOGY_SELECTORS {
        return false;
    }
    let references = selectors
        .iter()
        .map(|selector| {
            (selector.kind == expected_kind)
                .then(|| selector.reference().ok())
                .flatten()
        })
        .collect::<Option<Vec<_>>>();
    references.is_some_and(|references| {
        references.iter().all(|reference| {
            reference.document_id.0 == document_id
                && reference.definition_id.0 == definition_id
                && reference.producer_feature_id.0 == target_node.source_feature_id
        }) && references.windows(2).all(|pair| pair[0] < pair[1])
    })
}

fn valid_operation_profiles(operation: &ExactBRepOperation, profiles: &[ExactBRepProfile]) -> bool {
    match operation {
        ExactBRepOperation::Loft { sections } => sections.iter().all(|section| {
            matches!(
                profiles
                    .get(section.profile.0 as usize)
                    .map(|profile| &profile.geometry),
                Some(ExactBRepPlanarGeometry::Spline { control_point_bits })
                    if (4..=MAX_EXACT_BREP_LOFT_CONTROL_POINTS)
                        .contains(&control_point_bits.len())
            )
        }),
        ExactBRepOperation::Sweep { profile, path } => {
            profile != path
                && matches!(
                    profiles
                        .get(profile.0 as usize)
                        .map(|profile| &profile.geometry),
                    Some(
                        ExactBRepPlanarGeometry::Boundary { closed: true, .. }
                            | ExactBRepPlanarGeometry::Circle { .. }
                            | ExactBRepPlanarGeometry::Region { .. }
                    )
                )
                && matches!(
                    profiles.get(path.0 as usize).map(|profile| &profile.geometry),
                    Some(ExactBRepPlanarGeometry::Boundary {
                        closed: false,
                        segments,
                    }) if matches!(segments.as_slice(), [ExactBRepPlanarSegment::Line { .. }])
                )
        }
        _ => true,
    }
}

fn valid_operation(
    operation: &ExactBRepOperation,
    document_id: u64,
    definition_id: u64,
    prior_nodes: &[ExactBRepNode],
) -> bool {
    let positive = |bits| {
        let value = f64::from_bits(bits);
        value.is_finite() && value > MIN_LENGTH_MM && value <= MAX_ABS_MM
    };
    let signed = |bits| {
        let value = f64::from_bits(bits);
        value.is_finite() && value.abs() > MIN_LENGTH_MM && value.abs() <= MAX_ABS_MM
    };
    match operation {
        ExactBRepOperation::Extrude {
            distance_bits,
            interval,
            ..
        } => {
            valid_linear_interval(*interval)
                && positive(*distance_bits)
                && *distance_bits == interval.length_mm().to_bits()
        }
        ExactBRepOperation::ProfileCut {
            depth_bits,
            interval,
            support_lineage_digest,
            ..
        } => {
            valid_linear_interval(*interval)
                && depth_bits
                    .is_none_or(|depth| positive(depth) && depth == interval.length_mm().to_bits())
                && support_lineage_digest
                    .as_ref()
                    .is_none_or(|digest| !digest.is_empty() && digest.len() <= 256)
        }
        ExactBRepOperation::Boolean { target, tool, .. } => target != tool,
        ExactBRepOperation::RigidTransform { matrix_bits, .. } => {
            let matrix = matrix_bits.map(f64::from_bits);
            Transform::from_matrix(matrix)
                .ok()
                .and_then(Transform::rigid_inverse)
                .is_some()
                && [matrix[3], matrix[7], matrix[11]]
                    .into_iter()
                    .all(|value| value.abs() <= MAX_ABS_MM)
        }
        ExactBRepOperation::Shell {
            target,
            removed_faces,
            thickness_bits,
        } => {
            positive(*thickness_bits)
                && valid_topology_selectors(
                    removed_faces,
                    ExactBRepTopologyKind::Face,
                    document_id,
                    definition_id,
                    *target,
                    prior_nodes,
                )
        }
        ExactBRepOperation::EdgeFinish {
            target,
            edges,
            amount_bits,
            ..
        } => {
            positive(*amount_bits)
                && valid_topology_selectors(
                    edges,
                    ExactBRepTopologyKind::Edge,
                    document_id,
                    definition_id,
                    *target,
                    prior_nodes,
                )
        }
        ExactBRepOperation::FaceOffset {
            target,
            face,
            distance_bits,
        } => {
            signed(*distance_bits)
                && valid_topology_selectors(
                    std::slice::from_ref(face),
                    ExactBRepTopologyKind::Face,
                    document_id,
                    definition_id,
                    *target,
                    prior_nodes,
                )
        }
        ExactBRepOperation::Revolve {
            axis_start_bits,
            axis_end_bits,
            angle_degrees_bits,
            ..
        } => {
            let start = axis_start_bits.map(f64::from_bits);
            let end = axis_end_bits.map(f64::from_bits);
            let angle = f64::from_bits(*angle_degrees_bits);
            start
                .into_iter()
                .chain(end)
                .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
                && start != end
                && angle.is_finite()
                && angle > 0.0
                && angle <= 360.0
        }
        ExactBRepOperation::Sweep { profile, path } => profile != path,
        ExactBRepOperation::Loft { sections } => {
            (2..=MAX_EXACT_BREP_LOFT_SECTIONS).contains(&sections.len())
                && sections.windows(2).all(|pair| {
                    let lower = f64::from_bits(pair[0].elevation_bits);
                    let upper = f64::from_bits(pair[1].elevation_bits);
                    lower.is_finite()
                        && upper.is_finite()
                        && lower.abs() <= MAX_ABS_MM
                        && upper.abs() <= MAX_ABS_MM
                        && lower < upper
                        && pair[0].profile != pair[1].profile
                })
        }
        ExactBRepOperation::ImportedExact {
            source_sha256,
            source_byte_len,
            result_fingerprint,
        } => {
            source_sha256.iter().any(|byte| *byte != 0)
                && (1..=32 * 1024 * 1024).contains(source_byte_len)
                && !result_fingerprint.is_empty()
                && result_fingerprint.len() <= 128
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactBRepGraphError {
    DefinitionNotFound(DefinitionId),
    FeatureNotFound(FeatureId),
    UnsupportedFeature(FeatureId),
    UnsupportedProfile(FeatureId),
    SuppressedFeature(FeatureId),
    DependencyCycle(FeatureId),
    InvalidDependencyGraph,
    InvalidParameter,
    InvalidTopologySelector,
    UnresolvedExtent,
    AmbiguousExtent,
    InvalidGraph,
    DigestMismatch,
    ResourceLimit,
    Serialization(String),
}

impl fmt::Display for ExactBRepGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefinitionNotFound(id) => write!(formatter, "definition {} was not found", id.0),
            Self::FeatureNotFound(id) => write!(formatter, "feature {} was not found", id.0),
            Self::UnsupportedFeature(id) => {
                write!(
                    formatter,
                    "feature {} is not an exact B-Rep graph operation",
                    id.0
                )
            }
            Self::UnsupportedProfile(id) => {
                write!(
                    formatter,
                    "feature {} is not a supported planar profile",
                    id.0
                )
            }
            Self::SuppressedFeature(id) => write!(formatter, "feature {} is suppressed", id.0),
            Self::DependencyCycle(id) => {
                write!(formatter, "feature {} closes a dependency cycle", id.0)
            }
            Self::InvalidDependencyGraph => {
                formatter.write_str("canonical feature dependency graph is invalid")
            }
            Self::InvalidParameter => formatter.write_str("exact B-Rep graph parameter is invalid"),
            Self::InvalidTopologySelector => {
                formatter.write_str("exact B-Rep graph topology selector is invalid")
            }
            Self::UnresolvedExtent => {
                formatter.write_str("exact feature extent target is missing, stale, or unreachable")
            }
            Self::AmbiguousExtent => {
                formatter.write_str("exact feature extent target does not define one intersection")
            }
            Self::InvalidGraph => formatter.write_str("exact B-Rep graph structure is invalid"),
            Self::DigestMismatch => {
                formatter.write_str("exact B-Rep graph digest does not match its payload")
            }
            Self::ResourceLimit => {
                formatter.write_str("exact B-Rep graph exceeds a resource limit")
            }
            Self::Serialization(error) => {
                write!(formatter, "exact B-Rep graph serialization failed: {error}")
            }
        }
    }
}

impl std::error::Error for ExactBRepGraphError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn loft_operation(section_count: usize) -> ExactBRepOperation {
        ExactBRepOperation::Loft {
            sections: (0..section_count)
                .map(|index| ExactBRepLoftSection {
                    profile: ExactBRepProfileId(index as u32 + 1),
                    elevation_bits: (index as f64).to_bits(),
                })
                .collect(),
        }
    }

    #[test]
    fn loft_operation_limit_matches_the_exact_backend() {
        assert_eq!(MAX_EXACT_BREP_LOFT_SECTIONS, 16);
        assert!(valid_operation(
            &loft_operation(MAX_EXACT_BREP_LOFT_SECTIONS),
            1,
            1,
            &[],
        ));
        assert!(!valid_operation(
            &loft_operation(MAX_EXACT_BREP_LOFT_SECTIONS + 1),
            1,
            1,
            &[],
        ));
    }

    #[test]
    fn loft_control_point_limit_matches_the_exact_backend() {
        let profile = |id, point_count| ExactBRepProfile {
            id: ExactBRepProfileId(id),
            source_feature_id: id as u64 + 1,
            region_id: None,
            frame_bits: identity_frame(),
            geometry: ExactBRepPlanarGeometry::Spline {
                control_point_bits: (0..point_count)
                    .map(|index| [index as f64, (index % 2) as f64].map(f64::to_bits))
                    .collect(),
            },
        };
        let operation = ExactBRepOperation::Loft {
            sections: vec![
                ExactBRepLoftSection {
                    profile: ExactBRepProfileId(0),
                    elevation_bits: 0.0_f64.to_bits(),
                },
                ExactBRepLoftSection {
                    profile: ExactBRepProfileId(1),
                    elevation_bits: 10.0_f64.to_bits(),
                },
            ],
        };
        let profiles = vec![
            profile(0, MAX_EXACT_BREP_LOFT_CONTROL_POINTS),
            profile(1, MAX_EXACT_BREP_LOFT_CONTROL_POINTS),
        ];
        let mut graph = ExactBRepGraph {
            schema: EXACT_BREP_GRAPH_SCHEMA_V4.to_owned(),
            document_id: 1,
            source_revision: 1,
            source_digest: "source".to_owned(),
            definition_id: 1,
            producer_feature_id: 3,
            profiles,
            nodes: vec![ExactBRepNode {
                id: ExactBRepNodeId(0),
                source_feature_id: 3,
                operation,
            }],
            graph_digest: String::new(),
            canonical_input_digest: String::new(),
        };
        graph.graph_digest = graph.compute_graph_digest().unwrap();
        graph.canonical_input_digest = graph.compute_canonical_input_digest();
        let boundary_bytes = graph.to_bytes().unwrap();
        assert_eq!(ExactBRepGraph::from_bytes(&boundary_bytes).unwrap(), graph);

        graph.profiles[1] = profile(1, MAX_EXACT_BREP_LOFT_CONTROL_POINTS + 1);
        let over_limit_bytes = serde_json::to_vec(&graph).unwrap();
        assert_eq!(
            ExactBRepGraph::from_bytes(&over_limit_bytes),
            Err(ExactBRepGraphError::InvalidGraph)
        );
    }

    fn circle_loop(center: [f64; 2], radius: f64) -> ExactBRepPlanarLoop {
        ExactBRepPlanarLoop::Circle {
            center_bits: center.map(f64::to_bits),
            radius_bits: radius.to_bits(),
        }
    }

    fn boundary_loop(center: [f64; 2], radius: f64, segment_count: usize) -> ExactBRepPlanarLoop {
        let points = (0..segment_count)
            .map(|index| {
                let angle = std::f64::consts::TAU * index as f64 / segment_count as f64;
                [
                    (center[0] + radius * angle.cos()).to_bits(),
                    (center[1] + radius * angle.sin()).to_bits(),
                ]
            })
            .collect::<Vec<_>>();
        ExactBRepPlanarLoop::Boundary {
            segments: (0..segment_count)
                .map(|index| ExactBRepPlanarSegment::Line {
                    start_bits: points[index],
                    end_bits: points[(index + 1) % segment_count],
                })
                .collect(),
        }
    }

    #[test]
    fn planar_region_resource_limits_are_enforced() {
        assert_eq!(MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS, 64);
        assert_eq!(MAX_EXACT_BREP_REGION_HOLES, 64);
        assert_eq!(MAX_EXACT_BREP_REGION_SEGMENTS, 4_096);

        let hole_center = |index: usize| {
            [
                (index % 9) as f64 * 10.0 - 40.0,
                (index / 9) as f64 * 10.0 - 40.0,
            ]
        };
        let geometry_with_holes = |hole_count| ExactBRepPlanarGeometry::Region {
            outer: circle_loop([0.0, 0.0], 100.0),
            holes: (0..hole_count)
                .map(|index| circle_loop(hole_center(index), 2.0))
                .collect(),
        };
        assert_eq!(
            validate_geometry(&geometry_with_holes(MAX_EXACT_BREP_REGION_HOLES)),
            Ok(MAX_EXACT_BREP_REGION_HOLES + 1),
        );
        assert_eq!(
            validate_geometry(&geometry_with_holes(MAX_EXACT_BREP_REGION_HOLES + 1)),
            Err(ExactBRepGraphError::ResourceLimit),
        );

        assert_eq!(
            validate_geometry(&loop_geometry(&boundary_loop(
                [0.0, 0.0],
                100.0,
                MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS + 1,
            ))),
            Err(ExactBRepGraphError::ResourceLimit),
        );

        let outer = || boundary_loop([0.0, 0.0], 100.0, MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS);
        let boundary_holes = || {
            (0..63)
                .map(|index| {
                    let center = [
                        (index % 9) as f64 * 10.0 - 40.0,
                        (index / 9) as f64 * 10.0 - 30.0,
                    ];
                    boundary_loop(center, 2.0, MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS)
                })
                .collect::<Vec<_>>()
        };
        let accepted = ExactBRepPlanarGeometry::Region {
            outer: outer(),
            holes: boundary_holes(),
        };
        assert_eq!(
            validate_geometry(&accepted),
            Ok(MAX_EXACT_BREP_REGION_SEGMENTS),
        );
        let mut over_limit_holes = boundary_holes();
        over_limit_holes.push(circle_loop([0.0, 45.0], 2.0));
        let over_limit = ExactBRepPlanarGeometry::Region {
            outer: outer(),
            holes: over_limit_holes,
        };
        assert_eq!(
            validate_geometry(&over_limit),
            Err(ExactBRepGraphError::ResourceLimit),
        );
    }

    #[test]
    fn sweep_path_contract_matches_the_current_worker() {
        let profile = |id, geometry| ExactBRepProfile {
            id: ExactBRepProfileId(id),
            source_feature_id: id as u64 + 1,
            region_id: None,
            frame_bits: identity_frame(),
            geometry,
        };
        let line = |start: [f64; 2], end: [f64; 2]| ExactBRepPlanarSegment::Line {
            start_bits: start.map(f64::to_bits),
            end_bits: end.map(f64::to_bits),
        };
        let mut profiles = vec![
            profile(
                0,
                ExactBRepPlanarGeometry::Circle {
                    center_bits: [0.0f64.to_bits(), 0.0f64.to_bits()],
                    radius_bits: 2.0f64.to_bits(),
                },
            ),
            profile(
                1,
                ExactBRepPlanarGeometry::Boundary {
                    closed: false,
                    segments: vec![line([0.0, 0.0], [10.0, 0.0])],
                },
            ),
        ];
        let sweep = ExactBRepOperation::Sweep {
            profile: ExactBRepProfileId(0),
            path: ExactBRepProfileId(1),
        };
        assert!(valid_operation_profiles(&sweep, &profiles));

        profiles[0].geometry = ExactBRepPlanarGeometry::Spline {
            control_point_bits: vec![
                [0.0f64.to_bits(), 0.0f64.to_bits()],
                [1.0f64.to_bits(), 0.0f64.to_bits()],
                [1.0f64.to_bits(), 1.0f64.to_bits()],
                [0.0f64.to_bits(), 1.0f64.to_bits()],
            ],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles));
        profiles[0].geometry = ExactBRepPlanarGeometry::Circle {
            center_bits: [0.0f64.to_bits(), 0.0f64.to_bits()],
            radius_bits: 2.0f64.to_bits(),
        };

        profiles[1].geometry = ExactBRepPlanarGeometry::Circle {
            center_bits: [0.0f64.to_bits(), 0.0f64.to_bits()],
            radius_bits: 10.0f64.to_bits(),
        };
        assert!(!valid_operation_profiles(&sweep, &profiles));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![line([0.0, 0.0], [5.0, 0.0]), line([5.0, 0.0], [10.0, 0.0])],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles));
    }
}
