use crate::document::{BodyKind, DefinitionId, FeatureId, Snapshot};
use crate::exact_brep_graph::ExactBRepGraph;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write;

pub const CAM_PLAN_SCHEMA_V1: &str = "ketchup.cam-plan.v1";
pub const CAM_TOOLPATH_SCHEMA_V1: &str = "ketchup.cam-toolpath.v1";
pub const CAM_SIMULATION_SCHEMA_V1: &str = "ketchup.cam-simulation.v1";
pub const CAM_POSTPROCESSOR_SCHEMA_V1: &str = "ketchup.cam-postprocessor.v1";
const MAX_CAM_POSTPROCESSOR_BYTES: usize = 16 * 1024 * 1024;
const CAM_POSTPROCESSOR_RESOLUTION_MM: f64 = 1.0e-9;
const MAX_ABS_MM: f64 = 1.0e6;
const MAX_RPM: u32 = 200_000;
const MAX_FEED_MM_PER_MIN: f64 = 1.0e6;
const MAX_CAM_OPERATIONS: usize = 1_024;
const MAX_PATH_SEGMENTS: usize = 4_096;
const MAX_DRILL_POINTS: usize = 4_096;
const GEOMETRY_TOLERANCE_MM: f64 = 1.0e-9;

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct CamPlanId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamUnits {
    Millimetres,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamToolKind {
    FlatEndMill,
    BallEndMill,
    Drill,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamWorkOffset {
    G54,
    G55,
    G56,
    G57,
    G58,
    G59,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamStock {
    pub minimum_mm: [f64; 3],
    pub maximum_mm: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamSetup {
    pub work_offset: CamWorkOffset,
    pub origin_mm: [f64; 3],
    pub x_axis: [f64; 3],
    pub y_axis: [f64; 3],
    pub safe_height_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamCutParameters {
    pub maximum_stepdown_mm: f64,
    pub stepover_ratio: f64,
    pub radial_allowance_mm: f64,
    pub axial_allowance_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamTarget {
    pub definition_id: DefinitionId,
    pub feature_id: FeatureId,
    pub exact_graph_digest: String,
}

#[derive(Clone, Debug, PartialEq)]
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
        bottom_z_mm: f64,
    },
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamPostprocessorDialect {
    IsoMetricGCode,
    ControllerNeutralJson,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamPostprocessorError {
    Plan(CamPlannerError),
    UnsafeSimulation,
    UnsafeFinalRetract,
    ArtifactTooLarge,
    Parse,
    DigestMismatch,
    RoundtripMismatch,
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
            Self::Parse => formatter.write_str("CAM postprocessor artifact is malformed"),
            Self::DigestMismatch => {
                formatter.write_str("CAM postprocessor artifact digest does not match its content")
            }
            Self::RoundtripMismatch => formatter.write_str(
                "CAM postprocessor artifact does not roundtrip to the canonical motion program",
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamPlannerError {
    Plan(CamError),
    InvalidOperations,
    GeometryOutsideTarget,
    ToolOutsideStock,
    UnsafeRapidHeight,
    CuttingDepthExceedsFlute,
    UnsupportedTool,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamError {
    InvalidPlan,
    TargetMissing,
    WrongBodyKind,
    StaleTarget,
    StockDoesNotContainTarget,
}

impl std::fmt::Display for CamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPlan => "CAM stock, tool, setup, or cutting parameters are invalid",
            Self::TargetMissing => "CAM target is missing or has no exact bounded result",
            Self::WrongBodyKind => "CAM target must be a solid body",
            Self::StaleTarget => "CAM target exact graph changed since the plan was created",
            Self::StockDoesNotContainTarget => "CAM stock does not contain the exact target bounds",
        })
    }
}

impl std::error::Error for CamError {}

impl CamPlan {
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
            Err(CamError::TargetMissing) => CamPlanHealth::TargetMissing,
            Err(CamError::WrongBodyKind) => CamPlanHealth::WrongBodyKind,
            Err(CamError::StaleTarget) => CamPlanHealth::StaleTarget,
            Err(CamError::InvalidPlan | CamError::StockDoesNotContainTarget) => {
                CamPlanHealth::Invalid
            }
        }
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
                .map_err(|_| CamError::TargetMissing)?
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
            .map_err(|_| CamError::TargetMissing)?
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
            .map_err(|_| CamError::TargetMissing)?
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
                    ..
                } => {
                    if points_mm.is_empty() || points_mm.len() > MAX_DRILL_POINTS {
                        return Err(CamPlannerError::InvalidOperations);
                    }
                    validate_z_range(*top_z_mm, *bottom_z_mm, target_bounds)?;
                    let finished_z = *bottom_z_mm + plan.cut_parameters.axial_allowance_mm;
                    validate_z(finished_z, target_bounds)?;
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
                    for point in points_mm {
                        for depth in &depths {
                            builder.rapid_to(*point)?;
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
            || self.motions.len() > 100_000
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
                            > 1.0e-7
                    {
                        return Err(CamPlannerError::StaleToolpath);
                    }
                    (*start_mm, *end_mm)
                }
            };
            if start
                .into_iter()
                .chain(end)
                .any(|value| !value.is_finite() || value.abs() > MAX_ABS_MM)
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
        let program = canonical_postprocessed_program(plan, toolpath, simulation)?;
        let content = match dialect {
            CamPostprocessorDialect::IsoMetricGCode => {
                render_iso_metric_gcode(&program).into_bytes()
            }
            CamPostprocessorDialect::ControllerNeutralJson => {
                serde_json::to_vec_pretty(&program).map_err(|_| CamPostprocessorError::Parse)?
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
            return Err(CamPostprocessorError::Parse);
        }
        if self.content_digest != content_digest(&self.content) {
            return Err(CamPostprocessorError::DigestMismatch);
        }
        let mut parsed = match self.dialect {
            CamPostprocessorDialect::IsoMetricGCode => parse_iso_metric_gcode(&self.content)?,
            CamPostprocessorDialect::ControllerNeutralJson => {
                serde_json::from_slice(&self.content).map_err(|_| CamPostprocessorError::Parse)?
            }
        };
        canonicalize_postprocessed_program(&mut parsed);
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
        let expected = canonical_postprocessed_program(plan, toolpath, simulation)?;
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
    let volume_tolerance = simulation.stock_before_mm3.abs().max(1.0) * 1.0e-9;
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
        || simulation.backend.len() > 1024
        || simulation.tolerance.is_empty()
        || simulation.tolerance.len() > 1024
        || !valid_sha256_hex(&simulation.result_fingerprint)
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
) -> Result<CamPostprocessedProgram, CamPostprocessorError> {
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
        .map(|motion| match &motion.path {
            CamMotionPath::Line { end_mm, .. } => CamPostprocessedMotion {
                kind: match motion.kind {
                    CamMotionKind::Rapid => CamPostprocessedMotionKind::Rapid,
                    CamMotionKind::Plunge => CamPostprocessedMotionKind::Plunge,
                    CamMotionKind::Cut => CamPostprocessedMotionKind::Cut,
                    CamMotionKind::Retract => CamPostprocessedMotionKind::Retract,
                },
                end_mm: end_mm.map(canonical_postprocessor_number),
                center_offset_mm: None,
                feed_mm_per_min: motion.feed_mm_per_min.map(canonical_postprocessor_number),
            },
            CamMotionPath::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => CamPostprocessedMotion {
                kind: if *clockwise {
                    CamPostprocessedMotionKind::ArcClockwise
                } else {
                    CamPostprocessedMotionKind::ArcCounterclockwise
                },
                end_mm: end_mm.map(canonical_postprocessor_number),
                center_offset_mm: Some([
                    canonical_postprocessor_number(center_mm[0] - start_mm[0]),
                    canonical_postprocessor_number(center_mm[1] - start_mm[1]),
                ]),
                feed_mm_per_min: motion.feed_mm_per_min.map(canonical_postprocessor_number),
            },
        })
        .collect();
    Ok(CamPostprocessedProgram {
        schema: CAM_POSTPROCESSOR_SCHEMA_V1.to_owned(),
        plan_digest: toolpath.plan_digest.clone(),
        toolpath_digest: toolpath.toolpath_digest.clone(),
        simulation_fingerprint: simulation.result_fingerprint.clone(),
        units: "mm".to_owned(),
        work_offset: work_offset_code(plan.setup.work_offset).to_owned(),
        tool_number: plan.tool.number,
        spindle_rpm: plan.tool.spindle_rpm,
        cutting_feed_mm_per_min: canonical_postprocessor_number(plan.tool.feed_mm_per_min),
        plunge_feed_mm_per_min: canonical_postprocessor_number(plan.tool.plunge_mm_per_min),
        safe_retract_z_mm: canonical_postprocessor_number(plan.setup.safe_height_mm),
        motions,
    })
}

fn validate_postprocessed_program(
    program: &CamPostprocessedProgram,
) -> Result<(), CamPostprocessorError> {
    if program.schema != CAM_POSTPROCESSOR_SCHEMA_V1
        || !valid_sha256_hex(&program.plan_digest)
        || !valid_sha256_hex(&program.toolpath_digest)
        || !valid_sha256_hex(&program.simulation_fingerprint)
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
        || program.motions.len() > 100_000
    {
        return Err(CamPostprocessorError::Parse);
    }
    for motion in &program.motions {
        if motion
            .end_mm
            .into_iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_ABS_MM)
        {
            return Err(CamPostprocessorError::Parse);
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
            return Err(CamPostprocessorError::Parse);
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
                    .any(|value| !value.is_finite() || value.abs() > MAX_ABS_MM)
            })
        {
            return Err(CamPostprocessorError::Parse);
        }
    }
    let last = program.motions.last().ok_or(CamPostprocessorError::Parse)?;
    if last.kind != CamPostprocessedMotionKind::Retract
        || (last.end_mm[2] - program.safe_retract_z_mm).abs() > GEOMETRY_TOLERANCE_MM
    {
        return Err(CamPostprocessorError::UnsafeFinalRetract);
    }
    Ok(())
}

fn render_iso_metric_gcode(program: &CamPostprocessedProgram) -> String {
    let mut lines = vec![
        "%".to_owned(),
        ";KETCHUP_CAM_POSTPROCESSOR_V1".to_owned(),
        format!(";PLAN_DIGEST={}", program.plan_digest),
        format!(";TOOLPATH_DIGEST={}", program.toolpath_digest),
        format!(";SIMULATION_FINGERPRINT={}", program.simulation_fingerprint),
        "G21".to_owned(),
        "G90".to_owned(),
        "G17".to_owned(),
        program.work_offset.clone(),
        format!("T{} M6", program.tool_number),
        format!("S{} M3", program.spindle_rpm),
        format!(
            ";CUTTING_FEED_MM_PER_MIN={}",
            format_cam_number(program.cutting_feed_mm_per_min)
        ),
        format!(
            ";PLUNGE_FEED_MM_PER_MIN={}",
            format_cam_number(program.plunge_feed_mm_per_min)
        ),
        format!(
            ";SAFE_RETRACT_Z_MM={}",
            format_cam_number(program.safe_retract_z_mm)
        ),
    ];
    for motion in &program.motions {
        let code = match motion.kind {
            CamPostprocessedMotionKind::Rapid | CamPostprocessedMotionKind::Retract => "G0",
            CamPostprocessedMotionKind::Plunge | CamPostprocessedMotionKind::Cut => "G1",
            CamPostprocessedMotionKind::ArcClockwise => "G2",
            CamPostprocessedMotionKind::ArcCounterclockwise => "G3",
        };
        let mut line = format!(
            "{code} X{} Y{} Z{}",
            format_cam_number(motion.end_mm[0]),
            format_cam_number(motion.end_mm[1]),
            format_cam_number(motion.end_mm[2])
        );
        if let Some(offset) = motion.center_offset_mm {
            line.push_str(&format!(
                " I{} J{}",
                format_cam_number(offset[0]),
                format_cam_number(offset[1])
            ));
        }
        if let Some(feed) = motion.feed_mm_per_min {
            line.push_str(&format!(" F{}", format_cam_number(feed)));
        }
        line.push_str(match motion.kind {
            CamPostprocessedMotionKind::Rapid => " ;RAPID",
            CamPostprocessedMotionKind::Plunge => " ;PLUNGE",
            CamPostprocessedMotionKind::Cut => " ;CUT",
            CamPostprocessedMotionKind::Retract => " ;RETRACT",
            CamPostprocessedMotionKind::ArcClockwise
            | CamPostprocessedMotionKind::ArcCounterclockwise => " ;CUT_ARC",
        });
        lines.push(line);
    }
    lines.extend(["M5".to_owned(), "M30".to_owned(), "%".to_owned()]);
    let mut output = lines.join("\n");
    output.push('\n');
    output
}

fn parse_iso_metric_gcode(bytes: &[u8]) -> Result<CamPostprocessedProgram, CamPostprocessorError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CamPostprocessorError::Parse)?;
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() < 18
        || lines[0] != "%"
        || lines[1] != ";KETCHUP_CAM_POSTPROCESSOR_V1"
        || lines[5] != "G21"
        || lines[6] != "G90"
        || lines[7] != "G17"
        || !matches!(lines[8], "G54" | "G55" | "G56" | "G57" | "G58" | "G59")
        || lines[lines.len() - 3..] != ["M5", "M30", "%"]
    {
        return Err(CamPostprocessorError::Parse);
    }
    let plan_digest = prefixed_value(lines[2], ";PLAN_DIGEST=")?.to_owned();
    let toolpath_digest = prefixed_value(lines[3], ";TOOLPATH_DIGEST=")?.to_owned();
    let simulation_fingerprint = prefixed_value(lines[4], ";SIMULATION_FINGERPRINT=")?.to_owned();
    let tool_line = lines[9]
        .strip_prefix('T')
        .and_then(|line| line.strip_suffix(" M6"))
        .ok_or(CamPostprocessorError::Parse)?;
    let tool_number = parse_u32(tool_line)?;
    let spindle_line = lines[10]
        .strip_prefix('S')
        .and_then(|line| line.strip_suffix(" M3"))
        .ok_or(CamPostprocessorError::Parse)?;
    let spindle_rpm = parse_u32(spindle_line)?;
    let cutting_feed_mm_per_min =
        parse_f64(prefixed_value(lines[11], ";CUTTING_FEED_MM_PER_MIN=")?)?;
    let plunge_feed_mm_per_min = parse_f64(prefixed_value(lines[12], ";PLUNGE_FEED_MM_PER_MIN=")?)?;
    let safe_retract_z_mm = parse_f64(prefixed_value(lines[13], ";SAFE_RETRACT_Z_MM=")?)?;
    let motions = lines[14..lines.len() - 3]
        .iter()
        .map(|line| parse_iso_motion(line))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CamPostprocessedProgram {
        schema: CAM_POSTPROCESSOR_SCHEMA_V1.to_owned(),
        plan_digest,
        toolpath_digest,
        simulation_fingerprint,
        units: "mm".to_owned(),
        work_offset: lines[8].to_owned(),
        tool_number,
        spindle_rpm,
        cutting_feed_mm_per_min,
        plunge_feed_mm_per_min,
        safe_retract_z_mm,
        motions,
    })
}

