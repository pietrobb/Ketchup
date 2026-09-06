use crate::document::{
    BooleanOperation, DefinitionId, EdgeFinishKind, FeatureDependencyGraph, FeatureId, FeatureKind,
    LoftSection, ProfileSegment, Snapshot, SpatialPathSegment, Transform,
};
use crate::exact_product::{
    EXACT_MIN_LENGTH_MM, ExactCircleProfile, ExactPlanarOffsetRegion,
    MAX_EXACT_PLANAR_OFFSET_LENGTH_MM, accepts_planar_circle_offset_geometry,
    accepts_planar_offset_geometry, exact_planar_offset_profile_from_segments,
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

pub const EXACT_BREP_GRAPH_SCHEMA_V6: &str = "ketchup.exact-brep-graph.v6";
pub const EXACT_BREP_GRAPH_SCHEMA_V7: &str = "ketchup.exact-brep-graph.v7";
pub const EXACT_BREP_GRAPH_SCHEMA_V8: &str = "ketchup.exact-brep-graph.v8";
pub const EXACT_BREP_GRAPH_SCHEMA_V9: &str = "ketchup.exact-brep-graph.v9";
pub const EXACT_BREP_GRAPH_SCHEMA_V10: &str = "ketchup.exact-brep-graph.v10";
pub const EXACT_BREP_GRAPH_SCHEMA_V11: &str = "ketchup.exact-brep-graph.v11";
pub const EXACT_BREP_GRAPH_SCHEMA_V12: &str = "ketchup.exact-brep-graph.v12";
pub const MAX_EXACT_BREP_GRAPH_PROFILES: usize = 1_024;
pub const MAX_EXACT_BREP_GRAPH_NODES: usize = 1_024;
pub const MAX_EXACT_BREP_GRAPH_SEGMENTS: usize = 16_384;
pub const MAX_EXACT_BREP_LOFT_SECTIONS: usize = 16;
pub const MAX_EXACT_BREP_LOFT_CONTROL_POINTS: usize = 64;
pub const MIN_EXACT_BREP_SWEEP_PATH_SEGMENT_LENGTH_MM: f64 = 1.0e-7;
pub const MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM: f64 = 0.01;
pub const MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM: f64 = 100_000.0;
pub const MAX_EXACT_BREP_SWEEP_PATH_SEGMENTS: usize = 64;
pub const MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS: usize = 64;
pub const MAX_EXACT_BREP_REGION_HOLES: usize = 64;
pub const MAX_EXACT_BREP_REGION_SEGMENTS: usize = 4_096;
pub const MAX_EXACT_BREP_GRAPH_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EXACT_BREP_TOPOLOGY_SELECTORS: usize = 64;
pub const MAX_EXACT_BREP_COORDINATE_MM: f64 = 1_000_000.0;
const MAX_ABS_MM: f64 = MAX_EXACT_BREP_COORDINATE_MM;
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
pub enum ExactBRepSpatialPathSegment {
    Line {
        start_bits: [u64; 3],
        end_bits: [u64; 3],
    },
    CircularArc {
        start_bits: [u64; 3],
        end_bits: [u64; 3],
        center_bits: [u64; 3],
        normal_bits: [u64; 3],
        clockwise: bool,
    },
    CubicBezier {
        start_bits: [u64; 3],
        control_1_bits: [u64; 3],
        control_2_bits: [u64; 3],
        end_bits: [u64; 3],
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepSpatialPath {
    pub source_feature_id: u64,
    pub segments: Vec<ExactBRepSpatialPathSegment>,
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
    PlanarOffset {
        profile: ExactBRepProfileId,
        distance_bits: u64,
    },
    Sweep {
        profile: ExactBRepProfileId,
        path: ExactBRepProfileId,
    },
    SpatialSweep {
        profile: ExactBRepProfileId,
        path: ExactBRepSpatialPath,
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
            | Self::PlanarOffset { .. }
            | Self::Sweep { .. }
            | Self::SpatialSweep { .. }
            | Self::Loft { .. }
            | Self::ImportedExact { .. } => Vec::new(),
        }
    }

    fn profile_ids(&self) -> Vec<ExactBRepProfileId> {
        match self {
            Self::Extrude { profile, .. }
            | Self::ProfileCut { profile, .. }
            | Self::Revolve { profile, .. }
            | Self::PlanarOffset { profile, .. } => vec![*profile],
            Self::Sweep { profile, path } => vec![*profile, *path],
            Self::SpatialSweep { profile, .. } => vec![*profile],
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
        let dependencies = snapshot
            .feature_dependency_graph()
            .map_err(|_| ExactBRepGraphError::InvalidDependencyGraph)?;
        Self::from_snapshot_with_dependencies(
            snapshot,
            definition_id,
            producer_feature_id,
            &dependencies,
        )
    }

    pub(crate) fn from_snapshot_with_dependencies(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
        _dependencies: &FeatureDependencyGraph,
    ) -> Result<Self, ExactBRepGraphError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactBRepGraphError::DefinitionNotFound(definition_id))?;
        if !definition.feature_ids().contains(&producer_feature_id) {
            return Err(ExactBRepGraphError::FeatureNotFound(producer_feature_id));
        }
        let mut compiler = GraphCompiler::new(snapshot, definition_id);
        compiler.compile_body(producer_feature_id)?;
        let source_digest = snapshot.canonical_digest();
        let schema = if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v12(&node.operation))
        {
            EXACT_BREP_GRAPH_SCHEMA_V12
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v11(&node.operation, &compiler.profiles))
        {
            EXACT_BREP_GRAPH_SCHEMA_V11
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v10(&node.operation, &compiler.profiles))
        {
            EXACT_BREP_GRAPH_SCHEMA_V10
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v9(&node.operation, &compiler.profiles))
        {
            EXACT_BREP_GRAPH_SCHEMA_V9
        } else {
            EXACT_BREP_GRAPH_SCHEMA_V8
        };
        let mut graph = Self {
            schema: schema.to_owned(),
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

    #[must_use]
    pub fn terminal_is_planar_offset(&self) -> bool {
        matches!(
            self.nodes.last().map(|node| &node.operation),
            Some(ExactBRepOperation::PlanarOffset { .. })
        )
    }

    #[must_use]
    pub fn accepts_terminal_planar_offset_geometry(
        &self,
        bounds_mm: [[f64; 3]; 2],
        area_mm2: f64,
        topology_counts: [u32; 5],
        wire_count: Option<u32>,
    ) -> bool {
        if self.validate().is_err() {
            return false;
        }
        let Some(ExactBRepOperation::PlanarOffset {
            profile,
            distance_bits,
        }) = self.nodes.last().map(|node| &node.operation)
        else {
            return false;
        };
        let Some(profile) = self.profiles.get(profile.0 as usize) else {
            return false;
        };
        if profile.frame_bits != identity_frame() {
            return false;
        }
        let distance_mm = f64::from_bits(*distance_bits);
        match &profile.geometry {
            ExactBRepPlanarGeometry::Circle {
                center_bits,
                radius_bits,
            } => accepts_planar_circle_offset_geometry(
                ExactCircleProfile {
                    center_x_bits: center_bits[0],
                    center_y_bits: center_bits[1],
                    radius_bits: *radius_bits,
                    clockwise: false,
                },
                distance_mm,
                bounds_mm,
                area_mm2,
                topology_counts,
            ),
            ExactBRepPlanarGeometry::Boundary {
                closed: true,
                segments,
            } => exact_planar_offset_profile_from_segments(segments.clone()).is_some_and(|exact| {
                accepts_planar_offset_geometry(
                    &exact,
                    distance_mm,
                    bounds_mm,
                    area_mm2,
                    topology_counts,
                )
            }),
            ExactBRepPlanarGeometry::Region { outer, holes } => {
                if wire_count != u32::try_from(holes.len() + 1).ok() {
                    return false;
                }
                let region = ExactPlanarOffsetRegion {
                    outer: outer.clone(),
                    holes: holes.clone(),
                };
                accepts_planar_offset_geometry(
                    &region,
                    distance_mm,
                    bounds_mm,
                    area_mm2,
                    topology_counts,
                )
            }
            _ => false,
        }
    }

    pub fn producer_bounds_mm(&self) -> Result<Option<[[f64; 3]; 2]>, ExactBRepGraphError> {
        let terminal_index = self
            .nodes
            .len()
            .checked_sub(1)
            .ok_or(ExactBRepGraphError::InvalidGraph)?;
        self.node_bounds_mm(ExactBRepNodeId(terminal_index as u32))
    }

    pub fn node_bounds_mm(
        &self,
        node_id: ExactBRepNodeId,
    ) -> Result<Option<[[f64; 3]; 2]>, ExactBRepGraphError> {
        self.validate()?;
        let target_index = node_id.0 as usize;
        if target_index >= self.nodes.len() {
            return Err(ExactBRepGraphError::InvalidGraph);
        }
        let mut node_bounds = Vec::with_capacity(target_index + 1);
        for node in self.nodes.iter().take(target_index + 1) {
            node_bounds.push(operation_bounds(
                &node.operation,
                &self.profiles,
                &node_bounds,
            )?);
        }
        Ok(node_bounds[target_index])
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
        if !matches!(
            self.schema.as_str(),
            EXACT_BREP_GRAPH_SCHEMA_V6
                | EXACT_BREP_GRAPH_SCHEMA_V7
                | EXACT_BREP_GRAPH_SCHEMA_V8
                | EXACT_BREP_GRAPH_SCHEMA_V9
                | EXACT_BREP_GRAPH_SCHEMA_V10
                | EXACT_BREP_GRAPH_SCHEMA_V11
                | EXACT_BREP_GRAPH_SCHEMA_V12
        ) || self.document_id == 0
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
            if let ExactBRepOperation::SpatialSweep { path, .. } = &node.operation {
                segment_count = segment_count
                    .checked_add(path.segments.len())
                    .ok_or(ExactBRepGraphError::ResourceLimit)?;
                if segment_count > MAX_EXACT_BREP_GRAPH_SEGMENTS {
                    return Err(ExactBRepGraphError::ResourceLimit);
                }
            }
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
                || (self.schema == EXACT_BREP_GRAPH_SCHEMA_V6
                    && operation_requires_v7(&node.operation, &self.profiles))
                || (matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V6 | EXACT_BREP_GRAPH_SCHEMA_V7
                ) && operation_requires_v8(&node.operation, &self.profiles))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V9
                        | EXACT_BREP_GRAPH_SCHEMA_V10
                        | EXACT_BREP_GRAPH_SCHEMA_V11
                        | EXACT_BREP_GRAPH_SCHEMA_V12
                ) && operation_requires_v9(&node.operation, &self.profiles))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V10
                        | EXACT_BREP_GRAPH_SCHEMA_V11
                        | EXACT_BREP_GRAPH_SCHEMA_V12
                ) && operation_requires_v10(&node.operation, &self.profiles))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V11 | EXACT_BREP_GRAPH_SCHEMA_V12
                ) && operation_requires_v11(&node.operation, &self.profiles))
                || (self.schema != EXACT_BREP_GRAPH_SCHEMA_V12
                    && operation_requires_v12(&node.operation))
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
            } => {
                let target = self.compile_body(*target)?;
                let (profile, direction) = match self
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
                        let (profile, _, direction) = self.compile_sketch_profile(
                            *profile,
                            region.id,
                            FeatureDirection::AlongNormal,
                        )?;
                        (profile, direction)
                    }
                    _ => (
                        self.compile_profile(*profile, None, identity_frame())?,
                        [0.0, 0.0, 1.0],
                    ),
                };
                ExactBRepOperation::ProfileCut {
                    target,
                    profile,
                    depth_bits: Some(positive_distance(depth.millimetres())?),
                    interval: linear_interval(direction, 0.0, depth.millimetres())?,
                    support_lineage_digest: None,
                }
            }
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
            FeatureKind::PlanarOffset { profile, distance } => {
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
                ExactBRepOperation::PlanarOffset {
                    profile: compiled_profile,
                    distance_bits: planar_offset_distance(distance.millimetres())?,
                }
            }
            FeatureKind::Sweep { profile, path } => {
                if let Some(FeatureKind::SpatialPath { segments }) =
                    self.snapshot.feature(*path).map(|feature| feature.kind())
                {
                    ExactBRepOperation::SpatialSweep {
                        profile: self.compile_profile(*profile, None, identity_frame())?,
                        path: spatial_path(*path, segments)?,
                    }
                } else {
                    ExactBRepOperation::Sweep {
                        profile: self.compile_profile(*profile, None, identity_frame())?,
                        path: self.compile_profile(*path, None, identity_frame())?,
                    }
                }
            }
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
            ProfileSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => cubic_bezier_segment(*start_mm, *control_1_mm, *control_2_mm, *end_mm),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExactBRepPlanarGeometry::Boundary { closed, segments })
}

