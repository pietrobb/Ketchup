// Unsafe code is denied, not forbidden, for exactly one audited reason: the
// native file dialogs must borrow the raw main-window handle to be owned by,
// and modal to, the Ketchup window. See `dialogs::DialogParentWindow`.
#![deny(unsafe_code)]

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use gesture::{Gesture, MirrorPlane, PushPullAnchor, ZoomWindowDrag};
use ketchup_analysis::fea::{FeaMaterial, FeaSolveSettings};
use ketchup_application::cam_workflow::{CamReviewRequest, CamReviewSummary, CamReviewWorkflow};
use ketchup_application::diagnostics::{
    AssistantPlanningResult, AssistantRejection, assistant_canonical_rejection, assistant_rejection,
};
use ketchup_application::evaluation::{ExactEvaluationTask, ExactSource, exact_worker_candidates};
use ketchup_application::fea_workflow::{
    ExactFeaFaceTraction, ExactFeaSetup, ExactVolumeMeshWireOptions, FeaReviewSummary,
    FeaReviewWorkflow, FeaStudyRequest,
};
use ketchup_application::pdm_workflow::{
    LocalPdmWorkflow, PdmCreateReleaseRequest, PdmDocumentState, PdmSourceIdentity,
};
pub use ketchup_application::topology::GeneralFinishKind;
use ketchup_application::topology::plan_topology_finish_kind;
use ketchup_application::transforms::{
    mirrored_copy_commands, rotation_in_parent_space, translated_transform,
    world_axis_rotation_transform, world_edit_in_parent_space, world_plane_mirror_transform,
};
use ketchup_application::validation::*;
#[cfg(test)]
use ketchup_assistant::sidecar::{
    AssistantCadBodyFeature, AssistantCadBooleanOperation, AssistantCadDeletePolicy,
    AssistantCadLoftContinuity,
};
use ketchup_assistant::sidecar::{
    AssistantCadEditProgram, AssistantRejectionDiagnostic, AssistantRejectionPhase,
};
use ketchup_geometry::linalg::{circumcenter, cross, dot, length, signed_outline_distance};
use ketchup_geometry::sketch::{
    FeatureDirection, FeatureExtent, PadOperation, PadProfile, PadSpec, PrincipalPlane,
    SketchEntity, WorkplaneFrame, WorkplaneSupport,
};
use ketchup_interaction::{
    Axis, ElementId, ExactHit, LocaleCatalog, PickResult, Ray, SelectionId, Side, SnapKind,
    SnapResult, Vec3,
    exact_projection::{
        ExactInteractionProjection, SnapshotBoundTopologicalSelection, TopologicalPickLocator,
    },
    mesh_projection::{MeshInteractionProjection, canonical_sketch_profile_mesh},
    projection::{
        CanonicalInteractionProjection, InteractionProjection, ProjectedBox,
        definition_requires_evaluated_geometry,
    },
};
use ketchup_manufacturing::blender_export::{ExactGlbExport, MeshGlbInstance, model_glb_export};
use ketchup_manufacturing::dxf_export::{DxfProfileExport, export_visible_profiles_dxf};
use ketchup_manufacturing::fabrication::{
    BtlxExportOptions, BtlxProfileProcessingRequest, GeneralFabricationProjection,
    project_general_fabrication,
};
use ketchup_manufacturing::three_mf_export::{
    ExactThreeMfExport, MeshThreeMfInstance, model_three_mf_export,
};
use ketchup_model::cam::{CamPlanId, CamPostprocessorDialect};
#[cfg(test)]
use ketchup_model::document::ParameterValueType;
use ketchup_model::document::{
    AuthenticatedApprover, BodyId, BooleanOperation, CanonicalCommand, CanonicalError,
    ClassificationCategoryId, ClassificationDimensionId, CloneDefinitionPlan, CommandBatch,
    ConvertGroupPlan, DefinitionId, Dimension, DocumentId, DocumentStore, EdgeFinishKind, EdgeRef,
    EvaluationIdentity, EvaluatorParameterEdit, FaceRef, FeatureId, FeatureKind, GroupId,
    HighRiskClass, HighRiskScope, InstancePath, LoftContinuity, LoftSection,
    MAX_HUMAN_CONFIRMATION_LIFETIME_MS, MeshBodySpec, NodeId, OccurrenceId, ProfileSegment,
    Proposal, ProposalCommitError, ProposalContext, ProposalPrepareError, ProposalPrincipal,
    SPLINE_MIN_POINTS, SceneOccurrence, SceneQueryContext, SectionPlane,
    SideEffectAuthorizationReceipt, Snapshot, SolidToolPlan, SpatialPathSegment, TagId,
    TipReplacementParent, TipReplacementProposal, Transform, TrustedConfirmationSurface,
    ellipse_segments, polygon_segments, regular_polygon_points,
};
use ketchup_model::exact_brep_graph::ExactBRepGraph;
use ketchup_model::exact_product::{
    AssemblySelectionTarget, ExactBodyPackage, ExactBodyView, ExactFaceRole, ExactMeshExport,
    ExactResultRegistry, ExactStlExport, MeshExportBody, MeshExportSource,
    exact_body_terminal_features, model_stl_export,
};
#[cfg(test)]
use ketchup_model::exact_product::{ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence};
use ketchup_model::exact_validation::GeneralBodyParticipant;
use ketchup_model::graph::{EvaluationStatus, EvaluatorNodeKind, sha256_bytes};
use ketchup_model::import::{
    DxfImportOptions, IgesXdeImportEvidence, ImportDiagnosticSeverity, ImportFormat,
    ImportLengthUnit, ImportUnitAuthority, ImportUnitDecision, ParsedDxf, ParsedGlbScene,
    ParsedSketchupScene, ParsedStlMesh, StepXdeImportEvidence, inspect_dxf, inspect_glb,
    inspect_sketchup_scene, parse_stl, plan_dxf_import, plan_glb_import, plan_iges_xde_import,
    plan_sketchup_scene_import, plan_step_xde_import, plan_stl_import,
};
#[cfg(test)]
use ketchup_model::import::{
    StepImportEvidence, StepImportMesh, StepMeshTriangle, plan_step_import,
};
use ketchup_model::persistence::ContainerData;
use ketchup_model::sheet_metal::{
    SheetMetalManufacturingProjection, project_sheet_metal_manufacturing,
};
use ketchup_model::tolerance::limits;
use ketchup_model::tolerance::{
    ACCUMULATED_ROUNDING, APPROXIMATION, DEFAULT_LINEAR_TOLERANCE_MM, ROUNDING, SCREEN_ROUNDING_PX,
};
use ketchup_model::topology::{TopologicalElementKind, TopologicalElementRef};
use ketchup_pdm::local::{
    ReleaseAudit, ReleaseCatalogEntry, ReleaseComparison, ReleaseConflictVerdict,
    ReleaseDependencyInput, ReleaseManifest,
};
use ketchup_scheduler::ExactWorkerSupervisor;
use modal::Modal;
use slot::Slot;
use tool_preview::ToolPreview;
pub use view_settings::{ViewFlag, ViewSettings};
mod app;
use app::*;
mod app_state;
mod assembly_ui;
mod body_ui;
mod bounded_parser;
mod close_guard;
mod command_registry;
pub mod dialogs;
mod drawn_shape;
mod export_bundle;
mod face_workflow_ui;
mod feature_history_ui;
mod gesture;
mod glb_import_ui;
mod helix_ui;
use export_bundle::{
    ExportBundlePrecondition, ExportConsent, ExportError, export_target_sha256,
    write_export_artifact_if_unchanged, write_export_bundle,
};
mod import_source;
mod migration_review;
use import_source::{ImportError, ImportFailure, ImportSourcePlan, read_import_source};
mod keymap;
pub mod language;
mod length_input;
pub mod live_bridge;
mod mesh_conversion_ui;
mod modal;
mod native_document_inspection;
mod occurrence_color_ui;
mod planar_push_pull;
mod program_edit;
mod program_evaluation;
mod program_source_ui;
mod refusal;
mod selection_measures_ui;
use ketchup_rejection::{Rejection, RejectionPhase};
use refusal::{Refuse, failed, invalid_field};
mod slot;
mod tool_preview;
mod transform_operation;
mod validator_ui;
mod view_settings;
mod viewport_feedback;
pub use face_workflow_ui::HeadlessFaceWorkflowFailure;
pub use helix_ui::{AxisSpec, HelixHandedness, HelixToolParameters, helix_segments};
pub use native_document_inspection::{
    NativeDocumentInspection, NativeDocumentInspectionError, inspect_native_document,
};
use transform_operation::{TransformPlan, TransformRequest, TransformTarget};
mod drawing_plane;
mod line_geometry;
mod rectangle_authoring;
mod scene_snapping;
pub mod theme;
use theme::{Icon, Palette, ThemeKind};
pub mod renderer;

use dialogs::{
    DialogParentWindow, DiscardRequest, ExportRequest, FileDialogs, HighRiskConfirmationRequest,
    HistoryTruncationRequest, ImportDialogRequest, NativeFileDialogs, SaveRequest,
};
use renderer::{DerivedRenderCache, GpuInstancedRenderer, InstancedRenderPlan, ScenePaintCallback};

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const INITIAL_BOX_DEFINITION: DefinitionId = DefinitionId(1);
const BOX_WIDTH_MM: f64 = 100.0;
const BOX_DEPTH_MM: f64 = 60.0;
const GRID_STEP_MM: f64 = 10.0;
const MESH_CONVERSION_TOLERANCE_MM: f64 = 0.25;
const PRECISE_SNAP_SCREEN_TOLERANCE_PX: f64 = 6.0;
const PRECISE_SNAP_WORLD_TOLERANCE_MM: f64 = 0.5;
/// Rotate snaps to this many degrees unless Shift asks for a free angle.
const ROTATION_SNAP_DEGREES: f64 = 15.0;
/// Below this arm length the pointer is effectively on the rotation centre and
/// its angle is noise, so the gesture holds its previous value.
const ROTATION_MIN_ARM_MM: f64 = 0.5;
const SHELL_TITLE_SIZE: f32 = 14.0;
const SHELL_BODY_SIZE: f32 = 12.5;
const SHELL_SMALL_SIZE: f32 = 11.0;
const SHELL_MONO_SIZE: f32 = 12.0;
const SHELL_SECTION_SIZE: f32 = 11.5;
/// Edge of one square tool-rail button, in points.
const TOOL_BUTTON_SIZE: f32 = 36.0;
/// Edge of the glyph drawn inside a tool-rail button, in points.
const TOOL_ICON_SIZE: f32 = 20.0;
/// Width of the tool rail, in points.
const TOOL_RAIL_WIDTH: f32 = 54.0;
/// Closest millimetre distance a point may sit in front of a converging eye.
const PERSPECTIVE_NEAR_MM: f64 = 1.0;
/// How far outside the model's bounding sphere a converging eye must sit.
const CAMERA_CLEARANCE: f64 = 2.5;
/// Smallest useful magnification. This still frames scenes hundreds of kilometres wide.
// not a tolerance: a view limit.
const MIN_CAMERA_ZOOM: f32 = 1.0e-6;
/// Largest useful magnification before floating-point picking becomes unstable.
const MAX_CAMERA_ZOOM: f32 = 8.0;
const CAMERA_ZOOM_STEP: f32 = 1.25;
#[derive(Clone, Debug, Eq, PartialEq)]
struct AssistantValidationSelection {
    mode: &'static str,
    requested: BTreeSet<&'static str>,
    unknown: Vec<String>,
}