fn parse_iso_motion(line: &str) -> Result<CamPostprocessedMotion, CamPostprocessorError> {
    let (words, label) = line.rsplit_once(" ;").ok_or(CamPostprocessorError::Parse)?;
    let mut words = words.split_whitespace();
    let code = words.next().ok_or(CamPostprocessorError::Parse)?;
    let x = parse_axis(words.next(), 'X')?;
    let y = parse_axis(words.next(), 'Y')?;
    let z = parse_axis(words.next(), 'Z')?;
    let kind = match (code, label) {
        ("G0", "RAPID") => CamPostprocessedMotionKind::Rapid,
        ("G0", "RETRACT") => CamPostprocessedMotionKind::Retract,
        ("G1", "PLUNGE") => CamPostprocessedMotionKind::Plunge,
        ("G1", "CUT") => CamPostprocessedMotionKind::Cut,
        ("G2", "CUT_ARC") => CamPostprocessedMotionKind::ArcClockwise,
        ("G3", "CUT_ARC") => CamPostprocessedMotionKind::ArcCounterclockwise,
        _ => return Err(CamPostprocessorError::Parse),
    };
    let is_arc = matches!(
        kind,
        CamPostprocessedMotionKind::ArcClockwise | CamPostprocessedMotionKind::ArcCounterclockwise
    );
    let center_offset_mm = if is_arc {
        Some([
            parse_axis(words.next(), 'I')?,
            parse_axis(words.next(), 'J')?,
        ])
    } else {
        None
    };
    let needs_feed = !matches!(
        kind,
        CamPostprocessedMotionKind::Rapid | CamPostprocessedMotionKind::Retract
    );
    let feed_mm_per_min = if needs_feed {
        Some(parse_axis(words.next(), 'F')?)
    } else {
        None
    };
    if words.next().is_some() {
        return Err(CamPostprocessorError::Parse);
    }
    Ok(CamPostprocessedMotion {
        kind,
        end_mm: [x, y, z],
        center_offset_mm,
        feed_mm_per_min,
    })
}