fn spatial_path(
    source_feature_id: FeatureId,
    segments: &[SpatialPathSegment],
) -> Result<ExactBRepSpatialPath, ExactBRepGraphError> {
    let path = ExactBRepSpatialPath {
        source_feature_id: source_feature_id.0,
        segments: segments
            .iter()
            .map(|segment| match segment {
                SpatialPathSegment::Line { start_mm, end_mm } => {
                    ExactBRepSpatialPathSegment::Line {
                        start_bits: start_mm.map(canonical_bits),
                        end_bits: end_mm.map(canonical_bits),
                    }
                }
                SpatialPathSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    normal,
                    clockwise,
                } => ExactBRepSpatialPathSegment::CircularArc {
                    start_bits: start_mm.map(canonical_bits),
                    end_bits: end_mm.map(canonical_bits),
                    center_bits: center_mm.map(canonical_bits),
                    normal_bits: normal.map(canonical_bits),
                    clockwise: *clockwise,
                },
                SpatialPathSegment::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => ExactBRepSpatialPathSegment::CubicBezier {
                    start_bits: start_mm.map(canonical_bits),
                    control_1_bits: control_1_mm.map(canonical_bits),
                    control_2_bits: control_2_mm.map(canonical_bits),
                    end_bits: end_mm.map(canonical_bits),
                },
            })
            .collect(),
    };
    valid_spatial_path(&path)
        .then_some(path)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