/// One validator finding rendered for the operator, with the parts it refers to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorPanelFinding {
    pub validator: &'static str,
    pub code: String,
    pub severity: String,
    pub parts: Vec<String>,
    pub detail: String,
    /// The parts named in `parts`, selected when the finding is clicked.
    pub occurrences: Vec<OccurrenceId>,
    /// The clearance volume the finding is about, outlined when clicked.
    pub clearance_volume: Option<ketchup_model::space::ClearanceVolumeId>,
}

/// The result of one manual validator run, bound to the revision it was run on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorPanelReport {
    pub revision: u64,
    pub canonical_digest: String,
    pub executed: Vec<&'static str>,
    pub state: String,
    pub complete: bool,
    pub issue_count: usize,
    pub findings: Vec<ValidatorPanelFinding>,
    pub not_evaluated: Vec<(String, String)>,
}

fn validator_finding_parts(issue: &serde_json::Value) -> Vec<(OccurrenceId, String)> {
    let labelled = |id: &serde_json::Value, name: &serde_json::Value| {
        let id = id.as_u64()?;
        let label = match name.as_str() {
            Some(name) => format!("{name} (#{id})"),
            None => format!("#{id}"),
        };
        Some((OccurrenceId(id), label))
    };
    let mut parts = Vec::new();
    parts.extend(labelled(&issue["occurrence_id"], &issue["name"]));
    parts.extend(labelled(&issue["left_occurrence_id"], &issue["left_name"]));
    parts.extend(labelled(
        &issue["right_occurrence_id"],
        &issue["right_name"],
    ));
    if let Some(ids) = issue["occurrence_ids"].as_array() {
        let names = issue["names"].as_array();
        for (index, id) in ids.iter().enumerate() {
            let name = names
                .and_then(|names| names.get(index))
                .unwrap_or(&serde_json::Value::Null);
            parts.extend(labelled(id, name));
        }
    }
    parts
}