fn prefixed_value<'a>(line: &'a str, prefix: &str) -> Result<&'a str, CamPostprocessorError> {
    line.strip_prefix(prefix)
        .filter(|value| !value.is_empty())
        .ok_or(CamPostprocessorError::Parse)
}

fn parse_axis(word: Option<&str>, axis: char) -> Result<f64, CamPostprocessorError> {
    let word = word.ok_or(CamPostprocessorError::Parse)?;
    let value = word
        .strip_prefix(axis)
        .ok_or(CamPostprocessorError::Parse)?;
    parse_f64(value)
}

fn parse_f64(value: &str) -> Result<f64, CamPostprocessorError> {
    value.parse().map_err(|_| CamPostprocessorError::Parse)
}

fn parse_u32(value: &str) -> Result<u32, CamPostprocessorError> {
    value.parse().map_err(|_| CamPostprocessorError::Parse)
}

fn canonicalize_postprocessed_program(program: &mut CamPostprocessedProgram) {
    program.cutting_feed_mm_per_min =
        canonical_postprocessor_number(program.cutting_feed_mm_per_min);
    program.plunge_feed_mm_per_min = canonical_postprocessor_number(program.plunge_feed_mm_per_min);
    program.safe_retract_z_mm = canonical_postprocessor_number(program.safe_retract_z_mm);
    for motion in &mut program.motions {
        motion.end_mm = motion.end_mm.map(canonical_postprocessor_number);
        motion.center_offset_mm = motion
            .center_offset_mm
            .map(|offset| offset.map(canonical_postprocessor_number));
        motion.feed_mm_per_min = motion.feed_mm_per_min.map(canonical_postprocessor_number);
    }
}