fn valid_spatial_path(path: &ExactBRepSpatialPath) -> bool {
    if path.source_feature_id == 0 {
        return false;
    }
    let segments = path
        .segments
        .iter()
        .map(|segment| match segment {
            ExactBRepSpatialPathSegment::Line {
                start_bits,
                end_bits,
            } => SpatialPathSegment::Line {
                start_mm: start_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
            },
            ExactBRepSpatialPathSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                normal_bits,
                clockwise,
            } => SpatialPathSegment::CircularArc {
                start_mm: start_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
                center_mm: center_bits.map(f64::from_bits),
                normal: normal_bits.map(f64::from_bits),
                clockwise: *clockwise,
            },
            ExactBRepSpatialPathSegment::CubicBezier {
                start_bits,
                control_1_bits,
                control_2_bits,
                end_bits,
            } => SpatialPathSegment::CubicBezier {
                start_mm: start_bits.map(f64::from_bits),
                control_1_mm: control_1_bits.map(f64::from_bits),
                control_2_mm: control_2_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
            },
        })
        .collect::<Vec<_>>();
    crate::document::is_valid_spatial_sweep_path(&segments)
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
        ExactBRepOperation::PlanarOffset {
            profile,
            distance_bits,
        } => planar_offset_profile_bounds(
            &profiles[profile.0 as usize],
            f64::from_bits(*distance_bits),
        )
        .map(Some),
        ExactBRepOperation::Sweep { profile, path } => {
            sweep_profile_bounds(&profiles[profile.0 as usize], &profiles[path.0 as usize])
                .map(Some)
        }
        ExactBRepOperation::SpatialSweep { profile, path } => {
            spatial_sweep_bounds(&profiles[profile.0 as usize], path).map(Some)
        }
        ExactBRepOperation::Revolve { .. }
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

pub fn exact_brep_planar_rectangle_bounds(profile: &ExactBRepProfile) -> Option<[f64; 4]> {
    if profile.frame_bits != identity_frame() {
        return None;
    }
    let ExactBRepPlanarGeometry::Boundary {
        closed: true,
        segments,
    } = &profile.geometry
    else {
        return None;
    };
    if segments.len() != 4 {
        return None;
    }
    let mut points = Vec::with_capacity(4);
    for segment in segments {
        let ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } = segment
        else {
            return None;
        };
        let start = start_bits.map(f64::from_bits);
        let end = end_bits.map(f64::from_bits);
        let delta = [end[0] - start[0], end[1] - start[1]];
        if start.into_iter().chain(end).any(|value| !value.is_finite())
            || (delta[0] != 0.0 && delta[1] != 0.0)
        {
            return None;
        }
        points.push(start);
    }
    let min_x = points.iter().map(|point| point[0]).reduce(f64::min)?;
    let min_y = points.iter().map(|point| point[1]).reduce(f64::min)?;
    let max_x = points.iter().map(|point| point[0]).reduce(f64::max)?;
    let max_y = points.iter().map(|point| point[1]).reduce(f64::max)?;
    let corners = [
        [min_x, min_y],
        [min_x, max_y],
        [max_x, min_y],
        [max_x, max_y],
    ];
    (min_x < max_x
        && min_y < max_y
        && corners
            .iter()
            .all(|corner| points.iter().any(|point| point == corner)))
    .then_some([min_x, min_y, max_x, max_y])
}

