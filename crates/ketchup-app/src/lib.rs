// Unsafe code is denied, not forbidden, for exactly one audited reason: the
// native file dialogs must borrow the raw main-window handle to be owned by,
// and modal to, the Ketchup window. See `dialogs::DialogParentWindow`.
#![deny(unsafe_code)]

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use gesture::{Gesture, MirrorPlane, PushPullAnchor, ZoomWindowDrag};
use ketchup_analysis::fea::{FeaMaterial, FeaSolveSettings};
use ketchup_application::cam_workflow::{CamReviewRequest, CamReviewSummary, CamReviewWorkflow};
use ketchup_application::diagnostics::{
    AssistantPlanningResult, AssistantRejection, assistant_canonical_rejection,
    assistant_planning_rejection, assistant_rejection,
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
use ketchup_application::topology::{assistant_topology_references, plan_topology_finish_kind};
use ketchup_application::transforms::{
    mirrored_copy_commands, rotation_in_parent_space, translated_transform,
    world_axis_rotation_transform, world_edit_in_parent_space, world_plane_mirror_transform,
};
use ketchup_application::validation::*;
use ketchup_assistant::intent::{IntentRequest, WorkflowIntent, propose_intent};
use ketchup_assistant::sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantApiDiagnostics, AssistantBoxIntent,
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantCapability, AssistantChatResult, AssistantDistribution, AssistantFeaReviewRequest,
    AssistantHandshake, AssistantModelIntent, AssistantRejectionDiagnostic,
    AssistantRejectionPhase,
};
#[cfg(test)]
use ketchup_assistant::sidecar::{
    AssistantCadBodyFeature, AssistantCadBooleanOperation, AssistantCadDeletePolicy,
    AssistantCadLoftContinuity,
};
use ketchup_geometry::linalg::{circumcenter, cross, dot, length};
use ketchup_geometry::prismatic::JointId;
use ketchup_geometry::sketch::{
    FeatureDirection, FeatureExtent, PadOperation, PadProfile, PadSpec, PrincipalPlane,
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId,
    SketchPointKind, SketchPointRef, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
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
use ketchup_model::document::{
    AuthenticatedApprover, AuthoritativeDependency, BodyId, BooleanOperation, CanonicalCommand,
    CanonicalError, ClassificationCategoryId, ClassificationDimensionId, CloneDefinitionPlan,
    CollectionId, CommandBatch, ConvertGroupPlan, DefinitionId, Dimension, DimensionDisplayUnit,
    DimensionPresentation, DocumentId, DocumentStore, EdgeFinishKind, EdgeRef, EvaluationIdentity,
    EvaluatorParameterEdit, FaceRef, FeatureId, FeatureKind, FeatureParameterTarget, GroupId,
    HighRiskClass, HighRiskScope, InstancePath, LoftContinuity, LoftSection,
    MAX_HUMAN_CONFIRMATION_LIFETIME_MS, MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec, NodeId,
    OccurrenceId, PersistentDimensionId, ProfileSegment, Proposal, ProposalCommitError,
    ProposalContext, ProposalGoal, ProposalPrepareError, ProposalPrincipal, ProposalValue,
    SPLINE_MIN_POINTS, SceneOccurrence, SceneQueryContext, SideEffectAuthorizationReceipt,
    SlotPath, Snapshot, SolidToolPlan, SpatialPathSegment, TagId, TipReplacementParent,
    TipReplacementProposal, Transform, TrustedConfirmationSurface, ellipse_segments,
    polygon_segments, regular_polygon_points,
};
#[cfg(test)]
use ketchup_model::document::{
    OverrideParameterSpec, ParameterValueType, PersistentDimension, PersistentDimensionTarget,
    SlotResolution,
};
use ketchup_model::exact_brep_graph::ExactBRepGraph;
use ketchup_model::exact_product::{
    AssemblySelectionTarget, ExactBodyPackage, ExactBodyView, ExactFaceRole, ExactMeshExport,
    ExactResultRegistry, ExactStlExport, MeshExportBody, MeshExportSource,
    exact_body_terminal_features, model_stl_export,
};
#[cfg(test)]
use ketchup_model::exact_product::{ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence};
use ketchup_model::exact_validation::{
    GeneralBodyNarrowPhaseRelation, GeneralBodyParticipant, general_body_narrow_phase,
};
use ketchup_model::graph::{
    DerivedIdentity, EvaluationStatus, EvaluatorNodeKind, RuleOutput, SlotSegment, sha256_bytes,
};
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
#[cfg(test)]
use ketchup_model::space::ClearanceOwner;
use ketchup_model::space::{ClearanceSeverity, ClearanceVolumeId, SpaceId};
use ketchup_model::state_view::{AGENT_STATE_VIEW, encode_semantic_state};
use ketchup_model::tolerance::TolerancePolicy;
use ketchup_model::tolerance::limits;
use ketchup_model::tolerance::{
    ACCUMULATED_ROUNDING, APPROXIMATION, DEFAULT_LINEAR_TOLERANCE_MM, MAX_COORDINATE_MM, ROUNDING,
    SCREEN_ROUNDING_PX,
};
use ketchup_model::topology::{TopologicalElementKind, TopologicalElementRef};
use ketchup_model::validation::ValidatorRoleIndex;
use ketchup_pdm::local::{
    ReleaseAudit, ReleaseCatalogEntry, ReleaseComparison, ReleaseConflictVerdict,
    ReleaseDependencyInput, ReleaseManifest,
};
use ketchup_scheduler::{ExactWorkerSupervisor, assistant::AssistantCancellation};
use modal::Modal;
use slot::Slot;
use tool_preview::ToolPreview;
pub use view_settings::{ViewFlag, ViewSettings};
mod app;
use app::*;
mod app_state;
mod assembly_ui;
mod assistant_fea_review;
mod assistant_runtime;
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
pub mod live_bridge;
mod mesh_conversion_ui;
mod modal;
mod native_document_inspection;
mod occurrence_color_ui;
mod planar_push_pull;
mod program_edit;
mod program_source_ui;
mod refusal;
use ketchup_rejection::{Rejection, RejectionPhase};
use refusal::{Refuse, failed, invalid_field};
mod slot;
mod tool_preview;
mod transform_operation;
mod validator_ui;
mod view_settings;
mod viewport_feedback;
use assistant_runtime::ProcessAssistantTransport;
pub use assistant_runtime::{
    AssistantLaunchError, private_assistant_launch, private_assistant_launch_for_executable,
    public_assistant_launch_for_install_root, verify_public_assistant_runtime,
};
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
const ASSISTANT_MODELS_YAML: &str = include_str!("../assistant-models.yaml");
const MAX_ASSISTANT_MODEL_CATALOG_BYTES: u64 = 64 * 1024;
const ASSISTANT_CHAT_NAMESPACE: &str = "org.ketchup.assistant";
const ASSISTANT_CHAT_PATH: &str = "conversation-v1.json";
const ASSISTANT_MEMORY_PATH: &str = "project-memory-v1.json";
const ASSISTANT_MEMORY_SCHEMA: &str = "ketchup.project-memory.v1";
const MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES: usize = 24 * 1024;
const ASSISTANT_LOCAL_INSPECTION_CATALOG: &str = "_local_inspection_catalog";
const MAX_ASSISTANT_PROVIDER_STATE_VIEW_BYTES: usize = 1024;
const MAX_ASSISTANT_PROVIDER_CONVERSATION_MESSAGES: usize = 6;
const MAX_ASSISTANT_PROVIDER_CONVERSATION_TEXT_BYTES: usize = 2 * 1024;
const MAX_ASSISTANT_STATE_VIEW_BYTES: usize = 12 * 1024;
const MAX_ASSISTANT_MEMORY_ENTRIES: usize = 128;
const MAX_ASSISTANT_MEMORY_TEXT_BYTES: usize = 1024;
const MAX_ASSISTANT_MEMORY_RETRIEVAL_ENTRIES: usize = 4;
const MAX_ASSISTANT_MEMORY_RETRIEVAL_BYTES: usize = 8 * 1024;
const MAX_ASSISTANT_MEMORY_STORAGE_BYTES: usize = 320 * 1024;
use ketchup_application::validation::{
    MAX_ASSISTANT_VALIDATION_ISSUES, MAX_ASSISTANT_VALIDATION_OCCURRENCES,
};
pub const ASSISTANT_REPAIR_PROGRAM_SCHEMA_V1: &str = "ketchup.assistant-repair-program.v1";
#[derive(Clone, Debug, Eq, PartialEq)]
struct AssistantValidationSelection {
    mode: &'static str,
    requested: BTreeSet<&'static str>,
    unknown: Vec<String>,
}

impl AssistantValidationSelection {
    fn all(mode: &'static str) -> Self {
        Self {
            mode,
            requested: ASSISTANT_VALIDATOR_IDS.into_iter().collect(),
            unknown: Vec::new(),
        }
    }

    fn parse(query: &str) -> Self {
        let normalized = query.to_lowercase();
        let all_except_markers = [
            "všetky validátory okrem ",
            "vsetky validatory okrem ",
            "všetky okrem ",
            "vsetky okrem ",
            "all validators except ",
            "all except ",
        ];
        if let Some(suffix) = selection_suffix(&normalized, &all_except_markers) {
            let (excluded, unknown) = resolve_assistant_validator_names(suffix);
            return Self {
                mode: "all_except",
                requested: ASSISTANT_VALIDATOR_IDS
                    .into_iter()
                    .filter(|validator| !excluded.contains(validator))
                    .collect(),
                unknown,
            };
        }

        let only_markers = ["iba ", "len ", "only "];
        if let Some(suffix) = selection_suffix(&normalized, &only_markers) {
            let (requested, unknown) = resolve_assistant_validator_names(suffix);
            return Self {
                mode: "only",
                requested,
                unknown,
            };
        }

        Self::all("all")
    }

    fn is_valid(&self) -> bool {
        self.unknown.is_empty() && !self.requested.is_empty()
    }
}

/// One validator finding rendered for the operator, with the parts it refers to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorPanelFinding {
    pub validator: &'static str,
    pub code: String,
    pub severity: String,
    pub parts: Vec<String>,
    pub detail: String,
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

fn validator_finding_parts(issue: &serde_json::Value) -> Vec<String> {
    let labelled = |id: &serde_json::Value, name: &serde_json::Value| {
        let id = id.as_u64();
        match (name.as_str(), id) {
            (Some(name), Some(id)) => Some(format!("{name} (#{id})")),
            (None, Some(id)) => Some(format!("#{id}")),
            _ => None,
        }
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
                .map(|issue| ValidatorPanelFinding {
                    validator,
                    code: issue["code"].as_str().unwrap_or_default().to_owned(),
                    severity: issue["severity"].as_str().unwrap_or_default().to_owned(),
                    parts: validator_finding_parts(issue),
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

fn selection_suffix<'a>(query: &'a str, markers: &[&str]) -> Option<&'a str> {
    markers
        .iter()
        .filter_map(|marker| {
            query
                .find(marker)
                .map(|index| &query[index + marker.len()..])
        })
        .next()
}

fn bind_assistant_cad_current_selection(
    program: &mut AssistantCadEditProgram,
    occurrence_ids: &[u64],
) {
    for operation in &mut program.operations {
        let Some(selector) = (match operation {
            AssistantCadEditOperation::CreateSketch { .. }
            | AssistantCadEditOperation::CreatePart { .. }
            | AssistantCadEditOperation::CreatePinJoint { .. }
            | AssistantCadEditOperation::DeletePhysicalPinJoint { .. }
            | AssistantCadEditOperation::MovePhysicalPinPair { .. }
            | AssistantCadEditOperation::CreateTag { .. }
            | AssistantCadEditOperation::SetOccurrenceTag { .. }
            | AssistantCadEditOperation::SetTagVisibility { .. }
            | AssistantCadEditOperation::CreateSpatialPath { .. }
            | AssistantCadEditOperation::CreateConstructionPoint { .. }
            | AssistantCadEditOperation::CreateConstructionAxis { .. }
            | AssistantCadEditOperation::CreateConstructionPlane { .. }
            | AssistantCadEditOperation::CreateHelix { .. }
            | AssistantCadEditOperation::FilletEdges { .. }
            | AssistantCadEditOperation::ChamferEdges { .. }
            | AssistantCadEditOperation::AppendFeature { .. }
            | AssistantCadEditOperation::BindProgramOutput { .. }
            | AssistantCadEditOperation::SetDimension { .. }
            | AssistantCadEditOperation::SetFeatureParameter { .. }
            | AssistantCadEditOperation::MakeOccurrenceUnique { .. }
            | AssistantCadEditOperation::CreateAssemblyJoint { .. }
            | AssistantCadEditOperation::SetAssemblyJointPosition { .. }
            | AssistantCadEditOperation::CreateDrawing { .. }
            | AssistantCadEditOperation::UpsertCamPlan { .. }
            | AssistantCadEditOperation::UpsertClassificationDimension { .. }
            | AssistantCadEditOperation::CreateEvaluatorInput { .. } => None,
            AssistantCadEditOperation::Delete { selector, .. }
            | AssistantCadEditOperation::SetColor { selector, .. }
            | AssistantCadEditOperation::SetGrounded { selector, .. }
            | AssistantCadEditOperation::SetOccurrenceClassification { selector, .. }
            | AssistantCadEditOperation::Transform { selector, .. }
            | AssistantCadEditOperation::Copy { selector, .. }
            | AssistantCadEditOperation::LinearPattern { selector, .. }
            | AssistantCadEditOperation::CircularPattern { selector, .. }
            | AssistantCadEditOperation::Mirror { selector, .. } => Some(selector),
        }) else {
            continue;
        };
        if matches!(selector, AssistantCadEntitySelector::CurrentSelection {}) {
            *selector = AssistantCadEntitySelector::Occurrences {
                occurrence_ids: occurrence_ids.to_vec(),
            };
        }
    }
}

fn assistant_query_requests_repair(query: &str) -> bool {
    query
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| word.starts_with("oprav") || word.starts_with("repair") || word == "fix")
}

fn resolve_assistant_validator_names(text: &str) -> (BTreeSet<&'static str>, Vec<String>) {
    let aliases = [
        (
            "static_load",
            [
                "statika a sily",
                "statický výpočet",
                "staticky vypocet",
                "silový výpočet",
                "silovy vypocet",
                "static load",
                "force calculation",
                "statika",
                "sily",
            ]
            .as_slice(),
        ),
        (
            "passage_clearance",
            [
                "priechodnosť miestnosti",
                "priechodnost miestnosti",
                "priechodové zóny",
                "priechodove zony",
                "passage clearance",
                "walking clearance",
                "priechodnosť",
                "priechodnost",
                "priechody",
                "passages",
                "aisles",
            ]
            .as_slice(),
        ),
        (
            "room_placement",
            [
                "umiestnenie nábytku v miestnosti",
                "umiestnenie nabytku v miestnosti",
                "umiestnenie v miestnosti",
                "room placement",
                "furniture placement",
                "miestnosť",
                "miestnost",
            ]
            .as_slice(),
        ),
        (
            "hardware_manufacturing",
            [
                "kovanie a vyrobiteľnosť",
                "kovanie a vyrobitelnost",
                "kovanie a výroba",
                "kovanie a vyroba",
                "hardware and manufacturing",
                "hardware manufacturing",
                "drawer slides",
                "vyrobiteľnosť",
                "vyrobitelnost",
                "manufacturing",
                "výroba",
                "vyroba",
                "pánty",
                "panty",
                "výsuvy",
                "vysuvy",
                "diery",
                "otvory",
                "hrany",
                "hinges",
                "holes",
                "edges",
                "kovanie",
                "hardware",
            ]
            .as_slice(),
        ),
        (
            "beam_deflection",
            [
                "priehyb nosníka",
                "priehyb nosnika",
                "beam deflection",
                "priehyb políc",
                "priehyb polic",
                "priehyb police",
                "priehyb poličky",
                "priehyb policky",
                "shelf deflection",
                "priehyb",
                "deflection",
            ]
            .as_slice(),
        ),
        (
            "tipping",
            [
                "stabilita proti prevráteniu",
                "stabilita proti prevrateniu",
                "prevrátenie",
                "prevratenie",
                "tip-over",
                "tipping",
            ]
            .as_slice(),
        ),
        (
            "anchoring",
            [
                "požiadavka kotvenia",
                "poziadavka kotvenia",
                "ukotvenie",
                "kotvenie",
                "anchoring",
                "anchor requirement",
            ]
            .as_slice(),
        ),
        (
            "gravity_support",
            [
                "nepodopreté diely",
                "nepodoprete diely",
                "gravity support",
                "unsupported parts",
                "podopretie",
                "podopretia",
                "podpora",
            ]
            .as_slice(),
        ),
        (
            "collision",
            [
                "kolízie",
                "kolizie",
                "kolízií",
                "kolizii",
                "collisions",
                "collision",
            ]
            .as_slice(),
        ),
    ];
    let mut remainder = text.to_owned();
    let mut resolved = BTreeSet::new();
    for (validator, names) in aliases {
        for name in names {
            if remainder.contains(name) {
                resolved.insert(validator);
                remainder = remainder.replace(name, " ");
            }
        }
    }
    let filler = [
        "a",
        "and",
        "validator",
        "validators",
        "validátor",
        "validátory",
        "validatorov",
        "validátorov",
        "model",
        "prosím",
        "prosim",
        "please",
    ];
    let remainder = remainder
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    let unknown = remainder
        .split_whitespace()
        .filter(|word| !filler.contains(word))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    (resolved, unknown)
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

    #[cfg(test)]
    fn provenance_revision(&self) -> u64 {
        match self {
            Self::Append(proposal) => proposal.provenance_revision(),
            Self::TipReplacement(proposal) => proposal.superseded_revision(),
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
    target_tag: Option<TagId>,
    target_visible: bool,
    target_box: RenderBox,
    target_feature_id: FeatureId,
    tool_selection: SelectionId,
    tool_name: String,
    tool_transform: Transform,
    tool_parent: Option<GroupId>,
    tool_tag: Option<TagId>,
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
            && self.target_tag == other.target_tag
            && self.target_visible == other.target_visible
            && self.target_feature_id == other.target_feature_id
            && self.tool_selection == other.tool_selection
            && self.tool_name == other.tool_name
            && self.tool_transform == other.tool_transform
            && self.tool_parent == other.tool_parent
            && self.tool_tag == other.tool_tag
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
}

impl ProjectedPolygon {
    fn points(&self) -> &[Pos2] {
        match self {
            Self::Triangle(points) => points,
            Self::Quad(points) => points,
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
    Exact(Box<ExactBodyPackage>),
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

fn assistant_subtracted_box_feature_commands(
    item: &AssistantBoxIntent,
    definition_id: DefinitionId,
    workplane_id: FeatureId,
) -> Option<(Vec<CanonicalCommand>, u64)> {
    let rectangle_sketch = |workplane: FeatureId, origin_mm: [f64; 2], size_mm: [f64; 2]| {
        let [x, y] = origin_mm;
        let [width, depth] = size_mm;
        let points = [
            [x, y],
            [x + width, y],
            [x + width, y + depth],
            [x, y + depth],
        ];
        let mut entities = Vec::with_capacity(points.len());
        let mut constraints = Vec::with_capacity(points.len() * 2);
        for index in 0..points.len() {
            let entity = SketchEntityId(index as u64 + 1);
            let start = points[index];
            let end = points[(index + 1) % points.len()];
            entities.push(SketchEntity::Line {
                id: entity,
                start_mm: start,
                end_mm: end,
            });
            for (point_index, (point, position_mm)) in
                [(SketchPointKind::Start, start), (SketchPointKind::End, end)]
                    .into_iter()
                    .enumerate()
            {
                constraints.push(SketchConstraint {
                    id: SketchConstraintId(index as u64 * 2 + point_index as u64 + 1),
                    kind: SketchConstraintKind::FixedPoint {
                        point: SketchPointRef { entity, point },
                        position_mm,
                    },
                });
            }
        }
        let sketch = SketchSpec {
            workplane,
            entities,
            constraints,
        };
        let regions = sketch.solved_regions().ok()?;
        let [region] = regions.as_slice() else {
            return None;
        };
        Some((sketch, region.id))
    };

    let sketch_id = workplane_id.0.checked_add(1).map(FeatureId)?;
    let body_id = workplane_id.0.checked_add(2).map(FeatureId)?;
    let [width, depth, height] = item.size_mm;
    let (sketch, region) = rectangle_sketch(workplane_id, [0.0, 0.0], [width, depth])?;
    let height = Dimension::new(height.to_string(), height).ok()?;
    let mut commands = vec![
        CanonicalCommand::CreateFeature {
            id: workplane_id,
            definition_id,
            name: format!("{} workplane", item.name),
            kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
        },
        CanonicalCommand::CreateFeature {
            id: sketch_id,
            definition_id,
            name: format!("{} base sketch", item.name),
            kind: FeatureKind::Sketch(sketch),
        },
        CanonicalCommand::CreateFeature {
            id: body_id,
            definition_id,
            name: format!("{} base pad", item.name),
            kind: FeatureKind::Pad(PadSpec {
                profile: PadProfile::SketchRegion {
                    sketch: sketch_id,
                    region,
                },
                direction: FeatureDirection::AlongNormal,
                extent: FeatureExtent::Blind(height),
                operation: PadOperation::NewBody,
            }),
        },
    ];
    let mut target = body_id;
    let mut next_feature = body_id.0.checked_add(1)?;
    for (index, subtraction) in item.subtract_boxes.iter().enumerate() {
        let cut_workplane = FeatureId(next_feature);
        let cut_sketch = next_feature.checked_add(1).map(FeatureId)?;
        let cut_tool = next_feature.checked_add(2).map(FeatureId)?;
        let cut_result = next_feature.checked_add(3).map(FeatureId)?;
        let [cut_width, cut_depth, cut_height] = subtraction.size_mm;
        let [cut_x, cut_y, cut_z] = subtraction.origin_mm;
        let offset = Dimension::new(cut_z.to_string(), cut_z).ok()?;
        let distance = Dimension::new(cut_height.to_string(), cut_height).ok()?;
        let (sketch, region) =
            rectangle_sketch(cut_workplane, [cut_x, cut_y], [cut_width, cut_depth])?;
        commands.extend([
            CanonicalCommand::CreateFeature {
                id: cut_workplane,
                definition_id,
                name: format!("{} cut {} workplane", item.name, index + 1),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Offset {
                        base: workplane_id,
                        distance: offset,
                    },
                    frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(cut_z),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: cut_sketch,
                definition_id,
                name: format!("{} cut {} sketch", item.name, index + 1),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: cut_tool,
                definition_id,
                name: format!("{} cut {} tool", item.name, index + 1),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::SketchRegion {
                        sketch: cut_sketch,
                        region,
                    },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(distance),
                    operation: PadOperation::NewBody,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: cut_result,
                definition_id,
                name: format!("{} cut {} result", item.name, index + 1),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Cut,
                    target,
                    tool: cut_tool,
                },
            },
        ]);
        target = cut_result;
        next_feature = cut_result.0.checked_add(1)?;
    }
    Some((commands, next_feature.checked_sub(workplane_id.0)?))
}

#[allow(dead_code)]
fn assistant_subtracted_box_mesh(item: &AssistantBoxIntent) -> Option<MeshBodySpec> {
    let [width, depth, height] = item.size_mm;
    let mut xs = vec![0.0, width];
    let mut ys = vec![0.0, depth];
    let mut zs = vec![0.0, height];
    for cut in &item.subtract_boxes {
        xs.extend([cut.origin_mm[0], cut.origin_mm[0] + cut.size_mm[0]]);
        ys.extend([cut.origin_mm[1], cut.origin_mm[1] + cut.size_mm[1]]);
        zs.extend([cut.origin_mm[2], cut.origin_mm[2] + cut.size_mm[2]]);
    }
    for coordinates in [&mut xs, &mut ys, &mut zs] {
        coordinates.sort_by(f64::total_cmp);
        coordinates.dedup_by(|left, right| left.to_bits() == right.to_bits());
    }
    let nx = xs.len() - 1;
    let ny = ys.len() - 1;
    let nz = zs.len() - 1;
    let mut solid = vec![false; nx * ny * nz];
    let index = |x: usize, y: usize, z: usize| (z * ny + y) * nx + x;
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                let center = [
                    f64::midpoint(xs[x], xs[x + 1]),
                    f64::midpoint(ys[y], ys[y + 1]),
                    f64::midpoint(zs[z], zs[z + 1]),
                ];
                solid[index(x, y, z)] = !item.subtract_boxes.iter().any(|cut| {
                    (0..3).all(|axis| {
                        center[axis] > cut.origin_mm[axis]
                            && center[axis] < cut.origin_mm[axis] + cut.size_mm[axis]
                    })
                });
            }
        }
    }
    let mut vertices = Vec::<[f64; 3]>::new();
    let mut vertex_ids = BTreeMap::<(u64, u64, u64), u32>::new();
    let mut triangles = Vec::<[u32; 3]>::new();
    let mut add_quad = |points: [[f64; 3]; 4]| {
        let ids = points.map(|point| {
            let key = (point[0].to_bits(), point[1].to_bits(), point[2].to_bits());
            *vertex_ids.entry(key).or_insert_with(|| {
                let id = vertices.len() as u32;
                vertices.push(point);
                id
            })
        });
        triangles.extend([[ids[0], ids[1], ids[2]], [ids[0], ids[2], ids[3]]]);
    };
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                if !solid[index(x, y, z)] {
                    continue;
                }
                let (x0, x1) = (xs[x], xs[x + 1]);
                let (y0, y1) = (ys[y], ys[y + 1]);
                let (z0, z1) = (zs[z], zs[z + 1]);
                if x == 0 || !solid[index(x - 1, y, z)] {
                    add_quad([[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]]);
                }
                if x + 1 == nx || !solid[index(x + 1, y, z)] {
                    add_quad([[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]]);
                }
                if y == 0 || !solid[index(x, y - 1, z)] {
                    add_quad([[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]]);
                }
                if y + 1 == ny || !solid[index(x, y + 1, z)] {
                    add_quad([[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]]);
                }
                if z == 0 || !solid[index(x, y, z - 1)] {
                    add_quad([[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]]);
                }
                if z + 1 == nz || !solid[index(x, y, z + 1)] {
                    add_quad([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]);
                }
            }
        }
    }
    Some(MeshBodySpec {
        schema: MESH_BODY_SCHEMA_V1.to_owned(),
        vertices_mm: vertices,
        triangles,
        authority: MeshAuthority::Authored {
            provenance: "ketchup-assistant-subtracted-box-v1".to_owned(),
        },
    })
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
    ExportGeneralFabrication,
    ExportWeldmentCutList,
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
    About,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantProvider {
    AnthropicApi,
    OpenAiApi,
    #[cfg(feature = "private-oauth")]
    ClaudeCodeOauth,
    #[cfg(feature = "private-oauth")]
    CodexOauth,
}

impl AssistantProvider {
    const fn initial() -> Self {
        #[cfg(feature = "private-oauth")]
        {
            Self::CodexOauth
        }
        #[cfg(not(feature = "private-oauth"))]
        {
            Self::AnthropicApi
        }
    }

    const fn distribution(self) -> AssistantDistribution {
        match self {
            Self::AnthropicApi | Self::OpenAiApi => AssistantDistribution::PublicApi,
            #[cfg(feature = "private-oauth")]
            Self::ClaudeCodeOauth | Self::CodexOauth => AssistantDistribution::PrivateOauth,
        }
    }

    const fn protocol_name(self) -> &'static str {
        match self {
            Self::AnthropicApi => "anthropic-api",
            Self::OpenAiApi => "openai-api",
            #[cfg(feature = "private-oauth")]
            Self::ClaudeCodeOauth => "claude-code-oauth",
            #[cfg(feature = "private-oauth")]
            Self::CodexOauth => "codex-oauth",
        }
    }

    const fn label_key(self) -> &'static str {
        match self {
            Self::AnthropicApi => "assistant-provider-anthropic-api",
            Self::OpenAiApi => "assistant-provider-openai-api",
            #[cfg(feature = "private-oauth")]
            Self::ClaudeCodeOauth => "assistant-provider-claude-oauth",
            #[cfg(feature = "private-oauth")]
            Self::CodexOauth => "assistant-provider-codex-oauth",
        }
    }

    const fn default_model(self) -> &'static str {
        match self {
            Self::AnthropicApi => "claude-sonnet-5",
            Self::OpenAiApi => "gpt-5.2",
            #[cfg(feature = "private-oauth")]
            Self::ClaudeCodeOauth => "claude-sonnet-5",
            #[cfg(feature = "private-oauth")]
            Self::CodexOauth => "gpt-5.6-sol",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantWorkspaceMode {
    Dock,
    Tab,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantMessageRole {
    User,
    Assistant,
    Error,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantChatMessage {
    pub role: AssistantMessageRole,
    pub text: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<AssistantRejectionDiagnostic>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AssistantConversation {
    document_id: u64,
    messages: Vec<AssistantChatMessage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AssistantMemoryEntry {
    sequence: u64,
    user: String,
    assistant: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AssistantProjectMemory {
    schema: String,
    document_id: u64,
    next_sequence: u64,
    entries: Vec<AssistantMemoryEntry>,
}

impl AssistantProjectMemory {
    fn empty(document_id: u64) -> Self {
        Self {
            schema: ASSISTANT_MEMORY_SCHEMA.to_owned(),
            document_id,
            next_sequence: 1,
            entries: Vec::new(),
        }
    }

    fn validate(&self, document_id: u64) -> bool {
        if self.schema != ASSISTANT_MEMORY_SCHEMA
            || self.document_id != document_id
            || self.entries.len() > MAX_ASSISTANT_MEMORY_ENTRIES
            || self.next_sequence == 0
        {
            return false;
        }
        let mut previous = 0;
        for entry in &self.entries {
            if entry.sequence <= previous
                || entry.user.is_empty()
                || entry.assistant.is_empty()
                || entry.user.len() > MAX_ASSISTANT_MEMORY_TEXT_BYTES
                || entry.assistant.len() > MAX_ASSISTANT_MEMORY_TEXT_BYTES
            {
                return false;
            }
            previous = entry.sequence;
        }
        self.next_sequence > previous
    }

    fn remember(&mut self, user: &str, assistant: &str) {
        if self.next_sequence == u64::MAX {
            return;
        }
        let user = bounded_assistant_memory_text(user);
        let assistant = bounded_assistant_memory_text(assistant);
        if user.is_empty() || assistant.is_empty() {
            return;
        }
        self.entries.push(AssistantMemoryEntry {
            sequence: self.next_sequence,
            user,
            assistant,
        });
        self.next_sequence += 1;
        if self.entries.len() > MAX_ASSISTANT_MEMORY_ENTRIES {
            self.entries
                .drain(..self.entries.len() - MAX_ASSISTANT_MEMORY_ENTRIES);
        }
    }

    fn search(&self, query: &str) -> Vec<&AssistantMemoryEntry> {
        let query_tokens = assistant_memory_tokens(query);
        let mut ranked = self
            .entries
            .iter()
            .map(|entry| {
                let entry_tokens =
                    assistant_memory_tokens(&format!("{} {}", entry.user, entry.assistant));
                let score = query_tokens.intersection(&entry_tokens).count();
                (score, entry)
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.sequence.cmp(&left.1.sequence))
        });
        if query_tokens.is_empty() {
            return ranked.into_iter().map(|(_, entry)| entry).collect();
        }
        ranked
            .into_iter()
            .filter_map(|(score, entry)| (score > 0).then_some(entry))
            .collect()
    }

    fn retrieval_context(&self, query: &str) -> serde_json::Value {
        let query_tokens = assistant_memory_tokens(query);
        let mut ranked = self
            .entries
            .iter()
            .map(|entry| {
                let entry_tokens =
                    assistant_memory_tokens(&format!("{} {}", entry.user, entry.assistant));
                let score = query_tokens.intersection(&entry_tokens).count();
                (score, entry)
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.sequence.cmp(&left.1.sequence))
        });
        let has_positive_match = ranked.first().is_some_and(|item| item.0 > 0);
        let mut entries = Vec::new();
        for (score, entry) in ranked {
            if has_positive_match && score == 0 {
                continue;
            }
            let value = serde_json::json!({
                "sequence": entry.sequence,
                "user": entry.user,
                "assistant": entry.assistant,
                "sha256": assistant_memory_entry_sha256(entry),
            });
            entries.push(value);
            if entries.len() > MAX_ASSISTANT_MEMORY_RETRIEVAL_ENTRIES
                || serde_json::to_vec(&entries)
                    .is_ok_and(|bytes| bytes.len() > MAX_ASSISTANT_MEMORY_RETRIEVAL_BYTES)
            {
                entries.pop();
                break;
            }
        }
        let byte_length = serde_json::to_vec(&entries).map_or(0, |bytes| bytes.len());
        serde_json::json!({
            "schema": ASSISTANT_MEMORY_SCHEMA,
            "document_id": self.document_id,
            "stored_count": self.entries.len(),
            "retrieved_count": entries.len(),
            "complete": entries.len() == self.entries.len(),
            "byte_length": byte_length,
            "entries": entries,
        })
    }
}

fn bounded_assistant_memory_text(text: &str) -> String {
    let text = text.trim();
    if text.len() <= MAX_ASSISTANT_MEMORY_TEXT_BYTES {
        return text.to_owned();
    }
    let mut end = MAX_ASSISTANT_MEMORY_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].trim_end().to_owned()
}

fn assistant_memory_tokens(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

fn assistant_memory_entry_sha256(entry: &AssistantMemoryEntry) -> String {
    ketchup_model::graph::sha256_hex(
        format!("{}\n{}\n{}", entry.sequence, entry.user, entry.assistant).as_bytes(),
    )
}

fn assistant_conversation_digest(messages: &[AssistantChatMessage]) -> String {
    let bytes = serde_json::to_vec(messages).expect("assistant messages are serializable");
    ketchup_model::graph::sha256_hex(&bytes)
}

fn bounded_assistant_state_view(content: &str) -> serde_json::Value {
    let complete = content.len() <= MAX_ASSISTANT_STATE_VIEW_BYTES;
    let bounded = if complete {
        content
    } else {
        let mut end = MAX_ASSISTANT_STATE_VIEW_BYTES.min(content.len());
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        let line_end = content[..end].rfind('\n').map_or(end, |index| index + 1);
        &content[..line_end]
    };
    serde_json::json!({
        "format": AGENT_STATE_VIEW,
        "complete": complete,
        "byte_length": content.len(),
        "sha256": ketchup_model::graph::sha256_hex(content.as_bytes()),
        "content": bounded,
    })
}

fn assistant_context_byte_length(context: &serde_json::Value) -> usize {
    serde_json::to_vec(context).map_or(usize::MAX, |bytes| bytes.len())
}

fn assistant_interoperability_context(snapshot: &Snapshot) -> serde_json::Value {
    let receipt_count = snapshot
        .import_receipts()
        .filter(|receipt| matches!(receipt.format(), ImportFormat::Step | ImportFormat::Iges))
        .count();
    let imports = snapshot
        .import_receipts()
        .filter(|receipt| matches!(receipt.format(), ImportFormat::Step | ImportFormat::Iges))
        .take(64)
        .map(|receipt| {
            let diagnostic_count = receipt.diagnostics().len();
            serde_json::json!({
                "import_id": receipt.id().0,
                "format": match receipt.format() {
                    ImportFormat::Step => "step",
                    ImportFormat::Iges => "iges",
                    _ => unreachable!("interoperability context filters exact exchange formats"),
                },
                "source_name": receipt.source_name(),
                "exact_brep": "preserved",
                "source_parametric_history": "unavailable",
                "canonical_editability": "exact_brep_and_occurrence_metadata",
                "diagnostics_complete": diagnostic_count <= 32,
                "diagnostic_codes": receipt
                    .diagnostics()
                    .iter()
                    .take(32)
                    .map(|diagnostic| diagnostic.code())
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "complete": receipt_count <= 64,
        "import_count": receipt_count,
        "imports": imports,
    })
}

fn bounded_assistant_provider_text(text: &str) -> String {
    if text.len() <= MAX_ASSISTANT_PROVIDER_CONVERSATION_TEXT_BYTES {
        return text.to_owned();
    }
    let mut end = MAX_ASSISTANT_PROVIDER_CONVERSATION_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].trim_end().to_owned()
}

fn truncate_assistant_validation_arrays(value: &mut serde_json::Value, limit: usize) {
    let serde_json::Value::Object(object) = value else {
        return;
    };
    let mut truncated = false;
    let mut issues_truncated = false;
    for key in [
        "issues",
        "evaluations",
        "not_evaluated",
        "unavailable_occurrences",
    ] {
        let Some(array) = object
            .get_mut(key)
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        if array.len() > limit {
            array.truncate(limit);
            truncated = true;
            issues_truncated |= key == "issues";
        }
    }
    if truncated && object.contains_key("complete") {
        object.insert("complete".to_owned(), serde_json::Value::Bool(false));
    }
    if issues_truncated && object.contains_key("issues_complete") {
        object.insert("issues_complete".to_owned(), serde_json::Value::Bool(false));
    }
    for child in object.values_mut() {
        truncate_assistant_validation_arrays(child, limit);
    }
}

fn summarized_assistant_validation_context(validation: &serde_json::Value) -> serde_json::Value {
    let mut summary = serde_json::Map::new();
    for key in [
        "schema",
        "document_id",
        "revision",
        "canonical_digest",
        "selection_mode",
        "validators",
        "requested",
        "executed",
        "skipped",
        "selection_error",
        "state",
        "visible_occurrence_count",
        "checked_occurrence_count",
        "checked_pair_count",
        "issue_count",
    ] {
        if let Some(value) = validation.get(key) {
            summary.insert(key.to_owned(), value.clone());
        }
    }
    summary.insert("complete".to_owned(), serde_json::Value::Bool(false));
    summary.insert("issues_complete".to_owned(), serde_json::Value::Bool(false));
    summary.insert("issues".to_owned(), serde_json::Value::Array(Vec::new()));
    summary.insert(
        "details_truncated".to_owned(),
        serde_json::Value::Bool(true),
    );
    serde_json::Value::Object(summary)
}

fn bounded_assistant_provider_context(mut context: serde_json::Value) -> serde_json::Value {
    let inspection_catalog = context
        .as_object_mut()
        .and_then(|object| object.remove(ASSISTANT_LOCAL_INSPECTION_CATALOG));
    let mut context = bounded_assistant_public_context(context);
    if let Some(inspection_catalog) = inspection_catalog {
        context[ASSISTANT_LOCAL_INSPECTION_CATALOG] = inspection_catalog;
    }
    context
}

fn bounded_assistant_public_context(mut context: serde_json::Value) -> serde_json::Value {
    if assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
        return context;
    }
    let object = context
        .as_object_mut()
        .expect("assistant context is always an object");
    object.insert(
        "context_complete".to_owned(),
        serde_json::Value::Bool(false),
    );

    if let Some(boxes) = object
        .get_mut("boxes")
        .and_then(serde_json::Value::as_array_mut)
    {
        boxes.clear();
        object.insert("boxes_complete".to_owned(), serde_json::Value::Bool(false));
    }
    if let Some(state_view) = object
        .get_mut("state_view")
        .and_then(serde_json::Value::as_object_mut)
        && let Some(content) = state_view
            .get("content")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    {
        let bounded = if content.len() <= MAX_ASSISTANT_PROVIDER_STATE_VIEW_BYTES {
            content
        } else {
            let mut end = MAX_ASSISTANT_PROVIDER_STATE_VIEW_BYTES;
            while !content.is_char_boundary(end) {
                end -= 1;
            }
            content[..end].to_owned()
        };
        state_view.insert("content".to_owned(), serde_json::Value::String(bounded));
        state_view.insert("complete".to_owned(), serde_json::Value::Bool(false));
    }
    if let Some(conversation) = object
        .get_mut("conversation")
        .and_then(serde_json::Value::as_array_mut)
    {
        let original_len = conversation.len();
        if conversation.len() > MAX_ASSISTANT_PROVIDER_CONVERSATION_MESSAGES {
            conversation.drain(..conversation.len() - MAX_ASSISTANT_PROVIDER_CONVERSATION_MESSAGES);
        }
        let mut text_truncated = false;
        for message in conversation.iter_mut() {
            let Some(message) = message.as_object_mut() else {
                continue;
            };
            let Some(text) = message
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            let bounded = bounded_assistant_provider_text(&text);
            text_truncated |= bounded.len() != text.len();
            message.insert("text".to_owned(), serde_json::Value::String(bounded));
        }
        if original_len != conversation.len() || text_truncated {
            object.insert(
                "conversation_complete".to_owned(),
                serde_json::Value::Bool(false),
            );
        }
    }
    if assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
        return context;
    }

    if let Some(validation) = context.get_mut("validation") {
        truncate_assistant_validation_arrays(validation, 8);
    }
    if assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
        return context;
    }

    if let Some(references) = context
        .get_mut("topology_face_references")
        .and_then(serde_json::Value::as_array_mut)
    {
        references.clear();
        context["topology_face_references_complete"] = serde_json::Value::Bool(false);
    }
    if let Some(faces) = context
        .get_mut("fea_faces")
        .and_then(serde_json::Value::as_array_mut)
    {
        faces.clear();
        context["fea_faces_complete"] = serde_json::Value::Bool(false);
    }
    if let Some(references) = context
        .get_mut("topology_edge_references")
        .and_then(serde_json::Value::as_array_mut)
    {
        references.clear();
        context["topology_edge_references_complete"] = serde_json::Value::Bool(false);
    }
    if assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
        return context;
    }

    let selected = context["selected_occurrence_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_u64)
        .collect::<BTreeSet<_>>();
    loop {
        if assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
            return context;
        }
        let Some(occurrences) = context
            .get_mut("occurrences")
            .and_then(serde_json::Value::as_array_mut)
        else {
            break;
        };
        let removable = occurrences.iter().rposition(|occurrence| {
            occurrence["occurrence_id"]
                .as_u64()
                .is_some_and(|id| !selected.contains(&id))
        });
        let Some(index) = removable else {
            break;
        };
        occurrences.remove(index);
        context["occurrences_complete"] = serde_json::Value::Bool(false);
    }
    if assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
        return context;
    }

    if let Some(validation) = context.get_mut("validation") {
        truncate_assistant_validation_arrays(validation, 2);
    }
    if let Some(state_view) = context
        .get_mut("state_view")
        .and_then(serde_json::Value::as_object_mut)
    {
        state_view.insert(
            "content".to_owned(),
            serde_json::Value::String(String::new()),
        );
        state_view.insert("complete".to_owned(), serde_json::Value::Bool(false));
    }
    if assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
        return context;
    }

    if let Some(validation) = context.get_mut("validation") {
        truncate_assistant_validation_arrays(validation, 0);
        *validation = summarized_assistant_validation_context(validation);
    }
    if assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
        return context;
    }

    if let Some(conversation) = context
        .get_mut("conversation")
        .and_then(serde_json::Value::as_array_mut)
    {
        if conversation.len() > 2 {
            conversation.drain(..conversation.len() - 2);
        }
        context["conversation_complete"] = serde_json::Value::Bool(false);
    }
    if assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
        return context;
    }

    if let Some(occurrences) = context
        .get_mut("occurrences")
        .and_then(serde_json::Value::as_array_mut)
    {
        occurrences.clear();
        context["occurrences_complete"] = serde_json::Value::Bool(false);
    }
    if assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
        return context;
    }

    if let Some(conversation) = context
        .get_mut("conversation")
        .and_then(serde_json::Value::as_array_mut)
    {
        conversation.clear();
    }
    if let Some(memory) = context
        .get_mut("project_memory")
        .and_then(serde_json::Value::as_object_mut)
    {
        memory.insert("retrieved_count".to_owned(), serde_json::json!(0));
        memory.insert("byte_length".to_owned(), serde_json::json!(2));
        memory.insert("entries".to_owned(), serde_json::json!([]));
        let complete = memory
            .get("stored_count")
            .and_then(serde_json::Value::as_u64)
            == Some(0);
        memory.insert("complete".to_owned(), serde_json::Value::Bool(complete));
    }
    if assistant_context_byte_length(&context) > MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES {
        context["selected_profile_translation_target"] = serde_json::Value::Null;
        context["selected_parameter_edit_target"] = serde_json::Value::Null;
    }
    debug_assert!(
        assistant_context_byte_length(&context) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES,
        "minimal assistant provider context exceeds its byte envelope"
    );
    context
}