fn canonical_postprocessor_number(value: f64) -> f64 {
    let rounded =
        (value / CAM_POSTPROCESSOR_RESOLUTION_MM).round() * CAM_POSTPROCESSOR_RESOLUTION_MM;
    if rounded == 0.0 { 0.0 } else { rounded }
}

fn format_cam_number(value: f64) -> String {
    if value == 0.0 {
        "0".to_owned()
    } else {
        value.to_string()
    }
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

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
                .any(|value| !value.is_finite() || value.abs() > MAX_ABS_MM)
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
        if self.motions.len() >= 100_000 {
            Err(CamPlannerError::InvalidOperations)
        } else {
            Ok(())
        }
    }
}

fn append_raster(
    builder: &mut MotionBuilder,
    minimum_x: f64,
    maximum_x: f64,
    lanes: &[f64],
    depth: f64,
    plan: &CamPlan,
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
    builder.retract()
}

fn operation_id(operation: &CamOperation) -> u64 {
    match operation {
        CamOperation::Face { id, .. }
        | CamOperation::Pocket { id, .. }
        | CamOperation::Contour { id, .. }
        | CamOperation::Drill { id, .. } => *id,
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
        .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
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
    let intervals = ((maximum - minimum) / step).ceil() as usize;
    if intervals > 50_000 {
        return Err(CamPlannerError::InvalidOperations);
    }
    let mut lanes = Vec::with_capacity(intervals + 1);
    for index in 0..=intervals {
        lanes.push((minimum + step * index as f64).min(maximum));
    }
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
    let count = ((top - bottom) / stepdown).ceil() as usize;
    if count == 0 || count > 50_000 {
        return Err(CamPlannerError::InvalidOperations);
    }
    Ok((1..=count)
        .map(|index| (top - stepdown * index as f64).max(bottom))
        .collect())
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
                    || (start_radius - end_radius).abs() > 1.0e-7
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

fn setup_bounds(world_bounds: [[f64; 3]; 2], setup: &CamSetup) -> [[f64; 3]; 2] {
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

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.into_iter().zip(right).map(|(a, b)| a * b).sum()
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
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
        .map_err(|_| CamError::TargetMissing)
}

fn validate_scalar_data(plan: &CamPlan) -> Result<(), CamError> {
    let bounded = |value: f64| value.is_finite() && value.abs() <= MAX_ABS_MM;
    let positive = |value: f64| bounded(value) && value > 0.0;
    if plan.id.0 == 0
        || plan.name.trim().is_empty()
        || plan.name.len() > 1024
        || plan.target.definition_id.0 == 0
        || plan.target.feature_id.0 == 0
        || plan.target.exact_graph_digest.len() != 64
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
    (length(x) - 1.0).abs() <= 1.0e-9 && (length(y) - 1.0).abs() <= 1.0e-9 && dot.abs() <= 1.0e-9
}