fn planar_offset_profile_bounds(
    profile: &ExactBRepProfile,
    distance_mm: f64,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    if profile.frame_bits != identity_frame()
        || !distance_mm.is_finite()
        || !(EXACT_MIN_LENGTH_MM..=MAX_ABS_MM).contains(&distance_mm.abs())
    {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    let bounds = match &profile.geometry {
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => {
            let center = center_bits.map(f64::from_bits);
            let radius = f64::from_bits(*radius_bits);
            let output_radius = radius + distance_mm;
            if distance_mm.abs() > MAX_EXACT_PLANAR_OFFSET_LENGTH_MM
                || !(EXACT_MIN_LENGTH_MM..=MAX_EXACT_PLANAR_OFFSET_LENGTH_MM).contains(&radius)
                || !(EXACT_MIN_LENGTH_MM..=MAX_EXACT_PLANAR_OFFSET_LENGTH_MM)
                    .contains(&output_radius)
                || [
                    center[0] - radius,
                    center[1] - radius,
                    center[0] + radius,
                    center[1] + radius,
                ]
                .into_iter()
                .any(|value| !value.is_finite() || value.abs() > MAX_ABS_MM)
            {
                return Err(ExactBRepGraphError::InvalidParameter);
            }
            [
                [center[0] - output_radius, center[1] - output_radius, 0.0],
                [center[0] + output_radius, center[1] + output_radius, 0.0],
            ]
        }
        ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments,
        } => {
            if let Some([min_x, min_y, max_x, max_y]) = exact_brep_planar_rectangle_bounds(profile)
            {
                let bounds = [
                    [min_x - distance_mm, min_y - distance_mm, 0.0],
                    [max_x + distance_mm, max_y + distance_mm, 0.0],
                ];
                if bounds[1][0] - bounds[0][0] < EXACT_MIN_LENGTH_MM
                    || bounds[1][1] - bounds[0][1] < EXACT_MIN_LENGTH_MM
                {
                    return Err(ExactBRepGraphError::InvalidParameter);
                }
                bounds
            } else {
                if distance_mm.abs() > MAX_EXACT_PLANAR_OFFSET_LENGTH_MM {
                    return Err(ExactBRepGraphError::InvalidParameter);
                }
                let exact = exact_planar_offset_profile_from_segments(segments.clone())
                    .ok_or(ExactBRepGraphError::InvalidParameter)?;
                let [min_x, min_y, max_x, max_y] = exact.bounds_bits.map(f64::from_bits);
                if distance_mm < 0.0
                    && (max_x - min_x <= 2.0 * distance_mm.abs()
                        || max_y - min_y <= 2.0 * distance_mm.abs())
                {
                    return Err(ExactBRepGraphError::InvalidParameter);
                }
                let margin = exact
                    .max_planar_offset_displacement_mm(distance_mm)
                    .ok_or(ExactBRepGraphError::InvalidParameter)?;
                if distance_mm > 0.0 {
                    [
                        [min_x - margin, min_y - margin, 0.0],
                        [max_x + margin, max_y + margin, 0.0],
                    ]
                } else {
                    [[min_x, min_y, 0.0], [max_x, max_y, 0.0]]
                }
            }
        }
        ExactBRepPlanarGeometry::Region { outer, holes } => {
            if !(ExactPlanarOffsetRegion {
                outer: outer.clone(),
                holes: holes.clone(),
            })
            .has_valid_encoding(distance_mm)
            {
                return Err(ExactBRepGraphError::InvalidParameter);
            }
            let mut loop_profile = profile.clone();
            loop_profile.geometry = loop_geometry(outer);
            let bounds = planar_offset_profile_bounds(&loop_profile, distance_mm)?;
            for hole in holes {
                loop_profile.geometry = loop_geometry(hole);
                planar_offset_profile_bounds(&loop_profile, -distance_mm)?;
            }
            bounds
        }
        _ => return Err(ExactBRepGraphError::InvalidParameter),
    };
    bounds
        .iter()
        .flatten()
        .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

fn sweep_path_segment_metrics(
    segment: &ExactBRepPlanarSegment,
) -> Option<(f64, [f64; 2], [f64; 2])> {
    match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } => {
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let direction = [end[0] - start[0], end[1] - start[1]];
            let length = direction[0].hypot(direction[1]);
            if !length.is_finite() || length <= MIN_EXACT_BREP_SWEEP_PATH_SEGMENT_LENGTH_MM {
                return None;
            }
            let tangent = [direction[0] / length, direction[1] / length];
            Some((length, tangent, tangent))
        }
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } => {
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let center = center_bits.map(f64::from_bits);
            let start_radius = [start[0] - center[0], start[1] - center[1]];
            let end_radius = [end[0] - center[0], end[1] - center[1]];
            let radius = start_radius[0].hypot(start_radius[1]);
            let end_radius_length = end_radius[0].hypot(end_radius[1]);
            let start_angle = start_radius[1].atan2(start_radius[0]);
            let end_angle = end_radius[1].atan2(end_radius[0]);
            let sweep_angle = if *clockwise {
                (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
            } else {
                (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
            };
            if !radius.is_finite()
                || radius <= MIN_EXACT_BREP_SWEEP_PATH_SEGMENT_LENGTH_MM
                || (radius - end_radius_length).abs() > SWEEP_PATH_INTERSECTION_EPSILON_MM
                || start == end
                || !sweep_angle.is_finite()
                || radius * sweep_angle <= MIN_EXACT_BREP_SWEEP_PATH_SEGMENT_LENGTH_MM
            {
                return None;
            }
            let tangent = |radial: [f64; 2], radial_length: f64| {
                if *clockwise {
                    [radial[1] / radial_length, -radial[0] / radial_length]
                } else {
                    [-radial[1] / radial_length, radial[0] / radial_length]
                }
            };
            Some((
                radius * sweep_angle,
                tangent(start_radius, radius),
                tangent(end_radius, end_radius_length),
            ))
        }
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => {
            let start = start_bits.map(f64::from_bits);
            let control_1 = control_1_bits.map(f64::from_bits);
            let control_2 = control_2_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let chord = [end[0] - start[0], end[1] - start[1]];
            let start_handle = [control_1[0] - start[0], control_1[1] - start[1]];
            let end_handle = [end[0] - control_2[0], end[1] - control_2[1]];
            let middle = [control_2[0] - control_1[0], control_2[1] - control_1[1]];
            let chord_squared = chord[0] * chord[0] + chord[1] * chord[1];
            let start_length = start_handle[0].hypot(start_handle[1]);
            let end_length = end_handle[0].hypot(end_handle[1]);
            let control_length = start_length + middle[0].hypot(middle[1]) + end_length;
            let projection_1 = start_handle[0] * chord[0] + start_handle[1] * chord[1];
            let control_2_from_start = [control_2[0] - start[0], control_2[1] - start[1]];
            let projection_2 =
                control_2_from_start[0] * chord[0] + control_2_from_start[1] * chord[1];
            if !control_length.is_finite()
                || start_length <= MIN_EXACT_BREP_SWEEP_PATH_SEGMENT_LENGTH_MM
                || end_length <= MIN_EXACT_BREP_SWEEP_PATH_SEGMENT_LENGTH_MM
                || projection_1 <= 0.0
                || projection_2 < projection_1
                || projection_2 >= chord_squared
            {
                return None;
            }
            Some((
                control_length,
                [
                    start_handle[0] / start_length,
                    start_handle[1] / start_length,
                ],
                [end_handle[0] / end_length, end_handle[1] / end_length],
            ))
        }
    }
}

