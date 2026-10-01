use crate::document::{
    BodyKind, BooleanOperation, CanonicalError, ChamferMode, DefinitionId, EdgeFinishKind, EdgeRef,
    FaceRef, FeatureDependencyGraph, FeatureId, FeatureKind, LoftContinuity, LoftSection,
    ProfileFaceReference, ProfileSegment, ShellDirection, Snapshot, SpatialPathSegment,
    SurfaceBodySpec, Transform, WeldmentJointPolicy, WeldmentJointPrimary,
    solved_sketch_sweep_path,
};
use crate::exact_product::{
    EXACT_MIN_LENGTH_MM, ExactCircleProfile, ExactPlanarOffsetRegion,
    MAX_EXACT_PLANAR_OFFSET_LENGTH_MM, accepts_planar_circle_offset_geometry,
    accepts_planar_offset_geometry, exact_planar_offset_profile_from_segments,
};
use crate::sheet_metal::{BendShape, SheetMetalShape};
use crate::tolerance::{APPROXIMATION, MAX_COORDINATE_MM, ROUNDING, TolerancePolicy};
use crate::topology::{TopologicalElementKind, TopologicalElementRef, TopologicalReferenceError};
use ketchup_geometry::linalg::{Frame, cross, dot, sub};
use ketchup_geometry::sketch::{
    CutStart, FeatureDirection, FeatureExtent, FeatureExtentEnd, PadOperation, PadProfile,
    SketchError, SketchRegionId, SolvedSketchRegion, SolvedSketchRegionEdge,
    SolvedSketchRegionProfile, WorkplaneFrame,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

mod bounds;
mod compiler;
mod geometry;
mod validation;
pub use bounds::*;
use compiler::*;
use geometry::*;
use validation::*;

pub const EXACT_BREP_GRAPH_SCHEMA_V6: &str = "ketchup.exact-brep-graph.v6";
pub const EXACT_BREP_GRAPH_SCHEMA_V7: &str = "ketchup.exact-brep-graph.v7";
pub const EXACT_BREP_GRAPH_SCHEMA_V8: &str = "ketchup.exact-brep-graph.v8";
pub const EXACT_BREP_GRAPH_SCHEMA_V9: &str = "ketchup.exact-brep-graph.v9";
pub const EXACT_BREP_GRAPH_SCHEMA_V10: &str = "ketchup.exact-brep-graph.v10";
pub const EXACT_BREP_GRAPH_SCHEMA_V11: &str = "ketchup.exact-brep-graph.v11";
pub const EXACT_BREP_GRAPH_SCHEMA_V12: &str = "ketchup.exact-brep-graph.v12";
pub const EXACT_BREP_GRAPH_SCHEMA_V13: &str = "ketchup.exact-brep-graph.v13";
pub const EXACT_BREP_GRAPH_SCHEMA_V14: &str = "ketchup.exact-brep-graph.v14";
pub const EXACT_BREP_GRAPH_SCHEMA_V15: &str = "ketchup.exact-brep-graph.v15";
pub const EXACT_BREP_GRAPH_SCHEMA_V16: &str = "ketchup.exact-brep-graph.v16";
pub const EXACT_BREP_GRAPH_SCHEMA_V17: &str = "ketchup.exact-brep-graph.v17";
pub const EXACT_BREP_GRAPH_SCHEMA_V18: &str = "ketchup.exact-brep-graph.v18";
pub const EXACT_BREP_GRAPH_SCHEMA_V19: &str = "ketchup.exact-brep-graph.v19";
pub const EXACT_BREP_GRAPH_SCHEMA_V20: &str = "ketchup.exact-brep-graph.v20";
pub const EXACT_BREP_GRAPH_SCHEMA_V21: &str = "ketchup.exact-brep-graph.v21";
pub const EXACT_BREP_GRAPH_SCHEMA_V22: &str = "ketchup.exact-brep-graph.v22";
pub const EXACT_BREP_GRAPH_SCHEMA_V23: &str = "ketchup.exact-brep-graph.v23";
pub const MAX_EXACT_BREP_GRAPH_PROFILES: usize = 1_024;
pub const MAX_EXACT_BREP_GRAPH_NODES: usize = 1_024;
pub const MAX_EXACT_BREP_GRAPH_SEGMENTS: usize = 16_384;
pub const MAX_EXACT_BREP_LOFT_SECTIONS: usize = 16;
pub const MAX_EXACT_BREP_LOFT_CONTROL_POINTS: usize = 64;
pub const MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM: f64 = 0.01;
pub const MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM: f64 = 100_000.0;
pub const MAX_EXACT_BREP_GRAPH_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EXACT_BREP_TOPOLOGY_SELECTORS: usize = 64;
const MAX_ABS_MM: f64 = MAX_COORDINATE_MM;

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactBRepWeldmentJointPolicy {
    Butt,
    Miter,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactBRepWeldmentJointPrimary {
    First,
    Second,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExactBRepProfileFaceReference {
    Start,
    End,
    Segment { entity_id: u64, source_name: String },
    NamedResult { name: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepProfileEdgeReference {
    pub first: ExactBRepProfileFaceReference,
    pub second: ExactBRepProfileFaceReference,
}

impl ExactBRepTopologySelector {
    pub fn reference(&self) -> Result<TopologicalElementRef, ExactBRepGraphError> {
        let reference = TopologicalElementRef::from_bytes(&self.reference_bytes)
            .map_err(ExactBRepGraphError::InvalidTopologyReference)?;
        if reference.kind != self.kind.element_kind() {
            return Err(ExactBRepGraphError::InvalidTopologySelector);
        }
        Ok(reference)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepFilletRadiusStation {
    pub position_bits: u64,
    pub radius_bits: u64,
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExactBRepChamferMode {
    #[default]
    Symmetric,
    TwoDistance {
        second_distance_bits: u64,
    },
    DistanceAngle {
        angle_degrees_bits: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepChamferEdgeSide {
    pub edge: ExactBRepTopologySelector,
    pub side_face: ExactBRepTopologySelector,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactBRepShellDirection {
    #[default]
    Inward,
    Outward,
    Symmetric,
}

impl From<ShellDirection> for ExactBRepShellDirection {
    fn from(direction: ShellDirection) -> Self {
        match direction {
            ShellDirection::Inward => Self::Inward,
            ShellDirection::Outward => Self::Outward,
            ShellDirection::Symmetric => Self::Symmetric,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segment_entity_ids: Vec<u64>,
    pub geometry: ExactBRepPlanarGeometry,
}

impl ExactBRepProfile {
    /// The profile plane: origin, in-plane x and y axes, and the direction
    /// the profile is extruded or swept along.
    #[must_use]
    pub fn frame(&self) -> Frame {
        Frame::from(self.frame_bits.map(f64::from_bits))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactBRepLoftContinuity {
    #[default]
    Position,
    Tangent,
    Curvature,
}

impl From<LoftContinuity> for ExactBRepLoftContinuity {
    fn from(value: LoftContinuity) -> Self {
        match value {
            LoftContinuity::Position => Self::Position,
            LoftContinuity::Tangent => Self::Tangent,
            LoftContinuity::Curvature => Self::Curvature,
        }
    }
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

/// A bend of [`ExactBRepOperation::SheetMetal`]; see [`crate::sheet_metal::SheetMetalBend`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBRepSheetMetalBend {
    pub parent: Option<usize>,
    pub edge: usize,
    pub length_bits: u64,
    pub angle_degrees_bits: u64,
    pub inner_radius_bits: u64,
}

impl ExactBRepSheetMetalBend {
    #[must_use]
    pub fn shape(&self) -> BendShape {
        BendShape {
            parent: self.parent,
            edge: self.edge,
            length_mm: f64::from_bits(self.length_bits),
            angle_degrees: f64::from_bits(self.angle_degrees_bits),
            inner_radius_mm: f64::from_bits(self.inner_radius_bits),
        }
    }
}

/// The sheet [`ExactBRepOperation::SheetMetal`] describes.
#[must_use]
pub fn exact_sheet_metal_shape(
    base_mm_bits: &[[u64; 2]],
    thickness_bits: u64,
    k_factor_bits: u64,
    bends: &[ExactBRepSheetMetalBend],
) -> SheetMetalShape {
    SheetMetalShape {
        base_mm: base_mm_bits
            .iter()
            .map(|corner| corner.map(f64::from_bits))
            .collect(),
        thickness_mm: f64::from_bits(thickness_bits),
        k_factor: f64::from_bits(k_factor_bits),
        bends: bends.iter().map(ExactBRepSheetMetalBend::shape).collect(),
    }
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
    },
    Boolean {
        operation: ExactBRepBooleanOperation,
        target: ExactBRepNodeId,
        tool: ExactBRepNodeId,
        /// Prefix naming the faces the tool contributes (`tool_name.face`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
    },
    Shell {
        target: ExactBRepNodeId,
        removed_faces: Vec<ExactBRepTopologySelector>,
        thickness_bits: u64,
        #[serde(default)]
        direction: ExactBRepShellDirection,
        /// Program-named faces to open, instead of `removed_faces`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        profile_faces: Vec<ExactBRepProfileFaceReference>,
        /// Prefix naming the inner walls (`name.face`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    EdgeFinish {
        target: ExactBRepNodeId,
        edges: Vec<ExactBRepTopologySelector>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        profile_edges: Vec<ExactBRepProfileEdgeReference>,
        kind: ExactBRepEdgeFinishKind,
        amount_bits: u64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fillet_radius_stations: Vec<ExactBRepFilletRadiusStation>,
        #[serde(default)]
        chamfer_mode: ExactBRepChamferMode,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        chamfer_edge_sides: Vec<ExactBRepChamferEdgeSide>,
    },
    FaceOffset {
        target: ExactBRepNodeId,
        face: Option<ExactBRepTopologySelector>,
        profile_face: Option<ExactBRepProfileFaceReference>,
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
    PlanarSurface {
        profile: ExactBRepProfileId,
    },
    LoftSurface {
        sections: Vec<ExactBRepLoftSection>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guide: Option<ExactBRepSpatialPath>,
        #[serde(default)]
        continuity: ExactBRepLoftContinuity,
    },
    SurfaceTrim {
        target: ExactBRepNodeId,
        cutter: ExactBRepNodeId,
    },
    SurfaceExtend {
        target: ExactBRepNodeId,
        distance_bits: u64,
    },
    SurfaceKnit {
        surfaces: Vec<ExactBRepNodeId>,
        tolerance_bits: u64,
        make_solid: bool,
    },
    SurfaceThicken {
        target: ExactBRepNodeId,
        thickness_bits: u64,
        direction: ExactBRepShellDirection,
    },
    Sweep {
        profile: ExactBRepProfileId,
        path: ExactBRepProfileId,
    },
    SpatialSweep {
        profile: ExactBRepProfileId,
        path: ExactBRepSpatialPath,
        /// Unit direction the profile's v keeps (fixed binormal); see
        /// `FeatureKind::Sweep::up`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        up_bits: Option<[u64; 3]>,
    },
    WeldmentJoint {
        first: ExactBRepNodeId,
        second: ExactBRepNodeId,
        joint_point_bits: [u64; 3],
        first_direction_bits: [u64; 3],
        second_direction_bits: [u64; 3],
        policy: ExactBRepWeldmentJointPolicy,
        primary: ExactBRepWeldmentJointPrimary,
    },
    Loft {
        sections: Vec<ExactBRepLoftSection>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guide: Option<ExactBRepSpatialPath>,
        #[serde(default)]
        continuity: ExactBRepLoftContinuity,
    },
    SheetMetal {
        base_mm_bits: Vec<[u64; 2]>,
        thickness_bits: u64,
        k_factor_bits: u64,
        bends: Vec<ExactBRepSheetMetalBend>,
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
            | Self::SurfaceExtend { target, .. }
            | Self::SurfaceThicken { target, .. }
            | Self::RigidTransform { target, .. } => vec![*target],
            Self::Boolean { target, tool, .. }
            | Self::SurfaceTrim {
                target,
                cutter: tool,
            } => vec![*target, *tool],
            Self::WeldmentJoint { first, second, .. } => vec![*first, *second],
            Self::SurfaceKnit { surfaces, .. } => surfaces.clone(),
            Self::Extrude { .. }
            | Self::Revolve { .. }
            | Self::PlanarOffset { .. }
            | Self::PlanarSurface { .. }
            | Self::LoftSurface { .. }
            | Self::Sweep { .. }
            | Self::SpatialSweep { .. }
            | Self::Loft { .. }
            | Self::SheetMetal { .. }
            | Self::ImportedExact { .. } => Vec::new(),
        }
    }

    fn profile_ids(&self) -> Vec<ExactBRepProfileId> {
        match self {
            Self::Extrude { profile, .. }
            | Self::ProfileCut { profile, .. }
            | Self::Revolve { profile, .. }
            | Self::PlanarOffset { profile, .. }
            | Self::PlanarSurface { profile } => vec![*profile],
            Self::Sweep { profile, path } => vec![*profile, *path],
            Self::SpatialSweep { profile, .. } => vec![*profile],
            Self::Loft { sections, .. } | Self::LoftSurface { sections, .. } => {
                sections.iter().map(|section| section.profile).collect()
            }
            Self::Boolean { .. }
            | Self::SurfaceTrim { .. }
            | Self::SurfaceExtend { .. }
            | Self::SurfaceKnit { .. }
            | Self::SurfaceThicken { .. }
            | Self::WeldmentJoint { .. }
            | Self::Shell { .. }
            | Self::EdgeFinish { .. }
            | Self::FaceOffset { .. }
            | Self::SheetMetal { .. }
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
    /// The document's tolerance, so validation and the exact kernel use the same values as
    /// the model. Absent when it is the default, which keeps existing graph digests.
    #[serde(default, skip_serializing_if = "TolerancePolicy::is_default")]
    pub tolerance: TolerancePolicy,
}

#[derive(Serialize)]
struct GraphDigestPayload<'a> {
    schema: &'a str,
    definition_id: u64,
    producer_feature_id: u64,
    profiles: &'a [ExactBRepProfile],
    nodes: &'a [ExactBRepNode],
    #[serde(skip_serializing_if = "TolerancePolicy::is_default")]
    tolerance: TolerancePolicy,
}

impl ExactBRepGraph {
    /// The profile feature whose cut made the face at `point_mm` (definition
    /// coordinates): the point lies on a wall swept by a cut profile's boundary,
    /// or on the floor of a blind cut within the profile's bounds. The latest
    /// cut wins when several match.
    #[must_use]
    pub fn profile_cut_at(&self, point_mm: [f64; 3], tolerance_mm: f64) -> Option<u64> {
        self.nodes.iter().rev().find_map(|node| {
            let ExactBRepOperation::ProfileCut {
                profile,
                depth_bits,
                interval,
                ..
            } = &node.operation
            else {
                return None;
            };
            let profile = self.profiles.get(profile.0 as usize)?;
            let [origin, x_axis, y_axis, _] = profile.frame().to_vectors();
            let relative = sub(point_mm, origin);
            let local = [dot(relative, x_axis), dot(relative, y_axis)];
            let along = dot(relative, interval.direction());
            let blind = depth_bits.is_some();
            if blind
                && (along < interval.start_mm() - tolerance_mm
                    || along > interval.end_mm() + tolerance_mm)
            {
                return None;
            }
            let on_wall = planar_geometry_distance(&profile.geometry, local)
                .is_some_and(|distance| distance <= tolerance_mm);
            // The floor is the closed end; the open end has no face to hit.
            let on_floor = blind
                && ((along - interval.start_mm()).abs() <= tolerance_mm
                    || (along - interval.end_mm()).abs() <= tolerance_mm)
                && planar_geometry_contains(&profile.geometry, local);
            (on_wall || on_floor).then_some(profile.source_feature_id)
        })
    }

    /// Frame of a planar face made by extruding `profile_feature_id`, named by
    /// the semantic role the evaluator gives it: an end cap (`extrusion.top`,
    /// `extrusion.bottom`) or the side swept by the profile's first line
    /// (`extrusion.side(profile_edge=line.0)`). The frame is
    /// right-handed, its normal points out of the solid and, for sides, its y
    /// axis follows the extrusion. Later cuts may split a face but keep its plane.
    #[must_use]
    pub fn extrusion_face_frame(
        &self,
        profile_feature_id: u64,
        semantic_role: &str,
    ) -> Option<WorkplaneFrame> {
        let (profile, interval) = self.nodes.iter().find_map(|node| {
            let ExactBRepOperation::Extrude {
                profile, interval, ..
            } = &node.operation
            else {
                return None;
            };
            let profile = self.profiles.get(profile.0 as usize)?;
            (profile.source_feature_id == profile_feature_id).then_some((profile, *interval))
        })?;
        let [origin, x_axis, y_axis, _] = profile.frame().to_vectors();
        let direction = interval.direction();
        let at = |u: f64, v: f64, along: f64| {
            [0, 1, 2].map(|axis| {
                origin[axis] + x_axis[axis] * u + y_axis[axis] * v + direction[axis] * along
            })
        };
        match semantic_role {
            "extrusion.top" => Some(WorkplaneFrame {
                origin_mm: at(0.0, 0.0, interval.end_mm()),
                x_axis: cross(y_axis, direction),
                y_axis,
                normal: direction,
            }),
            "extrusion.bottom" => {
                let normal = direction.map(|value| -value);
                Some(WorkplaneFrame {
                    origin_mm: at(0.0, 0.0, interval.start_mm()),
                    x_axis,
                    y_axis: cross(normal, x_axis),
                    normal,
                })
            }
            _ => {
                let ExactBRepPlanarGeometry::Boundary {
                    closed: true,
                    segments,
                } = &profile.geometry
                else {
                    return None;
                };
                let lines = segments
                    .iter()
                    .map(|segment| match segment {
                        ExactBRepPlanarSegment::Line {
                            start_bits,
                            end_bits,
                        } => Some((start_bits.map(f64::from_bits), end_bits.map(f64::from_bits))),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let (start, end) = match semantic_role {
                    // The evaluator names the first line only when the profile has no arc.
                    "extrusion.side(profile_edge=line.0)" => {
                        if segments.iter().any(|segment| {
                            matches!(segment, ExactBRepPlanarSegment::CircularArc { .. })
                        }) {
                            return None;
                        }
                        lines.iter().flatten().copied().next()?
                    }
                    _ => return None,
                };
                // Twice the signed area: positive for a counter-clockwise profile.
                let doubled_area = segments
                    .iter()
                    .map(|segment| {
                        let (start_bits, end_bits) = match segment {
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
                            } => (start_bits, end_bits),
                        };
                        let [start, end] =
                            [start_bits, end_bits].map(|bits| bits.map(f64::from_bits));
                        start[0] * end[1] - end[0] * start[1]
                    })
                    .sum::<f64>();
                let edge = [end[0] - start[0], end[1] - start[1]];
                let length = edge[0].hypot(edge[1]);
                if length <= self.tolerance.linear_mm()
                    || doubled_area.abs() <= self.tolerance.linear_mm()
                {
                    return None;
                }
                let outward = if doubled_area > 0.0 {
                    [edge[1] / length, -edge[0] / length]
                } else {
                    [-edge[1] / length, edge[0] / length]
                };
                let normal =
                    [0, 1, 2].map(|axis| x_axis[axis] * outward[0] + y_axis[axis] * outward[1]);
                let side_x_axis = cross(direction, normal);
                let [first, second] =
                    [start, end].map(|point| at(point[0], point[1], interval.start_mm()));
                let origin_mm = if dot(sub(second, first), side_x_axis) >= 0.0 {
                    first
                } else {
                    second
                };
                Some(WorkplaneFrame {
                    origin_mm,
                    x_axis: side_x_axis,
                    y_axis: direction,
                    normal,
                })
            }
        }
    }

    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
    ) -> Result<Self, ExactBRepGraphError> {
        let dependencies = snapshot
            .feature_dependency_graph()
            .map_err(|error| ExactBRepGraphError::DependencyGraph(Box::new(error)))?;
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
            .any(|node| operation_requires_v23(&node.operation))
        {
            EXACT_BREP_GRAPH_SCHEMA_V23
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v22(&node.operation))
        {
            EXACT_BREP_GRAPH_SCHEMA_V22
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v21(&node.operation))
        {
            EXACT_BREP_GRAPH_SCHEMA_V21
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v20(&node.operation))
        {
            EXACT_BREP_GRAPH_SCHEMA_V20
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v19(&node.operation))
        {
            EXACT_BREP_GRAPH_SCHEMA_V19
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v18(&node.operation))
        {
            EXACT_BREP_GRAPH_SCHEMA_V18
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v17(&node.operation))
        {
            EXACT_BREP_GRAPH_SCHEMA_V17
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v16(&node.operation))
        {
            EXACT_BREP_GRAPH_SCHEMA_V16
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v15(&node.operation))
        {
            EXACT_BREP_GRAPH_SCHEMA_V15
        } else if compiler
            .nodes
            .iter()
            .any(|node| operation_requires_v14(&node.operation, &compiler.profiles))
        {
            EXACT_BREP_GRAPH_SCHEMA_V14
        } else if compiler
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
            tolerance: snapshot.tolerance(),
        };
        graph.graph_digest = graph.compute_graph_digest()?;
        graph.canonical_input_digest = graph.compute_canonical_input_digest();
        graph.validate()?;
        Ok(graph)
    }

    #[must_use]
    pub fn terminal_body_kind(&self) -> BodyKind {
        match self.nodes.last().map(|node| &node.operation) {
            Some(ExactBRepOperation::SurfaceKnit { make_solid, .. }) => {
                if *make_solid {
                    BodyKind::Solid
                } else {
                    BodyKind::Surface
                }
            }
            Some(
                ExactBRepOperation::PlanarOffset { .. }
                | ExactBRepOperation::PlanarSurface { .. }
                | ExactBRepOperation::LoftSurface { .. }
                | ExactBRepOperation::SurfaceTrim { .. }
                | ExactBRepOperation::SurfaceExtend { .. },
            ) => BodyKind::Surface,
            _ => BodyKind::Solid,
        }
    }

    #[must_use]
    pub fn terminal_is_surface(&self) -> bool {
        self.terminal_body_kind() == BodyKind::Surface
    }

    #[must_use]
    pub fn terminal_is_planar_offset(&self) -> bool {
        matches!(
            self.nodes.last().map(|node| &node.operation),
            Some(ExactBRepOperation::PlanarOffset { .. })
        )
    }

    #[must_use]
    pub fn terminal_planar_offset_is_framed(&self) -> bool {
        let Some(ExactBRepOperation::PlanarOffset { profile, .. }) =
            self.nodes.last().map(|node| &node.operation)
        else {
            return false;
        };
        self.profiles
            .get(profile.0 as usize)
            .is_some_and(|profile| profile.frame_bits != identity_frame())
    }

    #[must_use]
    pub fn accepts_terminal_planar_offset_geometry(
        &self,
        bounds_mm: [[f64; 3]; 2],
        local_bounds_mm: Option<[[f64; 3]; 2]>,
        area_mm2: f64,
        topology_counts: [u32; 5],
        wire_count: Option<u32>,
    ) -> bool {
        if self.validate().is_err() {
            return false;
        }
        let (profile, distance_mm) = match self.nodes.last().map(|node| &node.operation) {
            Some(ExactBRepOperation::PlanarOffset {
                profile,
                distance_bits,
            }) => (profile, f64::from_bits(*distance_bits)),
            Some(ExactBRepOperation::PlanarSurface { profile }) => (profile, 0.0),
            _ => return false,
        };
        let Some(profile) = self.profiles.get(profile.0 as usize) else {
            return false;
        };
        if !(0..3).all(|axis| {
            bounds_mm[0][axis].is_finite()
                && bounds_mm[1][axis].is_finite()
                && bounds_mm[0][axis] <= bounds_mm[1][axis]
                && bounds_mm[0][axis].abs() <= MAX_ABS_MM
                && bounds_mm[1][axis].abs() <= MAX_ABS_MM
        }) {
            return false;
        }
        let framed = profile.frame_bits != identity_frame();
        if framed && local_bounds_mm.is_none() {
            return false;
        }
        let Some(expected_world_bounds) = self.producer_bounds_mm().ok().flatten() else {
            return false;
        };
        const BOUNDS_TOLERANCE_MM: f64 = APPROXIMATION;
        if !(0..3).all(|axis| {
            bounds_mm[0][axis] >= expected_world_bounds[0][axis] - BOUNDS_TOLERANCE_MM
                && bounds_mm[1][axis] <= expected_world_bounds[1][axis] + BOUNDS_TOLERANCE_MM
        }) {
            return false;
        }
        let evidence_bounds = if framed {
            local_bounds_mm.expect("framed planar offsets require projected local bounds")
        } else {
            bounds_mm
        };
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
                evidence_bounds,
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
                    evidence_bounds,
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
                    evidence_bounds,
                    area_mm2,
                    topology_counts,
                )
            }
            _ => false,
        }
    }

    pub fn terminal_planar_offset_local_bounds_mm(
        &self,
        vertices_mm: &[[f64; 3]],
    ) -> Option<[[f64; 3]; 2]> {
        let profile = match &self.nodes.last()?.operation {
            ExactBRepOperation::PlanarOffset { profile, .. }
            | ExactBRepOperation::PlanarSurface { profile } => profile,
            _ => return None,
        };
        let [origin, x_axis, y_axis, normal] =
            self.profiles.get(profile.0 as usize)?.frame().to_vectors();
        let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
        for vertex in vertices_mm {
            if vertex.iter().any(|value| !value.is_finite()) {
                return None;
            }
            let relative = [0, 1, 2].map(|axis| vertex[axis] - origin[axis]);
            let local = [
                dot(relative, x_axis),
                dot(relative, y_axis),
                dot(relative, normal),
            ];
            for axis in 0..3 {
                bounds[0][axis] = bounds[0][axis].min(local[axis]);
                bounds[1][axis] = bounds[1][axis].max(local[axis]);
            }
        }
        (!vertices_mm.is_empty()
            && bounds.iter().flatten().all(|value| value.is_finite())
            && bounds[0][2].abs() <= APPROXIMATION
            && bounds[1][2].abs() <= APPROXIMATION)
            .then_some(bounds)
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
                self.tolerance.linear_mm(),
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
                | EXACT_BREP_GRAPH_SCHEMA_V13
                | EXACT_BREP_GRAPH_SCHEMA_V14
                | EXACT_BREP_GRAPH_SCHEMA_V15
                | EXACT_BREP_GRAPH_SCHEMA_V16
                | EXACT_BREP_GRAPH_SCHEMA_V17
                | EXACT_BREP_GRAPH_SCHEMA_V18
                | EXACT_BREP_GRAPH_SCHEMA_V19
                | EXACT_BREP_GRAPH_SCHEMA_V20
                | EXACT_BREP_GRAPH_SCHEMA_V21
                | EXACT_BREP_GRAPH_SCHEMA_V22
                | EXACT_BREP_GRAPH_SCHEMA_V23
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
                .checked_add(validate_geometry(
                    &profile.geometry,
                    self.tolerance.linear_mm(),
                )?)
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
            let spatial_segment_count = match &node.operation {
                ExactBRepOperation::SpatialSweep { path, .. } => path.segments.len(),
                ExactBRepOperation::Loft {
                    guide: Some(path), ..
                }
                | ExactBRepOperation::LoftSurface {
                    guide: Some(path), ..
                } => path.segments.len(),
                _ => 0,
            };
            segment_count = segment_count
                .checked_add(spatial_segment_count)
                .ok_or(ExactBRepGraphError::ResourceLimit)?;
            if segment_count > MAX_EXACT_BREP_GRAPH_SEGMENTS {
                return Err(ExactBRepGraphError::ResourceLimit);
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
                || !valid_operation_profiles(
                    &node.operation,
                    &self.profiles,
                    self.tolerance.linear_mm(),
                )
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
                        | EXACT_BREP_GRAPH_SCHEMA_V13
                        | EXACT_BREP_GRAPH_SCHEMA_V14
                        | EXACT_BREP_GRAPH_SCHEMA_V15
                        | EXACT_BREP_GRAPH_SCHEMA_V16
                        | EXACT_BREP_GRAPH_SCHEMA_V17
                        | EXACT_BREP_GRAPH_SCHEMA_V18
                        | EXACT_BREP_GRAPH_SCHEMA_V19
                        | EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v9(&node.operation, &self.profiles))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V10
                        | EXACT_BREP_GRAPH_SCHEMA_V11
                        | EXACT_BREP_GRAPH_SCHEMA_V12
                        | EXACT_BREP_GRAPH_SCHEMA_V13
                        | EXACT_BREP_GRAPH_SCHEMA_V14
                        | EXACT_BREP_GRAPH_SCHEMA_V15
                        | EXACT_BREP_GRAPH_SCHEMA_V16
                        | EXACT_BREP_GRAPH_SCHEMA_V17
                        | EXACT_BREP_GRAPH_SCHEMA_V18
                        | EXACT_BREP_GRAPH_SCHEMA_V19
                        | EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v10(&node.operation, &self.profiles))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V11
                        | EXACT_BREP_GRAPH_SCHEMA_V12
                        | EXACT_BREP_GRAPH_SCHEMA_V13
                        | EXACT_BREP_GRAPH_SCHEMA_V14
                        | EXACT_BREP_GRAPH_SCHEMA_V15
                        | EXACT_BREP_GRAPH_SCHEMA_V16
                        | EXACT_BREP_GRAPH_SCHEMA_V17
                        | EXACT_BREP_GRAPH_SCHEMA_V18
                        | EXACT_BREP_GRAPH_SCHEMA_V19
                        | EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v11(&node.operation, &self.profiles))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V12
                        | EXACT_BREP_GRAPH_SCHEMA_V13
                        | EXACT_BREP_GRAPH_SCHEMA_V14
                        | EXACT_BREP_GRAPH_SCHEMA_V15
                        | EXACT_BREP_GRAPH_SCHEMA_V16
                        | EXACT_BREP_GRAPH_SCHEMA_V17
                        | EXACT_BREP_GRAPH_SCHEMA_V18
                        | EXACT_BREP_GRAPH_SCHEMA_V19
                        | EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v12(&node.operation))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V14
                        | EXACT_BREP_GRAPH_SCHEMA_V15
                        | EXACT_BREP_GRAPH_SCHEMA_V16
                        | EXACT_BREP_GRAPH_SCHEMA_V17
                        | EXACT_BREP_GRAPH_SCHEMA_V18
                        | EXACT_BREP_GRAPH_SCHEMA_V19
                        | EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v14(&node.operation, &self.profiles))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V15
                        | EXACT_BREP_GRAPH_SCHEMA_V16
                        | EXACT_BREP_GRAPH_SCHEMA_V17
                        | EXACT_BREP_GRAPH_SCHEMA_V18
                        | EXACT_BREP_GRAPH_SCHEMA_V19
                        | EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v15(&node.operation))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V16
                        | EXACT_BREP_GRAPH_SCHEMA_V17
                        | EXACT_BREP_GRAPH_SCHEMA_V18
                        | EXACT_BREP_GRAPH_SCHEMA_V19
                        | EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v16(&node.operation))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V17
                        | EXACT_BREP_GRAPH_SCHEMA_V18
                        | EXACT_BREP_GRAPH_SCHEMA_V19
                        | EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v17(&node.operation))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V18
                        | EXACT_BREP_GRAPH_SCHEMA_V19
                        | EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v18(&node.operation))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V19
                        | EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v19(&node.operation))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V20
                        | EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v20(&node.operation))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V21
                        | EXACT_BREP_GRAPH_SCHEMA_V22
                        | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v21(&node.operation))
                || (!matches!(
                    self.schema.as_str(),
                    EXACT_BREP_GRAPH_SCHEMA_V22 | EXACT_BREP_GRAPH_SCHEMA_V23
                ) && operation_requires_v22(&node.operation))
                || (self.schema != EXACT_BREP_GRAPH_SCHEMA_V23
                    && operation_requires_v23(&node.operation))
                || !valid_operation(
                    &node.operation,
                    self.document_id,
                    self.definition_id,
                    &self.nodes[..index],
                    self.tolerance.linear_mm(),
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
            tolerance: self.tolerance,
        };
        let bytes = serde_json::to_vec(&payload)
            .map_err(|error| ExactBRepGraphError::Serialization(error.to_string()))?;
        Ok(digest(&bytes))
    }

    /// Identifies the exact inputs of this body only, so results and face
    /// references stay valid across edits elsewhere in the document.
    fn compute_canonical_input_digest(&self) -> String {
        digest(
            format!(
                "{}:{}:{}:{}:{}",
                self.schema,
                self.document_id,
                self.definition_id,
                self.producer_feature_id,
                self.graph_digest,
            )
            .as_bytes(),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactBRepGraphError {
    DefinitionNotFound(DefinitionId),
    FeatureNotFound(FeatureId),
    UnsupportedFeature(FeatureId),
    UnsupportedProfile(FeatureId),
    /// The feature's profile sketch does not solve.
    UnsolvedProfile(FeatureId, SketchError),
    SuppressedFeature(FeatureId),
    DependencyCycle(FeatureId),
    InvalidDependencyGraph,
    DependencyGraph(Box<CanonicalError>),
    InvalidParameter,
    InvalidTopologySelector,
    InvalidTopologyReference(TopologicalReferenceError),
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
            Self::InvalidTopologyReference(error) => {
                write!(formatter, "topology selector reference is invalid: {error}")
            }
            Self::DependencyGraph(error) => {
                write!(formatter, "feature dependency graph is invalid: {error}")
            }
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
            Self::UnsolvedProfile(id, error) => {
                write!(
                    formatter,
                    "feature {} profile sketch does not solve: {error}",
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

impl std::error::Error for ExactBRepGraphError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTopologyReference(error) => Some(error),
            Self::DependencyGraph(error) => Some(error.as_ref()),
            Self::UnsolvedProfile(_, error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_tolerance::limits;

    fn loft_operation(section_count: usize) -> ExactBRepOperation {
        ExactBRepOperation::Loft {
            sections: (0..section_count)
                .map(|index| ExactBRepLoftSection {
                    profile: ExactBRepProfileId(index as u32 + 1),
                    elevation_bits: (index as f64).to_bits(),
                })
                .collect(),
            guide: None,
            continuity: ExactBRepLoftContinuity::Position,
        }
    }

    #[test]
    fn inward_circle_offset_rejects_source_outside_coordinate_envelope() {
        let profile = ExactBRepProfile {
            id: ExactBRepProfileId(0),
            source_feature_id: 1,
            region_id: Some(1),
            frame_bits: identity_frame(),
            segment_entity_ids: Vec::new(),
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
            segment_entity_ids: Vec::new(),
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
        let tolerance_mm = crate::tolerance::DEFAULT_LINEAR_TOLERANCE_MM;
        assert_eq!(MAX_EXACT_BREP_LOFT_SECTIONS, 16);
        assert!(valid_operation(
            &loft_operation(MAX_EXACT_BREP_LOFT_SECTIONS),
            1,
            1,
            &[],
            tolerance_mm,
        ));
        assert!(!valid_operation(
            &loft_operation(MAX_EXACT_BREP_LOFT_SECTIONS + 1),
            1,
            1,
            &[],
            tolerance_mm,
        ));
    }

    #[test]
    fn loft_control_point_limit_matches_the_exact_backend() {
        let profile = |id, point_count| ExactBRepProfile {
            id: ExactBRepProfileId(id),
            source_feature_id: id as u64 + 1,
            region_id: None,
            frame_bits: identity_frame(),
            segment_entity_ids: Vec::new(),
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
            guide: None,
            continuity: ExactBRepLoftContinuity::Position,
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
            tolerance: Default::default(),
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
        let tolerance_mm = crate::tolerance::DEFAULT_LINEAR_TOLERANCE_MM;
        assert_eq!(limits::PATH_SEGMENTS, 64);
        assert_eq!(limits::REGION_HOLES, 64);
        assert_eq!(limits::REGION_SEGMENTS, 4_096);

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
            validate_geometry(&geometry_with_holes(limits::REGION_HOLES), tolerance_mm),
            Ok(limits::REGION_HOLES + 1),
        );
        assert_eq!(
            validate_geometry(&geometry_with_holes(limits::REGION_HOLES + 1), tolerance_mm),
            Err(ExactBRepGraphError::ResourceLimit),
        );

        assert_eq!(
            validate_geometry(
                &loop_geometry(&boundary_loop([0.0, 0.0], 100.0, limits::PATH_SEGMENTS + 1)),
                tolerance_mm
            ),
            Err(ExactBRepGraphError::ResourceLimit),
        );

        let outer = || boundary_loop([0.0, 0.0], 100.0, limits::PATH_SEGMENTS);
        let boundary_holes = || {
            (0..63)
                .map(|index| {
                    let center = [
                        (index % 9) as f64 * 10.0 - 40.0,
                        (index / 9) as f64 * 10.0 - 30.0,
                    ];
                    boundary_loop(center, 2.0, limits::PATH_SEGMENTS)
                })
                .collect::<Vec<_>>()
        };
        let accepted = ExactBRepPlanarGeometry::Region {
            outer: outer(),
            holes: boundary_holes(),
        };
        assert_eq!(
            validate_geometry(&accepted, tolerance_mm),
            Ok(limits::REGION_SEGMENTS),
        );
        let mut over_limit_holes = boundary_holes();
        over_limit_holes.push(circle_loop([0.0, 45.0], 2.0));
        let over_limit = ExactBRepPlanarGeometry::Region {
            outer: outer(),
            holes: over_limit_holes,
        };
        assert_eq!(
            validate_geometry(&over_limit, tolerance_mm),
            Err(ExactBRepGraphError::ResourceLimit),
        );
    }

    #[test]
    fn sweep_path_contract_matches_the_current_worker() {
        let tolerance_mm = crate::tolerance::DEFAULT_LINEAR_TOLERANCE_MM;
        let profile = |id, geometry| ExactBRepProfile {
            id: ExactBRepProfileId(id),
            source_feature_id: id as u64 + 1,
            region_id: None,
            frame_bits: identity_frame(),
            segment_entity_ids: Vec::new(),
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
        assert!(valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        for length in [
            MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM,
            MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM,
        ] {
            profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
                closed: false,
                segments: vec![line([0.0, 0.0], [length, 0.0])],
            };
            assert!(valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        }
        for length in [
            MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM - 0.001,
            MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM + 0.001,
        ] {
            profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
                closed: false,
                segments: vec![line([0.0, 0.0], [length, 0.0])],
            };
            assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        }
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![line([MAX_COORDINATE_MM, 0.0], [MAX_COORDINATE_MM, 100.0])],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));

        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![line([0.0, 0.0], [100.0, 0.0])],
        };
        profiles[0].frame_bits[0] = 10.0_f64.to_bits();
        assert!(valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[0].frame_bits[0] = MAX_COORDINATE_MM.to_bits();
        assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[0].frame_bits = identity_frame();

        profiles[0].geometry = ExactBRepPlanarGeometry::Spline {
            control_point_bits: vec![
                [0.0f64.to_bits(), 0.0f64.to_bits()],
                [1.0f64.to_bits(), 0.0f64.to_bits()],
                [1.0f64.to_bits(), 1.0f64.to_bits()],
                [0.0f64.to_bits(), 1.0f64.to_bits()],
            ],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[0].geometry = ExactBRepPlanarGeometry::Circle {
            center_bits: [0.0f64.to_bits(), 0.0f64.to_bits()],
            radius_bits: 2.0f64.to_bits(),
        };

        profiles[1].geometry = ExactBRepPlanarGeometry::Circle {
            center_bits: [0.0f64.to_bits(), 0.0f64.to_bits()],
            radius_bits: 10.0f64.to_bits(),
        };
        assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![
                line([0.0, 0.0], [50.0, 0.0]),
                arc([50.0, 0.0], [75.0, 25.0], [50.0, 25.0]),
            ],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[0].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments: vec![
                line([-2.0, -1.0], [2.0, -1.0]),
                line([2.0, -1.0], [2.0, 1.0]),
                line([2.0, 1.0], [-2.0, 1.0]),
                line([-2.0, 1.0], [-2.0, -1.0]),
            ],
        };
        assert!(valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![line([0.0, 0.0], [100.0, 0.0])],
        };
        profiles[0].region_id = Some(1);
        assert!(sweep_profile_bounds(&profiles[0], &profiles[1], tolerance_mm).is_ok());
        assert!(valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[0].region_id = None;
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![
                line([0.0, 0.0], [50.0, 0.0]),
                arc([50.0, 0.0], [75.0, 25.0], [50.0, 25.0]),
            ],
        };
        profiles[1].frame_bits[0] = 1.0_f64.to_bits();
        assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[1].frame_bits = identity_frame();

        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![line([0.0, 0.0], [5.0, 0.0]), line([5.0, 0.0], [10.0, 0.0])],
        };
        assert!(valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![
                line([0.0, 0.0], [tolerance_mm / 2.0, 0.0]),
                line([tolerance_mm / 2.0, 0.0], [10.0, 0.0]),
            ],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));

        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![line([0.0, 0.0], [5.0, 0.0]), line([5.0, 0.0], [5.0, 5.0])],
        };
        assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: vec![
                line([0.0, 0.0], [5.0, 0.0]),
                line([5.0, 0.0], [10.0, 0.0]),
                line([10.0, 0.0], [15.0, 0.0]),
            ],
        };
        assert!(valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: (0..limits::PATH_SEGMENTS)
                .map(|index| line([index as f64, 0.0], [index as f64 + 1.0, 0.0]))
                .collect(),
        };
        assert!(valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: (0..limits::PATH_SEGMENTS)
                .map(|index| cubic(index as f64 * 2.0))
                .collect(),
        };
        assert!(valid_operation_profiles(&sweep, &profiles, tolerance_mm));
        profiles[1].geometry = ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments: (0..=limits::PATH_SEGMENTS)
                .map(|index| line([index as f64, 0.0], [index as f64 + 1.0, 0.0]))
                .collect(),
        };
        assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));
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
        assert!(!valid_operation_profiles(&sweep, &profiles, tolerance_mm));
    }
}
