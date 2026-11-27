use crate::document::{BodyKind, DefinitionId, FeatureId, Snapshot};
use crate::exact_brep_graph::{ExactBRepGraph, ExactBRepGraphError};
use crate::tolerance::{DEFAULT_LINEAR_TOLERANCE_MM, MAX_COORDINATE_MM, ROUNDING};
use ketchup_geometry::linalg::{cross, dot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write;

use crate::cam_gcode::CONTROLLER_NEUTRAL_JSON;
pub use crate::cam_gcode::{
    CAM_GCODE_DIALECT_SCHEMA_V1, CamDecimalPoint, CamGCodeDialect, CamGCodeDialectError,
    CamLineNumbers,
};
pub use crate::cam_region::{CamContourSide, CamHole, CamRegion2d};

pub const CAM_PLAN_SCHEMA_V1: &str = "ketchup.cam-plan.v1";
pub const CAM_TOOLPATH_SCHEMA_V1: &str = "ketchup.cam-toolpath.v1";
pub const CAM_SIMULATION_SCHEMA_V1: &str = "ketchup.cam-simulation.v1";
pub const CAM_POSTPROCESSOR_SCHEMA_V1: &str = "ketchup.cam-postprocessor.v1";
const MAX_CAM_POSTPROCESSOR_BYTES: usize = 16 * 1024 * 1024;
const MAX_RPM: u32 = 200_000;
const MAX_FEED_MM_PER_MIN: f64 = 1.0e6;
const MAX_CAM_OPERATIONS: usize = 1_024;
/// Most motions one toolpath holds.
pub(crate) const MAX_CAM_MOTIONS: usize = 100_000;
const MAX_PATH_SEGMENTS: usize = 4_096;
const MAX_DRILL_POINTS: usize = 4_096;
const GEOMETRY_TOLERANCE_MM: f64 = ROUNDING;

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct CamPlanId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CamUnits {
    Millimetres,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CamToolKind {
    FlatEndMill,
    BallEndMill,
    Drill,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CamWorkOffset {
    G54,
    G55,
    G56,
    G57,
    G58,
    G59,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CamStock {
    pub minimum_mm: [f64; 3],
    pub maximum_mm: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CamTool {
    pub number: u32,
    pub kind: CamToolKind,
    pub diameter_mm: f64,
    pub flute_length_mm: f64,
    pub overall_length_mm: f64,
    pub holder_diameter_mm: f64,
    pub holder_length_mm: f64,
    pub spindle_rpm: u32,
    pub feed_mm_per_min: f64,
    pub plunge_mm_per_min: f64,
    /// Included angle of a drill's point; `None` is the standard
    /// [`STANDARD_DRILL_POINT_ANGLE_DEG`], 180 a flat-bottom drill. Only a drill has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point_angle_deg: Option<f64>,
}

/// The point angle of a standard twist drill (DIN 338).
pub const STANDARD_DRILL_POINT_ANGLE_DEG: f64 = 118.0;
/// Narrowest and widest point angle a drill may have.
const DRILL_POINT_ANGLE_RANGE_DEG: std::ops::RangeInclusive<f64> = 60.0..=180.0;
/// How far a drill's full diameter passes the exit of a through hole, so the hole
/// breaks out clean instead of leaving a burr ring.
pub const DRILL_BREAKTHROUGH_MM: f64 = 1.0;
/// How far above the stock's top, or above the bottom the previous step drilled,
/// each drilling step ends its rapid descent; it is fed only from there, so a deep
/// hole is not fed again through the bore already drilled (review 2026-11-20 ac P3).
pub const DRILL_APPROACH_MM: f64 = 1.0;

impl CamTool {
    /// How far the cutting end reaches below the start of its full diameter: the
    /// cone of a drill, the half sphere of a ball end mill, none for a flat end mill.
    #[must_use]
    pub fn tip_length_mm(&self) -> f64 {
        let radius = self.diameter_mm * 0.5;
        match self.kind {
            CamToolKind::FlatEndMill => 0.0,
            CamToolKind::BallEndMill => radius,
            CamToolKind::Drill => {
                let angle = self
                    .point_angle_deg
                    .unwrap_or(STANDARD_DRILL_POINT_ANGLE_DEG);
                if angle >= 180.0 {
                    0.0
                } else {
                    radius / (angle.to_radians() * 0.5).tan()
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CamSetup {
    pub work_offset: CamWorkOffset,
    pub origin_mm: [f64; 3],
    pub x_axis: [f64; 3],
    pub y_axis: [f64; 3],
    pub safe_height_mm: f64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CamCutParameters {
    pub maximum_stepdown_mm: f64,
    pub stepover_ratio: f64,
    pub radial_allowance_mm: f64,
    pub axial_allowance_mm: f64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CamTarget {
    pub definition_id: DefinitionId,
    pub feature_id: FeatureId,
    pub exact_graph_digest: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CamPlan {
    id: CamPlanId,
    name: String,
    units: CamUnits,
    target: CamTarget,
    stock: CamStock,
    tool: CamTool,
    setup: CamSetup,
    cut_parameters: CamCutParameters,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamPath2d {
    pub start_mm: [f64; 2],
    pub segments: Vec<CamPathSegment2d>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CamPathSegment2d {
    Line {
        to_mm: [f64; 2],
    },
    Arc {
        to_mm: [f64; 2],
        center_mm: [f64; 2],
        clockwise: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CamOperation {
    Face {
        id: u64,
        minimum_mm: [f64; 2],
        maximum_mm: [f64; 2],
        target_z_mm: f64,
    },
    Pocket {
        id: u64,
        minimum_mm: [f64; 2],
        maximum_mm: [f64; 2],
        top_z_mm: f64,
        bottom_z_mm: f64,
    },
    Contour {
        id: u64,
        center_path: CamPath2d,
        top_z_mm: f64,
        bottom_z_mm: f64,
        applied_radial_allowance_mm: f64,
    },
    Drill {
        id: u64,
        points_mm: Vec<[f64; 2]>,
        top_z_mm: f64,
        /// The floor of a blind hole, where the drill's point stops, or the exit
        /// of a through hole, which its full diameter passes by
        /// [`DRILL_BREAKTHROUGH_MM`].
        bottom_z_mm: f64,
        through: bool,
    },
    /// Clears `region` down to `bottom_z_mm`, keeping the cutter one radius off
    /// its outer boundary and its islands.
    RegionPocket {
        id: u64,
        region: CamRegion2d,
        top_z_mm: f64,
        bottom_z_mm: f64,
    },
    /// Runs the cutter along `region`'s boundary on `side`, offset by its radius.
    RegionContour {
        id: u64,
        region: CamRegion2d,
        side: CamContourSide,
        top_z_mm: f64,
        bottom_z_mm: f64,
    },
    /// Clears the pocket whose floor is planar face `face_ordinal` of the
    /// target, from the stock top down to the floor.
    FacePocket { id: u64, face_ordinal: u32 },
    /// Runs the cutter on `side` of planar face `face_ordinal`'s boundary,
    /// from the stock top down to the face.
    FaceContour {
        id: u64,
        face_ordinal: u32,
        side: CamContourSide,
    },
    /// Drills the hole whose wall is cylindrical face `face_ordinal`.
    FaceDrill { id: u64, face_ordinal: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamOperationKind {
    Face,
    Pocket,
    Contour,
    Drill,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamMotionKind {
    Rapid,
    Plunge,
    Cut,
    Retract,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CamMotionPath {
    Line {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
    },
    Arc {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
        center_mm: [f64; 3],
        clockwise: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamMotion {
    pub kind: CamMotionKind,
    pub path: CamMotionPath,
    pub feed_mm_per_min: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamToolpathOperation {
    pub id: u64,
    pub kind: CamOperationKind,
    pub first_motion: usize,
    pub motion_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamToolpath {
    pub schema: &'static str,
    pub plan_id: CamPlanId,
    pub plan_digest: String,
    pub target_exact_graph_digest: String,
    pub operations: Vec<CamToolpathOperation>,
    pub motions: Vec<CamMotion>,
    pub toolpath_digest: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamFixture {
    pub id: u64,
    pub minimum_mm: [f64; 3],
    pub maximum_mm: [f64; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamCollisionParticipant {
    Cutter,
    Holder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamCollisionTarget {
    Stock,
    Fixture(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamCollisionEvidence {
    pub motion_index: usize,
    pub motion_kind: CamMotionKind,
    pub participant: CamCollisionParticipant,
    pub target: CamCollisionTarget,
    pub common_volume_mm3: f64,
    pub contact_area_mm2: f64,
    pub distance_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamSimulationEvidence {
    pub schema: String,
    pub plan_digest: String,
    pub toolpath_digest: String,
    pub target_exact_graph_digest: String,
    pub motion_count: usize,
    pub cutting_motion_count: usize,
    pub fixture_count: usize,
    pub stock_before_mm3: f64,
    pub stock_after_mm3: f64,
    pub removed_stock_mm3: f64,
    pub residual_stock_mm3: f64,
    pub gouge_mm3: f64,
    pub collisions: Vec<CamCollisionEvidence>,
    pub backend: String,
    pub tolerance: String,
    pub result_fingerprint: String,
}

impl CamSimulationEvidence {
    #[must_use]
    pub fn stable_fingerprint(&self) -> String {
        let mut canonical = format!(
            "{}|{}|{}|{}|{}|{}|{}|{:016x}|{:016x}|{:016x}|{:016x}|{:016x}|{}|{}|{}",
            self.schema,
            self.plan_digest,
            self.toolpath_digest,
            self.target_exact_graph_digest,
            self.motion_count,
            self.cutting_motion_count,
            self.fixture_count,
            self.stock_before_mm3.to_bits(),
            self.stock_after_mm3.to_bits(),
            self.removed_stock_mm3.to_bits(),
            self.residual_stock_mm3.to_bits(),
            self.gouge_mm3.to_bits(),
            self.backend,
            self.tolerance,
            self.collisions.len(),
        );
        for collision in &self.collisions {
            let motion_kind = match collision.motion_kind {
                CamMotionKind::Rapid => 0,
                CamMotionKind::Plunge => 1,
                CamMotionKind::Cut => 2,
                CamMotionKind::Retract => 3,
            };
            let participant = match collision.participant {
                CamCollisionParticipant::Cutter => 0,
                CamCollisionParticipant::Holder => 1,
            };
            let (target, fixture_id) = match collision.target {
                CamCollisionTarget::Stock => (0, 0),
                CamCollisionTarget::Fixture(id) => (1, id),
            };
            write!(
                canonical,
                "|{}|{}|{}|{}|{}|{:016x}|{:016x}|{:016x}",
                collision.motion_index,
                motion_kind,
                participant,
                target,
                fixture_id,
                collision.common_volume_mm3.to_bits(),
                collision.contact_area_mm2.to_bits(),
                collision.distance_mm.to_bits(),
            )
            .expect("writing to a String cannot fail");
        }
        content_digest(canonical.as_bytes())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CamPostprocessorDialect {
    /// A controller's G-code, from the library or from the caller's own description.
    GCode(std::sync::Arc<CamGCodeDialect>),
    /// The program as JSON, for a controller interface that reads it.
    ControllerNeutralJson,
}

impl CamPostprocessorDialect {
    /// The library dialect `id`: a G-code dialect or `controller_neutral_json`.
    pub fn library(id: &str) -> Result<Self, CamGCodeDialectError> {
        if id == CONTROLLER_NEUTRAL_JSON {
            return Ok(Self::ControllerNeutralJson);
        }
        CamGCodeDialect::library()?
            .into_iter()
            .find(|dialect| dialect.id() == id)
            .map(|dialect| Self::GCode(dialect.into()))
            .ok_or_else(|| CamGCodeDialectError::UnknownLibraryDialect { id: id.to_owned() })
    }

    /// The ids `library` accepts.
    pub fn library_ids() -> Result<Vec<String>, CamGCodeDialectError> {
        let mut ids = CamGCodeDialect::library()?
            .iter()
            .map(|dialect| dialect.id().to_owned())
            .collect::<Vec<_>>();
        ids.push(CONTROLLER_NEUTRAL_JSON.to_owned());
        Ok(ids)
    }

    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::GCode(dialect) => dialect.id(),
            Self::ControllerNeutralJson => CONTROLLER_NEUTRAL_JSON,
        }
    }

    /// The step lengths and feeds are written in.
    #[must_use]
    pub fn resolution_mm(&self) -> f64 {
        1.0 / self.scale()
    }

    fn scale(&self) -> f64 {
        match self {
            Self::GCode(dialect) => dialect.scale(),
            Self::ControllerNeutralJson => 1.0 / ROUNDING,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CamPostprocessedMotionKind {
    Rapid,
    Plunge,
    Cut,
    Retract,
    ArcClockwise,
    ArcCounterclockwise,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CamPostprocessedMotion {
    pub kind: CamPostprocessedMotionKind,
    pub end_mm: [f64; 3],
    pub center_offset_mm: Option<[f64; 2]>,
    pub feed_mm_per_min: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CamPostprocessedProgram {
    pub schema: String,
    pub plan_digest: String,
    pub toolpath_digest: String,
    pub simulation_fingerprint: String,
    pub units: String,
    pub work_offset: String,
    pub tool_number: u32,
    pub spindle_rpm: u32,
    pub cutting_feed_mm_per_min: f64,
    pub plunge_feed_mm_per_min: f64,
    pub safe_retract_z_mm: f64,
    pub motions: Vec<CamPostprocessedMotion>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamPostprocessorOutput {
    pub schema: &'static str,
    pub dialect: CamPostprocessorDialect,
    pub content: Vec<u8>,
    pub content_digest: String,
    pub program: CamPostprocessedProgram,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CamPostprocessorError {
    Plan(CamPlannerError),
    UnsafeSimulation,
    UnsafeFinalRetract,
    ArtifactTooLarge,
    /// The artifact is malformed; the text names what was wrong.
    Parse(String),
    DigestMismatch,
    RoundtripMismatch,
    /// An arc the dialect's grid cannot write so that a controller runs it as planned
    /// (R1120ac-CAM-900): the operation it is in, its index among the toolpath's motions and
    /// where it starts ([`place_mm`]).
    UnwritableArc {
        operation: u64,
        motion: usize,
        start: String,
    },
}

/// Where `xy` is, as an error names it: `X 12.345 Y -6.000 mm`.
pub fn place_mm(xy: [f64; 2]) -> String {
    let [x, y] = xy;
    format!("X {x:.3} Y {y:.3} mm")
}

impl std::fmt::Display for CamPostprocessorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => write!(formatter, "{error}"),
            Self::UnsafeSimulation => formatter.write_str(
                "CAM postprocessing requires current, complete, collision-free and gouge-free simulation evidence",
            ),
            Self::UnsafeFinalRetract => formatter.write_str(
                "CAM postprocessing requires a final retract to the configured safe height",
            ),
            Self::ArtifactTooLarge => {
                formatter.write_str("CAM postprocessor artifact exceeds the bounded size")
            }
            Self::Parse(reason) => {
                write!(formatter, "CAM postprocessor artifact is malformed: {reason}")
            }
            Self::DigestMismatch => {
                formatter.write_str("CAM postprocessor artifact digest does not match its content")
            }
            Self::RoundtripMismatch => formatter.write_str(
                "CAM postprocessor artifact does not roundtrip to the canonical motion program",
            ),
            Self::UnwritableArc {
                operation,
                motion,
                start,
            } => write!(
                formatter,
                "the arc of operation {operation} (motion {motion}, from {}) does not survive rounding to the step of the dialect: a controller would run it as a different arc or a full circle; choose a dialect with more decimals or a larger corner radius",
                start
            ),
        }
    }
}

impl std::error::Error for CamPostprocessorError {}

impl From<CamPlannerError> for CamPostprocessorError {
    fn from(value: CamPlannerError) -> Self {
        Self::Plan(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CamPlannerError {
    Plan(CamError),
    InvalidOperations,
    GeometryOutsideTarget,
    ToolOutsideStock,
    UnsafeRapidHeight,
    CuttingDepthExceedsFlute,
    UnsupportedTool,
    UnsupportedRegion,
    UnsupportedHole,
    /// A blind hole's floor is the drill's point; this drill's is not the floor's: the
    /// operation and the hole's centre ([`place_mm`]).
    DrillPointDoesNotFitFloor {
        operation: u64,
        center: String,
    },
    UnresolvedFace,
    StaleToolpath,
}

impl std::fmt::Display for CamPlannerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => write!(formatter, "{error}"),
            Self::InvalidOperations => formatter.write_str(
                "CAM operations are empty, malformed, duplicated, or exceed bounded limits",
            ),
            Self::GeometryOutsideTarget => {
                formatter.write_str("CAM operation geometry lies outside the exact target bounds")
            }
            Self::ToolOutsideStock => {
                formatter.write_str("CAM cutter motion lies outside stock bounds")
            }
            Self::UnsafeRapidHeight => {
                formatter.write_str("CAM safe height is not strictly above stock")
            }
            Self::CuttingDepthExceedsFlute => {
                formatter.write_str("CAM cutting depth exceeds the tool flute length")
            }
            Self::UnsupportedTool => {
                formatter.write_str("the selected CAM tool cannot execute this operation")
            }
            Self::UnsupportedRegion => formatter.write_str(
                "CAM region is not one planar face bounded by lines and circular arcs, \
                 or the cutter does not fit in it",
            ),
            Self::UnsupportedHole => formatter.write_str(
                "CAM hole is not a straight round hole along the setup Z axis, \
                 or the drill is not its diameter",
            ),
            Self::DrillPointDoesNotFitFloor { operation, center } => write!(
                formatter,
                "the drill's point does not make the floor of the blind hole of operation \
                 {operation} at {}: a flat floor needs a flat-bottom (180 degree) drill, a \
                 pointed floor a drill of its point angle; or model the floor the drill leaves",
                center
            ),
            Self::UnresolvedFace => formatter.write_str(
                "CAM operation names a face that was not read from the evaluated target",
            ),
            Self::StaleToolpath => formatter.write_str(
                "CAM toolpath is malformed or does not match the current plan and exact target",
            ),
        }
    }
}

impl std::error::Error for CamPlannerError {}

impl From<CamError> for CamPlannerError {
    fn from(value: CamError) -> Self {
        Self::Plan(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamPlanHealth {
    Current,
    TargetMissing,
    WrongBodyKind,
    StaleTarget,
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CamError {
    InvalidPlan,
    TargetMissing,
    /// The target's exact B-Rep graph could not be built or bounded.
    TargetGraph(ExactBRepGraphError),
    WrongBodyKind,
    StaleTarget,
    StockDoesNotContainTarget,
}

impl std::fmt::Display for CamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPlan => "CAM stock, tool, setup, or cutting parameters are invalid",
            Self::TargetMissing => "CAM target is missing or has no exact bounded result",
            Self::TargetGraph(error) => {
                return write!(formatter, "CAM target has no exact bounded result: {error}");
            }
            Self::WrongBodyKind => "CAM target must be a solid body",
            Self::StaleTarget => "CAM target exact graph changed since the plan was created",
            Self::StockDoesNotContainTarget => "CAM stock does not contain the exact target bounds",
        })
    }
}

impl std::error::Error for CamError {}

impl CamPlan {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        id: CamPlanId,
        name: String,
        units: CamUnits,
        target: CamTarget,
        stock: CamStock,
        tool: CamTool,
        setup: CamSetup,
        cut_parameters: CamCutParameters,
    ) -> Self {
        Self {
            id,
            name,
            units,
            target,
            stock,
            tool,
            setup,
            cut_parameters,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        snapshot: &Snapshot,
        id: CamPlanId,
        name: impl Into<String>,
        stock: CamStock,
        tool: CamTool,
        setup: CamSetup,
        cut_parameters: CamCutParameters,
        definition_id: DefinitionId,
        feature_id: FeatureId,
    ) -> Result<Self, CamError> {
        let graph = target_graph(snapshot, definition_id, feature_id)?;
        let plan = Self {
            id,
            name: name.into(),
            units: CamUnits::Millimetres,
            target: CamTarget {
                definition_id,
                feature_id,
                exact_graph_digest: graph.graph_digest,
            },
            stock,
            tool,
            setup,
            cut_parameters,
        };
        plan.validate(snapshot)?;
        Ok(plan)
    }

    #[must_use]
    pub const fn id(&self) -> CamPlanId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn units(&self) -> CamUnits {
        self.units
    }

    #[must_use]
    pub const fn target(&self) -> &CamTarget {
        &self.target
    }

    #[must_use]
    pub const fn stock(&self) -> &CamStock {
        &self.stock
    }

    #[must_use]
    pub const fn tool(&self) -> &CamTool {
        &self.tool
    }

    #[must_use]
    pub const fn setup(&self) -> &CamSetup {
        &self.setup
    }

    #[must_use]
    pub const fn cut_parameters(&self) -> &CamCutParameters {
        &self.cut_parameters
    }

    #[must_use]
    pub fn stable_digest(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(CAM_PLAN_SCHEMA_V1.as_bytes());
        digest.update(self.id.0.to_le_bytes());
        digest_bytes(&mut digest, self.name.as_bytes());
        digest.update([0]);
        digest.update(self.target.definition_id.0.to_le_bytes());
        digest.update(self.target.feature_id.0.to_le_bytes());
        digest_bytes(&mut digest, self.target.exact_graph_digest.as_bytes());
        for value in self
            .stock
            .minimum_mm
            .into_iter()
            .chain(self.stock.maximum_mm)
        {
            digest_f64(&mut digest, value);
        }
        digest.update(self.tool.number.to_le_bytes());
        digest.update([match self.tool.kind {
            CamToolKind::FlatEndMill => 0,
            CamToolKind::BallEndMill => 1,
            CamToolKind::Drill => 2,
        }]);
        for value in [
            self.tool.diameter_mm,
            self.tool.flute_length_mm,
            self.tool.overall_length_mm,
            self.tool.holder_diameter_mm,
            self.tool.holder_length_mm,
        ] {
            digest_f64(&mut digest, value);
        }
        digest.update(self.tool.spindle_rpm.to_le_bytes());
        digest_f64(&mut digest, self.tool.feed_mm_per_min);
        digest_f64(&mut digest, self.tool.plunge_mm_per_min);
        // Absent from plans made before drills had a point angle, so their digest stays.
        if let Some(angle) = self.tool.point_angle_deg {
            digest.update(b"point_angle");
            digest_f64(&mut digest, angle);
        }
        digest.update([match self.setup.work_offset {
            CamWorkOffset::G54 => 0,
            CamWorkOffset::G55 => 1,
            CamWorkOffset::G56 => 2,
            CamWorkOffset::G57 => 3,
            CamWorkOffset::G58 => 4,
            CamWorkOffset::G59 => 5,
        }]);
        for value in self
            .setup
            .origin_mm
            .into_iter()
            .chain(self.setup.x_axis)
            .chain(self.setup.y_axis)
        {
            digest_f64(&mut digest, value);
        }
        digest_f64(&mut digest, self.setup.safe_height_mm);
        for value in [
            self.cut_parameters.maximum_stepdown_mm,
            self.cut_parameters.stepover_ratio,
            self.cut_parameters.radial_allowance_mm,
            self.cut_parameters.axial_allowance_mm,
        ] {
            digest_f64(&mut digest, value);
        }
        hex_digest(digest.finalize())
    }

    #[must_use]
    pub fn health(&self, snapshot: &Snapshot) -> CamPlanHealth {
        match self.validate(snapshot) {
            Ok(()) => CamPlanHealth::Current,
            Err(CamError::TargetMissing | CamError::TargetGraph(_)) => CamPlanHealth::TargetMissing,
            Err(CamError::WrongBodyKind) => CamPlanHealth::WrongBodyKind,
            Err(CamError::StaleTarget) => CamPlanHealth::StaleTarget,
            Err(CamError::InvalidPlan | CamError::StockDoesNotContainTarget) => {
                CamPlanHealth::Invalid
            }
        }
    }

    /// The threads of the plan's part, tapped holes as "M8×1.25 depth 12" and
    /// shaft threads as "M8×1.25 length 20": no CAM operation cuts a thread,
    /// so a program of the plan leaves every one for the shop (review
    /// 2026-11-19 MF-807).
    #[must_use]
    pub fn uncut_threads(&self, snapshot: &Snapshot) -> Vec<String> {
        let of_part = |id: FeatureId| {
            snapshot
                .feature(id)
                .is_some_and(|feature| feature.definition_id() == self.target.definition_id)
        };
        let tapped = snapshot
            .hole_threads()
            .filter(|(id, _)| of_part(*id))
            .map(|(_, thread)| thread.label());
        let shafts = snapshot
            .shaft_threads()
            .filter(|(id, _)| of_part(*id))
            .map(|(_, thread)| {
                format!(
                    "{}×{} length {}",
                    thread.size, thread.pitch_mm, thread.length_mm
                )
            });
        tapped.chain(shafts).collect()
    }

    pub fn default_facing_operation(
        &self,
        snapshot: &Snapshot,
        operation_id: u64,
    ) -> Result<CamOperation, CamPlannerError> {
        self.validate(snapshot)?;
        if operation_id == 0 || self.tool.kind == CamToolKind::Drill {
            return Err(CamPlannerError::InvalidOperations);
        }
        let graph = target_graph(snapshot, self.target.definition_id, self.target.feature_id)?;
        let bounds = setup_bounds(
            graph
                .producer_bounds_mm()
                .map_err(CamError::TargetGraph)?
                .ok_or(CamError::TargetMissing)?,
            &self.setup,
        );
        let radius = self.tool.diameter_mm * 0.5;
        let minimum_mm = [bounds[0][0] + radius, bounds[0][1] + radius];
        let maximum_mm = [bounds[1][0] - radius, bounds[1][1] - radius];
        if !valid_rectangle(minimum_mm, maximum_mm) {
            return Err(CamPlannerError::InvalidOperations);
        }
        Ok(CamOperation::Face {
            id: operation_id,
            minimum_mm,
            maximum_mm,
            target_z_mm: bounds[1][2],
        })
    }

    pub fn validate(&self, snapshot: &Snapshot) -> Result<(), CamError> {
        self.validate_structure()?;
        let graph = target_graph(snapshot, self.target.definition_id, self.target.feature_id)?;
        if graph.graph_digest != self.target.exact_graph_digest {
            return Err(CamError::StaleTarget);
        }
        let bounds = graph
            .producer_bounds_mm()
            .map_err(CamError::TargetGraph)?
            .ok_or(CamError::TargetMissing)?;
        if (0..3).any(|axis| {
            self.stock.minimum_mm[axis] > bounds[0][axis]
                || self.stock.maximum_mm[axis] < bounds[1][axis]
        }) {
            return Err(CamError::StockDoesNotContainTarget);
        }
        Ok(())
    }

    pub(crate) fn validate_structure(&self) -> Result<(), CamError> {
        validate_scalar_data(self)
    }
}

impl CamToolpath {
    pub fn plan(
        snapshot: &Snapshot,
        plan: &CamPlan,
        operations: &[CamOperation],
    ) -> Result<Self, CamPlannerError> {
        plan.validate(snapshot)?;
        if operations.is_empty() || operations.len() > MAX_CAM_OPERATIONS {
            return Err(CamPlannerError::InvalidOperations);
        }
        let mut ids = BTreeSet::new();
        if operations.iter().any(|operation| {
            let id = operation_id(operation);
            id == 0 || !ids.insert(id)
        }) {
            return Err(CamPlannerError::InvalidOperations);
        }

        let graph = target_graph(snapshot, plan.target.definition_id, plan.target.feature_id)?;
        let world_target_bounds = graph
            .producer_bounds_mm()
            .map_err(CamError::TargetGraph)?
            .ok_or(CamError::TargetMissing)?;
        let target_bounds = setup_bounds(world_target_bounds, &plan.setup);
        let stock_bounds =
            setup_bounds([plan.stock.minimum_mm, plan.stock.maximum_mm], &plan.setup);
        if plan.setup.safe_height_mm <= stock_bounds[1][2] + GEOMETRY_TOLERANCE_MM {
            return Err(CamPlannerError::UnsafeRapidHeight);
        }

        let mut builder = MotionBuilder::new(plan.setup.safe_height_mm);
        let mut planned_operations = Vec::with_capacity(operations.len());
        for operation in operations {
            let first_motion = builder.motions.len();
            let kind = match operation {
                CamOperation::Face {
                    minimum_mm,
                    maximum_mm,
                    target_z_mm,
                    ..
                } => {
                    if plan.tool.kind == CamToolKind::Drill {
                        return Err(CamPlannerError::UnsupportedTool);
                    }
                    validate_rectangle(*minimum_mm, *maximum_mm, target_bounds)?;
                    validate_z(*target_z_mm, target_bounds)?;
                    let finished_z = *target_z_mm + plan.cut_parameters.axial_allowance_mm;
                    let stock_top = stock_bounds[1][2];
                    validate_cut_depth(stock_top, finished_z, plan.tool.flute_length_mm)?;
                    let radius = plan.tool.diameter_mm * 0.5;
                    let cutter_min = [minimum_mm[0] - radius, minimum_mm[1] - radius];
                    let cutter_max = [maximum_mm[0] + radius, maximum_mm[1] + radius];
                    validate_tool_rectangle(cutter_min, cutter_max, stock_bounds)?;
                    let lanes = lane_coordinates(
                        cutter_min[1],
                        cutter_max[1],
                        plan.tool.diameter_mm * plan.cut_parameters.stepover_ratio,
                    )?;
                    for depth in depth_passes(
                        stock_top,
                        finished_z,
                        plan.cut_parameters.maximum_stepdown_mm,
                    )? {
                        append_raster(
                            &mut builder,
                            cutter_min[0],
                            cutter_max[0],
                            &lanes,
                            depth,
                            plan,
                            false,
                        )?;
                    }
                    CamOperationKind::Face
                }
                CamOperation::Pocket {
                    minimum_mm,
                    maximum_mm,
                    top_z_mm,
                    bottom_z_mm,
                    ..
                } => {
                    if plan.tool.kind == CamToolKind::Drill {
                        return Err(CamPlannerError::UnsupportedTool);
                    }
                    validate_rectangle(*minimum_mm, *maximum_mm, target_bounds)?;
                    validate_z_range(*top_z_mm, *bottom_z_mm, target_bounds)?;
                    let finished_z = *bottom_z_mm + plan.cut_parameters.axial_allowance_mm;
                    validate_z(finished_z, target_bounds)?;
                    validate_cut_depth(*top_z_mm, finished_z, plan.tool.flute_length_mm)?;
                    let inset =
                        plan.tool.diameter_mm * 0.5 + plan.cut_parameters.radial_allowance_mm;
                    let cutter_min = [minimum_mm[0] + inset, minimum_mm[1] + inset];
                    let cutter_max = [maximum_mm[0] - inset, maximum_mm[1] - inset];
                    if !valid_rectangle(cutter_min, cutter_max) {
                        return Err(CamPlannerError::InvalidOperations);
                    }
                    validate_tool_rectangle(
                        [
                            cutter_min[0] - plan.tool.diameter_mm * 0.5,
                            cutter_min[1] - plan.tool.diameter_mm * 0.5,
                        ],
                        [
                            cutter_max[0] + plan.tool.diameter_mm * 0.5,
                            cutter_max[1] + plan.tool.diameter_mm * 0.5,
                        ],
                        stock_bounds,
                    )?;
                    let lanes = lane_coordinates(
                        cutter_min[1],
                        cutter_max[1],
                        plan.tool.diameter_mm * plan.cut_parameters.stepover_ratio,
                    )?;
                    for depth in depth_passes(
                        *top_z_mm,
                        finished_z,
                        plan.cut_parameters.maximum_stepdown_mm,
                    )? {
                        append_raster(
                            &mut builder,
                            cutter_min[0],
                            cutter_max[0],
                            &lanes,
                            depth,
                            plan,
                            true,
                        )?;
                    }
                    CamOperationKind::Pocket
                }
                CamOperation::Contour {
                    center_path,
                    top_z_mm,
                    bottom_z_mm,
                    applied_radial_allowance_mm,
                    ..
                } => {
                    if plan.tool.kind == CamToolKind::Drill {
                        return Err(CamPlannerError::UnsupportedTool);
                    }
                    if !applied_radial_allowance_mm.is_finite()
                        || (*applied_radial_allowance_mm - plan.cut_parameters.radial_allowance_mm)
                            .abs()
                            > GEOMETRY_TOLERANCE_MM
                    {
                        return Err(CamPlannerError::InvalidOperations);
                    }
                    validate_z_range(*top_z_mm, *bottom_z_mm, target_bounds)?;
                    let finished_z = *bottom_z_mm + plan.cut_parameters.axial_allowance_mm;
                    validate_z(finished_z, target_bounds)?;
                    validate_cut_depth(*top_z_mm, finished_z, plan.tool.flute_length_mm)?;
                    validate_path(
                        center_path,
                        target_bounds,
                        stock_bounds,
                        plan.tool.diameter_mm * 0.5,
                    )?;
                    for depth in depth_passes(
                        *top_z_mm,
                        finished_z,
                        plan.cut_parameters.maximum_stepdown_mm,
                    )? {
                        builder.rapid_to([center_path.start_mm[0], center_path.start_mm[1]])?;
                        builder.line(
                            CamMotionKind::Plunge,
                            [center_path.start_mm[0], center_path.start_mm[1], depth],
                            Some(plan.tool.plunge_mm_per_min),
                        )?;
                        for segment in &center_path.segments {
                            match segment {
                                CamPathSegment2d::Line { to_mm } => builder.line(
                                    CamMotionKind::Cut,
                                    [to_mm[0], to_mm[1], depth],
                                    Some(plan.tool.feed_mm_per_min),
                                )?,
                                CamPathSegment2d::Arc {
                                    to_mm,
                                    center_mm,
                                    clockwise,
                                } => builder.arc(
                                    [to_mm[0], to_mm[1], depth],
                                    [center_mm[0], center_mm[1], depth],
                                    *clockwise,
                                    plan.tool.feed_mm_per_min,
                                )?,
                            }
                        }
                        builder.retract()?;
                    }
                    CamOperationKind::Contour
                }
                CamOperation::Drill {
                    points_mm,
                    top_z_mm,
                    bottom_z_mm,
                    through,
                    ..
                } => {
                    if points_mm.is_empty() || points_mm.len() > MAX_DRILL_POINTS {
                        return Err(CamPlannerError::InvalidOperations);
                    }
                    validate_z_range(*top_z_mm, *bottom_z_mm, target_bounds)?;
                    // A hole is drilled to size in one go: no finishing pass takes an
                    // axial allowance off later. A through hole is drilled until the
                    // full diameter has passed its exit. Only the flute length is
                    // checked here; the simulation checks the room under the exit
                    // against declared fixtures alone (R1124og-CAM-901).
                    let finished_z = if *through {
                        *bottom_z_mm - plan.tool.tip_length_mm() - DRILL_BREAKTHROUGH_MM
                    } else {
                        *bottom_z_mm
                    };
                    validate_cut_depth(*top_z_mm, finished_z, plan.tool.flute_length_mm)?;
                    let radius = plan.tool.diameter_mm * 0.5;
                    for point in points_mm {
                        validate_point(*point, target_bounds)?;
                        validate_tool_rectangle(
                            [point[0] - radius, point[1] - radius],
                            [point[0] + radius, point[1] + radius],
                            stock_bounds,
                        )?;
                    }
                    let depths = depth_passes(
                        *top_z_mm,
                        finished_z,
                        plan.cut_parameters.maximum_stepdown_mm,
                    )?;
                    let approach_z = stock_bounds[1][2] + DRILL_APPROACH_MM;
                    for point in points_mm {
                        let mut drilled_z = f64::INFINITY;
                        for depth in &depths {
                            let feed_from_z = approach_z.min(drilled_z + DRILL_APPROACH_MM);
                            drilled_z = *depth;
                            builder.rapid_to(*point)?;
                            if feed_from_z < builder.safe_z - GEOMETRY_TOLERANCE_MM {
                                builder.line(
                                    CamMotionKind::Rapid,
                                    [point[0], point[1], feed_from_z],
                                    None,
                                )?;
                            }
                            builder.line(
                                CamMotionKind::Plunge,
                                [point[0], point[1], *depth],
                                Some(plan.tool.plunge_mm_per_min),
                            )?;
                            builder.retract()?;
                        }
                    }
                    CamOperationKind::Drill
                }
                CamOperation::RegionPocket {
                    region,
                    top_z_mm,
                    bottom_z_mm,
                    ..
                } => {
                    let paths = crate::cam_region::pocket_paths(
                        region,
                        plan.tool.diameter_mm * 0.5 + plan.cut_parameters.radial_allowance_mm,
                        plan.tool.diameter_mm * plan.cut_parameters.stepover_ratio,
                    )?;
                    let bounds = [target_bounds, stock_bounds];
                    let depths = [*top_z_mm, *bottom_z_mm];
                    append_region_paths(&mut builder, plan, region, &paths, depths, bounds)?;
                    CamOperationKind::Pocket
                }
                CamOperation::RegionContour {
                    region,
                    side,
                    top_z_mm,
                    bottom_z_mm,
                    ..
                } => {
                    let paths = crate::cam_region::contour_paths(
                        region,
                        *side,
                        plan.tool.diameter_mm * 0.5 + plan.cut_parameters.radial_allowance_mm,
                    )?;
                    let bounds = [target_bounds, stock_bounds];
                    let depths = [*top_z_mm, *bottom_z_mm];
                    append_region_paths(&mut builder, plan, region, &paths, depths, bounds)?;
                    CamOperationKind::Contour
                }
                CamOperation::FacePocket { .. }
                | CamOperation::FaceContour { .. }
                | CamOperation::FaceDrill { .. } => return Err(CamPlannerError::UnresolvedFace),
            };
            planned_operations.push(CamToolpathOperation {
                id: operation_id(operation),
                kind,
                first_motion,
                motion_count: builder.motions.len() - first_motion,
            });
        }

        let plan_digest = plan.stable_digest();
        let toolpath_digest = compute_toolpath_digest(
            plan.id,
            &plan_digest,
            &plan.target.exact_graph_digest,
            &planned_operations,
            &builder.motions,
        );
        Ok(Self {
            schema: CAM_TOOLPATH_SCHEMA_V1,
            plan_id: plan.id,
            plan_digest,
            target_exact_graph_digest: plan.target.exact_graph_digest.clone(),
            operations: planned_operations,
            motions: builder.motions,
            toolpath_digest,
        })
    }

    pub fn validate(&self, snapshot: &Snapshot, plan: &CamPlan) -> Result<(), CamPlannerError> {
        plan.validate(snapshot)?;
        if self.schema != CAM_TOOLPATH_SCHEMA_V1
            || self.plan_id != plan.id
            || self.plan_digest != plan.stable_digest()
            || self.target_exact_graph_digest != plan.target.exact_graph_digest
            || self.operations.is_empty()
            || self.operations.len() > MAX_CAM_OPERATIONS
            || self.motions.is_empty()
            || self.motions.len() > MAX_CAM_MOTIONS
        {
            return Err(CamPlannerError::StaleToolpath);
        }
        let mut ids = BTreeSet::new();
        let mut next_motion = 0;
        for operation in &self.operations {
            if operation.id == 0
                || !ids.insert(operation.id)
                || operation.motion_count == 0
                || operation.first_motion != next_motion
            {
                return Err(CamPlannerError::StaleToolpath);
            }
            next_motion = next_motion
                .checked_add(operation.motion_count)
                .ok_or(CamPlannerError::StaleToolpath)?;
            if next_motion > self.motions.len() {
                return Err(CamPlannerError::StaleToolpath);
            }
        }
        if next_motion != self.motions.len() {
            return Err(CamPlannerError::StaleToolpath);
        }
        let mut expected_start = [0.0, 0.0, plan.setup.safe_height_mm];
        for motion in &self.motions {
            let (start, end) = match &motion.path {
                CamMotionPath::Line { start_mm, end_mm } => (*start_mm, *end_mm),
                CamMotionPath::Arc {
                    start_mm,
                    end_mm,
                    center_mm,
                    ..
                } => {
                    if motion.kind != CamMotionKind::Cut
                        || center_mm.iter().any(|value| !value.is_finite())
                        || (start_mm[2] - end_mm[2]).abs() > GEOMETRY_TOLERANCE_MM
                        || (start_mm[2] - center_mm[2]).abs() > GEOMETRY_TOLERANCE_MM
                        || (distance_3d(*start_mm, *center_mm) - distance_3d(*end_mm, *center_mm))
                            .abs()
                            > DEFAULT_LINEAR_TOLERANCE_MM
                    {
                        return Err(CamPlannerError::StaleToolpath);
                    }
                    (*start_mm, *end_mm)
                }
            };
            if start
                .into_iter()
                .chain(end)
                .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
                || distance_3d(start, expected_start) > GEOMETRY_TOLERANCE_MM
            {
                return Err(CamPlannerError::StaleToolpath);
            }
            let valid_feed = match motion.kind {
                CamMotionKind::Rapid | CamMotionKind::Retract => motion.feed_mm_per_min.is_none(),
                CamMotionKind::Plunge => {
                    motion.feed_mm_per_min == Some(plan.tool.plunge_mm_per_min)
                }
                CamMotionKind::Cut => motion.feed_mm_per_min == Some(plan.tool.feed_mm_per_min),
            };
            if !valid_feed {
                return Err(CamPlannerError::StaleToolpath);
            }
            expected_start = end;
        }
        if self.toolpath_digest
            != compute_toolpath_digest(
                self.plan_id,
                &self.plan_digest,
                &self.target_exact_graph_digest,
                &self.operations,
                &self.motions,
            )
        {
            return Err(CamPlannerError::StaleToolpath);
        }
        Ok(())
    }
}

impl CamPostprocessorOutput {
    pub fn generate(
        snapshot: &Snapshot,
        plan: &CamPlan,
        toolpath: &CamToolpath,
        simulation: &CamSimulationEvidence,
        dialect: CamPostprocessorDialect,
    ) -> Result<Self, CamPostprocessorError> {
        toolpath.validate(snapshot, plan)?;
        validate_simulation_for_postprocessing(plan, toolpath, simulation)?;
        let program = canonical_postprocessed_program(plan, toolpath, simulation, dialect.scale())?;
        let content = match &dialect {
            CamPostprocessorDialect::GCode(gcode) => gcode.render(&program)?.into_bytes(),
            CamPostprocessorDialect::ControllerNeutralJson => {
                serde_json::to_vec_pretty(&program).map_err(|error| malformed(error.to_string()))?
            }
        };
        if content.len() > MAX_CAM_POSTPROCESSOR_BYTES {
            return Err(CamPostprocessorError::ArtifactTooLarge);
        }
        let output = Self {
            schema: CAM_POSTPROCESSOR_SCHEMA_V1,
            dialect,
            content_digest: content_digest(&content),
            content,
            program: program.clone(),
        };
        if output.parse()? != program {
            return Err(CamPostprocessorError::RoundtripMismatch);
        }
        Ok(output)
    }

    pub fn parse(&self) -> Result<CamPostprocessedProgram, CamPostprocessorError> {
        if self.schema != CAM_POSTPROCESSOR_SCHEMA_V1
            || self.content.is_empty()
            || self.content.len() > MAX_CAM_POSTPROCESSOR_BYTES
        {
            return Err(malformed(format!(
                "schema must be {CAM_POSTPROCESSOR_SCHEMA_V1} and content 1..={MAX_CAM_POSTPROCESSOR_BYTES} bytes"
            )));
        }
        if self.content_digest != content_digest(&self.content) {
            return Err(CamPostprocessorError::DigestMismatch);
        }
        let mut parsed = match &self.dialect {
            CamPostprocessorDialect::GCode(gcode) => gcode.parse(&self.content)?,
            CamPostprocessorDialect::ControllerNeutralJson => serde_json::from_slice(&self.content)
                .map_err(|error| malformed(error.to_string()))?,
        };
        canonicalize_postprocessed_program(&mut parsed, self.dialect.scale());
        validate_postprocessed_program(&parsed)?;
        Ok(parsed)
    }

    pub fn verify(
        &self,
        snapshot: &Snapshot,
        plan: &CamPlan,
        toolpath: &CamToolpath,
        simulation: &CamSimulationEvidence,
    ) -> Result<(), CamPostprocessorError> {
        toolpath.validate(snapshot, plan)?;
        validate_simulation_for_postprocessing(plan, toolpath, simulation)?;
        let expected =
            canonical_postprocessed_program(plan, toolpath, simulation, self.dialect.scale())?;
        let parsed = self.parse()?;
        if self.program != expected || parsed != expected {
            return Err(CamPostprocessorError::RoundtripMismatch);
        }
        Ok(())
    }
}

fn validate_simulation_for_postprocessing(
    plan: &CamPlan,
    toolpath: &CamToolpath,
    simulation: &CamSimulationEvidence,
) -> Result<(), CamPostprocessorError> {
    let volumes = [
        simulation.stock_before_mm3,
        simulation.stock_after_mm3,
        simulation.removed_stock_mm3,
        simulation.residual_stock_mm3,
        simulation.gouge_mm3,
    ];
    let expected_cutting = toolpath
        .motions
        .iter()
        .filter(|motion| matches!(motion.kind, CamMotionKind::Plunge | CamMotionKind::Cut))
        .count();
    let volume_tolerance = simulation.stock_before_mm3.abs().max(1.0) * ROUNDING;
    if simulation.schema != CAM_SIMULATION_SCHEMA_V1
        || simulation.plan_digest != plan.stable_digest()
        || simulation.plan_digest != toolpath.plan_digest
        || simulation.toolpath_digest != toolpath.toolpath_digest
        || simulation.target_exact_graph_digest != toolpath.target_exact_graph_digest
        || simulation.motion_count != toolpath.motions.len()
        || simulation.cutting_motion_count != expected_cutting
        || simulation.fixture_count > 64
        || volumes
            .into_iter()
            .any(|value| !value.is_finite() || value < 0.0)
        || simulation.stock_after_mm3 > simulation.stock_before_mm3 + volume_tolerance
        || (simulation.stock_before_mm3 - simulation.stock_after_mm3 - simulation.removed_stock_mm3)
            .abs()
            > volume_tolerance
        || simulation.gouge_mm3 > volume_tolerance
        || !simulation.collisions.is_empty()
        || simulation.backend.is_empty()
        || simulation.backend.len() > ketchup_tolerance::limits::TEXT_BYTES
        || simulation.tolerance.is_empty()
        || simulation.tolerance.len() > ketchup_tolerance::limits::TEXT_BYTES
        || !crate::graph::is_sha256_hex(&simulation.result_fingerprint)
        || simulation.result_fingerprint != simulation.stable_fingerprint()
    {
        return Err(CamPostprocessorError::UnsafeSimulation);
    }
    Ok(())
}

fn canonical_postprocessed_program(
    plan: &CamPlan,
    toolpath: &CamToolpath,
    simulation: &CamSimulationEvidence,
    scale: f64,
) -> Result<CamPostprocessedProgram, CamPostprocessorError> {
    let number = |value| canonical_postprocessor_number(value, scale);
    let last = toolpath
        .motions
        .last()
        .ok_or(CamPostprocessorError::UnsafeFinalRetract)?;
    let last_end = motion_end(last);
    if last.kind != CamMotionKind::Retract
        || (last_end[2] - plan.setup.safe_height_mm).abs() > GEOMETRY_TOLERANCE_MM
    {
        return Err(CamPostprocessorError::UnsafeFinalRetract);
    }
    let motions = toolpath
        .motions
        .iter()
        .enumerate()
        .map(|(index, motion)| match &motion.path {
            CamMotionPath::Line { end_mm, .. } => Ok(CamPostprocessedMotion {
                kind: match motion.kind {
                    CamMotionKind::Rapid => CamPostprocessedMotionKind::Rapid,
                    CamMotionKind::Plunge => CamPostprocessedMotionKind::Plunge,
                    CamMotionKind::Cut => CamPostprocessedMotionKind::Cut,
                    CamMotionKind::Retract => CamPostprocessedMotionKind::Retract,
                },
                end_mm: end_mm.map(number),
                center_offset_mm: None,
                feed_mm_per_min: motion.feed_mm_per_min.map(number),
            }),
            CamMotionPath::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                let center_offset_mm = [
                    number(center_mm[0] - start_mm[0]),
                    number(center_mm[1] - start_mm[1]),
                ];
                let written = crate::cam_arc_rounding::WrittenArc {
                    start_mm: start_mm.map(number),
                    end_mm: end_mm.map(number),
                    center_offset_mm,
                    clockwise: *clockwise,
                };
                let unwritable = || CamPostprocessorError::UnwritableArc {
                    operation: toolpath
                        .operations
                        .iter()
                        .find(|operation| {
                            (operation.first_motion
                                ..operation.first_motion + operation.motion_count)
                                .contains(&index)
                        })
                        .map_or(0, |operation| operation.id),
                    motion: index,
                    start: place_mm([start_mm[0], start_mm[1]]),
                };
                let kind = match written
                    .as_planned(start_mm, end_mm, center_mm, scale)
                    .ok_or_else(unwritable)?
                {
                    crate::cam_arc_rounding::Written::Arc if *clockwise => {
                        CamPostprocessedMotionKind::ArcClockwise
                    }
                    crate::cam_arc_rounding::Written::Arc => {
                        CamPostprocessedMotionKind::ArcCounterclockwise
                    }
                    crate::cam_arc_rounding::Written::Line => CamPostprocessedMotionKind::Cut,
                };
                Ok(CamPostprocessedMotion {
                    kind,
                    end_mm: written.end_mm,
                    center_offset_mm: (kind != CamPostprocessedMotionKind::Cut)
                        .then_some(center_offset_mm),
                    feed_mm_per_min: motion.feed_mm_per_min.map(number),
                })
            }
        })
        .collect::<Result<_, CamPostprocessorError>>()?;
    Ok(CamPostprocessedProgram {
        schema: CAM_POSTPROCESSOR_SCHEMA_V1.to_owned(),
        plan_digest: toolpath.plan_digest.clone(),
        toolpath_digest: toolpath.toolpath_digest.clone(),
        simulation_fingerprint: simulation.result_fingerprint.clone(),
        units: "mm".to_owned(),
        work_offset: work_offset_code(plan.setup.work_offset).to_owned(),
        tool_number: plan.tool.number,
        spindle_rpm: plan.tool.spindle_rpm,
        cutting_feed_mm_per_min: number(plan.tool.feed_mm_per_min),
        plunge_feed_mm_per_min: number(plan.tool.plunge_mm_per_min),
        safe_retract_z_mm: number(plan.setup.safe_height_mm),
        motions,
    })
}

fn validate_postprocessed_program(
    program: &CamPostprocessedProgram,
) -> Result<(), CamPostprocessorError> {
    if program.schema != CAM_POSTPROCESSOR_SCHEMA_V1
        || !crate::graph::is_sha256_hex(&program.plan_digest)
        || !crate::graph::is_sha256_hex(&program.toolpath_digest)
        || !crate::graph::is_sha256_hex(&program.simulation_fingerprint)
        || program.units != "mm"
        || !matches!(
            program.work_offset.as_str(),
            "G54" | "G55" | "G56" | "G57" | "G58" | "G59"
        )
        || program.tool_number == 0
        || program.spindle_rpm == 0
        || program.spindle_rpm > MAX_RPM
        || !valid_postprocessor_number(program.cutting_feed_mm_per_min)
        || !valid_postprocessor_number(program.plunge_feed_mm_per_min)
        || program.plunge_feed_mm_per_min > program.cutting_feed_mm_per_min
        || !valid_postprocessor_number(program.safe_retract_z_mm)
        || program.motions.is_empty()
        || program.motions.len() > MAX_CAM_MOTIONS
    {
        return Err(malformed(
            "header values (units, tool, spindle, feeds, retract, motion count) are out of range",
        ));
    }
    for motion in &program.motions {
        if motion
            .end_mm
            .into_iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
        {
            return Err(malformed(
                "motion end point is not finite or exceeds the coordinate limit",
            ));
        }
        let expected_feed = matches!(
            motion.kind,
            CamPostprocessedMotionKind::Plunge
                | CamPostprocessedMotionKind::Cut
                | CamPostprocessedMotionKind::ArcClockwise
                | CamPostprocessedMotionKind::ArcCounterclockwise
        );
        if expected_feed != motion.feed_mm_per_min.is_some()
            || motion
                .feed_mm_per_min
                .is_some_and(|feed| !valid_postprocessor_number(feed))
        {
            return Err(malformed(
                "motion feed must be present exactly for feed motions and valid",
            ));
        }
        let expected_center = matches!(
            motion.kind,
            CamPostprocessedMotionKind::ArcClockwise
                | CamPostprocessedMotionKind::ArcCounterclockwise
        );
        if expected_center != motion.center_offset_mm.is_some()
            || motion.center_offset_mm.is_some_and(|offset| {
                offset
                    .into_iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
            })
        {
            return Err(malformed(
                "arc center offset must be present exactly for arcs and finite",
            ));
        }
    }
    let last = program
        .motions
        .last()
        .ok_or_else(|| malformed("program has no motions"))?;
    if last.kind != CamPostprocessedMotionKind::Retract
        || (last.end_mm[2] - program.safe_retract_z_mm).abs() > GEOMETRY_TOLERANCE_MM
    {
        return Err(CamPostprocessorError::UnsafeFinalRetract);
    }
    Ok(())
}

pub(crate) fn malformed(reason: impl Into<String>) -> CamPostprocessorError {
    CamPostprocessorError::Parse(reason.into())
}

fn canonicalize_postprocessed_program(program: &mut CamPostprocessedProgram, scale: f64) {
    let number = |value| canonical_postprocessor_number(value, scale);
    program.cutting_feed_mm_per_min = number(program.cutting_feed_mm_per_min);
    program.plunge_feed_mm_per_min = number(program.plunge_feed_mm_per_min);
    program.safe_retract_z_mm = number(program.safe_retract_z_mm);
    for motion in &mut program.motions {
        motion.end_mm = motion.end_mm.map(number);
        motion.center_offset_mm = motion.center_offset_mm.map(|offset| offset.map(number));
        motion.feed_mm_per_min = motion.feed_mm_per_min.map(number);
    }
}

/// `value` on the grid of `1 / scale` millimetres, written once so that a program read
/// back lands on the very same numbers.
fn canonical_postprocessor_number(value: f64, scale: f64) -> f64 {
    let rounded = (value * scale).round() / scale;
    if rounded == 0.0 { 0.0 } else { rounded }
}

fn work_offset_code(offset: CamWorkOffset) -> &'static str {
    match offset {
        CamWorkOffset::G54 => "G54",
        CamWorkOffset::G55 => "G55",
        CamWorkOffset::G56 => "G56",
        CamWorkOffset::G57 => "G57",
        CamWorkOffset::G58 => "G58",
        CamWorkOffset::G59 => "G59",
    }
}

fn motion_end(motion: &CamMotion) -> [f64; 3] {
    match motion.path {
        CamMotionPath::Line { end_mm, .. } | CamMotionPath::Arc { end_mm, .. } => end_mm,
    }
}

fn valid_postprocessor_number(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= MAX_FEED_MM_PER_MIN
}

fn content_digest(content: &[u8]) -> String {
    hex_digest(Sha256::digest(content))
}

impl CamFixture {
    pub fn validate(&self) -> Result<(), CamPlannerError> {
        if self.id == 0
            || self
                .minimum_mm
                .into_iter()
                .chain(self.maximum_mm)
                .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
            || (0..3).any(|axis| self.minimum_mm[axis] >= self.maximum_mm[axis])
        {
            Err(CamPlannerError::InvalidOperations)
        } else {
            Ok(())
        }
    }
}

struct MotionBuilder {
    safe_z: f64,
    current: [f64; 3],
    motions: Vec<CamMotion>,
}

impl MotionBuilder {
    fn new(safe_z: f64) -> Self {
        Self {
            safe_z,
            current: [0.0, 0.0, safe_z],
            motions: Vec::new(),
        }
    }

    fn rapid_to(&mut self, point: [f64; 2]) -> Result<(), CamPlannerError> {
        if (self.current[2] - self.safe_z).abs() > GEOMETRY_TOLERANCE_MM {
            return Err(CamPlannerError::UnsafeRapidHeight);
        }
        self.line(
            CamMotionKind::Rapid,
            [point[0], point[1], self.safe_z],
            None,
        )
    }

    fn retract(&mut self) -> Result<(), CamPlannerError> {
        self.line(
            CamMotionKind::Retract,
            [self.current[0], self.current[1], self.safe_z],
            None,
        )
    }

    fn line(
        &mut self,
        kind: CamMotionKind,
        end_mm: [f64; 3],
        feed_mm_per_min: Option<f64>,
    ) -> Result<(), CamPlannerError> {
        self.ensure_capacity()?;
        let start_mm = self.current;
        self.motions.push(CamMotion {
            kind,
            path: CamMotionPath::Line { start_mm, end_mm },
            feed_mm_per_min,
        });
        self.current = end_mm;
        Ok(())
    }

    fn arc(
        &mut self,
        end_mm: [f64; 3],
        center_mm: [f64; 3],
        clockwise: bool,
        feed_mm_per_min: f64,
    ) -> Result<(), CamPlannerError> {
        self.ensure_capacity()?;
        let start_mm = self.current;
        self.motions.push(CamMotion {
            kind: CamMotionKind::Cut,
            path: CamMotionPath::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            },
            feed_mm_per_min: Some(feed_mm_per_min),
        });
        self.current = end_mm;
        Ok(())
    }

    fn ensure_capacity(&self) -> Result<(), CamPlannerError> {
        if self.motions.len() >= MAX_CAM_MOTIONS {
            Err(CamPlannerError::InvalidOperations)
        } else {
            Ok(())
        }
    }
}

/// Zigzags the cutter centre along X over `lanes` at `depth`; with `walls`, then
/// runs it once round the rectangle the lanes span.
fn append_raster(
    builder: &mut MotionBuilder,
    minimum_x: f64,
    maximum_x: f64,
    lanes: &[f64],
    depth: f64,
    plan: &CamPlan,
    walls: bool,
) -> Result<(), CamPlannerError> {
    let start = [minimum_x, lanes[0]];
    builder.rapid_to(start)?;
    builder.line(
        CamMotionKind::Plunge,
        [start[0], start[1], depth],
        Some(plan.tool.plunge_mm_per_min),
    )?;
    for (index, y) in lanes.iter().enumerate() {
        let x = if index % 2 == 0 { maximum_x } else { minimum_x };
        builder.line(
            CamMotionKind::Cut,
            [x, *y, depth],
            Some(plan.tool.feed_mm_per_min),
        )?;
        if let Some(next_y) = lanes.get(index + 1) {
            builder.line(
                CamMotionKind::Cut,
                [x, *next_y, depth],
                Some(plan.tool.feed_mm_per_min),
            )?;
        }
    }
    if walls {
        // The zigzag turns at each end only every other lane, so between its
        // lanes it leaves ridges on the walls across them (R1120ac-CAM-902): one
        // pass round the cutter's rectangle cuts every wall to its allowance.
        let corners = [
            [minimum_x, lanes[0]],
            [maximum_x, lanes[0]],
            [maximum_x, lanes[lanes.len() - 1]],
            [minimum_x, lanes[lanes.len() - 1]],
        ];
        let last = if lanes.len() % 2 == 1 { 2 } else { 3 };
        for step in 1..=4 {
            let [x, y] = corners[(last + step) % 4];
            builder.line(
                CamMotionKind::Cut,
                [x, y, depth],
                Some(plan.tool.feed_mm_per_min),
            )?;
        }
    }
    builder.retract()
}

/// Cuts every cutter-centre path of a region operation at each depth pass from
/// `top` down to `bottom`, after checking the region lies on the target and
/// the cutter stays in the stock.
fn append_region_paths(
    builder: &mut MotionBuilder,
    plan: &CamPlan,
    region: &CamRegion2d,
    paths: &[CamPath2d],
    [top, bottom]: [f64; 2],
    [target_bounds, stock_bounds]: [[[f64; 3]; 2]; 2],
) -> Result<(), CamPlannerError> {
    if plan.tool.kind == CamToolKind::Drill {
        return Err(CamPlannerError::UnsupportedTool);
    }
    let [low, high] = region.bounds()?;
    validate_point(low, target_bounds)?;
    validate_point(high, target_bounds)?;
    validate_z_range(top, bottom, target_bounds)?;
    let finished_z = bottom + plan.cut_parameters.axial_allowance_mm;
    validate_z(finished_z, target_bounds)?;
    validate_cut_depth(top, finished_z, plan.tool.flute_length_mm)?;
    let radius = plan.tool.diameter_mm * 0.5;
    let [low, high] = crate::cam_region::paths_bounds(paths)?;
    validate_tool_rectangle(
        [low[0] - radius, low[1] - radius],
        [high[0] + radius, high[1] + radius],
        stock_bounds,
    )?;
    for depth in depth_passes(top, finished_z, plan.cut_parameters.maximum_stepdown_mm)? {
        for path in paths {
            builder.rapid_to(path.start_mm)?;
            builder.line(
                CamMotionKind::Plunge,
                [path.start_mm[0], path.start_mm[1], depth],
                Some(plan.tool.plunge_mm_per_min),
            )?;
            for segment in &path.segments {
                match segment {
                    CamPathSegment2d::Line { to_mm } => builder.line(
                        CamMotionKind::Cut,
                        [to_mm[0], to_mm[1], depth],
                        Some(plan.tool.feed_mm_per_min),
                    )?,
                    CamPathSegment2d::Arc {
                        to_mm,
                        center_mm,
                        clockwise,
                    } => builder.arc(
                        [to_mm[0], to_mm[1], depth],
                        [center_mm[0], center_mm[1], depth],
                        *clockwise,
                        plan.tool.feed_mm_per_min,
                    )?,
                }
            }
            builder.retract()?;
        }
    }
    Ok(())
}

fn operation_id(operation: &CamOperation) -> u64 {
    match operation {
        CamOperation::Face { id, .. }
        | CamOperation::Pocket { id, .. }
        | CamOperation::Contour { id, .. }
        | CamOperation::Drill { id, .. }
        | CamOperation::RegionPocket { id, .. }
        | CamOperation::RegionContour { id, .. }
        | CamOperation::FacePocket { id, .. }
        | CamOperation::FaceContour { id, .. }
        | CamOperation::FaceDrill { id, .. } => *id,
    }
}

fn validate_rectangle(
    minimum: [f64; 2],
    maximum: [f64; 2],
    target_bounds: [[f64; 3]; 2],
) -> Result<(), CamPlannerError> {
    if !valid_rectangle(minimum, maximum) {
        return Err(CamPlannerError::InvalidOperations);
    }
    validate_point(minimum, target_bounds)?;
    validate_point(maximum, target_bounds)
}

fn valid_rectangle(minimum: [f64; 2], maximum: [f64; 2]) -> bool {
    minimum
        .into_iter()
        .chain(maximum)
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
        && minimum[0] < maximum[0]
        && minimum[1] < maximum[1]
}

fn validate_point(point: [f64; 2], target_bounds: [[f64; 3]; 2]) -> Result<(), CamPlannerError> {
    if point.into_iter().any(|value| !value.is_finite()) {
        return Err(CamPlannerError::InvalidOperations);
    }
    if (0..2).any(|axis| {
        point[axis] < target_bounds[0][axis] - GEOMETRY_TOLERANCE_MM
            || point[axis] > target_bounds[1][axis] + GEOMETRY_TOLERANCE_MM
    }) {
        return Err(CamPlannerError::GeometryOutsideTarget);
    }
    Ok(())
}

fn validate_tool_rectangle(
    minimum: [f64; 2],
    maximum: [f64; 2],
    stock_bounds: [[f64; 3]; 2],
) -> Result<(), CamPlannerError> {
    if (0..2).any(|axis| {
        minimum[axis] < stock_bounds[0][axis] - GEOMETRY_TOLERANCE_MM
            || maximum[axis] > stock_bounds[1][axis] + GEOMETRY_TOLERANCE_MM
    }) {
        return Err(CamPlannerError::ToolOutsideStock);
    }
    Ok(())
}

fn validate_z(value: f64, target_bounds: [[f64; 3]; 2]) -> Result<(), CamPlannerError> {
    if !value.is_finite() {
        return Err(CamPlannerError::InvalidOperations);
    }
    if value < target_bounds[0][2] - GEOMETRY_TOLERANCE_MM
        || value > target_bounds[1][2] + GEOMETRY_TOLERANCE_MM
    {
        return Err(CamPlannerError::GeometryOutsideTarget);
    }
    Ok(())
}

fn validate_z_range(
    top: f64,
    bottom: f64,
    target_bounds: [[f64; 3]; 2],
) -> Result<(), CamPlannerError> {
    validate_z(top, target_bounds)?;
    validate_z(bottom, target_bounds)?;
    if top <= bottom + GEOMETRY_TOLERANCE_MM {
        return Err(CamPlannerError::InvalidOperations);
    }
    Ok(())
}

fn validate_cut_depth(top: f64, bottom: f64, flute_length: f64) -> Result<(), CamPlannerError> {
    if !top.is_finite() || !bottom.is_finite() || top <= bottom + GEOMETRY_TOLERANCE_MM {
        return Err(CamPlannerError::InvalidOperations);
    }
    if top - bottom > flute_length + GEOMETRY_TOLERANCE_MM {
        return Err(CamPlannerError::CuttingDepthExceedsFlute);
    }
    Ok(())
}

fn lane_coordinates(minimum: f64, maximum: f64, step: f64) -> Result<Vec<f64>, CamPlannerError> {
    if !minimum.is_finite()
        || !maximum.is_finite()
        || !step.is_finite()
        || minimum > maximum
        || step <= 0.0
    {
        return Err(CamPlannerError::InvalidOperations);
    }
    let intervals = whole_steps(maximum - minimum, step);
    if intervals > 50_000 {
        return Err(CamPlannerError::InvalidOperations);
    }
    let mut lanes = Vec::with_capacity(intervals + 1);
    for index in 0..intervals {
        lanes.push((minimum + step * index as f64).min(maximum));
    }
    lanes.push(maximum);
    Ok(lanes)
}

fn depth_passes(top: f64, bottom: f64, stepdown: f64) -> Result<Vec<f64>, CamPlannerError> {
    if !top.is_finite()
        || !bottom.is_finite()
        || !stepdown.is_finite()
        || top <= bottom
        || stepdown <= 0.0
    {
        return Err(CamPlannerError::InvalidOperations);
    }
    let count = whole_steps(top - bottom, stepdown).max(1);
    if count > 50_000 {
        return Err(CamPlannerError::InvalidOperations);
    }
    Ok((1..count)
        .map(|index| (top - stepdown * index as f64).max(bottom))
        .chain([bottom])
        .collect())
}

/// Steps of `step` needed to cover `span`. A quotient a rounding error above a
/// whole number ((20 - 18.9) / 0.1 = 11.000000000000014) is that number, so no extra
/// pass repeats the last depth or lane.
fn whole_steps(span: f64, step: f64) -> usize {
    let quotient = span / step;
    let nearest = quotient.round();
    if (quotient - nearest).abs() <= ketchup_tolerance::ROUNDING * nearest.max(1.0) {
        crate::whole_count(nearest)
    } else {
        crate::whole_count(quotient.ceil())
    }
}

fn validate_path(
    path: &CamPath2d,
    target_bounds: [[f64; 3]; 2],
    stock_bounds: [[f64; 3]; 2],
    tool_radius: f64,
) -> Result<(), CamPlannerError> {
    if path.segments.is_empty() || path.segments.len() > MAX_PATH_SEGMENTS {
        return Err(CamPlannerError::InvalidOperations);
    }
    validate_point(path.start_mm, target_bounds)?;
    let mut current = path.start_mm;
    for segment in &path.segments {
        let (end, extent_min, extent_max) = match segment {
            CamPathSegment2d::Line { to_mm } => {
                validate_point(*to_mm, target_bounds)?;
                (
                    *to_mm,
                    [current[0].min(to_mm[0]), current[1].min(to_mm[1])],
                    [current[0].max(to_mm[0]), current[1].max(to_mm[1])],
                )
            }
            CamPathSegment2d::Arc {
                to_mm, center_mm, ..
            } => {
                validate_point(*to_mm, target_bounds)?;
                if center_mm.iter().any(|value| !value.is_finite()) {
                    return Err(CamPlannerError::InvalidOperations);
                }
                let start_radius = distance_2d(current, *center_mm);
                let end_radius = distance_2d(*to_mm, *center_mm);
                if start_radius <= GEOMETRY_TOLERANCE_MM
                    || (start_radius - end_radius).abs() > DEFAULT_LINEAR_TOLERANCE_MM
                {
                    return Err(CamPlannerError::InvalidOperations);
                }
                let minimum = [center_mm[0] - start_radius, center_mm[1] - start_radius];
                let maximum = [center_mm[0] + start_radius, center_mm[1] + start_radius];
                validate_point(minimum, target_bounds)?;
                validate_point(maximum, target_bounds)?;
                (*to_mm, minimum, maximum)
            }
        };
        validate_tool_rectangle(
            [extent_min[0] - tool_radius, extent_min[1] - tool_radius],
            [extent_max[0] + tool_radius, extent_max[1] + tool_radius],
            stock_bounds,
        )?;
        current = end;
    }
    if distance_2d(current, path.start_mm) > GEOMETRY_TOLERANCE_MM {
        return Err(CamPlannerError::InvalidOperations);
    }
    Ok(())
}

fn distance_2d(left: [f64; 2], right: [f64; 2]) -> f64 {
    ((left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2)).sqrt()
}

fn distance_3d(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| (left - right).powi(2))
        .sum::<f64>()
        .sqrt()
}

pub(crate) fn setup_bounds(world_bounds: [[f64; 3]; 2], setup: &CamSetup) -> [[f64; 3]; 2] {
    let z_axis = cross(setup.x_axis, setup.y_axis);
    let axes = [setup.x_axis, setup.y_axis, z_axis];
    let mut minimum = [f64::INFINITY; 3];
    let mut maximum = [f64::NEG_INFINITY; 3];
    for mask in 0..8 {
        let world = [
            world_bounds[(mask & 1 != 0) as usize][0],
            world_bounds[(mask & 2 != 0) as usize][1],
            world_bounds[(mask & 4 != 0) as usize][2],
        ];
        let relative = [
            world[0] - setup.origin_mm[0],
            world[1] - setup.origin_mm[1],
            world[2] - setup.origin_mm[2],
        ];
        for axis in 0..3 {
            let value = dot(relative, axes[axis]);
            minimum[axis] = minimum[axis].min(value);
            maximum[axis] = maximum[axis].max(value);
        }
    }
    [minimum, maximum]
}

fn compute_toolpath_digest(
    plan_id: CamPlanId,
    plan_digest: &str,
    target_digest: &str,
    operations: &[CamToolpathOperation],
    motions: &[CamMotion],
) -> String {
    let mut digest = Sha256::new();
    digest.update(CAM_TOOLPATH_SCHEMA_V1.as_bytes());
    digest.update(plan_id.0.to_le_bytes());
    digest_bytes(&mut digest, plan_digest.as_bytes());
    digest_bytes(&mut digest, target_digest.as_bytes());
    digest.update((operations.len() as u64).to_le_bytes());
    for operation in operations {
        digest.update(operation.id.to_le_bytes());
        digest.update([match operation.kind {
            CamOperationKind::Face => 0,
            CamOperationKind::Pocket => 1,
            CamOperationKind::Contour => 2,
            CamOperationKind::Drill => 3,
        }]);
        digest.update((operation.first_motion as u64).to_le_bytes());
        digest.update((operation.motion_count as u64).to_le_bytes());
    }
    digest.update((motions.len() as u64).to_le_bytes());
    for motion in motions {
        digest.update([match motion.kind {
            CamMotionKind::Rapid => 0,
            CamMotionKind::Plunge => 1,
            CamMotionKind::Cut => 2,
            CamMotionKind::Retract => 3,
        }]);
        match &motion.path {
            CamMotionPath::Line { start_mm, end_mm } => {
                digest.update([0]);
                for value in start_mm.iter().chain(end_mm) {
                    digest_f64(&mut digest, *value);
                }
            }
            CamMotionPath::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                digest.update([1, u8::from(*clockwise)]);
                for value in start_mm.iter().chain(end_mm).chain(center_mm) {
                    digest_f64(&mut digest, *value);
                }
            }
        }
        match motion.feed_mm_per_min {
            Some(feed) => {
                digest.update([1]);
                digest_f64(&mut digest, feed);
            }
            None => digest.update([0]),
        }
    }
    hex_digest(digest.finalize())
}

fn digest_bytes(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

fn digest_f64(digest: &mut Sha256, value: f64) {
    digest.update(
        if value == 0.0 { 0.0 } else { value }
            .to_bits()
            .to_le_bytes(),
    );
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn target_graph(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    feature_id: FeatureId,
) -> Result<ExactBRepGraph, CamError> {
    let feature = snapshot
        .feature(feature_id)
        .ok_or(CamError::TargetMissing)?;
    if feature.definition_id() != definition_id {
        return Err(CamError::TargetMissing);
    }
    if feature.kind().body_kind() != Some(BodyKind::Solid) {
        return Err(CamError::WrongBodyKind);
    }
    ExactBRepGraph::from_snapshot(snapshot, definition_id, feature_id)
        .map_err(CamError::TargetGraph)
}

fn validate_scalar_data(plan: &CamPlan) -> Result<(), CamError> {
    let bounded = |value: f64| value.is_finite() && value.abs() <= MAX_COORDINATE_MM;
    let positive = |value: f64| bounded(value) && value > 0.0;
    if plan.id.0 == 0
        || plan.name.trim().is_empty()
        || plan.name.len() > ketchup_tolerance::limits::TEXT_BYTES
        || plan.target.definition_id.0 == 0
        || plan.target.feature_id.0 == 0
        || !crate::graph::is_sha256_hex(&plan.target.exact_graph_digest)
        || plan.tool.number == 0
        || plan.tool.spindle_rpm == 0
        || plan.tool.spindle_rpm > MAX_RPM
        || !positive(plan.tool.diameter_mm)
        || !positive(plan.tool.flute_length_mm)
        || !positive(plan.tool.overall_length_mm)
        || plan.tool.flute_length_mm > plan.tool.overall_length_mm
        || !positive(plan.tool.holder_diameter_mm)
        || !positive(plan.tool.holder_length_mm)
        || !positive(plan.tool.feed_mm_per_min)
        || !positive(plan.tool.plunge_mm_per_min)
        || plan.tool.feed_mm_per_min > MAX_FEED_MM_PER_MIN
        || plan.tool.plunge_mm_per_min > plan.tool.feed_mm_per_min
        || plan.tool.point_angle_deg.is_some_and(|angle| {
            plan.tool.kind != CamToolKind::Drill || !DRILL_POINT_ANGLE_RANGE_DEG.contains(&angle)
        })
        || !positive(plan.setup.safe_height_mm)
        || !positive(plan.cut_parameters.maximum_stepdown_mm)
        || !plan.cut_parameters.stepover_ratio.is_finite()
        || !(0.0..=1.0).contains(&plan.cut_parameters.stepover_ratio)
        || plan.cut_parameters.stepover_ratio == 0.0
        || !bounded(plan.cut_parameters.radial_allowance_mm)
        || !bounded(plan.cut_parameters.axial_allowance_mm)
        || plan.stock.minimum_mm.iter().any(|value| !bounded(*value))
        || plan.stock.maximum_mm.iter().any(|value| !bounded(*value))
        || (0..3).any(|axis| plan.stock.minimum_mm[axis] >= plan.stock.maximum_mm[axis])
        || plan.setup.origin_mm.iter().any(|value| !bounded(*value))
        || !orthonormal_xy(plan.setup.x_axis, plan.setup.y_axis)
    {
        return Err(CamError::InvalidPlan);
    }
    Ok(())
}

fn orthonormal_xy(x: [f64; 3], y: [f64; 3]) -> bool {
    let length = |v: [f64; 3]| v.iter().map(|value| value * value).sum::<f64>().sqrt();
    let dot = x
        .iter()
        .zip(y)
        .map(|(left, right)| left * right)
        .sum::<f64>();
    (length(x) - 1.0).abs() <= ROUNDING
        && (length(y) - 1.0).abs() <= ROUNDING
        && dot.abs() <= ROUNDING
}

#[cfg(test)]
mod step_tests {
    use super::*;

    /// Review 2026-10-09 (P3): 1.1 mm at 0.1 mm per pass gave 12 passes, the
    /// last two at the same depth.
    #[test]
    fn a_depth_a_rounding_error_above_whole_passes_takes_no_extra_pass() {
        let passes = depth_passes(20.0, 18.9, 0.1).unwrap();
        assert_eq!(passes.len(), 11, "{passes:?}");
        assert_eq!(passes.last(), Some(&18.9));
        assert!(passes.windows(2).all(|pair| pair[1] < pair[0]));
        assert_eq!(depth_passes(2.1, 0.0, 0.7).unwrap().len(), 3);
        assert_eq!(depth_passes(20.0, 18.85, 0.1).unwrap().len(), 12);
        let lanes = lane_coordinates(0.0, 2.1, 0.7).unwrap();
        assert_eq!(lanes, [0.0, 0.7, 1.4, 2.1]);
    }
}