fn sweep_path_planar_bounds(segments: &[ExactBRepPlanarSegment]) -> Option<[[f64; 2]; 2]> {
    let mut bounds = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
    let mut include = |point: [f64; 2]| {
        for axis in 0..2 {
            bounds[0][axis] = bounds[0][axis].min(point[axis]);
            bounds[1][axis] = bounds[1][axis].max(point[axis]);
        }
    };
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
                center_bits,
                start_bits,
                end_bits,
                ..
            } => {
                let center = center_bits.map(f64::from_bits);
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let start_radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                let end_radius = (end[0] - center[0]).hypot(end[1] - center[1]);
                let radius = start_radius.max(end_radius);
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
    bounds
        .iter()
        .flatten()
        .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
        .then_some(bounds)
}

const SWEEP_PATH_INTERSECTION_EPSILON_MM: f64 = 1.0e-9;

fn sweep_path_segment_endpoints(segment: &ExactBRepPlanarSegment) -> ([f64; 2], [f64; 2]) {
    match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        }
        | ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            ..
        }
        | ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            end_bits,
            ..
        } => (start_bits.map(f64::from_bits), end_bits.map(f64::from_bits)),
    }
}

fn sweep_path_segment_bounds(segment: &ExactBRepPlanarSegment) -> [[f64; 2]; 2] {
    let points = match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } => vec![start_bits.map(f64::from_bits), end_bits.map(f64::from_bits)],
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            ..
        } => {
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let center = center_bits.map(f64::from_bits);
            let start_radius = (start[0] - center[0]).hypot(start[1] - center[1]);
            let end_radius = (end[0] - center[0]).hypot(end[1] - center[1]);
            let radius = start_radius.max(end_radius);
            return [
                [center[0] - radius, center[1] - radius],
                [center[0] + radius, center[1] + radius],
            ];
        }
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => vec![
            start_bits.map(f64::from_bits),
            control_1_bits.map(f64::from_bits),
            control_2_bits.map(f64::from_bits),
            end_bits.map(f64::from_bits),
        ],
    };
    [0, 1].map(|bound| {
        [0, 1].map(|axis| {
            points.iter().fold(
                if bound == 0 {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                },
                |value, point| {
                    if bound == 0 {
                        value.min(point[axis])
                    } else {
                        value.max(point[axis])
                    }
                },
            )
        })
    })
}