#[derive(Clone, Debug)]
pub struct AssistantTransportResponse {
    pub result: AssistantChatResult,
    pub cad_edit_program: Option<AssistantCadEditProgram>,
    pub fea_review: Option<AssistantFeaReviewRequest>,
    pub diagnostics: Option<AssistantApiDiagnostics>,
}

#[derive(Clone, Debug)]
struct AssistantApiLogEntry {
    request_id: String,
    diagnostics: AssistantApiDiagnostics,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AssistantInspectorTab {
    #[default]
    ApiLogs,
    Memory,
}

pub trait AssistantTransport: Send + Sync {
    fn chat(
        &self,
        handshake: AssistantHandshake,
        request_id: &str,
        message: &str,
        context: &serde_json::Value,
        cancellation: AssistantCancellation,
    ) -> Result<AssistantChatResult, Rejection>;

    fn chat_with_diagnostics(
        &self,
        handshake: AssistantHandshake,
        request_id: &str,
        message: &str,
        context: &serde_json::Value,
        cancellation: AssistantCancellation,
    ) -> Result<AssistantTransportResponse, Rejection> {
        self.chat(handshake, request_id, message, context, cancellation)
            .map(|result| AssistantTransportResponse {
                result,
                cad_edit_program: None,
                fea_review: None,
                diagnostics: None,
            })
    }
}

struct AssistantChatTask {
    receiver: Receiver<Result<AssistantTransportResponse, Rejection>>,
    request_id: String,
    message: String,
    replan_attempted: bool,
    started_at: Instant,
    cancellation: AssistantCancellation,
    document_id: DocumentId,
    revision_id: u64,
    canonical_digest: String,
    selected_occurrence_ids: Vec<u64>,
    source: String,
}

impl Drop for AssistantChatTask {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

fn assistant_fea_face_context(
    snapshot: &Snapshot,
    topology_results: &ExactResultRegistry,
) -> (bool, Vec<serde_json::Value>) {
    let mut faces = topology_results
        .body_values(snapshot)
        .unwrap_or_default()
        .into_values()
        .flat_map(|package| match package.as_ref() {
            ExactBodyPackage::Graph(package) => package
                .face_evidence
                .iter()
                .map(|face| {
                    serde_json::json!({
                        "definition_id": package.identity.definition_id.0,
                        "target_feature_id": package.identity.producer_feature_id.0,
                        "face_ordinal": face.face_ordinal,
                        "surface_kind": face.surface_kind,
                        "centroid_mm": face.centroid_mm,
                        "unit_normal": face.unit_normal,
                    })
                })
                .collect::<Vec<_>>(),
            ExactBodyPackage::Imported(_) => Vec::new(),
        })
        .collect::<Vec<_>>();
    faces.sort_unstable_by_key(|face| {
        (
            face["definition_id"].as_u64().unwrap_or_default(),
            face["target_feature_id"].as_u64().unwrap_or_default(),
            face["face_ordinal"].as_u64().unwrap_or_default(),
        )
    });
    let complete = faces.len() <= 256;
    faces.truncate(256);
    (complete, faces)
}

struct AssistantRequestSnapshot {
    snapshot: Snapshot,
    exact_results: ExactResultRegistry,
    topology_results: ExactResultRegistry,
    container_data: ContainerData,
    worker_path: Option<PathBuf>,
    query: String,
    project_memory: AssistantProjectMemory,
    conversation: Vec<AssistantChatMessage>,
    selected_paths: BTreeSet<InstancePath>,
    selected_occurrence_ids: Vec<u64>,
    selection_scope: &'static str,
    selected_group_id: Option<u64>,
    selected_profile_translation_target: serde_json::Value,
    selected_parameter_edit_target: serde_json::Value,
    preparation_delay: Duration,
}

impl AssistantRequestSnapshot {
    fn build(
        &self,
        cancellation: &AssistantCancellation,
        request_context: bool,
    ) -> Result<serde_json::Value, ketchup_scheduler::assistant::AssistantProcessError> {
        if cancellation.is_cancelled() {
            return Err(ketchup_scheduler::assistant::AssistantProcessError::Cancelled);
        }
        let delay_started = Instant::now();
        while delay_started.elapsed() < self.preparation_delay {
            if cancellation.is_cancelled() {
                return Err(ketchup_scheduler::assistant::AssistantProcessError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let semantic_state = encode_semantic_state(&self.snapshot);
        let state_view = bounded_assistant_state_view(&semantic_state.agent());
        let (fea_faces_complete, fea_faces) =
            assistant_fea_face_context(&self.snapshot, &self.topology_results);
        let project_memory = self.project_memory.retrieval_context(&self.query);
        let validation_selection = AssistantValidationSelection::parse(&self.query);
        let validation =
            ketchup_application::validation::assistant_validation_context_with_worker_cancellation(
                &self.snapshot,
                &self.exact_results,
                &ketchup_application::validation::AssistantValidationSelection {
                    mode: validation_selection.mode,
                    requested: validation_selection.requested,
                    unknown: validation_selection.unknown,
                },
                &self.container_data,
                self.worker_path.clone(),
                Duration::from_secs(30),
                cancellation.shared_flag(),
            );
        if cancellation.is_cancelled() {
            return Err(ketchup_scheduler::assistant::AssistantProcessError::Cancelled);
        }
        let body_bounds = assistant_body_bounds_from_snapshot(&self.snapshot, &self.exact_results);
        let occurrence_records =
            assistant_occurrence_records_from_snapshot(&self.snapshot, &self.exact_results);
        let occurrence_count = occurrence_records.len();
        let occurrences = occurrence_records
            .iter()
            .filter(|(path, _)| self.selected_paths.contains(path))
            .chain(
                occurrence_records
                    .iter()
                    .filter(|(path, _)| !self.selected_paths.contains(path)),
            )
            .take(100)
            .map(|(_, record)| record.clone())
            .collect::<Vec<_>>();
        let conversation = self
            .conversation
            .iter()
            .rev()
            .take(20)
            .rev()
            .map(|message| {
                serde_json::json!({
                    "role": match message.role {
                        AssistantMessageRole::User => "user",
                        AssistantMessageRole::Assistant => "assistant",
                        AssistantMessageRole::Error => "error",
                    },
                    "text": message.text,
                    "diagnostic": message.diagnostic,
                })
            })
            .collect::<Vec<_>>();
        let selected_instance_paths = self
            .selected_paths
            .iter()
            .map(KetchupApp::assistant_instance_path_label)
            .collect::<Vec<_>>();
        let mut topology_face_references = assistant_topology_references(
            &self.snapshot,
            &self.topology_results,
            TopologicalElementKind::Face,
        );
        let topology_face_references_complete = topology_face_references.len() <= 64;
        topology_face_references.truncate(64);
        let topology_face_references = topology_face_references
            .into_iter()
            .map(|reference| {
                serde_json::json!({
                    "definition_id": reference.definition_id.0,
                    "target_feature_id": reference.producer_feature_id.0,
                    "reference_id": reference.lineage_digest,
                })
            })
            .collect::<Vec<_>>();
        let mut topology_edge_references = assistant_topology_references(
            &self.snapshot,
            &self.topology_results,
            TopologicalElementKind::Edge,
        );
        let topology_edge_references_complete = topology_edge_references.len() <= 64;
        topology_edge_references.truncate(64);
        let topology_edge_references = topology_edge_references
            .into_iter()
            .map(|reference| {
                serde_json::json!({
                    "definition_id": reference.definition_id.0,
                    "target_feature_id": reference.producer_feature_id.0,
                    "reference_id": reference.lineage_digest,
                })
            })
            .collect::<Vec<_>>();
        let boxes = body_bounds
            .into_iter()
            .filter(|(path, _)| path.is_root())
            .take(100)
            .map(|(path, (definition_id, [minimum, maximum]))| {
                let size = maximum - minimum;
                serde_json::json!({
                    "occurrence_id": path.root_occurrence().0,
                    "definition_id": definition_id.0,
                    "origin_mm": [minimum.x, minimum.y, minimum.z],
                    "size_mm": [size.x, size.y, size.z],
                })
            })
            .collect::<Vec<_>>();
        let mut context = serde_json::json!({
            "document_id": self.snapshot.document_id().0,
            "revision": self.snapshot.revision_id(),
            "canonical_digest": self.snapshot.canonical_digest(),
            "state_view": state_view,
            "interoperability": assistant_interoperability_context(&self.snapshot),
            "project_memory": project_memory,
            "validation": validation,
            "selected_occurrence_ids": self.selected_occurrence_ids,
            "selected_instance_paths": selected_instance_paths,
            "selection_scope": self.selection_scope,
            "selected_group_id": self.selected_group_id,
            "selected_profile_translation_target": self.selected_profile_translation_target,
            "selected_parameter_edit_target": self.selected_parameter_edit_target,
            "topology_face_references_complete": topology_face_references_complete,
            "topology_face_references": topology_face_references,
            "fea_faces_complete": fea_faces_complete,
            "fea_faces": fea_faces,
            "topology_edge_references_complete": topology_edge_references_complete,
            "topology_edge_references": topology_edge_references,
            "occurrence_count": occurrence_count,
            "occurrences_complete": occurrence_count <= 100,
            "occurrences": occurrences,
            "boxes": boxes,
            "conversation": conversation,
        });
        if request_context {
            context = bounded_assistant_provider_context(context);
            let selection = self
                .selected_paths
                .iter()
                .map(|path| KetchupApp::assistant_instance_path_value(&self.snapshot, path))
                .collect::<Vec<_>>();
            context[ASSISTANT_LOCAL_INSPECTION_CATALOG] = serde_json::json!({
                "document_id": self.snapshot.document_id().0,
                "revision": self.snapshot.revision_id(),
                "canonical_digest": self.snapshot.canonical_digest(),
                "selection": selection,
                "occurrences": occurrence_records.into_iter().map(|(_, record)| record).collect::<Vec<_>>(),
            });
        }
        Ok(context)
    }
}

fn format_assistant_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn assistant_clock_frame(elapsed: Duration) -> &'static str {
    ["◴", "◷", "◶", "◵"][(elapsed.as_millis() / 250) as usize % 4]
}

struct AssistantPendingExecution {
    result: AssistantChatResult,
    cad_edit_program: Option<AssistantCadEditProgram>,
    message: String,
    replan_attempted: bool,
    document_id: DocumentId,
    revision_id: u64,
    canonical_digest: String,
    source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantIntentKind {
    CreateEvaluatorInput,
    CreateEvaluatorExpression,
    CreateEvaluatorRule,
    CreateRuleOverride,
    DeleteRuleOverride,
    CreateFeatureParameterBinding,
    DeleteFeatureParameterBinding,
    CreatePersistentDimension,
    CreateSpace,
    CreateClearanceVolume,
    CreateJoint,
    CloneProfileDefinitionAndRepoint,
    ConvertEmptyGroupToComponent,
    RecomputeFeatureParameter,
    DeleteJoint,
    DeleteSpace,
    DeleteClearanceVolume,
    DeletePersistentDimension,
    RuleDimension,
    EvaluatorName,
    EvaluatorExpression,
    RuleOutputs,
    FeatureDimension,
    ProfilePoints,
    DefinitionName,
    OccurrenceVisibility,
    OccurrenceTranslation,
    OccurrenceTag,
    TagVisibility,
    OccurrenceDefinition,
    OccurrenceParent,
    GroupTranslation,
    GroupParent,
    CollectionOccurrences,
    CreateTag,
    DeleteTag,
    CreateCollection,
    DeleteCollection,
    DeleteGroup,
    DeleteOccurrence,
    CreateDefinition,
    DeleteDefinition,
    CreateProfileFeature,
    DeleteProfileFeature,
    CreateGroup,
    CreateOccurrence,
}

#[derive(Clone, Debug, PartialEq)]
enum AssistantPreviewSource {
    Workflow(WorkflowIntent),
    Assembly(assembly_ui::AssemblyPreviewSource),
    Model(AssistantModelIntent),
    #[cfg_attr(not(test), allow(dead_code))]
    CadEdit(AssistantCadEditProgram),
    ValidationRepair(AssistantValidationSelection),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantRepairOperation {
    ResolveCollision {
        left_occurrence_id: u64,
        moved_occurrence_id: u64,
        delta_mm: [f64; 3],
    },
    RestoreGravitySupport {
        occurrence_id: u64,
        support_occurrence_id: u64,
        gravity_direction: [f64; 3],
        delta_mm: [f64; 3],
    },
}

impl AssistantRepairOperation {
    const fn validator(&self) -> &'static str {
        match self {
            Self::ResolveCollision { .. } => "collision",
            Self::RestoreGravitySupport { .. } => "gravity_support",
        }
    }

    const fn issue_code(&self) -> &'static str {
        match self {
            Self::ResolveCollision { .. } => "collision.detected",
            Self::RestoreGravitySupport { .. } => "gravity.unsupported",
        }
    }

    fn occurrence_ids(&self) -> Vec<u64> {
        match self {
            Self::ResolveCollision {
                left_occurrence_id,
                moved_occurrence_id,
                ..
            } => vec![*left_occurrence_id, *moved_occurrence_id],
            Self::RestoreGravitySupport { occurrence_id, .. } => vec![*occurrence_id],
        }
    }

    const fn moved_occurrence_id(&self) -> u64 {
        match self {
            Self::ResolveCollision {
                moved_occurrence_id,
                ..
            } => *moved_occurrence_id,
            Self::RestoreGravitySupport { occurrence_id, .. } => *occurrence_id,
        }
    }

    const fn delta_mm(&self) -> [f64; 3] {
        match self {
            Self::ResolveCollision { delta_mm, .. }
            | Self::RestoreGravitySupport { delta_mm, .. } => *delta_mm,
        }
    }

    fn validate(&self) -> bool {
        let occurrence_ids = match self {
            Self::ResolveCollision {
                left_occurrence_id,
                moved_occurrence_id,
                ..
            } => [*left_occurrence_id, *moved_occurrence_id],
            Self::RestoreGravitySupport {
                occurrence_id,
                support_occurrence_id,
                gravity_direction,
                ..
            } => {
                if gravity_direction
                    .iter()
                    .any(|component| !component.is_finite())
                    || (gravity_direction
                        .iter()
                        .map(|value| value * value)
                        .sum::<f64>()
                        - 1.0)
                        .abs()
                        > ROUNDING
                {
                    return false;
                }
                [*occurrence_id, *support_occurrence_id]
            }
        };
        occurrence_ids[0] != occurrence_ids[1]
            && self
                .delta_mm()
                .iter()
                .all(|component| component.is_finite() && component.abs() <= MAX_COORDINATE_MM)
            && self
                .delta_mm()
                .iter()
                .any(|component| component.abs() > f64::EPSILON)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantRepairProgram {
    pub schema: String,
    pub document_id: u64,
    pub revision_id: u64,
    pub canonical_digest: String,
    pub max_operations: usize,
    pub operations: Vec<AssistantRepairOperation>,
}

impl AssistantRepairProgram {
    fn validate(&self) -> bool {
        self.schema == ASSISTANT_REPAIR_PROGRAM_SCHEMA_V1
            && self.max_operations == MAX_ASSISTANT_VALIDATION_ISSUES
            && !self.operations.is_empty()
            && self.operations.len() <= self.max_operations
            && self
                .operations
                .iter()
                .all(AssistantRepairOperation::validate)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct AssistantRepairPreview {
    program: AssistantRepairProgram,
    validation_before: serde_json::Value,
    validation_after: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq)]
struct AssistantPreviewPlan {
    source: AssistantPreviewSource,
    proposal: Proposal,
    repair: Option<AssistantRepairPreview>,
}

impl std::ops::Deref for AssistantPreviewPlan {
    type Target = Proposal;

    fn deref(&self) -> &Self::Target {
        &self.proposal
    }
}

fn assistant_rejection_phase_key(phase: AssistantRejectionPhase) -> &'static str {
    match phase {
        AssistantRejectionPhase::IntentValidation => "assistant-rejection-phase-intent-validation",
        AssistantRejectionPhase::ProposalPlanning => "assistant-rejection-phase-proposal-planning",
        AssistantRejectionPhase::CanonicalValidation => {
            "assistant-rejection-phase-canonical-validation"
        }
        AssistantRejectionPhase::ExactValidation => "assistant-rejection-phase-exact-validation",
        AssistantRejectionPhase::DomainValidation => "assistant-rejection-phase-domain-validation",
        AssistantRejectionPhase::CommitValidation => "assistant-rejection-phase-commit-validation",
    }
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
        ProposalPrepareError::RequestedBudgetExceeded => assistant_rejection(
            AssistantRejectionPhase::ProposalPlanning,
            "planning.requested_budget_exceeded",
            operation,
            target,
            "The proposal exceeds its declared work budget.",
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

fn assistant_feature_edit_rejection(
    error: ketchup_model::feature_history::BodyParameterEditError,
    operation: &str,
    target: &str,
) -> AssistantRejection {
    match error {
        ketchup_model::feature_history::BodyParameterEditError::Proposal(error) => {
            assistant_proposal_prepare_rejection(error, operation, target)
        }
        error => assistant_planning_rejection(
            "planning.feature_edit_rejected",
            operation,
            target,
            error.to_string(),
            "Refresh the exact feature target and request a supported bounded edit.",
        ),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssistantVerification {
    pub revision_id: u64,
    pub command_digest: String,
    pub result_digest: String,
    pub canonical_digest: String,
    pub verified_write_count: usize,
    pub repair_program: Option<AssistantRepairProgram>,
    pub repair_validator: Option<String>,
    pub validation_before: Option<serde_json::Value>,
    pub validation_after: Option<serde_json::Value>,
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
    tag: Option<TagId>,
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
    source_tags: BTreeMap<OccurrenceId, Option<TagId>>,
    available_tags: BTreeMap<TagId, String>,
    initial_tag: Option<TagId>,
}

#[derive(Clone, Debug, PartialEq)]
struct TagAssignmentPlan {
    source: TagAssignmentSourcePlan,
    target_tag: Option<TagId>,
    target_tag_name: String,
    changed_occurrence_ids: BTreeSet<OccurrenceId>,
    commands: Vec<CanonicalCommand>,
}

#[derive(Clone, Debug)]
struct PendingTagAssignment {
    source: TagAssignmentSourcePlan,
    target_tag: Option<TagId>,
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
    moving_tag: Option<TagId>,
    moving_visible: bool,
    moving_box: RenderBox,
    reference_id: OccurrenceId,
    reference_definition_id: DefinitionId,
    reference_name: String,
    reference_transform: Transform,
    reference_parent: Option<GroupId>,
    reference_tag: Option<TagId>,
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
    tag: Option<TagId>,
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
    source_tag: Option<TagId>,
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
    source_tag: Option<TagId>,
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
    source_tag: Option<TagId>,
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
    assembly_editor: assembly_ui::AssemblyEditorState,
    body_editor: body_ui::BodyEditorState,
    face_workflow: face_workflow_ui::FaceWorkflowUiState,
    feature_history: feature_history_ui::FeatureHistoryUiState,
    push_pull: app_state::PushPullState,
    solid_tools: app_state::SolidToolInputs,
    helix_tool: helix_ui::HelixUiState,
    parameter: app_state::ParameterEditor,
    validator_panel: app_state::ValidatorPanel,
    status_key: &'static str,
    theme: ThemeKind,
    camera: app_state::CameraState,
    view: ViewSettings,
    selection: SelectionState,
    hover: app_state::HoverState,
    active_tool: ActiveTool,
    panels: app_state::Panels,
    digest: String,
    assistant: app_state::AssistantState,
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

fn assistant_model_catalog_text() -> String {
    let path = std::env::var_os("KETCHUP_ASSISTANT_MODELS")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe().ok().and_then(|path| {
                path.parent()
                    .map(|parent| parent.join("assistant-models.yaml"))
            })
        });
    assistant_model_catalog_text_from_path(path.as_deref())
}

fn assistant_model_catalog_text_from_path(path: Option<&Path>) -> String {
    path.and_then(|path| {
        let file = std::fs::File::open(path).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > MAX_ASSISTANT_MODEL_CATALOG_BYTES {
            return None;
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_ASSISTANT_MODEL_CATALOG_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_ASSISTANT_MODEL_CATALOG_BYTES {
            return None;
        }
        String::from_utf8(bytes).ok()
    })
    .unwrap_or_else(|| ASSISTANT_MODELS_YAML.to_owned())
}

fn assistant_models_for(provider: AssistantProvider) -> Vec<String> {
    let catalog = assistant_model_catalog_text();
    catalog
        .lines()
        .filter_map(|line| {
            let mut value = line.trim().strip_prefix('-')?.trim();
            if let Some(rest) = value.strip_prefix("name:") {
                value = rest.trim();
            }
            value = value.trim_matches('"');
            if value.is_empty() || value.starts_with("──") {
                return None;
            }
            let subscription = value.contains("[sub]");
            let api = value.contains("[api]");
            let model = value
                .replace(" [api]", "")
                .replace("[sub]", "")
                .trim()
                .to_owned();
            let compatible = match provider {
                AssistantProvider::AnthropicApi => model.starts_with("claude-") && !subscription,
                AssistantProvider::OpenAiApi => model.starts_with("gpt-") && api,
                #[cfg(feature = "private-oauth")]
                AssistantProvider::ClaudeCodeOauth => model.starts_with("claude-") && !api,
                #[cfg(feature = "private-oauth")]
                AssistantProvider::CodexOauth => model.starts_with("gpt-") && subscription,
            };
            compatible.then_some(model)
        })
        .collect()
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

fn assistant_definition_local_bounds(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    definition_id: DefinitionId,
    local_box: Option<ProjectedBox>,
) -> Option<[Vec3; 2]> {
    if let Some(package) = exact_results.get_render(snapshot, definition_id) {
        let [minimum, maximum] = package.bounds_mm();
        return Some([
            Vec3::new(minimum[0], minimum[1], minimum[2]),
            Vec3::new(maximum[0], maximum[1], maximum[2]),
        ]);
    }
    if let Some(definition) = snapshot.definition(definition_id) {
        if let [feature_id] = definition.feature_ids()
            && let Some(feature) = snapshot.feature(*feature_id)
            && let FeatureKind::ImportedExactBody(spec) = feature.kind()
        {
            return Some([
                Vec3::new(
                    spec.bounds_mm[0][0],
                    spec.bounds_mm[0][1],
                    spec.bounds_mm[0][2],
                ),
                Vec3::new(
                    spec.bounds_mm[1][0],
                    spec.bounds_mm[1][1],
                    spec.bounds_mm[1][2],
                ),
            ]);
        }
        if let Some([minimum, maximum]) =
            definition
                .feature_ids()
                .iter()
                .rev()
                .find_map(|feature_id| {
                    ExactBRepGraph::from_snapshot(snapshot, definition_id, *feature_id)
                        .ok()?
                        .producer_bounds_mm()
                        .ok()?
                })
        {
            return Some([
                Vec3::new(minimum[0], minimum[1], minimum[2]),
                Vec3::new(maximum[0], maximum[1], maximum[2]),
            ]);
        }
        for feature_id in definition.feature_ids() {
            if let Some(feature) = snapshot.feature(*feature_id)
                && let FeatureKind::MeshBody(mesh) = feature.kind()
            {
                return bounds_of(
                    mesh.vertices_mm
                        .iter()
                        .map(|vertex| Vec3::new(vertex[0], vertex[1], vertex[2])),
                );
            }
        }
    }
    local_box.map(|item| [item.origin_mm, item.origin_mm + item.size_mm])
}

fn assistant_mesh_body_bounds(
    snapshot: &Snapshot,
    occurrence: &SceneOccurrence,
) -> Option<[Vec3; 2]> {
    let definition = snapshot.definition(occurrence.definition_id)?;
    let vertices = definition
        .feature_ids()
        .iter()
        .filter_map(|feature_id| snapshot.feature(*feature_id))
        .find_map(|feature| match feature.kind() {
            FeatureKind::MeshBody(mesh) => Some(mesh.vertices_mm.as_slice()),
            _ => None,
        })?;
    let mut points = vertices.iter().map(|point| {
        transform_model_point(
            occurrence.transform,
            Vec3::new(point[0], point[1], point[2]),
        )
    });
    let first = points.next()?;
    Some(points.fold([first, first], |[minimum, maximum], point| {
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
    }))
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
    let trimmed = input.trim();
    let numeric = trimmed
        .strip_suffix("mm")
        .or_else(|| trimmed.strip_suffix("MM"))
        .unwrap_or(trimmed)
        .trim();
    let distance = numeric.parse::<f64>().ok()?;
    distance.is_finite().then_some(distance)
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
mod program_source_ui_tests;
#[cfg(test)]
mod push_pull_snapping_tests;
#[cfg(test)]
mod rotation_input_tests;
#[cfg(test)]
mod scene_snapping_tests;
#[cfg(test)]
mod tests;