fn validator_panel_report(validation: &serde_json::Value) -> ValidatorPanelReport {
    let executed = ASSISTANT_VALIDATOR_IDS
        .into_iter()
        .filter(|validator| {
            validation["executed"]
                .as_array()
                .is_some_and(|executed| executed.iter().any(|value| value == validator))
        })
        .collect::<Vec<_>>();
    let findings = executed
        .iter()
        .flat_map(|validator| {
            validation[validator]["issues"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|issue| {
                    let (occurrences, parts) = validator_finding_parts(issue).into_iter().unzip();
                    ValidatorPanelFinding {
                        validator,
                        code: issue["code"].as_str().unwrap_or_default().to_owned(),
                        severity: issue["severity"].as_str().unwrap_or_default().to_owned(),
                        parts,
                        occurrences,
                        clearance_volume: issue["clearance_volume_id"]
                            .as_u64()
                            .map(ketchup_model::space::ClearanceVolumeId),
                        detail: issue["rule"]
                            .as_str()
                            .or_else(|| issue["evidence"].as_str())
                            .map(str::to_owned)
                            .unwrap_or_else(|| {
                                issue
                                    .get("evidence")
                                    .map(ToString::to_string)
                                    .unwrap_or_default()
                            }),
                    }
                })
        })
        .collect::<Vec<_>>();
    let not_evaluated = validation["not_evaluated"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            (
                entry["validator"].as_str().unwrap_or_default().to_owned(),
                entry["reason"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    ValidatorPanelReport {
        revision: validation["revision"].as_u64().unwrap_or_default(),
        canonical_digest: validation["canonical_digest"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        executed,
        state: validation["state"].as_str().unwrap_or_default().to_owned(),
        complete: validation["complete"].as_bool().unwrap_or_default(),
        issue_count: validation["issue_count"].as_u64().unwrap_or_default() as usize,
        findings,
        not_evaluated,
    }
}

pub type SelectedAdapterInfo = Arc<Mutex<Option<eframe::wgpu::AdapterInfo>>>;

pub struct AdapterRequirement {
    pub name: String,
    pub device_type: eframe::wgpu::DeviceType,
}

#[derive(Clone)]
struct PushPullDrag {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    selection: SelectionId,
    pointer_start: Pos2,
    extent_start_mm: f64,
    screen_normal: Vec2,
    pixels_per_mm: f32,
}

#[derive(Clone)]
struct LastPushPull {
    selection: SelectionId,
    /// Document revision produced by that Push/Pull. A typed value is treated as
    /// a correction of this operation only while the document still sits on it.
    revision: u64,
    canonical_digest: String,
}

#[derive(Clone)]
struct MoveProfileTarget {
    definition_id: DefinitionId,
    body_id: BodyId,
    profile_id: FeatureId,
    world_origin: Vec3,
    world_x_axis: Vec3,
    world_y_axis: Vec3,
}

#[derive(Clone, Copy)]
struct SelectionWindowDrag {
    start: Pos2,
    cursor: Pos2,
    additive: bool,
}

#[derive(Clone)]
struct MoveDrag {
    source_document_id: DocumentId,
    source_revision: u64,
    selection: SelectionId,
    occurrence_paths: BTreeSet<InstancePath>,
    group_id: Option<GroupId>,
    profile_target: Option<MoveProfileTarget>,
    pointer_start_world: Vec3,
    plane_z: f64,
    /// The axis the arrow keys pinned this gesture to, if any. Without a pin
    /// the pointer stays on the horizontal plane it started on.
    axis: Option<Axis>,
    /// Where along the pinned axis the gesture started, established from the
    /// first pointer sample after the pin so the body never jumps.
    axis_reference: Option<f64>,
    delta_mm: Vec3,
    copy: bool,
}

/// One in-progress Rotate gesture.
///
/// The rotation happens in the plane through `centre_mm` whose normal is
/// `axis`, so the live angle is read from where the pointer meets that plane
/// rather than from raw screen motion.
#[derive(Clone)]
struct ScaleDrag {
    source_document_id: DocumentId,
    source_revision: u64,
    selection: SelectionId,
    occurrence_paths: BTreeSet<InstancePath>,
    group_id: Option<GroupId>,
    centre_mm: Vec3,
    centre_screen: Pos2,
    reference_radius_points: f32,
    axis: Option<Axis>,
    factor: f64,
}

#[derive(Clone)]
struct RotateDrag {
    source_document_id: DocumentId,
    source_revision: u64,
    selection: SelectionId,
    occurrence_paths: BTreeSet<InstancePath>,
    group_id: Option<GroupId>,
    centre_mm: Vec3,
    axis: Axis,
    /// The starting arm, measured from `centre_mm` inside the rotation plane.
    ///
    /// The first click chooses the pivot; a second click establishes this arm.
    /// Until then the gesture reads zero and accepts a typed angle.
    reference_mm: Option<Vec3>,
    angle_degrees: f64,
    copy: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolSessionPhase {
    Gesture,
    Anchor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransformInputEvent {
    ToggleCopy,
    CopyRequested,
}

#[derive(Default)]
struct TransformInputInterpreter {
    command_down: bool,
    command_chord_seen: bool,
}

impl TransformInputInterpreter {
    fn interpret_command(
        &mut self,
        enabled: bool,
        command_down: bool,
        command_chord: bool,
    ) -> Option<TransformInputEvent> {
        if !enabled {
            self.command_down = command_down;
            self.command_chord_seen = true;
            return None;
        }
        if command_down {
            if !self.command_down {
                self.command_chord_seen = command_chord;
            } else {
                self.command_chord_seen |= command_chord;
            }
            self.command_down = true;
            return None;
        }

        let event = (self.command_down && !self.command_chord_seen)
            .then_some(TransformInputEvent::ToggleCopy);
        self.command_down = false;
        self.command_chord_seen = false;
        event
    }

    fn interpret_pointer_copy(command_down: bool) -> Option<TransformInputEvent> {
        command_down.then_some(TransformInputEvent::CopyRequested)
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone)]
enum ToolSession {
    Move {
        phase: ToolSessionPhase,
        drag: MoveDrag,
    },
    Rotate {
        phase: ToolSessionPhase,
        drag: RotateDrag,
    },
    Scale(ScaleDrag),
}

/// The on-screen protractor a frame draws for the Rotate tool.
#[derive(Clone, Copy)]
struct RotationGuide {
    centre_mm: Vec3,
    axis: Axis,
    radius_mm: f64,
    /// Where the starting arm points inside the rotation plane, once a gesture
    /// has established one. `None` means the tool is merely armed.
    start_degrees: Option<f64>,
    angle_degrees: f64,
}

#[derive(Clone, Eq, PartialEq)]
enum CorrectionSelection {
    Occurrences {
        occurrence_ids: BTreeSet<OccurrenceId>,
        primary_occurrence_id: Option<OccurrenceId>,
    },
    Group(GroupId),
}

#[derive(Clone)]
struct MoveCorrection {
    direction: Vec3,
    applied_distance_mm: f64,
    accepts_vector_correction: bool,
}

#[derive(Clone)]
struct MoveCopyCorrection {
    source_occurrence_ids: Vec<OccurrenceId>,
    first_copy_occurrence_id: OccurrenceId,
    primary_source_index: usize,
    element: ElementId,
    delta_mm: Vec3,
    array_mode: MoveCopyArrayMode,
    array_count: usize,
}

#[derive(Clone)]
enum CorrectionOperation {
    Move(MoveCorrection),
    MoveCopy(MoveCopyCorrection),
    Rotate(RotateCorrection),
    Scale(ScaleCorrection),
}

#[derive(Clone)]
struct CorrectionSession {
    revision: u64,
    canonical_digest: String,
    selection: CorrectionSelection,
    operation: CorrectionOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MoveCopyArrayMode {
    Multiply,
    Divide,
}

#[derive(Clone)]
struct RotateCorrection {
    copy_source_occurrence_ids: Option<Vec<OccurrenceId>>,
    centre_mm: Vec3,
    axis: Axis,
}

#[derive(Clone)]
struct ScaleCorrection {
    centre_mm: Vec3,
    axis: Option<Axis>,
}

#[derive(Clone)]
struct BoxFace {
    element: ElementId,
    corners: [usize; 4],
    color: Color32,
}

#[derive(Clone, Debug, PartialEq)]
struct RenderBox {
    definition_id: DefinitionId,
    profile_feature_id: FeatureId,
    extrusion_feature_id: Option<FeatureId>,
    instance_path: InstancePath,
    origin_mm: Vec3,
    size_mm: Vec3,
}

/// Any closed profile can cut or pocket a solid; the exact evaluator decides
/// whether the result is valid.
fn is_closed_profile(segments: &[ProfileSegment], closed: bool) -> bool {
    closed && !segments.is_empty()
}

fn exact_solid_tool_feature_id(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> Option<FeatureId> {
    let terminals = exact_body_terminal_features(snapshot, definition_id).ok()?;
    let mut terminal_ids = terminals.values().copied();
    let feature_id = terminal_ids.next()?;
    if terminal_ids.next().is_some() {
        return None;
    }
    snapshot.solid_tool_feature_clone_count(feature_id).ok()?;
    Some(feature_id)
}

#[derive(Clone, Debug, PartialEq)]
enum PushPullPlanningPlan {
    Append,
    TipReplacement {
        document_id: DocumentId,
        parent_revision: u64,
        parent_digest: String,
        superseded_revision: u64,
        superseded_digest: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct PushPullSourcePlan {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    planning: PushPullPlanningPlan,
    target: SelectionId,
    topological_selection: Option<SnapshotBoundTopologicalSelection>,
    topological_reference: Option<TopologicalElementRef>,
    target_box: RenderBox,
}

#[derive(Clone, Debug, PartialEq)]
struct PushPullPreviewPlan {
    source: PushPullSourcePlan,
    principal: ProposalPrincipal,
    distance_expression: String,
    distance_mm_bits: u64,
    current_extent_mm_bits: u64,
    new_extent_mm_bits: u64,
    commands: Vec<CanonicalCommand>,
    preview_box: RenderBox,
    shared_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct EphemeralBoxPreview {
    plan: PushPullPreviewPlan,
    batch: CommandBatch,
}

/// Everything [`KetchupApp::has_preview`] re-plans the Push/Pull from.
/// Re-planning touches the whole document and painting asks once per face,
/// so the answer is kept until one of these changes.
#[derive(PartialEq)]
struct PreviewCheckKey {
    document_id: DocumentId,
    revision_id: u64,
    canonical_digest: String,
    planning: Option<PushPullPlanningPlan>,
    exact_results_stamps: (u64, u64),
    primary: Option<SelectionId>,
    selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    topological: Vec<(SelectionId, SnapshotBoundTopologicalSelection)>,
    preview: EphemeralBoxPreview,
}

#[derive(Clone)]
enum SmartPushPullPlanning {
    Append,
    TipReplacement(TipReplacementParent),
}

#[derive(Clone)]
enum SmartPushPullProposal {
    Append(Proposal),
    TipReplacement(TipReplacementProposal),
}

impl SmartPushPullProposal {
    fn batch(&self) -> &CommandBatch {
        match self {
            Self::Append(proposal) => proposal.batch(),
            Self::TipReplacement(proposal) => proposal.batch(),
        }
    }

    fn command_digest(&self) -> &str {
        match self {
            Self::Append(proposal) => proposal.command_digest(),
            Self::TipReplacement(proposal) => proposal.command_digest(),
        }
    }

    #[cfg(test)]
    fn principal(&self) -> ProposalPrincipal {
        match self {
            Self::Append(proposal) => proposal.principal(),
            Self::TipReplacement(proposal) => proposal.principal(),
        }
    }

    fn is_current(&self, snapshot: &Snapshot) -> bool {
        match self {
            Self::Append(proposal) => {
                proposal.document_id() == snapshot.document_id()
                    && proposal.provenance_revision() == snapshot.revision_id()
                    && proposal.provenance_digest() == snapshot.canonical_digest()
            }
            Self::TipReplacement(proposal) => {
                proposal.document_id() == snapshot.document_id()
                    && proposal.superseded_revision() == snapshot.revision_id()
                    && proposal.superseded_digest() == snapshot.canonical_digest()
            }
        }
    }

    fn preview(&self, document: &DocumentStore) -> Option<Snapshot> {
        match self {
            Self::Append(proposal) => document.preview_batch(proposal.batch()).ok(),
            Self::TipReplacement(proposal) => {
                document.preview_tip_replacement_proposal(proposal).ok()
            }
        }
    }

    fn commit(&self, document: &mut DocumentStore) -> Result<(), ManualProposalCommitError> {
        match self {
            Self::Append(proposal) => document
                .commit_verified_proposal(proposal)
                .map(|_| ())
                .map_err(ManualProposalCommitError::Append),
            Self::TipReplacement(proposal) => document
                .commit_tip_replacement_proposal(proposal)
                .map(|_| ())
                .map_err(ManualProposalCommitError::TipReplacement),
        }
    }
}

/// The store's refusal to commit a manual Push/Pull proposal, by proposal kind.
#[derive(Debug)]
enum ManualProposalCommitError {
    Append(ProposalCommitError),
    TipReplacement(ketchup_model::document::TipReplacementProposalError),
}

impl std::fmt::Display for ManualProposalCommitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Append(error) => error.fmt(formatter),
            Self::TipReplacement(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ManualProposalCommitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Append(error) => error.source(),
            Self::TipReplacement(error) => error.source(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignMode {
    Minimum,
    Center,
    Maximum,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistributionMode {
    Centers,
    EqualGaps,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangularPatternSpec {
    pub primary_axis: Axis,
    pub primary_spacing_mm: f64,
    pub primary_count: usize,
    pub secondary_axis: Axis,
    pub secondary_spacing_mm: f64,
    pub secondary_count: usize,
}

const MAX_PATTERN_COUNT: usize = 10_000;

#[derive(Clone, Debug, PartialEq)]
struct SolidToolSourcePlan {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    operation: BooleanOperation,
    keep_tool: bool,
    target_selection: SelectionId,
    target_name: String,
    target_transform: Transform,
    target_parent: Option<GroupId>,
    target_tags: BTreeSet<TagId>,
    target_visible: bool,
    target_box: RenderBox,
    target_feature_id: FeatureId,
    tool_selection: SelectionId,
    tool_name: String,
    tool_transform: Transform,
    tool_parent: Option<GroupId>,
    tool_tags: BTreeSet<TagId>,
    tool_visible: bool,
    tool_box: RenderBox,
    tool_feature_id: FeatureId,
    result_definition_id: DefinitionId,
    result_feature_ids: Vec<FeatureId>,
}

impl SolidToolSourcePlan {
    // Exact/render bounds are derived products and deliberately do not define identity.
    fn same_canonical_identity(&self, other: &Self) -> bool {
        self.source_document_id == other.source_document_id
            && self.source_revision == other.source_revision
            && self.source_digest == other.source_digest
            && self.source_primary == other.source_primary
            && self.source_selected_group == other.source_selected_group
            && self.edit_context == other.edit_context
            && self.operation == other.operation
            && self.keep_tool == other.keep_tool
            && self.target_selection == other.target_selection
            && self.target_name == other.target_name
            && self.target_transform == other.target_transform
            && self.target_parent == other.target_parent
            && self.target_tags == other.target_tags
            && self.target_visible == other.target_visible
            && self.target_feature_id == other.target_feature_id
            && self.tool_selection == other.tool_selection
            && self.tool_name == other.tool_name
            && self.tool_transform == other.tool_transform
            && self.tool_parent == other.tool_parent
            && self.tool_tags == other.tool_tags
            && self.tool_visible == other.tool_visible
            && self.tool_feature_id == other.tool_feature_id
            && self.result_definition_id == other.result_definition_id
            && self.result_feature_ids == other.result_feature_ids
    }
}

#[derive(Clone, Debug, PartialEq)]
struct SolidToolPreviewPlan {
    source: SolidToolSourcePlan,
    command: CanonicalCommand,
    preview_box: RenderBox,
    hidden_occurrences: BTreeSet<OccurrenceId>,
    selection_after: SelectionId,
    committed_digest_key: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
enum OccurrenceCanonicalPreviewPlan {
    Alignment(Box<OccurrenceAlignmentPlan>),
    Distribution(OccurrenceDistributionPlan),
    LinearPattern(LinearPatternPlan),
    RectangularPattern(RectangularPatternPlan),
    CircularPattern(CircularPatternPlan),
}

#[derive(Clone, Debug, PartialEq)]
struct OccurrenceOperationPreview {
    source_revision: u64,
    command_digest: String,
    batch: CommandBatch,
    boxes: BTreeMap<OccurrenceId, RenderBox>,
    hidden_occurrences: BTreeSet<OccurrenceId>,
    selection_after: Option<SelectionId>,
    committed_digest_key: &'static str,
    canonical_plan: Option<OccurrenceCanonicalPreviewPlan>,
    solid_tool_plan: Option<SolidToolPreviewPlan>,
}

#[derive(Clone, Debug, PartialEq)]
struct RevolveSourcePlan {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    definition_id: DefinitionId,
    profile_feature_id: FeatureId,
    profile_kind: FeatureKind,
    world_transform: Transform,
    translation_mm: Vec3,
    plane_z: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct RevolveToolState {
    source: RevolveSourcePlan,
    axis_start_mm: Option<[f64; 2]>,
    axis_end_mm: Option<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
struct RevolvePreviewPlan {
    source: RevolveSourcePlan,
    generated_feature_id: FeatureId,
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    angle_degrees_bits: u64,
    command: CanonicalCommand,
    exact_request: ExactBRepGraph,
}

#[derive(Clone, Debug, PartialEq)]
struct RevolvePreview {
    plan: RevolvePreviewPlan,
    batch: CommandBatch,
}

#[derive(Clone, Debug, PartialEq)]
struct PlanarOffsetSourcePlan {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    definition_id: DefinitionId,
    profile_feature_id: FeatureId,
    profile_kind: FeatureKind,
    world_transform: Transform,
}

#[derive(Clone, Debug, PartialEq)]
struct PlanarOffsetPreviewPlan {
    source: PlanarOffsetSourcePlan,
    generated_feature_id: FeatureId,
    distance_expression: String,
    distance_mm_bits: u64,
    command: CanonicalCommand,
    exact_graph: ExactBRepGraph,
}

#[derive(Clone, Debug, PartialEq)]
struct PlanarOffsetPreview {
    plan: PlanarOffsetPreviewPlan,
    batch: CommandBatch,
}

#[derive(Clone, Debug, PartialEq)]
struct SweepSourcePlan {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    definition_id: DefinitionId,
    profile_feature_id: FeatureId,
    profile_kind: FeatureKind,
    path_feature_id: FeatureId,
    path_kind: FeatureKind,
    world_transform: Transform,
}

#[derive(Clone, Debug, PartialEq)]
struct SweepPreviewPlan {
    source: SweepSourcePlan,
    generated_feature_id: FeatureId,
    command: CanonicalCommand,
    exact_graph: ExactBRepGraph,
}

#[derive(Clone, Debug, PartialEq)]
struct SweepPreview {
    plan: SweepPreviewPlan,
    batch: CommandBatch,
}

#[derive(Clone, Debug, PartialEq)]
struct LoftSourcePlan {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    definition_id: DefinitionId,
    sections: Vec<LoftSection>,
    profile_kinds: Vec<(FeatureId, FeatureKind)>,
    world_transform: Transform,
}

#[derive(Clone, Debug, PartialEq)]
struct LoftPreviewPlan {
    source: LoftSourcePlan,
    generated_feature_id: FeatureId,
    command: CanonicalCommand,
    exact_graph: ExactBRepGraph,
}

#[derive(Clone, Debug, PartialEq)]
struct LoftPreview {
    plan: LoftPreviewPlan,
    batch: CommandBatch,
}

#[derive(Clone, Debug, PartialEq)]
struct GeneralFinishSourcePlan {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    definition_id: DefinitionId,
    target_feature_id: FeatureId,
    target_feature_kind: FeatureKind,
    topological_selections: Vec<SnapshotBoundTopologicalSelection>,
    kind: GeneralFinishKind,
    world_transform: Transform,
    exact_graph: ExactBRepGraph,
}

#[derive(Clone, Debug, PartialEq)]
struct GeneralFinishPreviewPlan {
    source: GeneralFinishSourcePlan,
    generated_feature_id: FeatureId,
    amount_mm_bits: u64,
    command: CanonicalCommand,
    exact_graph: ExactBRepGraph,
}

#[derive(Clone, Debug, PartialEq)]
struct GeneralFinishPreview {
    plan: GeneralFinishPreviewPlan,
    batch: CommandBatch,
}

enum ProjectedPolygon {
    Triangle([Pos2; 3]),
    Quad([Pos2; 4]),
    /// What a section plane leaves of a face.
    Polygon(Vec<Pos2>),
}

impl ProjectedPolygon {
    fn points(&self) -> &[Pos2] {
        match self {
            Self::Triangle(points) => points,
            Self::Quad(points) => points,
            Self::Polygon(points) => points,
        }
    }
}

struct ProjectedFace {
    selection: SelectionId,
    polygon: ProjectedPolygon,
    color: Color32,
    depth: f64,
    previewed: bool,
    out_of_context: bool,
}

struct ProjectedEdge {
    selection: SelectionId,
    points: [Pos2; 2],
    depth: f64,
    dominant_axis: Option<Axis>,
}

fn definition_mesh_body(snapshot: &Snapshot, definition_id: DefinitionId) -> Option<&MeshBodySpec> {
    snapshot
        .definition(definition_id)?
        .feature_ids()
        .iter()
        .find_map(|feature_id| match snapshot.feature(*feature_id)?.kind() {
            FeatureKind::MeshBody(mesh) => Some(mesh),
            _ => None,
        })
}

enum CurrentVisibleMeshSource {
    Exact(Arc<ExactBodyPackage>),
    Canonical {
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
        mesh: Box<MeshBodySpec>,
    },
}

impl CurrentVisibleMeshSource {
    fn as_export_source(&self) -> MeshExportSource<'_> {
        match self {
            Self::Exact(package) => MeshExportSource::Exact(package),
            Self::Canonical {
                definition_id,
                producer_feature_id,
                mesh,
            } => MeshExportSource::Canonical {
                definition_id: *definition_id,
                producer_feature_id: *producer_feature_id,
                mesh,
            },
        }
    }
}

struct CurrentVisibleMesh {
    source: CurrentVisibleMeshSource,
    occurrence: SceneOccurrence,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ArcGeometry {
    start: Vec3,
    end: Vec3,
    center: Vec3,
    clockwise: bool,
}

type ExactArcProfileGeometry = ([f64; 2], [f64; 2], [f64; 2], bool);
pub type LoftPreviewParameters = (Vec<(FeatureId, f64)>, [[f64; 3]; 2]);

/// Straight pieces that draw one curve of a drawing-tool preview.
const PREVIEW_CURVE_SEGMENTS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveTool {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    Polygon,
    Ellipse,
    Spline,
    Mirror,
    SolidSubtract,
    SolidTrim,
    SolidUnion,
    SolidIntersect,
    SolidSplit,
    PlanarOffset,
    Helix,
    Sweep,
    Loft,
    Revolve,
    Shell,
    Fillet,
    Chamfer,
    PushPull,
    Move,
    Rotate,
    Scale,
    Measure,
    Orbit,
    Pan,
    ZoomWindow,
}

impl ActiveTool {
    const fn label_key(self) -> &'static str {
        match self {
            Self::Select => "tool-select",
            Self::Line => "tool-line",
            Self::Rectangle => "tool-rectangle",
            Self::Circle => "tool-circle",
            Self::Arc => "tool-arc",
            Self::Polygon => "tool-polygon",
            Self::Ellipse => "tool-ellipse",
            Self::Spline => "tool-spline",
            Self::Mirror => "tool-mirror",
            Self::SolidSubtract => "solid-tool-subtract",
            Self::SolidTrim => "solid-tool-trim",
            Self::SolidUnion => "solid-tool-union",
            Self::SolidIntersect => "solid-tool-intersect",
            Self::SolidSplit => "solid-tool-split",
            Self::PlanarOffset => "feature-planar-offset",
            Self::Helix => "feature-helix",
            Self::Sweep => "feature-sweep",
            Self::Loft => "feature-loft",
            Self::Revolve => "feature-revolve",
            Self::Shell => "feature-shell",
            Self::Fillet => "feature-fillet",
            Self::Chamfer => "feature-chamfer",
            Self::PushPull => "tool-push-pull",
            Self::Move => "tool-move",
            Self::Rotate => "tool-rotate",
            Self::Scale => "tool-scale",
            Self::Measure => "tool-measure",
            Self::Orbit => "tool-orbit",
            Self::Pan => "tool-pan",
            Self::ZoomWindow => "view-zoom-window",
        }
    }

    const fn hint_key(self) -> &'static str {
        match self {
            Self::Select => "hint-select",
            Self::Line => "hint-line",
            Self::Rectangle => "hint-rectangle",
            Self::Circle => "hint-circle",
            Self::Arc => "hint-arc",
            Self::Polygon => "hint-polygon",
            Self::Ellipse => "hint-ellipse",
            Self::Spline => "hint-spline",
            Self::Mirror => "hint-mirror",
            Self::SolidSubtract => "hint-solid-subtract",
            Self::SolidTrim => "hint-solid-trim",
            Self::SolidUnion => "hint-solid-union",
            Self::SolidIntersect => "hint-solid-intersect",
            Self::SolidSplit => "hint-solid-split",
            Self::PlanarOffset => "hint-planar-offset",
            Self::Helix => "hint-helix",
            Self::Sweep => "hint-sweep",
            Self::Loft => "hint-loft",
            Self::Revolve => "hint-revolve",
            Self::Shell => "hint-shell",
            Self::Fillet => "hint-fillet",
            Self::Chamfer => "hint-chamfer",
            Self::PushPull => "hint-push-pull",
            Self::Move => "hint-move",
            Self::Rotate => "hint-rotate",
            Self::Scale => "hint-scale",
            Self::Measure => "hint-measure",
            Self::Orbit => "hint-orbit",
            Self::Pan => "hint-pan",
            Self::ZoomWindow => "hint-zoom-window",
        }
    }
}

/// Every command the designed shell can dispatch.
///
/// The variant is the stable identity of a command; its visible label is a
/// localization key resolved at paint time. Acceptance tests address widgets by
/// variant and resolve the expected label through the same catalog, so neither a
/// translation change nor an icon-only presentation can break them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppCommand {
    New,
    Open,
    Save,
    SaveAs,
    ImportMeshStl,
    ImportDrawingDxf,
    ImportExactStep,
    ImportExactIges,
    ImportSketchupScene,
    ImportBlenderGlb,
    ConvertSelectedMeshToExact,
    ExportDrawingDxf,
    ExportExactStep,
    ExportExactIges,
    ExportMeshStl,
    ExportPrintThreeMf,
    ExportBlenderGlb,
    ExportViewer,
    OpenViewerComments,
    ExportGeneralFabrication,
    ExportWeldmentCutList,
    ExportProjectDrawings,
    ExportSheetMetalManufacturing,
    ExportHomagMpr,
    ReviewCamExport,
    ReviewStaticFea,
    ReviewLocalPdm,
    ExportHundeggerBtlx,
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    Polygon,
    Ellipse,
    Spline,
    Mirror,
    SolidSubtract,
    SolidTrim,
    SolidUnion,
    SolidIntersect,
    SolidSplit,
    PlanarOffset,
    Helix,
    Sweep,
    Loft,
    Revolve,
    Shell,
    Fillet,
    Chamfer,
    PushPull,
    Move,
    Rotate,
    Scale,
    Measure,
    Orbit,
    Pan,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    Duplicate,
    Delete,
    Deselect,
    SelectAll,
    InvertSelection,
    Group,
    Ungroup,
    MakeComponent,
    MakeUnique,
    ReplaceComponent,
    SelectAllInstances,
    AssignTag,
    AlignOccurrences,
    DistributeOccurrences,
    LinearPattern,
    RectangularPattern,
    CircularPattern,
    GroundOccurrence,
    UngroundOccurrence,
    RenameOccurrence,
    RenameDefinition,
    PurgeUnused,
    Hide,
    HideOthers,
    Unhide,
    UnhideAll,
    PreviousView,
    HomeView,
    ViewIso,
    ViewTop,
    ViewBottom,
    ViewFront,
    ViewBack,
    ViewRight,
    ViewLeft,
    View(ViewFlag),
    ViewShaded,
    ViewProjection,
    ZoomFit,
    ZoomSelection,
    CenterSelection,
    ZoomWindow,
    ZoomIn,
    ZoomOut,
    Shortcuts,
    CommandSearch,
    About,
    MaterialTakeoff,
}

/// How the viewport maps the model onto the screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionMode {
    /// Converging view — parallel edges meet, the way an eye sees a room.
    Perspective,
    /// Parallel view — parallel edges stay parallel, the way a drawing measures.
    Parallel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CameraViewState {
    projection_mode: ProjectionMode,
    yaw: f32,
    pitch: f32,
    target_z: f64,
    zoom: f32,
    pan: Vec2,
    view: ViewSettings,
}

impl ProjectionMode {
    const fn toggled(self) -> Self {
        match self {
            Self::Perspective => Self::Parallel,
            Self::Parallel => Self::Perspective,
        }
    }

    const fn label_key(self) -> &'static str {
        match self {
            Self::Perspective => "view-projection-perspective",
            Self::Parallel => "view-projection-parallel",
        }
    }
}

#[derive(Clone, Copy)]
struct CommandSpec {
    id: AppCommand,
    label_key: &'static str,
    tool: Option<ActiveTool>,
    implemented: bool,
}

struct CommandRegistry;

#[derive(Clone, Debug, Eq, PartialEq)]
enum EditContext {
    Group(GroupId),
    Definition {
        definition_id: DefinitionId,
        instance_path: InstancePath,
    },
}

/// Feature edges of one body, kept between frames.
///
/// `identity` names the geometry the edges were derived from, so a recomputed
/// body invalidates them without a document-wide cache flush.
struct OverlayEdges {
    identity: String,
    edges: Arc<Vec<([u32; 2], Vec<u32>)>>,
}

struct InteractionProjectionCache {
    document_id: DocumentId,
    revision_id: u64,
    canonical_digest: String,
    edit_context: Vec<EditContext>,
    exact_results_stamp: (u64, u64),
    canonical: ketchup_interaction::projection::InteractionProjection,
    exact: ExactInteractionProjection,
    mesh: MeshInteractionProjection,
    boxes: ketchup_interaction::InteractionScene,
    proxies: ketchup_interaction::InteractionScene,
    render_boxes: std::cell::OnceCell<Vec<RenderBox>>,
    frame_bounds: std::cell::OnceCell<Vec<(Vec3, Vec3)>>,
    snap_geometry: std::cell::OnceCell<scene_snapping::SceneSnapGeometry>,
}

#[derive(Default)]
struct SelectionState {
    occurrences: BTreeSet<InstancePath>,
    primary: Option<SelectionId>,
    exact_references: BTreeMap<InstancePath, SelectionId>,
    topological: Vec<(SelectionId, SnapshotBoundTopologicalSelection)>,
    selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RootOccurrenceSelectionError {
    Nested { paths: BTreeSet<InstancePath> },
    Mixed { paths: BTreeSet<InstancePath> },
}

impl SelectionState {
    fn clear(&mut self) {
        self.occurrences.clear();
        self.primary = None;
        self.exact_references.clear();
        self.topological.clear();
        self.selected_group = None;
    }

    fn contains(&self, instance_path: &InstancePath) -> bool {
        self.occurrences.contains(instance_path)
    }

    fn select_exact(&mut self, selection: SelectionId, additive: bool) {
        self.topological.clear();
        let instance_path = selection.instance_path.clone();
        if additive && self.occurrences.contains(&instance_path) {
            self.occurrences.remove(&instance_path);
            self.exact_references.remove(&instance_path);
            if self
                .primary
                .as_ref()
                .is_some_and(|primary| primary.instance_path == selection.instance_path)
            {
                self.primary = self
                    .occurrences
                    .iter()
                    .find_map(|path| self.exact_references.get(path).cloned());
            }
            self.selected_group = None;
            return;
        }
        if !additive {
            self.occurrences.clear();
            self.exact_references.clear();
        }
        self.occurrences.insert(instance_path.clone());
        self.exact_references
            .insert(instance_path, selection.clone());
        if !additive || self.primary.is_none() {
            self.primary = Some(selection);
        }
        self.selected_group = None;
    }

    fn select_topological(
        &mut self,
        selection: SelectionId,
        topological: SnapshotBoundTopologicalSelection,
        additive: bool,
    ) -> bool {
        let target = topological.target();
        if additive {
            if self.topological.is_empty() {
                self.select_exact(selection.clone(), false);
                self.topological.push((selection, topological));
                return true;
            }
            if self.primary.as_ref().is_none_or(|primary| {
                primary.definition_id != selection.definition_id
                    || primary.instance_path != selection.instance_path
            }) || self.topological.first().is_some_and(|(_, existing)| {
                existing.target().reference.kind != target.reference.kind
                    || existing.target().reference.producer_feature_id
                        != target.reference.producer_feature_id
            }) {
                return false;
            }
            if let Some(index) = self
                .topological
                .iter()
                .position(|(_, existing)| existing.target() == target)
            {
                self.topological.remove(index);
                if self.topological.is_empty() {
                    self.occurrences.clear();
                    self.primary = None;
                    self.exact_references.clear();
                    self.selected_group = None;
                } else {
                    self.primary = Some(self.topological[0].0.clone());
                }
                return true;
            }
            if self.topological.len() == ketchup_model::tolerance::limits::FEATURE_REFERENCES {
                return false;
            }
            self.topological.push((selection, topological));
            self.topological.sort_by(|(_, left), (_, right)| {
                left.target().reference.cmp(&right.target().reference)
            });
            self.primary = Some(self.topological[0].0.clone());
            return true;
        }

        self.select_exact(selection.clone(), false);
        self.topological.push((selection, topological));
        true
    }

    fn select_path(&mut self, instance_path: InstancePath, additive: bool) {
        if additive && self.occurrences.contains(&instance_path) {
            self.occurrences.remove(&instance_path);
        } else {
            if !additive {
                self.occurrences.clear();
            }
            self.occurrences.insert(instance_path);
        }
        self.primary = None;
        self.exact_references.clear();
        self.topological.clear();
        self.selected_group = None;
    }

    fn select_occurrence(&mut self, occurrence_id: OccurrenceId, additive: bool) {
        self.select_path(InstancePath::root(occurrence_id), additive);
    }
}

#[derive(Clone)]
struct OutlinerOccurrence {
    instance_path: InstancePath,
    name: String,
    #[cfg(test)]
    position: String,
    visible: bool,
    parent: Option<GroupId>,
}

#[derive(Clone)]
struct OutlinerGroup {
    id: GroupId,
    name: String,
    member_count: usize,
}

#[derive(Clone)]
struct OutlinerDefinition {
    id: DefinitionId,
    name: String,
    specification: String,
    occurrences: Vec<OutlinerOccurrence>,
}

#[derive(Clone)]
struct ClassificationCategoryRow {
    id: ClassificationCategoryId,
    name: String,
    occurrence_count: usize,
}

#[derive(Clone)]
struct ClassificationDimensionRow {
    id: ClassificationDimensionId,
    name: String,
    categories: Vec<ClassificationCategoryRow>,
}

fn assistant_proposal_prepare_rejection(
    error: ProposalPrepareError,
    operation: &str,
    target: &str,
) -> AssistantRejection {
    match error {
        ProposalPrepareError::Canonical(error) => {
            assistant_canonical_rejection(error, operation, target)
        }
        ProposalPrepareError::HostBudgetExceeded => assistant_rejection(
            AssistantRejectionPhase::ProposalPlanning,
            "planning.host_budget_exceeded",
            operation,
            target,
            "The proposal exceeds the host work budget.",
            "Reduce the number of requested edits and retry.",
            true,
        ),
        ProposalPrepareError::RequestedBudgetExceeded { .. } => assistant_rejection(
            AssistantRejectionPhase::ProposalPlanning,
            "planning.requested_budget_exceeded",
            operation,
            target,
            error.to_string(),
            "Split the request into smaller atomic edits and retry.",
            true,
        ),
        ProposalPrepareError::Confirmation(error) => assistant_rejection(
            AssistantRejectionPhase::ProposalPlanning,
            "planning.confirmation_requirement_invalid",
            operation,
            target,
            error.to_string(),
            "Use the required review and confirmation path for this operation.",
            false,
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CopySourcePlan {
    occurrence_ids: BTreeSet<OccurrenceId>,
    occurrence_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct CutClipboardOccurrence {
    color: Option<[u8; 3]>,
    source_occurrence_id: OccurrenceId,
    definition_id: DefinitionId,
    transform: Transform,
    parent: Option<GroupId>,
    tags: BTreeSet<TagId>,
    visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct CutSourcePlan {
    source_revision: u64,
    occurrence_ids: BTreeSet<OccurrenceId>,
    occurrence_count: usize,
    clipboard: Vec<CutClipboardOccurrence>,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug, PartialEq)]
struct PasteSourcePlan {
    source_revision: u64,
    source_occurrence_ids: BTreeSet<OccurrenceId>,
    source_occurrence_count: usize,
    commands: Vec<CanonicalCommand>,
    pasted: Vec<(OccurrenceId, DefinitionId)>,
}

#[derive(Clone, Debug, PartialEq)]
struct DuplicateSourcePlan {
    source_revision: u64,
    source_occurrence_ids: BTreeSet<OccurrenceId>,
    source_occurrence_count: usize,
    commands: Vec<CanonicalCommand>,
    duplicated: Vec<(OccurrenceId, DefinitionId)>,
}

#[derive(Clone, Debug, PartialEq)]
struct DeleteSelectionSourcePlan {
    source_revision: u64,
    occurrence_ids: BTreeSet<OccurrenceId>,
    occurrence_count: usize,
    group_ids: BTreeSet<GroupId>,
    group_count: usize,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DeselectSourcePlan {
    source_revision: u64,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    primary: Option<SelectionId>,
    selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectAllSourcePlan {
    source_revision: u64,
    source_occurrence_paths: BTreeSet<InstancePath>,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    target_instance_paths: BTreeSet<InstancePath>,
    target_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InvertSelectionSourcePlan {
    source_revision: u64,
    source_occurrence_paths: BTreeSet<InstancePath>,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    active_instance_paths: BTreeSet<InstancePath>,
    target_instance_paths: BTreeSet<InstancePath>,
    target_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectAllInstancesSourcePlan {
    source_revision: u64,
    source_instance_paths: BTreeSet<InstancePath>,
    source_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    source_instance_path: InstancePath,
    definition_id: DefinitionId,
    definition_name: String,
    target_instance_paths: BTreeSet<InstancePath>,
    target_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PurgeUnusedSourcePlan {
    definition_ids: BTreeSet<DefinitionId>,
    definition_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct GroupSelectionSourcePlan {
    source_revision: u64,
    group_id: GroupId,
    occurrence_ids: BTreeSet<OccurrenceId>,
    occurrence_count: usize,
    edit_context: Vec<EditContext>,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug, PartialEq)]
struct UngroupSelectionSourcePlan {
    source_revision: u64,
    group_id: GroupId,
    occurrence_ids: BTreeSet<OccurrenceId>,
    occurrence_count: usize,
    item_count: usize,
    edit_context: Vec<EditContext>,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug, PartialEq)]
struct MakeComponentSourcePlan {
    source_revision: u64,
    group_id: GroupId,
    group_name: String,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    primary: Option<SelectionId>,
    selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    component_name: String,
    subtree_occurrence_count: usize,
    new_definition_id: DefinitionId,
    new_occurrence_id: OccurrenceId,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug, PartialEq)]
struct SelectionVisibilitySourcePlan {
    source_revision: u64,
    source_digest: String,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    source_visibility: BTreeMap<OccurrenceId, bool>,
    changed_occurrence_ids: BTreeSet<OccurrenceId>,
    target_visible: bool,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug, PartialEq)]
struct HideOthersSourcePlan {
    source_revision: u64,
    source_digest: String,
    selected_occurrence_ids: BTreeSet<OccurrenceId>,
    edit_context: Vec<EditContext>,
    hidden_occurrence_ids: BTreeSet<OccurrenceId>,
    occurrence_count: usize,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug, PartialEq)]
struct UnhideAllSourcePlan {
    source_revision: u64,
    source_digest: String,
    edit_context: Vec<EditContext>,
    hidden_occurrence_ids: BTreeSet<OccurrenceId>,
    occurrence_count: usize,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug, PartialEq)]
struct GroundedOccurrenceSourcePlan {
    source_revision: u64,
    source_digest: String,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    occurrence_id: OccurrenceId,
    source_grounded: bool,
    target_grounded: bool,
    command: CanonicalCommand,
}

#[derive(Clone, Debug, PartialEq)]
struct MakeUniqueSourcePlan {
    source_revision: u64,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_id: OccurrenceId,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    source_definition_id: DefinitionId,
    source_definition_name: String,
    visible_peer_ids: BTreeSet<OccurrenceId>,
    visible_peer_count: usize,
    command: CloneDefinitionPlan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DefinitionRenameSourcePlan {
    source_revision: u64,
    source_digest: String,
    instance_paths: BTreeSet<InstancePath>,
    instance_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    definition_id: DefinitionId,
    original_name: String,
}

#[derive(Clone, Debug, PartialEq)]
struct DefinitionRenamePlan {
    source: DefinitionRenameSourcePlan,
    target_name: String,
    command: CanonicalCommand,
}

#[derive(Clone, Debug)]
struct PendingDefinitionRename {
    source: DefinitionRenameSourcePlan,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OccurrenceRenameSourcePlan {
    source_revision: u64,
    source_digest: String,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    occurrence_id: OccurrenceId,
    original_name: String,
}

#[derive(Clone, Debug, PartialEq)]
struct OccurrenceRenamePlan {
    source: OccurrenceRenameSourcePlan,
    target_name: String,
    command: CanonicalCommand,
}

#[derive(Clone, Debug)]
struct PendingOccurrenceRename {
    source: OccurrenceRenameSourcePlan,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ComponentReplacementSourcePlan {
    source_revision: u64,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    occurrence_id: OccurrenceId,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    source_definition_id: DefinitionId,
    source_definition_name: String,
    candidate_definitions: BTreeMap<DefinitionId, String>,
    candidate_count: usize,
    initial_target_definition_id: DefinitionId,
}

#[derive(Clone, Debug, PartialEq)]
struct ComponentReplacementPlan {
    source: ComponentReplacementSourcePlan,
    target_definition_id: DefinitionId,
    target_definition_name: String,
    command: CanonicalCommand,
}

#[derive(Clone, Debug)]
struct PendingComponentReplacement {
    source: ComponentReplacementSourcePlan,
    target_definition_id: DefinitionId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TagCreationSourcePlan {
    source_revision: u64,
    source_digest: String,
    tags: BTreeMap<TagId, (String, bool)>,
    id: TagId,
    occurrence_ids: Option<BTreeSet<OccurrenceId>>,
}

#[derive(Clone, Debug, PartialEq)]
struct TagCreationPlan {
    source: TagCreationSourcePlan,
    target_name: String,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug)]
struct PendingTagCreation {
    source: TagCreationSourcePlan,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TagDeletionSourcePlan {
    source_revision: u64,
    source_digest: String,
    tags: BTreeMap<TagId, (String, bool)>,
    id: TagId,
    original_name: String,
    original_visible: bool,
    occurrence_ids: BTreeSet<OccurrenceId>,
    /// Parts inside definitions that carry the tag, e.g. given it by a program.
    local_occurrences: BTreeSet<ketchup_model::document::LocalOccurrenceKey>,
}

#[derive(Clone, Debug, PartialEq)]
struct TagDeletionPlan {
    source: TagDeletionSourcePlan,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug)]
struct PendingTagDeletion {
    source: TagDeletionSourcePlan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TagClearSourcePlan {
    source_revision: u64,
    source_digest: String,
    tags: BTreeMap<TagId, (String, bool)>,
    id: TagId,
    original_name: String,
    original_visible: bool,
    occurrence_ids: BTreeSet<OccurrenceId>,
}

#[derive(Clone, Debug, PartialEq)]
struct TagClearPlan {
    source: TagClearSourcePlan,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug)]
struct PendingTagClear {
    source: TagClearSourcePlan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TagRenameSourcePlan {
    source_revision: u64,
    source_digest: String,
    tags: BTreeMap<TagId, (String, bool)>,
    id: TagId,
    original_name: String,
    original_visible: bool,
    occurrence_ids: BTreeSet<OccurrenceId>,
}

#[derive(Clone, Debug, PartialEq)]
struct TagRenamePlan {
    source: TagRenameSourcePlan,
    target_name: String,
    command: CanonicalCommand,
}

#[derive(Clone, Debug)]
struct PendingTagRename {
    source: TagRenameSourcePlan,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TagAssignmentSourcePlan {
    source_revision: u64,
    source_digest: String,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    source_tags: BTreeMap<OccurrenceId, BTreeSet<TagId>>,
    available_tags: BTreeMap<TagId, String>,
    initial_tags: BTreeSet<TagId>,
}

#[derive(Clone, Debug, PartialEq)]
struct TagAssignmentPlan {
    source: TagAssignmentSourcePlan,
    target_tags: BTreeSet<TagId>,
    target_tag_name: String,
    changed_occurrence_ids: BTreeSet<OccurrenceId>,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug)]
struct PendingTagAssignment {
    source: TagAssignmentSourcePlan,
    target_tags: BTreeSet<TagId>,
}

#[derive(Clone, Debug, PartialEq)]
struct OccurrenceAlignmentSourcePlan {
    source_revision: u64,
    source_digest: String,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    moving_id: OccurrenceId,
    moving_definition_id: DefinitionId,
    moving_name: String,
    moving_transform: Transform,
    moving_parent: Option<GroupId>,
    moving_tags: BTreeSet<TagId>,
    moving_visible: bool,
    moving_box: RenderBox,
    reference_id: OccurrenceId,
    reference_definition_id: DefinitionId,
    reference_name: String,
    reference_transform: Transform,
    reference_parent: Option<GroupId>,
    reference_tags: BTreeSet<TagId>,
    reference_visible: bool,
    reference_box: RenderBox,
}

#[derive(Clone, Debug, PartialEq)]
struct OccurrenceAlignmentPlan {
    source: OccurrenceAlignmentSourcePlan,
    axis: Axis,
    mode: AlignMode,
    moving_coordinate_mm: f64,
    reference_coordinate_mm: f64,
    offset_mm: f64,
    command: CanonicalCommand,
    preview_box: RenderBox,
}

#[derive(Clone, Debug)]
struct PendingOccurrenceAlign {
    source: OccurrenceAlignmentSourcePlan,
    axis: Axis,
    mode: AlignMode,
    preview_plan: Option<OccurrenceAlignmentPlan>,
}

#[derive(Clone, Debug, PartialEq)]
struct OccurrenceDistributionSourceItem {
    definition_id: DefinitionId,
    name: String,
    transform: Transform,
    parent: Option<GroupId>,
    tags: BTreeSet<TagId>,
    visible: bool,
    render_box: RenderBox,
}

#[derive(Clone, Debug, PartialEq)]
struct OccurrenceDistributionSourcePlan {
    source_revision: u64,
    source_digest: String,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    occurrences: BTreeMap<OccurrenceId, OccurrenceDistributionSourceItem>,
}

#[derive(Clone, Debug, PartialEq)]
struct OccurrenceDistributionPlan {
    source: OccurrenceDistributionSourcePlan,
    axis: Axis,
    mode: DistributionMode,
    ordered_occurrence_ids: Vec<OccurrenceId>,
    source_coordinates_mm: Vec<f64>,
    target_coordinates_mm: Vec<f64>,
    spacing_mm: f64,
    commands: Vec<CanonicalCommand>,
    preview_boxes: BTreeMap<OccurrenceId, RenderBox>,
}

#[derive(Clone, Debug)]
struct PendingOccurrenceDistribution {
    source: OccurrenceDistributionSourcePlan,
    axis: Axis,
    mode: DistributionMode,
    preview_plan: Option<OccurrenceDistributionPlan>,
}

#[derive(Clone, Debug, PartialEq)]
struct LinearPatternSourcePlan {
    source_revision: u64,
    source_digest: String,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    occurrence_id: OccurrenceId,
    definition_id: DefinitionId,
    definition_name: String,
    source_transform: Transform,
    source_parent: Option<GroupId>,
    source_tags: BTreeSet<TagId>,
    source_visible: bool,
    source_color: Option<[u8; 3]>,
    next_occurrence_id: OccurrenceId,
    existing_definition_occurrence_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct LinearPatternPlan {
    source: LinearPatternSourcePlan,
    axis: Axis,
    spacing_mm: f64,
    count: usize,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug)]
struct PendingLinearPattern {
    source: LinearPatternSourcePlan,
    axis: Axis,
    spacing: String,
    count: String,
    preview_plan: Option<LinearPatternPlan>,
}

#[derive(Clone, Debug, PartialEq)]
struct RectangularPatternSourcePlan {
    source_revision: u64,
    source_digest: String,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    occurrence_id: OccurrenceId,
    definition_id: DefinitionId,
    definition_name: String,
    source_transform: Transform,
    source_parent: Option<GroupId>,
    source_tags: BTreeSet<TagId>,
    source_visible: bool,
    source_color: Option<[u8; 3]>,
    next_occurrence_id: OccurrenceId,
    existing_definition_occurrence_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct RectangularPatternPlan {
    source: RectangularPatternSourcePlan,
    primary_axis: Axis,
    primary_spacing_mm: f64,
    primary_count: usize,
    secondary_axis: Axis,
    secondary_spacing_mm: f64,
    secondary_count: usize,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug)]
struct PendingRectangularPattern {
    source: RectangularPatternSourcePlan,
    primary_axis: Axis,
    primary_spacing: String,
    primary_count: String,
    secondary_axis: Axis,
    secondary_spacing: String,
    secondary_count: String,
    preview_plan: Option<RectangularPatternPlan>,
}

#[derive(Clone, Debug, PartialEq)]
struct CircularPatternSourcePlan {
    source_revision: u64,
    source_digest: String,
    occurrence_paths: BTreeSet<InstancePath>,
    occurrence_count: usize,
    source_primary: Option<SelectionId>,
    source_selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
    occurrence_id: OccurrenceId,
    definition_id: DefinitionId,
    definition_name: String,
    source_transform: Transform,
    source_parent: Option<GroupId>,
    source_tags: BTreeSet<TagId>,
    source_visible: bool,
    source_color: Option<[u8; 3]>,
    next_occurrence_id: OccurrenceId,
    existing_definition_occurrence_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct CircularPatternPlan {
    source: CircularPatternSourcePlan,
    axis: Axis,
    centre_mm: Vec3,
    angle_step_degrees: f64,
    count: usize,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug)]
struct PendingCircularPattern {
    source: CircularPatternSourcePlan,
    axis: Axis,
    centre_x: String,
    centre_y: String,
    centre_z: String,
    angle: String,
    count: String,
    preview_plan: Option<CircularPatternPlan>,
}

#[derive(Clone, Debug, PartialEq)]
struct StlImportPreviewPlan {
    source: ImportSourcePlan<ImportLengthUnit>,
    review: ParsedStlMesh,
    proposal: Proposal,
}

#[derive(Clone, Debug)]
struct PendingStlImport {
    plan: StlImportPreviewPlan,
    review_error: Option<String>,
    invalidated: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct StepImportPreviewPlan {
    source: ImportSourcePlan,
    evidence: StepXdeImportEvidence,
    proposal: Proposal,
    blob_hash: String,
}

#[derive(Clone, Debug, PartialEq)]
struct IgesImportPreviewPlan {
    source: ImportSourcePlan,
    evidence: IgesXdeImportEvidence,
    proposal: Proposal,
    blob_hash: String,
}

#[derive(Clone, Debug)]
struct PendingStepImport {
    plan: StepImportPreviewPlan,
    invalidated: bool,
}

#[derive(Clone, Debug)]
struct PendingIgesImport {
    plan: IgesImportPreviewPlan,
    invalidated: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct DxfImportPreviewPlan {
    source: ImportSourcePlan<ImportLengthUnit>,
    review: ParsedDxf,
    proposal: Proposal,
}

#[derive(Clone, Debug)]
struct PendingDxfImport {
    plan: DxfImportPreviewPlan,
    unit_confirmed: bool,
    review_error: Option<String>,
    invalidated: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct SketchupSceneImportPreviewPlan {
    source: ImportSourcePlan,
    review: ParsedSketchupScene,
    proposal: Proposal,
}

#[derive(Clone, Debug)]
struct PendingSketchupSceneImport {
    plan: SketchupSceneImportPreviewPlan,
    invalidated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecoveryOpenState {
    requested_path: PathBuf,
    source_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
struct MigrationReviewSourcePlan {
    path: PathBuf,
    effective_path: PathBuf,
    source: Vec<u8>,
    source_sha256: [u8; 32],
    source_byte_len: u64,
    active_document_id: DocumentId,
    active_revision_id: u64,
    active_canonical_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MigrationReviewIdentity {
    document_id: DocumentId,
    revision_id: u64,
    canonical_digest: String,
    audit: ketchup_model::persistence::LoadAudit,
    container_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq)]
struct MigrationReviewPlan {
    source: MigrationReviewSourcePlan,
    review: MigrationReviewIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BtlxProfileStrategy {
    PortableFreeContour,
    EdgeSawCutsThenMillContour,
}

struct CamExportDialog {
    plan_id: CamPlanId,
    review: Option<CamReviewSummary>,
}

struct FeaReviewDialog {
    definition_id: DefinitionId,
    feature_id: FeatureId,
    occurrence_id: OccurrenceId,
    case_id: String,
    youngs_modulus_mpa: String,
    poisson_ratio: String,
    yield_strength_mpa: String,
    constrained_face_ordinals: String,
    loaded_face_ordinal: String,
    traction_x_n_per_mm2: String,
    traction_y_n_per_mm2: String,
    traction_z_n_per_mm2: String,
    coarse_deflection_mm: String,
    fine_deflection_mm: String,
    review: Option<FeaReviewSummary>,
}

struct PdmReviewDialog {
    source: PdmSourceIdentity,
    repository: String,
    parent_release_id: String,
    release_id: String,
    compare_release_id: String,
    dependencies: String,
    actor: String,
    note: String,
    catalog: Vec<ReleaseCatalogEntry>,
    opened: Option<ReleaseManifest>,
    comparison: Option<ReleaseComparison>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MutationReadiness {
    Ready,
    Pending,
}

#[derive(Debug)]
enum WorkRecoveryMutationError<E> {
    Mutation(E),
    Recovery(ketchup_model::persistence::FilePersistenceError),
}

impl<E: std::fmt::Display> std::fmt::Display for WorkRecoveryMutationError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mutation(error) => error.fmt(formatter),
            Self::Recovery(error) => error.fmt(formatter),
        }
    }
}

impl<E: std::error::Error> std::error::Error for WorkRecoveryMutationError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mutation(error) => error.source(),
            Self::Recovery(error) => error.source(),
        }
    }
}

/// The detaching edit is undone only while it is still the current revision;
/// any later change closes the notice.
struct ProgramDetachNotice {
    file_name: String,
    revision_id: u64,
}

pub struct KetchupApp {
    document: DocumentStore,
    live: app_state::LiveState,
    close_guard: close_guard::CloseGuard,
    file: app_state::FileState,
    confirmation_surface: TrustedConfirmationSurface,
    side_effect_receipts: Vec<SideEffectAuthorizationReceipt>,
    btlx_profile_strategy: BtlxProfileStrategy,
    btlx_intermediate_saw_cuts: u8,
    catalog: LocaleCatalog,
    /// The language `catalog` is in; `None` for a test catalog such as pseudo.
    language: Option<language::UiLanguage>,
    /// Whether a language picked in the menu is remembered for the next start.
    remember_language: bool,
    /// The program the last edit detached, while the one-time notice about it
    /// is on screen.
    program_detach_notice: Option<ProgramDetachNotice>,
    /// The notice is shown for the first detaching edit of a session only.
    program_detach_warned: bool,
    assembly_editor: assembly_ui::AssemblyEditorState,
    body_editor: body_ui::BodyEditorState,
    face_workflow: face_workflow_ui::FaceWorkflowUiState,
    feature_history: feature_history_ui::FeatureHistoryUiState,
    push_pull: app_state::PushPullState,
    program_evaluations: program_evaluation::ProgramEvaluations,
    /// The tool whose typed value waits for the program evaluation; it is
    /// applied when the evaluation is ready if that tool is still active.
    value_input_awaits_program: Option<ActiveTool>,
    solid_tools: app_state::SolidToolInputs,
    helix_tool: helix_ui::HelixUiState,
    parameter: app_state::ParameterEditor,
    validator_panel: app_state::ValidatorPanel,
    status_key: &'static str,
    theme: ThemeKind,
    camera: app_state::CameraState,
    view: ViewSettings,
    /// The typed name and the last shown scene of the saved views.
    saved_views_ui: app::saved_views::SavedViewsUi,
    /// The section plane cutting the viewport open; display only, never the model.
    section: Option<SectionPlane>,
    selection: SelectionState,
    /// The space or clearance volume picked in the dock or by a validator
    /// finding, with the part selection it was picked together with.
    picked_space: Option<(app::spaces_ui::SpaceRow, BTreeSet<InstancePath>)>,
    space_fields: app::spaces_ui::SpaceFields,
    hover: app_state::HoverState,
    active_tool: ActiveTool,
    panels: app_state::Panels,
    command_search: app_state::CommandSearch,
    takeoff: app_state::TakeoffState,
    drawings: app_state::DrawingsState,
    viewer_prep: app_state::ViewerPrepState,
    digest: String,
    classification: app_state::ClassificationInputs,
    transform_tool: app_state::TransformToolState,
    value_box: app_state::ValueBox,
    clipboard: app_state::Clipboard,
    /// The dialog waiting for the user, if any.
    modal: Option<Modal>,
    /// The live tool preview shown before a commit, if any.
    tool_preview: Option<ToolPreview>,
    /// The drawing, measurement, transform modifiers and pointer drag in progress.
    gesture: Gesture,
    mesh_conversion_state: mesh_conversion_ui::MeshConversionUiState,
    dialogs: Box<dyn FileDialogs>,
    reviews: app_state::ReviewWorkflows,
    exact: app_state::ExactState,
    render: app_state::RenderState,
}

impl Default for KetchupApp {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for KetchupApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(context);
    }
}

fn bounds_of(points: impl Iterator<Item = Vec3>) -> Option<[Vec3; 2]> {
    points.fold(None, |bounds, point| {
        Some(
            bounds.map_or([point, point], |[minimum, maximum]: [Vec3; 2]| {
                [
                    Vec3::new(
                        minimum.x.min(point.x),
                        minimum.y.min(point.y),
                        minimum.z.min(point.z),
                    ),
                    Vec3::new(
                        maximum.x.max(point.x),
                        maximum.y.max(point.y),
                        maximum.z.max(point.z),
                    ),
                ]
            }),
        )
    })
}

const fn axis_direction(axis: Axis) -> Vec3 {
    match axis {
        Axis::X => Vec3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Axis::Y => Vec3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        Axis::Z => Vec3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    }
}

/// The two in-plane unit vectors of the rotation plane, ordered so that turning
/// from the first towards the second is the positive direction about `axis`.
const fn axis_plane_frame(axis: Axis) -> (Vec3, Vec3) {
    match axis {
        Axis::X => (axis_direction(Axis::Y), axis_direction(Axis::Z)),
        Axis::Y => (axis_direction(Axis::Z), axis_direction(Axis::X)),
        Axis::Z => (axis_direction(Axis::X), axis_direction(Axis::Y)),
    }
}

fn exact_surface_element(normal: Vec3) -> Option<ElementId> {
    let components = [normal.x.abs(), normal.y.abs(), normal.z.abs()];
    let (axis_index, magnitude) = components
        .into_iter()
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(&right.1))?;
    if magnitude <= ROUNDING {
        return None;
    }
    let (axis, signed_component) = match axis_index {
        0 => (Axis::X, normal.x),
        1 => (Axis::Y, normal.y),
        _ => (Axis::Z, normal.z),
    };
    Some(ElementId::Face {
        axis,
        side: if signed_component >= 0.0 {
            Side::Maximum
        } else {
            Side::Minimum
        },
    })
}

fn transform_model_point(transform: Transform, point: Vec3) -> Vec3 {
    let matrix = transform.matrix();
    Vec3::new(
        matrix[0] * point.x + matrix[1] * point.y + matrix[2] * point.z + matrix[3],
        matrix[4] * point.x + matrix[5] * point.y + matrix[6] * point.z + matrix[7],
        matrix[8] * point.x + matrix[9] * point.y + matrix[10] * point.z + matrix[11],
    )
}

fn triangle_normal([first, second, third]: [Vec3; 3]) -> Vec3 {
    let first_edge = second - first;
    let second_edge = third - first;
    Vec3::new(
        first_edge.y * second_edge.z - first_edge.z * second_edge.y,
        first_edge.z * second_edge.x - first_edge.x * second_edge.z,
        first_edge.x * second_edge.y - first_edge.y * second_edge.x,
    )
}

fn face_element_from_normal(normal: Vec3) -> ElementId {
    let (axis, direction) = if normal.x.abs() >= normal.y.abs() && normal.x.abs() >= normal.z.abs()
    {
        (Axis::X, normal.x)
    } else if normal.y.abs() >= normal.z.abs() {
        (Axis::Y, normal.y)
    } else {
        (Axis::Z, normal.z)
    };
    ElementId::Face {
        axis,
        side: if direction < 0.0 {
            Side::Minimum
        } else {
            Side::Maximum
        },
    }
}

fn tangent_points(anchor: Vec3, center: Vec3, radius: f64) -> Vec<Vec3> {
    let delta = anchor - center;
    let distance_squared = delta.x * delta.x + delta.y * delta.y;
    if !radius.is_finite() || radius <= 0.0 || distance_squared <= radius * radius + ROUNDING {
        return Vec::new();
    }
    let base_scale = radius * radius / distance_squared;
    let perpendicular_scale =
        radius * (distance_squared - radius * radius).sqrt() / distance_squared;
    let base = Vec3::new(
        center.x + delta.x * base_scale,
        center.y + delta.y * base_scale,
        center.z,
    );
    let offset = Vec3::new(
        -delta.y * perpendicular_scale,
        delta.x * perpendicular_scale,
        0.0,
    );
    vec![base + offset, base - offset]
}

fn point_line_signed_distance(point: Vec3, start: Vec3, end: Vec3) -> f64 {
    let chord = end - start;
    let length = chord.x.hypot(chord.y);
    if length <= ROUNDING {
        0.0
    } else {
        (chord.x * (point.y - start.y) - chord.y * (point.x - start.x)) / length
    }
}

fn arc_geometry(start: Vec3, end: Vec3, bulge: Vec3) -> Option<ArcGeometry> {
    if (start.z - end.z).abs() > APPROXIMATION || (start.z - bulge.z).abs() > APPROXIMATION {
        return None;
    }
    if 2.0 * length(cross(end - start, bulge - start)) <= ROUNDING {
        return None;
    }
    let center = Vec3 {
        z: start.z,
        ..circumcenter(start, end, bulge)?
    };
    let radius = length(center - start);
    let end_radius = length(center - end);
    if !radius.is_finite()
        || radius <= 0.01
        || (radius - end_radius).abs() > ACCUMULATED_ROUNDING * radius.max(end_radius).max(1.0)
    {
        return None;
    }
    let cross = (end.x - start.x) * (bulge.y - start.y) - (end.y - start.y) * (bulge.x - start.x);
    Some(ArcGeometry {
        start,
        end,
        center,
        clockwise: cross > 0.0,
    })
}

fn parse_distance_mm(input: &str) -> Option<f64> {
    length_input::parse_length_mm(input)
}

fn parse_dimension(input: &str) -> Option<Dimension> {
    let millimetres = parse_distance_mm(input)?;
    Dimension::new(input.trim().to_owned(), millimetres).ok()
}

fn format_height(height: f64) -> String {
    let formatted = format!("{height:.2}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn screen_cross(first: Pos2, second: Pos2, third: Pos2) -> f32 {
    let left = second - first;
    let right = third - first;
    left.x * right.y - left.y * right.x
}

fn screen_point_on_segment(point: Pos2, first: Pos2, second: Pos2) -> bool {
    const EPSILON: f32 = SCREEN_ROUNDING_PX;
    screen_cross(first, second, point).abs() <= EPSILON
        && point.x >= first.x.min(second.x) - EPSILON
        && point.x <= first.x.max(second.x) + EPSILON
        && point.y >= first.y.min(second.y) - EPSILON
        && point.y <= first.y.max(second.y) + EPSILON
}

fn screen_segments_intersect(first: [Pos2; 2], second: [Pos2; 2]) -> bool {
    let [a, b] = first;
    let [c, d] = second;
    let ab_c = screen_cross(a, b, c);
    let ab_d = screen_cross(a, b, d);
    let cd_a = screen_cross(c, d, a);
    let cd_b = screen_cross(c, d, b);
    (ab_c.signum() != ab_d.signum() && cd_a.signum() != cd_b.signum())
        || screen_point_on_segment(c, a, b)
        || screen_point_on_segment(d, a, b)
        || screen_point_on_segment(a, c, d)
        || screen_point_on_segment(b, c, d)
}

fn screen_triangle_contains_point(triangle: [Pos2; 3], point: Pos2) -> bool {
    const EPSILON: f32 = SCREEN_ROUNDING_PX;
    let signs = [
        screen_cross(triangle[0], triangle[1], point),
        screen_cross(triangle[1], triangle[2], point),
        screen_cross(triangle[2], triangle[0], point),
    ];
    !signs.iter().any(|value| *value < -EPSILON) || !signs.iter().any(|value| *value > EPSILON)
}

fn box_corners(width: f64, depth: f64, height: f64) -> [Vec3; 8] {
    [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(width, 0.0, 0.0),
        Vec3::new(0.0, depth, 0.0),
        Vec3::new(width, depth, 0.0),
        Vec3::new(0.0, 0.0, height),
        Vec3::new(width, 0.0, height),
        Vec3::new(0.0, depth, height),
        Vec3::new(width, depth, height),
    ]
}

#[cfg(test)]
mod drawing_plane_tests;
#[cfg(test)]
mod mesh_snapping_tests;
#[cfg(test)]
mod nested_transform_tests;
#[cfg(test)]
mod planar_offset_tests;
#[cfg(test)]
mod program_source_ui_tests;
#[cfg(test)]
mod push_pull_snapping_tests;
#[cfg(test)]
mod rotation_input_tests;
#[cfg(test)]
mod scene_snapping_tests;
#[cfg(test)]
mod selection_measures_ui_tests;
#[cfg(test)]
mod tests;