fn sweep_path_arc_angle(segment: &ExactBRepPlanarSegment) -> Option<f64> {
    let ExactBRepPlanarSegment::CircularArc {
        start_bits,
        end_bits,
        center_bits,
        clockwise,
    } = segment
    else {
        return None;
    };
    let start = start_bits.map(f64::from_bits);
    let end = end_bits.map(f64::from_bits);
    let center = center_bits.map(f64::from_bits);
    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
    let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
    Some(if *clockwise {
        (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
    } else {
        (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
    })
}

fn sweep_path_join_is_separated(
    left: &ExactBRepPlanarSegment,
    right: &ExactBRepPlanarSegment,
    tangent: [f64; 2],
) -> bool {
    let join = sweep_path_segment_endpoints(left).1;
    let projection =
        |point: [f64; 2]| (point[0] - join[0]) * tangent[0] + (point[1] - join[1]) * tangent[1];
    let left_is_behind = match left {
        ExactBRepPlanarSegment::Line { start_bits, .. } => {
            projection(start_bits.map(f64::from_bits)) < -SWEEP_PATH_INTERSECTION_EPSILON_MM
        }
        ExactBRepPlanarSegment::CircularArc { .. } => sweep_path_arc_angle(left)
            .is_some_and(|angle| angle < std::f64::consts::PI - SWEEP_PATH_INTERSECTION_EPSILON_MM),
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            ..
        } => [*start_bits, *control_1_bits, *control_2_bits]
            .map(|point| point.map(f64::from_bits))
            .into_iter()
            .all(|point| projection(point) < -SWEEP_PATH_INTERSECTION_EPSILON_MM),
    };
    let right_is_ahead = match right {
        ExactBRepPlanarSegment::Line { end_bits, .. } => {
            projection(end_bits.map(f64::from_bits)) > SWEEP_PATH_INTERSECTION_EPSILON_MM
        }
        ExactBRepPlanarSegment::CircularArc { .. } => sweep_path_arc_angle(right)
            .is_some_and(|angle| angle < std::f64::consts::PI - SWEEP_PATH_INTERSECTION_EPSILON_MM),
        ExactBRepPlanarSegment::CubicBezier {
            control_1_bits,
            control_2_bits,
            end_bits,
            ..
        } => [*control_1_bits, *control_2_bits, *end_bits]
            .map(|point| point.map(f64::from_bits))
            .into_iter()
            .all(|point| projection(point) > SWEEP_PATH_INTERSECTION_EPSILON_MM),
    };
    left_is_behind && right_is_ahead
}

fn sweep_path_self_intersects(
    segments: &[ExactBRepPlanarSegment],
    metrics: &[(f64, [f64; 2], [f64; 2])],
) -> bool {
    if segments
        .windows(2)
        .zip(metrics.windows(2))
        .any(|(segments, metrics)| {
            !sweep_path_join_is_separated(&segments[0], &segments[1], metrics[0].2)
        })
    {
        return true;
    }
    let bounds = segments
        .iter()
        .map(sweep_path_segment_bounds)
        .collect::<Vec<_>>();
    for left in 0..bounds.len() {
        for right in left + 2..bounds.len() {
            if [0, 1].into_iter().all(|axis| {
                bounds[left][0][axis] <= bounds[right][1][axis] + SWEEP_PATH_INTERSECTION_EPSILON_MM
                    && bounds[right][0][axis]
                        <= bounds[left][1][axis] + SWEEP_PATH_INTERSECTION_EPSILON_MM
            }) {
                return true;
            }
        }
    }
    false
}

fn sweep_path_length(segments: &[ExactBRepPlanarSegment]) -> Option<f64> {
    if !(1..=MAX_EXACT_BREP_SWEEP_PATH_SEGMENTS).contains(&segments.len())
        || segments.len() == 1 && !matches!(segments[0], ExactBRepPlanarSegment::Line { .. })
    {
        return None;
    }
    let metrics = segments
        .iter()
        .map(sweep_path_segment_metrics)
        .collect::<Option<Vec<_>>>()?;
    let total_length = metrics.iter().map(|metrics| metrics.0).sum::<f64>();
    if !(MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM..=MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM)
        .contains(&total_length)
        || metrics.windows(2).any(|pair| {
            let outgoing = pair[0].2;
            let incoming = pair[1].1;
            let dot = outgoing[0] * incoming[0] + outgoing[1] * incoming[1];
            let cross = outgoing[0] * incoming[1] - outgoing[1] * incoming[0];
            dot < 1.0 - 1.0e-9 || cross.abs() > 1.0e-9
        })
        || sweep_path_self_intersects(segments, &metrics)
    {
        return None;
    }
    Some(total_length)
}

fn sweep_profile_bounds(
    profile: &ExactBRepProfile,
    path: &ExactBRepProfile,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    let [[min_u, min_v], [max_u, max_v]] = planar_geometry_bounds(&profile.geometry)?;
    let ExactBRepPlanarGeometry::Boundary {
        closed: false,
        segments,
    } = &path.geometry
    else {
        return Err(ExactBRepGraphError::InvalidParameter);
    };
    let path_length = sweep_path_length(segments).ok_or(ExactBRepGraphError::InvalidParameter)?;
    if segments.len() > 1 {
        if profile.frame_bits != identity_frame()
            || path.frame_bits != identity_frame()
            || !matches!(
                profile.geometry,
                ExactBRepPlanarGeometry::Boundary { closed: true, .. }
            )
        {
            return Err(ExactBRepGraphError::InvalidParameter);
        }
        let [[min_x, min_y], [max_x, max_y]] =
            sweep_path_planar_bounds(segments).ok_or(ExactBRepGraphError::InvalidParameter)?;
        let radial_extent = min_u.abs().max(max_u.abs());
        let bounds = [
            [min_x - radial_extent, min_y - radial_extent, min_v],
            [max_x + radial_extent, max_y + radial_extent, max_v],
        ];
        return valid_bounds(bounds)
            .then_some(bounds)
            .ok_or(ExactBRepGraphError::InvalidParameter);
    }
    let [
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        },
    ] = segments.as_slice()
    else {
        return Err(ExactBRepGraphError::InvalidParameter);
    };
    let start = start_bits.map(f64::from_bits);
    let end = end_bits.map(f64::from_bits);
    let direction = [end[0] - start[0], end[1] - start[1]];
    let tangent = [direction[0] / path_length, direction[1] / path_length];
    let section = [tangent[1], -tangent[0]];
    let frame = profile.frame_bits.map(f64::from_bits);
    let mut framed_bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for u in [min_u, max_u] {
        for v in [min_v, max_v] {
            for along in [0.0, path_length] {
                let profile_point = [
                    frame[0] + frame[3] * u + frame[6] * v + frame[9] * along,
                    frame[1] + frame[4] * u + frame[7] * v + frame[10] * along,
                    frame[2] + frame[5] * u + frame[8] * v + frame[11] * along,
                ];
                let point = [
                    start[0] + section[0] * profile_point[0] + tangent[0] * profile_point[2],
                    start[1] + section[1] * profile_point[0] + tangent[1] * profile_point[2],
                    profile_point[1],
                ];
                for axis in 0..3 {
                    framed_bounds[0][axis] = framed_bounds[0][axis].min(profile_point[axis]);
                    framed_bounds[1][axis] = framed_bounds[1][axis].max(profile_point[axis]);
                    bounds[0][axis] = bounds[0][axis].min(point[axis]);
                    bounds[1][axis] = bounds[1][axis].max(point[axis]);
                }
            }
        }
    }
    (valid_bounds(framed_bounds) && valid_bounds(bounds))
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

fn spatial_sweep_bounds(
    profile: &ExactBRepProfile,
    path: &ExactBRepSpatialPath,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    if profile.frame_bits != identity_frame() || !valid_spatial_path(path) {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    let [[min_u, min_v], [max_u, max_v]] = planar_geometry_bounds(&profile.geometry)?;
    let section_radius = min_u
        .abs()
        .max(max_u.abs())
        .hypot(min_v.abs().max(max_v.abs()));
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut include = |point: [f64; 3]| {
        for axis in 0..3 {
            bounds[0][axis] = bounds[0][axis].min(point[axis] - section_radius);
            bounds[1][axis] = bounds[1][axis].max(point[axis] + section_radius);
        }
    };
    for segment in &path.segments {
        match segment {
            ExactBRepSpatialPathSegment::Line {
                start_bits,
                end_bits,
            } => {
                include(start_bits.map(f64::from_bits));
                include(end_bits.map(f64::from_bits));
            }
            ExactBRepSpatialPathSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                let radius = subtract(start, center);
                let radius = dot(radius, radius).sqrt();
                include(center.map(|coordinate| coordinate - radius));
                include(center.map(|coordinate| coordinate + radius));
                include(end_bits.map(f64::from_bits));
            }
            ExactBRepSpatialPathSegment::CubicBezier {
                start_bits,
                control_1_bits,
                control_2_bits,
                end_bits,
            } => {
                for point in [start_bits, control_1_bits, control_2_bits, end_bits] {
                    include(point.map(f64::from_bits));
                }
            }
        }
    }
    valid_bounds(bounds)
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

pub(crate) fn spatial_sweep_bounds_are_valid(
    profile: &FeatureKind,
    segments: &[SpatialPathSegment],
) -> bool {
    let geometry = match profile {
        FeatureKind::Profile { points_mm } => polygon_geometry(points_mm),
        FeatureKind::SegmentProfile {
            segments,
            closed: true,
        } => boundary_geometry(segments, true),
        _ => return false,
    };
    let (Ok(geometry), Ok(path)) = (geometry, spatial_path(FeatureId(1), segments)) else {
        return false;
    };
    spatial_sweep_bounds(
        &ExactBRepProfile {
            id: ExactBRepProfileId(0),
            source_feature_id: 1,
            region_id: None,
            frame_bits: identity_frame(),
            geometry,
        },
        &path,
    )
    .is_ok()
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

fn planar_offset_distance(value: f64) -> Result<u64, ExactBRepGraphError> {
    if value.is_finite() && (EXACT_MIN_LENGTH_MM..=MAX_ABS_MM).contains(&value.abs()) {
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

fn operation_requires_v7(operation: &ExactBRepOperation, profiles: &[ExactBRepProfile]) -> bool {
    let ExactBRepOperation::PlanarOffset { profile, .. } = operation else {
        return false;
    };
    matches!(
        profiles.get(profile.0 as usize).map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Boundary { segments, .. })
            if segments.iter().any(|segment| matches!(segment, ExactBRepPlanarSegment::CubicBezier { .. }))
    )
}

fn operation_requires_v8(operation: &ExactBRepOperation, profiles: &[ExactBRepProfile]) -> bool {
    let ExactBRepOperation::PlanarOffset { profile, .. } = operation else {
        return false;
    };
    matches!(
        profiles
            .get(profile.0 as usize)
            .map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Region { .. })
    )
}

fn operation_requires_v9(operation: &ExactBRepOperation, profiles: &[ExactBRepProfile]) -> bool {
    let ExactBRepOperation::Sweep { path, .. } = operation else {
        return false;
    };
    matches!(
        profiles.get(path.0 as usize).map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Boundary { segments, .. }) if segments.len() > 1
    )
}

fn operation_requires_v10(operation: &ExactBRepOperation, profiles: &[ExactBRepProfile]) -> bool {
    let ExactBRepOperation::Sweep { path, .. } = operation else {
        return false;
    };
    matches!(
        profiles.get(path.0 as usize).map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Boundary { segments, .. }) if segments.len() > 2
    )
}

fn operation_requires_v11(operation: &ExactBRepOperation, profiles: &[ExactBRepProfile]) -> bool {
    let ExactBRepOperation::Sweep { path, .. } = operation else {
        return false;
    };
    matches!(
        profiles.get(path.0 as usize).map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Boundary { segments, .. })
            if segments.iter().any(|segment| matches!(segment, ExactBRepPlanarSegment::CubicBezier { .. }))
    )
}

fn operation_requires_v12(operation: &ExactBRepOperation) -> bool {
    matches!(operation, ExactBRepOperation::SpatialSweep { .. })
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
        ExactBRepOperation::PlanarOffset {
            profile,
            distance_bits,
        } => profiles.get(profile.0 as usize).is_some_and(|profile| {
            planar_offset_profile_bounds(profile, f64::from_bits(*distance_bits)).is_ok()
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
                && profiles
                    .get(profile.0 as usize)
                    .zip(profiles.get(path.0 as usize))
                    .is_some_and(|(profile, path)| sweep_profile_bounds(profile, path).is_ok())
        }
        ExactBRepOperation::SpatialSweep { profile, path } => {
            profiles.get(profile.0 as usize).is_some_and(|profile| {
                matches!(
                    profile.geometry,
                    ExactBRepPlanarGeometry::Boundary { closed: true, .. }
                        | ExactBRepPlanarGeometry::Circle { .. }
                        | ExactBRepPlanarGeometry::Region { .. }
                ) && spatial_sweep_bounds(profile, path).is_ok()
            })
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
        ExactBRepOperation::PlanarOffset { distance_bits, .. } => {
            planar_offset_distance(f64::from_bits(*distance_bits)).is_ok()
        }
        ExactBRepOperation::Sweep { profile, path } => profile != path,
        ExactBRepOperation::SpatialSweep { path, .. } => valid_spatial_path(path),
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
    fn inward_circle_offset_rejects_source_outside_coordinate_envelope() {
        let profile = ExactBRepProfile {
            id: ExactBRepProfileId(0),
            source_feature_id: 1,
            region_id: Some(1),
            frame_bits: identity_frame(),
            geometry: ExactBRepPlanarGeometry::Circle {
                center_bits: [MAX_ABS_MM - 5.0, 0.0].map(f64::to_bits),
                radius_bits: 10.0_f64.to_bits(),
            },
        };
        assert_eq!(
            planar_offset_profile_bounds(&profile, -5.0),
            Err(ExactBRepGraphError::InvalidParameter)
        );
    }

    #[test]
    fn rectangle_offset_exception_requires_exact_axis_alignment() {
        let profile = |points: [[f64; 2]; 4]| ExactBRepProfile {
            id: ExactBRepProfileId(0),
            source_feature_id: 1,
            region_id: None,
            frame_bits: identity_frame(),
            geometry: ExactBRepPlanarGeometry::Boundary {
                closed: true,
                segments: (0..4)
                    .map(|index| ExactBRepPlanarSegment::Line {
                        start_bits: points[index].map(f64::to_bits),
                        end_bits: points[(index + 1) % 4].map(f64::to_bits),
                    })
                    .collect(),
            },
        };
        let rectangle = profile([[0.0, 0.0], [10.0, 0.0], [10.0, 8.0], [0.0, 8.0]]);
        assert_eq!(
            planar_offset_profile_bounds(&rectangle, 800_000.0),
            Ok([[-800_000.0, -800_000.0, 0.0], [800_010.0, 800_008.0, 0.0],])
        );

        let skewed = profile([[0.0, 0.0], [10.0, 0.000_001_5], [10.0, 8.0], [0.0, 8.0]]);
        assert!(exact_brep_planar_rectangle_bounds(&skewed).is_none());
        assert_eq!(
            planar_offset_profile_bounds(&skewed, 100_000.001),
            Err(ExactBRepGraphError::InvalidParameter)
        );
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
            schema: EXACT_BREP_GRAPH_SCHEMA_V6.to_owned(),
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
        let arc = |start: [f64; 2], end: [f64; 2], center: [f64; 2]| {
            ExactBRepPlanarSegment::CircularArc {
                start_bits: start.map(f64::to_bits),
                end_bits: end.map(f64::to_bits),
                center_bits: center.map(f64::to_bits),
                clockwise: false,
            }
        };
        let cubic = |start: f64| ExactBRepPlanarSegment::CubicBezier {
            start_bits: [start.to_bits(), 0.0_f64.to_bits()],
            control_1_bits: [(start + 0.5).to_bits(), 0.0_f64.to_bits()],
            control_2_bits: [(start + 1.5).to_bits(), 0.0_f64.to_bits()],
            end_bits: [(start + 2.0).to_bits(), 0.0_f64.to_bits()],
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
        for length in [
            MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM,
            MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM,
        ] {
            profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
                closed: false,
                segments: vec![line([0.0, 0.0], [length, 0.0])],
            };
            assert!(valid_operation_profiles(&sweep, &profiles));
        }
        for length in [
            MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM - 0.001,
            MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM + 0.001,
        ] {
            profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
                closed: false,
                segments: vec![line([0.0, 0.0], [length, 0.0])],
            };
            assert!(!valid_operation_profiles(&sweep, &profiles));
        }
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![line(
                [MAX_EXACT_BREP_COORDINATE_MM, 0.0],
                [MAX_EXACT_BREP_COORDINATE_MM, 100.0],
            )],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles));

        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![line([0.0, 0.0], [100.0, 0.0])],
        };
        profiles[0].frame_bits[0] = 10.0_f64.to_bits();
        assert!(valid_operation_profiles(&sweep, &profiles));
        profiles[0].frame_bits[0] = MAX_EXACT_BREP_COORDINATE_MM.to_bits();
        assert!(!valid_operation_profiles(&sweep, &profiles));
        profiles[0].frame_bits = identity_frame();

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
            segments: vec![
                line([0.0, 0.0], [50.0, 0.0]),
                arc([50.0, 0.0], [75.0, 25.0], [50.0, 25.0]),
            ],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles));
        profiles[0].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments: vec![
                line([-2.0, -1.0], [2.0, -1.0]),
                line([2.0, -1.0], [2.0, 1.0]),
                line([2.0, 1.0], [-2.0, 1.0]),
                line([-2.0, 1.0], [-2.0, -1.0]),
            ],
        };
        assert!(valid_operation_profiles(&sweep, &profiles));
        profiles[1].frame_bits[0] = 1.0_f64.to_bits();
        assert!(!valid_operation_profiles(&sweep, &profiles));
        profiles[1].frame_bits = identity_frame();

        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![line([0.0, 0.0], [5.0, 0.0]), line([5.0, 0.0], [10.0, 0.0])],
        };
        assert!(valid_operation_profiles(&sweep, &profiles));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![
                line(
                    [0.0, 0.0],
                    [MIN_EXACT_BREP_SWEEP_PATH_SEGMENT_LENGTH_MM / 2.0, 0.0],
                ),
                line(
                    [MIN_EXACT_BREP_SWEEP_PATH_SEGMENT_LENGTH_MM / 2.0, 0.0],
                    [10.0, 0.0],
                ),
            ],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles));

        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![line([0.0, 0.0], [5.0, 0.0]), line([5.0, 0.0], [5.0, 5.0])],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![
                line([0.0, 0.0], [5.0, 0.0]),
                line([5.0, 0.0], [10.0, 0.0]),
                line([10.0, 0.0], [15.0, 0.0]),
            ],
        };
        assert!(valid_operation_profiles(&sweep, &profiles));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: (0..MAX_EXACT_BREP_SWEEP_PATH_SEGMENTS)
                .map(|index| line([index as f64, 0.0], [index as f64 + 1.0, 0.0]))
                .collect(),
        };
        assert!(valid_operation_profiles(&sweep, &profiles));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: (0..MAX_EXACT_BREP_SWEEP_PATH_SEGMENTS)
                .map(|index| cubic(index as f64 * 2.0))
                .collect(),
        };
        assert!(valid_operation_profiles(&sweep, &profiles));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: (0..=MAX_EXACT_BREP_SWEEP_PATH_SEGMENTS)
                .map(|index| line([index as f64, 0.0], [index as f64 + 1.0, 0.0]))
                .collect(),
        };
        assert!(!valid_operation_profiles(&sweep, &profiles));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![
                line([0.0, 0.0], [MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM, 0.0]),
                line(
                    [MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM, 0.0],
                    [MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM + 1.0, 0.0],
                ),
            ],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles));
    }
}
