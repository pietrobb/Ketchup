// Unsafe code is denied, not forbidden, for exactly one audited reason: the
// native file dialogs must borrow the raw main-window handle to be owned by,
// and modal to, the Ketchup window. See `dialogs::DialogParentWindow`.
#![deny(unsafe_code)]

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantBoxIntent, AssistantCapability, AssistantChatResult,
    AssistantDistribution, AssistantHandshake, AssistantModelIntent,
};
use ketchup_core::beam_m4ae::{
    BeamChangeSummary, BeamSlice, BeamValidationVerdict, BeamWorkspace, GroovePosition, GroupedBom,
};
use ketchup_core::beam_m5::{BeamExactPiecePackage, BeamM5Products};
use ketchup_core::bottle_m6::{BottleAuthorityReport, ExactRevolvePackage, ExactRevolveRequest};
use ketchup_core::document::{
    AuthenticatedApprover, AuthoritativeDependency, BOTTLE_SHELL_OPENING_FACE_ROLE,
    BOTTLE_SHOULDER_EDGE_ROLE, BooleanOperation, BottleControlDimension, BottleEdgeFinishKind,
    CanonicalCommand, CollectionId, CommandBatch, DefinitionId, Dimension, DimensionDisplayUnit,
    DimensionPresentation, DocumentId, DocumentStore, EvaluationIdentity, FeatureId, FeatureKind,
    FeatureParameterSlot, FeatureParameterTarget, GroupId, HighRiskClass, HighRiskScope,
    InstancePath, LoftSection, MAX_HUMAN_CONFIRMATION_LIFETIME_MS, MESH_BODY_SCHEMA_V1,
    MeshAuthority, MeshBodySpec, NodeId, OccurrenceId, PersistentDimensionId, ProfileSegment,
    Proposal, ProposalCommitError, ProposalContext, ProposalGoal, ProposalPrincipal, ProposalValue,
    SceneOccurrence, SceneQueryContext, SideEffectAuthorizationReceipt, SlotPath, Snapshot,
    SolidToolPlan, StableEdgeRole, StableFaceRole, TagId, TipReplacementParent,
    TipReplacementProposal, Transform, TrustedConfirmationSurface,
};
#[cfg(test)]
use ketchup_core::document::{
    OverrideParameterSpec, PersistentDimension, PersistentDimensionTarget, SlotResolution,
};
use ketchup_core::exact_product::{
    AssemblySelectionTarget, ExactBodyPackage, ExactBodyView, ExactFaceRole,
    ExactFeatureChainRequest, ExactLoftRequest, ExactMeshExport, ExactPlanarOffsetRequest,
    ExactResultRegistry, ExactStlExport, ExactSweepRequest, exact_model_stl_export,
};
use ketchup_core::fabrication::{FullBomProjection, PieceDimensionSheet};
use ketchup_core::graph::{
    DerivedIdentity, EvaluationStatus, EvaluatorNodeKind, RuleOutput, SlotSegment, sha256_bytes,
};
use ketchup_core::import::{
    ImportFormat, ImportLengthUnit, ImportUnitAuthority, ImportUnitDecision, MAX_STL_SOURCE_BYTES,
    plan_stl_import,
};
use ketchup_core::intent::{IntentRequest, WorkflowIntent, propose_intent};
use ketchup_core::prismatic::JointId;
#[cfg(test)]
use ketchup_core::prismatic::TolerancePolicy;
#[cfg(test)]
use ketchup_core::space::ClearanceOwner;
use ketchup_core::space::{ClearanceSeverity, ClearanceVolumeId, SpaceId};
use ketchup_core::validation::ValidationReport;
use ketchup_interaction::{
    Axis, ElementId, ExactHit, LocaleCatalog, PickResult, Ray, SelectionId, Side, SnapKind,
    SnapPolicy, SnapResult, SnapTracker, Vec3,
    exact_projection::ExactInteractionProjection,
    mesh_projection::MeshInteractionProjection,
    projection::{CanonicalInteractionProjection, InteractionProjection, ProjectedBox},
};
use ketchup_scheduler::{
    ExactWorkerSupervisor,
    assistant::{AssistantCancellation, AssistantProcessClient},
};
pub mod dialogs;
pub mod theme;

use theme::{Icon, Palette, ThemeKind};
pub mod renderer;

use dialogs::{
    DialogParentWindow, DiscardRequest, ExportRequest, FileDialogs, HighRiskConfirmationRequest,
    ImportDialogRequest, NativeFileDialogs, SaveRequest,
};
use renderer::{
    DerivedRenderCache, GpuInstancedRenderer, InstancedRenderPlan, ScenePaintCallback,
    feature_edges,
};

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
const MIN_CAMERA_ZOOM: f32 = 1.0e-6;
/// Largest useful magnification before floating-point picking becomes unstable.
const MAX_CAMERA_ZOOM: f32 = 8.0;
const ASSISTANT_MODELS_YAML: &str = include_str!("../assistant-models.yaml");
const ASSISTANT_CHAT_NAMESPACE: &str = "org.ketchup.assistant";
const ASSISTANT_CHAT_PATH: &str = "conversation-v1.json";
const ASSISTANT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeDocumentInspection {
    pub schema_version: u16,
    pub document_id: u64,
    pub revision: u64,
    pub canonical_digest: String,
    pub container_sha256: String,
    pub definitions: usize,
    pub root_occurrences: usize,
    pub profiles: usize,
    pub extrusions: usize,
    pub profile_extrusion_definitions: usize,
    pub visible_profile_extrusion_root_occurrences: usize,
}

impl NativeDocumentInspection {
    #[must_use]
    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema_version\":{},\"document_id\":{},\"revision\":{},\"canonical_digest\":\"{}\",\"container_sha256\":\"{}\",\"definitions\":{},\"root_occurrences\":{},\"profiles\":{},\"extrusions\":{},\"profile_extrusion_definitions\":{},\"visible_profile_extrusion_root_occurrences\":{}}}",
            self.schema_version,
            self.document_id,
            self.revision,
            self.canonical_digest,
            self.container_sha256,
            self.definitions,
            self.root_occurrences,
            self.profiles,
            self.extrusions,
            self.profile_extrusion_definitions,
            self.visible_profile_extrusion_root_occurrences
        )
    }
}

pub fn inspect_native_document(path: &Path) -> Result<NativeDocumentInspection, String> {
    let container_bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let loaded = ketchup_core::persistence::load_file(path).map_err(|error| error.to_string())?;
    if loaded.source_schema() != ketchup_core::persistence::CURRENT_SCHEMA
        || loaded.disposition() != ketchup_core::persistence::LoadDisposition::EditableLossless
    {
        return Err("document is not a lossless current-schema document".to_owned());
    }
    let snapshot = loaded.snapshot();
    let profiles = snapshot
        .features()
        .filter(|feature| {
            matches!(
                feature.kind(),
                FeatureKind::Profile { .. } | FeatureKind::SegmentProfile { .. }
            )
        })
        .count();
    let extrusions = snapshot
        .features()
        .filter(|feature| matches!(feature.kind(), FeatureKind::Extrusion { .. }))
        .count();
    let profile_extrusion_definition_ids = snapshot
        .definitions()
        .filter(|definition| {
            definition.feature_ids().iter().any(|feature_id| {
                let Some(feature) = snapshot.feature(*feature_id) else {
                    return false;
                };
                let FeatureKind::Extrusion { profile, .. } = feature.kind() else {
                    return false;
                };
                snapshot.feature(*profile).is_some_and(|profile_feature| {
                    profile_feature.definition_id() == definition.id()
                        && matches!(
                            profile_feature.kind(),
                            FeatureKind::Profile { .. }
                                | FeatureKind::SegmentProfile { closed: true, .. }
                        )
                })
            })
        })
        .map(|definition| definition.id())
        .collect::<BTreeSet<_>>();
    let visible_profile_extrusion_root_occurrences = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| {
            occurrence.visible
                && occurrence.instance_path.is_root()
                && profile_extrusion_definition_ids.contains(&occurrence.definition_id)
        })
        .count();
    Ok(NativeDocumentInspection {
        schema_version: loaded.source_schema(),
        document_id: snapshot.document_id().0,
        revision: snapshot.revision_id(),
        canonical_digest: snapshot.canonical_digest(),
        container_sha256: ketchup_core::graph::sha256_hex(&container_bytes),
        definitions: snapshot.definitions().count(),
        root_occurrences: snapshot.occurrences().count(),
        profiles,
        extrusions,
        profile_extrusion_definitions: profile_extrusion_definition_ids.len(),
        visible_profile_extrusion_root_occurrences,
    })
}

pub type SelectedAdapterInfo = Arc<Mutex<Option<eframe::wgpu::AdapterInfo>>>;

pub struct AdapterRequirement {
    pub name: String,
    pub device_type: eframe::wgpu::DeviceType,
}

#[derive(Clone, Copy)]
struct PushPullDrag {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BottleFeatureIds {
    control: FeatureId,
    shell: FeatureId,
    finish: FeatureId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BottleEditorInputs {
    definition_id: DefinitionId,
    body_radius: String,
    body_height: String,
    shoulder_rise: String,
    thickness: String,
    finish_amount: String,
    finish_kind: BottleEdgeFinishKind,
}

#[derive(Clone, Copy)]
struct BottleDirectDrag {
    definition_id: DefinitionId,
    feature_id: FeatureId,
    control: BottleControlDimension,
    pointer_start: Pos2,
    value_start_mm: f64,
    screen_direction: Vec2,
    pixels_per_mm: f32,
}

#[derive(Clone)]
struct MoveDrag {
    source_document_id: DocumentId,
    source_revision: u64,
    selection: SelectionId,
    group_id: Option<GroupId>,
    pointer_start_world: Vec3,
    plane_z: f64,
    delta_mm: Vec3,
    copy: bool,
}

#[derive(Clone, Copy)]
struct LastMove {
    occurrence_id: OccurrenceId,
    direction: Vec3,
    applied_distance_mm: f64,
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

#[derive(Clone, Debug, PartialEq)]
struct EphemeralBoxPreview {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    context: Option<EditContext>,
    selection_state: Option<SelectionId>,
    target: SelectionId,
    command_digest: String,
    box_data: RenderBox,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SmartPushPullChoice {
    NewFeature,
    CircularCut(OccurrenceId),
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

    fn commit(&self, document: &mut DocumentStore) -> bool {
        match self {
            Self::Append(proposal) => document.commit_verified_proposal(proposal).is_ok(),
            Self::TipReplacement(proposal) => {
                document.commit_tip_replacement_proposal(proposal).is_ok()
            }
        }
    }
}

#[derive(Clone)]
struct SmartPushPullChooser {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    selection: SelectionId,
    distance_mm: f64,
    planning: SmartPushPullPlanning,
    targets: Vec<RenderBox>,
    selected: SmartPushPullChoice,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignMode {
    Minimum,
    Center,
    Maximum,
}

const MAX_LINEAR_PATTERN_COUNT: usize = 10_000;

#[derive(Clone, Debug, PartialEq)]
struct OccurrenceOperationPreview {
    source_revision: u64,
    command_digest: String,
    batch: CommandBatch,
    boxes: BTreeMap<OccurrenceId, RenderBox>,
    hidden_occurrences: BTreeSet<OccurrenceId>,
    selection_after: Option<SelectionId>,
    committed_digest_key: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
struct RevolveToolState {
    source_revision: u64,
    source_digest: String,
    definition_id: DefinitionId,
    profile_feature_id: FeatureId,
    translation_mm: Vec3,
    plane_z: f64,
    axis_start_mm: Option<[f64; 2]>,
    axis_end_mm: Option<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
struct RevolvePreview {
    source_revision: u64,
    source_digest: String,
    definition_id: DefinitionId,
    profile_feature_id: FeatureId,
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    angle_degrees: f64,
    command_digest: String,
    batch: CommandBatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralFinishKind {
    Shell,
    Fillet,
    Chamfer,
}

#[derive(Clone, Debug, PartialEq)]
struct PlanarOffsetPreview {
    source_revision: u64,
    source_digest: String,
    definition_id: DefinitionId,
    profile_feature_id: FeatureId,
    distance_mm: f64,
    bounds_mm: [[f64; 3]; 2],
    command_digest: String,
    batch: CommandBatch,
}

#[derive(Clone, Debug, PartialEq)]
struct SweepPreview {
    source_revision: u64,
    source_digest: String,
    definition_id: DefinitionId,
    profile_feature_id: FeatureId,
    path_feature_id: FeatureId,
    bounds_mm: [[f64; 3]; 2],
    volume_mm3: f64,
    command_digest: String,
    batch: CommandBatch,
}

#[derive(Clone, Debug, PartialEq)]
struct LoftPreview {
    source_revision: u64,
    source_digest: String,
    definition_id: DefinitionId,
    sections: Vec<LoftSection>,
    bounds_mm: [[f64; 3]; 2],
    control_point_count: usize,
    command_digest: String,
    batch: CommandBatch,
}

#[derive(Clone, Debug, PartialEq)]
struct GeneralFinishPreview {
    source_revision: u64,
    source_digest: String,
    definition_id: DefinitionId,
    target_feature_id: FeatureId,
    stable_role: String,
    kind: GeneralFinishKind,
    amount_mm: f64,
    command_digest: String,
    batch: CommandBatch,
}

#[derive(Clone)]
struct PocketPreview {
    source_document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    selection: SelectionId,
    command_digest: String,
    batch: CommandBatch,
    start: Vec3,
    end: Vec3,
    depth_mm: f64,
    shared_count: usize,
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
pub type LoftPreviewParameters = (Vec<(FeatureId, f64)>, [[f64; 3]; 2], usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveTool {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    CutThrough,
    Pocket,
    SolidSubtract,
    SolidUnion,
    SolidIntersect,
    SolidSplit,
    PlanarOffset,
    Sweep,
    Loft,
    Revolve,
    Shell,
    Fillet,
    Chamfer,
    PushPull,
    Move,
    Measure,
    Orbit,
    Pan,
}

impl ActiveTool {
    const fn label_key(self) -> &'static str {
        match self {
            Self::Select => "tool-select",
            Self::Line => "tool-line",
            Self::Rectangle => "tool-rectangle",
            Self::Circle => "tool-circle",
            Self::Arc => "tool-arc",
            Self::CutThrough => "feature-cut-through",
            Self::Pocket => "feature-pocket",
            Self::SolidSubtract => "solid-tool-subtract",
            Self::SolidUnion => "solid-tool-union",
            Self::SolidIntersect => "solid-tool-intersect",
            Self::SolidSplit => "solid-tool-split",
            Self::PlanarOffset => "feature-planar-offset",
            Self::Sweep => "feature-sweep",
            Self::Loft => "feature-loft",
            Self::Revolve => "feature-revolve",
            Self::Shell => "feature-shell",
            Self::Fillet => "feature-fillet",
            Self::Chamfer => "feature-chamfer",
            Self::PushPull => "tool-push-pull",
            Self::Move => "tool-move",
            Self::Measure => "tool-measure",
            Self::Orbit => "tool-orbit",
            Self::Pan => "tool-pan",
        }
    }

    const fn hint_key(self) -> &'static str {
        match self {
            Self::Select => "hint-select",
            Self::Line => "hint-line",
            Self::Rectangle => "hint-rectangle",
            Self::Circle => "hint-circle",
            Self::Arc => "hint-arc",
            Self::CutThrough => "hint-cut-through",
            Self::Pocket => "hint-pocket",
            Self::SolidSubtract => "hint-solid-subtract",
            Self::SolidUnion => "hint-solid-union",
            Self::SolidIntersect => "hint-solid-intersect",
            Self::SolidSplit => "hint-solid-split",
            Self::PlanarOffset => "hint-planar-offset",
            Self::Sweep => "hint-sweep",
            Self::Loft => "hint-loft",
            Self::Revolve => "hint-revolve",
            Self::Shell => "hint-shell",
            Self::Fillet => "hint-fillet",
            Self::Chamfer => "hint-chamfer",
            Self::PushPull => "hint-push-pull",
            Self::Move => "hint-move",
            Self::Measure => "hint-measure",
            Self::Orbit => "hint-orbit",
            Self::Pan => "hint-pan",
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
    ExportExactStep,
    ExportMeshStl,
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    CutThrough,
    Pocket,
    SolidSubtract,
    SolidUnion,
    SolidIntersect,
    SolidSplit,
    PlanarOffset,
    Sweep,
    Loft,
    Revolve,
    Shell,
    Fillet,
    Chamfer,
    PushPull,
    Move,
    Measure,
    Orbit,
    Pan,
    Undo,
    Redo,
    Copy,
    Paste,
    Delete,
    Deselect,
    SelectAll,
    Group,
    Ungroup,
    MakeComponent,
    MakeUnique,
    Hide,
    Unhide,
    ViewIso,
    ViewTop,
    ViewFront,
    ViewProjection,
    ZoomFit,
    Shortcuts,
}

/// How the viewport maps the model onto the screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionMode {
    /// Converging view — parallel edges meet, the way an eye sees a room.
    Perspective,
    /// Parallel view — parallel edges stay parallel, the way a drawing measures.
    Parallel,
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
    shortcut_key: &'static str,
    tool: Option<ActiveTool>,
    implemented: bool,
}

struct CommandRegistry;

impl CommandRegistry {
    const COMMANDS: [CommandSpec; 49] = [
        CommandSpec {
            id: AppCommand::New,
            label_key: "file-new",
            shortcut_key: "shortcut-new",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Open,
            label_key: "file-open",
            shortcut_key: "shortcut-open",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Save,
            label_key: "file-save",
            shortcut_key: "shortcut-save",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::SaveAs,
            label_key: "file-save-as",
            shortcut_key: "shortcut-save-as",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::ImportMeshStl,
            label_key: "file-import-stl",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::ExportExactStep,
            label_key: "file-export-exact",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::ExportMeshStl,
            label_key: "file-export-mesh",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Select,
            label_key: "tool-select",
            shortcut_key: "shortcut-space",
            tool: Some(ActiveTool::Select),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Line,
            label_key: "tool-line",
            shortcut_key: "shortcut-line",
            tool: Some(ActiveTool::Line),
            implemented: false,
        },
        CommandSpec {
            id: AppCommand::Rectangle,
            label_key: "tool-rectangle",
            shortcut_key: "shortcut-rectangle",
            tool: Some(ActiveTool::Rectangle),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Circle,
            label_key: "tool-circle",
            shortcut_key: "shortcut-circle",
            tool: Some(ActiveTool::Circle),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Arc,
            label_key: "tool-arc",
            shortcut_key: "shortcut-arc",
            tool: Some(ActiveTool::Arc),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::CutThrough,
            label_key: "feature-cut-through",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::CutThrough),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Pocket,
            label_key: "feature-pocket",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::Pocket),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::SolidSubtract,
            label_key: "solid-tool-subtract",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::SolidSubtract),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::SolidUnion,
            label_key: "solid-tool-union",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::SolidUnion),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::SolidIntersect,
            label_key: "solid-tool-intersect",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::SolidIntersect),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::SolidSplit,
            label_key: "solid-tool-split",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::SolidSplit),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::PlanarOffset,
            label_key: "feature-planar-offset",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::PlanarOffset),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Sweep,
            label_key: "feature-sweep",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::Sweep),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Loft,
            label_key: "feature-loft",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::Loft),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Revolve,
            label_key: "feature-revolve",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::Revolve),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Shell,
            label_key: "feature-shell",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::Shell),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Fillet,
            label_key: "feature-fillet",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::Fillet),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Chamfer,
            label_key: "feature-chamfer",
            shortcut_key: "shortcut-none",
            tool: Some(ActiveTool::Chamfer),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::PushPull,
            label_key: "tool-push-pull",
            shortcut_key: "shortcut-push-pull",
            tool: Some(ActiveTool::PushPull),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Move,
            label_key: "tool-move",
            shortcut_key: "shortcut-move",
            tool: Some(ActiveTool::Move),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Measure,
            label_key: "tool-measure",
            shortcut_key: "shortcut-measure",
            tool: Some(ActiveTool::Measure),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Orbit,
            label_key: "tool-orbit",
            shortcut_key: "shortcut-orbit",
            tool: Some(ActiveTool::Orbit),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Pan,
            label_key: "tool-pan",
            shortcut_key: "shortcut-pan",
            tool: Some(ActiveTool::Pan),
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Undo,
            label_key: "action-undo",
            shortcut_key: "shortcut-undo",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Redo,
            label_key: "action-redo",
            shortcut_key: "shortcut-redo",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Copy,
            label_key: "action-copy",
            shortcut_key: "shortcut-copy",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Paste,
            label_key: "action-paste",
            shortcut_key: "shortcut-paste",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Delete,
            label_key: "action-delete",
            shortcut_key: "shortcut-delete",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Deselect,
            label_key: "action-deselect",
            shortcut_key: "shortcut-escape",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::SelectAll,
            label_key: "action-select-all",
            shortcut_key: "shortcut-select-all",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Group,
            label_key: "model-group",
            shortcut_key: "shortcut-group",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Ungroup,
            label_key: "model-ungroup",
            shortcut_key: "shortcut-ungroup",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::MakeComponent,
            label_key: "model-make-component",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::MakeUnique,
            label_key: "model-make-unique",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Hide,
            label_key: "model-hide",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Unhide,
            label_key: "model-unhide",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::ViewIso,
            label_key: "view-iso",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::ViewTop,
            label_key: "view-top",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::ViewFront,
            label_key: "view-front",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::ViewProjection,
            label_key: "view-projection",
            shortcut_key: "shortcut-none",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::ZoomFit,
            label_key: "view-zoom-fit",
            shortcut_key: "shortcut-zoom-fit",
            tool: None,
            implemented: true,
        },
        CommandSpec {
            id: AppCommand::Shortcuts,
            label_key: "help-shortcuts",
            shortcut_key: "shortcut-shortcuts",
            tool: None,
            implemented: true,
        },
    ];

    fn spec(id: AppCommand) -> &'static CommandSpec {
        Self::COMMANDS
            .iter()
            .find(|command| command.id == id)
            .expect("every application command is registered")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EditContext {
    Group(GroupId),
    Definition {
        definition_id: DefinitionId,
        instance_path: InstancePath,
    },
}

struct InteractionProjectionCache {
    document_id: DocumentId,
    revision_id: u64,
    edit_context: Vec<EditContext>,
    exact_result_count: usize,
    canonical: ketchup_interaction::projection::InteractionProjection,
    exact: ExactInteractionProjection,
    mesh: MeshInteractionProjection,
    boxes: ketchup_interaction::InteractionScene,
    proxies: ketchup_interaction::InteractionScene,
}

#[derive(Default)]
struct SelectionState {
    occurrences: BTreeSet<InstancePath>,
    primary: Option<SelectionId>,
    selected_group: Option<GroupId>,
    edit_context: Vec<EditContext>,
}

impl SelectionState {
    fn clear(&mut self) {
        self.occurrences.clear();
        self.primary = None;
        self.selected_group = None;
    }

    fn contains(&self, instance_path: &InstancePath) -> bool {
        self.occurrences.contains(instance_path)
    }

    fn select_exact(&mut self, selection: SelectionId, additive: bool) {
        let instance_path = selection.instance_path.clone();
        if additive && self.occurrences.contains(&instance_path) {
            self.occurrences.remove(&instance_path);
            if self
                .primary
                .as_ref()
                .is_some_and(|primary| primary.instance_path == selection.instance_path)
            {
                self.primary = None;
            }
            return;
        }
        if !additive {
            self.occurrences.clear();
        }
        self.occurrences.insert(instance_path);
        self.primary = Some(selection);
        self.selected_group = None;
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

type ExactSource = (DocumentId, u64, String);
type ExactEvaluationResult = Result<Vec<Arc<ExactBodyPackage>>, String>;

enum ExactEvaluationRequest {
    Rectangle(ExactFeatureChainRequest),
    Revolve(ExactRevolveRequest),
}

struct ExactEvaluationTask {
    source: ExactSource,
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<ExactEvaluationResult>,
}

type BeamM5EvaluationResult = Result<Vec<BeamExactPiecePackage>, String>;

struct BeamM5EvaluationTask {
    source: ExactSource,
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<BeamM5EvaluationResult>,
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
            Self::CodexOauth => "gpt-5.5",
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
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AssistantConversation {
    document_id: u64,
    messages: Vec<AssistantChatMessage>,
}

fn assistant_conversation_digest(messages: &[AssistantChatMessage]) -> String {
    let bytes = serde_json::to_vec(messages).expect("assistant messages are serializable");
    ketchup_core::graph::sha256_hex(&bytes)
}

pub trait AssistantTransport: Send + Sync {
    fn chat(
        &self,
        handshake: AssistantHandshake,
        request_id: &str,
        message: &str,
        context: &serde_json::Value,
        cancellation: AssistantCancellation,
    ) -> Result<AssistantChatResult, String>;
}

struct ProcessAssistantTransport;

impl AssistantTransport for ProcessAssistantTransport {
    fn chat(
        &self,
        handshake: AssistantHandshake,
        request_id: &str,
        message: &str,
        context: &serde_json::Value,
        cancellation: AssistantCancellation,
    ) -> Result<AssistantChatResult, String> {
        let (program, arguments) = assistant_sidecar_command(handshake.distribution)?;
        let mut client = AssistantProcessClient::spawn_with_cancellation(
            program,
            &arguments,
            handshake,
            ASSISTANT_TIMEOUT,
            cancellation,
        )
        .map_err(|error| error.to_string())?;
        let answer = client
            .chat(request_id, message, context)
            .map_err(|error| error.to_string());
        let _ = client.shutdown();
        answer
    }
}

struct AssistantChatTask {
    receiver: Receiver<Result<AssistantChatResult, String>>,
    cancellation: AssistantCancellation,
    document_id: DocumentId,
    revision_id: u64,
    canonical_digest: String,
    source: String,
}

struct AssistantPendingExecution {
    result: AssistantChatResult,
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
    BottleControlDimension,
    BottleEdgeFinishKind,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantVerification {
    pub revision_id: u64,
    pub command_digest: String,
    pub result_digest: String,
    pub canonical_digest: String,
    pub verified_write_count: usize,
}

#[derive(Clone, Debug)]
struct PendingStlImport {
    path: PathBuf,
    unit: ImportLengthUnit,
    document_id: DocumentId,
    revision_id: u64,
    canonical_digest: String,
    source_sha256: [u8; 32],
    source_byte_len: u64,
}

pub struct KetchupApp {
    document: DocumentStore,
    container_data: ketchup_core::persistence::ContainerData,
    review_candidate: Option<ketchup_core::persistence::LoadOutcome>,
    review_candidate_source_path: Option<PathBuf>,
    document_path: Option<PathBuf>,
    saved_digest: String,
    confirmation_surface: TrustedConfirmationSurface,
    side_effect_receipts: Vec<SideEffectAuthorizationReceipt>,
    catalog: LocaleCatalog,
    push_pull_distance_input: String,
    preview: Option<CommandBatch>,
    preview_box: Option<EphemeralBoxPreview>,
    preview_definition_id: Option<DefinitionId>,
    smart_push_pull_proposal: Option<SmartPushPullProposal>,
    smart_push_pull_planning: Option<SmartPushPullPlanning>,
    smart_push_pull_chooser: Option<SmartPushPullChooser>,
    occurrence_operation_preview: Option<OccurrenceOperationPreview>,
    solid_tool_target: Option<SelectionId>,
    revolve_tool: Option<RevolveToolState>,
    revolve_preview: Option<RevolvePreview>,
    planar_offset_preview: Option<PlanarOffsetPreview>,
    sweep_preview: Option<SweepPreview>,
    loft_input_sections: Option<(DefinitionId, Vec<LoftSection>)>,
    loft_preview: Option<LoftPreview>,
    general_finish_preview: Option<GeneralFinishPreview>,
    pocket_preview: Option<PocketPreview>,
    pocket_editor_feature: Option<FeatureId>,
    pocket_depth_input: String,
    parameter_editor_node: Option<NodeId>,
    parameter_expression_input: String,
    parameter_canonical_source: String,
    parameter_provenance: Option<(DocumentId, u64, String)>,
    parameter_last_recomputed_nodes: BTreeSet<NodeId>,
    status_key: &'static str,
    theme: ThemeKind,
    projection_mode: ProjectionMode,
    camera_distance_mm: f64,
    yaw: f32,
    pitch: f32,
    camera_target_z: f64,
    zoom: f32,
    pan: Vec2,
    selection: SelectionState,
    hovered: Option<SelectionId>,
    hover_pick: Option<PickResult>,
    hover_snap: Option<SnapResult>,
    hover_overlap_index: usize,
    snap_tracker: SnapTracker,
    interaction_projection_cache: RefCell<Option<InteractionProjectionCache>>,
    active_tool: ActiveTool,
    digest: String,
    assistant_provider: AssistantProvider,
    assistant_model: String,
    assistant_workspace_mode: AssistantWorkspaceMode,
    assistant_input: String,
    assistant_messages: Vec<AssistantChatMessage>,
    assistant_transport: Arc<dyn AssistantTransport>,
    assistant_chat_task: Option<AssistantChatTask>,
    assistant_pending_execution: Option<AssistantPendingExecution>,
    assistant_request_sequence: u64,
    saved_assistant_conversation_digest: String,
    assistant_intent_kind: AssistantIntentKind,
    assistant_target_input: String,
    assistant_value_input: String,
    assistant_proposal: Option<Proposal>,
    assistant_verification: Option<AssistantVerification>,
    push_pull_drag: Option<PushPullDrag>,
    last_push_pull: Option<LastPushPull>,
    bottle_direct_drag: Option<BottleDirectDrag>,
    bottle_editor: Option<BottleEditorInputs>,
    move_drag: Option<MoveDrag>,
    move_anchor: Option<MoveDrag>,
    last_move: Option<LastMove>,
    sketch_mode: bool,
    sketch_start: Option<Vec3>,
    sketch_end: Option<Vec3>,
    sketch_cursor: Option<Vec3>,
    value_input: String,
    focus_value_box: bool,
    occurrence_clipboard: Vec<OccurrenceId>,
    measure_start: Option<Vec3>,
    measure_cursor: Option<Vec3>,
    measure_end: Option<Vec3>,
    shortcuts_open: bool,
    pending_stl_import: Option<PendingStlImport>,
    viewport_rect: Option<Rect>,
    dialogs: Box<dyn FileDialogs>,
    exact_worker_path: Option<PathBuf>,
    exact_worker_attempted: bool,
    exact_task: Option<ExactEvaluationTask>,
    exact_results: ExactResultRegistry,
    exact_source: Option<ExactSource>,
    exact_retry_at: Option<Instant>,
    beam_workspace: Option<BeamWorkspace>,
    beam_zone1_gap_input: String,
    beam_m5_task: Option<BeamM5EvaluationTask>,
    beam_m5_products: Option<Arc<BeamM5Products>>,
    beam_exact_results: ExactResultRegistry,
    beam_m5_source: Option<ExactSource>,
    beam_m5_retry_at: Option<Instant>,
    render_cache: DerivedRenderCache,
    render_plan: Option<Arc<InstancedRenderPlan>>,
    wgpu_target_format: Option<eframe::wgpu::TextureFormat>,
}

impl Default for KetchupApp {
    fn default() -> Self {
        Self::new()
    }
}

impl KetchupApp {
    #[must_use]
    pub fn new() -> Self {
        Self::with_catalog(LocaleCatalog::english())
    }

    #[must_use]
    pub fn with_catalog(catalog: LocaleCatalog) -> Self {
        catalog
            .validate_complete_against(&LocaleCatalog::english())
            .expect("the active locale must match the complete English key set");
        let mut confirmation_key = [0; 32];
        getrandom::fill(&mut confirmation_key)
            .expect("the operating system must provide confirmation-key entropy");
        let confirmation_surface = TrustedConfirmationSurface::new(confirmation_key, 1)
            .expect("the built-in non-zero confirmation policy is valid");
        let mut document = DocumentStore::new();
        document
            .configure_human_confirmation_policy(confirmation_surface.verifying_key(), 1)
            .expect("a fresh document accepts the application confirmation policy");
        let box_name = catalog.format(
            "model-default-box",
            &BTreeMap::from([("number", "1".to_owned())]),
        );
        let occurrence_name = catalog.format(
            "model-default-occurrence",
            &BTreeMap::from([("name", box_name.clone())]),
        );
        document
            .apply_batch(&create_box_batch(
                DefinitionId(1),
                [FeatureId(1), FeatureId(2)],
                OccurrenceId(1),
                [
                    &box_name,
                    &catalog.text("model-default-profile"),
                    &catalog.text("model-default-extrusion"),
                    &occurrence_name,
                ],
                Vec3::ZERO,
                Vec3::new(BOX_WIDTH_MM, BOX_DEPTH_MM, 20.0),
            ))
            .expect("the built-in initial document is valid");
        document.discard_history_before_current();
        let saved_digest = document.current().canonical_digest();
        let digest = catalog.text("status-ready");
        Self {
            document,
            container_data: ketchup_core::persistence::ContainerData::default(),
            review_candidate: None,
            review_candidate_source_path: None,
            document_path: None,
            saved_digest,
            confirmation_surface,
            side_effect_receipts: Vec::new(),
            catalog,
            push_pull_distance_input: String::new(),
            preview: None,
            preview_box: None,
            preview_definition_id: None,
            smart_push_pull_proposal: None,
            smart_push_pull_planning: None,
            smart_push_pull_chooser: None,
            occurrence_operation_preview: None,
            solid_tool_target: None,
            revolve_tool: None,
            revolve_preview: None,
            planar_offset_preview: None,
            sweep_preview: None,
            loft_input_sections: None,
            loft_preview: None,
            general_finish_preview: None,
            pocket_preview: None,
            pocket_editor_feature: None,
            pocket_depth_input: String::new(),
            parameter_editor_node: None,
            parameter_expression_input: String::new(),
            parameter_canonical_source: String::new(),
            parameter_provenance: None,
            parameter_last_recomputed_nodes: BTreeSet::new(),
            status_key: "status-ready",
            theme: ThemeKind::default(),
            projection_mode: ProjectionMode::Parallel,
            camera_distance_mm: 420.0 / 2.8,
            yaw: -0.65,
            pitch: -0.5,
            camera_target_z: 10.0,
            zoom: 2.8,
            pan: Vec2::ZERO,
            selection: SelectionState::default(),
            hovered: None,
            hover_pick: None,
            hover_snap: None,
            hover_overlap_index: 0,
            snap_tracker: SnapTracker::default(),
            interaction_projection_cache: RefCell::new(None),
            active_tool: ActiveTool::Select,
            digest,
            assistant_provider: AssistantProvider::AnthropicApi,
            assistant_model: AssistantProvider::AnthropicApi.default_model().to_owned(),
            assistant_workspace_mode: AssistantWorkspaceMode::Dock,
            assistant_input: String::new(),
            assistant_messages: Vec::new(),
            assistant_transport: Arc::new(ProcessAssistantTransport),
            assistant_chat_task: None,
            assistant_pending_execution: None,
            assistant_request_sequence: 0,
            saved_assistant_conversation_digest: assistant_conversation_digest(&[]),
            assistant_intent_kind: AssistantIntentKind::FeatureDimension,
            assistant_target_input: "2".to_owned(),
            assistant_value_input: "35".to_owned(),
            assistant_proposal: None,
            assistant_verification: None,
            push_pull_drag: None,
            last_push_pull: None,
            bottle_direct_drag: None,
            bottle_editor: None,
            move_drag: None,
            move_anchor: None,
            last_move: None,
            sketch_mode: false,
            sketch_start: None,
            sketch_end: None,
            sketch_cursor: None,
            value_input: String::new(),
            focus_value_box: false,
            occurrence_clipboard: Vec::new(),
            measure_start: None,
            measure_cursor: None,
            measure_end: None,
            shortcuts_open: false,
            pending_stl_import: None,
            viewport_rect: None,
            dialogs: Box::new(NativeFileDialogs::default()),
            exact_worker_path: None,
            exact_worker_attempted: false,
            exact_task: None,
            exact_results: ExactResultRegistry::default(),
            exact_source: None,
            exact_retry_at: None,
            beam_workspace: None,
            beam_zone1_gap_input: "415".to_owned(),
            beam_m5_task: None,
            beam_m5_products: None,
            beam_exact_results: ExactResultRegistry::default(),
            beam_m5_source: None,
            beam_m5_retry_at: None,
            render_cache: DerivedRenderCache::default(),
            render_plan: None,
            wgpu_target_format: None,
        }
    }

    #[must_use]
    pub fn from_creation_context(context: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::new();
        app.dialogs = Box::new(NativeFileDialogs::with_parent(
            DialogParentWindow::from_creation_context(context),
        ));
        if let Some(render_state) = context.wgpu_render_state.as_ref() {
            render_state
                .renderer
                .write()
                .callback_resources
                .insert(GpuInstancedRenderer::new(
                    &render_state.device,
                    render_state.target_format,
                ));
            app.wgpu_target_format = Some(render_state.target_format);
        }
        app
    }

    /// Answer file dialogs from `dialogs` instead of the operating system.
    #[must_use]
    pub fn with_dialogs(mut self, dialogs: Box<dyn FileDialogs>) -> Self {
        self.dialogs = dialogs;
        self
    }

    #[must_use]
    pub fn with_assistant_transport(mut self, transport: Arc<dyn AssistantTransport>) -> Self {
        self.assistant_transport = transport;
        self
    }

    #[must_use]
    pub fn title() -> String {
        LocaleCatalog::english().text("app-title")
    }

    fn document_title(&self) -> String {
        let name = self
            .document_path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(
                || self.catalog.text("document-untitled"),
                |name| name.to_string_lossy().into_owned(),
            );
        self.catalog.format(
            if self.is_dirty() {
                "document-title-dirty"
            } else {
                "document-title-clean"
            },
            &BTreeMap::from([("name", name)]),
        )
    }

    fn cancel_pending_assistant_work(&mut self) {
        self.assistant_proposal = None;
        self.assistant_pending_execution = None;
        if let Some(task) = self.assistant_chat_task.take() {
            task.cancellation.cancel();
        }
    }

    fn reset_document_presentation(&mut self) {
        self.preview = None;
        self.preview_box = None;
        self.preview_definition_id = None;
        self.smart_push_pull_proposal = None;
        self.smart_push_pull_planning = None;
        self.smart_push_pull_chooser = None;
        self.occurrence_operation_preview = None;
        self.solid_tool_target = None;
        self.revolve_tool = None;
        self.revolve_preview = None;
        self.planar_offset_preview = None;
        self.sweep_preview = None;
        self.loft_input_sections = None;
        self.loft_preview = None;
        self.general_finish_preview = None;
        self.pocket_preview = None;
        self.pocket_editor_feature = None;
        self.pocket_depth_input.clear();
        self.parameter_editor_node = None;
        self.parameter_expression_input.clear();
        self.parameter_canonical_source.clear();
        self.parameter_provenance = None;
        self.parameter_last_recomputed_nodes.clear();
        self.selection = SelectionState::default();
        self.hovered = None;
        self.hover_pick = None;
        self.hover_snap = None;
        self.hover_overlap_index = 0;
        self.snap_tracker.clear();
        self.interaction_projection_cache.get_mut().take();
        self.active_tool = ActiveTool::Select;
        self.cancel_pending_assistant_work();
        self.assistant_verification = None;
        self.side_effect_receipts.clear();
        self.push_pull_drag = None;
        self.last_push_pull = None;
        self.bottle_direct_drag = None;
        self.bottle_editor = None;
        self.move_drag = None;
        self.move_anchor = None;
        self.last_move = None;
        self.occurrence_clipboard.clear();
        self.sketch_mode = false;
        self.sketch_start = None;
        self.sketch_end = None;
        self.sketch_cursor = None;
        self.value_input.clear();
        self.focus_value_box = false;
        if let Some(task) = self.exact_task.take() {
            task.cancelled.store(true, Ordering::Release);
        }
        self.exact_results.clear();
        self.render_plan = None;
        self.exact_source = None;
        self.exact_retry_at = None;
        self.status_key = "status-ready";
    }

    fn new_document(&mut self) {
        self.cancel_pending_assistant_work();
        let dialogs = std::mem::replace(&mut self.dialogs, Box::new(NativeFileDialogs::default()));
        let assistant_transport = Arc::clone(&self.assistant_transport);
        let assistant_request_sequence = self.assistant_request_sequence;
        *self = Self::new()
            .with_dialogs(dialogs)
            .with_assistant_transport(assistant_transport);
        self.assistant_request_sequence = assistant_request_sequence;
        self.digest = self.catalog.text("digest-new-document");
    }

    fn open_document_from(&mut self, path: &Path) -> bool {
        self.cancel_pending_assistant_work();
        match ketchup_core::persistence::load_file(path) {
            Ok(outcome) => {
                if !outcome.is_editable() {
                    self.review_candidate = Some(outcome);
                    self.review_candidate_source_path = Some(path.to_owned());
                    self.digest = self.catalog.format(
                        "error-open-document",
                        &BTreeMap::from([
                            ("path", path.display().to_string()),
                            (
                                "reason",
                                "document requires read-only migration review".to_owned(),
                            ),
                        ]),
                    );
                    return false;
                }
                let Ok((mut document, container_data)) = outcome.into_editable_with_container()
                else {
                    unreachable!("editable load outcome must contain an editable document");
                };
                document.discard_history_before_current();
                document
                    .configure_human_confirmation_policy(
                        self.confirmation_surface.verifying_key(),
                        1,
                    )
                    .expect("an opened document accepts the application confirmation policy");
                self.document = document;
                self.container_data = container_data;
                self.review_candidate = None;
                self.review_candidate_source_path = None;
                self.document_path = Some(path.to_owned());
                self.saved_digest = self.document.current().canonical_digest();
                self.reset_document_presentation();
                self.load_assistant_conversation();
                self.digest = self.catalog.format(
                    "digest-opened-document",
                    &BTreeMap::from([("path", path.display().to_string())]),
                );
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-open-document",
                    &BTreeMap::from([
                        ("path", path.display().to_string()),
                        ("reason", error.to_string()),
                    ]),
                );
                false
            }
        }
    }

    fn authorize_path_side_effect(
        &mut self,
        class: HighRiskClass,
        operation: &str,
        title: &str,
        risk: &str,
        path: &Path,
        payload: &[u8],
    ) -> Result<(), String> {
        let scope = HighRiskScope::new(class, None, None, Some(path.display().to_string()))
            .map_err(|error| error.to_string())?;
        let proposal = self
            .document
            .prepare_high_risk_side_effect(
                operation,
                ProposalPrincipal::LocalAssistant,
                scope,
                payload,
            )
            .map_err(|error| error.to_string())?;
        let description = format!(
            "Risk: {risk}\nPath: {}\nDocument revision: {}\nPayload SHA-256: {}\nOperation digest: {}",
            path.display(),
            proposal.provenance_revision(),
            proposal.payload_digest(),
            proposal.operation_digest(),
        );
        let Some(approving_human) = self.dialogs.confirm_high_risk(HighRiskConfirmationRequest {
            title,
            description: &description,
        }) else {
            return Err(format!("authenticated human declined {risk}"));
        };
        let now_ms = current_unix_time_ms()?;
        let approval = self
            .confirmation_surface
            .issue_side_effect(
                &proposal,
                AuthenticatedApprover::Human(approving_human),
                now_ms,
                now_ms
                    .checked_add(MAX_HUMAN_CONFIRMATION_LIFETIME_MS)
                    .ok_or_else(|| "confirmation expiry exceeds the supported clock".to_owned())?,
            )
            .map_err(|error| error.to_string())?;
        let receipt = self
            .document
            .authorize_high_risk_side_effect(&proposal, &approval, now_ms)
            .map_err(|error| error.to_string())?;
        self.side_effect_receipts.push(receipt);
        Ok(())
    }

    fn authorize_beam_path_side_effect(
        &mut self,
        class: HighRiskClass,
        operation: &str,
        title: &str,
        risk: &str,
        path: &Path,
        payload: &[u8],
    ) -> Result<(), String> {
        self.side_effect_receipts.clear();
        let scope = HighRiskScope::new(class, None, None, Some(path.display().to_string()))
            .map_err(|error| error.to_string())?;
        let proposal = self
            .beam_workspace
            .as_ref()
            .ok_or_else(|| "Beam workspace is unavailable".to_owned())?
            .prepare_high_risk_side_effect(
                operation,
                ProposalPrincipal::LocalAssistant,
                scope,
                payload,
            )
            .map_err(|error| error.to_string())?;
        let description = format!(
            "Risk: {risk}\nPath: {}\nDocument revision: {}\nPayload SHA-256: {}\nOperation digest: {}",
            path.display(),
            proposal.provenance_revision(),
            proposal.payload_digest(),
            proposal.operation_digest(),
        );
        let Some(approving_human) = self.dialogs.confirm_high_risk(HighRiskConfirmationRequest {
            title,
            description: &description,
        }) else {
            return Err(format!("authenticated human declined {risk}"));
        };
        let now_ms = current_unix_time_ms()?;
        let approval = self
            .confirmation_surface
            .issue_side_effect(
                &proposal,
                AuthenticatedApprover::Human(approving_human),
                now_ms,
                now_ms
                    .checked_add(MAX_HUMAN_CONFIRMATION_LIFETIME_MS)
                    .ok_or_else(|| "confirmation expiry exceeds the supported clock".to_owned())?,
            )
            .map_err(|error| error.to_string())?;
        let receipt = self
            .beam_workspace
            .as_mut()
            .ok_or_else(|| "Beam workspace is unavailable".to_owned())?
            .authorize_high_risk_side_effect(&proposal, &approval, now_ms)
            .map_err(|error| error.to_string())?;
        self.side_effect_receipts.push(receipt);
        Ok(())
    }

    fn authorize_overwrite(&mut self, path: &Path, payload: &[u8]) -> Result<(), String> {
        self.side_effect_receipts.clear();
        self.authorize_path_side_effect(
            HighRiskClass::Overwrite,
            "overwrite-native-document",
            "Confirm high-risk overwrite",
            "overwrite existing file",
            path,
            payload,
        )
    }

    fn save_document_to(&mut self, path: &Path) -> bool {
        if path.is_dir() {
            self.digest = self.catalog.format(
                "error-save-document",
                &BTreeMap::from([
                    ("path", path.display().to_string()),
                    ("reason", "the target path is a directory".to_owned()),
                ]),
            );
            return false;
        }
        self.store_assistant_conversation();
        let snapshot = self.document.current();
        let prepared = ketchup_core::persistence::save_container(&snapshot, &self.container_data);
        if path.exists()
            && let Err(error) = prepared
                .as_ref()
                .map_err(ToString::to_string)
                .and_then(|bytes| self.authorize_overwrite(path, bytes))
        {
            self.digest = self.catalog.format(
                "error-save-document",
                &BTreeMap::from([("path", path.display().to_string()), ("reason", error)]),
            );
            return false;
        }
        let result = prepared.map_err(|error| error.to_string()).and_then(|_| {
            ketchup_core::persistence::save_atomic_with_container(
                path,
                &snapshot,
                &self.container_data,
            )
            .map_err(|error| error.to_string())
        });
        match result {
            Ok(()) => {
                self.document_path = Some(path.to_owned());
                self.saved_digest = snapshot.canonical_digest();
                self.saved_assistant_conversation_digest =
                    assistant_conversation_digest(&self.assistant_messages);
                self.digest = self.catalog.format(
                    "digest-saved-document",
                    &BTreeMap::from([("path", path.display().to_string())]),
                );
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-save-document",
                    &BTreeMap::from([
                        ("path", path.display().to_string()),
                        ("reason", error.to_string()),
                    ]),
                );
                false
            }
        }
    }

    fn confirm_discard_if_dirty(&mut self) -> bool {
        if !self.is_dirty() {
            return true;
        }
        let title = self.catalog.text("dialog-unsaved-title");
        let description = self.catalog.text("dialog-unsaved-description");
        self.dialogs.confirm_discard(DiscardRequest {
            title: &title,
            description: &description,
        })
    }

    fn choose_open_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-ketchup");
        self.dialogs.pick_open_path(&filter_label)
    }

    fn choose_save_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-ketchup");
        let title = self.document_title();
        let suggested_name = title.trim_end_matches(" *");
        self.dialogs.pick_save_path(SaveRequest {
            filter_label: &filter_label,
            suggested_name,
        })
    }

    fn choose_export_path(&mut self, extension: &str) -> Option<PathBuf> {
        let (filter_key, suffix) = match extension {
            "step" => ("file-filter-step", "step"),
            "stl" => ("file-filter-stl", "stl"),
            _ => unreachable!("the File menu exposes only STEP and STL export"),
        };
        let filter_label = self.catalog.text(filter_key);
        let stem = self
            .document_path
            .as_deref()
            .and_then(Path::file_stem)
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.trim().is_empty())
            .unwrap_or("Untitled");
        let suggested_name = format!("{stem}.{suffix}");
        self.dialogs.pick_export_path(ExportRequest {
            filter_label: &filter_label,
            extension,
            suggested_name: &suggested_name,
        })
    }

    fn choose_stl_import_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-stl");
        self.dialogs.pick_import_path(ImportDialogRequest {
            format: ImportFormat::Stl,
            filter_label: &filter_label,
            extensions: &["stl"],
        })
    }

    fn read_stl_source(path: &Path) -> Result<Vec<u8>, String> {
        if std::fs::metadata(path)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_STL_SOURCE_BYTES
        {
            return Err("STL source exceeds the bounded 200,000-facet text envelope".to_owned());
        }
        let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        let mut source = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take(MAX_STL_SOURCE_BYTES + 1)
            .read_to_end(&mut source)
            .map_err(|error| error.to_string())?;
        if source.len() as u64 > MAX_STL_SOURCE_BYTES {
            return Err("STL source exceeds the bounded 200,000-facet text envelope".to_owned());
        }
        Ok(source)
    }

    fn import_stl_from(&mut self, pending: &PendingStlImport) -> bool {
        let path = &pending.path;
        let result = (|| {
            let snapshot = self.document.current();
            if snapshot.document_id() != pending.document_id
                || snapshot.revision_id() != pending.revision_id
                || snapshot.canonical_digest() != pending.canonical_digest
            {
                return Err("STL import review is stale for the active document".to_owned());
            }
            let source = Self::read_stl_source(path)?;
            if source.len() as u64 != pending.source_byte_len
                || sha256_bytes(&source) != pending.source_sha256
            {
                return Err("STL source changed after it was selected for review".to_owned());
            }
            let source_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| "STL source name is not valid UTF-8".to_owned())?;
            let units = ImportUnitDecision::new(pending.unit, ImportUnitAuthority::UserDeclared);
            let batch = std::panic::catch_unwind(|| {
                plan_stl_import(&snapshot, &source, source_name, units)
            })
            .map_err(|_| "bounded STL parser stopped without publishing geometry".to_owned())?
            .map_err(|error| error.to_string())?;
            let proposal = self
                .document
                .prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
                .map_err(|error| error.to_string())?;
            self.document
                .commit_verified_proposal(&proposal)
                .map_err(|error| error.to_string())?;
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => {
                self.digest = self.catalog.format(
                    "digest-imported-stl",
                    &BTreeMap::from([("path", path.display().to_string())]),
                );
                true
            }
            Err(reason) => {
                self.digest = self.catalog.format(
                    "error-import-stl",
                    &BTreeMap::from([("path", path.display().to_string()), ("reason", reason)]),
                );
                false
            }
        }
    }

    fn current_visible_exact_model(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Vec<(ExactBodyPackage, Transform)>, String> {
        let occurrences = snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .filter(|occurrence| {
                snapshot
                    .definition(occurrence.definition_id)
                    .is_some_and(|definition| {
                        definition.feature_ids().iter().any(|feature_id| {
                            snapshot
                                .feature(*feature_id)
                                .is_some_and(|feature| feature.kind().produces_body())
                        })
                    })
            })
            .collect::<Vec<_>>();
        if occurrences.is_empty() {
            return Err("the visible model is empty".to_owned());
        }
        occurrences
            .into_iter()
            .map(|occurrence| {
                self.exact_results
                    .get(&occurrence.definition_id)
                    .filter(|package| package.is_current(snapshot))
                    .map(|package| ((**package).clone(), occurrence.transform))
                    .ok_or_else(|| {
                        format!(
                            "visible occurrence {:?} has no current accepted exact result",
                            occurrence.instance_path
                        )
                    })
            })
            .collect()
    }

    fn export_current_model_stl_to(&mut self, path: &Path) -> bool {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = self
            .current_visible_exact_model(&snapshot)
            .and_then(|model| {
                let bodies = model
                    .iter()
                    .map(|(package, transform)| (package, *transform))
                    .collect::<Vec<_>>();
                exact_model_stl_export(&snapshot, &bodies).map_err(|error| error.to_string())
            })
            .and_then(|bundle| {
                let report_path = path.with_extension("stl.loss.txt");
                let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
                let evidence = exact_stl_export_evidence(path, &bundle);
                let title = self.catalog.text("dialog-export-lossy-title");
                let risk = self.catalog.text("dialog-export-lossy-risk");
                self.authorize_path_side_effect(
                    HighRiskClass::LossyConversion,
                    "export-current-model-stl-with-loss-report",
                    &title,
                    &risk,
                    path,
                    &evidence,
                )?;
                if precondition.primary_sha256.is_some() {
                    let title = self.catalog.text("dialog-export-overwrite-title");
                    let risk = self.catalog.text("dialog-export-overwrite-risk");
                    self.authorize_path_side_effect(
                        HighRiskClass::Overwrite,
                        "overwrite-current-model-stl-export",
                        &title,
                        &risk,
                        path,
                        &evidence,
                    )?;
                }
                if precondition.report_sha256.is_some() {
                    let title = self.catalog.text("dialog-export-overwrite-title");
                    let risk = self.catalog.text("dialog-export-overwrite-risk");
                    self.authorize_path_side_effect(
                        HighRiskClass::Overwrite,
                        "overwrite-current-model-stl-loss-report",
                        &title,
                        &risk,
                        &report_path,
                        &evidence,
                    )?;
                }
                write_export_bundle(
                    path,
                    bundle.mesh_stl.as_bytes(),
                    &report_path,
                    bundle.loss_report.as_bytes(),
                    &precondition,
                )
            });
        match result {
            Ok(()) => {
                self.digest = self.catalog.format(
                    "digest-exported-stl",
                    &BTreeMap::from([("path", path.display().to_string())]),
                );
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-export-stl",
                    &BTreeMap::from([("path", path.display().to_string()), ("reason", error)]),
                );
                false
            }
        }
    }

    fn export_current_model_step_to(&mut self, path: &Path) -> bool {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = (|| {
            let model = self.current_visible_exact_model(&snapshot)?;
            let executable = self
                .exact_worker_path
                .clone()
                .or_else(|| {
                    exact_worker_candidates()
                        .into_iter()
                        .find(|candidate| candidate.is_file())
                })
                .ok_or_else(|| "exact worker is unavailable".to_owned())?;
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            let prepared_directory = tempfile::Builder::new()
                .prefix(".ketchup-prepared-export-")
                .tempdir_in(parent)
                .map_err(|error| error.to_string())?;
            let prepared_step = prepared_directory.path().join("model.step");
            let mut worker =
                ExactWorkerSupervisor::spawn(executable).map_err(|error| error.to_string())?;
            worker
                .export_current_model_step(&snapshot, &model, &prepared_step)
                .map_err(|error| error.to_string())?;
            let step = std::fs::read(&prepared_step).map_err(|error| error.to_string())?;
            let report = exact_model_step_loss_report(&snapshot, &model);
            let report_path = path.with_extension("step.loss.txt");
            let evidence = export_bundle_evidence(
                b"ketchup.current-model-step-export.v1",
                path,
                &step,
                &report_path,
                report.as_bytes(),
            );
            let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
            let title = self.catalog.text("dialog-export-lossy-title");
            let risk = self.catalog.text("dialog-export-lossy-risk");
            self.authorize_path_side_effect(
                HighRiskClass::LossyConversion,
                "export-current-model-step-with-loss-report",
                &title,
                &risk,
                path,
                &evidence,
            )?;
            if precondition.primary_sha256.is_some() {
                let title = self.catalog.text("dialog-export-overwrite-title");
                let risk = self.catalog.text("dialog-export-overwrite-risk");
                self.authorize_path_side_effect(
                    HighRiskClass::Overwrite,
                    "overwrite-current-model-step-export",
                    &title,
                    &risk,
                    path,
                    &evidence,
                )?;
            }
            if precondition.report_sha256.is_some() {
                let title = self.catalog.text("dialog-export-overwrite-title");
                let risk = self.catalog.text("dialog-export-overwrite-risk");
                self.authorize_path_side_effect(
                    HighRiskClass::Overwrite,
                    "overwrite-current-model-step-loss-report",
                    &title,
                    &risk,
                    &report_path,
                    &evidence,
                )?;
            }
            write_export_bundle(path, &step, &report_path, report.as_bytes(), &precondition)
        })();
        match result {
            Ok(()) => {
                self.digest = self.catalog.format(
                    "digest-exported-step",
                    &BTreeMap::from([("path", path.display().to_string())]),
                );
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-export-step",
                    &BTreeMap::from([("path", path.display().to_string()), ("reason", error)]),
                );
                false
            }
        }
    }

    fn dispatch_file_command(&mut self, id: AppCommand) {
        match id {
            AppCommand::New if self.confirm_discard_if_dirty() => self.new_document(),
            AppCommand::Open if self.confirm_discard_if_dirty() => {
                if let Some(path) = self.choose_open_path() {
                    self.open_document_from(&path);
                }
            }
            AppCommand::Save => {
                if let Some(path) = self
                    .document_path
                    .clone()
                    .or_else(|| self.choose_save_path())
                {
                    self.save_document_to(&path);
                }
            }
            AppCommand::SaveAs => {
                if let Some(path) = self.choose_save_path() {
                    self.save_document_to(&path);
                }
            }
            AppCommand::ImportMeshStl => {
                if let Some(path) = self.choose_stl_import_path() {
                    match Self::read_stl_source(&path) {
                        Ok(source) => {
                            let snapshot = self.document.current();
                            self.pending_stl_import = Some(PendingStlImport {
                                path,
                                unit: ImportLengthUnit::Millimetre,
                                document_id: snapshot.document_id(),
                                revision_id: snapshot.revision_id(),
                                canonical_digest: snapshot.canonical_digest(),
                                source_sha256: sha256_bytes(&source),
                                source_byte_len: source.len() as u64,
                            });
                        }
                        Err(reason) => {
                            self.digest = self.catalog.format(
                                "error-import-stl",
                                &BTreeMap::from([
                                    ("path", path.display().to_string()),
                                    ("reason", reason),
                                ]),
                            );
                        }
                    }
                }
            }
            AppCommand::ExportExactStep => {
                if let Some(path) = self.choose_export_path("step") {
                    self.export_current_model_step_to(&path);
                }
            }
            AppCommand::ExportMeshStl => {
                if let Some(path) = self.choose_export_path("stl") {
                    self.export_current_model_stl_to(&path);
                }
            }
            AppCommand::New | AppCommand::Open => {}
            _ => unreachable!("only file commands are routed here"),
        }
    }

    #[must_use]
    pub fn native_options() -> eframe::NativeOptions {
        Self::native_options_for_adapter(None, Arc::new(Mutex::new(None)))
    }

    #[must_use]
    pub fn native_options_for_adapter(
        requirement: Option<AdapterRequirement>,
        selected_info: SelectedAdapterInfo,
    ) -> eframe::NativeOptions {
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::default();
        setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
        setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
            let mut matching = adapters.iter().filter(|adapter| {
                let info = adapter.get_info();
                info.backend == eframe::wgpu::Backend::Dx12
                    && !matches!(
                        info.device_type,
                        eframe::wgpu::DeviceType::Cpu | eframe::wgpu::DeviceType::VirtualGpu
                    )
                    && requirement.as_ref().is_none_or(|required| {
                        info.name == required.name && info.device_type == required.device_type
                    })
                    && surface.is_none_or(|surface| adapter.is_surface_supported(surface))
            });
            let selected = matching.next().ok_or_else(|| {
                "no Direct3D 12 physical adapter matched the frozen requirement".to_owned()
            })?;
            if matching.next().is_some() {
                return Err(
                    "multiple Direct3D 12 physical adapters matched the frozen requirement"
                        .to_owned(),
                );
            }
            let info = selected.get_info();
            *selected_info
                .lock()
                .map_err(|_| "selected-adapter evidence lock is unavailable".to_owned())? =
                Some(info);
            Ok(selected.clone())
        }));

        eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
                present_mode: eframe::wgpu::PresentMode::AutoNoVsync,
                wgpu_setup: setup.into(),
                ..Default::default()
            },
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1_100.0, 720.0])
                .with_min_inner_size([1_100.0, 600.0]),
            ..Default::default()
        }
    }

    pub fn load_beam_m4ae(&mut self) -> bool {
        match BeamWorkspace::load() {
            Ok(mut workspace) => {
                workspace
                    .configure_human_confirmation_policy(
                        self.confirmation_surface.verifying_key(),
                        1,
                    )
                    .expect("the Beam workspace accepts the application confirmation policy");
                if let Some(task) = self.beam_m5_task.take() {
                    task.cancelled.store(true, Ordering::Release);
                }
                self.beam_workspace = Some(workspace);
                self.beam_zone1_gap_input = "415".to_owned();
                self.beam_m5_products = None;
                self.beam_exact_results.clear();
                self.beam_m5_source = None;
                self.beam_m5_retry_at = None;
                true
            }
            Err(error) => {
                self.digest = error.to_string();
                false
            }
        }
    }

    pub fn set_beam_zone1_gap_mm(&mut self, value: f64) -> bool {
        let Some(workspace) = self.beam_workspace.as_mut() else {
            return false;
        };
        match workspace.set_zone1_gap_mm(value) {
            Ok(_) => {
                if let Some(task) = self.beam_m5_task.take() {
                    task.cancelled.store(true, Ordering::Release);
                }
                self.beam_zone1_gap_input = format_height(value);
                self.beam_m5_products = None;
                self.beam_exact_results.clear();
                self.beam_m5_source = None;
                self.beam_m5_retry_at = None;
                true
            }
            Err(error) => {
                self.digest = error.to_string();
                false
            }
        }
    }

    #[must_use]
    pub fn beam_slice(&self) -> Option<&BeamSlice> {
        self.beam_workspace.as_ref().map(BeamWorkspace::slice)
    }

    #[must_use]
    pub fn beam_groove_positions(&self) -> Option<&[GroovePosition]> {
        self.beam_slice().map(|slice| slice.positions.as_slice())
    }

    #[must_use]
    pub fn beam_bom(&self) -> Option<&GroupedBom> {
        self.beam_slice().map(|slice| &slice.bom)
    }

    #[must_use]
    pub fn beam_full_bom(&self) -> Option<&FullBomProjection> {
        self.beam_slice().map(|slice| &slice.full_bom)
    }

    #[must_use]
    pub fn beam_dimension_sheet(&self) -> Option<&PieceDimensionSheet> {
        self.beam_slice().map(|slice| &slice.dimension_sheet)
    }

    #[must_use]
    pub fn beam_validation_report(&self) -> Option<&ValidationReport> {
        self.beam_slice().map(|slice| &slice.validation_report)
    }

    #[must_use]
    pub fn beam_validation_is_green(&self) -> bool {
        self.beam_slice()
            .is_some_and(|slice| slice.validation == BeamValidationVerdict::Green)
    }

    #[must_use]
    pub fn beam_last_change(&self) -> Option<&BeamChangeSummary> {
        self.beam_workspace
            .as_ref()
            .and_then(BeamWorkspace::last_change)
    }

    #[must_use]
    pub fn beam_m5_products(&self) -> Option<&BeamM5Products> {
        self.beam_m5_products.as_deref()
    }

    #[must_use]
    pub fn beam_m5_stable_reference_count(&self) -> usize {
        self.beam_m5_products()
            .map_or(0, BeamM5Products::stable_reference_count)
    }

    #[must_use]
    pub fn beam_exact_body_count(&self) -> usize {
        let Some(snapshot) = self.beam_workspace.as_ref().map(BeamWorkspace::snapshot) else {
            return 0;
        };
        self.beam_exact_results
            .beam_values()
            .filter(|package| package.is_current(&snapshot))
            .count()
    }

    pub fn export_beam_piece_mesh_to(&mut self, piece: &DerivedIdentity, path: &Path) -> bool {
        let result = self
            .beam_workspace
            .as_ref()
            .map(BeamWorkspace::snapshot)
            .ok_or_else(|| "Beam workspace is unavailable".to_owned())
            .and_then(|snapshot| {
                self.beam_exact_results
                    .get_beam(piece)
                    .filter(|package| package.is_current(&snapshot))
                    .ok_or_else(|| "current accepted Beam exact body is unavailable".to_owned())
                    .map(|package| package.mesh_export(Transform::identity()))
            })
            .and_then(|bundle| {
                self.authorize_beam_path_side_effect(
                    HighRiskClass::LossyConversion,
                    "export-lossy-obj-with-loss-report",
                    "Confirm lossy mesh export",
                    "lossy exact-to-mesh conversion",
                    path,
                    &exact_mesh_export_evidence(&bundle),
                )?;
                write_exact_mesh_export(path, bundle)
            });
        match result {
            Ok(()) => {
                self.digest = format!(
                    "Exported Beam exact body OBJ with explicit loss report to {}",
                    path.display()
                );
                true
            }
            Err(error) => {
                self.digest = format!("Beam exact body export blocked: {error}");
                false
            }
        }
    }

    pub fn export_beam_drawing_to(&mut self, path: &Path) -> bool {
        let result = self
            .beam_m5_products()
            .ok_or_else(|| "M5 exact products are not current".to_owned())
            .and_then(|products| products.drawing_svg().map_err(|error| error.to_string()))
            .and_then(|bytes| std::fs::write(path, bytes).map_err(|error| error.to_string()));
        match result {
            Ok(()) => {
                self.digest = format!("Exported Beam A piece drawing to {}", path.display());
                true
            }
            Err(error) => {
                self.digest = format!("Beam A drawing export blocked: {error}");
                false
            }
        }
    }

    pub fn export_beam_manufacturing_to(&mut self, path: &Path) -> bool {
        let result = self
            .beam_m5_products()
            .ok_or_else(|| "M5 exact products are not current".to_owned())
            .and_then(|products| {
                products
                    .manufacturing_export()
                    .map_err(|error| error.to_string())
            })
            .and_then(|bytes| {
                self.authorize_beam_path_side_effect(
                    HighRiskClass::ReleaseManufacturingExportWithWarnings,
                    "release-manufacturing-export-with-warnings",
                    "Confirm manufacturing export release",
                    "manufacturing export with unresolved warnings",
                    path,
                    &bytes,
                )?;
                std::fs::write(path, bytes).map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => {
                self.digest = format!(
                    "Exported Beam A manufacturing operations to {}",
                    path.display()
                );
                true
            }
            Err(error) => {
                self.digest = format!("Beam A manufacturing export blocked: {error}");
                false
            }
        }
    }

    fn refresh_beam_m5_products(&mut self, context: &egui::Context) {
        let Some(workspace) = self.beam_workspace.as_ref() else {
            return;
        };
        let snapshot = workspace.snapshot();
        let source = (
            snapshot.document_id(),
            snapshot.revision_id(),
            snapshot.canonical_digest(),
        );
        if self
            .beam_m5_source
            .as_ref()
            .is_some_and(|known| known != &source)
        {
            self.beam_m5_products = None;
            self.beam_exact_results.clear();
            self.beam_m5_source = None;
        }
        if self
            .beam_m5_task
            .as_ref()
            .is_some_and(|task| task.source != source)
            && let Some(task) = self.beam_m5_task.take()
        {
            task.cancelled.store(true, Ordering::Release);
        }
        if let Some(task) = self.beam_m5_task.as_ref() {
            match task.receiver.try_recv() {
                Ok(result) => {
                    let task = self
                        .beam_m5_task
                        .take()
                        .expect("the completed M5 task exists");
                    if task.source == source && !task.cancelled.load(Ordering::Acquire) {
                        match result
                            .and_then(|packages| {
                                self.beam_workspace
                                    .as_ref()
                                    .expect("the source beam workspace exists")
                                    .accept_m5_packages(packages)
                                    .map_err(|error| error.to_string())
                            })
                            .and_then(|products| {
                                ExactResultRegistry::accept_beam(
                                    &snapshot,
                                    products.packages.values().cloned(),
                                )
                                .map(|results| (products, results))
                                .map_err(|error| error.to_string())
                            }) {
                            Ok((products, results)) => {
                                self.beam_m5_products = Some(Arc::new(products));
                                self.beam_exact_results = results;
                                self.beam_m5_source = Some(source.clone());
                                self.beam_m5_retry_at = None;
                            }
                            Err(error) => {
                                self.digest = format!("M5 exact evaluation failed: {error}");
                                self.beam_m5_retry_at =
                                    Some(Instant::now() + Duration::from_secs(1));
                                context.request_repaint_after(Duration::from_secs(1));
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.beam_m5_task = None;
                    self.digest = "M5 exact evaluation worker disconnected".to_owned();
                    self.beam_m5_retry_at = Some(Instant::now() + Duration::from_secs(1));
                    context.request_repaint_after(Duration::from_secs(1));
                }
            }
        }
        if self.beam_m5_source.as_ref() == Some(&source)
            || self
                .beam_m5_retry_at
                .is_some_and(|retry_at| retry_at > Instant::now())
        {
            return;
        }
        if !self.exact_worker_attempted {
            self.exact_worker_attempted = true;
            self.exact_worker_path = exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file());
        }
        let Some(executable) = self.exact_worker_path.clone() else {
            return;
        };
        let Ok(requests) = self
            .beam_workspace
            .as_ref()
            .expect("the source beam workspace exists")
            .m5_requests()
        else {
            return;
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let repaint = context.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let mut worker =
                    ExactWorkerSupervisor::spawn_with_cancellation(executable, &worker_cancelled)
                        .map_err(|error| error.to_string())?;
                requests
                    .iter()
                    .map(|request| {
                        worker
                            .evaluate_beam_piece_with_cancellation(request, &worker_cancelled)
                            .map_err(|error| error.to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()
            })();
            if !worker_cancelled.load(Ordering::Acquire) && sender.send(result).is_ok() {
                repaint.request_repaint();
            }
        });
        self.beam_m5_task = Some(BeamM5EvaluationTask {
            source,
            cancelled,
            receiver,
        });
    }

    #[must_use]
    pub fn document_revision(&self) -> u64 {
        self.document.current().revision_id()
    }

    #[must_use]
    pub fn document_snapshot(&self) -> Snapshot {
        self.document.current()
    }

    #[must_use]
    pub const fn parameter_last_recomputed_nodes(&self) -> &BTreeSet<NodeId> {
        &self.parameter_last_recomputed_nodes
    }

    /// Canonical identity of the active document: schema, units, IDs,
    /// hierarchy, parameters, transforms, and sharing folded into one value.
    #[must_use]
    pub fn canonical_digest(&self) -> String {
        self.document.current().canonical_digest()
    }

    /// Screen rectangle of the 3D viewport, or `None` before the first frame
    /// has laid the shell out.
    #[must_use]
    pub fn viewport_rect(&self) -> Option<Rect> {
        self.viewport_rect
    }

    #[must_use]
    pub fn viewport_position(&self, point_mm: Vec3) -> Option<Pos2> {
        self.viewport_rect.map(|rect| self.project(point_mm, rect))
    }

    #[must_use]
    pub fn hovered_selection(&self) -> Option<&SelectionId> {
        self.hovered.as_ref()
    }

    #[must_use]
    pub fn hovered_snap_kind(&self) -> Option<SnapKind> {
        self.hover_snap.as_ref().map(|snap| snap.kind)
    }

    #[must_use]
    pub fn hovered_snap_position(&self) -> Option<Vec3> {
        self.hover_snap.as_ref().map(|snap| snap.position_mm)
    }

    #[must_use]
    pub fn hovered_overlap_choice(&self) -> Option<(usize, usize)> {
        self.hover_pick
            .as_ref()
            .map(|pick| (self.hover_overlap_index, pick.overlapping.len()))
    }

    #[must_use]
    pub fn last_side_effect_receipt(&self) -> Option<&SideEffectAuthorizationReceipt> {
        self.side_effect_receipts.last()
    }

    #[must_use]
    pub fn side_effect_receipt_count(&self) -> usize {
        self.side_effect_receipts.len()
    }

    /// Whether the active document carries unsaved changes.
    #[must_use]
    pub fn has_review_candidate(&self) -> bool {
        self.review_candidate.is_some()
    }

    pub fn confirm_review_candidate_migration_to(&mut self, destination: &Path) -> bool {
        let Some(source) = self.review_candidate_source_path.as_deref() else {
            return false;
        };
        let comparable_path = |path: &Path| {
            std::fs::canonicalize(path).unwrap_or_else(|_| {
                path.parent()
                    .and_then(|parent| std::fs::canonicalize(parent).ok())
                    .and_then(|parent| path.file_name().map(|name| parent.join(name)))
                    .unwrap_or_else(|| path.to_owned())
            })
        };
        if comparable_path(source) == comparable_path(destination) {
            return false;
        }
        let migration = self
            .review_candidate
            .as_ref()
            .and_then(ketchup_core::persistence::LoadOutcome::review_candidate)
            .ok_or(
                ketchup_core::persistence::PersistenceError::MigrationNotConfirmable(
                    "no review candidate is pending",
                ),
            )
            .and_then(ketchup_core::persistence::ReviewCandidate::confirm_semantic_migration);
        let confirmed = match migration {
            Ok(confirmed) => confirmed,
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-migrate-document",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                return false;
            }
        };
        let (mut document, container_data) = confirmed.into_parts();
        let snapshot = document.current();
        if let Err(error) = ketchup_core::persistence::save_atomic_with_container(
            destination,
            &snapshot,
            &container_data,
        ) {
            self.digest = self.catalog.format(
                "error-migrate-document",
                &BTreeMap::from([("reason", error.to_string())]),
            );
            return false;
        }

        document.discard_history_before_current();
        self.document = document;
        self.container_data = container_data;
        self.review_candidate = None;
        self.review_candidate_source_path = None;
        self.document_path = Some(destination.to_owned());
        self.saved_digest = snapshot.canonical_digest();
        self.reset_document_presentation();
        self.digest = self.catalog.format(
            "digest-migrated-document",
            &BTreeMap::from([("path", destination.display().to_string())]),
        );
        true
    }

    pub fn is_dirty(&self) -> bool {
        self.document.current().canonical_digest() != self.saved_digest
            || assistant_conversation_digest(&self.assistant_messages)
                != self.saved_assistant_conversation_digest
    }

    /// Path the active document is bound to, if it has been saved or opened.
    #[must_use]
    pub fn document_path(&self) -> Option<&Path> {
        self.document_path.as_deref()
    }

    /// The action digest the shell is currently reporting to the user.
    #[must_use]
    pub fn action_digest(&self) -> &str {
        &self.digest
    }

    pub fn prepare_assistant_intent(&mut self, intent: WorkflowIntent) -> bool {
        let is_smart_push_pull = matches!(&intent, WorkflowIntent::SetFeatureDimension { .. });
        match propose_intent(&self.document, IntentRequest::m7a(intent)) {
            Ok(proposal) => {
                let proposal = if is_smart_push_pull {
                    let context = ProposalContext {
                        principal: ProposalPrincipal::LocalAssistant,
                        goal: proposal.goal(),
                        assumptions: proposal.assumptions().to_vec(),
                        risk: proposal.risk(),
                        confirmation: proposal.confirmation().clone(),
                        requested_budget: proposal.requested_budget(),
                    };
                    let Some(proposal) = self.prepare_smart_push_pull_proposal_with_context(
                        proposal.batch().clone(),
                        context,
                    ) else {
                        self.assistant_proposal = None;
                        return false;
                    };
                    proposal
                } else {
                    proposal
                };
                self.digest = self.catalog.format(
                    "assistant-digest-preview",
                    &BTreeMap::from([
                        (
                            "reads",
                            proposal.authoritative_dependencies().len().to_string(),
                        ),
                        ("writes", proposal.authoritative_writes().len().to_string()),
                    ]),
                );
                self.status_key = "status-preview";
                self.assistant_verification = None;
                self.assistant_proposal = Some(proposal);
                true
            }
            Err(error) => {
                self.assistant_proposal = None;
                self.digest = self.catalog.format(
                    "assistant-digest-rejected",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                false
            }
        }
    }

    pub fn apply_assistant_intent(&mut self, intent: WorkflowIntent) -> bool {
        self.prepare_assistant_intent(intent)
            && self
                .assistant_proposal
                .as_ref()
                .is_some_and(Self::assistant_proposal_is_low_risk)
            && self.confirm_assistant_proposal()
    }

    pub fn confirm_assistant_proposal(&mut self) -> bool {
        let Some(proposal) = self.assistant_proposal.take() else {
            return false;
        };
        let snapshot = self.document.current();
        if proposal.document_id() != snapshot.document_id()
            || proposal.provenance_revision() != snapshot.revision_id()
            || proposal.provenance_digest() != snapshot.canonical_digest()
        {
            self.status_key = "status-ready";
            self.digest = self.catalog.format(
                "assistant-digest-rejected",
                &BTreeMap::from([(
                    "reason",
                    self.catalog.text("assistant-error-stale-response"),
                )]),
            );
            return false;
        }
        match self.document.commit_verified_proposal(&proposal) {
            Ok(committed) => {
                let verification = AssistantVerification {
                    revision_id: committed.revision().id(),
                    command_digest: committed.command_digest().to_owned(),
                    result_digest: committed.result_digest().to_owned(),
                    canonical_digest: self.document.current().canonical_digest(),
                    verified_write_count: committed.verified_writes().len(),
                };
                self.digest = self.catalog.format(
                    "assistant-digest-committed",
                    &BTreeMap::from([
                        ("revision", verification.revision_id.to_string()),
                        ("writes", verification.verified_write_count.to_string()),
                    ]),
                );
                self.status_key = "status-ready";
                self.assistant_verification = Some(verification);
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "assistant-digest-rejected",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                false
            }
        }
    }

    pub fn cancel_assistant_proposal(&mut self) -> bool {
        if self.assistant_proposal.take().is_none() {
            return false;
        }
        self.status_key = "status-ready";
        self.digest = self.catalog.text("assistant-digest-cancelled");
        true
    }

    #[must_use]
    pub const fn assistant_proposal(&self) -> Option<&Proposal> {
        self.assistant_proposal.as_ref()
    }

    fn assistant_proposal_target_label(&self, target: &AuthoritativeDependency) -> String {
        let identified = match target {
            AuthoritativeDependency::EvaluatorNode(id) => {
                Some(("assistant-entity-evaluator", id.0))
            }
            AuthoritativeDependency::Override(id) => Some(("assistant-entity-override", *id)),
            AuthoritativeDependency::Joint(id) => Some(("assistant-entity-joint", id.0)),
            AuthoritativeDependency::Space(id) => Some(("assistant-entity-space", id.0)),
            AuthoritativeDependency::ClearanceVolume(id) => {
                Some(("assistant-entity-clearance", id.0))
            }
            AuthoritativeDependency::PersistentDimension(id) => {
                Some(("assistant-entity-persistent-dimension", id.0))
            }
            AuthoritativeDependency::Tag(id) => Some(("assistant-entity-tag", id.0)),
            AuthoritativeDependency::Collection(id) => Some(("assistant-entity-collection", id.0)),
            AuthoritativeDependency::Import(_) => None,
            AuthoritativeDependency::Definition(id) => Some(("assistant-entity-definition", id.0)),
            AuthoritativeDependency::DefinitionUsers(id) => {
                Some(("assistant-entity-definition-users", id.0))
            }
            AuthoritativeDependency::Feature(id) => Some(("assistant-entity-feature", id.0)),
            AuthoritativeDependency::FeatureUsers(id) => {
                Some(("assistant-entity-feature-users", id.0))
            }
            AuthoritativeDependency::FeatureParameterBindings(id) => {
                Some(("assistant-entity-feature-bindings", id.0))
            }
            AuthoritativeDependency::Occurrence(id) => Some(("assistant-entity-occurrence", id.0)),
            AuthoritativeDependency::OccurrenceCollections(id) => {
                Some(("assistant-entity-occurrence-collections", id.0))
            }
            AuthoritativeDependency::Group(id) => Some(("assistant-entity-group", id.0)),
            AuthoritativeDependency::GroupChildren(id) => {
                Some(("assistant-entity-group-children", id.0))
            }
            AuthoritativeDependency::GroupSubtree(id) => {
                Some(("assistant-entity-group-subtree", id.0))
            }
            AuthoritativeDependency::FeatureParameterBinding(target) => {
                return self.catalog.format(
                    "assistant-target-feature-parameter",
                    &BTreeMap::from([
                        ("feature", target.feature_id.0.to_string()),
                        ("slot", target.slot.label().to_owned()),
                    ]),
                );
            }
            AuthoritativeDependency::LocalGroup(key) => {
                return self.catalog.format(
                    "assistant-target-local-group",
                    &BTreeMap::from([
                        ("definition", key.definition_id.0.to_string()),
                        ("local", key.local_id.0.to_string()),
                    ]),
                );
            }
            AuthoritativeDependency::LocalOccurrence(key) => {
                return self.catalog.format(
                    "assistant-target-local-occurrence",
                    &BTreeMap::from([
                        ("definition", key.definition_id.0.to_string()),
                        ("local", key.local_id.0.to_string()),
                    ]),
                );
            }
        };
        identified.map_or_else(
            || self.catalog.text("assistant-target-canonical"),
            |(kind, id)| {
                self.catalog.format(
                    "assistant-target-identified",
                    &BTreeMap::from([("kind", self.catalog.text(kind)), ("id", id.to_string())]),
                )
            },
        )
    }

    fn assistant_derived_identity_label(identity: &DerivedIdentity) -> String {
        let path = identity
            .slot_path
            .segments()
            .iter()
            .map(|segment| {
                format!(
                    "{}:{}:{}",
                    segment.producer_rule_id.0, segment.output_port, segment.semantic_key
                )
            })
            .collect::<Vec<_>>()
            .join(" / ");
        format!("{} / {path}", identity.root_rule_node_id.0)
    }

    fn assistant_rule_outputs_label(outputs: &[RuleOutput]) -> String {
        fn collect(output: &RuleOutput, prefix: &str, labels: &mut Vec<String>) {
            let segment = output.segment();
            let current = format!(
                "{prefix}{}:{}:{}",
                segment.producer_rule_id.0, segment.output_port, segment.semantic_key
            );
            labels.push(current.clone());
            for child in output.children() {
                collect(child, &format!("{current} / "), labels);
            }
        }

        let mut labels = Vec::new();
        for output in outputs {
            collect(output, "", &mut labels);
        }
        labels.join(", ")
    }

    fn assistant_instance_path_label(path: &InstancePath) -> String {
        let mut label = path.root_occurrence().0.to_string();
        for step in path.steps() {
            match step {
                ketchup_core::document::InstancePathStep::Group(id) => {
                    label.push_str(&format!(" / G{}", id.0));
                }
                ketchup_core::document::InstancePathStep::Occurrence(id) => {
                    label.push_str(&format!(" / O{}", id.0));
                }
            }
        }
        label
    }

    fn assistant_transform_matrix_label(transform: &Transform) -> String {
        transform
            .matrix()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn assistant_proposal_value_label(&self, value: &ProposalValue) -> String {
        match value {
            ProposalValue::Missing => self.catalog.text("assistant-value-missing"),
            ProposalValue::Boolean(value) => self.catalog.text(if *value {
                "assistant-value-true"
            } else {
                "assistant-value-false"
            }),
            ProposalValue::Dimension(value) => self.catalog.format(
                "assistant-value-dimension",
                &BTreeMap::from([
                    ("source", value.source_token().to_owned()),
                    ("value", value.millimetres().to_string()),
                ]),
            ),
            ProposalValue::BottleEdgeFinishKind(BottleEdgeFinishKind::Fillet) => {
                self.catalog.text("assistant-value-fillet")
            }
            ProposalValue::BottleEdgeFinishKind(BottleEdgeFinishKind::Chamfer) => {
                self.catalog.text("assistant-value-chamfer")
            }
            ProposalValue::ProfilePoints(points) => self.catalog.format(
                "assistant-value-profile-points",
                &BTreeMap::from([(
                    "points",
                    points
                        .iter()
                        .map(|point| format!("{},{}", point[0], point[1]))
                        .collect::<Vec<_>>()
                        .join("; "),
                )]),
            ),
            ProposalValue::Transform(value) => self.catalog.format(
                "assistant-value-transform",
                &BTreeMap::from([("matrix", Self::assistant_transform_matrix_label(value))]),
            ),
            ProposalValue::Tag(Some(id)) => self.catalog.format(
                "assistant-value-tag",
                &BTreeMap::from([("id", id.0.to_string())]),
            ),
            ProposalValue::Tag(None) => self.catalog.text("assistant-value-no-tag"),
            ProposalValue::Definition(id) => self.catalog.format(
                "assistant-value-definition",
                &BTreeMap::from([("id", id.0.to_string())]),
            ),
            ProposalValue::Group(Some(id)) => self.catalog.format(
                "assistant-value-group",
                &BTreeMap::from([("id", id.0.to_string())]),
            ),
            ProposalValue::Group(None) => self.catalog.text("assistant-value-no-group"),
            ProposalValue::Occurrences(ids) => self.catalog.format(
                "assistant-value-occurrences",
                &BTreeMap::from([(
                    "ids",
                    ids.iter()
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                )]),
            ),
            ProposalValue::Text(value) => value.clone(),
            ProposalValue::Digest(value) => self.catalog.format(
                "assistant-value-digest",
                &BTreeMap::from([("digest", value.clone())]),
            ),
            ProposalValue::EvaluatorInputState {
                name,
                dimension,
                dependencies,
            } => self.catalog.format(
                "assistant-value-evaluator-input-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("source", dimension.source_token().to_owned()),
                    ("value", dimension.millimetres().to_string()),
                    (
                        "dependencies",
                        dependencies
                            .iter()
                            .map(|id| id.0.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]),
            ),
            ProposalValue::EvaluatorExpressionState {
                name,
                expression,
                dependencies,
            } => self.catalog.format(
                "assistant-value-evaluator-expression-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("expression", expression.clone()),
                    (
                        "dependencies",
                        dependencies
                            .iter()
                            .map(|id| id.0.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]),
            ),
            ProposalValue::EvaluatorRuleState {
                name,
                expression,
                dependencies,
                input_ports,
                output_ports,
                outputs,
                override_parameters,
            } => self.catalog.format(
                "assistant-value-evaluator-rule-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("expression", expression.clone()),
                    (
                        "dependencies",
                        dependencies
                            .iter()
                            .map(|id| id.0.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    (
                        "inputs",
                        input_ports
                            .iter()
                            .map(|port| port.name().to_owned())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    (
                        "output-ports",
                        output_ports
                            .iter()
                            .map(|port| port.name().to_owned())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    ("outputs", Self::assistant_rule_outputs_label(outputs)),
                    (
                        "overrides",
                        override_parameters
                            .iter()
                            .map(|parameter| parameter.name().to_owned())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]),
            ),
            ProposalValue::RuleOverrideState {
                target,
                parameter,
                value,
                health,
            } => {
                let health = match health {
                    ketchup_core::graph::SlotResolution::Resolved => {
                        self.catalog.text("assistant-health-resolved")
                    }
                    ketchup_core::graph::SlotResolution::Ambiguous { segment_index } => {
                        self.catalog.format(
                            "assistant-health-ambiguous",
                            &BTreeMap::from([("segment", segment_index.to_string())]),
                        )
                    }
                    ketchup_core::graph::SlotResolution::Lost { segment_index } => {
                        self.catalog.format(
                            "assistant-health-lost",
                            &BTreeMap::from([("segment", segment_index.to_string())]),
                        )
                    }
                };
                self.catalog.format(
                    "assistant-value-rule-override-state",
                    &BTreeMap::from([
                        ("rule", target.root_rule_node_id.0.to_string()),
                        ("path", Self::assistant_derived_identity_label(target)),
                        ("parameter", parameter.clone()),
                        ("value", value.to_string()),
                        ("health", health),
                    ]),
                )
            }
            ProposalValue::FeatureParameterBindingState {
                target,
                derived_from,
            } => self.catalog.format(
                "assistant-value-feature-parameter-binding-state",
                &BTreeMap::from([
                    ("feature", target.feature_id.0.to_string()),
                    ("slot", target.slot.label().to_owned()),
                    ("rule", derived_from.root_rule_node_id.0.to_string()),
                    ("path", Self::assistant_derived_identity_label(derived_from)),
                ]),
            ),
            ProposalValue::JointState {
                participant_a,
                participant_b,
                volume_min,
                volume_max,
            } => self.catalog.format(
                "assistant-value-joint-state",
                &BTreeMap::from([
                    (
                        "participant-a",
                        Self::assistant_derived_identity_label(participant_a),
                    ),
                    (
                        "participant-b",
                        Self::assistant_derived_identity_label(participant_b),
                    ),
                    (
                        "min",
                        format!("{},{},{}", volume_min[0], volume_min[1], volume_min[2]),
                    ),
                    (
                        "max",
                        format!("{},{},{}", volume_max[0], volume_max[1], volume_max[2]),
                    ),
                ]),
            ),
            ProposalValue::SpaceState {
                purpose,
                volume_min,
                volume_max,
                adjacent_to,
                accessible_to,
            } => self.catalog.format(
                "assistant-value-space-state",
                &BTreeMap::from([
                    ("purpose", purpose.clone()),
                    (
                        "min",
                        format!("{},{},{}", volume_min[0], volume_min[1], volume_min[2]),
                    ),
                    (
                        "max",
                        format!("{},{},{}", volume_max[0], volume_max[1], volume_max[2]),
                    ),
                    (
                        "adjacent",
                        adjacent_to
                            .iter()
                            .map(|id| id.0.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    (
                        "accessible",
                        accessible_to
                            .iter()
                            .map(|id| id.0.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]),
            ),
            ProposalValue::ClearanceVolumeState {
                owner,
                reason,
                volume_min,
                volume_max,
                coordinate_frame: _,
                tolerance_mm,
                severity,
                derived_from,
            } => {
                let owner = match owner {
                    ketchup_core::space::ClearanceOwner::Occurrence(path) => {
                        Self::assistant_instance_path_label(path)
                    }
                    ketchup_core::space::ClearanceOwner::Space(id) => id.0.to_string(),
                };
                let severity = match severity {
                    ClearanceSeverity::Advisory => "assistant-severity-advisory",
                    ClearanceSeverity::Required => "assistant-severity-required",
                };
                self.catalog.format(
                    "assistant-value-clearance-volume-state",
                    &BTreeMap::from([
                        ("reason", reason.clone()),
                        ("owner", owner),
                        (
                            "min",
                            format!("{},{},{}", volume_min[0], volume_min[1], volume_min[2]),
                        ),
                        (
                            "max",
                            format!("{},{},{}", volume_max[0], volume_max[1], volume_max[2]),
                        ),
                        ("frame", self.catalog.text("assistant-frame-world")),
                        ("tolerance", tolerance_mm.to_string()),
                        ("severity", self.catalog.text(severity)),
                        (
                            "derived",
                            derived_from.as_ref().map_or_else(
                                || self.catalog.text("assistant-value-missing"),
                                Self::assistant_derived_identity_label,
                            ),
                        ),
                    ]),
                )
            }
            ProposalValue::PersistentDimensionState {
                name,
                target,
                presentation,
            } => {
                let target = match target {
                    ketchup_core::document::PersistentDimensionTarget::FeatureParameter(target) => {
                        format!("{}:{}", target.feature_id.0, target.slot.label())
                    }
                    ketchup_core::document::PersistentDimensionTarget::DerivedOutput(identity) => {
                        Self::assistant_derived_identity_label(identity)
                    }
                    ketchup_core::document::PersistentDimensionTarget::ExactFeatureParameter {
                        definition_id,
                        producer_feature_id,
                        semantic_role,
                        source_element_id,
                        slot,
                    } => format!(
                        "{}:{}:{}:{}:{}",
                        definition_id.0,
                        producer_feature_id.0,
                        semantic_role,
                        source_element_id,
                        slot.label()
                    ),
                };
                self.catalog.format(
                    "assistant-value-persistent-dimension-state",
                    &BTreeMap::from([
                        ("name", name.clone()),
                        ("target", target),
                        ("unit", presentation.unit.label().to_owned()),
                        ("precision", presentation.decimal_places.to_string()),
                    ]),
                )
            }
            ProposalValue::RuleOutputs(outputs) => self.catalog.format(
                "assistant-value-rule-outputs",
                &BTreeMap::from([("outputs", Self::assistant_rule_outputs_label(outputs))]),
            ),
            ProposalValue::TagState { name, visible } => self.catalog.format(
                "assistant-value-tag-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    (
                        "visible",
                        self.catalog.text(if *visible {
                            "assistant-value-true"
                        } else {
                            "assistant-value-false"
                        }),
                    ),
                ]),
            ),
            ProposalValue::CollectionState {
                name,
                occurrence_ids,
            } => self.catalog.format(
                "assistant-value-collection-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    (
                        "ids",
                        occurrence_ids
                            .iter()
                            .map(|id| id.0.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]),
            ),
            ProposalValue::DefinitionState {
                name,
                feature_ids,
                local_occurrence_ids,
                local_group_ids,
            } => self.catalog.format(
                "assistant-value-definition-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    (
                        "features",
                        feature_ids
                            .iter()
                            .map(|id| id.0.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    (
                        "occurrences",
                        local_occurrence_ids
                            .iter()
                            .map(|id| id.0.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    (
                        "groups",
                        local_group_ids
                            .iter()
                            .map(|id| id.0.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]),
            ),
            ProposalValue::DefinitionFeatures(ids) => self.catalog.format(
                "assistant-value-definition-features",
                &BTreeMap::from([(
                    "ids",
                    ids.iter()
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                )]),
            ),
            ProposalValue::ProfileFeatureState {
                definition,
                name,
                points_mm,
            } => self.catalog.format(
                "assistant-value-profile-feature-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("definition", definition.0.to_string()),
                    (
                        "points",
                        points_mm
                            .iter()
                            .map(|point| format!("{},{}", point[0], point[1]))
                            .collect::<Vec<_>>()
                            .join("; "),
                    ),
                ]),
            ),
            ProposalValue::GroupState {
                name,
                transform,
                parent,
            } => {
                let matrix = transform.matrix();
                self.catalog.format(
                    "assistant-value-group-state",
                    &BTreeMap::from([
                        ("name", name.clone()),
                        ("x", matrix[3].to_string()),
                        ("y", matrix[7].to_string()),
                        ("z", matrix[11].to_string()),
                        ("matrix", Self::assistant_transform_matrix_label(transform)),
                        (
                            "parent",
                            parent.map_or_else(
                                || self.catalog.text("assistant-value-no-group"),
                                |id| id.0.to_string(),
                            ),
                        ),
                    ]),
                )
            }
            ProposalValue::OccurrenceState {
                definition,
                name,
                transform,
                parent,
                tag,
                visible,
            } => {
                let matrix = transform.matrix();
                self.catalog.format(
                    "assistant-value-occurrence-state",
                    &BTreeMap::from([
                        ("name", name.clone()),
                        ("definition", definition.0.to_string()),
                        ("x", matrix[3].to_string()),
                        ("y", matrix[7].to_string()),
                        ("z", matrix[11].to_string()),
                        ("matrix", Self::assistant_transform_matrix_label(transform)),
                        (
                            "parent",
                            parent.map_or_else(
                                || self.catalog.text("assistant-value-no-group"),
                                |id| id.0.to_string(),
                            ),
                        ),
                        (
                            "tag",
                            tag.map_or_else(
                                || self.catalog.text("assistant-value-no-tag"),
                                |id| id.0.to_string(),
                            ),
                        ),
                        (
                            "visible",
                            self.catalog.text(if *visible {
                                "assistant-value-true"
                            } else {
                                "assistant-value-false"
                            }),
                        ),
                    ]),
                )
            }
        }
    }

    fn assistant_proposal_is_low_risk(proposal: &Proposal) -> bool {
        matches!(
            proposal.goal(),
            ProposalGoal::SetRuleDimension(_)
                | ProposalGoal::SetFeatureDimension(_)
                | ProposalGoal::SetBottleControlDimension(_, _)
                | ProposalGoal::SetBottleEdgeFinishKind(_)
                | ProposalGoal::SetOccurrenceVisibility(_)
                | ProposalGoal::SetTagVisibility(_)
        )
    }

    #[must_use]
    pub const fn assistant_provider(&self) -> AssistantProvider {
        self.assistant_provider
    }

    #[must_use]
    pub fn assistant_model(&self) -> &str {
        &self.assistant_model
    }

    pub fn select_assistant_provider(&mut self, provider: AssistantProvider) {
        self.assistant_provider = provider;
        self.assistant_model = provider.default_model().to_owned();
    }

    pub fn set_assistant_model(&mut self, model: impl Into<String>) {
        self.assistant_model = model.into();
    }

    #[must_use]
    pub const fn assistant_workspace_mode(&self) -> AssistantWorkspaceMode {
        self.assistant_workspace_mode
    }

    pub fn set_assistant_workspace_mode(&mut self, mode: AssistantWorkspaceMode) {
        self.assistant_workspace_mode = mode;
    }

    #[must_use]
    pub fn assistant_messages(&self) -> &[AssistantChatMessage] {
        &self.assistant_messages
    }

    pub fn new_assistant_chat(&mut self) {
        self.assistant_input.clear();
        self.assistant_messages.clear();
        self.cancel_pending_assistant_work();
        self.assistant_verification = None;
        self.assistant_request_sequence = self.assistant_request_sequence.saturating_add(1);
        self.store_assistant_conversation();
    }

    fn store_assistant_conversation(&mut self) {
        let conversation = AssistantConversation {
            document_id: self.document.current().document_id().0,
            messages: self.assistant_messages.clone(),
        };
        let Ok(bytes) = serde_json::to_vec(&conversation) else {
            return;
        };
        let Ok(entry) = ketchup_core::persistence::ExtensionEntry::new(
            ASSISTANT_CHAT_NAMESPACE,
            ASSISTANT_CHAT_PATH,
            false,
            bytes,
        ) else {
            return;
        };
        self.container_data.set_extension(entry);
    }

    fn load_assistant_conversation(&mut self) {
        let document_id = self.document.current().document_id().0;
        self.assistant_messages = self
            .container_data
            .extensions()
            .find(|entry| {
                entry.namespace() == ASSISTANT_CHAT_NAMESPACE && entry.path() == ASSISTANT_CHAT_PATH
            })
            .and_then(|entry| serde_json::from_slice::<AssistantConversation>(entry.bytes()).ok())
            .filter(|conversation| conversation.document_id == document_id)
            .map_or_else(Vec::new, |conversation| conversation.messages);
        self.saved_assistant_conversation_digest =
            assistant_conversation_digest(&self.assistant_messages);
    }

    #[must_use]
    pub fn assistant_models(&self) -> Vec<String> {
        assistant_models_for(self.assistant_provider)
    }

    fn assistant_source_label(&self) -> String {
        format!(
            "{} · {}",
            self.catalog.text(self.assistant_provider.label_key()),
            self.assistant_model
        )
    }

    #[must_use]
    pub fn assistant_handshake(&self) -> AssistantHandshake {
        AssistantHandshake {
            protocol_version: ASSISTANT_PROTOCOL_VERSION,
            distribution: self.assistant_provider.distribution(),
            provider: self.assistant_provider.protocol_name().to_owned(),
            model: self.assistant_model.clone(),
            capabilities: BTreeSet::from([
                AssistantCapability::Chat,
                AssistantCapability::LocalMemory,
                AssistantCapability::QueryDocument,
                AssistantCapability::ProposeWorkflowIntent,
            ]),
        }
    }

    pub fn assistant_context(&self) -> serde_json::Value {
        let snapshot = self.document.current();
        let box_bounds = self
            .active_boxes()
            .into_iter()
            .map(|item| {
                (
                    item.instance_path.root_occurrence(),
                    [item.origin_mm, item.origin_mm + item.size_mm],
                )
            })
            .collect::<BTreeMap<_, _>>();
        let scene_occurrences = snapshot.scene_query();
        let occurrence_count = scene_occurrences.len();
        let occurrences = scene_occurrences
            .into_iter()
            .take(100)
            .map(|occurrence| {
                let bounds = box_bounds
                    .get(&occurrence.occurrence_id)
                    .copied()
                    .or_else(|| assistant_mesh_body_bounds(&snapshot, &occurrence));
                serde_json::json!({
                    "occurrence_id": occurrence.occurrence_id.0,
                    "definition_id": occurrence.definition_id.0,
                    "name": occurrence.occurrence_name,
                    "visible": occurrence.visible,
                    "copyable": occurrence.instance_path.is_root(),
                    "bounds_mm": bounds.map(|[minimum, maximum]| serde_json::json!({
                        "min": [minimum.x, minimum.y, minimum.z],
                        "max": [maximum.x, maximum.y, maximum.z],
                    })),
                })
            })
            .collect::<Vec<_>>();
        let conversation = self
            .assistant_messages
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
                })
            })
            .collect::<Vec<_>>();
        let selected_occurrence_ids = self
            .selected_occurrence_ids()
            .into_iter()
            .map(|id| id.0)
            .collect::<Vec<_>>();
        let boxes = self
            .active_boxes()
            .into_iter()
            .take(100)
            .map(|item| {
                serde_json::json!({
                    "occurrence_id": item.instance_path.root_occurrence().0,
                    "definition_id": item.definition_id.0,
                    "origin_mm": [item.origin_mm.x, item.origin_mm.y, item.origin_mm.z],
                    "size_mm": [item.size_mm.x, item.size_mm.y, item.size_mm.z],
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "document_id": snapshot.document_id().0,
            "revision": snapshot.revision_id(),
            "canonical_digest": snapshot.canonical_digest(),
            "selected_occurrence_ids": selected_occurrence_ids,
            "occurrence_count": occurrence_count,
            "occurrences_complete": occurrence_count <= 100,
            "occurrences": occurrences,
            "boxes": boxes,
            "conversation": conversation,
        })
    }

    fn send_assistant_message(&mut self, context: &egui::Context) {
        if self.assistant_chat_task.is_some() || self.assistant_pending_execution.is_some() {
            return;
        }
        let message = self.assistant_input.trim().to_owned();
        if message.is_empty() {
            return;
        }
        let handshake = self.assistant_handshake();
        if let Err(error) = handshake.validate() {
            self.assistant_messages.push(AssistantChatMessage {
                role: AssistantMessageRole::Error,
                text: error.to_string(),
                source: self.assistant_source_label(),
            });
            return;
        }
        self.assistant_input.clear();
        let document_context = self.assistant_context();
        let request_document_id = self.document.current().document_id();
        let request_revision_id = self.document.current().revision_id();
        let request_canonical_digest = self.document.current().canonical_digest();
        let source = self.assistant_source_label();
        self.assistant_messages.push(AssistantChatMessage {
            role: AssistantMessageRole::User,
            text: message.clone(),
            source: source.clone(),
        });
        self.assistant_request_sequence = self.assistant_request_sequence.saturating_add(1);
        let request_id = format!("chat-{}", self.assistant_request_sequence);
        let transport = Arc::clone(&self.assistant_transport);
        let repaint = context.clone();
        let cancellation = AssistantCancellation::default();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = transport
                .chat(
                    handshake,
                    &request_id,
                    &message,
                    &document_context,
                    worker_cancellation,
                )
                .and_then(|result| {
                    result.validate()?;
                    Ok(result)
                });
            if sender.send(result).is_ok() {
                repaint.request_repaint();
            }
        });
        self.assistant_chat_task = Some(AssistantChatTask {
            receiver,
            cancellation,
            document_id: request_document_id,
            revision_id: request_revision_id,
            canonical_digest: request_canonical_digest,
            source,
        });
    }

    pub fn prepare_assistant_model_intent(&mut self, intent: AssistantModelIntent) -> bool {
        if let Err(error) = intent.validate() {
            self.digest = self.catalog.format(
                "assistant-digest-rejected",
                &BTreeMap::from([("reason", error)]),
            );
            return false;
        }
        let snapshot = self.document.current();
        let mut commands = Vec::new();
        if intent.replace_scene {
            commands.extend(snapshot.collections().map(|collection| {
                CanonicalCommand::SetCollectionOccurrences {
                    id: collection.id(),
                    occurrence_ids: Vec::new(),
                }
            }));
            commands.extend(
                snapshot
                    .occurrences()
                    .map(|item| CanonicalCommand::DeleteOccurrence { id: item.id() }),
            );
            commands.extend(
                snapshot
                    .definitions()
                    .map(|item| CanonicalCommand::DeleteDefinition { id: item.id() }),
            );
        }
        let mut next_definition = snapshot
            .definitions()
            .map(|item| item.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let mut next_feature = snapshot
            .features()
            .map(|item| item.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let mut next_occurrence = snapshot
            .occurrences()
            .map(|item| item.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        for translation in intent.translations {
            let Some(occurrence) = snapshot.occurrence(OccurrenceId(translation.occurrence_id))
            else {
                self.digest = self.catalog.format(
                    "assistant-digest-rejected",
                    &BTreeMap::from([(
                        "reason",
                        format!("occurrence {} does not exist", translation.occurrence_id),
                    )]),
                );
                return false;
            };
            let [x, y, z] = translation.delta_mm;
            let Ok(transform) = translated_transform(occurrence.transform(), Vec3::new(x, y, z))
            else {
                return false;
            };
            commands.push(CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(translation.occurrence_id),
                transform,
            });
        }
        for array in intent.linear_arrays {
            let sources = array
                .occurrence_ids
                .iter()
                .map(|id| snapshot.occurrence(OccurrenceId(*id)).cloned().ok_or(*id))
                .collect::<Result<Vec<_>, _>>();
            let Ok(sources) = sources else {
                let missing_id = sources.unwrap_err();
                self.digest = self.catalog.format(
                    "assistant-digest-rejected",
                    &BTreeMap::from([(
                        "reason",
                        format!("occurrence {missing_id} does not exist"),
                    )]),
                );
                return false;
            };
            for instance in 1..array.instances {
                let [step_x, step_y, step_z] = array.step_mm;
                let delta = Vec3::new(
                    step_x * f64::from(instance),
                    step_y * f64::from(instance),
                    step_z * f64::from(instance),
                );
                for source in &sources {
                    let Some(occurrence) = next_occurrence.map(OccurrenceId) else {
                        self.digest = self.catalog.format(
                            "assistant-digest-rejected",
                            &BTreeMap::from([(
                                "reason",
                                "assistant model identifiers are exhausted".to_owned(),
                            )]),
                        );
                        return false;
                    };
                    let Ok(transform) = translated_transform(source.transform(), delta) else {
                        return false;
                    };
                    commands.push(CanonicalCommand::CreateOccurrence {
                        id: occurrence,
                        definition_id: source.definition_id(),
                        name: source.name().to_owned(),
                        transform,
                        parent: source.parent(),
                        tag: source.tag(),
                        visible: source.visible(),
                    });
                    next_occurrence = occurrence.0.checked_add(1);
                }
            }
        }
        for item in intent.boxes {
            let feature_count = if item.subtract_boxes.is_empty() { 2 } else { 1 };
            let (Some(definition), Some(feature), Some(occurrence)) = (
                next_definition.map(DefinitionId),
                next_feature.map(FeatureId),
                next_occurrence.map(OccurrenceId),
            ) else {
                self.digest = self.catalog.format(
                    "assistant-digest-rejected",
                    &BTreeMap::from([(
                        "reason",
                        "assistant model identifiers are exhausted".to_owned(),
                    )]),
                );
                return false;
            };
            let [width, depth, height] = item.size_mm;
            let [x, y, z] = item.origin_mm;
            let Ok(transform) = Transform::from_translation(x, y, z) else {
                return false;
            };
            commands.push(CanonicalCommand::CreateDefinition {
                id: definition,
                name: item.name.clone(),
            });
            if item.subtract_boxes.is_empty() {
                let Some(extrusion) = feature.0.checked_add(1).map(FeatureId) else {
                    return false;
                };
                let Ok(height_dimension) = Dimension::new(height.to_string(), height) else {
                    return false;
                };
                commands.extend([
                    CanonicalCommand::CreateFeature {
                        id: feature,
                        definition_id: definition,
                        name: format!("{} profile", item.name),
                        kind: FeatureKind::Profile {
                            points_mm: vec![[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]],
                        },
                    },
                    CanonicalCommand::CreateFeature {
                        id: extrusion,
                        definition_id: definition,
                        name: format!("{} extrusion", item.name),
                        kind: FeatureKind::Extrusion {
                            profile: feature,
                            height: height_dimension,
                        },
                    },
                ]);
            } else {
                let Some(mesh) = assistant_subtracted_box_mesh(&item) else {
                    return false;
                };
                commands.push(CanonicalCommand::CreateFeature {
                    id: feature,
                    definition_id: definition,
                    name: format!("{} solid", item.name),
                    kind: FeatureKind::MeshBody(mesh),
                });
            }
            commands.push(CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: definition,
                name: item.name,
                transform,
                parent: None,
                tag: None,
                visible: true,
            });
            next_definition = definition.0.checked_add(1);
            next_feature = feature.0.checked_add(feature_count);
            next_occurrence = occurrence.0.checked_add(1);
        }
        match self.document.prepare_proposal_with_context(
            CommandBatch::new(commands),
            ProposalContext::local_assistant_model(),
        ) {
            Ok(proposal) => {
                self.digest = self.catalog.format(
                    "assistant-digest-preview",
                    &BTreeMap::from([
                        (
                            "reads",
                            proposal.authoritative_dependencies().len().to_string(),
                        ),
                        ("writes", proposal.authoritative_writes().len().to_string()),
                    ]),
                );
                self.status_key = "status-preview";
                self.assistant_verification = None;
                self.assistant_proposal = Some(proposal);
                true
            }
            Err(error) => {
                self.assistant_proposal = None;
                self.digest = self.catalog.format(
                    "assistant-digest-rejected",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                false
            }
        }
    }

    pub fn apply_assistant_model_intent(&mut self, intent: AssistantModelIntent) -> bool {
        self.prepare_assistant_model_intent(intent)
            && self
                .assistant_proposal
                .as_ref()
                .is_some_and(Self::assistant_proposal_is_low_risk)
            && self.confirm_assistant_proposal()
    }

    fn poll_assistant_chat(&mut self, context: &egui::Context) {
        if let Some(pending) = self.assistant_pending_execution.take() {
            let snapshot = self.document.current();
            if snapshot.document_id() != pending.document_id
                || snapshot.revision_id() != pending.revision_id
                || snapshot.canonical_digest() != pending.canonical_digest
            {
                self.assistant_messages.push(AssistantChatMessage {
                    role: AssistantMessageRole::Error,
                    text: self.catalog.text("assistant-error-stale-response"),
                    source: self.catalog.text("assistant-role-error"),
                });
            } else if self.prepare_assistant_model_intent(
                pending
                    .result
                    .model_intent
                    .expect("pending execution always carries a model intent"),
            ) {
                self.assistant_messages.push(AssistantChatMessage {
                    role: AssistantMessageRole::Assistant,
                    text: pending.result.message,
                    source: pending.source,
                });
            } else {
                self.assistant_messages.push(AssistantChatMessage {
                    role: AssistantMessageRole::Error,
                    text: self.catalog.text("assistant-error-rejected-change"),
                    source: self.catalog.text("assistant-role-error"),
                });
            }
            self.store_assistant_conversation();
            return;
        }

        let Some(task) = self.assistant_chat_task.as_ref() else {
            return;
        };
        let source = task.source.clone();
        let request_document_id = task.document_id;
        let request_revision_id = task.revision_id;
        let request_canonical_digest = task.canonical_digest.clone();
        match task.receiver.try_recv() {
            Ok(result) => {
                self.assistant_chat_task = None;
                match result {
                    Ok(result) if result.model_intent.is_some() => {
                        self.assistant_pending_execution = Some(AssistantPendingExecution {
                            result,
                            document_id: request_document_id,
                            revision_id: request_revision_id,
                            canonical_digest: request_canonical_digest,
                            source,
                        });
                        context.request_repaint();
                        return;
                    }
                    Ok(result) => self.assistant_messages.push(AssistantChatMessage {
                        role: AssistantMessageRole::Assistant,
                        text: result.message,
                        source,
                    }),
                    Err(text) => self.assistant_messages.push(AssistantChatMessage {
                        role: AssistantMessageRole::Error,
                        text,
                        source,
                    }),
                }
                self.store_assistant_conversation();
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.assistant_chat_task = None;
                self.assistant_messages.push(AssistantChatMessage {
                    role: AssistantMessageRole::Error,
                    text: self.catalog.text("assistant-error-disconnected"),
                    source,
                });
                self.store_assistant_conversation();
            }
        }
    }

    #[must_use]
    pub const fn assistant_verification(&self) -> Option<&AssistantVerification> {
        self.assistant_verification.as_ref()
    }

    #[must_use]
    pub fn assistant_change_can_undo(&self) -> bool {
        self.assistant_verification
            .as_ref()
            .is_some_and(|verification| {
                let snapshot = self.document.current();
                snapshot.revision_id() == verification.revision_id
                    && snapshot.canonical_digest() == verification.canonical_digest
                    && self.can_undo()
            })
    }

    fn assistant_selection_summary(&self) -> String {
        let selected = self.selected_occurrence_ids();
        match selected.len() {
            0 => self.catalog.text("assistant-selection-none"),
            1 => {
                let id = selected
                    .first()
                    .expect("one selected occurrence must have a first item");
                let name = self
                    .document
                    .current()
                    .scene_query()
                    .into_iter()
                    .find(|occurrence| {
                        occurrence.instance_path.is_root() && occurrence.occurrence_id == *id
                    })
                    .map_or_else(
                        || format!("#{}", id.0),
                        |occurrence| occurrence.occurrence_name,
                    );
                self.catalog
                    .format("assistant-selection-one", &BTreeMap::from([("name", name)]))
            }
            count => self.catalog.format(
                "assistant-selection-many",
                &BTreeMap::from([("count", count.to_string())]),
            ),
        }
    }

    /// The localization catalog the shell paints with.
    ///
    /// Acceptance tests resolve expected labels through this catalog instead of
    /// hard-coding English, so a translation change cannot break them.
    #[must_use]
    pub fn catalog(&self) -> &LocaleCatalog {
        &self.catalog
    }

    /// The visible label of `command` in the active locale.
    #[must_use]
    pub fn command_label(&self, command: AppCommand) -> String {
        self.catalog.text(CommandRegistry::spec(command).label_key)
    }

    #[must_use]
    pub fn document_height_mm(&self) -> f64 {
        self.box_height_mm(INITIAL_BOX_DEFINITION)
            .expect("the initial box definition exists")
    }

    fn box_height_mm(&self, definition_id: DefinitionId) -> Option<f64> {
        self.active_boxes()
            .into_iter()
            .find(|item| item.definition_id == definition_id)
            .map(|item| item.size_mm.z)
    }

    fn refresh_exact_products(&mut self, context: &egui::Context) {
        let snapshot = self.document.current();
        let source = (
            snapshot.document_id(),
            snapshot.revision_id(),
            snapshot.canonical_digest(),
        );
        if self
            .exact_source
            .as_ref()
            .is_some_and(|known| known != &source)
        {
            self.exact_results.clear();
            self.exact_source = None;
        }
        if self
            .exact_task
            .as_ref()
            .is_some_and(|task| task.source != source)
            && let Some(task) = self.exact_task.take()
        {
            task.cancelled.store(true, Ordering::Release);
        }
        if let Some(task) = self.exact_task.as_ref() {
            match task.receiver.try_recv() {
                Ok(result) => {
                    let task = self.exact_task.take().expect("the completed task exists");
                    if task.source == source && !task.cancelled.load(Ordering::Acquire) {
                        match result.and_then(|packages| {
                            ExactResultRegistry::accept(&snapshot, packages)
                                .map_err(|error| error.to_string())
                        }) {
                            Ok(results) => {
                                let references = results
                                    .values()
                                    .flat_map(|package| package.references().iter().cloned())
                                    .collect::<Vec<_>>();
                                if references.into_iter().all(|reference| {
                                    self.document
                                        .register_exact_reference_evidence(reference)
                                        .is_ok()
                                }) {
                                    self.exact_results = results;
                                    self.exact_source = Some(source.clone());
                                    self.exact_retry_at = None;
                                }
                            }
                            Err(_) => {
                                self.exact_retry_at = Some(Instant::now() + Duration::from_secs(1));
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.exact_task = None;
                    self.exact_retry_at = Some(Instant::now() + Duration::from_secs(1));
                }
            }
        }
        if self.exact_source.as_ref() == Some(&source)
            || self
                .exact_retry_at
                .is_some_and(|retry_at| retry_at > Instant::now())
        {
            return;
        }
        if !self.exact_worker_attempted {
            self.exact_worker_attempted = true;
            self.exact_worker_path = exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file());
        }
        let Some(executable) = self.exact_worker_path.clone() else {
            return;
        };
        let requests = snapshot
            .scene_query()
            .into_iter()
            .map(|occurrence| occurrence.definition_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|definition_id| {
                ExactFeatureChainRequest::from_snapshot(&snapshot, definition_id)
                    .map(ExactEvaluationRequest::Rectangle)
                    .or_else(|_| {
                        ExactRevolveRequest::from_snapshot(&snapshot, definition_id)
                            .map(ExactEvaluationRequest::Revolve)
                    })
                    .ok()
                    .map(|request| (definition_id, request))
            })
            .collect::<Vec<_>>();
        if requests.is_empty() {
            self.exact_results.clear();
            self.exact_source = Some(source);
            return;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let repaint = context.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let mut worker =
                    ExactWorkerSupervisor::spawn_with_cancellation(executable, &worker_cancelled)
                        .map_err(|error| error.to_string())?;
                let mut packages = Vec::new();
                for (_definition_id, request) in requests {
                    if worker_cancelled.load(Ordering::Acquire) {
                        return Err("exact evaluation cancelled".to_owned());
                    }
                    let package = match request {
                        ExactEvaluationRequest::Rectangle(request) => worker
                            .evaluate_rectangle_with_cancellation(&request, &worker_cancelled)
                            .map(ExactBodyPackage::from)
                            .map_err(|error| error.to_string())?,
                        ExactEvaluationRequest::Revolve(request) => worker
                            .evaluate_revolve_with_cancellation(&request, &worker_cancelled)
                            .map(ExactBodyPackage::from)
                            .map_err(|error| error.to_string())?,
                    };
                    packages.push(Arc::new(package));
                }
                Ok(packages)
            })();
            if !worker_cancelled.load(Ordering::Acquire) && sender.send(result).is_ok() {
                repaint.request_repaint();
            }
        });
        self.exact_task = Some(ExactEvaluationTask {
            source,
            cancelled,
            receiver,
        });
    }

    pub fn connect_exact_worker(&mut self, executable: impl AsRef<Path>) -> Result<(), String> {
        let executable = executable.as_ref();
        if !executable.is_file() {
            return Err("exact worker executable was not found".to_owned());
        }
        if let Some(task) = self.exact_task.take() {
            task.cancelled.store(true, Ordering::Release);
        }
        if let Some(task) = self.beam_m5_task.take() {
            task.cancelled.store(true, Ordering::Release);
        }
        self.exact_worker_path = Some(executable.to_owned());
        self.exact_worker_attempted = true;
        self.exact_results.clear();
        self.exact_source = None;
        self.exact_retry_at = None;
        self.beam_m5_products = None;
        self.beam_exact_results.clear();
        self.beam_m5_source = None;
        self.beam_m5_retry_at = None;
        Ok(())
    }

    #[doc(hidden)]
    pub fn enable_headless_instanced_scene(&mut self) {
        self.wgpu_target_format = Some(eframe::wgpu::TextureFormat::Bgra8UnormSrgb);
    }

    #[must_use]
    pub fn exact_render_body_count(&self) -> usize {
        let snapshot = self.document.current();
        self.exact_results
            .values()
            .filter(|package| package.is_current(&snapshot))
            .count()
    }

    #[must_use]
    pub fn exact_render_bounds(&self) -> Vec<[[f64; 3]; 2]> {
        let snapshot = self.document.current();
        self.exact_results
            .values()
            .filter(|package| package.is_current(&snapshot))
            .map(|package| package.bounds_mm())
            .collect()
    }

    #[must_use]
    pub fn exact_stable_reference_count(&self) -> usize {
        let snapshot = self.document.current();
        self.exact_results
            .values()
            .filter(|package| package.is_current(&snapshot))
            .map(|package| package.references().len())
            .sum()
    }

    fn exact_projection(&self, snapshot: &Snapshot) -> ExactInteractionProjection {
        ExactInteractionProjection::from_snapshot(snapshot, &self.exact_results)
    }

    fn refresh_interaction_projection_cache(&self, snapshot: &Snapshot) {
        let rebuild = self
            .interaction_projection_cache
            .borrow()
            .as_ref()
            .is_none_or(|cache| {
                cache.document_id != snapshot.document_id()
                    || cache.revision_id != snapshot.revision_id()
                    || cache.edit_context != self.selection.edit_context
                    || cache.exact_result_count != self.exact_results.len()
            });
        if rebuild {
            let active_context_paths = self
                .active_scene_query()
                .into_iter()
                .map(|occurrence| occurrence.instance_path)
                .collect::<BTreeSet<_>>();
            let canonical = CanonicalInteractionProjection::from_snapshot(snapshot);
            let exact = ExactInteractionProjection::from_snapshot_where(
                snapshot,
                &self.exact_results,
                |path| active_context_paths.contains(path),
            );
            let mesh = MeshInteractionProjection::from_snapshot_where(snapshot, |path| {
                active_context_paths.contains(path) && !exact.contains_occurrence(path)
            });
            let boxes = canonical
                .scene_where(|occurrence| {
                    active_context_paths.contains(&occurrence.instance_path)
                        && !exact.contains_occurrence(&occurrence.instance_path)
                        && !mesh.contains_occurrence(&occurrence.instance_path)
                        && occurrence.local_box.is_some()
                })
                .expect("canonical visible box projections are valid");
            let proxies = canonical
                .scene_where(|occurrence| {
                    active_context_paths.contains(&occurrence.instance_path)
                        && occurrence.local_box.is_some()
                })
                .expect("canonical visible proxy projections are valid");
            *self.interaction_projection_cache.borrow_mut() = Some(InteractionProjectionCache {
                document_id: snapshot.document_id(),
                revision_id: snapshot.revision_id(),
                edit_context: self.selection.edit_context.clone(),
                exact_result_count: self.exact_results.len(),
                canonical,
                exact,
                mesh,
                boxes,
                proxies,
            });
        }
    }

    #[must_use]
    pub fn exact_pick_durable(&self, ray: Ray) -> Option<AssemblySelectionTarget> {
        let snapshot = self.document.current();
        self.exact_projection(&snapshot)
            .exact_pick(ray)
            .map(|hit| hit.target)
    }

    fn active_boxes(&self) -> Vec<RenderBox> {
        let snapshot = self.document.current();
        self.refresh_interaction_projection_cache(&snapshot);
        let cache = self.interaction_projection_cache.borrow();
        self.render_boxes_from_projection(
            &cache
                .as_ref()
                .expect("interaction cache was built")
                .canonical,
            true,
        )
    }

    fn active_boxes_for_snapshot(&self, snapshot: &Snapshot) -> Vec<RenderBox> {
        let current = self.document.current();
        if snapshot.revision_id() == current.revision_id()
            && snapshot.canonical_digest() == current.canonical_digest()
        {
            return self.active_boxes();
        }
        self.render_boxes_from_projection(
            &CanonicalInteractionProjection::from_snapshot(snapshot),
            false,
        )
    }

    fn render_boxes_from_projection(
        &self,
        projection: &InteractionProjection,
        use_exact_bounds: bool,
    ) -> Vec<RenderBox> {
        projection
            .occurrences()
            .iter()
            .filter(|occurrence| occurrence.visible)
            .filter_map(|occurrence| {
                let box_proxy = occurrence.box_proxy?;
                let matrix = occurrence.canonical_world_transform.matrix();
                let translation_only = matrix[0] == 1.0
                    && matrix[1] == 0.0
                    && matrix[2] == 0.0
                    && matrix[4] == 0.0
                    && matrix[5] == 1.0
                    && matrix[6] == 0.0
                    && matrix[8] == 0.0
                    && matrix[9] == 0.0
                    && matrix[10] == 1.0
                    && matrix[12] == 0.0
                    && matrix[13] == 0.0
                    && matrix[14] == 0.0
                    && matrix[15] == 1.0;
                let exact_bounds = use_exact_bounds
                    .then(|| self.exact_results.get(&occurrence.body.definition_id))
                    .flatten()
                    .filter(|_| translation_only)
                    .map(|package| package.bounds_mm());
                let (origin_mm, size_mm) =
                    exact_bounds.map_or((box_proxy.origin_mm, box_proxy.size_mm), |[min, max]| {
                        (
                            box_proxy.origin_mm + Vec3::new(min[0], min[1], min[2]),
                            Vec3::new(max[0] - min[0], max[1] - min[1], max[2] - min[2]),
                        )
                    });
                Some(RenderBox {
                    definition_id: occurrence.body.definition_id,
                    profile_feature_id: occurrence.body.profile_feature_id?,
                    extrusion_feature_id: occurrence.body.extrusion_feature_id,
                    instance_path: occurrence.instance_path.clone(),
                    origin_mm,
                    size_mm,
                })
            })
            .collect()
    }

    #[must_use]
    pub fn active_box_count(&self) -> usize {
        self.document.current().scene_query().len()
    }

    /// Canonical definition referenced by an occurrence.
    #[must_use]
    pub fn occurrence_definition_id(&self, occurrence_id: OccurrenceId) -> Option<DefinitionId> {
        self.document
            .current()
            .occurrence(occurrence_id)
            .map(|occurrence| occurrence.definition_id())
    }

    /// Derived box origin and size for an occurrence in model millimetres.
    #[must_use]
    pub fn occurrence_box_geometry(&self, occurrence_id: u64) -> Option<(Vec3, Vec3)> {
        self.active_boxes()
            .into_iter()
            .find(|item| item.instance_path == InstancePath::root(OccurrenceId(occurrence_id)))
            .map(|item| (item.origin_mm, item.size_mm))
    }

    /// Where a world point currently lands inside the viewport.
    ///
    /// Callers that need to aim at real geometry must ask the camera instead of
    /// assuming a screen offset: under a converging projection the pixels per
    /// millimetre depend on how far the point is from the eye.
    #[must_use]
    pub fn project_to_screen(&self, point: Vec3, rect: Rect) -> Pos2 {
        self.project(point, rect)
    }

    /// How many occurrences are currently selected.
    #[must_use]
    pub fn selected_occurrence_count(&self) -> usize {
        self.selection.occurrences.len()
    }

    /// How many definitions the active document holds.
    #[must_use]
    pub fn definition_count(&self) -> usize {
        self.document.current().definitions().count()
    }

    #[must_use]
    pub fn mesh_body_count(&self) -> usize {
        self.document
            .current()
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::MeshBody(_)))
            .count()
    }

    #[must_use]
    pub fn import_receipt_count(&self) -> usize {
        self.document.current().import_receipts().count()
    }

    /// How many groups the active document holds.
    #[must_use]
    pub fn group_count(&self) -> usize {
        self.document.current().groups().count()
    }

    /// How deep the shell is inside group or component edit contexts.
    #[must_use]
    pub fn edit_context_depth(&self) -> usize {
        self.selection.edit_context.len()
    }

    fn selected_instance_paths(&self) -> BTreeSet<InstancePath> {
        let mut paths = self.selection.occurrences.clone();
        if let Some(primary) = &self.selection.primary {
            paths.insert(primary.instance_path.clone());
        }
        paths
    }

    fn selected_occurrence_ids(&self) -> BTreeSet<OccurrenceId> {
        if matches!(
            self.selection.edit_context.last(),
            Some(EditContext::Definition { .. })
        ) {
            return BTreeSet::new();
        }
        let paths = self.selected_instance_paths();
        if paths.iter().any(|path| !path.is_root()) {
            return BTreeSet::new();
        }
        paths
            .into_iter()
            .map(|path| path.root_occurrence())
            .collect()
    }

    fn selection_count(&self) -> usize {
        self.selected_instance_paths().len()
    }

    fn active_scene_query(&self) -> Vec<SceneOccurrence> {
        self.active_scene_query_for_snapshot(&self.document.current())
    }

    fn active_scene_query_for_snapshot(&self, snapshot: &Snapshot) -> Vec<SceneOccurrence> {
        let Some(context) = self.selection.edit_context.last() else {
            return snapshot
                .scene_query()
                .into_iter()
                .filter(|occurrence| occurrence.visible && occurrence.instance_path.is_root())
                .collect();
        };
        let context = match context {
            EditContext::Group(group_id) => SceneQueryContext::Group(*group_id),
            EditContext::Definition {
                definition_id,
                instance_path,
            } => SceneQueryContext::Definition {
                definition_id: *definition_id,
                instance_path: instance_path.clone(),
            },
        };
        snapshot
            .bind_scene_query(context)
            .and_then(|query| snapshot.scene_query_in(&query))
            .unwrap_or_default()
    }

    fn occurrence_in_active_context(&self, instance_path: &InstancePath) -> bool {
        self.active_scene_query()
            .into_iter()
            .any(|occurrence| occurrence.instance_path == *instance_path)
    }

    fn select_group(&mut self, group_id: GroupId) -> bool {
        let snapshot = self.document.current();
        let Some(group) = snapshot.group(group_id) else {
            return false;
        };
        let ids = self
            .active_scene_query()
            .into_iter()
            .filter(|occurrence| {
                occurrence.instance_path.is_root() && occurrence.parent == Some(group_id)
            })
            .map(|occurrence| occurrence.instance_path)
            .collect::<BTreeSet<_>>();
        if ids.is_empty() {
            return false;
        }
        let name = group.name().to_owned();
        self.selection.clear();
        self.selection.occurrences = ids;
        self.selection.selected_group = Some(group_id);
        self.digest = self.catalog.format(
            "digest-selected-group",
            &BTreeMap::from([
                ("name", name),
                ("count", self.selection_count().to_string()),
            ]),
        );
        true
    }

    fn enter_group_context(&mut self, group_id: GroupId) -> bool {
        if self.document.current().group(group_id).is_none() {
            return false;
        }
        self.selection
            .edit_context
            .push(EditContext::Group(group_id));
        self.selection.clear();
        self.digest = self.catalog.text("digest-entered-group-context");
        true
    }

    fn enter_occurrence_context(&mut self, instance_path: InstancePath) -> bool {
        let snapshot = self.document.current();
        let Ok(resolved) = snapshot.resolve_instance_path(&instance_path) else {
            return false;
        };
        if !self.occurrence_in_active_context(&instance_path) {
            return false;
        }
        let root = snapshot.occurrence(instance_path.root_occurrence());
        let context = match (self.selection.edit_context.last(), root) {
            (None, Some(occurrence))
                if instance_path.is_root() && occurrence.parent().is_some() =>
            {
                EditContext::Group(occurrence.parent().unwrap())
            }
            _ => EditContext::Definition {
                definition_id: resolved.definition_id,
                instance_path,
            },
        };
        self.selection.edit_context.push(context.clone());
        self.selection.clear();
        self.digest = self.catalog.text(match context {
            EditContext::Group(_) => "digest-entered-group-context",
            EditContext::Definition { .. } => "digest-entered-component-context",
        });
        true
    }

    fn exit_edit_context(&mut self) -> bool {
        if self.selection.edit_context.pop().is_none() {
            return false;
        }
        self.selection.clear();
        self.digest = self.catalog.text("digest-exited-edit-context");
        true
    }

    fn clear_selection(&mut self) {
        self.selection.clear();
        self.digest = self.catalog.text("digest-selection-cleared");
    }

    fn select_from_viewport(&mut self, target: Option<SelectionId>, additive: bool) {
        let Some(target) = target else {
            if !additive {
                self.clear_selection();
            }
            return;
        };
        let occurrence_id = target.instance_path.root_occurrence();
        if !self.occurrence_in_active_context(&target.instance_path) {
            return;
        }
        let snapshot = self.document.current();
        if self.selection.edit_context.is_empty()
            && target.instance_path.is_root()
            && let Some(group_id) = snapshot
                .occurrence(occurrence_id)
                .and_then(|occurrence| occurrence.parent())
        {
            self.select_group(group_id);
            return;
        }
        self.selection.select_exact(target.clone(), additive);
        if let Some(item) = snapshot
            .scene_query()
            .into_iter()
            .find(|item| item.instance_path == target.instance_path)
        {
            self.digest = self.catalog.format(
                "digest-selected-viewport",
                &BTreeMap::from([
                    ("name", item.definition_name),
                    ("count", item.shared_occurrence_count.to_string()),
                ]),
            );
        }
    }

    fn select_from_outliner(&mut self, instance_path: InstancePath, additive: bool) {
        if !self.occurrence_in_active_context(&instance_path) {
            return;
        }
        let snapshot = self.document.current();
        let root_id = instance_path.root_occurrence();
        if self.selection.edit_context.is_empty()
            && instance_path.is_root()
            && let Some(group_id) = snapshot
                .occurrence(root_id)
                .and_then(|occurrence| occurrence.parent())
        {
            self.select_group(group_id);
            return;
        }
        self.selection.select_path(instance_path.clone(), additive);
        if let Some(item) = snapshot
            .scene_query()
            .into_iter()
            .find(|item| item.instance_path == instance_path)
        {
            self.digest = self.catalog.format(
                "digest-selected-outliner",
                &BTreeMap::from([("name", item.occurrence_name)]),
            );
        }
    }

    fn select_definition(&mut self, definition_id: DefinitionId, additive: bool) {
        let snapshot = self.document.current();
        let Some(definition) = snapshot.definition(definition_id) else {
            return;
        };
        let ids = self
            .active_scene_query()
            .into_iter()
            .filter(|item| item.definition_id == definition_id)
            .map(|item| item.instance_path)
            .collect::<Vec<_>>();
        let name = definition.name().to_owned();
        if !additive {
            self.selection.clear();
        }
        self.selection.occurrences.extend(ids.iter().cloned());
        self.digest = self.catalog.format(
            "digest-selected-definition",
            &BTreeMap::from([("name", name), ("count", ids.len().to_string())]),
        );
    }

    fn select_all(&mut self) {
        let ids = self
            .active_scene_query()
            .into_iter()
            .map(|item| item.instance_path)
            .collect::<Vec<_>>();
        self.selection.clear();
        self.selection.occurrences.extend(ids);
        self.digest = self.catalog.format(
            "digest-selected-all",
            &BTreeMap::from([("count", self.selection_count().to_string())]),
        );
    }

    fn outliner_query(&self) -> Vec<OutlinerDefinition> {
        let snapshot = self.document.current();
        let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
        let scoped = !self.selection.edit_context.is_empty();
        let scene = if scoped {
            self.active_scene_query()
        } else {
            snapshot.scene_query()
        };
        snapshot
            .definitions()
            .filter(|definition| {
                !scoped
                    || scene
                        .iter()
                        .any(|occurrence| occurrence.definition_id == definition.id())
            })
            .map(|definition| {
                let size = projection
                    .occurrences()
                    .iter()
                    .find(|occurrence| occurrence.body.definition_id == definition.id())
                    .and_then(|occurrence| occurrence.local_box.map(|local_box| local_box.size_mm))
                    .unwrap_or(Vec3::ZERO);
                let occurrences = scene
                    .iter()
                    .filter(|item| item.definition_id == definition.id())
                    .map(|item| {
                        #[cfg(test)]
                        let matrix = item.transform.matrix();
                        OutlinerOccurrence {
                            instance_path: item.instance_path.clone(),
                            name: item.occurrence_name.clone(),
                            #[cfg(test)]
                            position: format!(
                                "{},{}",
                                format_height(matrix[3]),
                                format_height(matrix[7])
                            ),
                            visible: item.visible,
                            parent: item.parent,
                        }
                    })
                    .collect();
                OutlinerDefinition {
                    id: definition.id(),
                    name: definition.name().to_owned(),
                    specification: format!(
                        "{} × {} × {}",
                        format_height(size.x),
                        format_height(size.y),
                        format_height(size.z)
                    ),
                    occurrences,
                }
            })
            .collect()
    }

    fn outliner_groups(&self) -> Vec<OutlinerGroup> {
        let snapshot = self.document.current();
        snapshot
            .groups()
            .map(|group| OutlinerGroup {
                id: group.id(),
                name: group.name().to_owned(),
                member_count: snapshot
                    .occurrences()
                    .filter(|occurrence| occurrence.parent() == Some(group.id()))
                    .count(),
            })
            .collect()
    }

    fn selection_has_common_parent(&self) -> bool {
        let snapshot = self.document.current();
        let mut ids = self.selected_occurrence_ids().into_iter();
        let Some(first_id) = ids.next() else {
            return false;
        };
        let Some(parent) = snapshot
            .occurrence(first_id)
            .map(|occurrence| occurrence.parent())
        else {
            return false;
        };
        ids.all(|id| {
            snapshot
                .occurrence(id)
                .is_some_and(|item| item.parent() == parent)
        })
    }

    fn selected_group_id(&self) -> Option<GroupId> {
        if let Some(group_id) = self.selection.selected_group {
            return Some(group_id);
        }
        let snapshot = self.document.current();
        let mut ids = self.selected_occurrence_ids().into_iter();
        let first = snapshot.occurrence(ids.next()?)?.parent()?;
        ids.all(|id| {
            snapshot
                .occurrence(id)
                .is_some_and(|occurrence| occurrence.parent() == Some(first))
        })
        .then_some(first)
    }

    fn selected_shared_occurrence_count(&self) -> usize {
        let Some(occurrence_id) = self.selected_occurrence_ids().into_iter().next() else {
            return 0;
        };
        self.document
            .current()
            .scene_query()
            .into_iter()
            .find(|item| item.instance_path == InstancePath::root(occurrence_id))
            .map_or(0, |item| item.shared_occurrence_count)
    }

    fn through_cut_target(&self) -> Option<(SelectionId, RenderBox, Vec3)> {
        let selection = self.selection.primary.clone()?;
        if selection.element
            != (ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            })
        {
            return None;
        }
        let item = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)?;
        item.extrusion_feature_id?;
        let snapshot = self.document.current();
        ExactFeatureChainRequest::from_snapshot(&snapshot, selection.definition_id).ok()?;
        let resolved = snapshot
            .resolve_instance_path(&selection.instance_path)
            .ok()?;
        if resolved.definition_id != selection.definition_id {
            return None;
        }
        let matrix = resolved.world_transform.matrix();
        if matrix[0] != 1.0
            || matrix[1] != 0.0
            || matrix[2] != 0.0
            || matrix[4] != 0.0
            || matrix[5] != 1.0
            || matrix[6] != 0.0
            || matrix[8] != 0.0
            || matrix[9] != 0.0
            || matrix[10] != 1.0
        {
            return None;
        }
        let definition = snapshot.definition(selection.definition_id)?;
        if definition.feature_ids().iter().any(|id| {
            snapshot.feature(*id).is_some_and(|feature| {
                matches!(
                    feature.kind(),
                    FeatureKind::ThroughCut { .. }
                        | FeatureKind::Pocket { .. }
                        | FeatureKind::Boolean { .. }
                )
            })
        }) {
            return None;
        }
        Some((selection, item, Vec3::new(matrix[3], matrix[7], matrix[11])))
    }

    fn selected_revolve_profile(&self) -> Option<RevolveToolState> {
        let selection = self.selection.primary.as_ref()?;
        let item = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)?;
        if item.extrusion_feature_id.is_some() || !item.instance_path.is_root() {
            return None;
        }
        let snapshot = self.document.current();
        let profile = snapshot.feature(item.profile_feature_id)?;
        let is_closed = match profile.kind() {
            FeatureKind::Profile { points_mm } => points_mm.len() >= 3,
            FeatureKind::SegmentProfile { closed, .. } => *closed,
            _ => false,
        };
        if !is_closed || profile.definition_id() != selection.definition_id {
            return None;
        }
        let resolved = snapshot.resolve_instance_path(&item.instance_path).ok()?;
        let matrix = resolved.world_transform.matrix();
        if matrix[0] != 1.0
            || matrix[1] != 0.0
            || matrix[2] != 0.0
            || matrix[4] != 0.0
            || matrix[5] != 1.0
            || matrix[6] != 0.0
            || matrix[8] != 0.0
            || matrix[9] != 0.0
            || matrix[10] != 1.0
        {
            return None;
        }
        let translation_mm = Vec3::new(matrix[3], matrix[7], matrix[11]);
        Some(RevolveToolState {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            definition_id: selection.definition_id,
            profile_feature_id: item.profile_feature_id,
            translation_mm,
            plane_z: translation_mm.z,
            axis_start_mm: None,
            axis_end_mm: None,
        })
    }

    fn begin_revolve_tool(&mut self) -> bool {
        let Some(tool) = self.selected_revolve_profile() else {
            return false;
        };
        self.revolve_tool = Some(tool);
        self.revolve_preview = None;
        self.value_input = "360".to_owned();
        self.status_key = "status-revolve-axis-start";
        true
    }

    fn add_revolve_axis_point(&mut self, point_mm: Vec3) -> bool {
        let Some(mut tool) = self.revolve_tool.clone() else {
            return false;
        };
        let snapshot = self.document.current();
        if snapshot.revision_id() != tool.source_revision
            || snapshot.canonical_digest() != tool.source_digest
        {
            self.revolve_tool = None;
            self.revolve_preview = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let local = [
            point_mm.x - tool.translation_mm.x,
            point_mm.y - tool.translation_mm.y,
        ];
        if let Some(start) = tool.axis_start_mm {
            if (local[0] - start[0]).hypot(local[1] - start[1]) <= 0.01 {
                return false;
            }
            tool.axis_end_mm = Some(local);
            self.status_key = "status-revolve-angle";
        } else {
            tool.axis_start_mm = Some(local);
            self.status_key = "status-revolve-axis-end";
        }
        self.revolve_tool = Some(tool);
        self.refresh_revolve_preview()
    }

    fn refresh_revolve_preview(&mut self) -> bool {
        self.revolve_preview = None;
        let Some(tool) = self.revolve_tool.as_ref() else {
            return false;
        };
        let Some(axis_start_mm) = tool.axis_start_mm else {
            return false;
        };
        let Some(axis_end_mm) = tool.axis_end_mm else {
            return false;
        };
        let Some(angle_degrees) =
            parse_distance_mm(&self.value_input).filter(|angle| *angle > 0.0 && *angle <= 360.0)
        else {
            self.digest = self.catalog.text("digest-revolve-invalid-angle");
            return false;
        };
        let snapshot = self.document.current();
        if snapshot.revision_id() != tool.source_revision
            || snapshot.canonical_digest() != tool.source_digest
        {
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(feature_id) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)
        else {
            return false;
        };
        let batch = CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id: tool.definition_id,
            name: self.catalog.text("model-revolve-feature"),
            kind: FeatureKind::Revolve {
                profile: tool.profile_feature_id,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            },
        }]);
        let Ok(preview_snapshot) = self.document.preview_batch(&batch) else {
            return false;
        };
        if ExactRevolveRequest::from_snapshot(&preview_snapshot, tool.definition_id).is_err() {
            return false;
        }
        let command_digest = batch.digest();
        self.revolve_preview = Some(RevolvePreview {
            source_revision: tool.source_revision,
            source_digest: tool.source_digest.clone(),
            definition_id: tool.definition_id,
            profile_feature_id: tool.profile_feature_id,
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            command_digest,
            batch,
        });
        self.status_key = "status-revolve-preview";
        self.digest = self.catalog.format(
            "digest-revolve-live",
            &BTreeMap::from([("angle", format_height(angle_degrees))]),
        );
        true
    }

    #[must_use]
    pub fn has_revolve_preview(&self) -> bool {
        let Some(preview) = self.revolve_preview.as_ref() else {
            return false;
        };
        let snapshot = self.document.current();
        preview.source_revision == snapshot.revision_id()
            && preview.source_digest == snapshot.canonical_digest()
            && preview.command_digest == preview.batch.digest()
            && self
                .document
                .preview_batch(&preview.batch)
                .ok()
                .and_then(|snapshot| {
                    ExactRevolveRequest::from_snapshot(&snapshot, preview.definition_id).ok()
                })
                .is_some()
    }

    #[must_use]
    pub fn revolve_preview_parameters(&self) -> Option<([f64; 2], [f64; 2], f64)> {
        self.has_revolve_preview().then(|| {
            let preview = self
                .revolve_preview
                .as_ref()
                .expect("a current Revolve preview exists");
            (
                preview.axis_start_mm,
                preview.axis_end_mm,
                preview.angle_degrees,
            )
        })
    }

    #[must_use]
    pub fn latest_revolve_parameters(&self) -> Option<(FeatureId, [f64; 2], [f64; 2], f64)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::Revolve {
                    axis_start_mm,
                    axis_end_mm,
                    angle_degrees,
                    ..
                } = feature.kind()
                else {
                    return None;
                };
                Some((feature.id(), *axis_start_mm, *axis_end_mm, *angle_degrees))
            })
            .last()
    }

    fn confirm_revolve_preview(&mut self) -> bool {
        if !self.has_revolve_preview() {
            self.revolve_preview = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.revolve_preview.take() else {
            return false;
        };
        if self.document.apply_batch(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.value_input.clear();
        self.status_key = "status-ready";
        self.digest = self.catalog.format(
            "digest-revolve-committed",
            &BTreeMap::from([("angle", format_height(preview.angle_degrees))]),
        );
        true
    }

    fn selected_planar_offset_profile(&self) -> Option<(DefinitionId, FeatureId)> {
        let selection = self.selection.primary.as_ref()?;
        if selection.element
            != (ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            })
            || !selection.instance_path.is_root()
        {
            return None;
        }
        let item = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)?;
        if item.extrusion_feature_id.is_some() {
            return None;
        }
        let snapshot = self.document.current();
        let definition = snapshot.definition(selection.definition_id)?;
        if definition.feature_ids() != [item.profile_feature_id] {
            return None;
        }
        let FeatureKind::Profile { points_mm } = snapshot.feature(item.profile_feature_id)?.kind()
        else {
            return None;
        };
        let [south_west, south_east, north_east, north_west] = points_mm.as_slice() else {
            return None;
        };
        (south_west[1] == south_east[1]
            && south_east[0] == north_east[0]
            && north_east[1] == north_west[1]
            && north_west[0] == south_west[0]
            && south_east[0] > south_west[0]
            && north_west[1] > south_west[1])
            .then_some((selection.definition_id, item.profile_feature_id))
    }

    fn refresh_planar_offset_preview(&mut self) -> bool {
        self.planar_offset_preview = None;
        let Some((definition_id, profile_feature_id)) = self.selected_planar_offset_profile()
        else {
            return false;
        };
        let Some(distance_mm) =
            parse_distance_mm(&self.value_input).filter(|distance| distance.abs() > 1.0e-6)
        else {
            self.digest = self.catalog.text("digest-planar-offset-invalid-distance");
            return false;
        };
        let Ok(distance) = Dimension::new(self.value_input.clone(), distance_mm) else {
            return false;
        };
        let snapshot = self.document.current();
        let source_revision = snapshot.revision_id();
        let source_digest = snapshot.canonical_digest();
        let Some(feature_id) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)
        else {
            return false;
        };
        let batch = CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id,
            name: self.catalog.text("model-planar-offset-feature"),
            kind: FeatureKind::PlanarOffset {
                profile: profile_feature_id,
                distance,
            },
        }]);
        let Ok(preview_snapshot) = self.document.preview_batch(&batch) else {
            self.digest = self.catalog.text("digest-planar-offset-invalid-distance");
            return false;
        };
        let Ok(request) = ExactPlanarOffsetRequest::from_snapshot(&preview_snapshot, definition_id)
        else {
            return false;
        };
        self.planar_offset_preview = Some(PlanarOffsetPreview {
            source_revision,
            source_digest,
            definition_id,
            profile_feature_id,
            distance_mm,
            bounds_mm: request.expected_bounds_mm(),
            command_digest: batch.digest(),
            batch,
        });
        self.status_key = "status-planar-offset-preview";
        self.digest = self.catalog.format(
            "digest-planar-offset-live",
            &BTreeMap::from([("distance", format_height(distance_mm))]),
        );
        true
    }

    #[must_use]
    pub fn planar_offset_preview_parameters(&self) -> Option<(FeatureId, f64, [[f64; 3]; 2])> {
        let preview = self.planar_offset_preview.as_ref()?;
        self.planar_offset_preview_is_current().then_some((
            preview.profile_feature_id,
            preview.distance_mm,
            preview.bounds_mm,
        ))
    }

    #[must_use]
    pub fn planar_offset_preview_exact_evaluator(&self) -> Option<&'static str> {
        let preview = self.planar_offset_preview.as_ref()?;
        let snapshot = self.document.preview_batch(&preview.batch).ok()?;
        ExactPlanarOffsetRequest::from_snapshot(&snapshot, preview.definition_id)
            .ok()
            .map(|request| request.evaluator())
    }

    #[must_use]
    pub fn planar_offset_preview_is_current(&self) -> bool {
        let Some(preview) = self.planar_offset_preview.as_ref() else {
            return false;
        };
        let snapshot = self.document.current();
        preview.source_revision == snapshot.revision_id()
            && preview.source_digest == snapshot.canonical_digest()
            && preview.command_digest == preview.batch.digest()
            && self
                .document
                .preview_batch(&preview.batch)
                .ok()
                .and_then(|snapshot| {
                    ExactPlanarOffsetRequest::from_snapshot(&snapshot, preview.definition_id).ok()
                })
                .is_some()
    }

    #[must_use]
    pub fn latest_planar_offset_parameters(&self) -> Option<(FeatureId, FeatureId, f64)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::PlanarOffset { profile, distance } = feature.kind() else {
                    return None;
                };
                Some((feature.id(), *profile, distance.millimetres()))
            })
            .last()
    }

    fn confirm_planar_offset_preview(&mut self) -> bool {
        if !self.planar_offset_preview_is_current() {
            self.planar_offset_preview = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.planar_offset_preview.take() else {
            return false;
        };
        if self.document.apply_batch(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = ActiveTool::Select;
        self.value_input.clear();
        self.status_key = "status-ready";
        self.digest = self.catalog.format(
            "digest-planar-offset-committed",
            &BTreeMap::from([("distance", format_height(preview.distance_mm))]),
        );
        true
    }

    fn sweep_preview_candidate(
        &self,
    ) -> Option<(
        DefinitionId,
        FeatureId,
        FeatureId,
        ExactSweepRequest,
        CommandBatch,
    )> {
        let selection = self.selection.primary.as_ref()?;
        if selection.element
            != (ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            })
            || !selection.instance_path.is_root()
        {
            return None;
        }
        let snapshot = self.document.current();
        let definition = snapshot.definition(selection.definition_id)?;
        let [profile_feature_id, path_feature_id] = definition.feature_ids() else {
            return None;
        };
        let feature_id = FeatureId(
            snapshot
                .features()
                .map(|feature| feature.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let batch = CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id: selection.definition_id,
            name: self.catalog.text("model-sweep-feature"),
            kind: FeatureKind::Sweep {
                profile: *profile_feature_id,
                path: *path_feature_id,
            },
        }]);
        let preview_snapshot = self.document.preview_batch(&batch).ok()?;
        let request =
            ExactSweepRequest::from_snapshot(&preview_snapshot, selection.definition_id).ok()?;
        Some((
            selection.definition_id,
            *profile_feature_id,
            *path_feature_id,
            request,
            batch,
        ))
    }

    fn refresh_sweep_preview(&mut self) -> bool {
        self.sweep_preview = None;
        let Some((definition_id, profile_feature_id, path_feature_id, request, batch)) =
            self.sweep_preview_candidate()
        else {
            self.digest = self.catalog.text("digest-sweep-invalid-inputs");
            return false;
        };
        let snapshot = self.document.current();
        self.sweep_preview = Some(SweepPreview {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            definition_id,
            profile_feature_id,
            path_feature_id,
            bounds_mm: request.expected_bounds_mm(),
            volume_mm3: request.expected_volume_mm3(),
            command_digest: batch.digest(),
            batch,
        });
        self.status_key = "status-sweep-preview";
        self.digest = self.catalog.text("digest-sweep-live");
        true
    }

    #[must_use]
    pub fn sweep_preview_parameters(&self) -> Option<(FeatureId, FeatureId, [[f64; 3]; 2], f64)> {
        let preview = self.sweep_preview.as_ref()?;
        self.sweep_preview_is_current().then_some((
            preview.profile_feature_id,
            preview.path_feature_id,
            preview.bounds_mm,
            preview.volume_mm3,
        ))
    }

    #[must_use]
    pub fn sweep_preview_exact_evaluator(&self) -> Option<&'static str> {
        let preview = self.sweep_preview.as_ref()?;
        let snapshot = self.document.preview_batch(&preview.batch).ok()?;
        ExactSweepRequest::from_snapshot(&snapshot, preview.definition_id)
            .ok()
            .map(|request| request.evaluator())
    }

    #[must_use]
    pub fn sweep_preview_is_current(&self) -> bool {
        let Some(preview) = self.sweep_preview.as_ref() else {
            return false;
        };
        let snapshot = self.document.current();
        preview.source_revision == snapshot.revision_id()
            && preview.source_digest == snapshot.canonical_digest()
            && preview.command_digest == preview.batch.digest()
            && self
                .document
                .preview_batch(&preview.batch)
                .ok()
                .and_then(|snapshot| {
                    ExactSweepRequest::from_snapshot(&snapshot, preview.definition_id).ok()
                })
                .is_some()
    }

    #[must_use]
    pub fn latest_sweep_parameters(&self) -> Option<(FeatureId, FeatureId, FeatureId)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::Sweep { profile, path } = feature.kind() else {
                    return None;
                };
                Some((feature.id(), *profile, *path))
            })
            .last()
    }

    fn confirm_sweep_preview(&mut self) -> bool {
        if !self.sweep_preview_is_current() {
            self.sweep_preview = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.sweep_preview.take() else {
            return false;
        };
        if self.document.apply_batch(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = ActiveTool::Select;
        self.status_key = "status-ready";
        self.digest = self.catalog.text("digest-sweep-committed");
        true
    }

    fn loft_preview_candidate(
        &self,
    ) -> Option<(
        DefinitionId,
        Vec<LoftSection>,
        ExactLoftRequest,
        CommandBatch,
    )> {
        let selection = self.selection.primary.as_ref()?;
        if selection.element
            != (ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            })
            || !selection.instance_path.is_root()
        {
            return None;
        }
        let (definition_id, sections) = self.loft_input_sections.as_ref()?;
        if selection.definition_id != *definition_id {
            return None;
        }
        let snapshot = self.document.current();
        let definition = snapshot.definition(*definition_id)?;
        if !definition
            .feature_ids()
            .iter()
            .copied()
            .eq(sections.iter().map(|section| section.profile))
        {
            return None;
        }
        let feature_id = FeatureId(
            snapshot
                .features()
                .map(|feature| feature.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let batch = CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id: *definition_id,
            name: self.catalog.text("model-loft-feature"),
            kind: FeatureKind::Loft {
                sections: sections.clone(),
            },
        }]);
        let preview_snapshot = self.document.preview_batch(&batch).ok()?;
        let request = ExactLoftRequest::from_snapshot(&preview_snapshot, *definition_id).ok()?;
        Some((*definition_id, sections.clone(), request, batch))
    }

    fn refresh_loft_preview(&mut self) -> bool {
        self.loft_preview = None;
        let Some((definition_id, sections, request, batch)) = self.loft_preview_candidate() else {
            self.digest = self.catalog.text("digest-loft-invalid-inputs");
            return false;
        };
        let mut minimum = [f64::INFINITY; 3];
        let mut maximum = [f64::NEG_INFINITY; 3];
        for section in &request.sections {
            let elevation = f64::from_bits(section.elevation_bits);
            minimum[2] = minimum[2].min(elevation);
            maximum[2] = maximum[2].max(elevation);
            for point in &section.control_point_bits {
                for axis in 0..2 {
                    let coordinate = f64::from_bits(point[axis]);
                    minimum[axis] = minimum[axis].min(coordinate);
                    maximum[axis] = maximum[axis].max(coordinate);
                }
            }
        }
        let snapshot = self.document.current();
        self.loft_preview = Some(LoftPreview {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            definition_id,
            sections,
            bounds_mm: [minimum, maximum],
            control_point_count: request.control_point_count(),
            command_digest: batch.digest(),
            batch,
        });
        self.status_key = "status-loft-preview";
        self.digest = self.catalog.text("digest-loft-live");
        true
    }

    #[must_use]
    pub fn loft_preview_parameters(&self) -> Option<LoftPreviewParameters> {
        let preview = self.loft_preview.as_ref()?;
        self.loft_preview_is_current().then(|| {
            (
                preview
                    .sections
                    .iter()
                    .map(|section| (section.profile, section.elevation_mm))
                    .collect(),
                preview.bounds_mm,
                preview.control_point_count,
            )
        })
    }

    #[must_use]
    pub fn loft_preview_exact_evaluator(&self) -> Option<&'static str> {
        let preview = self.loft_preview.as_ref()?;
        let snapshot = self.document.preview_batch(&preview.batch).ok()?;
        ExactLoftRequest::from_snapshot(&snapshot, preview.definition_id)
            .ok()
            .map(|request| request.evaluator())
    }

    #[must_use]
    pub fn loft_preview_is_current(&self) -> bool {
        let Some(preview) = self.loft_preview.as_ref() else {
            return false;
        };
        let snapshot = self.document.current();
        preview.source_revision == snapshot.revision_id()
            && preview.source_digest == snapshot.canonical_digest()
            && preview.command_digest == preview.batch.digest()
            && self
                .document
                .preview_batch(&preview.batch)
                .ok()
                .and_then(|snapshot| {
                    ExactLoftRequest::from_snapshot(&snapshot, preview.definition_id).ok()
                })
                .is_some()
    }

    #[must_use]
    pub fn latest_loft_parameters(&self) -> Option<(FeatureId, Vec<(FeatureId, f64)>)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::Loft { sections } = feature.kind() else {
                    return None;
                };
                Some((
                    feature.id(),
                    sections
                        .iter()
                        .map(|section| (section.profile, section.elevation_mm))
                        .collect(),
                ))
            })
            .last()
    }

    fn confirm_loft_preview(&mut self) -> bool {
        if !self.loft_preview_is_current() {
            self.loft_preview = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.loft_preview.take() else {
            return false;
        };
        if self.document.apply_batch(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = ActiveTool::Select;
        self.status_key = "status-ready";
        self.digest = self.catalog.text("digest-loft-committed");
        true
    }

    fn selected_general_shell_target(&self) -> Option<(DefinitionId, FeatureId, StableFaceRole)> {
        let selection = self.selection.primary.as_ref()?;
        if selection.element
            != (ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            })
            || !selection.instance_path.is_root()
        {
            return None;
        }
        let item = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)?;
        let extrusion = item.extrusion_feature_id?;
        let snapshot = self.document.current();
        let request =
            ExactFeatureChainRequest::from_snapshot(&snapshot, selection.definition_id).ok()?;
        if request.shell.is_some() {
            return None;
        }
        Some((
            selection.definition_id,
            extrusion,
            StableFaceRole::new("extrusion.top").expect("the built-in top face role is valid"),
        ))
    }

    fn selected_general_edge_finish_target(
        &self,
    ) -> Option<(DefinitionId, FeatureId, StableEdgeRole)> {
        let selection = self.selection.primary.as_ref()?;
        if !matches!(
            selection.element,
            ElementId::Edge(7)
                | ElementId::EdgeMidpoint(7)
                | ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                }
        ) || !selection.instance_path.is_root()
        {
            return None;
        }
        let snapshot = self.document.current();
        let request =
            ExactFeatureChainRequest::from_snapshot(&snapshot, selection.definition_id).ok()?;
        let shell = request.shell?;
        if shell.edge_finish_feature_id.is_some() {
            return None;
        }
        Some((
            selection.definition_id,
            shell.shell_feature_id,
            StableEdgeRole::new("shell.edge.top-east")
                .expect("the built-in top-east edge role is valid"),
        ))
    }

    fn refresh_general_finish_preview(&mut self) -> bool {
        self.general_finish_preview = None;
        let kind = match self.active_tool {
            ActiveTool::Shell => GeneralFinishKind::Shell,
            ActiveTool::Fillet => GeneralFinishKind::Fillet,
            ActiveTool::Chamfer => GeneralFinishKind::Chamfer,
            _ => return false,
        };
        let Some(amount_mm) = parse_distance_mm(&self.value_input).filter(|amount| *amount > 0.0)
        else {
            self.digest = self.catalog.text("digest-general-finish-invalid-amount");
            return false;
        };
        let Ok(dimension) = Dimension::new(self.value_input.clone(), amount_mm) else {
            return false;
        };
        let snapshot = self.document.current();
        let source_revision = snapshot.revision_id();
        let source_digest = snapshot.canonical_digest();
        let Some(feature_id) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)
        else {
            return false;
        };
        let (definition_id, target_feature_id, stable_role, feature_kind, name_key) = match kind {
            GeneralFinishKind::Shell => {
                let Some((definition_id, target, role)) = self.selected_general_shell_target()
                else {
                    return false;
                };
                (
                    definition_id,
                    target,
                    role.as_str().to_owned(),
                    FeatureKind::Shell {
                        target,
                        removed_faces: vec![role],
                        thickness: dimension.clone(),
                    },
                    "model-shell-feature",
                )
            }
            GeneralFinishKind::Fillet | GeneralFinishKind::Chamfer => {
                let Some((definition_id, target, role)) =
                    self.selected_general_edge_finish_target()
                else {
                    return false;
                };
                (
                    definition_id,
                    target,
                    role.as_str().to_owned(),
                    FeatureKind::BottleEdgeFinish {
                        target,
                        edges: vec![role],
                        kind: if kind == GeneralFinishKind::Fillet {
                            BottleEdgeFinishKind::Fillet
                        } else {
                            BottleEdgeFinishKind::Chamfer
                        },
                        amount: dimension.clone(),
                    },
                    if kind == GeneralFinishKind::Fillet {
                        "model-fillet-feature"
                    } else {
                        "model-chamfer-feature"
                    },
                )
            }
        };
        let batch = CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id,
            name: self.catalog.text(name_key),
            kind: feature_kind,
        }]);
        let Ok(preview_snapshot) = self.document.preview_batch(&batch) else {
            return false;
        };
        if ExactFeatureChainRequest::from_snapshot(&preview_snapshot, definition_id).is_err() {
            return false;
        }
        self.general_finish_preview = Some(GeneralFinishPreview {
            source_revision,
            source_digest,
            definition_id,
            target_feature_id,
            stable_role,
            kind,
            amount_mm,
            command_digest: batch.digest(),
            batch,
        });
        self.status_key = "status-general-finish-preview";
        self.digest = self.catalog.format(
            "digest-general-finish-live",
            &BTreeMap::from([("amount", format_height(amount_mm))]),
        );
        true
    }

    #[must_use]
    pub fn general_finish_preview_parameters(
        &self,
    ) -> Option<(FeatureId, String, GeneralFinishKind, f64)> {
        let preview = self.general_finish_preview.as_ref()?;
        self.general_finish_preview_is_current().then(|| {
            (
                preview.target_feature_id,
                preview.stable_role.clone(),
                preview.kind,
                preview.amount_mm,
            )
        })
    }

    #[must_use]
    pub fn general_finish_preview_is_current(&self) -> bool {
        let Some(preview) = self.general_finish_preview.as_ref() else {
            return false;
        };
        let snapshot = self.document.current();
        preview.source_revision == snapshot.revision_id()
            && preview.source_digest == snapshot.canonical_digest()
            && preview.command_digest == preview.batch.digest()
            && self
                .document
                .preview_batch(&preview.batch)
                .ok()
                .and_then(|snapshot| {
                    ExactFeatureChainRequest::from_snapshot(&snapshot, preview.definition_id).ok()
                })
                .is_some()
    }

    #[must_use]
    pub fn latest_general_shell_parameters(&self) -> Option<(FeatureId, String, f64)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::Shell {
                    removed_faces,
                    thickness,
                    ..
                } = feature.kind()
                else {
                    return None;
                };
                Some((
                    feature.id(),
                    removed_faces.first()?.as_str().to_owned(),
                    thickness.millimetres(),
                ))
            })
            .last()
    }

    #[must_use]
    pub fn latest_general_edge_finish_parameters(
        &self,
    ) -> Option<(FeatureId, String, BottleEdgeFinishKind, f64)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::BottleEdgeFinish {
                    edges,
                    kind,
                    amount,
                    ..
                } = feature.kind()
                else {
                    return None;
                };
                Some((
                    feature.id(),
                    edges.first()?.as_str().to_owned(),
                    *kind,
                    amount.millimetres(),
                ))
            })
            .last()
    }

    fn confirm_general_finish_preview(&mut self) -> bool {
        if !self.general_finish_preview_is_current() {
            self.general_finish_preview = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.general_finish_preview.take() else {
            return false;
        };
        if self.document.apply_batch(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = ActiveTool::Select;
        self.value_input.clear();
        self.status_key = "status-ready";
        self.digest = self.catalog.format(
            "digest-general-finish-committed",
            &BTreeMap::from([("amount", format_height(preview.amount_mm))]),
        );
        true
    }

    fn command_enabled(&self, id: AppCommand) -> bool {
        let spec = CommandRegistry::spec(id);
        spec.implemented
            && match id {
                AppCommand::Undo => self.can_undo(),
                AppCommand::Redo => self.can_redo(),
                AppCommand::Copy => self.selection_count() > 0,
                AppCommand::Paste => !self.occurrence_clipboard.is_empty(),
                AppCommand::Delete | AppCommand::Deselect => self.selection_count() > 0,
                AppCommand::SelectAll => self.active_box_count() > 0,
                AppCommand::CutThrough | AppCommand::Pocket => self.through_cut_target().is_some(),
                AppCommand::PlanarOffset => self.selected_planar_offset_profile().is_some(),
                AppCommand::Sweep => self.sweep_preview_candidate().is_some(),
                AppCommand::Loft => self.loft_preview_candidate().is_some(),
                AppCommand::Revolve => self.selected_revolve_profile().is_some(),
                AppCommand::Shell => self.selected_general_shell_target().is_some(),
                AppCommand::Fillet | AppCommand::Chamfer => {
                    self.selected_general_edge_finish_target().is_some()
                }
                AppCommand::SolidSubtract
                | AppCommand::SolidUnion
                | AppCommand::SolidIntersect
                | AppCommand::SolidSplit => {
                    self.active_boxes()
                        .into_iter()
                        .filter(|item| {
                            item.instance_path.is_root() && item.extrusion_feature_id.is_some()
                        })
                        .count()
                        >= 2
                }
                AppCommand::Group => {
                    self.selection_count() >= 2
                        && self.selection.selected_group.is_none()
                        && self.selection_has_common_parent()
                }
                AppCommand::Ungroup => self.selected_group_id().is_some(),
                AppCommand::MakeComponent => self.selected_group_id().is_some(),
                AppCommand::MakeUnique => self.selected_shared_occurrence_count() > 1,
                AppCommand::Hide => self.selected_occurrences_with_visibility(true) > 0,
                AppCommand::Unhide => self.selected_occurrences_with_visibility(false) > 0,
                _ => true,
            }
    }

    fn dispatch_command(&mut self, id: AppCommand) {
        if !self.command_enabled(id) {
            return;
        }
        let spec = CommandRegistry::spec(id);
        if let Some(tool) = spec.tool {
            self.clear_ephemeral_edit_state();
            self.cancel_rectangle_sketch();
            self.active_tool = tool;
            self.value_input.clear();
            if tool == ActiveTool::PlanarOffset {
                self.value_input = "5".to_owned();
                self.refresh_planar_offset_preview();
            } else if tool == ActiveTool::Sweep {
                self.refresh_sweep_preview();
            } else if tool == ActiveTool::Loft {
                self.refresh_loft_preview();
            } else if tool == ActiveTool::Revolve {
                self.begin_revolve_tool();
            } else if matches!(
                tool,
                ActiveTool::Shell | ActiveTool::Fillet | ActiveTool::Chamfer
            ) {
                self.value_input = if tool == ActiveTool::Shell {
                    "2".to_owned()
                } else {
                    "1".to_owned()
                };
                self.refresh_general_finish_preview();
            } else if matches!(
                tool,
                ActiveTool::Rectangle
                    | ActiveTool::Circle
                    | ActiveTool::Arc
                    | ActiveTool::CutThrough
                    | ActiveTool::Pocket
            ) {
                self.sketch_mode = true;
                self.status_key = match tool {
                    ActiveTool::Circle => "status-circle-center",
                    ActiveTool::Arc => "status-arc-start",
                    ActiveTool::CutThrough => "status-cut-through-first-point",
                    ActiveTool::Pocket => "status-pocket-first-point",
                    _ => "status-sketch-first-point",
                };
            } else if tool == ActiveTool::Measure {
                self.status_key = "status-measure-first-point";
            } else if matches!(
                tool,
                ActiveTool::SolidSubtract
                    | ActiveTool::SolidUnion
                    | ActiveTool::SolidIntersect
                    | ActiveTool::SolidSplit
            ) {
                self.selection.clear();
                self.status_key = "status-solid-tool-target";
            }
            self.digest = self.catalog.format(
                "digest-tool-active",
                &BTreeMap::from([("tool", self.catalog.text(tool.label_key()))]),
            );
            return;
        }
        match id {
            AppCommand::New
            | AppCommand::Open
            | AppCommand::Save
            | AppCommand::SaveAs
            | AppCommand::ImportMeshStl
            | AppCommand::ExportExactStep
            | AppCommand::ExportMeshStl => {
                self.dispatch_file_command(id);
            }
            AppCommand::Undo => {
                if self.undo() {
                    self.digest = self.catalog.text("digest-undo");
                }
            }
            AppCommand::Redo => {
                if self.redo() {
                    self.digest = self.catalog.text("digest-redo");
                }
            }
            AppCommand::Copy => {
                self.copy_selection_to_clipboard();
            }
            AppCommand::Paste => {
                self.paste_clipboard();
            }
            AppCommand::Delete => {
                if self.delete_selected() {
                    self.digest = self.catalog.text("digest-deleted");
                }
            }
            AppCommand::Deselect => self.clear_selection(),
            AppCommand::SelectAll => self.select_all(),
            AppCommand::Group => {
                self.group_selected();
            }
            AppCommand::Ungroup => {
                self.ungroup_selected();
            }
            AppCommand::MakeComponent => {
                self.make_component();
            }
            AppCommand::MakeUnique => {
                self.make_unique();
            }
            AppCommand::Hide => {
                self.set_selection_visibility(false);
            }
            AppCommand::Unhide => {
                self.set_selection_visibility(true);
            }
            AppCommand::ViewIso => self.look_from(-2.25, 0.52, "view-iso"),
            AppCommand::ViewTop => {
                self.look_from(-std::f32::consts::FRAC_PI_2, 1.44, "view-top");
            }
            AppCommand::ViewFront => {
                self.look_from(-std::f32::consts::FRAC_PI_2, 0.02, "view-front");
            }
            AppCommand::ViewProjection => self.toggle_projection_mode(),
            AppCommand::ZoomFit => self.zoom_fit(),
            AppCommand::Shortcuts => self.shortcuts_open = true,
            AppCommand::Select
            | AppCommand::Line
            | AppCommand::Rectangle
            | AppCommand::Circle
            | AppCommand::Arc
            | AppCommand::CutThrough
            | AppCommand::Pocket
            | AppCommand::SolidSubtract
            | AppCommand::SolidUnion
            | AppCommand::SolidIntersect
            | AppCommand::SolidSplit
            | AppCommand::PlanarOffset
            | AppCommand::Sweep
            | AppCommand::Loft
            | AppCommand::Revolve
            | AppCommand::Shell
            | AppCommand::Fillet
            | AppCommand::Chamfer
            | AppCommand::PushPull
            | AppCommand::Move
            | AppCommand::Measure
            | AppCommand::Orbit
            | AppCommand::Pan => {}
        }
    }

    /// How many selected occurrences currently have the given visibility.
    fn selected_occurrences_with_visibility(&self, visible: bool) -> usize {
        let snapshot = self.document.current();
        self.selected_occurrence_ids()
            .into_iter()
            .filter(|id| {
                snapshot
                    .occurrence(*id)
                    .is_some_and(|occurrence| occurrence.visible() == visible)
            })
            .count()
    }

    /// Commit the visibility of every selected occurrence as one undo step.
    pub fn set_selection_visibility(&mut self, visible: bool) -> bool {
        let snapshot = self.document.current();
        let commands = self
            .selected_occurrence_ids()
            .into_iter()
            .filter(|id| {
                snapshot
                    .occurrence(*id)
                    .is_some_and(|occurrence| occurrence.visible() != visible)
            })
            .map(|id| CanonicalCommand::SetOccurrenceVisibility { id, visible })
            .collect::<Vec<_>>();
        let count = commands.len();
        if count == 0
            || self
                .document
                .apply_batch(&CommandBatch::new(commands))
                .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            if visible {
                "digest-unhidden"
            } else {
                "digest-hidden"
            },
            &BTreeMap::from([("count", count.to_string())]),
        );
        true
    }

    /// Contents of the value box, where exact input is typed.
    #[must_use]
    pub fn value_input(&self) -> &str {
        &self.value_input
    }

    /// Current non-authoritative Circle preview as centre and radius.
    #[must_use]
    pub fn circle_preview_geometry(&self) -> Option<(Vec3, f64)> {
        (self.active_tool == ActiveTool::Circle)
            .then_some((self.sketch_start?, self.sketch_cursor?))
            .map(|(center, cursor)| {
                (
                    center,
                    vector_length(Vec3::new(cursor.x - center.x, cursor.y - center.y, 0.0)),
                )
            })
    }

    /// Number of canonical closed two-arc Circle profiles in the document.
    #[must_use]
    pub fn circle_profile_count(&self) -> usize {
        self.document
            .current()
            .features()
            .filter(|feature| {
                let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
                    return false;
                };
                exact_circle_geometry(segments, *closed).is_some()
            })
            .count()
    }

    /// Centre and radius of the newest canonical Circle occurrence.
    #[must_use]
    pub fn latest_circle_geometry(&self) -> Option<(Vec3, f64)> {
        let snapshot = self.document.current();
        snapshot
            .occurrences()
            .filter_map(|occurrence| {
                let definition = snapshot.definition(occurrence.definition_id())?;
                let (center, radius) = definition.feature_ids().iter().find_map(|feature_id| {
                    let FeatureKind::SegmentProfile { segments, closed } =
                        snapshot.feature(*feature_id)?.kind()
                    else {
                        return None;
                    };
                    exact_circle_geometry(segments, *closed)
                })?;
                let transform = occurrence.transform();
                let matrix = transform.matrix();
                Some((
                    occurrence.id(),
                    Vec3::new(
                        matrix[0] * center[0] + matrix[1] * center[1] + matrix[3],
                        matrix[4] * center[0] + matrix[5] * center[1] + matrix[7],
                        matrix[8] * center[0] + matrix[9] * center[1] + matrix[11],
                    ),
                    radius,
                ))
            })
            .max_by_key(|(id, _, _)| *id)
            .map(|(_, center, radius)| (center, radius))
    }

    /// Current non-authoritative endpoint-bulge Arc preview.
    #[must_use]
    pub fn arc_preview_geometry(&self) -> Option<(Vec3, Vec3, Vec3, bool)> {
        (self.active_tool == ActiveTool::Arc)
            .then_some(arc_geometry(
                self.sketch_start?,
                self.sketch_end?,
                self.sketch_cursor?,
            )?)
            .map(|arc| (arc.start, arc.end, arc.center, arc.clockwise))
    }

    /// Number of canonical closed Arc-plus-chord profiles in the document.
    #[must_use]
    pub fn arc_profile_count(&self) -> usize {
        self.document
            .current()
            .features()
            .filter(|feature| {
                let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
                    return false;
                };
                exact_arc_profile_geometry(segments, *closed).is_some()
            })
            .count()
    }

    /// World-space geometry of the newest canonical Arc-plus-chord profile.
    #[must_use]
    pub fn latest_arc_geometry(&self) -> Option<(Vec3, Vec3, Vec3, bool)> {
        let snapshot = self.document.current();
        snapshot
            .occurrences()
            .filter_map(|occurrence| {
                let definition = snapshot.definition(occurrence.definition_id())?;
                let (start, end, center, clockwise) =
                    definition.feature_ids().iter().find_map(|feature_id| {
                        let FeatureKind::SegmentProfile { segments, closed } =
                            snapshot.feature(*feature_id)?.kind()
                        else {
                            return None;
                        };
                        exact_arc_profile_geometry(segments, *closed)
                    })?;
                let transform = occurrence.transform();
                let matrix = transform.matrix();
                let world = |point: [f64; 2]| {
                    Vec3::new(
                        matrix[0] * point[0] + matrix[1] * point[1] + matrix[3],
                        matrix[4] * point[0] + matrix[5] * point[1] + matrix[7],
                        matrix[8] * point[0] + matrix[9] * point[1] + matrix[11],
                    )
                };
                Some((
                    occurrence.id(),
                    world(start),
                    world(end),
                    world(center),
                    clockwise,
                ))
            })
            .max_by_key(|(id, _, _, _, _)| *id)
            .map(|(_, start, end, center, clockwise)| (start, end, center, clockwise))
    }

    /// Current camera magnification, as changed by Zoom Fit and the wheel.
    #[must_use]
    pub const fn camera_zoom(&self) -> f32 {
        self.zoom
    }

    /// Whether the keyboard shortcut reference is on screen.
    #[must_use]
    pub const fn shortcuts_visible(&self) -> bool {
        self.shortcuts_open
    }

    /// How many occurrences of the active document are hidden.
    #[must_use]
    pub fn hidden_occurrence_count(&self) -> usize {
        self.document
            .current()
            .scene_query()
            .iter()
            .filter(|item| !item.visible)
            .count()
    }

    fn look_from(&mut self, yaw: f32, pitch: f32, view_key: &str) {
        self.yaw = yaw;
        self.pitch = pitch;
        self.digest = self.catalog.format(
            "digest-view-changed",
            &BTreeMap::from([("view", self.catalog.text(view_key))]),
        );
    }

    /// Frame every visible occurrence in the viewport laid out by the last frame.
    pub fn zoom_fit(&mut self) {
        let Some(rect) = self.viewport_rect else {
            return;
        };
        let corners = self
            .active_boxes()
            .into_iter()
            .flat_map(|item| {
                box_corners(item.size_mm.x, item.size_mm.y, item.size_mm.z)
                    .map(|point| point + item.origin_mm)
            })
            .collect::<Vec<_>>();
        let Some(first) = corners.first().copied() else {
            return;
        };
        let (mut low, mut high) = (first.z, first.z);
        for corner in &corners {
            low = low.min(corner.z);
            high = high.max(corner.z);
        }
        self.camera_target_z = f64::midpoint(low, high);
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        let flat = projected_bounds(&corners, |point| self.project(point, rect));
        let fit =
            (rect.width() / flat.width().max(1.0)).min(rect.height() / flat.height().max(1.0));
        self.zoom = (fit * 0.82).clamp(MIN_CAMERA_ZOOM, MAX_CAMERA_ZOOM);
        let scaled = projected_bounds(&corners, |point| self.project(point, rect));
        self.pan = rect.center() - scaled.center();
        self.digest = self.catalog.format(
            "digest-zoom-fit",
            &BTreeMap::from([("count", self.active_box_count().to_string())]),
        );
    }

    fn selected_box(&self) -> Option<RenderBox> {
        let instance_path = self.selected_instance_paths().into_iter().next()?;
        self.active_boxes()
            .into_iter()
            .find(|item| item.instance_path == instance_path)
    }

    pub fn group_selected(&mut self) -> bool {
        let ids = self
            .selected_occurrence_ids()
            .into_iter()
            .collect::<Vec<_>>();
        if ids.len() < 2 || self.selection.selected_group.is_some() {
            return false;
        }
        let snapshot = self.document.current();
        let parents = ids
            .iter()
            .map(|id| {
                snapshot
                    .occurrence(*id)
                    .map(|occurrence| occurrence.parent())
            })
            .collect::<Option<BTreeSet<_>>>();
        let Some(parents) = parents else {
            return false;
        };
        if parents.len() != 1 {
            return false;
        }
        let parent = parents.into_iter().next().flatten();
        let group_id = GroupId(
            snapshot
                .groups()
                .map(|group| group.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let mut commands = vec![CanonicalCommand::CreateGroup {
            id: group_id,
            name: self.catalog.format(
                "model-default-group",
                &BTreeMap::from([("number", group_id.0.to_string())]),
            ),
            transform: Transform::identity(),
            parent,
        }];
        commands.extend(
            ids.iter()
                .copied()
                .map(|id| CanonicalCommand::SetOccurrenceParent {
                    id,
                    parent: Some(group_id),
                }),
        );
        if self
            .document
            .apply_batch(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.select_group(group_id);
        self.digest = self.catalog.format(
            "digest-grouped",
            &BTreeMap::from([("count", ids.len().to_string())]),
        );
        true
    }

    pub fn ungroup_selected(&mut self) -> bool {
        let Some(group_id) = self.selected_group_id() else {
            return false;
        };
        let snapshot = self.document.current();
        let Some(group) = snapshot.group(group_id) else {
            return false;
        };
        let group_transform = group.transform();
        let parent = group.parent();
        let occurrences = snapshot
            .occurrences()
            .filter(|occurrence| occurrence.parent() == Some(group_id))
            .map(|occurrence| (occurrence.id(), occurrence.transform()))
            .collect::<Vec<_>>();
        let child_groups = snapshot
            .groups()
            .filter(|child| child.parent() == Some(group_id))
            .map(|child| (child.id(), child.transform()))
            .collect::<Vec<_>>();
        if occurrences.is_empty() && child_groups.is_empty() {
            return false;
        }
        let mut commands = Vec::new();
        for (id, transform) in &occurrences {
            commands.push(CanonicalCommand::SetOccurrenceTransform {
                id: *id,
                transform: group_transform.compose(*transform),
            });
            commands.push(CanonicalCommand::SetOccurrenceParent { id: *id, parent });
        }
        for (id, transform) in &child_groups {
            commands.push(CanonicalCommand::SetGroupTransform {
                id: *id,
                transform: group_transform.compose(*transform),
            });
            commands.push(CanonicalCommand::SetGroupParent { id: *id, parent });
        }
        commands.push(CanonicalCommand::DeleteGroup { id: group_id });
        if self
            .document
            .apply_batch(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.selection.clear();
        self.selection
            .occurrences
            .extend(occurrences.iter().map(|(id, _)| InstancePath::root(*id)));
        self.digest = self.catalog.format(
            "digest-ungrouped",
            &BTreeMap::from([(
                "count",
                (occurrences.len() + child_groups.len()).to_string(),
            )]),
        );
        true
    }

    pub fn make_component(&mut self) -> bool {
        let Some(group_id) = self.selected_group_id() else {
            return false;
        };
        let name = self.catalog.format(
            "model-component-name",
            &BTreeMap::from([("number", group_id.0.to_string())]),
        );
        let Ok(result) = self
            .document
            .convert_group_to_component(group_id, name.clone())
        else {
            return false;
        };
        self.selection.clear();
        self.selection
            .select_occurrence(result.component_occurrence_id, false);
        self.digest = self.catalog.format(
            "digest-made-component",
            &BTreeMap::from([("name", name), ("count", result.mappings.len().to_string())]),
        );
        true
    }

    pub fn make_unique(&mut self) -> bool {
        if self.selection_count() != 1 || self.selected_shared_occurrence_count() < 2 {
            return false;
        }
        let Some(occurrence_id) = self.selected_occurrence_ids().into_iter().next() else {
            return false;
        };
        let snapshot = self.document.current();
        let Some(source) = snapshot
            .occurrence(occurrence_id)
            .and_then(|item| snapshot.definition(item.definition_id()))
        else {
            return false;
        };
        let new_name = self.catalog.format(
            "model-unique-name",
            &BTreeMap::from([("name", source.name().to_owned())]),
        );
        if self.document.make_unique(occurrence_id, new_name).is_err() {
            return false;
        }
        self.selection.select_occurrence(occurrence_id, false);
        self.digest = self.catalog.text("digest-made-unique");
        true
    }

    fn bottle_feature_ids(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Option<BottleFeatureIds> {
        let definition = snapshot.definition(definition_id)?;
        let mut control = None;
        let mut shell = None;
        let mut finish = None;
        for feature_id in definition.feature_ids() {
            match snapshot.feature(*feature_id)?.kind() {
                FeatureKind::BottleProfileControl { .. } => control = Some(*feature_id),
                FeatureKind::Shell { .. } => shell = Some(*feature_id),
                FeatureKind::BottleEdgeFinish { .. } => finish = Some(*feature_id),
                _ => {}
            }
        }
        Some(BottleFeatureIds {
            control: control?,
            shell: shell?,
            finish: finish?,
        })
    }

    fn bottle_editor_inputs(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Option<BottleEditorInputs> {
        let ids = Self::bottle_feature_ids(snapshot, definition_id)?;
        let FeatureKind::BottleProfileControl {
            body_radius,
            body_height,
            shoulder_rise,
            ..
        } = snapshot.feature(ids.control)?.kind()
        else {
            return None;
        };
        let FeatureKind::Shell { thickness, .. } = snapshot.feature(ids.shell)?.kind() else {
            return None;
        };
        let FeatureKind::BottleEdgeFinish { kind, amount, .. } =
            snapshot.feature(ids.finish)?.kind()
        else {
            return None;
        };
        Some(BottleEditorInputs {
            definition_id,
            body_radius: body_radius.source_token().to_owned(),
            body_height: body_height.source_token().to_owned(),
            shoulder_rise: shoulder_rise.source_token().to_owned(),
            thickness: thickness.source_token().to_owned(),
            finish_amount: amount.source_token().to_owned(),
            finish_kind: *kind,
        })
    }

    fn selected_bottle_definition(&self) -> Option<DefinitionId> {
        let snapshot = self.document.current();
        let definition_id = self
            .selection
            .primary
            .as_ref()
            .map(|selection| selection.definition_id)
            .or_else(|| {
                let path = self.selection.occurrences.iter().next()?;
                snapshot
                    .occurrence(path.root_occurrence())
                    .map(|occurrence| occurrence.definition_id())
            })?;
        Self::bottle_feature_ids(&snapshot, definition_id).map(|_| definition_id)
    }

    pub fn create_bottle(&mut self) -> bool {
        let snapshot = self.document.current();
        let definition_id = DefinitionId(
            snapshot
                .definitions()
                .map(|definition| definition.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let first_feature_id = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            + 1;
        let profile = FeatureId(first_feature_id);
        let control = FeatureId(first_feature_id + 1);
        let revolve = FeatureId(first_feature_id + 2);
        let shell = FeatureId(first_feature_id + 3);
        let finish = FeatureId(first_feature_id + 4);
        let occurrence_id = OccurrenceId(
            snapshot
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let offset = snapshot.occurrences().count() as f64 * 90.0;
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: "Editable bottle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id,
                name: "Validated bottle profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [0.0, 0.0],
                        [30.0, 0.0],
                        [30.0, 110.0],
                        [12.0, 130.0],
                        [12.0, 155.0],
                        [0.0, 155.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: control,
                definition_id,
                name: "Scale stretch flatten controls".to_owned(),
                kind: FeatureKind::BottleProfileControl {
                    profile,
                    body_radius: Dimension::new("30", 30.0).expect("built-in radius is valid"),
                    body_height: Dimension::new("110", 110.0).expect("built-in height is valid"),
                    shoulder_rise: Dimension::new("20", 20.0).expect("built-in shoulder is valid"),
                },
            },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id,
                name: "Bottle revolve".to_owned(),
                kind: FeatureKind::full_revolve(control),
            },
            CanonicalCommand::CreateFeature {
                id: shell,
                definition_id,
                name: "Bottle shell".to_owned(),
                kind: FeatureKind::Shell {
                    target: revolve,
                    removed_faces: vec![
                        StableFaceRole::new(BOTTLE_SHELL_OPENING_FACE_ROLE)
                            .expect("built-in bottle face role is valid"),
                    ],
                    thickness: Dimension::new("2", 2.0).expect("built-in thickness is valid"),
                },
            },
            CanonicalCommand::CreateFeature {
                id: finish,
                definition_id,
                name: "Bottle shoulder finish".to_owned(),
                kind: FeatureKind::BottleEdgeFinish {
                    target: shell,
                    edges: vec![
                        StableEdgeRole::new(BOTTLE_SHOULDER_EDGE_ROLE)
                            .expect("built-in bottle edge role is valid"),
                    ],
                    kind: BottleEdgeFinishKind::Fillet,
                    amount: Dimension::new("2", 2.0).expect("built-in finish is valid"),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: "Editable bottle occurrence".to_owned(),
                transform: Transform::from_translation(offset, 0.0, 0.0)
                    .expect("built-in bottle placement is valid"),
                parent: None,
                tag: None,
                visible: true,
            },
        ]);
        if self.document.apply_batch(&batch).is_err() {
            return false;
        }
        self.selection
            .select_path(InstancePath::root(occurrence_id), false);
        self.bottle_editor = Self::bottle_editor_inputs(&self.document.current(), definition_id);
        self.digest = "Created editable exact bottle; worker evaluation pending".to_owned();
        true
    }

    fn set_bottle_parameters(&mut self, editor: &BottleEditorInputs) -> bool {
        let snapshot = self.document.current();
        let Some(ids) = Self::bottle_feature_ids(&snapshot, editor.definition_id) else {
            return false;
        };
        let Some(body_radius) = parse_dimension(&editor.body_radius) else {
            return false;
        };
        let Some(body_height) = parse_dimension(&editor.body_height) else {
            return false;
        };
        let Some(shoulder_rise) = parse_dimension(&editor.shoulder_rise) else {
            return false;
        };
        let Some(thickness) = parse_dimension(&editor.thickness) else {
            return false;
        };
        let Some(finish_amount) = parse_dimension(&editor.finish_amount) else {
            return false;
        };
        let batch = CommandBatch::new(vec![
            CanonicalCommand::SetBottleControlDimension {
                id: ids.control,
                control: BottleControlDimension::BodyRadius,
                dimension: body_radius,
            },
            CanonicalCommand::SetBottleControlDimension {
                id: ids.control,
                control: BottleControlDimension::BodyHeight,
                dimension: body_height,
            },
            CanonicalCommand::SetBottleControlDimension {
                id: ids.control,
                control: BottleControlDimension::ShoulderRise,
                dimension: shoulder_rise,
            },
            CanonicalCommand::SetFeatureDimension {
                id: ids.shell,
                dimension: thickness,
            },
            CanonicalCommand::SetFeatureDimension {
                id: ids.finish,
                dimension: finish_amount,
            },
            CanonicalCommand::SetBottleEdgeFinishKind {
                id: ids.finish,
                kind: editor.finish_kind,
            },
        ]);
        if self.document.apply_batch(&batch).is_err() {
            self.digest = "Bottle edit rejected; canonical document unchanged".to_owned();
            return false;
        }
        self.bottle_editor =
            Self::bottle_editor_inputs(&self.document.current(), editor.definition_id);
        self.digest =
            "Bottle parameters committed atomically; exact re-evaluation pending".to_owned();
        true
    }

    pub fn bottle_authority_report(
        &self,
        definition_id: DefinitionId,
    ) -> Option<BottleAuthorityReport> {
        let snapshot = self.document.current();
        let package = self.exact_results.get(&definition_id)?.revolve()?;
        Some(package.authority_report(&snapshot))
    }

    pub fn export_bottle_exact_recipe_to(
        &mut self,
        definition_id: DefinitionId,
        path: &Path,
    ) -> bool {
        let snapshot = self.document.current();
        let result = self
            .exact_results
            .get(&definition_id)
            .and_then(|package| package.revolve())
            .ok_or_else(|| "current accepted bottle result is unavailable".to_owned())
            .and_then(|package| {
                package
                    .export_bundle(&snapshot)
                    .map_err(|error| error.to_string())
            })
            .and_then(|bundle| {
                std::fs::write(path, bundle.exact_recipe).map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => {
                self.digest = format!("Exported current exact bottle recipe to {}", path.display());
                true
            }
            Err(error) => {
                self.digest = format!("Exact bottle export blocked: {error}");
                false
            }
        }
    }

    pub fn export_bottle_step_to(&mut self, definition_id: DefinitionId, path: &Path) -> bool {
        let snapshot = self.document.current();
        let result = (|| {
            let package = self
                .exact_results
                .get(&definition_id)
                .and_then(|package| package.revolve())
                .filter(|package| package.is_current(&snapshot))
                .cloned()
                .ok_or_else(|| "current accepted bottle result is unavailable".to_owned())?;
            let request = ExactRevolveRequest::from_snapshot(&snapshot, definition_id)
                .map_err(|error| error.to_string())?;
            let executable = self
                .exact_worker_path
                .clone()
                .or_else(|| {
                    exact_worker_candidates()
                        .into_iter()
                        .find(|path| path.is_file())
                })
                .ok_or_else(|| "exact worker is unavailable".to_owned())?;
            let mut worker =
                ExactWorkerSupervisor::spawn(executable).map_err(|error| error.to_string())?;
            worker
                .export_revolve_step(&snapshot, &request, &package, path)
                .map_err(|error| error.to_string())?;
            std::fs::write(
                path.with_extension("step.loss.txt"),
                exact_step_loss_report(&package),
            )
            .map_err(|error| error.to_string())
        })();
        match result {
            Ok(()) => {
                self.digest = format!(
                    "Exported current exact bottle STEP with explicit loss report to {}",
                    path.display()
                );
                true
            }
            Err(error) => {
                self.digest = format!("Exact bottle STEP export blocked: {error}");
                false
            }
        }
    }

    pub fn exact_reference_for_occurrence(
        &self,
        instance_path: &InstancePath,
        role: ExactFaceRole,
    ) -> Option<AssemblySelectionTarget> {
        let snapshot = self.document.current();
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| &occurrence.instance_path == instance_path)?;
        let package = self
            .exact_results
            .get(&occurrence.definition_id)
            .filter(|package| package.is_current(&snapshot))?;
        Some(AssemblySelectionTarget {
            instance_path: instance_path.clone(),
            body: package.reference(role)?.clone(),
        })
    }

    pub fn export_bottle_mesh_to(&mut self, definition_id: DefinitionId, path: &Path) -> bool {
        let snapshot = self.document.current();
        let result = self
            .exact_results
            .get(&definition_id)
            .filter(|package| package.is_current(&snapshot))
            .ok_or_else(|| "current accepted exact body result is unavailable".to_owned())
            .map(|package| package.mesh_export(Transform::identity()))
            .and_then(|bundle| {
                self.authorize_path_side_effect(
                    HighRiskClass::LossyConversion,
                    "export-lossy-obj-with-loss-report",
                    "Confirm lossy mesh export",
                    "lossy exact-to-mesh conversion",
                    path,
                    &exact_mesh_export_evidence(&bundle),
                )?;
                write_exact_mesh_export(path, bundle)
            });
        match result {
            Ok(()) => {
                self.digest = format!(
                    "Exported derived bottle OBJ with explicit loss report to {}",
                    path.display()
                );
                true
            }
            Err(error) => {
                self.digest = format!("Bottle mesh export blocked: {error}");
                false
            }
        }
    }

    pub fn export_exact_occurrence_mesh_to(
        &mut self,
        instance_path: &InstancePath,
        path: &Path,
    ) -> bool {
        let snapshot = self.document.current();
        let result = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| &occurrence.instance_path == instance_path)
            .ok_or_else(|| "canonical occurrence is unavailable".to_owned())
            .and_then(|occurrence| {
                self.exact_results
                    .get(&occurrence.definition_id)
                    .filter(|package| package.is_current(&snapshot))
                    .ok_or_else(|| "current accepted exact body result is unavailable".to_owned())
                    .map(|package| package.mesh_export(occurrence.transform))
            })
            .and_then(|bundle| {
                self.authorize_path_side_effect(
                    HighRiskClass::LossyConversion,
                    "export-lossy-obj-with-loss-report",
                    "Confirm lossy mesh export",
                    "lossy exact-to-mesh conversion",
                    path,
                    &exact_mesh_export_evidence(&bundle),
                )?;
                write_exact_mesh_export(path, bundle)
            });
        match result {
            Ok(()) => {
                self.digest = format!(
                    "Exported transformed exact occurrence OBJ with explicit loss report to {}",
                    path.display()
                );
                true
            }
            Err(error) => {
                self.digest = format!("Exact occurrence mesh export blocked: {error}");
                false
            }
        }
    }

    pub fn create_closed_polyline(&mut self, points_mm: Vec<[f64; 2]>) -> bool {
        self.create_profile_at(Vec3::ZERO, points_mm)
    }

    pub fn create_sweep_inputs(
        &mut self,
        profile_points_mm: Vec<[f64; 2]>,
        path_start_mm: [f64; 2],
        path_end_mm: [f64; 2],
    ) -> bool {
        let snapshot = self.document.current();
        let next_definition = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let next_feature = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let next_occurrence = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let (Some(definition), Some(profile), Some(occurrence)) =
            (next_definition, next_feature, next_occurrence)
        else {
            return false;
        };
        let Some(path) = profile.checked_add(1) else {
            return false;
        };
        let definition_id = DefinitionId(definition);
        let profile_feature_id = FeatureId(profile);
        let path_feature_id = FeatureId(path);
        let occurrence_id = OccurrenceId(occurrence);
        let name = self.catalog.format(
            "model-default-box",
            &BTreeMap::from([("number", definition.to_string())]),
        );
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: name.clone(),
            },
            CanonicalCommand::CreateFeature {
                id: profile_feature_id,
                definition_id,
                name: self.catalog.text("model-default-profile"),
                kind: FeatureKind::Profile {
                    points_mm: profile_points_mm,
                },
            },
            CanonicalCommand::CreateFeature {
                id: path_feature_id,
                definition_id,
                name: self.catalog.text("model-sweep-path"),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: path_start_mm,
                        end_mm: path_end_mm,
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: self.catalog.format(
                    "model-default-occurrence",
                    &BTreeMap::from([("name", name)]),
                ),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]);
        if self.document.apply_batch(&batch).is_err() {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.selection.select_exact(
            SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        self.status_key = "status-sweep-inputs-selected";
        true
    }

    pub fn create_loft_inputs(&mut self, sections: Vec<(Vec<[f64; 2]>, f64)>) -> bool {
        if !(2..=16).contains(&sections.len()) {
            return false;
        }
        let snapshot = self.document.current();
        let Some(definition_id) = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(DefinitionId)
        else {
            return false;
        };
        let first_feature = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let next_occurrence = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let (Some(first_feature), Some(occurrence)) = (first_feature, next_occurrence) else {
            return false;
        };
        let name = self.catalog.format(
            "model-loft-definition",
            &BTreeMap::from([("number", definition_id.0.to_string())]),
        );
        let mut commands = vec![CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: name.clone(),
        }];
        let mut loft_sections = Vec::with_capacity(sections.len());
        for (index, (control_points_mm, elevation_mm)) in sections.into_iter().enumerate() {
            let Some(feature_id) = first_feature.checked_add(index as u64).map(FeatureId) else {
                return false;
            };
            commands.push(CanonicalCommand::CreateFeature {
                id: feature_id,
                definition_id,
                name: self.catalog.format(
                    "model-spline-profile",
                    &BTreeMap::from([("number", (index + 1).to_string())]),
                ),
                kind: FeatureKind::SplineProfile { control_points_mm },
            });
            loft_sections.push(LoftSection {
                profile: feature_id,
                elevation_mm,
            });
        }
        let occurrence_id = OccurrenceId(occurrence);
        commands.push(CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name: self.catalog.format(
                "model-default-occurrence",
                &BTreeMap::from([("name", name)]),
            ),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        });
        if self
            .document
            .apply_batch(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.loft_input_sections = Some((definition_id, loft_sections));
        self.selection.select_exact(
            SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        self.status_key = "status-loft-inputs-selected";
        true
    }

    fn create_segment_profile_at(
        &mut self,
        origin_mm: Vec3,
        segments: Vec<ProfileSegment>,
        default_name_key: &str,
        profile_name_key: &str,
    ) -> bool {
        let snapshot = self.document.current();
        let Some(definition_id) = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(DefinitionId)
        else {
            return false;
        };
        let Some(profile_id) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)
        else {
            return false;
        };
        let Some(occurrence_id) = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(OccurrenceId)
        else {
            return false;
        };
        let name = self.catalog.format(
            default_name_key,
            &BTreeMap::from([("number", definition_id.0.to_string())]),
        );
        let occurrence_name = self.catalog.format(
            "model-default-occurrence",
            &BTreeMap::from([("name", name.clone())]),
        );
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name,
            },
            CanonicalCommand::CreateFeature {
                id: profile_id,
                definition_id,
                name: self.catalog.text(profile_name_key),
                kind: FeatureKind::SegmentProfile {
                    segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: occurrence_name,
                transform: Transform::from_translation(origin_mm.x, origin_mm.y, origin_mm.z)
                    .expect("validated profile origin is canonical"),
                parent: None,
                tag: None,
                visible: true,
            },
        ]);
        if self.document.apply_batch(&batch).is_err() {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.push_pull_distance_input.clear();
        self.selection.select_exact(
            SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        true
    }

    fn create_profile_at(&mut self, origin_mm: Vec3, points_mm: Vec<[f64; 2]>) -> bool {
        let snapshot = self.document.current();
        let Some(definition_id) = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(DefinitionId)
        else {
            return false;
        };
        let Some(profile_id) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)
        else {
            return false;
        };
        let Some(occurrence_id) = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(OccurrenceId)
        else {
            return false;
        };
        let name = self.catalog.format(
            "model-default-box",
            &BTreeMap::from([("number", definition_id.0.to_string())]),
        );
        let occurrence_name = self.catalog.format(
            "model-default-occurrence",
            &BTreeMap::from([("name", name.clone())]),
        );
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name,
            },
            CanonicalCommand::CreateFeature {
                id: profile_id,
                definition_id,
                name: self.catalog.text("model-default-profile"),
                kind: FeatureKind::Profile { points_mm },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: occurrence_name,
                transform: Transform::from_translation(origin_mm.x, origin_mm.y, origin_mm.z)
                    .expect("validated profile origin is canonical"),
                parent: None,
                tag: None,
                visible: true,
            },
        ]);
        if self.document.apply_batch(&batch).is_err() {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.push_pull_distance_input.clear();
        self.selection.select_exact(
            SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        self.status_key = "status-sketch-created";
        true
    }

    fn create_box_at(&mut self, origin_mm: Vec3, size_mm: Vec3) -> bool {
        if !size_mm.x.is_finite()
            || !size_mm.y.is_finite()
            || !size_mm.z.is_finite()
            || size_mm.x <= 0.01
            || size_mm.y <= 0.01
            || size_mm.z <= 0.01
        {
            return false;
        }
        let snapshot = self.document.current();
        let definition_id = DefinitionId(
            snapshot
                .definitions()
                .map(|definition| definition.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let first_feature_id = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            + 1;
        let profile_id = FeatureId(first_feature_id);
        let extrusion_id = FeatureId(first_feature_id + 1);
        let occurrence_id = OccurrenceId(
            snapshot
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let name = self.catalog.format(
            "model-default-box",
            &BTreeMap::from([("number", definition_id.0.to_string())]),
        );
        let occurrence_name = self.catalog.format(
            "model-default-occurrence",
            &BTreeMap::from([("name", name.clone())]),
        );
        if self
            .document
            .apply_batch(&create_box_batch(
                definition_id,
                [profile_id, extrusion_id],
                occurrence_id,
                [
                    &name,
                    &self.catalog.text("model-default-profile"),
                    &self.catalog.text("model-default-extrusion"),
                    &occurrence_name,
                ],
                origin_mm,
                size_mm,
            ))
            .is_err()
        {
            return false;
        }
        self.preview = None;
        self.preview_box = None;
        self.preview_definition_id = None;
        self.push_pull_distance_input.clear();
        self.selection.select_exact(
            SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        self.status_key = "status-box-created";
        true
    }

    pub fn create_box(&mut self) -> bool {
        let offset = self.active_box_count() as f64 * 35.0;
        self.create_box_at(
            Vec3::new(offset, offset, 0.0),
            Vec3::new(BOX_WIDTH_MM, BOX_DEPTH_MM, 20.0),
        )
    }

    fn selected_move_reference(&self) -> Option<SelectionId> {
        if let Some(primary) = &self.selection.primary {
            return Some(primary.clone());
        }
        let instance_path = self.selection.occurrences.iter().next()?.clone();
        let snapshot = self.document.current();
        let occurrence = snapshot.occurrence(instance_path.root_occurrence())?;
        Some(SelectionId {
            definition_id: occurrence.definition_id(),
            instance_path,
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        })
    }

    fn translate_group(&mut self, group_id: GroupId, delta_mm: Vec3) -> bool {
        let distance_mm = vector_length(delta_mm);
        if !delta_mm.x.is_finite()
            || !delta_mm.y.is_finite()
            || !delta_mm.z.is_finite()
            || distance_mm <= 0.0
        {
            return false;
        }
        let snapshot = self.document.current();
        let Some(group) = snapshot.group(group_id) else {
            return false;
        };
        let Ok(transform) = translated_transform(group.transform(), delta_mm) else {
            return false;
        };
        if self
            .document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetGroupTransform {
                    id: group_id,
                    transform,
                },
            ]))
            .is_err()
        {
            return false;
        }
        self.select_group(group_id);
        self.last_move = None;
        self.status_key = "status-object-moved";
        self.digest = self.catalog.format(
            "digest-move-committed",
            &BTreeMap::from([("distance", format_height(distance_mm))]),
        );
        true
    }

    fn commit_move_drag(&mut self, drag: &MoveDrag) -> bool {
        if let Some(group_id) = drag.group_id {
            self.translate_group(group_id, drag.delta_mm)
        } else {
            self.translate_occurrence(&drag.selection, drag.delta_mm, drag.copy)
        }
    }

    fn translate_occurrence(
        &mut self,
        selection: &SelectionId,
        delta_mm: Vec3,
        copy: bool,
    ) -> bool {
        let distance_mm = vector_length(delta_mm);
        if !delta_mm.x.is_finite()
            || !delta_mm.y.is_finite()
            || !delta_mm.z.is_finite()
            || distance_mm <= 0.0
        {
            return false;
        }
        if matches!(
            self.selection.edit_context.last(),
            Some(EditContext::Definition { .. })
        ) || !self.occurrence_in_active_context(&selection.instance_path)
        {
            return false;
        }
        let snapshot = self.document.current();
        if !selection.instance_path.is_root() {
            return false;
        }
        let source_id = selection.instance_path.root_occurrence();
        let Some(source) = snapshot.occurrence(source_id) else {
            return false;
        };
        let definition_id = source.definition_id();
        if definition_id != selection.definition_id {
            return false;
        }
        let Ok(transform) = translated_transform(source.transform(), delta_mm) else {
            return false;
        };
        let target_id = if copy {
            OccurrenceId(
                snapshot
                    .occurrences()
                    .map(|occurrence| occurrence.id().0)
                    .max()
                    .unwrap_or(0)
                    + 1,
            )
        } else {
            source_id
        };
        let command = if copy {
            let Some(definition) = snapshot.definition(definition_id) else {
                return false;
            };
            let number = snapshot
                .scene_query()
                .into_iter()
                .filter(|item| item.definition_id == definition_id)
                .count()
                + 1;
            CanonicalCommand::CreateOccurrence {
                id: target_id,
                definition_id,
                name: self.catalog.format(
                    "model-copy-occurrence",
                    &BTreeMap::from([
                        ("name", definition.name().to_owned()),
                        ("number", number.to_string()),
                    ]),
                ),
                transform,
                parent: source.parent(),
                tag: source.tag(),
                visible: source.visible(),
            }
        } else {
            CanonicalCommand::SetOccurrenceTransform {
                id: target_id,
                transform,
            }
        };
        if self
            .document
            .apply_batch(&CommandBatch::new(vec![command]))
            .is_err()
        {
            return false;
        }
        let target = SelectionId {
            definition_id,
            instance_path: InstancePath::root(target_id),
            element: selection.element.clone(),
        };
        self.selection.select_exact(target, false);
        self.last_move = Some(LastMove {
            occurrence_id: target_id,
            direction: delta_mm * (1.0 / distance_mm),
            applied_distance_mm: distance_mm,
        });
        self.status_key = if copy {
            "status-object-copied"
        } else {
            "status-object-moved"
        };
        self.digest = self.catalog.format(
            if copy {
                "digest-copy-committed"
            } else {
                "digest-move-committed"
            },
            &BTreeMap::from([("distance", format_height(distance_mm))]),
        );
        true
    }

    pub fn move_selected(&mut self, delta_mm: Vec3) -> bool {
        if let Some(group_id) = self.selection.selected_group {
            return self.translate_group(group_id, delta_mm);
        }
        let Some(selection) = self.selected_move_reference() else {
            return false;
        };
        self.translate_occurrence(&selection, delta_mm, false)
    }

    pub fn copy_selected(&mut self, delta_mm: Vec3) -> bool {
        let Some(selection) = self.selected_move_reference() else {
            return false;
        };
        self.translate_occurrence(&selection, delta_mm, true)
    }

    pub fn preview_align_occurrences(
        &mut self,
        moving_id: OccurrenceId,
        reference_id: OccurrenceId,
        axis: Axis,
        mode: AlignMode,
    ) -> bool {
        self.occurrence_operation_preview = None;
        if moving_id == reference_id
            || matches!(
                self.selection.edit_context.last(),
                Some(EditContext::Definition { .. })
            )
            || !self.occurrence_in_active_context(&InstancePath::root(moving_id))
            || !self.occurrence_in_active_context(&InstancePath::root(reference_id))
        {
            return false;
        }
        let snapshot = self.document.current();
        let Some(moving) = snapshot.occurrence(moving_id) else {
            return false;
        };
        if snapshot.occurrence(reference_id).is_none() {
            return false;
        }
        let boxes = self.active_boxes();
        let Some(moving_box) = boxes
            .iter()
            .find(|item| item.instance_path == InstancePath::root(moving_id))
        else {
            return false;
        };
        let Some(reference_box) = boxes
            .iter()
            .find(|item| item.instance_path == InstancePath::root(reference_id))
        else {
            return false;
        };
        let moving_coordinate = alignment_coordinate(moving_box, axis, mode);
        let reference_coordinate = alignment_coordinate(reference_box, axis, mode);
        let offset_mm = reference_coordinate - moving_coordinate;
        if !offset_mm.is_finite() || offset_mm.abs() <= f64::EPSILON {
            return false;
        }
        let delta_mm = axis_vector(axis, offset_mm);
        let Ok(transform) = translated_transform(moving.transform(), delta_mm) else {
            return false;
        };
        let batch = CommandBatch::new(vec![CanonicalCommand::SetOccurrenceTransform {
            id: moving_id,
            transform,
        }]);
        let mut preview_box = moving_box.clone();
        preview_box.origin_mm = preview_box.origin_mm + delta_mm;
        let preview = OccurrenceOperationPreview {
            source_revision: snapshot.revision_id(),
            command_digest: batch.digest(),
            batch,
            boxes: BTreeMap::from([(moving_id, preview_box)]),
            hidden_occurrences: BTreeSet::new(),
            selection_after: None,
            committed_digest_key: "digest-align-committed",
        };
        self.occurrence_operation_preview = Some(preview);
        self.status_key = "status-preview";
        self.digest = self.catalog.format(
            "digest-align-live",
            &BTreeMap::from([
                ("axis", alignment_axis_label(axis).to_owned()),
                ("mode", alignment_mode_label(mode).to_owned()),
            ]),
        );
        true
    }

    pub fn preview_linear_pattern(
        &mut self,
        source_id: OccurrenceId,
        axis: Axis,
        spacing_mm: f64,
        count: usize,
    ) -> bool {
        self.occurrence_operation_preview = None;
        if !(2..=MAX_LINEAR_PATTERN_COUNT).contains(&count)
            || !spacing_mm.is_finite()
            || spacing_mm.abs() <= f64::EPSILON
            || matches!(
                self.selection.edit_context.last(),
                Some(EditContext::Definition { .. })
            )
            || !self.occurrence_in_active_context(&InstancePath::root(source_id))
        {
            return false;
        }
        let snapshot = self.document.current();
        let Some(source) = snapshot.occurrence(source_id) else {
            return false;
        };
        let definition_id = source.definition_id();
        let Some(definition) = snapshot.definition(definition_id) else {
            return false;
        };
        let Some(source_box) = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == InstancePath::root(source_id))
        else {
            return false;
        };
        let Some(first_id) = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            return false;
        };
        let existing = snapshot
            .scene_query()
            .into_iter()
            .filter(|item| item.definition_id == definition_id)
            .count();
        let mut commands = Vec::with_capacity(count - 1);
        let mut boxes = BTreeMap::new();
        for index in 1..count {
            let Some(id_value) = first_id.checked_add((index - 1) as u64) else {
                return false;
            };
            let offset_mm = spacing_mm * index as f64;
            if !offset_mm.is_finite() {
                return false;
            }
            let delta_mm = axis_vector(axis, offset_mm);
            let Ok(transform) = translated_transform(source.transform(), delta_mm) else {
                return false;
            };
            let id = OccurrenceId(id_value);
            commands.push(CanonicalCommand::CreateOccurrence {
                id,
                definition_id,
                name: self.catalog.format(
                    "model-copy-occurrence",
                    &BTreeMap::from([
                        ("name", definition.name().to_owned()),
                        ("number", (existing + index).to_string()),
                    ]),
                ),
                transform,
                parent: source.parent(),
                tag: source.tag(),
                visible: source.visible(),
            });
            let mut preview_box = source_box.clone();
            preview_box.instance_path = InstancePath::root(id);
            preview_box.origin_mm = preview_box.origin_mm + delta_mm;
            boxes.insert(id, preview_box);
        }
        let batch = CommandBatch::new(commands);
        self.occurrence_operation_preview = Some(OccurrenceOperationPreview {
            source_revision: snapshot.revision_id(),
            command_digest: batch.digest(),
            batch,
            boxes,
            hidden_occurrences: BTreeSet::new(),
            selection_after: None,
            committed_digest_key: "digest-linear-pattern-committed",
        });
        self.status_key = "status-preview";
        self.digest = self.catalog.format(
            "digest-linear-pattern-live",
            &BTreeMap::from([
                ("axis", alignment_axis_label(axis).to_owned()),
                ("count", count.to_string()),
                ("spacing", format_height(spacing_mm)),
            ]),
        );
        true
    }

    fn active_solid_tool_operation(&self) -> Option<BooleanOperation> {
        match self.active_tool {
            ActiveTool::SolidSubtract => Some(BooleanOperation::Cut),
            ActiveTool::SolidUnion => Some(BooleanOperation::Union),
            ActiveTool::SolidIntersect => Some(BooleanOperation::Intersect),
            ActiveTool::SolidSplit => Some(BooleanOperation::Split),
            _ => None,
        }
    }

    fn solid_tool_candidate(&self, selection: &SelectionId) -> Option<(RenderBox, FeatureId)> {
        if !selection.instance_path.is_root()
            || !self.occurrence_in_active_context(&selection.instance_path)
        {
            return None;
        }
        let item = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)?;
        let extrusion_id = item.extrusion_feature_id?;
        let snapshot = self.document.current();
        let occurrence = snapshot.occurrence(selection.instance_path.root_occurrence())?;
        let feature = snapshot.feature(extrusion_id)?;
        if occurrence.definition_id() != selection.definition_id
            || feature.definition_id() != selection.definition_id
            || !matches!(feature.kind(), FeatureKind::Extrusion { .. })
        {
            return None;
        }
        Some((item, extrusion_id))
    }

    fn prepare_solid_tool_preview(&mut self, tool_selection: SelectionId, keep_tool: bool) -> bool {
        let Some(operation) = self.active_solid_tool_operation() else {
            return false;
        };
        let keep_tool = operation == BooleanOperation::Split || keep_tool;
        let Some(target_selection) = self.solid_tool_target.clone() else {
            return false;
        };
        if target_selection.instance_path == tool_selection.instance_path {
            self.digest = self.catalog.text("digest-solid-tool-distinct");
            return false;
        }
        let Some((target_box, target_feature_id)) = self.solid_tool_candidate(&target_selection)
        else {
            self.digest = self.catalog.text("digest-solid-tool-invalid");
            return false;
        };
        let Some((tool_box, tool_feature_id)) = self.solid_tool_candidate(&tool_selection) else {
            self.digest = self.catalog.text("digest-solid-tool-invalid");
            return false;
        };
        let snapshot = self.document.current();
        let Some(result_definition_value) = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            return false;
        };
        let Some(first_feature_value) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            return false;
        };
        let mut result_feature_ids = [FeatureId(0); 5];
        for (offset, id) in result_feature_ids.iter_mut().enumerate() {
            let Some(value) = first_feature_value.checked_add(offset as u64) else {
                return false;
            };
            *id = FeatureId(value);
        }
        let operation_label = self.catalog.text(match operation {
            BooleanOperation::Cut => "solid-tool-subtract",
            BooleanOperation::Union => "solid-tool-union",
            BooleanOperation::Intersect => "solid-tool-intersect",
            BooleanOperation::Split => "solid-tool-split",
        });
        let result_definition_id = DefinitionId(result_definition_value);
        let batch = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(SolidToolPlan {
            operation,
            target_occurrence_id: target_selection.instance_path.root_occurrence(),
            target_feature_id,
            tool_occurrence_id: tool_selection.instance_path.root_occurrence(),
            tool_feature_id,
            result_definition_id,
            result_feature_ids,
            result_definition_name: self.catalog.format(
                "solid-tool-result-definition",
                &BTreeMap::from([("operation", operation_label.clone())]),
            ),
            result_feature_name: self.catalog.format(
                "solid-tool-result-feature",
                &BTreeMap::from([("operation", operation_label.clone())]),
            ),
            keep_tool,
        })]);
        if self.document.validate_batch(&batch).is_err() {
            self.digest = self.catalog.text("digest-solid-tool-invalid");
            return false;
        }
        let target_occurrence_id = target_selection.instance_path.root_occurrence();
        let tool_occurrence_id = tool_selection.instance_path.root_occurrence();
        let mut result_box = target_box.clone();
        result_box.definition_id = result_definition_id;
        result_box.profile_feature_id = result_feature_ids[0];
        result_box.extrusion_feature_id = Some(result_feature_ids[1]);
        if operation == BooleanOperation::Union {
            let minimum = Vec3::new(
                target_box.origin_mm.x.min(tool_box.origin_mm.x),
                target_box.origin_mm.y.min(tool_box.origin_mm.y),
                target_box.origin_mm.z.min(tool_box.origin_mm.z),
            );
            let target_maximum = target_box.origin_mm + target_box.size_mm;
            let tool_maximum = tool_box.origin_mm + tool_box.size_mm;
            let maximum = Vec3::new(
                target_maximum.x.max(tool_maximum.x),
                target_maximum.y.max(tool_maximum.y),
                target_maximum.z.max(tool_maximum.z),
            );
            result_box.origin_mm = minimum;
            result_box.size_mm = maximum - minimum;
        } else if operation == BooleanOperation::Intersect {
            let minimum = Vec3::new(
                target_box.origin_mm.x.max(tool_box.origin_mm.x),
                target_box.origin_mm.y.max(tool_box.origin_mm.y),
                target_box.origin_mm.z.max(tool_box.origin_mm.z),
            );
            let target_maximum = target_box.origin_mm + target_box.size_mm;
            let tool_maximum = tool_box.origin_mm + tool_box.size_mm;
            let maximum = Vec3::new(
                target_maximum.x.min(tool_maximum.x),
                target_maximum.y.min(tool_maximum.y),
                target_maximum.z.min(tool_maximum.z),
            );
            result_box.origin_mm = minimum;
            result_box.size_mm = maximum - minimum;
        }
        let selection_after = SelectionId {
            definition_id: result_definition_id,
            instance_path: target_selection.instance_path.clone(),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        self.occurrence_operation_preview = Some(OccurrenceOperationPreview {
            source_revision: snapshot.revision_id(),
            command_digest: batch.digest(),
            batch,
            boxes: BTreeMap::from([(target_occurrence_id, result_box)]),
            hidden_occurrences: (!keep_tool)
                .then_some(tool_occurrence_id)
                .into_iter()
                .collect(),
            selection_after: Some(selection_after),
            committed_digest_key: match (operation, keep_tool) {
                (BooleanOperation::Cut, false) => "digest-solid-subtract-committed",
                (BooleanOperation::Cut, true) => "digest-solid-subtract-kept-committed",
                (BooleanOperation::Union, false) => "digest-solid-union-committed",
                (BooleanOperation::Union, true) => "digest-solid-union-kept-committed",
                (BooleanOperation::Intersect, false) => "digest-solid-intersect-committed",
                (BooleanOperation::Intersect, true) => "digest-solid-intersect-kept-committed",
                (BooleanOperation::Split, _) => "digest-solid-split-committed",
            },
        });
        self.status_key = "status-solid-tool-preview";
        self.digest = self.catalog.format(
            "digest-solid-tool-live",
            &BTreeMap::from([
                ("operation", operation_label),
                (
                    "tool",
                    self.catalog.text(if keep_tool {
                        "solid-tool-keep-enabled"
                    } else {
                        "solid-tool-keep-disabled"
                    }),
                ),
            ]),
        );
        true
    }

    fn select_solid_tool_occurrence(&mut self, selection: Option<SelectionId>, keep_tool: bool) {
        let Some(selection) = selection else {
            self.digest = self.catalog.text("digest-solid-tool-invalid");
            return;
        };
        if self.solid_tool_candidate(&selection).is_none() {
            self.digest = self.catalog.text("digest-solid-tool-invalid");
            return;
        }
        if self.solid_tool_target.is_none() {
            self.solid_tool_target = Some(selection.clone());
            self.selection.select_exact(selection, false);
            self.status_key = if self.active_solid_tool_operation() == Some(BooleanOperation::Split)
            {
                "status-solid-split-tool"
            } else {
                "status-solid-tool-tool"
            };
            self.digest = self.catalog.text(
                if self.active_solid_tool_operation() == Some(BooleanOperation::Split) {
                    "digest-solid-split-target-selected"
                } else {
                    "digest-solid-tool-target-selected"
                },
            );
            return;
        }
        self.prepare_solid_tool_preview(selection, keep_tool);
    }

    #[must_use]
    pub fn has_occurrence_operation_preview(&self) -> bool {
        self.occurrence_operation_preview
            .as_ref()
            .is_some_and(|preview| {
                preview.source_revision == self.document.current().revision_id()
                    && preview.command_digest == preview.batch.digest()
            })
    }

    #[must_use]
    pub fn occurrence_operation_preview_geometry(
        &self,
        occurrence_id: OccurrenceId,
    ) -> Option<(Vec3, Vec3)> {
        self.occurrence_operation_preview
            .as_ref()?
            .boxes
            .get(&occurrence_id)
            .map(|item| (item.origin_mm, item.size_mm))
    }

    pub fn confirm_occurrence_operation_preview(&mut self) -> bool {
        if !self.has_occurrence_operation_preview() {
            self.occurrence_operation_preview = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.occurrence_operation_preview.take() else {
            return false;
        };
        if self.document.apply_batch(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        if let Some(selection) = preview.selection_after {
            self.selection.select_exact(selection, false);
        }
        self.solid_tool_target = None;
        self.status_key = "status-ready";
        self.digest = self.catalog.text(preview.committed_digest_key);
        true
    }

    fn copy_selection_to_clipboard(&mut self) -> bool {
        let ids = self
            .selected_occurrence_ids()
            .into_iter()
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return false;
        }
        self.occurrence_clipboard = ids;
        self.digest = self.catalog.format(
            "digest-copied-to-clipboard",
            &BTreeMap::from([("count", self.occurrence_clipboard.len().to_string())]),
        );
        true
    }

    fn paste_clipboard(&mut self) -> bool {
        let snapshot = self.document.current();
        let mut next_id = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            + 1;
        let mut additions_by_definition = BTreeMap::<DefinitionId, usize>::new();
        let mut commands = Vec::new();
        let mut pasted = Vec::new();

        for source_id in self.occurrence_clipboard.iter().copied() {
            let Some(source) = snapshot.occurrence(source_id) else {
                continue;
            };
            let definition_id = source.definition_id();
            let Some(definition) = snapshot.definition(definition_id) else {
                continue;
            };
            let Ok(transform) =
                translated_transform(source.transform(), Vec3::new(100.0, 100.0, 0.0))
            else {
                continue;
            };
            let existing = snapshot
                .scene_query()
                .into_iter()
                .filter(|item| item.definition_id == definition_id)
                .count();
            let added = additions_by_definition.entry(definition_id).or_default();
            *added += 1;
            let target_id = OccurrenceId(next_id);
            next_id += 1;
            commands.push(CanonicalCommand::CreateOccurrence {
                id: target_id,
                definition_id,
                name: self.catalog.format(
                    "model-copy-occurrence",
                    &BTreeMap::from([
                        ("name", definition.name().to_owned()),
                        ("number", (existing + *added).to_string()),
                    ]),
                ),
                transform,
                parent: source.parent(),
                tag: source.tag(),
                visible: source.visible(),
            });
            pasted.push((target_id, definition_id));
        }
        if commands.is_empty()
            || self
                .document
                .apply_batch(&CommandBatch::new(commands))
                .is_err()
        {
            return false;
        }

        self.selection.clear();
        self.selection
            .occurrences
            .extend(pasted.iter().map(|(id, _)| InstancePath::root(*id)));
        if let Some((occurrence_id, definition_id)) = pasted.first().copied() {
            self.selection.primary = Some(SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            });
        }
        self.status_key = "status-object-copied";
        self.digest = self.catalog.format(
            "digest-pasted-from-clipboard",
            &BTreeMap::from([("count", pasted.len().to_string())]),
        );
        true
    }

    pub fn rotate_selected_90(&mut self) -> bool {
        let Some(selection) = &self.selection.primary else {
            return false;
        };
        let snapshot = self.document.current();
        if !selection.instance_path.is_root() {
            return false;
        }
        let occurrence_id = selection.instance_path.root_occurrence();
        let Some(occurrence) = snapshot.occurrence(occurrence_id) else {
            return false;
        };
        let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
        let Some(local_box) = projection
            .occurrences()
            .iter()
            .find(|projected| projected.occurrence_id == occurrence_id)
            .and_then(|projected| projected.local_box)
        else {
            return false;
        };
        let Ok(transform) = rotate_transform_90(occurrence.transform(), local_box) else {
            return false;
        };
        if self
            .document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: occurrence_id,
                    transform,
                },
            ]))
            .is_err()
        {
            return false;
        }
        self.status_key = "status-object-rotated";
        true
    }

    pub fn delete_selected(&mut self) -> bool {
        let commands = self
            .selected_occurrence_ids()
            .into_iter()
            .map(|id| CanonicalCommand::DeleteOccurrence { id })
            .collect::<Vec<_>>();
        if commands.is_empty()
            || self
                .document
                .apply_batch(&CommandBatch::new(commands))
                .is_err()
        {
            return false;
        }
        self.selection.clear();
        self.preview = None;
        self.preview_box = None;
        self.preview_definition_id = None;
        self.status_key = "status-object-deleted";
        true
    }

    #[must_use]
    pub fn selected_reference(&self) -> Option<SelectionId> {
        self.selection.primary.clone()
    }

    fn push_pull_face_selected(&self) -> bool {
        matches!(
            self.selection
                .primary
                .as_ref()
                .map(|selection| selection.element.clone()),
            Some(ElementId::Face { .. })
        )
    }

    pub fn set_push_pull_distance_input(&mut self, value: impl Into<String>) {
        self.push_pull_distance_input = value.into();
    }

    fn prepare_smart_push_pull_proposal(
        &self,
        batch: CommandBatch,
        principal: ProposalPrincipal,
    ) -> Option<Proposal> {
        let context = match principal {
            ProposalPrincipal::ManualClient => ProposalContext::canonical_preview(),
            ProposalPrincipal::LocalAssistant => ProposalContext::local_assistant_model(),
            _ => return None,
        };
        self.prepare_smart_push_pull_proposal_with_context(batch, context)
    }

    fn prepare_smart_push_pull_proposal_with_context(
        &self,
        batch: CommandBatch,
        context: ProposalContext,
    ) -> Option<Proposal> {
        self.document
            .prepare_proposal_with_context(batch, context)
            .ok()
    }

    fn push_pull_planning_snapshot(&self) -> Snapshot {
        match self.smart_push_pull_planning.as_ref() {
            Some(SmartPushPullPlanning::TipReplacement(parent)) => parent.snapshot().clone(),
            _ => self.document.current(),
        }
    }

    fn prepare_manual_push_pull_proposal(
        &self,
        batch: CommandBatch,
    ) -> Option<SmartPushPullProposal> {
        match self.smart_push_pull_planning.as_ref() {
            Some(SmartPushPullPlanning::TipReplacement(parent)) => self
                .document
                .prepare_tip_replacement_proposal(
                    parent,
                    batch,
                    ProposalContext::canonical_preview(),
                )
                .ok()
                .map(SmartPushPullProposal::TipReplacement),
            _ => self
                .prepare_smart_push_pull_proposal(batch, ProposalPrincipal::ManualClient)
                .map(SmartPushPullProposal::Append),
        }
    }

    fn circular_hole_targets(
        &self,
        selection: &SelectionId,
        tool_box: &RenderBox,
        distance_mm: f64,
    ) -> Vec<RenderBox> {
        if distance_mm >= -0.01
            || tool_box.extrusion_feature_id.is_some()
            || !selection.instance_path.is_root()
            || selection.element
                != (ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                })
        {
            return Vec::new();
        }
        let snapshot = self.push_pull_planning_snapshot();
        let Some(FeatureKind::SegmentProfile { segments, closed }) = snapshot
            .feature(tool_box.profile_feature_id)
            .map(|feature| feature.kind())
        else {
            return Vec::new();
        };
        if exact_circle_geometry(segments, *closed).is_none() {
            return Vec::new();
        }
        let depth_mm = -distance_mm;
        let mut targets = self
            .active_boxes_for_snapshot(&snapshot)
            .into_iter()
            .filter(|candidate| {
                candidate.instance_path != selection.instance_path
                    && candidate.instance_path.is_root()
                    && candidate.extrusion_feature_id.is_some()
                    && (candidate.origin_mm.z + candidate.size_mm.z - tool_box.origin_mm.z).abs()
                        <= 1.0e-8
                    && (candidate.size_mm.z - depth_mm).abs() <= 1.0e-8
                    && tool_box.origin_mm.x > candidate.origin_mm.x
                    && tool_box.origin_mm.y > candidate.origin_mm.y
                    && tool_box.origin_mm.x + tool_box.size_mm.x
                        < candidate.origin_mm.x + candidate.size_mm.x
                    && tool_box.origin_mm.y + tool_box.size_mm.y
                        < candidate.origin_mm.y + candidate.size_mm.y
            })
            .collect::<Vec<_>>();
        targets.sort_by_key(|target| target.instance_path.root_occurrence());
        targets
    }

    fn prepare_circular_hole_preview(
        &mut self,
        selection: &SelectionId,
        tool_box: &RenderBox,
        distance_mm: f64,
        target_box: RenderBox,
    ) -> bool {
        let snapshot = self.push_pull_planning_snapshot();
        let depth_mm = -distance_mm;
        let target_occurrence_id = target_box.instance_path.root_occurrence();
        let tool_occurrence_id = selection.instance_path.root_occurrence();
        let Some(target_feature_id) = target_box.extrusion_feature_id else {
            return false;
        };
        let Some(first_feature_value) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            return false;
        };
        let tool_feature_id = FeatureId(first_feature_value);
        let mut result_feature_ids = [FeatureId(0); 5];
        for (offset, id) in result_feature_ids.iter_mut().enumerate() {
            let Some(value) = first_feature_value.checked_add(offset as u64 + 1) else {
                return false;
            };
            *id = FeatureId(value);
        }
        let Some(result_definition_id) = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(DefinitionId)
        else {
            return false;
        };
        let (Some(target_occurrence), Some(tool_occurrence)) = (
            snapshot.occurrence(target_occurrence_id),
            snapshot.occurrence(tool_occurrence_id),
        ) else {
            return false;
        };
        let mut tool_matrix = *tool_occurrence.transform().matrix();
        tool_matrix[11] = target_occurrence.transform().matrix()[11];
        let Ok(tool_transform) = Transform::from_matrix(tool_matrix) else {
            return false;
        };
        let operation_label = self.catalog.text("solid-tool-subtract");
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: tool_feature_id,
                definition_id: selection.definition_id,
                name: self.catalog.text("model-default-extrusion"),
                kind: FeatureKind::Extrusion {
                    profile: tool_box.profile_feature_id,
                    height: Dimension::new(format_height(depth_mm), depth_mm)
                        .expect("validated circular-hole depth is canonical"),
                },
            },
            CanonicalCommand::SetOccurrenceTransform {
                id: tool_occurrence_id,
                transform: tool_transform,
            },
            CanonicalCommand::ApplySolidTool(SolidToolPlan {
                operation: BooleanOperation::Cut,
                target_occurrence_id,
                target_feature_id,
                tool_occurrence_id,
                tool_feature_id,
                result_definition_id,
                result_feature_ids,
                result_definition_name: self.catalog.format(
                    "solid-tool-result-definition",
                    &BTreeMap::from([("operation", operation_label.clone())]),
                ),
                result_feature_name: self.catalog.format(
                    "solid-tool-result-feature",
                    &BTreeMap::from([("operation", operation_label.clone())]),
                ),
                keep_tool: false,
            }),
            CanonicalCommand::DeleteDefinition {
                id: selection.definition_id,
            },
        ]);
        let Some(proposal) = self.prepare_manual_push_pull_proposal(batch.clone()) else {
            return false;
        };
        let Some(preview_snapshot) = proposal.preview(&self.document) else {
            return false;
        };
        let Ok(exact_request) =
            ExactFeatureChainRequest::from_snapshot(&preview_snapshot, result_definition_id)
        else {
            return false;
        };
        if exact_request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.circle)
            .is_none()
        {
            return false;
        }
        let selection_after = SelectionId {
            definition_id: result_definition_id,
            instance_path: target_box.instance_path.clone(),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        self.preview = None;
        self.preview_box = None;
        self.preview_definition_id = None;
        self.smart_push_pull_proposal = Some(proposal);
        self.smart_push_pull_chooser = None;
        self.occurrence_operation_preview = Some(OccurrenceOperationPreview {
            source_revision: self.document.current().revision_id(),
            command_digest: batch.digest(),
            batch,
            boxes: BTreeMap::from([(target_occurrence_id, target_box)]),
            hidden_occurrences: BTreeSet::from([tool_occurrence_id]),
            selection_after: Some(selection_after),
            committed_digest_key: "digest-solid-subtract-committed",
        });
        self.status_key = "status-preview";
        self.digest = self.catalog.format(
            "digest-solid-tool-live",
            &BTreeMap::from([
                ("operation", operation_label),
                ("tool", self.catalog.text("solid-tool-keep-disabled")),
            ]),
        );
        true
    }

    pub fn start_preview(&mut self) -> bool {
        self.start_preview_for(SmartPushPullPlanning::Append)
    }

    fn start_preview_for(&mut self, planning: SmartPushPullPlanning) -> bool {
        self.smart_push_pull_planning = Some(planning.clone());
        let Some(selection) = self.selection.primary.clone() else {
            self.clear_ephemeral_edit_state();
            self.status_key = "error-push-pull-selection-required";
            self.digest = self.catalog.text("error-push-pull-selection-required");
            return false;
        };
        if !self.occurrence_in_active_context(&selection.instance_path) {
            return false;
        }
        let Some(distance_mm) = parse_distance_mm(&self.push_pull_distance_input) else {
            return false;
        };
        let planning_snapshot = self.push_pull_planning_snapshot();
        let Some(item) = self
            .active_boxes_for_snapshot(&planning_snapshot)
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)
        else {
            return false;
        };
        let targets = self.circular_hole_targets(&selection, &item, distance_mm);
        if !targets.is_empty() {
            let snapshot = self.document.current();
            self.preview = None;
            self.preview_box = None;
            self.preview_definition_id = None;
            self.smart_push_pull_proposal = None;
            self.occurrence_operation_preview = None;
            self.smart_push_pull_chooser = Some(SmartPushPullChooser {
                source_document_id: snapshot.document_id(),
                source_revision: snapshot.revision_id(),
                source_digest: snapshot.canonical_digest(),
                selection,
                distance_mm,
                planning,
                targets,
                selected: SmartPushPullChoice::NewFeature,
            });
            self.status_key = "status-push-pull-choice";
            self.digest = self.catalog.text("choice-smart-push-pull-source");
            return true;
        }
        self.prepare_box_push_pull_preview(
            selection,
            item,
            distance_mm,
            ProposalPrincipal::ManualClient,
        )
    }

    fn prepare_box_push_pull_preview(
        &mut self,
        selection: SelectionId,
        item: RenderBox,
        distance_mm: f64,
        principal: ProposalPrincipal,
    ) -> bool {
        let Some(current_extent_mm) = face_extent(&item, Some(&selection.element)) else {
            return false;
        };
        let new_extent_mm = current_extent_mm + distance_mm;
        let after = resize_box_from_face(&item, &selection.element, new_extent_mm).or_else(|| {
            (item.extrusion_feature_id.is_none()
                && new_extent_mm < 0.0
                && selection.element
                    == (ElementId::Face {
                        axis: Axis::Z,
                        side: Side::Maximum,
                    }))
            .then(|| {
                let mut after = item.clone();
                after.origin_mm.z += new_extent_mm;
                after.size_mm.z = -new_extent_mm;
                after
            })
        });
        let Some(after) = after else {
            return false;
        };
        let snapshot = self.push_pull_planning_snapshot();
        let Some(batch) = push_pull_batch(
            &snapshot,
            &selection,
            &item,
            new_extent_mm,
            format_height(new_extent_mm),
        ) else {
            return false;
        };
        let proposal = if principal == ProposalPrincipal::ManualClient {
            self.prepare_manual_push_pull_proposal(batch.clone())
        } else {
            self.prepare_smart_push_pull_proposal(batch.clone(), principal)
                .map(SmartPushPullProposal::Append)
        };
        let Some(proposal) = proposal else {
            return false;
        };
        let command_digest = proposal.command_digest().to_owned();
        self.preview = Some(batch);
        self.smart_push_pull_proposal = Some(proposal);
        self.smart_push_pull_chooser = None;
        self.preview_box = Some(EphemeralBoxPreview {
            source_document_id: self.document.current().document_id(),
            source_revision: self.document.current().revision_id(),
            source_digest: self.document.current().canonical_digest(),
            context: self.selection.edit_context.last().cloned(),
            selection_state: self.selection.primary.clone(),
            target: selection.clone(),
            command_digest,
            box_data: after,
        });
        self.preview_definition_id = Some(selection.definition_id);
        self.status_key = "status-preview";
        let shared_count = snapshot
            .scene_query()
            .into_iter()
            .find(|candidate| candidate.instance_path == selection.instance_path)
            .map_or(1, |candidate| candidate.shared_occurrence_count);
        self.digest = match &selection.element {
            ElementId::Face { axis: Axis::Z, .. } => self.catalog.format(
                "digest-push-pull-live",
                &BTreeMap::from([
                    ("distance", format_signed_mm(distance_mm)),
                    ("height", format_height(new_extent_mm)),
                    ("count", shared_count.to_string()),
                ]),
            ),
            ElementId::Face { .. } => self.catalog.format(
                "digest-push-pull-profile-live",
                &BTreeMap::from([
                    ("distance", format_signed_mm(distance_mm)),
                    ("extent", format_height(new_extent_mm)),
                ]),
            ),
            _ => self.catalog.text("digest-nothing-to-apply"),
        };
        true
    }

    fn has_preview(&self) -> bool {
        let Some(expected_target) = self.selection.primary.as_ref() else {
            return false;
        };
        self.preview_box.as_ref().is_some_and(|preview| {
            let snapshot = self.document.current();
            preview.source_document_id == snapshot.document_id()
                && preview.source_revision == snapshot.revision_id()
                && preview.source_digest == snapshot.canonical_digest()
                && preview.context == self.selection.edit_context.last().cloned()
                && preview.selection_state == self.selection.primary
                && &preview.target == expected_target
                && self
                    .preview
                    .as_ref()
                    .is_some_and(|batch| batch.digest() == preview.command_digest)
                && self
                    .smart_push_pull_proposal
                    .as_ref()
                    .is_some_and(|proposal| {
                        proposal.command_digest() == preview.command_digest
                            && proposal.batch().digest() == preview.command_digest
                            && proposal.is_current(&snapshot)
                    })
        })
    }

    #[must_use]
    pub fn push_pull_preview_exact_evaluator(&self) -> Option<&'static str> {
        let (batch, definition_id) = if self.has_preview() {
            (self.preview.as_ref()?, self.preview_definition_id?)
        } else if self.has_occurrence_operation_preview() {
            let preview = self.occurrence_operation_preview.as_ref()?;
            (
                &preview.batch,
                preview.selection_after.as_ref()?.definition_id,
            )
        } else {
            return None;
        };
        let snapshot = if let Some(proposal) = self.smart_push_pull_proposal.as_ref() {
            if proposal.batch() != batch {
                return None;
            }
            proposal.preview(&self.document)?
        } else {
            self.document.preview_batch(batch).ok()?
        };
        ExactFeatureChainRequest::from_snapshot(&snapshot, definition_id)
            .ok()
            .map(|request| request.evaluator())
    }

    #[must_use]
    pub const fn has_smart_push_pull_chooser(&self) -> bool {
        self.smart_push_pull_chooser.is_some()
    }

    fn confirm_smart_push_pull_choice(&mut self) -> bool {
        let Some(chooser) = self.smart_push_pull_chooser.take() else {
            return false;
        };
        let snapshot = self.document.current();
        if chooser.source_document_id != snapshot.document_id()
            || chooser.source_revision != snapshot.revision_id()
            || chooser.source_digest != snapshot.canonical_digest()
            || self.selection.primary.as_ref() != Some(&chooser.selection)
        {
            self.status_key = "error-preview-stale";
            self.digest = self.catalog.text("error-preview-stale");
            return false;
        }
        self.smart_push_pull_planning = Some(chooser.planning.clone());
        let planning_snapshot = self.push_pull_planning_snapshot();
        let Some(tool_box) = self
            .active_boxes_for_snapshot(&planning_snapshot)
            .into_iter()
            .find(|item| item.instance_path == chooser.selection.instance_path)
        else {
            self.status_key = "error-preview-stale";
            return false;
        };
        match chooser.selected {
            SmartPushPullChoice::NewFeature => self.prepare_box_push_pull_preview(
                chooser.selection,
                tool_box,
                chooser.distance_mm,
                ProposalPrincipal::ManualClient,
            ),
            SmartPushPullChoice::CircularCut(target_id) => {
                let Some(target) = chooser
                    .targets
                    .into_iter()
                    .find(|target| target.instance_path.root_occurrence() == target_id)
                else {
                    self.status_key = "error-preview-stale";
                    return false;
                };
                self.prepare_circular_hole_preview(
                    &chooser.selection,
                    &tool_box,
                    chooser.distance_mm,
                    target,
                )
            }
        }
    }

    fn confirm_circular_hole_preview(&mut self) -> bool {
        if !self.has_occurrence_operation_preview() {
            self.smart_push_pull_proposal = None;
            self.occurrence_operation_preview = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(proposal) = self.smart_push_pull_proposal.take() else {
            return false;
        };
        let Some(preview) = self.occurrence_operation_preview.take() else {
            return false;
        };
        let snapshot = self.document.current();
        if !proposal.is_current(&snapshot)
            || proposal.command_digest() != preview.command_digest
            || proposal.batch() != &preview.batch
        {
            self.status_key = "error-preview-stale";
            return false;
        }
        if !proposal.commit(&mut self.document) {
            self.status_key = "error-preview-stale";
            return false;
        }
        if let Some(selection) = preview.selection_after {
            self.selection.select_exact(selection, false);
        }
        self.status_key = "status-ready";
        self.digest = self.catalog.text(preview.committed_digest_key);
        true
    }

    fn confirm_push_pull_preview(&mut self) -> bool {
        if self.has_occurrence_operation_preview() && self.smart_push_pull_proposal.is_some() {
            self.confirm_circular_hole_preview()
        } else if self.has_occurrence_operation_preview() {
            self.confirm_occurrence_operation_preview()
        } else {
            self.confirm_preview()
        }
    }

    pub fn cancel_preview(&mut self) {
        self.clear_ephemeral_edit_state();
        self.status_key = "status-ready";
    }

    fn clear_ephemeral_edit_state(&mut self) {
        self.preview = None;
        self.preview_box = None;
        self.preview_definition_id = None;
        self.smart_push_pull_proposal = None;
        self.smart_push_pull_planning = None;
        self.smart_push_pull_chooser = None;
        self.occurrence_operation_preview = None;
        self.solid_tool_target = None;
        self.revolve_tool = None;
        self.revolve_preview = None;
        self.planar_offset_preview = None;
        self.sweep_preview = None;
        self.loft_preview = None;
        self.general_finish_preview = None;
        self.pocket_preview = None;
        self.push_pull_drag = None;
        self.bottle_direct_drag = None;
        self.move_drag = None;
        self.move_anchor = None;
        self.clear_measurement();
    }

    fn reconcile_selection(&mut self) {
        let snapshot = self.document.current();
        self.selection
            .occurrences
            .retain(|path| snapshot.resolve_instance_path(path).is_ok());
        if self.selection.primary.as_ref().is_some_and(|selection| {
            snapshot
                .resolve_instance_path(&selection.instance_path)
                .is_err()
        }) {
            self.selection.primary = None;
        }
        if self
            .selection
            .selected_group
            .is_some_and(|group_id| snapshot.group(group_id).is_none())
        {
            self.selection.selected_group = None;
        }
        while self
            .selection
            .edit_context
            .last()
            .is_some_and(|context| match context {
                EditContext::Group(group_id) => snapshot.group(*group_id).is_none(),
                EditContext::Definition {
                    definition_id,
                    instance_path,
                } => snapshot
                    .bind_scene_query(SceneQueryContext::Definition {
                        definition_id: *definition_id,
                        instance_path: instance_path.clone(),
                    })
                    .is_err(),
            })
        {
            self.selection.edit_context.pop();
        }
        if !self.selection.edit_context.is_empty() {
            let allowed_paths = self
                .active_scene_query()
                .into_iter()
                .map(|occurrence| occurrence.instance_path)
                .collect::<BTreeSet<_>>();
            self.selection
                .occurrences
                .retain(|path| allowed_paths.contains(path));
            if self
                .selection
                .primary
                .as_ref()
                .is_some_and(|selection| !allowed_paths.contains(&selection.instance_path))
            {
                self.selection.primary = None;
            }
        }
        if self
            .last_move
            .is_some_and(|operation| snapshot.occurrence(operation.occurrence_id).is_none())
        {
            self.last_move = None;
        }
        self.push_pull_distance_input.clear();
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.document.visible_undo_steps() > 0
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.document.visible_redo_steps() > 0
    }

    #[must_use]
    pub fn undo_step_count(&self) -> usize {
        self.document.visible_undo_steps()
    }

    #[must_use]
    pub fn redo_step_count(&self) -> usize {
        self.document.visible_redo_steps()
    }

    pub fn undo(&mut self) -> bool {
        let undoing_assistant_change = self.assistant_change_can_undo();
        if self.document.undo().is_none() {
            return false;
        }
        if undoing_assistant_change {
            self.assistant_verification = None;
        }
        self.clear_ephemeral_edit_state();
        self.parameter_editor_node = None;
        self.parameter_provenance = None;
        self.parameter_last_recomputed_nodes.clear();
        self.reconcile_selection();
        self.status_key = "status-undo";
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.document.redo().is_none() {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.parameter_editor_node = None;
        self.parameter_provenance = None;
        self.parameter_last_recomputed_nodes.clear();
        self.reconcile_selection();
        self.status_key = "status-redo";
        true
    }

    pub fn confirm_preview(&mut self) -> bool {
        if !self.has_preview() {
            self.preview = None;
            self.preview_box = None;
            self.preview_definition_id = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(batch) = self.preview.take() else {
            return false;
        };
        let Some(proposal) = self.smart_push_pull_proposal.take() else {
            return false;
        };
        if proposal.batch() != &batch || !proposal.commit(&mut self.document) {
            self.preview_box = None;
            self.preview_definition_id = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let committed_extent = self.selection.primary.as_ref().and_then(|selection| {
            self.preview_box
                .as_ref()
                .and_then(|item| face_extent(&item.box_data, Some(&selection.element)))
        });
        self.preview_box = None;
        self.preview_definition_id = None;
        self.status_key = "status-ready";
        if let Some(selection) = self.selection.primary.clone() {
            self.last_push_pull = Some(LastPushPull {
                selection: selection.clone(),
                revision: self.document_revision(),
                canonical_digest: self.canonical_digest(),
            });
            self.digest = self.catalog.format(
                match selection.element {
                    ElementId::Face { axis: Axis::Z, .. } => "digest-push-pull-committed-height",
                    _ => "digest-push-pull-committed-profile",
                },
                &BTreeMap::from([
                    (
                        "distance",
                        parse_distance_mm(&self.push_pull_distance_input)
                            .map_or_else(String::new, format_signed_mm),
                    ),
                    (
                        "height",
                        committed_extent.map_or_else(String::new, format_height),
                    ),
                ]),
            );
        }
        true
    }

    #[must_use]
    pub fn preview_action_digest(&self) -> Option<String> {
        if !self.has_preview() {
            return None;
        }
        let selection = self.selection.primary.clone()?;
        let snapshot = self.document.current();
        let resolved = snapshot
            .resolve_instance_path(&selection.instance_path)
            .ok()?;
        if resolved.definition_id != selection.definition_id {
            return None;
        }
        let definition = snapshot.definition(resolved.definition_id)?;
        let from = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)
            .and_then(|item| face_extent(&item, Some(&selection.element)))?;
        let to = self
            .preview_box
            .as_ref()
            .and_then(|item| face_extent(&item.box_data, Some(&selection.element)))?;
        Some(self.catalog.format(
            "action-smart-push-pull-height",
            &BTreeMap::from([
                ("feature", definition.name().to_owned()),
                ("from", format_height(from)),
                ("to", format_height(to)),
            ]),
        ))
    }

    fn selected_face_extent_mm(&self) -> f64 {
        self.selection
            .primary
            .as_ref()
            .and_then(|selection| {
                self.selected_box()
                    .and_then(|item| face_extent(&item, Some(&selection.element)))
            })
            .unwrap_or_else(|| self.document_height_mm())
    }

    fn bottle_direct_target(
        &self,
        definition_id: DefinitionId,
        role: ExactFaceRole,
    ) -> Option<(FeatureId, BottleControlDimension, f64)> {
        let snapshot = self.document.current();
        let ids = Self::bottle_feature_ids(&snapshot, definition_id)?;
        let control = match role {
            ExactFaceRole::RevolveBody | ExactFaceRole::ShellOuterBody => {
                BottleControlDimension::BodyRadius
            }
            ExactFaceRole::RevolveShoulder
            | ExactFaceRole::ShellOuterShoulder
            | ExactFaceRole::ShellInnerShoulder => BottleControlDimension::ShoulderRise,
            ExactFaceRole::RevolveNeck
            | ExactFaceRole::RevolveMouth
            | ExactFaceRole::ShellOuterNeck
            | ExactFaceRole::ShellInnerNeck
            | ExactFaceRole::ShellRim => BottleControlDimension::BodyHeight,
            _ => return None,
        };
        let FeatureKind::BottleProfileControl {
            body_radius,
            body_height,
            shoulder_rise,
            ..
        } = snapshot.feature(ids.control)?.kind()
        else {
            return None;
        };
        let value = match control {
            BottleControlDimension::BodyRadius => body_radius.millimetres(),
            BottleControlDimension::BodyHeight => body_height.millimetres(),
            BottleControlDimension::ShoulderRise => shoulder_rise.millimetres(),
        };
        Some((ids.control, control, value))
    }

    fn begin_bottle_direct_drag(&mut self, pointer: Pos2, rect: Rect) -> bool {
        let ray = match self.view_ray(pointer, rect) {
            Some(ray) => ray,
            None => return false,
        };
        let snapshot = self.document.current();
        let hit = match self.exact_projection(&snapshot).exact_surface_pick(ray) {
            Some(hit) => hit,
            None => return false,
        };
        let role = match hit
            .durable_target
            .as_ref()
            .and_then(|target| target.body.role())
        {
            Some(role) => role,
            None => return false,
        };
        let Some((feature_id, control, value_start_mm)) =
            self.bottle_direct_target(hit.definition_id, role)
        else {
            return false;
        };
        let direction_world = if control == BottleControlDimension::BodyRadius {
            hit.outward_normal
        } else {
            Vec3::new(0.0, 0.0, 1.0)
        };
        let projected = self.project(hit.position_mm + direction_world, rect)
            - self.project(hit.position_mm, rect);
        let pixels_per_mm = projected.length();
        if pixels_per_mm <= 1.0e-4 {
            return false;
        }
        let element = exact_face_element(role)
            .or_else(|| exact_surface_element(hit.outward_normal))
            .unwrap_or(ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            });
        self.select_from_viewport(
            Some(SelectionId {
                definition_id: hit.definition_id,
                instance_path: hit.instance_path,
                element,
            }),
            false,
        );
        self.bottle_direct_drag = Some(BottleDirectDrag {
            definition_id: hit.definition_id,
            feature_id,
            control,
            pointer_start: pointer,
            value_start_mm,
            screen_direction: projected / pixels_per_mm,
            pixels_per_mm,
        });
        self.value_input = format_height(value_start_mm);
        true
    }

    fn bottle_direct_value(drag: BottleDirectDrag, pointer: Pos2) -> f64 {
        let delta = f64::from((pointer - drag.pointer_start).dot(drag.screen_direction))
            / f64::from(drag.pixels_per_mm);
        ((drag.value_start_mm + delta) * 2.0).round() / 2.0
    }

    fn commit_bottle_direct_drag(&mut self, drag: BottleDirectDrag, value_mm: f64) -> bool {
        let source = format_height(value_mm);
        let Ok(dimension) = Dimension::new(source, value_mm) else {
            return false;
        };
        let batch = CommandBatch::new(vec![CanonicalCommand::SetBottleControlDimension {
            id: drag.feature_id,
            control: drag.control,
            dimension,
        }]);
        if self.document.apply_batch(&batch).is_err() {
            self.digest = "Bottle direct edit rejected; canonical document unchanged".to_owned();
            return false;
        }
        self.bottle_editor =
            Self::bottle_editor_inputs(&self.document.current(), drag.definition_id);
        self.digest = format!(
            "Bottle {:?} direct edit committed at {} mm; exact re-evaluation pending",
            drag.control,
            format_height(value_mm)
        );
        true
    }

    fn push_pull_screen_projection(
        &self,
        selection: &SelectionId,
        rect: Rect,
    ) -> Option<(Vec2, f32)> {
        let item = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)?;
        let ElementId::Face { axis, side } = selection.element else {
            return None;
        };
        let sign = if side == Side::Maximum { 1.0 } else { -1.0 };
        let mut center = item.origin_mm + item.size_mm * 0.5;
        let normal = match axis {
            Axis::X => {
                center.x = if side == Side::Maximum {
                    item.origin_mm.x + item.size_mm.x
                } else {
                    item.origin_mm.x
                };
                Vec3::new(sign, 0.0, 0.0)
            }
            Axis::Y => {
                center.y = if side == Side::Maximum {
                    item.origin_mm.y + item.size_mm.y
                } else {
                    item.origin_mm.y
                };
                Vec3::new(0.0, sign, 0.0)
            }
            Axis::Z => {
                center.z = if side == Side::Maximum {
                    item.origin_mm.z + item.size_mm.z
                } else {
                    item.origin_mm.z
                };
                Vec3::new(0.0, 0.0, sign)
            }
        };
        let projected = self.project(center + normal, rect) - self.project(center, rect);
        let pixels_per_mm = projected.length();
        if pixels_per_mm > 1.0e-4 {
            Some((projected / pixels_per_mm, pixels_per_mm))
        } else {
            let fallback_scale = self.zoom * rect.width().min(rect.height()) / 420.0;
            Some((Vec2::new(0.0, -1.0), fallback_scale.max(1.0e-4)))
        }
    }

    fn render_box(&self, item: RenderBox) -> RenderBox {
        if self.has_occurrence_operation_preview()
            && item.instance_path.is_root()
            && let Some(preview) = self
                .occurrence_operation_preview
                .as_ref()
                .and_then(|operation| operation.boxes.get(&item.instance_path.root_occurrence()))
        {
            return preview.clone();
        }
        if !self.has_preview() {
            return item;
        }
        let Some(ephemeral) = self.preview_box.as_ref() else {
            return item;
        };
        let preview = &ephemeral.box_data;
        let selection = &ephemeral.target;
        if preview.definition_id != item.definition_id {
            return item;
        }
        let preview_element = match &selection.element {
            ElementId::Face {
                axis,
                side: Side::Minimum,
            } if item.instance_path != selection.instance_path => ElementId::Face {
                axis: *axis,
                side: Side::Maximum,
            },
            element => element.clone(),
        };
        face_extent(preview, Some(&selection.element))
            .and_then(|extent| resize_box_from_face(&item, &preview_element, extent))
            .unwrap_or(item)
    }

    fn move_preview_is_current(&self, drag: &MoveDrag) -> bool {
        let snapshot = self.document.current();
        drag.source_document_id == snapshot.document_id()
            && drag.source_revision == snapshot.revision_id()
            && snapshot
                .occurrence(drag.selection.instance_path.root_occurrence())
                .is_some_and(|occurrence| {
                    occurrence.definition_id() == drag.selection.definition_id
                })
            && drag
                .group_id
                .is_none_or(|group_id| snapshot.group(group_id).is_some())
    }

    fn group_contains_occurrence(
        snapshot: &Snapshot,
        group_id: GroupId,
        occurrence_id: OccurrenceId,
    ) -> bool {
        let mut parent = snapshot
            .occurrence(occurrence_id)
            .and_then(|occurrence| occurrence.parent());
        while let Some(candidate) = parent {
            if candidate == group_id {
                return true;
            }
            parent = snapshot.group(candidate).and_then(|group| group.parent());
        }
        false
    }

    fn move_drag_applies_to_path(&self, drag: &MoveDrag, instance_path: &InstancePath) -> bool {
        if let Some(group_id) = drag.group_id {
            Self::group_contains_occurrence(
                &self.document.current(),
                group_id,
                instance_path.root_occurrence(),
            )
        } else {
            drag.selection.instance_path == *instance_path
        }
    }

    fn move_preview_transform_overrides(&self) -> BTreeMap<InstancePath, Transform> {
        let Some(drag) = self.move_drag.as_ref().or(self.move_anchor.as_ref()) else {
            return BTreeMap::new();
        };
        if !self.move_preview_is_current(drag) || drag.copy {
            return BTreeMap::new();
        }
        self.document
            .current()
            .scene_query()
            .into_iter()
            .filter(|occurrence| self.move_drag_applies_to_path(drag, &occurrence.instance_path))
            .filter_map(|occurrence| {
                translated_transform(occurrence.transform, drag.delta_mm)
                    .ok()
                    .map(|transform| (occurrence.instance_path, transform))
            })
            .collect()
    }

    fn begin_move_drag_at(&mut self, pointer: Pos2, rect: Rect, copy: bool) -> bool {
        let Some(selection) = self
            .hovered
            .clone()
            .filter(|selection| self.occurrence_in_active_context(&selection.instance_path))
        else {
            self.digest = self.catalog.text("digest-move-start-missed");
            return false;
        };
        self.select_from_viewport(Some(selection.clone()), false);
        let plane_z = self
            .hover_pick
            .as_ref()
            .filter(|pick| pick.primary.reference.instance_path == selection.instance_path)
            .map(|pick| pick.primary.position_mm.z)
            .or_else(|| {
                self.active_boxes()
                    .into_iter()
                    .find(|item| item.instance_path == selection.instance_path)
                    .map(|item| item.origin_mm.z)
            });
        let Some(plane_z) = plane_z else {
            return false;
        };
        let Some(pointer_start_world) = self.screen_to_plane(pointer, rect, plane_z) else {
            return false;
        };
        self.value_input = "0".to_owned();
        let snapshot = self.document.current();
        let group_id = self.selection.selected_group;
        self.move_drag = Some(MoveDrag {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            selection,
            group_id,
            pointer_start_world,
            plane_z,
            delta_mm: Vec3::ZERO,
            copy: group_id.is_none() && copy,
        });
        true
    }

    fn proxy_preview_is_active(&self, item: &RenderBox) -> bool {
        let push_pull_preview =
            self.has_preview() && self.preview_definition_id == Some(item.definition_id);
        let move_preview = self
            .move_drag
            .as_ref()
            .or(self.move_anchor.as_ref())
            .is_some_and(|drag| {
                self.move_preview_is_current(drag)
                    && self.move_drag_applies_to_path(drag, &item.instance_path)
            });
        let occurrence_preview = self.has_occurrence_operation_preview()
            && item.instance_path.is_root()
            && self
                .occurrence_operation_preview
                .as_ref()
                .is_some_and(|preview| {
                    preview
                        .boxes
                        .contains_key(&item.instance_path.root_occurrence())
                });
        push_pull_preview || move_preview || occurrence_preview
    }

    fn viewport_boxes(&self, exact_projection: &ExactInteractionProjection) -> Vec<RenderBox> {
        let mut boxes = self.active_boxes();
        if let Some(drag) = self.move_drag.as_ref().or(self.move_anchor.as_ref())
            && self.move_preview_is_current(drag)
        {
            let mut copies = Vec::new();
            for item in boxes
                .iter_mut()
                .filter(|item| self.move_drag_applies_to_path(drag, &item.instance_path))
            {
                let mut preview = item.clone();
                preview.origin_mm = preview.origin_mm + drag.delta_mm;
                if drag.copy && drag.group_id.is_none() {
                    copies.push(preview);
                } else {
                    *item = preview;
                }
            }
            boxes.extend(copies);
        }
        if self.has_occurrence_operation_preview()
            && let Some(operation) = &self.occurrence_operation_preview
        {
            for (occurrence_id, preview_box) in &operation.boxes {
                if !boxes
                    .iter()
                    .any(|item| item.instance_path == InstancePath::root(*occurrence_id))
                {
                    boxes.push(preview_box.clone());
                }
            }
        }
        if self.has_occurrence_operation_preview()
            && let Some(operation) = &self.occurrence_operation_preview
        {
            boxes.retain(|item| {
                !item.instance_path.is_root()
                    || !operation
                        .hidden_occurrences
                        .contains(&item.instance_path.root_occurrence())
            });
        }
        boxes.retain(|item| {
            !exact_projection.contains_occurrence(&item.instance_path)
                || self.proxy_preview_is_active(item)
        });
        boxes
    }

    fn orbit(&mut self, pointer_delta: Vec2) {
        self.yaw += pointer_delta.x * 0.006;
        self.pitch += pointer_delta.y * 0.006;
    }

    /// The first measured point while a measurement is being taken.
    const fn measure_anchor(&self) -> Option<Vec3> {
        match (self.measure_start, self.measure_end) {
            (start, None) => start,
            _ => None,
        }
    }

    /// The measured segment, either finished or following the pointer.
    fn measure_span(&self) -> Option<(Vec3, Vec3)> {
        let start = self.measure_start?;
        let end = self.measure_end.or(self.measure_cursor)?;
        Some((start, end))
    }

    /// Record a measured point. Measuring never changes the document.
    fn add_measured_point(&mut self, point: Vec3) {
        if let Some(start) = self.measure_anchor() {
            self.measure_end = Some(point);
            self.measure_cursor = Some(point);
            self.status_key = "status-ready";
            self.digest = self.measurement_text(start, point, "digest-measured");
        } else {
            self.measure_start = Some(point);
            self.measure_cursor = Some(point);
            self.measure_end = None;
            self.value_input.clear();
            self.status_key = "status-measure-second-point";
        }
    }

    fn measurement_text(&self, start: Vec3, end: Vec3, key: &str) -> String {
        let delta = Vec3::new(end.x - start.x, end.y - start.y, end.z - start.z);
        self.catalog.format(
            key,
            &BTreeMap::from([
                ("distance", format_height(vector_length(delta))),
                ("vector", format_vector_mm(delta)),
            ]),
        )
    }

    fn clear_measurement(&mut self) {
        self.measure_start = None;
        self.measure_cursor = None;
        self.measure_end = None;
    }

    /// The measured distance in millimetres, once both points are placed.
    #[must_use]
    pub fn measured_points(&self) -> Option<(Vec3, Vec3)> {
        Some((self.measure_start?, self.measure_end?))
    }

    /// The measured distance in millimetres, once both points are placed.
    #[must_use]
    pub fn measured_distance_mm(&self) -> Option<f64> {
        let (start, end) = self.measured_points()?;
        Some(vector_length(Vec3::new(
            end.x - start.x,
            end.y - start.y,
            end.z - start.z,
        )))
    }

    fn cancel_rectangle_sketch(&mut self) {
        self.sketch_mode = false;
        self.sketch_start = None;
        self.sketch_end = None;
        self.sketch_cursor = None;
        self.status_key = "status-ready";
    }

    fn complete_through_cut_sketch(&mut self, start: Vec3, end: Vec3) -> bool {
        let Some((selection, item, translation)) = self.through_cut_target() else {
            self.digest = self.catalog.text("digest-cut-through-invalid-target");
            return false;
        };
        let target = item
            .extrusion_feature_id
            .expect("a Through Cut target always has an extrusion");
        let local_start = start - translation;
        let local_end = end - translation;
        let minimum = [
            local_start.x.min(local_end.x),
            local_start.y.min(local_end.y),
        ];
        let maximum = [
            local_start.x.max(local_end.x),
            local_start.y.max(local_end.y),
        ];
        let width = maximum[0] - minimum[0];
        let depth = maximum[1] - minimum[1];
        if !width.is_finite() || !depth.is_finite() || width <= 0.01 || depth <= 0.01 {
            self.digest = self.catalog.text("digest-cut-through-invalid-profile");
            return false;
        }

        let snapshot = self.document.current();
        let profile = snapshot.feature(item.profile_feature_id);
        let Some(FeatureKind::Profile { points_mm }) = profile.map(|feature| feature.kind()) else {
            self.digest = self.catalog.text("digest-cut-through-invalid-target");
            return false;
        };
        let Some(outer_min_x) = points_mm
            .iter()
            .map(|point| point[0])
            .min_by(f64::total_cmp)
        else {
            return false;
        };
        let Some(outer_max_x) = points_mm
            .iter()
            .map(|point| point[0])
            .max_by(f64::total_cmp)
        else {
            return false;
        };
        let Some(outer_min_y) = points_mm
            .iter()
            .map(|point| point[1])
            .min_by(f64::total_cmp)
        else {
            return false;
        };
        let Some(outer_max_y) = points_mm
            .iter()
            .map(|point| point[1])
            .max_by(f64::total_cmp)
        else {
            return false;
        };
        let tolerance = 1.0e-6;
        if minimum[0] <= outer_min_x + tolerance
            || maximum[0] >= outer_max_x - tolerance
            || minimum[1] <= outer_min_y + tolerance
            || maximum[1] >= outer_max_y - tolerance
        {
            self.digest = self.catalog.text("digest-cut-through-invalid-profile");
            return false;
        }

        let next_feature = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let Some(profile_id) = next_feature.map(FeatureId) else {
            return false;
        };
        let Some(cut_id) = profile_id.0.checked_add(1).map(FeatureId) else {
            return false;
        };
        let shared_count = snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.definition_id == selection.definition_id)
            .count();
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: profile_id,
                definition_id: selection.definition_id,
                name: self.catalog.text("model-cut-through-profile"),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [minimum[0], minimum[1]],
                        [maximum[0], minimum[1]],
                        [maximum[0], maximum[1]],
                        [minimum[0], maximum[1]],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: cut_id,
                definition_id: selection.definition_id,
                name: self.catalog.text("feature-cut-through"),
                kind: FeatureKind::ThroughCut {
                    target,
                    profile: profile_id,
                },
            },
        ]);
        if self.document.apply_batch(&batch).is_err() {
            self.digest = self.catalog.text("digest-cut-through-invalid-profile");
            return false;
        }
        self.sketch_mode = false;
        self.sketch_start = None;
        self.sketch_end = None;
        self.sketch_cursor = None;
        self.value_input.clear();
        self.status_key = "status-cut-through-created";
        self.digest = self.catalog.format(
            "digest-cut-through-committed",
            &BTreeMap::from([
                ("width", format_height(width)),
                ("depth", format_height(depth)),
                ("count", shared_count.to_string()),
            ]),
        );
        true
    }

    fn prepare_pocket_preview(&mut self, start: Vec3, end: Vec3, depth_mm: f64) -> bool {
        let Some((selection, item, translation)) = self.through_cut_target() else {
            self.digest = self.catalog.text("digest-pocket-invalid-target");
            return false;
        };
        if !depth_mm.is_finite() || depth_mm <= 0.01 || depth_mm >= item.size_mm.z {
            self.digest = self.catalog.text("digest-pocket-invalid-depth");
            return false;
        }
        let target = item
            .extrusion_feature_id
            .expect("a Pocket target always has an extrusion");
        let local_start = start - translation;
        let local_end = end - translation;
        let minimum = [
            local_start.x.min(local_end.x),
            local_start.y.min(local_end.y),
        ];
        let maximum = [
            local_start.x.max(local_end.x),
            local_start.y.max(local_end.y),
        ];
        let width = maximum[0] - minimum[0];
        let length = maximum[1] - minimum[1];
        if !width.is_finite() || !length.is_finite() || width <= 0.01 || length <= 0.01 {
            self.digest = self.catalog.text("digest-pocket-invalid-profile");
            return false;
        }

        let snapshot = self.document.current();
        let Some(FeatureKind::Profile { points_mm }) = snapshot
            .feature(item.profile_feature_id)
            .map(|feature| feature.kind())
        else {
            self.digest = self.catalog.text("digest-pocket-invalid-target");
            return false;
        };
        let Some(outer_min_x) = points_mm
            .iter()
            .map(|point| point[0])
            .min_by(f64::total_cmp)
        else {
            return false;
        };
        let Some(outer_max_x) = points_mm
            .iter()
            .map(|point| point[0])
            .max_by(f64::total_cmp)
        else {
            return false;
        };
        let Some(outer_min_y) = points_mm
            .iter()
            .map(|point| point[1])
            .min_by(f64::total_cmp)
        else {
            return false;
        };
        let Some(outer_max_y) = points_mm
            .iter()
            .map(|point| point[1])
            .max_by(f64::total_cmp)
        else {
            return false;
        };
        let tolerance = 1.0e-6;
        if minimum[0] <= outer_min_x + tolerance
            || maximum[0] >= outer_max_x - tolerance
            || minimum[1] <= outer_min_y + tolerance
            || maximum[1] >= outer_max_y - tolerance
        {
            self.digest = self.catalog.text("digest-pocket-invalid-profile");
            return false;
        }

        let Some(profile_id) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)
        else {
            return false;
        };
        let Some(pocket_id) = profile_id.0.checked_add(1).map(FeatureId) else {
            return false;
        };
        let shared_count = snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.definition_id == selection.definition_id)
            .count();
        let Ok(depth) = Dimension::new(format_height(depth_mm), depth_mm) else {
            return false;
        };
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: profile_id,
                definition_id: selection.definition_id,
                name: self.catalog.text("model-pocket-profile"),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [minimum[0], minimum[1]],
                        [maximum[0], minimum[1]],
                        [maximum[0], maximum[1]],
                        [minimum[0], maximum[1]],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: pocket_id,
                definition_id: selection.definition_id,
                name: self.catalog.text("feature-pocket"),
                kind: FeatureKind::Pocket {
                    target,
                    profile: profile_id,
                    depth,
                },
            },
        ]);
        self.pocket_preview = Some(PocketPreview {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            selection,
            command_digest: batch.digest(),
            batch,
            start,
            end,
            depth_mm,
            shared_count,
        });
        self.sketch_mode = false;
        self.sketch_start = Some(start);
        self.sketch_cursor = Some(end);
        self.value_input = format_height(depth_mm);
        self.focus_value_box = true;
        self.status_key = "status-pocket-depth";
        self.digest = self.catalog.format(
            "digest-pocket-live",
            &BTreeMap::from([
                ("width", format_height(width)),
                ("length", format_height(length)),
                ("depth", format_height(depth_mm)),
            ]),
        );
        true
    }

    fn has_pocket_preview(&self) -> bool {
        self.pocket_preview.as_ref().is_some_and(|preview| {
            let snapshot = self.document.current();
            preview.source_document_id == snapshot.document_id()
                && preview.source_revision == snapshot.revision_id()
                && preview.source_digest == snapshot.canonical_digest()
                && self.selection.primary.as_ref() == Some(&preview.selection)
                && preview.command_digest == preview.batch.digest()
        })
    }

    fn confirm_pocket_preview(&mut self) -> bool {
        if !self.has_pocket_preview() {
            self.pocket_preview = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.pocket_preview.take() else {
            return false;
        };
        if self.document.apply_batch(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        let width = (preview.end.x - preview.start.x).abs();
        let length = (preview.end.y - preview.start.y).abs();
        self.sketch_start = None;
        self.sketch_cursor = None;
        self.value_input = format_height(preview.depth_mm);
        self.status_key = "status-pocket-created";
        self.digest = self.catalog.format(
            "digest-pocket-committed",
            &BTreeMap::from([
                ("width", format_height(width)),
                ("length", format_height(length)),
                ("depth", format_height(preview.depth_mm)),
                ("count", preview.shared_count.to_string()),
            ]),
        );
        true
    }

    fn selected_pocket(&self) -> Option<(FeatureId, Dimension)> {
        let definition_id = self.selection.primary.as_ref()?.definition_id;
        let snapshot = self.document.current();
        snapshot
            .definition(definition_id)?
            .feature_ids()
            .iter()
            .rev()
            .find_map(|feature_id| {
                let feature = snapshot.feature(*feature_id)?;
                let FeatureKind::Pocket { depth, .. } = feature.kind() else {
                    return None;
                };
                Some((*feature_id, depth.clone()))
            })
    }

    fn set_selected_pocket_depth(&mut self, depth_mm: f64) -> bool {
        let Some((feature_id, _)) = self.selected_pocket() else {
            return false;
        };
        let Ok(dimension) = Dimension::new(format_height(depth_mm), depth_mm) else {
            return false;
        };
        if self
            .document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: feature_id,
                    dimension,
                },
            ]))
            .is_err()
        {
            self.digest = self.catalog.text("digest-pocket-invalid-depth");
            return false;
        }
        self.value_input = format_height(depth_mm);
        self.status_key = "status-pocket-created";
        self.digest = self.catalog.format(
            "digest-pocket-depth-edited",
            &BTreeMap::from([("depth", format_height(depth_mm))]),
        );
        true
    }

    fn complete_circle_sketch(&mut self, center: Vec3, radial_point: Vec3) -> bool {
        let direction = Vec3::new(radial_point.x - center.x, radial_point.y - center.y, 0.0);
        self.complete_circle(center, vector_length(direction), direction)
    }

    fn complete_circle(&mut self, center: Vec3, radius_mm: f64, direction: Vec3) -> bool {
        if !radius_mm.is_finite() || radius_mm <= 0.01 {
            return false;
        }
        let direction_length = vector_length(direction);
        let unit = if direction_length > 0.01 {
            Vec3::new(
                direction.x / direction_length,
                direction.y / direction_length,
                0.0,
            )
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        let radial = [unit.x * radius_mm, unit.y * radius_mm];
        let opposite = [-radial[0], -radial[1]];
        let created = self.create_segment_profile_at(
            center,
            vec![
                ProfileSegment::CircularArc {
                    start_mm: radial,
                    end_mm: opposite,
                    center_mm: [0.0, 0.0],
                    clockwise: false,
                },
                ProfileSegment::CircularArc {
                    start_mm: opposite,
                    end_mm: radial,
                    center_mm: [0.0, 0.0],
                    clockwise: false,
                },
            ],
            "model-default-circle",
            "model-circle-profile",
        );
        if created {
            self.sketch_mode = false;
            self.sketch_start = None;
            self.sketch_cursor = None;
            self.value_input = format_height(radius_mm);
            self.status_key = "status-circle-created";
            self.digest = self.catalog.format(
                "digest-exact-circle",
                &BTreeMap::from([("radius", format_height(radius_mm))]),
            );
        }
        created
    }

    fn complete_exact_circle(&mut self) -> bool {
        let Some(center) = self.sketch_start else {
            return false;
        };
        let Some(radius_mm) = parse_distance_mm(&self.value_input).filter(|radius| *radius > 0.01)
        else {
            return false;
        };
        let direction = self
            .sketch_cursor
            .map(|cursor| cursor - center)
            .unwrap_or(Vec3::new(1.0, 0.0, 0.0));
        self.complete_circle(center, radius_mm, direction)
    }

    fn complete_arc_sketch(&mut self, start: Vec3, end: Vec3, bulge_point: Vec3) -> bool {
        let Some(arc) = arc_geometry(start, end, bulge_point) else {
            return false;
        };
        let local_end = [end.x - start.x, end.y - start.y];
        let local_center = [arc.center.x - start.x, arc.center.y - start.y];
        let created = self.create_segment_profile_at(
            start,
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [0.0, 0.0],
                    end_mm: local_end,
                    center_mm: local_center,
                    clockwise: arc.clockwise,
                },
                ProfileSegment::Line {
                    start_mm: local_end,
                    end_mm: [0.0, 0.0],
                },
            ],
            "model-default-arc",
            "model-arc-profile",
        );
        if created {
            let bulge_mm = point_line_signed_distance(bulge_point, start, end).abs();
            self.sketch_mode = false;
            self.sketch_start = None;
            self.sketch_end = None;
            self.sketch_cursor = None;
            self.value_input = format_height(bulge_mm);
            self.status_key = "status-arc-created";
            self.digest = self.catalog.format(
                "digest-exact-arc",
                &BTreeMap::from([("bulge", format_height(bulge_mm))]),
            );
        }
        created
    }

    fn complete_exact_arc(&mut self) -> bool {
        let (Some(start), Some(end)) = (self.sketch_start, self.sketch_end) else {
            return false;
        };
        let Some(bulge_mm) =
            parse_distance_mm(&self.value_input).filter(|value| value.abs() > 0.01)
        else {
            return false;
        };
        let chord = end - start;
        let chord_length = vector_length(Vec3::new(chord.x, chord.y, 0.0));
        if chord_length <= 0.01 {
            return false;
        }
        let midpoint = (start + end) * 0.5;
        let normal = Vec3::new(-chord.y / chord_length, chord.x / chord_length, 0.0);
        let cursor_side = self.sketch_cursor.map_or(1.0, |cursor| {
            if point_line_signed_distance(cursor, start, end) < 0.0 {
                -1.0
            } else {
                1.0
            }
        });
        self.complete_arc_sketch(
            midpoint - chord * 0.5,
            end,
            midpoint + normal * bulge_mm * cursor_side,
        )
    }

    fn complete_rectangle_sketch(&mut self, start: Vec3, end: Vec3) -> bool {
        if self.active_tool == ActiveTool::CutThrough {
            return self.complete_through_cut_sketch(start, end);
        }
        if self.active_tool == ActiveTool::Pocket {
            let Some((_, item, _)) = self.through_cut_target() else {
                return false;
            };
            return self.prepare_pocket_preview(start, end, (item.size_mm.z * 0.5).min(10.0));
        }
        let origin = Vec3::new(start.x.min(end.x), start.y.min(end.y), start.z);
        let size = Vec3::new((end.x - start.x).abs(), (end.y - start.y).abs(), 0.0);
        let created = self.create_profile_at(
            origin,
            vec![[0.0, 0.0], [size.x, 0.0], [size.x, size.y], [0.0, size.y]],
        );
        if created {
            self.sketch_mode = false;
            self.sketch_start = None;
            self.sketch_cursor = None;
            self.value_input.clear();
            self.status_key = "status-sketch-created";
            self.digest = self.catalog.format(
                "digest-exact-rectangle",
                &BTreeMap::from([
                    ("width", format_height(size.x)),
                    ("depth", format_height(size.y)),
                ]),
            );
        }
        created
    }

    fn complete_exact_rectangle(&mut self) -> bool {
        let Some(start) = self.sketch_start else {
            return false;
        };
        let Some([width, depth]) = parse_rectangle_dimensions(&self.value_input) else {
            return false;
        };
        let cursor = self
            .sketch_cursor
            .unwrap_or(start + Vec3::new(1.0, 1.0, 0.0));
        let x_direction = if cursor.x < start.x { -1.0 } else { 1.0 };
        let y_direction = if cursor.y < start.y { -1.0 } else { 1.0 };
        self.complete_rectangle_sketch(
            start,
            Vec3::new(
                start.x + width * x_direction,
                start.y + depth * y_direction,
                start.z,
            ),
        )
    }

    fn apply_value_input(&mut self) -> bool {
        if self.sketch_mode && self.sketch_start.is_some() {
            return match self.active_tool {
                ActiveTool::Circle => self.complete_exact_circle(),
                ActiveTool::Arc => self.complete_exact_arc(),
                _ => self.complete_exact_rectangle(),
            };
        }
        if self.active_tool == ActiveTool::Pocket {
            let Some(depth_mm) = parse_distance_mm(&self.value_input).filter(|depth| *depth > 0.01)
            else {
                self.digest = self.catalog.text("digest-pocket-invalid-depth");
                return false;
            };
            if let Some(preview) = self.pocket_preview.clone() {
                if !self.prepare_pocket_preview(preview.start, preview.end, depth_mm) {
                    return false;
                }
                return self.confirm_pocket_preview();
            }
            if self.set_selected_pocket_depth(depth_mm) {
                return true;
            }
        }
        if self.active_tool == ActiveTool::PlanarOffset {
            return self.refresh_planar_offset_preview() && self.confirm_planar_offset_preview();
        }
        if self.active_tool == ActiveTool::Revolve {
            return self.refresh_revolve_preview() && self.confirm_revolve_preview();
        }
        if matches!(
            self.active_tool,
            ActiveTool::Shell | ActiveTool::Fillet | ActiveTool::Chamfer
        ) {
            return self.refresh_general_finish_preview() && self.confirm_general_finish_preview();
        }
        if self.active_tool == ActiveTool::PushPull {
            let selection = self.selection.primary.clone();
            let Some(selection) = selection else {
                self.status_key = "error-push-pull-selection-required";
                self.digest = self.catalog.text("error-push-pull-selection-required");
                return false;
            };
            if parse_distance_mm(&self.value_input).is_none() {
                self.digest = self.catalog.text("digest-nothing-to-apply");
                return false;
            }
            // A value typed straight after a Push/Pull is planned as an absolute
            // replacement against its guarded parent. The committed tip remains
            // untouched throughout chooser and preview interaction.
            let current = self.document.current();
            let correction = self.last_push_pull.as_ref().filter(|operation| {
                operation.selection == selection
                    && operation.revision == current.revision_id()
                    && operation.canonical_digest == current.canonical_digest()
            });
            let planning = correction
                .and_then(|_| self.document.tip_replacement_parent().ok())
                .map_or(SmartPushPullPlanning::Append, |parent| {
                    SmartPushPullPlanning::TipReplacement(parent)
                });
            self.selection.select_exact(selection.clone(), false);
            self.push_pull_distance_input = self.value_input.clone();
            if self.start_preview_for(planning) {
                if self.has_smart_push_pull_chooser() {
                    return true;
                }
                if self.confirm_push_pull_preview() {
                    self.digest = self.catalog.format(
                        "digest-exact-value-applied",
                        &BTreeMap::from([(
                            "value",
                            parse_distance_mm(&self.value_input)
                                .map_or_else(String::new, format_signed_mm),
                        )]),
                    );
                    return true;
                }
            }
        }
        if self.active_tool == ActiveTool::Move {
            if let Some(delta_mm) = parse_move_vector(&self.value_input) {
                if self.move_selected(delta_mm) {
                    self.digest = self.catalog.format(
                        "digest-exact-move-applied",
                        &BTreeMap::from([("value", format_vector_mm(delta_mm))]),
                    );
                    return true;
                }
            } else if let Some(distance_mm) = parse_distance_mm(&self.value_input) {
                if let Some(previous) = self.last_move {
                    let correction_mm = distance_mm - previous.applied_distance_mm;
                    if correction_mm.abs() < 0.01 {
                        self.digest = self.catalog.format(
                            "digest-exact-move-applied",
                            &BTreeMap::from([("value", format_signed_mm(distance_mm))]),
                        );
                        return true;
                    }
                    let snapshot = self.document.current();
                    let Some(occurrence) = snapshot.occurrence(previous.occurrence_id) else {
                        self.last_move = None;
                        return false;
                    };
                    let selection = SelectionId {
                        definition_id: occurrence.definition_id(),
                        instance_path: InstancePath::root(previous.occurrence_id),
                        element: ElementId::Face {
                            axis: Axis::Z,
                            side: Side::Maximum,
                        },
                    };
                    if self.translate_occurrence(
                        &selection,
                        previous.direction * correction_mm,
                        false,
                    ) {
                        self.last_move = Some(LastMove {
                            applied_distance_mm: distance_mm,
                            ..previous
                        });
                        self.digest = self.catalog.format(
                            "digest-exact-move-applied",
                            &BTreeMap::from([("value", format_signed_mm(distance_mm))]),
                        );
                        return true;
                    }
                } else if self.move_selected(Vec3::new(distance_mm, 0.0, 0.0)) {
                    self.digest = self.catalog.format(
                        "digest-exact-move-applied",
                        &BTreeMap::from([("value", format_signed_mm(distance_mm))]),
                    );
                    return true;
                }
            }
        }
        self.digest = self.catalog.text("digest-nothing-to-apply");
        false
    }

    fn value_label_key(&self) -> &'static str {
        match self.active_tool {
            ActiveTool::Rectangle | ActiveTool::CutThrough => "value-label-width-depth",
            ActiveTool::Circle => "value-label-radius",
            ActiveTool::Arc => "value-label-bulge",
            ActiveTool::Revolve => "value-label-angle",
            ActiveTool::Shell => "value-label-thickness",
            ActiveTool::Fillet | ActiveTool::Chamfer => "value-label-radius-distance",
            ActiveTool::Pocket
                if self.pocket_preview.is_some() || self.selected_pocket().is_some() =>
            {
                "value-label-pocket-depth"
            }
            ActiveTool::Pocket => "value-label-width-depth",
            ActiveTool::PushPull | ActiveTool::Move | ActiveTool::Measure => "value-label-distance",
            _ => "value-label-dimensions",
        }
    }

    /// Camera axes in world space: screen right, screen up, and view direction.
    fn camera_basis(&self) -> (Vec3, Vec3, Vec3) {
        let yaw_sin = f64::from(self.yaw.sin());
        let yaw_cos = f64::from(self.yaw.cos());
        let pitch_sin = f64::from(self.pitch.sin());
        let pitch_cos = f64::from(self.pitch.cos());
        (
            Vec3::new(yaw_cos, -yaw_sin, 0.0),
            Vec3::new(yaw_sin * pitch_cos, yaw_cos * pitch_cos, -pitch_sin),
            Vec3::new(-yaw_sin * pitch_sin, -yaw_cos * pitch_sin, -pitch_cos),
        )
    }

    fn camera_target(&self) -> Vec3 {
        Vec3::new(BOX_WIDTH_MM * 0.5, BOX_DEPTH_MM * 0.5, self.camera_target_z)
    }

    /// Millimetres between the eye and the orbit target — the readout's `dist`.
    ///
    /// The nominal distance follows the zoom, but a converging projection is
    /// only meaningful while the eye is outside the model, so the cached value
    /// is pushed back to clear the scene. See [`Self::refresh_camera_distance`].
    fn camera_distance(&self) -> f64 {
        self.camera_distance_mm
    }

    /// Recompute the eye distance for the current scene. Once per frame.
    fn refresh_camera_distance(&mut self) {
        let nominal = 420.0 / f64::from(self.zoom);
        let target = self.camera_target();
        let radius = self
            .active_boxes()
            .into_iter()
            .flat_map(|item| {
                let far = item.origin_mm + item.size_mm;
                [
                    vector_length(item.origin_mm - target),
                    vector_length(far - target),
                ]
            })
            .fold(0.0_f64, f64::max);
        self.camera_distance_mm = nominal
            .max(radius * CAMERA_CLEARANCE)
            .max(PERSPECTIVE_NEAR_MM);
    }

    /// Pixels per millimetre at the orbit target.
    ///
    /// Both projections agree here by construction, so switching between them
    /// keeps the model the same size and only changes how depth is treated.
    fn view_scale(&self, rect: Rect) -> f64 {
        f64::from(self.zoom) * f64::from(rect.width().min(rect.height())) / 420.0
    }

    /// Focal length in pixels, chosen so the target plane matches `view_scale`.
    fn camera_focal(&self, rect: Rect) -> f64 {
        self.view_scale(rect) * self.camera_distance()
    }

    pub fn toggle_projection_mode(&mut self) {
        self.projection_mode = self.projection_mode.toggled();
        self.digest = self.catalog.text(self.projection_mode.label_key());
    }

    /// Current viewport projection.
    #[must_use]
    pub const fn projection_mode(&self) -> ProjectionMode {
        self.projection_mode
    }

    /// Colours every surface of the shell paints with.
    #[must_use]
    pub const fn palette(&self) -> Palette {
        Palette::of(self.theme)
    }

    /// Which of the four appearances is showing.
    #[must_use]
    pub const fn theme(&self) -> ThemeKind {
        self.theme
    }

    /// Switch appearance. Purely presentational — the document never changes.
    pub fn set_theme(&mut self, theme: ThemeKind) {
        self.theme = theme;
        self.digest = self.catalog.text(theme.label_key());
    }

    fn view_ray(&self, pointer: Pos2, rect: Rect) -> Option<Ray> {
        let (right, up, forward) = self.camera_basis();
        let target = self.camera_target();
        let horizontal = f64::from(pointer.x - rect.center().x - self.pan.x);
        let vertical = f64::from(rect.center().y + self.pan.y - pointer.y);
        match self.projection_mode {
            ProjectionMode::Parallel => {
                let scale = self.view_scale(rect);
                let view_plane_point =
                    target + right * (horizontal / scale) + up * (vertical / scale);
                Ray::new(view_plane_point - forward * self.camera_distance(), forward).ok()
            }
            ProjectionMode::Perspective => {
                let focal = self.camera_focal(rect);
                let eye = target - forward * self.camera_distance();
                let direction = right * (horizontal / focal) + up * (vertical / focal) + forward;
                Ray::new(eye, direction).ok()
            }
        }
    }

    fn screen_to_plane(&self, pointer: Pos2, rect: Rect, plane_z: f64) -> Option<Vec3> {
        let ray = self.view_ray(pointer, rect)?;
        if ray.direction.z.abs() <= 1.0e-9 {
            return None;
        }
        let distance = (plane_z - ray.origin.z) / ray.direction.z;
        (distance >= 0.0).then(|| ray.origin + ray.direction * distance)
    }

    pub fn zoom_at_screen(&mut self, pointer: Pos2, rect: Rect, scroll: f32) {
        let anchor = self
            .surface_point_at_screen(pointer, rect)
            .or_else(|| self.screen_to_plane(pointer, rect, self.camera_target_z));
        let Some(anchor) = anchor else {
            return;
        };
        let old_position = self.project(anchor, rect);
        let new_zoom = (self.zoom * (scroll * 0.001).exp()).clamp(MIN_CAMERA_ZOOM, MAX_CAMERA_ZOOM);
        if new_zoom == self.zoom {
            return;
        }
        self.zoom = new_zoom;
        self.refresh_camera_distance();
        let new_position = self.project(anchor, rect);
        self.pan += old_position - new_position;
    }

    fn surface_point_at_screen(&self, pointer: Pos2, rect: Rect) -> Option<Vec3> {
        let ray = self.view_ray(pointer, rect)?;
        let snapshot = self.document.current();
        let exact = self
            .exact_projection(&snapshot)
            .exact_surface_pick(ray)
            .map(|hit| (hit.ray_distance_mm, hit.position_mm));
        let mesh = MeshInteractionProjection::from_snapshot(&snapshot)
            .exact_surface_pick(ray)
            .map(|hit| (hit.ray_distance_mm, hit.position_mm));
        [exact, mesh]
            .into_iter()
            .flatten()
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, point)| point)
            .or_else(|| {
                self.pick_result_at_screen(pointer, rect, 8.0)
                    .map(|pick| pick.primary.position_mm)
            })
    }

    fn rectangle_plane_z(&self, pointer: Pos2, rect: Rect) -> f64 {
        let Some(selection) = self.exact_pick_at_screen(pointer, rect) else {
            return 0.0;
        };
        if selection.element
            != (ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            })
        {
            return 0.0;
        }
        self.active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)
            .map_or(0.0, |item| item.origin_mm.z + item.size_mm.z)
    }

    fn hover_readout(&self) -> String {
        let Some(hovered) = &self.hovered else {
            return self.catalog.text("hover-none");
        };
        let snapshot = self.document.current();
        let Some(occurrence) = snapshot.occurrence(hovered.instance_path.root_occurrence()) else {
            return self.catalog.text("hover-none");
        };
        let Some(definition) = snapshot.definition(occurrence.definition_id()) else {
            return self.catalog.text("hover-none");
        };
        let face = self.catalog.text(match hovered.element {
            ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            } => "face-top",
            ElementId::Face {
                axis: Axis::Z,
                side: Side::Minimum,
            } => "face-bottom",
            _ => "face-side",
        });
        let overlap_count = self
            .hover_pick
            .as_ref()
            .map_or(0, |pick| pick.overlapping.len());
        if let Some(snap) = self.hover_snap.as_ref()
            && (snap.kind != SnapKind::Face || overlap_count > 1)
        {
            return self.catalog.format(
                "hover-inference",
                &BTreeMap::from([
                    ("name", definition.name().to_owned()),
                    ("face", face),
                    (
                        "snap",
                        self.catalog.text(match snap.kind {
                            SnapKind::Endpoint => "snap-endpoint",
                            SnapKind::Intersection => "snap-intersection",
                            SnapKind::Midpoint => "snap-midpoint",
                            SnapKind::Center => "snap-center",
                            SnapKind::Tangent => "snap-tangent",
                            SnapKind::Face => "snap-face",
                        }),
                    ),
                    ("index", (self.hover_overlap_index + 1).to_string()),
                    ("count", overlap_count.to_string()),
                ]),
            );
        }
        self.catalog.format(
            "hover-face",
            &BTreeMap::from([("name", definition.name().to_owned()), ("face", face)]),
        )
    }

    fn viewport_overlays(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let palette = self.palette();
        let painter = ui.painter();
        let glass = palette.glass();
        let line = palette.line;
        let text = palette.text;
        let dim = palette.dim;
        let mono = egui::FontId::monospace(11.0);

        // What is under the pointer, or selected, is the one thing the user
        // looks up most often, so it sits top-left where the eye starts.
        let hover_galley =
            painter.layout_no_wrap(self.hover_readout(), egui::FontId::proportional(12.0), text);
        let hover_rect = Rect::from_min_size(
            rect.left_top() + Vec2::new(14.0, 12.0),
            Vec2::new(hover_galley.size().x + 34.0, 28.0),
        );
        painter.rect_filled(hover_rect, 8.0, glass);
        painter.rect_stroke(
            hover_rect,
            8.0,
            Stroke::new(1.0_f32, line),
            egui::StrokeKind::Inside,
        );
        painter.circle_filled(
            Pos2::new(hover_rect.left() + 13.0, hover_rect.center().y),
            3.0,
            palette.accent,
        );
        painter.galley(
            Pos2::new(
                hover_rect.left() + 24.0,
                hover_rect.center().y - hover_galley.size().y * 0.5,
            ),
            hover_galley,
            text,
        );

        let camera = self.catalog.format(
            "camera-readout",
            &BTreeMap::from([
                ("distance", format_height(self.camera_distance())),
                ("azimuth", format_height(f64::from(self.yaw.to_degrees()))),
                (
                    "elevation",
                    format_height(f64::from(self.pitch.to_degrees())),
                ),
            ]),
        );
        // The camera readout is laid out first so the plate is sized to the text
        // instead of to a guessed constant that the text then overflows.
        let camera_galley = painter.layout_no_wrap(camera, mono.clone(), dim);
        let readout_rect = Rect::from_min_size(
            Pos2::new(
                rect.right() - 14.0 - (camera_galley.size().x + 24.0),
                rect.top() + 12.0,
            ),
            Vec2::new(camera_galley.size().x + 24.0, 28.0),
        );
        painter.rect_filled(readout_rect, 7.0, glass);
        painter.galley(
            Pos2::new(
                readout_rect.center().x - camera_galley.size().x * 0.5,
                readout_rect.center().y - camera_galley.size().y * 0.5,
            ),
            camera_galley,
            dim,
        );

        // The value box is placed first; the hint then takes whatever width is
        // left, so the two can never paint over one another.
        let value_size = Vec2::new(248.0_f32.min(rect.width() - 28.0), 46.0);
        let value_rect = Rect::from_min_size(
            Pos2::new(
                rect.right() - 14.0 - value_size.x,
                rect.bottom() - 14.0 - value_size.y,
            ),
            value_size,
        );
        painter.rect_filled(value_rect, 8.0, glass);
        painter.rect_stroke(
            value_rect,
            8.0,
            Stroke::new(1.0_f32, line),
            egui::StrokeKind::Inside,
        );
        // Label and field share one row: the label names the quantity on the
        // left, the accent-coloured number and its unit read off the right.
        painter.text(
            Pos2::new(value_rect.left() + 14.0, value_rect.center().y),
            egui::Align2::LEFT_CENTER,
            self.catalog.text(self.value_label_key()),
            egui::FontId::proportional(SHELL_SMALL_SIZE),
            dim,
        );
        painter.text(
            Pos2::new(value_rect.right() - 12.0, value_rect.center().y),
            egui::Align2::RIGHT_CENTER,
            self.catalog.text("unit-mm"),
            egui::FontId::monospace(10.5),
            palette.faint,
        );
        let input_rect = Rect::from_min_max(
            Pos2::new(value_rect.center().x - 6.0, value_rect.top() + 8.0),
            Pos2::new(value_rect.right() - 34.0, value_rect.bottom() - 8.0),
        );

        // The hint card names the armed tool and then explains it, so the two
        // read as a title and a body rather than as one wall of grey text.
        let hint_width = (value_rect.left() - 12.0) - (rect.left() + 14.0);
        if hint_width >= 200.0 {
            let body_width = hint_width - 46.0;
            let title_galley = painter.layout_no_wrap(
                self.catalog.text(self.active_tool.label_key()),
                egui::FontId::proportional(12.5),
                text,
            );
            let hint_galley = painter.layout(
                self.catalog.text(self.active_tool.hint_key()),
                egui::FontId::proportional(SHELL_SMALL_SIZE),
                dim,
                body_width,
            );
            let height = title_galley.size().y + hint_galley.size().y + 24.0;
            let hint_rect = Rect::from_min_size(
                Pos2::new(rect.left() + 14.0, value_rect.bottom() - height),
                Vec2::new(
                    title_galley.size().x.max(hint_galley.size().x) + 46.0,
                    height,
                ),
            );
            painter.rect_filled(hint_rect, 8.0, glass);
            painter.rect_stroke(
                hint_rect,
                8.0,
                Stroke::new(1.0_f32, line),
                egui::StrokeKind::Inside,
            );
            let badge = Pos2::new(hint_rect.left() + 20.0, hint_rect.top() + 19.0);
            painter.circle_stroke(badge, 7.0, Stroke::new(1.4_f32, palette.accent));
            painter.text(
                badge,
                egui::Align2::CENTER_CENTER,
                "i",
                egui::FontId::proportional(10.0),
                palette.accent,
            );
            let title_top = hint_rect.top() + 12.0;
            painter.galley(
                Pos2::new(hint_rect.left() + 36.0, title_top),
                title_galley.clone(),
                text,
            );
            painter.galley(
                Pos2::new(
                    hint_rect.left() + 36.0,
                    title_top + title_galley.size().y + 3.0,
                ),
                hint_galley,
                dim,
            );
        }
        let response = ui.put(
            input_rect,
            egui::TextEdit::singleline(&mut self.value_input)
                .id_salt("value-box-input")
                .hint_text(self.catalog.text("value-placeholder"))
                .font(egui::FontId::monospace(15.0))
                .text_color(palette.accent)
                .horizontal_align(egui::Align::Max)
                .frame(false),
        );
        if self.focus_value_box {
            response.request_focus();
            self.focus_value_box = false;
        }
        if response.changed() && self.active_tool == ActiveTool::PlanarOffset {
            self.refresh_planar_offset_preview();
        }
        // A single-line `TextEdit` surrenders focus on Enter, so the commit has
        // to be accepted on the frame the focus is lost as well.
        if (response.has_focus() || response.lost_focus())
            && ui.input(|input| input.key_pressed(egui::Key::Enter))
        {
            self.apply_value_input();
            response.surrender_focus();
        }
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        self.refresh_camera_distance();
        let desired = ui.available_size().max(Vec2::new(320.0, 280.0));
        let (response, painter) = ui.allocate_painter(desired, Sense::click_and_drag());
        self.viewport_rect = Some(response.rect);
        let palette = self.palette();
        theme::paint_vignette(
            &painter,
            response.rect,
            palette.viewport_inner,
            palette.viewport_outer,
        );
        self.update_viewport_inference(response.hover_pos(), response.rect);

        let primary_press = ui.input(|input| {
            input
                .pointer
                .button_pressed(egui::PointerButton::Primary)
                .then(|| input.pointer.press_origin())
                .flatten()
        });
        let primary_release =
            ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary));
        if response.hovered()
            && let Some(pointer) = primary_press
        {
            self.push_pull_drag = None;
            self.move_drag = None;
            if self.sketch_mode {
                let plane_z = self.sketch_start.map_or_else(
                    || {
                        if matches!(
                            self.active_tool,
                            ActiveTool::CutThrough | ActiveTool::Pocket
                        ) {
                            self.through_cut_target()
                                .map_or(0.0, |(_, item, _)| item.origin_mm.z + item.size_mm.z)
                        } else {
                            self.rectangle_plane_z(pointer, response.rect)
                        }
                    },
                    |start| start.z,
                );
                if let Some(point) = self.viewport_point_at_screen(pointer, response.rect, plane_z)
                {
                    if let Some(start) = self.sketch_start {
                        match self.active_tool {
                            ActiveTool::Circle => {
                                self.complete_circle_sketch(start, point);
                            }
                            ActiveTool::Arc => {
                                if let Some(end) = self.sketch_end {
                                    self.complete_arc_sketch(start, end, point);
                                } else if vector_length(point - start) > 0.01 {
                                    self.sketch_end = Some(point);
                                    self.sketch_cursor = Some(point);
                                    self.value_input.clear();
                                    self.status_key = "status-arc-bulge";
                                }
                            }
                            _ => {
                                self.complete_rectangle_sketch(start, point);
                            }
                        }
                    } else {
                        self.sketch_start = Some(point);
                        self.sketch_cursor = Some(point);
                        self.value_input.clear();
                        self.status_key = match self.active_tool {
                            ActiveTool::Circle => "status-circle-radius",
                            ActiveTool::Arc => "status-arc-end",
                            ActiveTool::CutThrough => "status-cut-through-second-point",
                            ActiveTool::Pocket => "status-pocket-second-point",
                            _ => "status-sketch-second-point",
                        };
                    }
                }
            } else if self.active_tool == ActiveTool::Revolve {
                if let Some(plane_z) = self.revolve_tool.as_ref().map(|tool| tool.plane_z)
                    && let Some(point) =
                        self.viewport_point_at_screen(pointer, response.rect, plane_z)
                {
                    self.add_revolve_axis_point(point);
                }
            } else if self.active_tool == ActiveTool::Select {
                let additive = ui.input(|input| input.modifiers.shift);
                let target = self
                    .hover_snap
                    .as_ref()
                    .filter(|snap| matches!(snap.reference.element, ElementId::EdgeMidpoint(_)))
                    .map(|snap| snap.reference.clone())
                    .or_else(|| self.hovered.clone());
                self.select_from_viewport(target, additive);
            } else if matches!(
                self.active_tool,
                ActiveTool::SolidSubtract
                    | ActiveTool::SolidUnion
                    | ActiveTool::SolidIntersect
                    | ActiveTool::SolidSplit
            ) {
                let selection = self.hovered.clone();
                let keep_tool = ui.input(|input| input.modifiers.ctrl);
                self.select_solid_tool_occurrence(selection, keep_tool);
            } else if self.active_tool == ActiveTool::PushPull {
                if !self.begin_bottle_direct_drag(pointer, response.rect) {
                    self.select_from_viewport(self.hovered.clone(), false);
                    if self.push_pull_face_selected()
                        && let Some(selection) = &self.selection.primary
                        && let Some((screen_normal, pixels_per_mm)) =
                            self.push_pull_screen_projection(selection, response.rect)
                    {
                        self.push_pull_distance_input = "0".to_owned();
                        self.value_input = "0".to_owned();
                        self.push_pull_drag = Some(PushPullDrag {
                            pointer_start: pointer,
                            extent_start_mm: self.selected_face_extent_mm(),
                            screen_normal,
                            pixels_per_mm,
                        });
                    }
                }
            } else if self.active_tool == ActiveTool::Move {
                if let Some(mut anchor) = self.move_anchor.take() {
                    if !self.move_preview_is_current(&anchor) {
                        self.digest = self.catalog.text("error-preview-stale");
                    } else {
                        if let Some(pointer_world) =
                            self.screen_to_plane(pointer, response.rect, anchor.plane_z)
                        {
                            anchor.delta_mm = snapped_move_delta(
                                anchor.pointer_start_world,
                                pointer_world,
                                ui.input(|input| input.modifiers.shift),
                            );
                        }
                        if vector_length(anchor.delta_mm) >= 0.01 {
                            self.commit_move_drag(&anchor);
                        } else {
                            self.move_anchor = Some(anchor);
                        }
                    }
                } else {
                    self.begin_move_drag_at(
                        pointer,
                        response.rect,
                        ui.input(|input| input.modifiers.command),
                    );
                }
            } else if self.active_tool == ActiveTool::Measure {
                let plane_z = self.measure_anchor().map_or_else(
                    || self.rectangle_plane_z(pointer, response.rect),
                    |start| start.z,
                );
                if let Some(point) =
                    self.measurement_point_at_screen(pointer, response.rect, plane_z)
                {
                    self.add_measured_point(point);
                }
            }
        }

        if self.active_tool == ActiveTool::Select
            && response.double_clicked()
            && let Some(target) = self.hovered.clone()
        {
            self.enter_occurrence_context(target.instance_path);
        }

        if self.active_tool == ActiveTool::Move
            && self.move_drag.is_none()
            && let Some(mut anchor) = self.move_anchor.clone()
            && let Some(pointer) = response.hover_pos()
            && let Some(pointer_world) =
                self.screen_to_plane(pointer, response.rect, anchor.plane_z)
        {
            anchor.delta_mm = snapped_move_delta(
                anchor.pointer_start_world,
                pointer_world,
                ui.input(|input| input.modifiers.shift),
            );
            let distance = vector_length(anchor.delta_mm);
            let delta_mm = anchor.delta_mm;
            let copy = anchor.copy;
            self.move_anchor = Some(anchor);
            self.value_input = format_height(distance);
            self.digest = self.catalog.format(
                if copy {
                    "digest-copy-live"
                } else {
                    "digest-move-live"
                },
                &BTreeMap::from([
                    ("distance", format_height(distance)),
                    ("vector", format_vector_mm(delta_mm)),
                ]),
            );
        }

        let pointer_delta = ui.input(|input| input.pointer.delta());
        if response.dragged_by(egui::PointerButton::Secondary) {
            self.orbit(pointer_delta);
        } else if response.dragged_by(egui::PointerButton::Middle) {
            if ui.input(|input| input.modifiers.shift) {
                self.pan += pointer_delta;
            } else {
                self.orbit(pointer_delta);
            }
        } else if response.dragged_by(egui::PointerButton::Primary) {
            if self.active_tool == ActiveTool::Orbit {
                self.orbit(pointer_delta);
            } else if self.active_tool == ActiveTool::Pan {
                self.pan += pointer_delta;
            } else if self.sketch_mode {
                if let (Some(start), Some(pointer)) =
                    (self.sketch_start, response.interact_pointer_pos())
                {
                    self.sketch_cursor = self.screen_to_plane(pointer, response.rect, start.z);
                }
            } else if let (Some(mut drag), Some(pointer)) =
                (self.move_drag.clone(), response.interact_pointer_pos())
            {
                if let Some(pointer_world) =
                    self.screen_to_plane(pointer, response.rect, drag.plane_z)
                {
                    drag.delta_mm = snapped_move_delta(
                        drag.pointer_start_world,
                        pointer_world,
                        ui.input(|input| input.modifiers.shift),
                    );
                    let distance = vector_length(drag.delta_mm);
                    let delta_mm = drag.delta_mm;
                    let copy = drag.copy;
                    self.move_drag = Some(drag);
                    self.value_input = format_height(distance);
                    self.digest = self.catalog.format(
                        if copy {
                            "digest-copy-live"
                        } else {
                            "digest-move-live"
                        },
                        &BTreeMap::from([
                            ("distance", format_height(distance)),
                            ("vector", format_vector_mm(delta_mm)),
                        ]),
                    );
                }
            } else if let (Some(drag), Some(pointer)) =
                (self.bottle_direct_drag, response.interact_pointer_pos())
            {
                let value = Self::bottle_direct_value(drag, pointer);
                self.value_input = format_height(value);
                self.digest = format!(
                    "Bottle {:?} direct preview: {} mm (release to commit once)",
                    drag.control,
                    format_height(value)
                );
            } else if let (Some(drag), Some(pointer)) =
                (self.push_pull_drag, response.interact_pointer_pos())
            {
                let distance = push_pull_distance_from_pointer(drag, pointer);
                self.push_pull_distance_input = format_height(distance);
                self.value_input = self.push_pull_distance_input.clone();
                if distance.abs() >= 0.01 {
                    self.start_preview();
                } else {
                    self.preview = None;
                    self.preview_box = None;
                    self.preview_definition_id = None;
                    self.occurrence_operation_preview = None;
                }
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary)
            || (response.hovered() && primary_release)
        {
            if let Some(drag) = self.move_drag.take() {
                if !self.move_preview_is_current(&drag) {
                    self.digest = self.catalog.text("error-preview-stale");
                } else if vector_length(drag.delta_mm) >= 0.01 {
                    self.commit_move_drag(&drag);
                } else {
                    self.move_anchor = Some(drag);
                    self.digest = self.catalog.text("digest-move-anchor-set");
                }
            } else if let Some(drag) = self.bottle_direct_drag.take() {
                let value = parse_distance_mm(&self.value_input).unwrap_or(drag.value_start_mm);
                self.commit_bottle_direct_drag(drag, value);
            } else if self.push_pull_drag.take().is_some()
                && (self.has_preview() || self.has_occurrence_operation_preview())
            {
                self.confirm_push_pull_preview();
            }
        }
        if self.sketch_mode
            && self.sketch_start.is_some()
            && response.hovered()
            && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
        {
            let plane_z = self.sketch_start.map_or(0.0, |start| start.z);
            self.sketch_cursor = self.viewport_point_at_screen(pointer, response.rect, plane_z);
            // Once the user starts typing, the value box owns the value: the
            // focus request only takes effect next frame, so the freshly typed
            // text would otherwise be overwritten by the hovered dimensions.
            if !ui.ctx().wants_keyboard_input()
                && !self.focus_value_box
                && let (Some(start), Some(cursor)) = (self.sketch_start, self.sketch_cursor)
            {
                self.value_input = match self.active_tool {
                    ActiveTool::Circle => format_height(vector_length(Vec3::new(
                        cursor.x - start.x,
                        cursor.y - start.y,
                        0.0,
                    ))),
                    ActiveTool::Arc => self.sketch_end.map_or_else(
                        || format_height(vector_length(cursor - start)),
                        |end| format_height(point_line_signed_distance(cursor, start, end).abs()),
                    ),
                    _ => format!(
                        "{},{}",
                        format_height((cursor.x - start.x).abs()),
                        format_height((cursor.y - start.y).abs())
                    ),
                };
            }
        }
        if let Some(start) = self.measure_anchor()
            && response.hovered()
            && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
            && let Some(cursor) = self.measurement_point_at_screen(pointer, response.rect, start.z)
        {
            self.measure_cursor = Some(cursor);
            self.digest = self.measurement_text(start, cursor, "digest-measure-live");
        }
        if let Some((start, end)) = self.measure_span()
            && !ui.ctx().wants_keyboard_input()
            && !self.focus_value_box
        {
            self.value_input = format_height(vector_length(Vec3::new(
                end.x - start.x,
                end.y - start.y,
                end.z - start.z,
            )));
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.raw_scroll_delta.y);
            if scroll != 0.0
                && let Some(pointer) = response.hover_pos()
            {
                self.zoom_at_screen(pointer, response.rect, scroll);
            }
        }
        let forward = Vec3::new(
            -f64::from(self.yaw.sin() * self.pitch.sin()),
            -f64::from(self.yaw.cos() * self.pitch.sin()),
            -f64::from(self.pitch.cos()),
        );
        let snapshot = self.document.current();
        let move_transform_overrides = self.move_preview_transform_overrides();
        let use_wgpu_scene =
            self.wgpu_target_format.is_some() && !self.has_occurrence_operation_preview();
        let scene_plan = if use_wgpu_scene {
            let preview_active = !move_transform_overrides.is_empty();
            if preview_active
                || self
                    .render_plan
                    .as_ref()
                    .is_none_or(|plan| !plan.is_same_revision(&snapshot))
            {
                let plan = Arc::new(InstancedRenderPlan::from_snapshot_with_transform_overrides(
                    &snapshot,
                    &self.exact_results,
                    &mut self.render_cache,
                    &move_transform_overrides,
                ));
                if !preview_active {
                    self.render_plan = Some(Arc::clone(&plan));
                }
                Some(plan)
            } else {
                self.render_plan.clone()
            }
        } else {
            None
        };
        self.refresh_interaction_projection_cache(&snapshot);
        let interaction_projection_cache = self.interaction_projection_cache.borrow();
        let exact_projection = &interaction_projection_cache
            .as_ref()
            .expect("interaction cache was built")
            .exact;
        let active_context_paths = self
            .active_scene_query()
            .into_iter()
            .map(|occurrence| occurrence.instance_path)
            .collect::<BTreeSet<_>>();
        let mut faces = Vec::new();
        let mut edges = Vec::new();
        for item in self.viewport_boxes(exact_projection) {
            let item = self.render_box(item);
            let proxy_preview = self.proxy_preview_is_active(&item);
            let out_of_context = !active_context_paths.contains(&item.instance_path);
            let needs_cpu_overlay = self.selection.contains(&item.instance_path)
                || self
                    .hovered
                    .as_ref()
                    .is_some_and(|hovered| hovered.instance_path == item.instance_path)
                || proxy_preview
                || out_of_context;
            let needs_cpu_fill = !use_wgpu_scene || proxy_preview;
            if use_wgpu_scene && !needs_cpu_overlay {
                continue;
            }
            let corners = box_corners(item.size_mm.x, item.size_mm.y, item.size_mm.z)
                .map(|point| point + item.origin_mm);
            let projected = corners.map(|point| self.project(point, response.rect));
            for face in box_faces().into_iter().filter(|face| {
                face_is_visible(&face.element, forward)
                    && projected_face_has_area(face.corners, &projected)
            }) {
                let selection = SelectionId {
                    definition_id: item.definition_id,
                    instance_path: item.instance_path.clone(),
                    element: face.element.clone(),
                };
                let points = face.corners.map(|index| projected[index]);
                for edge in 0..points.len() {
                    edges.push(ProjectedEdge {
                        selection: selection.clone(),
                        points: [points[edge], points[(edge + 1) % points.len()]],
                    });
                }
                let depth = face
                    .corners
                    .iter()
                    .map(|index| point_depth(corners[*index], forward))
                    .sum::<f64>()
                    / 4.0;
                if needs_cpu_fill {
                    faces.push(ProjectedFace {
                        selection,
                        polygon: ProjectedPolygon::Quad(points),
                        color: face.color,
                        depth,
                        previewed: (self.has_preview()
                            && self.preview_definition_id == Some(item.definition_id)
                            && matches!(
                                face.element,
                                ElementId::Face {
                                    axis: Axis::Z,
                                    side: Side::Maximum,
                                }
                            ))
                            || (self.has_occurrence_operation_preview()
                                && item.instance_path.is_root()
                                && self.occurrence_operation_preview.as_ref().is_some_and(
                                    |preview| {
                                        preview
                                            .boxes
                                            .contains_key(&item.instance_path.root_occurrence())
                                    },
                                )),
                        out_of_context,
                    });
                }
            }
        }
        for occurrence in interaction_projection_cache
            .as_ref()
            .expect("interaction cache was built")
            .canonical
            .occurrences()
            .iter()
            .filter(|occurrence| {
                occurrence.visible
                    && exact_projection.contains_occurrence(&occurrence.instance_path)
                    && !(self.has_occurrence_operation_preview()
                        && occurrence.instance_path.is_root()
                        && self
                            .occurrence_operation_preview
                            .as_ref()
                            .is_some_and(|preview| {
                                let occurrence_id = occurrence.instance_path.root_occurrence();
                                preview.boxes.contains_key(&occurrence_id)
                                    || preview.hidden_occurrences.contains(&occurrence_id)
                            }))
            })
        {
            let out_of_context = !active_context_paths.contains(&occurrence.instance_path);
            let needs_cpu_overlay = self.selection.contains(&occurrence.instance_path)
                || self
                    .hovered
                    .as_ref()
                    .is_some_and(|hovered| hovered.instance_path == occurrence.instance_path)
                || out_of_context;
            let needs_cpu_fill = !use_wgpu_scene;
            if use_wgpu_scene && !needs_cpu_overlay {
                continue;
            }
            let Some(package) = self.exact_results.get(&occurrence.body.definition_id) else {
                continue;
            };
            let transform = move_transform_overrides
                .get(&occurrence.instance_path)
                .copied()
                .unwrap_or(occurrence.canonical_world_transform);
            let positions = package
                .vertices()
                .iter()
                .map(|vertex| vertex.position_mm.map(|value| value as f32))
                .collect::<Vec<_>>();
            let triangles = package
                .triangles()
                .iter()
                .map(|triangle| triangle.vertex_indices)
                .collect::<Vec<_>>();
            let face_groups = package
                .triangles()
                .iter()
                .map(|triangle| triangle.face_role)
                .collect::<Vec<_>>();
            for edge in feature_edges(&positions, &triangles, &face_groups) {
                let points = edge.map(|index| {
                    let position = package.vertices()[index as usize].position_mm;
                    self.project(
                        transform_model_point(
                            transform,
                            Vec3::new(position[0], position[1], position[2]),
                        ),
                        response.rect,
                    )
                });
                let mut elements = Vec::new();
                for triangle in package.triangles().iter().filter(|triangle| {
                    triangle.vertex_indices.contains(&edge[0])
                        && triangle.vertex_indices.contains(&edge[1])
                }) {
                    let element = triangle
                        .face_role
                        .and_then(exact_face_element)
                        .unwrap_or_else(|| {
                            let points_mm = triangle.vertex_indices.map(|index| {
                                let position = package.vertices()[index as usize].position_mm;
                                transform_model_point(
                                    transform,
                                    Vec3::new(position[0], position[1], position[2]),
                                )
                            });
                            face_element_from_normal(triangle_normal(points_mm))
                        });
                    if !elements.contains(&element) {
                        elements.push(element);
                    }
                }
                for element in elements {
                    edges.push(ProjectedEdge {
                        selection: SelectionId {
                            definition_id: occurrence.body.definition_id,
                            instance_path: occurrence.instance_path.clone(),
                            element,
                        },
                        points,
                    });
                }
            }
            for triangle in package.triangles() {
                let points_mm = triangle.vertex_indices.map(|index| {
                    let position = package.vertices()[index as usize].position_mm;
                    transform_model_point(
                        transform,
                        Vec3::new(position[0], position[1], position[2]),
                    )
                });
                let normal = triangle_normal(points_mm);
                if point_depth(normal, forward) >= -1.0e-9 {
                    continue;
                }
                let projected = points_mm.map(|point| self.project(point, response.rect));
                if !projected_polygon_has_area(&projected) {
                    continue;
                }
                let element = triangle
                    .face_role
                    .and_then(exact_face_element)
                    .unwrap_or_else(|| face_element_from_normal(normal));
                if needs_cpu_fill {
                    faces.push(ProjectedFace {
                        selection: SelectionId {
                            definition_id: occurrence.body.definition_id,
                            instance_path: occurrence.instance_path.clone(),
                            element,
                        },
                        polygon: ProjectedPolygon::Triangle(projected),
                        color: face_color_from_normal(normal),
                        depth: points_mm
                            .into_iter()
                            .map(|point| point_depth(point, forward))
                            .sum::<f64>()
                            / 3.0,
                        previewed: false,
                        out_of_context,
                    });
                }
            }
        }
        // Canonical mesh bodies are drawn by the instanced scene, but hover and
        // selection feedback is a CPU overlay: without this loop a grooved beam
        // is visible yet never highlights, so it reads as if it were not there.
        for occurrence in interaction_projection_cache
            .as_ref()
            .expect("interaction cache was built")
            .canonical
            .occurrences()
            .iter()
            .filter(|occurrence| {
                occurrence.visible
                    && interaction_projection_cache
                        .as_ref()
                        .expect("interaction cache was built")
                        .mesh
                        .contains_occurrence(&occurrence.instance_path)
            })
        {
            let out_of_context = !active_context_paths.contains(&occurrence.instance_path);
            let needs_cpu_overlay = self.selection.contains(&occurrence.instance_path)
                || self
                    .hovered
                    .as_ref()
                    .is_some_and(|hovered| hovered.instance_path == occurrence.instance_path)
                || out_of_context;
            let needs_cpu_fill = !use_wgpu_scene;
            if use_wgpu_scene && !needs_cpu_overlay {
                continue;
            }
            let Some(mesh) = definition_mesh_body(&snapshot, occurrence.body.definition_id) else {
                continue;
            };
            let transform = move_transform_overrides
                .get(&occurrence.instance_path)
                .copied()
                .unwrap_or(occurrence.canonical_world_transform);
            let points_mm = mesh
                .vertices_mm
                .iter()
                .map(|point| {
                    transform_model_point(transform, Vec3::new(point[0], point[1], point[2]))
                })
                .collect::<Vec<_>>();
            let positions = mesh
                .vertices_mm
                .iter()
                .map(|point| point.map(|value| value as f32))
                .collect::<Vec<_>>();
            let face_groups = vec![None::<u8>; mesh.triangles.len()];
            for edge in feature_edges(&positions, &mesh.triangles, &face_groups) {
                let projected =
                    edge.map(|index| self.project(points_mm[index as usize], response.rect));
                let element = mesh
                    .triangles
                    .iter()
                    .find(|triangle| triangle.contains(&edge[0]) && triangle.contains(&edge[1]))
                    .map(|triangle| {
                        face_element_from_normal(triangle_normal(
                            triangle.map(|index| points_mm[index as usize]),
                        ))
                    });
                if let Some(element) = element {
                    edges.push(ProjectedEdge {
                        selection: SelectionId {
                            definition_id: occurrence.body.definition_id,
                            instance_path: occurrence.instance_path.clone(),
                            element,
                        },
                        points: projected,
                    });
                }
            }
            if !needs_cpu_fill {
                continue;
            }
            for triangle in &mesh.triangles {
                let corners = triangle.map(|index| points_mm[index as usize]);
                let normal = triangle_normal(corners);
                if point_depth(normal, forward) >= -1.0e-9 {
                    continue;
                }
                let projected = corners.map(|point| self.project(point, response.rect));
                if !projected_polygon_has_area(&projected) {
                    continue;
                }
                faces.push(ProjectedFace {
                    selection: SelectionId {
                        definition_id: occurrence.body.definition_id,
                        instance_path: occurrence.instance_path.clone(),
                        element: face_element_from_normal(normal),
                    },
                    polygon: ProjectedPolygon::Triangle(projected),
                    color: face_color_from_normal(normal),
                    depth: corners
                        .into_iter()
                        .map(|point| point_depth(point, forward))
                        .sum::<f64>()
                        / 3.0,
                    previewed: false,
                    out_of_context,
                });
            }
        }
        drop(interaction_projection_cache);
        faces.sort_by(|left, right| right.depth.total_cmp(&left.depth));

        self.paint_scene_base_layers(&painter, response.rect, scene_plan);

        self.paint_projected_faces(&painter, &faces);

        let edge_stroke = Stroke::new(1.25_f32, Color32::from_rgb(182, 192, 207));
        for edge in &edges {
            painter.line_segment(edge.points, edge_stroke);
        }

        let selection_stroke = Stroke::new(1.8_f32, Color32::from_rgb(240, 78, 35));
        for edge in edges.iter().filter(|edge| {
            if matches!(
                self.active_tool,
                ActiveTool::PushPull
                    | ActiveTool::CutThrough
                    | ActiveTool::Pocket
                    | ActiveTool::SolidSubtract
                    | ActiveTool::SolidUnion
                    | ActiveTool::SolidIntersect
                    | ActiveTool::SolidSplit
            ) {
                self.selection.primary.as_ref() == Some(&edge.selection)
            } else {
                self.selection.contains(&edge.selection.instance_path)
            }
        }) {
            painter.line_segment(edge.points, selection_stroke);
        }

        if let (Some(start), Some(cursor)) = (self.sketch_start, self.sketch_cursor) {
            if self.active_tool == ActiveTool::Arc {
                if let Some(end) = self.sketch_end
                    && let Some(arc) = arc_geometry(start, end, cursor)
                {
                    let stroke = Stroke::new(2.0_f32, Color32::from_rgb(255, 199, 68));
                    let points = arc_polyline(arc, 64)
                        .into_iter()
                        .map(|point| self.project(point, response.rect))
                        .collect();
                    painter.add(egui::Shape::line(points, stroke));
                    painter.line_segment(
                        [
                            self.project(start, response.rect),
                            self.project(end, response.rect),
                        ],
                        Stroke::new(1.0_f32, Color32::from_rgb(160, 160, 170)),
                    );
                    painter.text(
                        self.project(cursor, response.rect),
                        egui::Align2::CENTER_CENTER,
                        format!(
                            "B {} mm",
                            format_height(point_line_signed_distance(cursor, start, end).abs())
                        ),
                        egui::FontId::proportional(14.0),
                        Color32::WHITE,
                    );
                }
            } else if self.active_tool == ActiveTool::Circle {
                let radius = vector_length(Vec3::new(cursor.x - start.x, cursor.y - start.y, 0.0));
                let stroke = Stroke::new(2.0_f32, Color32::from_rgb(255, 199, 68));
                let mut points = Vec::with_capacity(65);
                for segment in 0..=64 {
                    let angle = std::f64::consts::TAU * segment as f64 / 64.0;
                    points.push(self.project(
                        Vec3::new(
                            start.x + radius * angle.cos(),
                            start.y + radius * angle.sin(),
                            start.z,
                        ),
                        response.rect,
                    ));
                }
                painter.add(egui::Shape::line(points, stroke));
                painter.text(
                    self.project(start, response.rect),
                    egui::Align2::CENTER_CENTER,
                    format!("R {} mm", format_height(radius)),
                    egui::FontId::proportional(14.0),
                    Color32::WHITE,
                );
            } else {
                let ground = [
                    start,
                    Vec3::new(cursor.x, start.y, start.z),
                    cursor,
                    Vec3::new(start.x, cursor.y, start.z),
                ];
                let points = ground.map(|point| self.project(point, response.rect));
                let stroke = Stroke::new(2.0_f32, Color32::from_rgb(255, 199, 68));
                for edge in 0..points.len() {
                    painter.line_segment([points[edge], points[(edge + 1) % points.len()]], stroke);
                }
                painter.text(
                    Pos2::new(
                        points.iter().map(|point| point.x).sum::<f32>() / 4.0,
                        points.iter().map(|point| point.y).sum::<f32>() / 4.0,
                    ),
                    egui::Align2::CENTER_CENTER,
                    format!(
                        "{} × {} mm",
                        format_height((cursor.x - start.x).abs()),
                        format_height((cursor.y - start.y).abs())
                    ),
                    egui::FontId::proportional(14.0),
                    Color32::WHITE,
                );
            }
        }

        if let Some(preview) = self
            .pocket_preview
            .as_ref()
            .filter(|_| self.has_pocket_preview())
        {
            let top = [
                preview.start,
                Vec3::new(preview.end.x, preview.start.y, preview.start.z),
                preview.end,
                Vec3::new(preview.start.x, preview.end.y, preview.start.z),
            ];
            let floor = top.map(|point| point - Vec3::new(0.0, 0.0, preview.depth_mm));
            let top_screen = top.map(|point| self.project(point, response.rect));
            let floor_screen = floor.map(|point| self.project(point, response.rect));
            let floor_fill = Color32::from_rgba_unmultiplied(58, 126, 174, 90);
            painter.add(egui::Shape::convex_polygon(
                floor_screen.to_vec(),
                floor_fill,
                Stroke::new(1.8_f32, Color32::from_rgb(94, 183, 235)),
            ));
            for index in 0..4 {
                painter.line_segment(
                    [top_screen[index], floor_screen[index]],
                    Stroke::new(1.4_f32, Color32::from_rgb(94, 183, 235)),
                );
            }
            painter.text(
                floor_screen.iter().copied().fold(Pos2::ZERO, |sum, point| {
                    Pos2::new(sum.x + point.x * 0.25, sum.y + point.y * 0.25)
                }),
                egui::Align2::CENTER_CENTER,
                format!("↓ {} mm", format_height(preview.depth_mm)),
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }

        if let Some((start, end)) = self.measure_span() {
            let from = self.project(start, response.rect);
            let to = self.project(end, response.rect);
            let stroke = Stroke::new(2.0_f32, Color32::from_rgb(120, 205, 255));
            painter.line_segment([from, to], stroke);
            painter.circle_filled(from, 3.5, stroke.color);
            painter.circle_filled(to, 3.5, stroke.color);
            painter.text(
                from.lerp(to, 0.5) - Vec2::new(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                format!(
                    "{} mm",
                    format_height(vector_length(Vec3::new(
                        end.x - start.x,
                        end.y - start.y,
                        end.z - start.z,
                    )))
                ),
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
        let measure_vertex = (self.active_tool == ActiveTool::Measure)
            .then(|| {
                ui.input(|input| input.pointer.hover_pos())
                    .and_then(|pointer| {
                        self.nearest_model_vertex_at_screen(pointer, response.rect, 12.0)
                    })
            })
            .flatten();
        if let Some(position) = measure_vertex.or_else(|| {
            self.hover_snap
                .as_ref()
                .filter(|snap| snap.kind != SnapKind::Face)
                .map(|snap| snap.position_mm)
        }) {
            let centre = self.project(position, response.rect);
            let stroke = Stroke::new(1.5_f32, Color32::from_rgb(80, 206, 190));
            painter.circle_stroke(centre, 6.0, stroke);
            painter.line_segment(
                [centre - Vec2::splat(4.0), centre + Vec2::splat(4.0)],
                stroke,
            );
            painter.line_segment(
                [centre + Vec2::new(-4.0, 4.0), centre + Vec2::new(4.0, -4.0)],
                stroke,
            );
        }
        if response.secondary_clicked()
            && let Some(target) = self.hovered.clone()
            && !self.selection.contains(&target.instance_path)
        {
            self.select_from_viewport(Some(target), false);
        }
        response.context_menu(|ui| self.show_viewport_context_menu(ui));
        self.viewport_overlays(ui, response.rect);
    }

    /// The viewport's right-click menu.
    ///
    /// Ordered the way SketchUp orders it: what the click landed on, then the
    /// edits that apply to it, then the commands that only move the camera.
    /// A right-click first selects whatever is under the pointer, so the menu
    /// always acts on the thing the user pointed at.
    fn show_viewport_context_menu(&mut self, ui: &mut egui::Ui) {
        if self.selection_count() > 0 {
            ui.label(
                egui::RichText::new(self.catalog.format(
                    "status-selected",
                    &BTreeMap::from([("count", self.selection_count().to_string())]),
                ))
                .color(self.palette().faint)
                .size(SHELL_SMALL_SIZE),
            );
            ui.separator();
            if ui.button(self.catalog.text("context-edit")).clicked() {
                if let Some(group_id) = self.selected_group_id() {
                    self.enter_group_context(group_id);
                } else if let Some(target) = self.selection.primary.clone() {
                    self.enter_occurrence_context(target.instance_path);
                }
                ui.close();
            }
            self.menu_command(ui, AppCommand::Delete);
            self.menu_command(ui, AppCommand::Hide);
            ui.separator();
            self.menu_command(ui, AppCommand::Group);
            self.menu_command(ui, AppCommand::Ungroup);
            self.menu_command(ui, AppCommand::MakeComponent);
            self.menu_command(ui, AppCommand::MakeUnique);
            ui.separator();
            self.menu_command(ui, AppCommand::Copy);
        }
        self.menu_command(ui, AppCommand::Paste);
        self.menu_command(ui, AppCommand::SelectAll);
        self.menu_command(ui, AppCommand::Deselect);
        self.menu_command(ui, AppCommand::Unhide);
        if self.edit_context_depth() > 0 {
            ui.separator();
            if ui
                .button(self.catalog.text("context-close-context"))
                .clicked()
            {
                self.exit_edit_context();
                ui.close();
            }
        }
        ui.separator();
        self.menu_command(ui, AppCommand::ZoomFit);
        self.menu_command(ui, AppCommand::ViewProjection);
    }

    fn pick_result_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        tolerance_px: f64,
    ) -> Option<PickResult> {
        let ray = self.view_ray(pointer, rect)?;
        let snapshot = self.document.current();
        self.refresh_interaction_projection_cache(&snapshot);
        let cache = self.interaction_projection_cache.borrow();
        let cache = cache.as_ref().expect("interaction cache was built");
        let scale = f64::from(self.zoom) * f64::from(rect.width().min(rect.height())) / 420.0;
        let exact_hit = cache.exact.exact_surface_pick(ray);
        let mesh_hit = cache.mesh.exact_surface_pick(ray);
        let box_pick = cache.boxes.exact_pick(ray, tolerance_px / scale);
        let prefer_exact = exact_hit.as_ref().is_some_and(|exact| {
            mesh_hit
                .as_ref()
                .is_none_or(|mesh| exact.ray_distance_mm <= mesh.ray_distance_mm)
                && box_pick
                    .as_ref()
                    .is_none_or(|pick| exact.ray_distance_mm <= pick.primary.ray_distance_mm)
        });
        if prefer_exact && let Some(hit) = exact_hit {
            let element = hit
                .durable_target
                .as_ref()
                .and_then(|target| target.body.role())
                .and_then(exact_face_element)
                .or_else(|| exact_surface_element(hit.outward_normal))?;
            let reference = SelectionId {
                definition_id: hit.definition_id,
                instance_path: hit.instance_path,
                element,
            };
            let primary = ExactHit {
                reference: reference.clone(),
                position_mm: hit.position_mm,
                ray_distance_mm: hit.ray_distance_mm,
            };
            let scale = f64::from(self.zoom) * f64::from(rect.width().min(rect.height())) / 420.0;
            let proxy_pick = cache.proxies.exact_pick(ray, tolerance_px / scale);
            let snap = proxy_pick
                .as_ref()
                .filter(|pick| {
                    pick.primary.reference.instance_path == primary.reference.instance_path
                })
                .map(|pick| pick.snap.clone())
                .unwrap_or(SnapResult {
                    kind: SnapKind::Face,
                    reference,
                    position_mm: hit.position_mm,
                    distance_mm: 0.0,
                });
            let mut overlapping = vec![primary.clone()];
            if let Some(proxy_pick) = proxy_pick {
                overlapping.extend(proxy_pick.overlapping.into_iter().filter(|candidate| {
                    candidate.reference.instance_path != primary.reference.instance_path
                }));
            }
            return Some(PickResult {
                primary,
                overlapping,
                snap,
            });
        }
        let prefer_mesh = mesh_hit.as_ref().is_some_and(|mesh| {
            box_pick
                .as_ref()
                .is_none_or(|pick| mesh.ray_distance_mm <= pick.primary.ray_distance_mm)
        });
        if prefer_mesh && let Some(hit) = mesh_hit {
            let reference = SelectionId {
                definition_id: hit.definition_id,
                instance_path: hit.instance_path,
                element: face_element_from_normal(hit.outward_normal),
            };
            let primary = ExactHit {
                reference: reference.clone(),
                position_mm: hit.position_mm,
                ray_distance_mm: hit.ray_distance_mm,
            };
            return Some(PickResult {
                primary: primary.clone(),
                overlapping: vec![primary],
                snap: SnapResult {
                    kind: SnapKind::Face,
                    reference,
                    position_mm: hit.position_mm,
                    distance_mm: 0.0,
                },
            });
        }

        box_pick
    }

    fn exact_pick_at_screen(&self, pointer: Pos2, rect: Rect) -> Option<SelectionId> {
        self.pick_result_at_screen(pointer, rect, 8.0)
            .map(|result| result.primary.reference)
    }

    #[cfg(test)]
    fn interaction_projection_cache_ptrs(&self) -> Option<(*const (), *const (), *const ())> {
        let cache = self.interaction_projection_cache.borrow();
        let cache = cache.as_ref()?;
        Some((
            std::ptr::from_ref(&cache.exact).cast(),
            std::ptr::from_ref(&cache.mesh).cast(),
            std::ptr::from_ref(&cache.boxes).cast(),
        ))
    }

    fn update_viewport_inference(&mut self, pointer: Option<Pos2>, rect: Rect) {
        let pick = pointer.and_then(|pointer| self.pick_result_at_screen(pointer, rect, 12.0));
        let previous = self.hover_pick.as_ref().map(overlap_signature);
        let current = pick.as_ref().map(overlap_signature);
        if previous != current {
            self.hover_overlap_index = 0;
        }
        let scale = f64::from(self.zoom) * f64::from(rect.width().min(rect.height())) / 420.0;
        let policy = SnapPolicy::new(8.0 / scale, 12.0 / scale)
            .expect("positive viewport snap tolerances are valid");
        self.hover_snap = self.snap_tracker.update(pick.as_ref(), policy).cloned();
        if matches!(self.active_tool, ActiveTool::Circle | ActiveTool::Arc)
            && self
                .hover_snap
                .as_ref()
                .is_none_or(|snap| snap.kind == SnapKind::Face)
            && let Some(pointer) = pointer
        {
            let plane_z = self
                .sketch_start
                .map_or_else(|| self.rectangle_plane_z(pointer, rect), |start| start.z);
            if let Some(snap) = self.profile_special_snap_at_screen(pointer, rect, plane_z) {
                self.hover_snap = Some(snap);
            }
        }
        self.hover_pick = pick;
        self.refresh_hover_choice();
    }

    fn refresh_hover_choice(&mut self) {
        self.hovered = self
            .hover_pick
            .as_ref()
            .and_then(|pick| pick.overlap_choice(self.hover_overlap_index))
            .map(|hit| hit.reference.clone());
    }

    pub fn cycle_hover_overlap(&mut self) -> bool {
        let Some(count) = self
            .hover_pick
            .as_ref()
            .map(|pick| pick.overlapping.len())
            .filter(|count| *count > 1)
        else {
            return false;
        };
        self.hover_overlap_index = (self.hover_overlap_index + 1) % count;
        self.refresh_hover_choice();
        self.digest = self.catalog.format(
            "digest-overlap-choice",
            &BTreeMap::from([
                ("index", (self.hover_overlap_index + 1).to_string()),
                ("count", count.to_string()),
            ]),
        );
        true
    }

    fn viewport_point_at_screen(&self, pointer: Pos2, rect: Rect, plane_z: f64) -> Option<Vec3> {
        self.pick_result_at_screen(pointer, rect, 8.0)
            .map(|pick| pick.snap)
            .filter(|snap| snap.kind != SnapKind::Face)
            .map(|snap| snap.position_mm)
            .or_else(|| {
                self.profile_special_snap_at_screen(pointer, rect, plane_z)
                    .map(|snap| snap.position_mm)
            })
            .or_else(|| self.screen_to_plane(pointer, rect, plane_z))
    }

    pub fn measurement_point_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        plane_z: f64,
    ) -> Option<Vec3> {
        self.nearest_model_vertex_at_screen(pointer, rect, 12.0)
            .or_else(|| self.viewport_point_at_screen(pointer, rect, plane_z))
            .or_else(|| self.surface_point_at_screen(pointer, rect))
    }

    fn nearest_model_vertex_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        tolerance_px: f32,
    ) -> Option<Vec3> {
        let snapshot = self.document.current();
        snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .filter_map(|occurrence| {
                let definition = snapshot.definition(occurrence.definition_id)?;
                let vertices = definition.feature_ids().iter().find_map(|feature_id| {
                    let FeatureKind::MeshBody(mesh) = snapshot.feature(*feature_id)?.kind() else {
                        return None;
                    };
                    Some(&mesh.vertices_mm)
                })?;
                Some((occurrence.transform, vertices))
            })
            .flat_map(|(transform, vertices)| {
                vertices.iter().map(move |point| {
                    transform_model_point(transform, Vec3::new(point[0], point[1], point[2]))
                })
            })
            .map(|point| (self.project(point, rect).distance(pointer), point))
            .filter(|(distance, _)| *distance <= tolerance_px)
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, point)| point)
    }

    fn profile_special_snap_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        plane_z: f64,
    ) -> Option<SnapResult> {
        let snapshot = self.document.current();
        snapshot
            .occurrences()
            .filter_map(|occurrence| {
                let definition = snapshot.definition(occurrence.definition_id())?;
                let (center, radius) = definition.feature_ids().iter().find_map(|feature_id| {
                    let FeatureKind::SegmentProfile { segments, closed } =
                        snapshot.feature(*feature_id)?.kind()
                    else {
                        return None;
                    };
                    exact_circle_geometry(segments, *closed)
                })?;
                let transform = occurrence.transform();
                let matrix = transform.matrix();
                let world_center = Vec3::new(
                    matrix[0] * center[0] + matrix[1] * center[1] + matrix[3],
                    matrix[4] * center[0] + matrix[5] * center[1] + matrix[7],
                    matrix[8] * center[0] + matrix[9] * center[1] + matrix[11],
                );
                if (world_center.z - plane_z).abs() > 1.0e-6 {
                    return None;
                }
                let world_radius = radius * matrix[0].hypot(matrix[4]).hypot(matrix[8]);
                let reference = SelectionId {
                    definition_id: occurrence.definition_id(),
                    instance_path: InstancePath::root(occurrence.id()),
                    element: ElementId::Face {
                        axis: Axis::Z,
                        side: Side::Maximum,
                    },
                };
                Some((world_center, world_radius, reference))
            })
            .flat_map(|(center, radius, reference)| {
                let mut candidates = vec![(SnapKind::Center, center)];
                if self.active_tool == ActiveTool::Arc
                    && self.sketch_end.is_none()
                    && let Some(anchor) = self.sketch_start
                {
                    candidates.extend(
                        tangent_points(anchor, center, radius)
                            .into_iter()
                            .map(|point| (SnapKind::Tangent, point)),
                    );
                }
                candidates
                    .into_iter()
                    .map(move |(kind, position_mm)| (kind, position_mm, reference.clone()))
            })
            .filter_map(|(kind, position_mm, reference)| {
                let distance_px = self.project(position_mm, rect).distance(pointer);
                (distance_px <= 8.0).then_some((
                    distance_px,
                    SnapResult {
                        kind,
                        reference,
                        position_mm,
                        distance_mm: 0.0,
                    },
                ))
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, snap)| snap)
    }

    fn world_to_clip(&self, rect: Rect) -> [f32; 16] {
        if self.projection_mode == ProjectionMode::Perspective {
            return self.perspective_world_to_clip(rect);
        }
        let yaw_sin = self.yaw.sin();
        let yaw_cos = self.yaw.cos();
        let pitch_sin = self.pitch.sin();
        let pitch_cos = self.pitch.cos();
        let scale = self.zoom * rect.width().min(rect.height()) / 420.0;
        let centre_x = BOX_WIDTH_MM as f32 * 0.5;
        let centre_y = BOX_DEPTH_MM as f32 * 0.5;
        let centre_z = self.camera_target_z as f32;
        let sx = 2.0 / rect.width();
        let sy = 2.0 / rect.height();
        let x_constant =
            rect.width() * 0.5 + self.pan.x - scale * (yaw_cos * centre_x - yaw_sin * centre_y);
        let y_constant = rect.height() * 0.5
            + self.pan.y
            + scale
                * (yaw_sin * pitch_cos * centre_x + yaw_cos * pitch_cos * centre_y
                    - pitch_sin * centre_z);
        [
            sx * scale * yaw_cos,
            sy * scale * yaw_sin * pitch_cos,
            0.0,
            0.0,
            -sx * scale * yaw_sin,
            sy * scale * yaw_cos * pitch_cos,
            0.0,
            0.0,
            0.0,
            -sy * scale * pitch_sin,
            0.0,
            0.0,
            sx * x_constant - 1.0,
            1.0 - sy * y_constant,
            0.0,
            1.0,
        ]
    }

    /// The converging counterpart of [`Self::world_to_clip`].
    ///
    /// The clip `w` carries the eye-space depth, so the fixed-function divide
    /// gives exactly the same picture the CPU painter draws in
    /// [`Self::project`]. There is no depth buffer, so clip `z` stays zero.
    fn perspective_world_to_clip(&self, rect: Rect) -> [f32; 16] {
        let (right, up, forward) = self.camera_basis();
        let eye = self.camera_target() - forward * self.camera_distance();
        let focal = self.camera_focal(rect);
        let sx = f64::from(2.0 / rect.width());
        let sy = f64::from(2.0 / rect.height());
        let a = sx * f64::from(rect.width() * 0.5 + self.pan.x) - 1.0;
        let b = 1.0 - sy * f64::from(rect.height() * 0.5 + self.pan.y);

        let clip_x = forward * a + right * (sx * focal);
        let clip_y = forward * b + up * (sy * focal);
        let clip_w = forward;
        [
            clip_x.x as f32,
            clip_y.x as f32,
            0.0,
            clip_w.x as f32,
            clip_x.y as f32,
            clip_y.y as f32,
            0.0,
            clip_w.y as f32,
            clip_x.z as f32,
            clip_y.z as f32,
            0.0,
            clip_w.z as f32,
            -dot(clip_x, eye) as f32,
            -dot(clip_y, eye) as f32,
            0.0,
            -dot(clip_w, eye) as f32,
        ]
    }

    fn project(&self, point: Vec3, rect: Rect) -> Pos2 {
        let (right, up, forward) = self.camera_basis();
        let centered = point - self.camera_target();
        let view_x = dot(centered, right);
        let view_y = dot(centered, up);
        let scale = match self.projection_mode {
            ProjectionMode::Parallel => self.view_scale(rect),
            ProjectionMode::Perspective => {
                // Depth measured from the eye. Points at or behind the eye have
                // no on-screen position, so they are clamped to a sliver in
                // front of it rather than folded through the origin.
                let depth =
                    (self.camera_distance() + dot(centered, forward)).max(PERSPECTIVE_NEAR_MM);
                self.camera_focal(rect) / depth
            }
        };
        Pos2::new(
            rect.center().x + self.pan.x + (view_x * scale) as f32,
            rect.center().y + self.pan.y - (view_y * scale) as f32,
        )
    }

    fn handle_shortcuts(&mut self, context: &egui::Context) {
        let new_document = context.input(|input| {
            input.modifiers.command && !input.modifiers.shift && input.key_pressed(egui::Key::N)
        });
        let open_document = context.input(|input| {
            input.modifiers.command && !input.modifiers.shift && input.key_pressed(egui::Key::O)
        });
        let save_as = context.input(|input| {
            input.modifiers.command && input.modifiers.shift && input.key_pressed(egui::Key::S)
        });
        let save_document = context.input(|input| {
            input.modifiers.command && !input.modifiers.shift && input.key_pressed(egui::Key::S)
        });
        let undo =
            context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
        let redo =
            context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y));
        let copy = !context.wants_keyboard_input()
            && context.input_mut(|input| {
                let mut native_copy = false;
                input.events.retain(|event| {
                    let is_copy = matches!(event, egui::Event::Copy);
                    native_copy |= is_copy;
                    !is_copy
                });
                native_copy || input.consume_key(egui::Modifiers::COMMAND, egui::Key::C)
            });
        let paste = !context.wants_keyboard_input()
            && context.input_mut(|input| {
                let mut native_paste = false;
                input.events.retain(|event| {
                    let is_paste = matches!(event, egui::Event::Paste(_));
                    native_paste |= is_paste;
                    !is_paste
                });
                native_paste || input.consume_key(egui::Modifiers::COMMAND, egui::Key::V)
            });
        let select_all = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::A));
        let delete = !context.wants_keyboard_input()
            && context.input(|input| input.key_pressed(egui::Key::Delete));
        let select = !context.wants_keyboard_input()
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Space));
        let rectangle = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::R));
        let circle = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::C));
        let arc = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::A));
        let push_pull = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::P));
        let move_tool = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::M));
        let measure = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::T));
        let zoom_fit = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::Z));
        let cycle_overlap = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
        let shortcuts = context.input(|input| input.key_pressed(egui::Key::F1));
        let group = !context.wants_keyboard_input()
            && context.input(|input| {
                input.modifiers.command && !input.modifiers.shift && input.key_pressed(egui::Key::G)
            });
        let ungroup = !context.wants_keyboard_input()
            && context.input(|input| {
                input.modifiers.command && input.modifiers.shift && input.key_pressed(egui::Key::G)
            });
        let confirm_box_preview = !context.wants_keyboard_input()
            && self.has_preview()
            && context.input(|input| input.key_pressed(egui::Key::Enter));
        let confirm_operation_preview = !context.wants_keyboard_input()
            && self.has_occurrence_operation_preview()
            && context.input(|input| input.key_pressed(egui::Key::Enter));
        let confirm_sweep_preview = !context.wants_keyboard_input()
            && self.sweep_preview.is_some()
            && context.input(|input| input.key_pressed(egui::Key::Enter));
        let confirm_loft_preview = !context.wants_keyboard_input()
            && self.loft_preview.is_some()
            && context.input(|input| input.key_pressed(egui::Key::Enter));
        let escape = context.input(|input| input.key_pressed(egui::Key::Escape));
        if !context.wants_keyboard_input() {
            let typed = context.input(|input| {
                input
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::Text(text)
                            if text
                                .chars()
                                .all(|character| "0123456789.,-;xX* ".contains(character)) =>
                        {
                            Some(text.as_str())
                        }
                        _ => None,
                    })
                    .collect::<String>()
            });
            if !typed.is_empty() {
                self.value_input.clear();
                self.value_input.push_str(&typed);
                self.focus_value_box = true;
                if self.active_tool == ActiveTool::PlanarOffset {
                    self.refresh_planar_offset_preview();
                } else if self.active_tool == ActiveTool::Revolve {
                    self.refresh_revolve_preview();
                } else if matches!(
                    self.active_tool,
                    ActiveTool::Shell | ActiveTool::Fillet | ActiveTool::Chamfer
                ) {
                    self.refresh_general_finish_preview();
                }
            }
        }

        if new_document {
            self.dispatch_command(AppCommand::New);
        } else if open_document {
            self.dispatch_command(AppCommand::Open);
        } else if save_as {
            self.dispatch_command(AppCommand::SaveAs);
        } else if save_document {
            self.dispatch_command(AppCommand::Save);
        } else if undo {
            self.dispatch_command(AppCommand::Undo);
        } else if redo {
            self.dispatch_command(AppCommand::Redo);
        } else if copy {
            if self.command_enabled(AppCommand::Copy) {
                self.dispatch_command(AppCommand::Copy);
                context.copy_text("Ketchup object selection".to_owned());
            }
        } else if paste {
            self.dispatch_command(AppCommand::Paste);
        } else if select_all {
            self.dispatch_command(AppCommand::SelectAll);
        } else if group {
            self.dispatch_command(AppCommand::Group);
        } else if ungroup {
            self.dispatch_command(AppCommand::Ungroup);
        } else if delete {
            self.dispatch_command(AppCommand::Delete);
        } else if select {
            self.dispatch_command(AppCommand::Select);
        } else if rectangle {
            self.dispatch_command(AppCommand::Rectangle);
        } else if circle {
            self.dispatch_command(AppCommand::Circle);
        } else if arc {
            self.dispatch_command(AppCommand::Arc);
        } else if push_pull {
            self.dispatch_command(AppCommand::PushPull);
        } else if move_tool {
            self.dispatch_command(AppCommand::Move);
        } else if measure {
            self.dispatch_command(AppCommand::Measure);
        } else if zoom_fit {
            self.dispatch_command(AppCommand::ZoomFit);
        } else if cycle_overlap {
            self.cycle_hover_overlap();
        } else if shortcuts {
            self.dispatch_command(AppCommand::Shortcuts);
        } else if confirm_box_preview {
            self.confirm_preview();
        } else if confirm_operation_preview {
            self.confirm_push_pull_preview();
        } else if confirm_sweep_preview {
            self.confirm_sweep_preview();
        } else if confirm_loft_preview {
            self.confirm_loft_preview();
        } else if escape {
            if self.measure_start.is_some() {
                self.clear_measurement();
                self.digest = self.catalog.text("digest-measure-cleared");
                self.status_key = "status-measure-first-point";
            } else if self.has_preview()
                || self.smart_push_pull_chooser.is_some()
                || self.has_pocket_preview()
                || self.has_occurrence_operation_preview()
                || self.revolve_tool.is_some()
                || self.revolve_preview.is_some()
                || self.planar_offset_preview.is_some()
                || self.sweep_preview.is_some()
                || self.loft_preview.is_some()
                || self.general_finish_preview.is_some()
                || self.solid_tool_target.is_some()
                || self.sketch_mode
            {
                self.clear_ephemeral_edit_state();
                self.cancel_rectangle_sketch();
                self.digest = self.catalog.text("digest-cancelled");
            } else if self.selection_count() > 0 {
                self.dispatch_command(AppCommand::Deselect);
            } else {
                self.exit_edit_context();
            }
        }
    }

    fn command_button(&mut self, ui: &mut egui::Ui, id: AppCommand) {
        let spec = CommandRegistry::spec(id);
        let label = self.catalog.text(spec.label_key);
        let shortcut = self.catalog.text(spec.shortcut_key);
        let enabled = self.command_enabled(id);
        if ui
            .add_enabled(enabled, egui::Button::new(label))
            .on_hover_text(shortcut)
            .clicked()
        {
            self.dispatch_command(id);
        }
    }

    fn menu_command(&mut self, ui: &mut egui::Ui, id: AppCommand) {
        let spec = CommandRegistry::spec(id);
        let label = self.catalog.format(
            "menu-command",
            &BTreeMap::from([
                ("label", self.catalog.text(spec.label_key)),
                ("shortcut", self.catalog.text(spec.shortcut_key)),
            ]),
        );
        let enabled = self.command_enabled(id);
        let response = ui.add_enabled(enabled, egui::Button::new(label));
        name_widget(&response, enabled, &self.catalog.text(spec.label_key));
        if response.clicked() {
            self.dispatch_command(id);
            ui.close();
        }
    }

    fn disabled_menu_item(&self, ui: &mut egui::Ui, key: &str) {
        ui.add_enabled(false, egui::Button::new(self.catalog.text(key)));
    }

    fn show_top_bar(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            let (logo, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
            ui.painter()
                .rect_filled(logo, egui::CornerRadius::same(7), palette.accent);
            theme::paint_icon(
                ui.painter(),
                logo,
                Icon::Logo,
                palette.accent_ink,
                palette.accent_ink,
                2.1,
            );
            ui.label(
                egui::RichText::new(self.catalog.text("app-title"))
                    .size(14.0)
                    .strong(),
            );

            vertical_rule(ui, palette);

            // The extension is set in the tertiary tone so the eye lands on the
            // model's name, and unsaved work is an accent dot instead of a `*`.
            let title = self.document_title();
            let (stem, extension) = title
                .trim_end_matches(" *")
                .rsplit_once('.')
                .map_or((title.trim_end_matches(" *"), ""), |(stem, extension)| {
                    (stem, extension)
                });
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(egui::RichText::new(stem).color(palette.dim));
            if !extension.is_empty() {
                ui.label(egui::RichText::new(format!(".{extension}")).color(palette.faint));
            }
            ui.spacing_mut().item_spacing.x = 8.0;
            if title.ends_with(" *") {
                let (dot, response) = ui.allocate_exact_size(Vec2::splat(9.0), Sense::hover());
                ui.painter()
                    .circle_filled(dot.center(), 2.5, palette.accent);
                response.on_hover_text(self.catalog.text("status-unsaved"));
            }

            ui.spacing_mut().item_spacing.x = 2.0;
            for (id, icon) in [
                (AppCommand::Undo, Icon::Undo),
                (AppCommand::Redo, Icon::Redo),
            ] {
                if self.icon_button(ui, id, icon, Vec2::new(28.0, 26.0)) {
                    self.dispatch_command(id);
                }
            }

            let views = [
                AppCommand::ViewIso,
                AppCommand::ViewTop,
                AppCommand::ViewFront,
                AppCommand::ZoomFit,
            ];
            let chips = ThemeKind::ALL;
            // Both clusters are laid out from the right so the centre one keeps
            // its place as the document name grows.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                segmented(ui, palette, |ui| {
                    for kind in chips {
                        self.theme_chip(ui, palette, kind);
                    }
                });
                ui.add_space(ui.available_width() * 0.5 - 150.0);
                segmented(ui, palette, |ui| {
                    for id in views {
                        if self.segment_button(ui, palette, id, false) {
                            self.dispatch_command(id);
                        }
                    }
                    vertical_rule(ui, palette);
                    if self.segment_button(ui, palette, AppCommand::ViewProjection, true) {
                        self.dispatch_command(AppCommand::ViewProjection);
                    }
                });
            });
        });
    }

    /// A frameless glyph button in the chrome, e.g. Undo.
    fn icon_button(&self, ui: &mut egui::Ui, id: AppCommand, icon: Icon, size: Vec2) -> bool {
        let palette = self.palette();
        let enabled = self.command_enabled(id);
        let label = self.command_label(id);
        let response = ui.add_enabled(enabled, egui::Button::new("").min_size(size).frame(false));
        let ink = if !enabled {
            palette.faint
        } else if response.hovered() {
            palette.text
        } else {
            palette.dim
        };
        if enabled && response.hovered() {
            ui.painter()
                .rect_filled(response.rect, egui::CornerRadius::same(6), palette.panel2);
        }
        theme::paint_icon(
            ui.painter(),
            shrink_to_icon(response.rect, 15.0),
            icon,
            ink,
            if enabled { palette.accent } else { ink },
            1.7,
        );
        name_widget(&response, enabled, &label);
        response
            .on_hover_text(self.catalog.text(CommandRegistry::spec(id).shortcut_key))
            .clicked()
    }

    /// One pill inside a segmented control. `accent_when_on` fills the pill.
    fn segment_button(
        &self,
        ui: &mut egui::Ui,
        palette: Palette,
        id: AppCommand,
        accent_when_on: bool,
    ) -> bool {
        let label = if id == AppCommand::ViewProjection {
            self.catalog.text(self.projection_mode.label_key())
        } else {
            self.command_label(id)
        };
        let enabled = self.command_enabled(id);
        let response =
            ui.add_enabled(
                enabled,
                egui::Button::new(egui::RichText::new(&label).size(12.0).color(
                    if accent_when_on {
                        palette.accent_ink
                    } else {
                        palette.dim
                    },
                ))
                .fill(if accent_when_on {
                    palette.accent
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(Stroke::NONE)
                .corner_radius(egui::CornerRadius::same(6))
                .min_size(Vec2::new(0.0, 26.0)),
            );
        name_widget(&response, enabled, &label);
        response.clicked()
    }

    /// A theme chip: colour swatch plus name, outlined while it is the active one.
    ///
    /// Drawn by hand rather than as a `Button`, because the swatch has to sit
    /// inside the chip's own padding and still belong to the same hit target.
    fn theme_chip(&mut self, ui: &mut egui::Ui, palette: Palette, kind: ThemeKind) {
        let selected = self.theme == kind;
        let label = self.catalog.text(kind.label_key());
        let font = egui::FontId::proportional(11.5);
        let galley = ui.painter().layout_no_wrap(
            label.clone(),
            font,
            if selected { palette.text } else { palette.dim },
        );
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(galley.size().x + 30.0, 24.0), Sense::click());
        let painter = ui.painter();
        let corner = egui::CornerRadius::same(6);
        if selected {
            painter.rect_filled(rect, corner, palette.panel2);
            painter.rect_stroke(
                rect,
                corner,
                Stroke::new(1.0_f32, palette.line),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(rect, corner, palette.panel);
        }
        painter.rect_filled(
            Rect::from_center_size(
                Pos2::new(rect.left() + 12.0, rect.center().y),
                Vec2::splat(12.0),
            ),
            egui::CornerRadius::same(4),
            Palette::of(kind).accent,
        );
        painter.galley(
            Pos2::new(rect.left() + 24.0, rect.center().y - galley.size().y * 0.5),
            galley,
            palette.text,
        );
        name_widget(&response, true, &label);
        if response.clicked() {
            self.set_theme(kind);
        }
    }

    fn show_menu_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // A menu bar is not a row of buttons: the top-level entries stay
            // frameless until they are hovered, the way the platform draws them.
            let widgets = &mut ui.visuals_mut().widgets;
            widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            widgets.inactive.bg_stroke = Stroke::NONE;
            widgets.hovered.bg_stroke = Stroke::NONE;
            widgets.active.bg_stroke = Stroke::NONE;
            widgets.open.bg_stroke = Stroke::NONE;
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = Vec2::new(9.0, 3.0);
            ui.menu_button(self.catalog.text("menu-file"), |ui| {
                self.menu_command(ui, AppCommand::New);
                self.menu_command(ui, AppCommand::Open);
                self.menu_command(ui, AppCommand::Save);
                self.menu_command(ui, AppCommand::SaveAs);
                ui.separator();
                self.menu_command(ui, AppCommand::ImportMeshStl);
                ui.separator();
                self.menu_command(ui, AppCommand::ExportExactStep);
                self.menu_command(ui, AppCommand::ExportMeshStl);
            });
            ui.menu_button(self.catalog.text("menu-edit"), |ui| {
                self.menu_command(ui, AppCommand::Undo);
                self.menu_command(ui, AppCommand::Redo);
                ui.separator();
                self.menu_command(ui, AppCommand::Copy);
                self.menu_command(ui, AppCommand::Paste);
                self.menu_command(ui, AppCommand::Delete);
                self.menu_command(ui, AppCommand::Group);
                self.menu_command(ui, AppCommand::Ungroup);
                self.menu_command(ui, AppCommand::MakeUnique);
                self.menu_command(ui, AppCommand::SelectAll);
                self.menu_command(ui, AppCommand::Deselect);
            });
            ui.menu_button(self.catalog.text("menu-view"), |ui| {
                self.menu_command(ui, AppCommand::ViewIso);
                self.menu_command(ui, AppCommand::ViewTop);
                self.menu_command(ui, AppCommand::ViewFront);
                self.menu_command(ui, AppCommand::ZoomFit);
                ui.separator();
                self.menu_command(ui, AppCommand::ViewProjection);
                ui.separator();
                self.menu_command(ui, AppCommand::Hide);
                self.menu_command(ui, AppCommand::Unhide);
            });
            ui.menu_button(self.catalog.text("menu-draw"), |ui| {
                self.menu_command(ui, AppCommand::Line);
                self.menu_command(ui, AppCommand::Rectangle);
                self.menu_command(ui, AppCommand::Circle);
                self.menu_command(ui, AppCommand::Arc);
            });
            ui.menu_button(self.catalog.text("menu-tools"), |ui| {
                self.menu_command(ui, AppCommand::Select);
                self.menu_command(ui, AppCommand::PushPull);
                self.menu_command(ui, AppCommand::Move);
                self.menu_command(ui, AppCommand::Measure);
                self.menu_command(ui, AppCommand::Orbit);
                self.menu_command(ui, AppCommand::Pan);
            });
            ui.menu_button(self.catalog.text("menu-model"), |ui| {
                self.menu_command(ui, AppCommand::CutThrough);
                self.menu_command(ui, AppCommand::Pocket);
                self.menu_command(ui, AppCommand::PlanarOffset);
                self.menu_command(ui, AppCommand::Sweep);
                self.menu_command(ui, AppCommand::Loft);
                self.menu_command(ui, AppCommand::Revolve);
                self.menu_command(ui, AppCommand::Shell);
                self.menu_command(ui, AppCommand::Fillet);
                self.menu_command(ui, AppCommand::Chamfer);
                ui.separator();
                self.menu_command(ui, AppCommand::SolidSubtract);
                self.menu_command(ui, AppCommand::SolidUnion);
                self.menu_command(ui, AppCommand::SolidIntersect);
                self.menu_command(ui, AppCommand::SolidSplit);
                ui.separator();
                self.menu_command(ui, AppCommand::Group);
                self.menu_command(ui, AppCommand::Ungroup);
                self.menu_command(ui, AppCommand::MakeComponent);
                self.menu_command(ui, AppCommand::MakeUnique);
                ui.separator();
                self.disabled_menu_item(ui, "model-purge-unused");
            });
            ui.menu_button(self.catalog.text("menu-window"), |ui| {
                self.disabled_menu_item(ui, "dock-outliner");
                self.disabled_menu_item(ui, "dock-tags");
            });
            ui.menu_button(self.catalog.text("menu-help"), |ui| {
                self.menu_command(ui, AppCommand::Shortcuts);
                self.disabled_menu_item(ui, "help-about");
            });
        });
    }

    fn show_tool_rail(&mut self, ui: &mut egui::Ui) {
        // Grouped the way the design groups them: pick, draw, modify, measure,
        // navigate. A group boundary draws a hairline.
        const TOOLS: [(AppCommand, u8); 10] = [
            (AppCommand::Select, 0),
            (AppCommand::Line, 1),
            (AppCommand::Rectangle, 1),
            (AppCommand::Circle, 1),
            (AppCommand::Arc, 1),
            (AppCommand::PushPull, 2),
            (AppCommand::Move, 2),
            (AppCommand::Measure, 3),
            (AppCommand::Orbit, 4),
            (AppCommand::Pan, 4),
        ];
        let palette = self.palette();
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut group = TOOLS[0].1;
            for (id, tool_group) in TOOLS {
                if tool_group != group {
                    group = tool_group;
                    ui.add_space(5.0);
                    let (rule, _) = ui.allocate_exact_size(Vec2::new(20.0, 1.0), Sense::hover());
                    ui.painter().rect_filled(rule, 0.0, palette.line);
                    ui.add_space(5.0);
                }
                let spec = CommandRegistry::spec(id);
                let active = spec.tool == Some(self.active_tool);
                let enabled = self.command_enabled(id);
                let label = self.catalog.text(spec.label_key);
                let response = ui.add_enabled(
                    enabled,
                    egui::Button::new("")
                        .frame(false)
                        .min_size(Vec2::new(TOOL_BUTTON_SIZE, TOOL_BUTTON_SIZE)),
                );
                self.paint_rail_button(ui, &response, id, enabled, active);
                name_widget(&response, enabled, &label);
                if response
                    .on_hover_text(self.catalog.format(
                        "tool-tooltip",
                        &BTreeMap::from([
                            ("tool", label.clone()),
                            ("shortcut", self.catalog.text(spec.shortcut_key)),
                        ]),
                    ))
                    .clicked()
                {
                    self.dispatch_command(id);
                }
            }
            ui.add_space((ui.available_height() - TOOL_BUTTON_SIZE - 12.0).max(0.0));
            let enabled = self.command_enabled(AppCommand::Delete);
            let response = ui.add_enabled(
                enabled,
                egui::Button::new("")
                    .frame(false)
                    .min_size(Vec2::new(TOOL_BUTTON_SIZE, TOOL_BUTTON_SIZE)),
            );
            self.paint_rail_button(ui, &response, AppCommand::Delete, enabled, false);
            name_widget(&response, enabled, &self.command_label(AppCommand::Delete));
            if response
                .on_hover_text(self.catalog.text("tooltip-delete"))
                .clicked()
            {
                self.dispatch_command(AppCommand::Delete);
            }
        });
    }

    fn paint_projected_faces(&self, painter: &egui::Painter, faces: &[ProjectedFace]) {
        let mut underlay = egui::Mesh::default();
        let mut fills = Vec::with_capacity(faces.len());
        for face in faces {
            let color = if face.out_of_context {
                Color32::from_rgb(43, 47, 54)
            } else if self.active_tool == ActiveTool::PushPull
                && self.selection.primary.as_ref() == Some(&face.selection)
            {
                Color32::from_rgb(194, 89, 48)
            } else if self.selection.contains(&face.selection.instance_path) {
                Color32::from_rgb(154, 91, 67)
            } else if self.hovered.as_ref() == Some(&face.selection) {
                Color32::from_rgb(76, 111, 158)
            } else if face.previewed {
                Color32::from_rgb(58, 126, 174)
            } else {
                face.color
            };
            fills.push(color);
            let base = u32::try_from(underlay.vertices.len())
                .expect("a viewport face mesh must fit in u32 indices");
            let points = face.polygon.points();
            for point in points {
                underlay.colored_vertex(*point, color);
            }
            for index in 1..points.len() - 1 {
                let index = u32::try_from(index).expect("a face vertex count must fit in u32");
                underlay.add_triangle(base, base + index, base + index + 1);
            }
        }
        if !underlay.indices.is_empty() {
            painter.add(egui::Shape::mesh(underlay));
        }
        for (face, color) in faces.iter().zip(fills) {
            painter.add(egui::Shape::convex_polygon(
                face.polygon.points().to_vec(),
                color,
                Stroke::NONE,
            ));
        }
    }

    fn paint_scene_base_layers(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        scene_plan: Option<Arc<InstancedRenderPlan>>,
    ) {
        self.paint_ground_plane(painter, rect);
        if let Some(plan) = scene_plan {
            let (_, _, forward) = self.camera_basis();
            painter.add(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                ScenePaintCallback::new(
                    plan,
                    rect,
                    self.world_to_clip(rect),
                    [forward.x as f32, forward.y as f32, forward.z as f32, 0.0],
                ),
            ));
        }
    }

    /// Paint an adaptive construction grid and the three world axes on Z = 0.
    ///
    /// The painted patch follows the visible part of the ground plane. Its
    /// spacing advances through metric 1/2/5 steps so lines stay legible while
    /// the camera moves between millimetre details and kilometre-scale scenes.
    fn paint_ground_plane(&self, painter: &egui::Painter, rect: Rect) {
        let palette = self.palette();
        let scale = self.view_scale(rect).max(1.0e-12);
        let step = adaptive_grid_step(scale);
        let centre = self
            .screen_to_plane(rect.center(), rect, 0.0)
            .unwrap_or_else(|| self.camera_target());
        let half_reach = f64::from(rect.width().max(rect.height())) / scale * 1.25;
        let x_start = ((centre.x - half_reach) / step).floor() as i64;
        let x_end = ((centre.x + half_reach) / step).ceil() as i64;
        let y_start = ((centre.y - half_reach) / step).floor() as i64;
        let y_end = ((centre.y + half_reach) / step).ceil() as i64;
        let x_min = x_start as f64 * step;
        let x_max = x_end as f64 * step;
        let y_min = y_start as f64 * step;
        let y_max = y_end as f64 * step;
        let painter = painter.with_clip_rect(rect);
        let segment = |from: Vec3, to: Vec3, major: bool| {
            let stroke = Stroke::new(
                if major { 1.2_f32 } else { 1.0_f32 },
                if major {
                    palette.grid_major
                } else {
                    palette.grid
                },
            );
            painter.line_segment([self.project(from, rect), self.project(to, rect)], stroke);
        };
        for index in x_start..=x_end {
            if index != 0 {
                segment(
                    Vec3::new(index as f64 * step, y_min, 0.0),
                    Vec3::new(index as f64 * step, y_max, 0.0),
                    index % 5 == 0,
                );
            }
        }
        for index in y_start..=y_end {
            if index != 0 {
                segment(
                    Vec3::new(x_min, index as f64 * step, 0.0),
                    Vec3::new(x_max, index as f64 * step, 0.0),
                    index % 5 == 0,
                );
            }
        }
        // The axes are the one place the viewport does not use the palette: red,
        // green and blue for X, Y and Z is a convention the user already knows
        // from every other modeller, and it must not shift with the theme.
        for (from, to, color) in [
            (
                Vec3::new(x_min, 0.0, 0.0),
                Vec3::new(x_max, 0.0, 0.0),
                Color32::from_rgb(224, 86, 63),
            ),
            (
                Vec3::new(0.0, y_min, 0.0),
                Vec3::new(0.0, y_max, 0.0),
                Color32::from_rgb(93, 187, 99),
            ),
            (
                Vec3::new(0.0, 0.0, -half_reach),
                Vec3::new(0.0, 0.0, half_reach),
                Color32::from_rgb(78, 134, 199),
            ),
        ] {
            painter.line_segment(
                [self.project(from, rect), self.project(to, rect)],
                Stroke::new(1.6_f32, color),
            );
        }
    }

    /// Paint the plate and glyph of one rail button.
    ///
    /// The active tool is a filled accent tile with a matching glow, hover is
    /// the raised panel tone, and everything else is flat — so which tool is
    /// armed is readable without reading any text.
    fn paint_rail_button(
        &self,
        ui: &egui::Ui,
        response: &egui::Response,
        id: AppCommand,
        enabled: bool,
        active: bool,
    ) {
        let palette = self.palette();
        let painter = ui.painter();
        let corner = egui::CornerRadius::same(8);
        if active {
            painter.rect_filled(response.rect.expand(2.0), corner, palette.accent_wash(56));
            painter.rect_filled(response.rect, corner, palette.accent);
        } else if enabled && response.hovered() {
            painter.rect_filled(response.rect, corner, palette.panel2);
        }
        let ink = if !enabled {
            palette.faint
        } else if active {
            palette.accent_ink
        } else if response.hovered() {
            palette.text
        } else {
            palette.dim
        };
        // On the filled accent tile the accent detail would vanish into the
        // plate, so there the whole glyph is drawn in the tile's ink instead.
        let detail = if enabled && !active {
            palette.accent
        } else {
            ink
        };
        theme::paint_icon(
            painter,
            shrink_to_icon(response.rect, TOOL_ICON_SIZE),
            command_icon(id),
            ink,
            detail,
            1.55,
        );
    }

    fn prepare_assistant_from_inputs(&mut self) -> bool {
        let Ok(target) = self.assistant_target_input.trim().parse::<u64>() else {
            self.digest = self.catalog.text("assistant-error-target");
            return false;
        };
        let value_text = self.assistant_value_input.clone();
        let intent = match self.assistant_intent_kind {
            AssistantIntentKind::CreateEvaluatorInput => {
                let Some((name, value_text)) = value_text.split_once(':') else {
                    self.digest = self.catalog.text("assistant-error-create-evaluator-input");
                    return false;
                };
                WorkflowIntent::CreateEvaluatorInput {
                    target: NodeId(target),
                    name: name.trim().to_owned(),
                    value_text: value_text.trim().to_owned(),
                }
            }
            AssistantIntentKind::CreateEvaluatorExpression => {
                let Some((name, expression)) = value_text.split_once(':') else {
                    self.digest = self
                        .catalog
                        .text("assistant-error-create-evaluator-expression");
                    return false;
                };
                WorkflowIntent::CreateEvaluatorExpression {
                    target: NodeId(target),
                    name: name.trim().to_owned(),
                    expression: expression.trim().to_owned(),
                }
            }
            AssistantIntentKind::CreateEvaluatorRule => {
                let Some((name, expression)) = value_text.split_once(':') else {
                    self.digest = self.catalog.text("assistant-error-create-evaluator-rule");
                    return false;
                };
                WorkflowIntent::CreateEvaluatorRule {
                    target: NodeId(target),
                    name: name.trim().to_owned(),
                    expression: expression.trim().to_owned(),
                }
            }
            AssistantIntentKind::CreateRuleOverride => {
                let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
                if fields.len() != 5 {
                    self.digest = self.catalog.text("assistant-error-create-rule-override");
                    return false;
                }
                let Ok(rule) = fields[0].parse::<u64>() else {
                    self.digest = self.catalog.text("assistant-error-create-rule-override");
                    return false;
                };
                WorkflowIntent::CreateRuleOverride {
                    target,
                    rule: NodeId(rule),
                    output_port: fields[1].to_owned(),
                    semantic_key: fields[2].to_owned(),
                    parameter: fields[3].to_owned(),
                    value_text: fields[4].to_owned(),
                }
            }
            AssistantIntentKind::DeleteRuleOverride => {
                WorkflowIntent::DeleteRuleOverride { target }
            }
            AssistantIntentKind::CreateFeatureParameterBinding => {
                let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
                if fields.len() != 4 {
                    self.digest = self
                        .catalog
                        .text("assistant-error-create-feature-parameter-binding");
                    return false;
                }
                let slot = match fields[0] {
                    "height" => FeatureParameterSlot::Height,
                    "body_radius" => FeatureParameterSlot::BodyRadius,
                    "body_height" => FeatureParameterSlot::BodyHeight,
                    "shoulder_rise" => FeatureParameterSlot::ShoulderRise,
                    "thickness" => FeatureParameterSlot::Thickness,
                    "amount" => FeatureParameterSlot::Amount,
                    "profile_width" => FeatureParameterSlot::ProfileWidth,
                    "profile_height" => FeatureParameterSlot::ProfileHeight,
                    _ => {
                        self.digest = self
                            .catalog
                            .text("assistant-error-create-feature-parameter-binding");
                        return false;
                    }
                };
                let Ok(rule) = fields[1].parse::<u64>() else {
                    self.digest = self
                        .catalog
                        .text("assistant-error-create-feature-parameter-binding");
                    return false;
                };
                WorkflowIntent::CreateFeatureParameterBinding {
                    target: FeatureParameterTarget {
                        feature_id: FeatureId(target),
                        slot,
                    },
                    rule: NodeId(rule),
                    output_port: fields[2].to_owned(),
                    semantic_key: fields[3].to_owned(),
                }
            }
            AssistantIntentKind::CreatePersistentDimension => {
                let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
                if fields.len() != 5 {
                    self.digest = self
                        .catalog
                        .text("assistant-error-create-persistent-dimension");
                    return false;
                }
                let Ok(feature_id) = fields[1].parse::<u64>() else {
                    self.digest = self
                        .catalog
                        .text("assistant-error-create-persistent-dimension");
                    return false;
                };
                let slot = match fields[2] {
                    "height" => FeatureParameterSlot::Height,
                    "body_radius" => FeatureParameterSlot::BodyRadius,
                    "body_height" => FeatureParameterSlot::BodyHeight,
                    "shoulder_rise" => FeatureParameterSlot::ShoulderRise,
                    "thickness" => FeatureParameterSlot::Thickness,
                    "amount" => FeatureParameterSlot::Amount,
                    "profile_width" => FeatureParameterSlot::ProfileWidth,
                    "profile_height" => FeatureParameterSlot::ProfileHeight,
                    _ => {
                        self.digest = self
                            .catalog
                            .text("assistant-error-create-persistent-dimension");
                        return false;
                    }
                };
                let unit = match fields[3] {
                    "mm" => DimensionDisplayUnit::Millimetres,
                    "cm" => DimensionDisplayUnit::Centimetres,
                    "in" => DimensionDisplayUnit::Inches,
                    _ => {
                        self.digest = self
                            .catalog
                            .text("assistant-error-create-persistent-dimension");
                        return false;
                    }
                };
                let Ok(decimal_places) = fields[4].parse::<u8>() else {
                    self.digest = self
                        .catalog
                        .text("assistant-error-create-persistent-dimension");
                    return false;
                };
                let Ok(presentation) = DimensionPresentation::new(unit, decimal_places) else {
                    self.digest = self
                        .catalog
                        .text("assistant-error-create-persistent-dimension");
                    return false;
                };
                WorkflowIntent::CreatePersistentDimension {
                    target: PersistentDimensionId(target),
                    name: fields[0].to_owned(),
                    dimension_target: FeatureParameterTarget {
                        feature_id: FeatureId(feature_id),
                        slot,
                    },
                    presentation,
                }
            }
            AssistantIntentKind::CreateSpace => {
                let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
                let parse_point = |value: &str| {
                    let values = value
                        .split(',')
                        .map(str::trim)
                        .map(str::parse::<f64>)
                        .collect::<Result<Vec<_>, _>>()
                        .ok()?;
                    <[f64; 3]>::try_from(values).ok()
                };
                let (Some(purpose), Some(volume_min), Some(volume_max)) = (
                    fields.first().filter(|purpose| !purpose.is_empty()),
                    fields.get(1).and_then(|value| parse_point(value)),
                    fields.get(2).and_then(|value| parse_point(value)),
                ) else {
                    self.digest = self.catalog.text("assistant-error-create-space");
                    return false;
                };
                if fields.len() != 3 {
                    self.digest = self.catalog.text("assistant-error-create-space");
                    return false;
                }
                WorkflowIntent::CreateSpace {
                    target: SpaceId(target),
                    purpose: (*purpose).to_owned(),
                    volume_min,
                    volume_max,
                }
            }
            AssistantIntentKind::CreateClearanceVolume => {
                let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
                let parse_point = |value: &str| {
                    let values = value
                        .split(',')
                        .map(str::trim)
                        .map(str::parse::<f64>)
                        .collect::<Result<Vec<_>, _>>()
                        .ok()?;
                    <[f64; 3]>::try_from(values).ok()
                };
                let (
                    Some(owner),
                    Some(reason),
                    Some(volume_min),
                    Some(volume_max),
                    Some(tolerance),
                    Some(severity),
                ) = (
                    fields.first().and_then(|value| value.parse::<u64>().ok()),
                    fields.get(1).filter(|reason| !reason.is_empty()),
                    fields.get(2).and_then(|value| parse_point(value)),
                    fields.get(3).and_then(|value| parse_point(value)),
                    fields.get(4).and_then(|value| value.parse::<f64>().ok()),
                    fields.get(5).and_then(|value| match *value {
                        "advisory" => Some(ClearanceSeverity::Advisory),
                        "required" => Some(ClearanceSeverity::Required),
                        _ => None,
                    }),
                )
                else {
                    self.digest = self.catalog.text("assistant-error-create-clearance-volume");
                    return false;
                };
                if fields.len() != 6 {
                    self.digest = self.catalog.text("assistant-error-create-clearance-volume");
                    return false;
                }
                WorkflowIntent::CreateClearanceVolume {
                    target: ClearanceVolumeId(target),
                    owner: SpaceId(owner),
                    reason: (*reason).to_owned(),
                    volume_min,
                    volume_max,
                    tolerance_mm: tolerance,
                    severity,
                }
            }
            AssistantIntentKind::CreateJoint => {
                let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
                let parse_participant = |value: &str| {
                    let fields = value.split(',').map(str::trim).collect::<Vec<_>>();
                    if fields.len() != 3 {
                        return None;
                    }
                    let root = NodeId(fields[0].parse::<u64>().ok()?);
                    let path =
                        SlotPath::new(vec![SlotSegment::new(root, fields[1], fields[2]).ok()?])
                            .ok()?;
                    DerivedIdentity::new(root, path).ok()
                };
                let parse_point = |value: &str| {
                    let values = value
                        .split(',')
                        .map(str::trim)
                        .map(str::parse::<f64>)
                        .collect::<Result<Vec<_>, _>>()
                        .ok()?;
                    <[f64; 3]>::try_from(values).ok()
                };
                let (Some(participant_a), Some(participant_b), Some(volume_min), Some(volume_max)) = (
                    fields.first().and_then(|value| parse_participant(value)),
                    fields.get(1).and_then(|value| parse_participant(value)),
                    fields.get(2).and_then(|value| parse_point(value)),
                    fields.get(3).and_then(|value| parse_point(value)),
                ) else {
                    self.digest = self.catalog.text("assistant-error-create-joint");
                    return false;
                };
                if fields.len() != 4 {
                    self.digest = self.catalog.text("assistant-error-create-joint");
                    return false;
                }
                WorkflowIntent::CreateJoint {
                    target: JointId(target),
                    participant_a,
                    participant_b,
                    volume_min,
                    volume_max,
                }
            }
            AssistantIntentKind::CloneProfileDefinitionAndRepoint => {
                let fields = value_text.splitn(5, ':').map(str::trim).collect::<Vec<_>>();
                let [
                    source_definition,
                    source_feature,
                    new_definition,
                    new_feature,
                    name,
                ] = fields.as_slice()
                else {
                    self.digest = self
                        .catalog
                        .text("assistant-error-clone-profile-definition");
                    return false;
                };
                let (
                    Ok(source_definition),
                    Ok(source_feature),
                    Ok(new_definition),
                    Ok(new_feature),
                ) = (
                    source_definition.parse::<u64>(),
                    source_feature.parse::<u64>(),
                    new_definition.parse::<u64>(),
                    new_feature.parse::<u64>(),
                )
                else {
                    self.digest = self
                        .catalog
                        .text("assistant-error-clone-profile-definition");
                    return false;
                };
                WorkflowIntent::CloneProfileDefinitionAndRepoint {
                    target: OccurrenceId(target),
                    source_definition: DefinitionId(source_definition),
                    source_feature: FeatureId(source_feature),
                    new_definition: DefinitionId(new_definition),
                    new_feature: FeatureId(new_feature),
                    name: (*name).to_owned(),
                }
            }
            AssistantIntentKind::ConvertEmptyGroupToComponent => {
                let fields = value_text.splitn(3, ':').map(str::trim).collect::<Vec<_>>();
                let [new_definition, new_occurrence, name] = fields.as_slice() else {
                    self.digest = self.catalog.text("assistant-error-convert-empty-group");
                    return false;
                };
                let (Ok(new_definition), Ok(new_occurrence)) =
                    (new_definition.parse::<u64>(), new_occurrence.parse::<u64>())
                else {
                    self.digest = self.catalog.text("assistant-error-convert-empty-group");
                    return false;
                };
                WorkflowIntent::ConvertEmptyGroupToComponent {
                    target: GroupId(target),
                    new_definition: DefinitionId(new_definition),
                    new_occurrence: OccurrenceId(new_occurrence),
                    name: (*name).to_owned(),
                }
            }
            AssistantIntentKind::DeleteFeatureParameterBinding => {
                let slot = match value_text.trim() {
                    "height" => FeatureParameterSlot::Height,
                    "body_radius" => FeatureParameterSlot::BodyRadius,
                    "body_height" => FeatureParameterSlot::BodyHeight,
                    "shoulder_rise" => FeatureParameterSlot::ShoulderRise,
                    "thickness" => FeatureParameterSlot::Thickness,
                    "amount" => FeatureParameterSlot::Amount,
                    "profile_width" => FeatureParameterSlot::ProfileWidth,
                    "profile_height" => FeatureParameterSlot::ProfileHeight,
                    _ => {
                        self.digest = self
                            .catalog
                            .text("assistant-error-delete-feature-parameter-binding");
                        return false;
                    }
                };
                WorkflowIntent::DeleteFeatureParameterBinding {
                    target: FeatureParameterTarget {
                        feature_id: FeatureId(target),
                        slot,
                    },
                }
            }
            AssistantIntentKind::DeleteJoint => WorkflowIntent::DeleteJoint {
                target: JointId(target),
            },
            AssistantIntentKind::DeleteSpace => WorkflowIntent::DeleteSpace {
                target: SpaceId(target),
            },
            AssistantIntentKind::DeleteClearanceVolume => WorkflowIntent::DeleteClearanceVolume {
                target: ClearanceVolumeId(target),
            },
            AssistantIntentKind::DeletePersistentDimension => {
                WorkflowIntent::DeletePersistentDimension {
                    target: PersistentDimensionId(target),
                }
            }
            AssistantIntentKind::RecomputeFeatureParameter => {
                let slot = match value_text.trim() {
                    "height" => FeatureParameterSlot::Height,
                    "body_radius" => FeatureParameterSlot::BodyRadius,
                    "body_height" => FeatureParameterSlot::BodyHeight,
                    "shoulder_rise" => FeatureParameterSlot::ShoulderRise,
                    "thickness" => FeatureParameterSlot::Thickness,
                    "amount" => FeatureParameterSlot::Amount,
                    "profile_width" => FeatureParameterSlot::ProfileWidth,
                    "profile_height" => FeatureParameterSlot::ProfileHeight,
                    _ => {
                        self.digest = self
                            .catalog
                            .text("assistant-error-recompute-feature-parameter");
                        return false;
                    }
                };
                WorkflowIntent::RecomputeFeatureParameter {
                    target: FeatureParameterTarget {
                        feature_id: FeatureId(target),
                        slot,
                    },
                }
            }
            AssistantIntentKind::RuleDimension => WorkflowIntent::SetRuleDimension {
                target: NodeId(target),
                value_text,
            },
            AssistantIntentKind::EvaluatorName => WorkflowIntent::RenameEvaluatorNode {
                target: NodeId(target),
                name: value_text,
            },
            AssistantIntentKind::EvaluatorExpression => WorkflowIntent::SetEvaluatorExpression {
                target: NodeId(target),
                expression: value_text,
            },
            AssistantIntentKind::RuleOutputs => {
                let outputs = if value_text.trim().eq_ignore_ascii_case("none") {
                    Vec::new()
                } else {
                    let mut outputs = Vec::new();
                    for output in value_text.split(';') {
                        let Some((port, key)) = output.split_once(':') else {
                            self.digest = self.catalog.text("assistant-error-rule-outputs");
                            return false;
                        };
                        let Ok(segment) = SlotSegment::new(NodeId(target), port.trim(), key.trim())
                        else {
                            self.digest = self.catalog.text("assistant-error-rule-outputs");
                            return false;
                        };
                        let Ok(output) = RuleOutput::new(segment, Vec::new()) else {
                            self.digest = self.catalog.text("assistant-error-rule-outputs");
                            return false;
                        };
                        outputs.push(output);
                    }
                    outputs
                };
                WorkflowIntent::SetRuleOutputs {
                    target: NodeId(target),
                    outputs,
                }
            }
            AssistantIntentKind::FeatureDimension => WorkflowIntent::SetFeatureDimension {
                target: FeatureId(target),
                value_text,
            },
            AssistantIntentKind::BottleControlDimension => {
                let Some((control, value_text)) = value_text.split_once('=') else {
                    self.digest = self.catalog.text("assistant-error-bottle-control");
                    return false;
                };
                let control = match control.trim().to_ascii_lowercase().as_str() {
                    "body_radius" => BottleControlDimension::BodyRadius,
                    "body_height" => BottleControlDimension::BodyHeight,
                    "shoulder_rise" => BottleControlDimension::ShoulderRise,
                    _ => {
                        self.digest = self.catalog.text("assistant-error-bottle-control");
                        return false;
                    }
                };
                WorkflowIntent::SetBottleControlDimension {
                    target: FeatureId(target),
                    control,
                    value_text: value_text.trim().to_owned(),
                }
            }
            AssistantIntentKind::BottleEdgeFinishKind => {
                let kind = match value_text.trim().to_ascii_lowercase().as_str() {
                    "fillet" => BottleEdgeFinishKind::Fillet,
                    "chamfer" => BottleEdgeFinishKind::Chamfer,
                    _ => {
                        self.digest = self.catalog.text("assistant-error-bottle-finish-kind");
                        return false;
                    }
                };
                WorkflowIntent::SetBottleEdgeFinishKind {
                    target: FeatureId(target),
                    kind,
                }
            }
            AssistantIntentKind::ProfilePoints => {
                let mut points_mm = Vec::new();
                for point in value_text.split(';') {
                    let coordinates = point.split(',').map(str::trim).collect::<Vec<_>>();
                    if coordinates.len() != 2 {
                        self.digest = self.catalog.text("assistant-error-profile-points");
                        return false;
                    }
                    let (Ok(x_mm), Ok(y_mm)) =
                        (coordinates[0].parse::<f64>(), coordinates[1].parse::<f64>())
                    else {
                        self.digest = self.catalog.text("assistant-error-profile-points");
                        return false;
                    };
                    points_mm.push([x_mm, y_mm]);
                }
                WorkflowIntent::SetProfilePoints {
                    target: FeatureId(target),
                    points_mm,
                }
            }
            AssistantIntentKind::DefinitionName => WorkflowIntent::RenameDefinition {
                target: DefinitionId(target),
                name: value_text,
            },
            AssistantIntentKind::OccurrenceVisibility => {
                let Ok(visible) = value_text.trim().parse::<bool>() else {
                    self.digest = self.catalog.text("assistant-error-boolean");
                    return false;
                };
                WorkflowIntent::SetOccurrenceVisibility {
                    target: OccurrenceId(target),
                    visible,
                }
            }
            AssistantIntentKind::TagVisibility => {
                let Ok(visible) = value_text.trim().parse::<bool>() else {
                    self.digest = self.catalog.text("assistant-error-boolean");
                    return false;
                };
                WorkflowIntent::SetTagVisibility {
                    target: TagId(target),
                    visible,
                }
            }
            AssistantIntentKind::OccurrenceTag => {
                let tag = if value_text.trim().eq_ignore_ascii_case("none") {
                    None
                } else {
                    let Ok(tag) = value_text.trim().parse::<u64>() else {
                        self.digest = self.catalog.text("assistant-error-tag");
                        return false;
                    };
                    Some(TagId(tag))
                };
                WorkflowIntent::SetOccurrenceTag {
                    target: OccurrenceId(target),
                    tag,
                }
            }
            AssistantIntentKind::OccurrenceDefinition => {
                let Ok(definition) = value_text.trim().parse::<u64>() else {
                    self.digest = self.catalog.text("assistant-error-definition");
                    return false;
                };
                WorkflowIntent::RepointOccurrence {
                    target: OccurrenceId(target),
                    definition: DefinitionId(definition),
                }
            }
            AssistantIntentKind::OccurrenceParent => {
                let parent = if value_text.trim().eq_ignore_ascii_case("none") {
                    None
                } else {
                    let Ok(parent) = value_text.trim().parse::<u64>() else {
                        self.digest = self.catalog.text("assistant-error-group");
                        return false;
                    };
                    Some(GroupId(parent))
                };
                WorkflowIntent::SetOccurrenceParent {
                    target: OccurrenceId(target),
                    parent,
                }
            }
            AssistantIntentKind::OccurrenceTranslation => {
                let values = value_text.split(',').map(str::trim).collect::<Vec<_>>();
                if values.len() != 3 {
                    self.digest = self.catalog.text("assistant-error-translation");
                    return false;
                }
                WorkflowIntent::SetOccurrenceTranslation {
                    target: OccurrenceId(target),
                    x_mm_text: values[0].to_owned(),
                    y_mm_text: values[1].to_owned(),
                    z_mm_text: values[2].to_owned(),
                }
            }
            AssistantIntentKind::CreateTag => {
                let Some((visible, name)) = value_text.split_once(':') else {
                    self.digest = self.catalog.text("assistant-error-create-tag");
                    return false;
                };
                let Ok(visible) = visible.trim().parse::<bool>() else {
                    self.digest = self.catalog.text("assistant-error-create-tag");
                    return false;
                };
                WorkflowIntent::CreateTag {
                    target: TagId(target),
                    name: name.trim().to_owned(),
                    visible,
                }
            }
            AssistantIntentKind::DeleteTag => WorkflowIntent::DeleteTag {
                target: TagId(target),
            },
            AssistantIntentKind::CreateCollection => WorkflowIntent::CreateCollection {
                target: CollectionId(target),
                name: value_text,
            },
            AssistantIntentKind::DeleteCollection => WorkflowIntent::DeleteCollection {
                target: CollectionId(target),
            },
            AssistantIntentKind::DeleteGroup => WorkflowIntent::DeleteGroup {
                target: GroupId(target),
            },
            AssistantIntentKind::DeleteOccurrence => WorkflowIntent::DeleteOccurrence {
                target: OccurrenceId(target),
            },
            AssistantIntentKind::CreateDefinition => WorkflowIntent::CreateDefinition {
                target: DefinitionId(target),
                name: value_text,
            },
            AssistantIntentKind::DeleteDefinition => WorkflowIntent::DeleteDefinition {
                target: DefinitionId(target),
            },
            AssistantIntentKind::CreateProfileFeature => {
                let fields = value_text.splitn(3, ':').map(str::trim).collect::<Vec<_>>();
                if fields.len() != 3 {
                    self.digest = self.catalog.text("assistant-error-create-profile-feature");
                    return false;
                }
                let Ok(definition) = fields[0].parse::<u64>() else {
                    self.digest = self.catalog.text("assistant-error-create-profile-feature");
                    return false;
                };
                let mut points_mm = Vec::new();
                for point in fields[2].split(';') {
                    let coordinates = point.split(',').map(str::trim).collect::<Vec<_>>();
                    if coordinates.len() != 2 {
                        self.digest = self.catalog.text("assistant-error-create-profile-feature");
                        return false;
                    }
                    let (Ok(x_mm), Ok(y_mm)) =
                        (coordinates[0].parse::<f64>(), coordinates[1].parse::<f64>())
                    else {
                        self.digest = self.catalog.text("assistant-error-create-profile-feature");
                        return false;
                    };
                    points_mm.push([x_mm, y_mm]);
                }
                WorkflowIntent::CreateProfileFeature {
                    target: FeatureId(target),
                    definition: DefinitionId(definition),
                    name: fields[1].to_owned(),
                    points_mm,
                }
            }
            AssistantIntentKind::DeleteProfileFeature => WorkflowIntent::DeleteProfileFeature {
                target: FeatureId(target),
            },
            AssistantIntentKind::CreateGroup => WorkflowIntent::CreateGroup {
                target: GroupId(target),
                name: value_text,
            },
            AssistantIntentKind::CreateOccurrence => {
                let Some((definition, name)) = value_text.split_once(':') else {
                    self.digest = self.catalog.text("assistant-error-create-occurrence");
                    return false;
                };
                let Ok(definition) = definition.trim().parse::<u64>() else {
                    self.digest = self.catalog.text("assistant-error-create-occurrence");
                    return false;
                };
                WorkflowIntent::CreateOccurrence {
                    target: OccurrenceId(target),
                    definition: DefinitionId(definition),
                    name: name.trim().to_owned(),
                }
            }
            AssistantIntentKind::CollectionOccurrences => {
                let occurrence_ids = if value_text.trim().eq_ignore_ascii_case("none") {
                    Vec::new()
                } else {
                    let parsed = value_text
                        .split(',')
                        .map(|value| value.trim().parse::<u64>().map(OccurrenceId))
                        .collect::<Result<Vec<_>, _>>();
                    let Ok(occurrence_ids) = parsed else {
                        self.digest = self.catalog.text("assistant-error-collection-occurrences");
                        return false;
                    };
                    if occurrence_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
                        self.digest = self.catalog.text("assistant-error-collection-occurrences");
                        return false;
                    }
                    occurrence_ids
                };
                WorkflowIntent::SetCollectionOccurrences {
                    target: CollectionId(target),
                    occurrence_ids,
                }
            }
            AssistantIntentKind::GroupParent => {
                let parent = if value_text.trim().eq_ignore_ascii_case("none") {
                    None
                } else {
                    let Ok(parent) = value_text.trim().parse::<u64>() else {
                        self.digest = self.catalog.text("assistant-error-group-parent");
                        return false;
                    };
                    Some(GroupId(parent))
                };
                WorkflowIntent::SetGroupParent {
                    target: GroupId(target),
                    parent,
                }
            }
            AssistantIntentKind::GroupTranslation => {
                let values = value_text.split(',').map(str::trim).collect::<Vec<_>>();
                if values.len() != 3 {
                    self.digest = self.catalog.text("assistant-error-group-translation");
                    return false;
                }
                WorkflowIntent::SetGroupTranslation {
                    target: GroupId(target),
                    x_mm_text: values[0].to_owned(),
                    y_mm_text: values[1].to_owned(),
                    z_mm_text: values[2].to_owned(),
                }
            }
        };
        self.prepare_assistant_intent(intent)
    }

    fn show_assistant(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        ui.horizontal(|ui| {
            section_header(ui, palette, &self.catalog.text("assistant-title"));
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button(self.catalog.text("assistant-new-chat")).clicked() {
                self.new_assistant_chat();
            }
            let mode_label = match self.assistant_workspace_mode {
                AssistantWorkspaceMode::Dock => self.catalog.text("assistant-open-tab"),
                AssistantWorkspaceMode::Tab => self.catalog.text("assistant-dock-right"),
            };
            if ui.button(mode_label).clicked() {
                self.assistant_workspace_mode = match self.assistant_workspace_mode {
                    AssistantWorkspaceMode::Dock => AssistantWorkspaceMode::Tab,
                    AssistantWorkspaceMode::Tab => AssistantWorkspaceMode::Dock,
                };
            }
        });
        let conversation_document = self
            .document_path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(
                || self.catalog.text("document-untitled"),
                |name| name.to_string_lossy().into_owned(),
            );
        ui.label(
            egui::RichText::new(self.catalog.format(
                "assistant-conversation-document",
                &BTreeMap::from([("document", conversation_document)]),
            ))
            .strong()
            .color(palette.text),
        );
        ui.label(
            egui::RichText::new(self.assistant_selection_summary())
                .small()
                .color(palette.accent),
        );
        ui.label(
            egui::RichText::new(self.catalog.text("assistant-boundary"))
                .small()
                .color(palette.dim),
        );
        let previous_provider = self.assistant_provider;
        ui.label(
            egui::RichText::new(self.catalog.text("assistant-provider"))
                .small()
                .color(palette.dim),
        );
        egui::ComboBox::from_id_salt("assistant-provider")
            .width(ui.available_width())
            .selected_text(self.catalog.text(self.assistant_provider.label_key()))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.assistant_provider,
                    AssistantProvider::AnthropicApi,
                    self.catalog.text("assistant-provider-anthropic-api"),
                );
                ui.selectable_value(
                    &mut self.assistant_provider,
                    AssistantProvider::OpenAiApi,
                    self.catalog.text("assistant-provider-openai-api"),
                );
                #[cfg(feature = "private-oauth")]
                ui.selectable_value(
                    &mut self.assistant_provider,
                    AssistantProvider::ClaudeCodeOauth,
                    self.catalog.text("assistant-provider-claude-oauth"),
                );
                #[cfg(feature = "private-oauth")]
                ui.selectable_value(
                    &mut self.assistant_provider,
                    AssistantProvider::CodexOauth,
                    self.catalog.text("assistant-provider-codex-oauth"),
                );
            });
        if self.assistant_provider != previous_provider {
            self.assistant_model = self.assistant_provider.default_model().to_owned();
        }
        let models = self.assistant_models();
        ui.label(
            egui::RichText::new(self.catalog.text("assistant-model"))
                .small()
                .color(palette.dim),
        );
        egui::ComboBox::from_id_salt("assistant-model")
            .width(ui.available_width())
            .selected_text(&self.assistant_model)
            .show_ui(ui, |ui| {
                for model in models {
                    ui.selectable_value(&mut self.assistant_model, model.clone(), model);
                }
            });
        let messages_height = if self.assistant_workspace_mode == AssistantWorkspaceMode::Tab {
            (ui.available_height() - 210.0).max(260.0)
        } else {
            (ui.available_height() - 330.0).clamp(220.0, 420.0)
        };
        egui::Frame::new()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0_f32, palette.line))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .max_height(messages_height)
                    .show(ui, |ui| {
                        if self.assistant_messages.is_empty() {
                            ui.weak(self.catalog.text("assistant-empty-chat"));
                        }
                        for message in &self.assistant_messages {
                            let (heading, fill, stroke) = match message.role {
                                AssistantMessageRole::User => (
                                    self.catalog.text("assistant-role-you"),
                                    palette.accent_wash(if palette.dark { 42 } else { 28 }),
                                    palette.accent,
                                ),
                                AssistantMessageRole::Assistant => (
                                    self.catalog.text("assistant-role-assistant"),
                                    palette.panel2,
                                    palette.line,
                                ),
                                AssistantMessageRole::Error => (
                                    self.catalog.text("assistant-role-error"),
                                    Color32::from_rgba_unmultiplied(180, 44, 44, 52),
                                    Color32::from_rgb(210, 72, 72),
                                ),
                            };
                            egui::Frame::new()
                                .fill(fill)
                                .stroke(Stroke::new(1.0_f32, stroke))
                                .corner_radius(egui::CornerRadius::same(6))
                                .inner_margin(egui::Margin::same(8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal_wrapped(|ui| {
                                        ui.strong(heading);
                                        ui.label(
                                            egui::RichText::new(&message.source)
                                                .small()
                                                .color(palette.dim),
                                        );
                                    });
                                    ui.horizontal_wrapped(|ui| {
                                        ui.add(
                                            egui::Label::new(&message.text).wrap().selectable(true),
                                        );
                                        let copy_label =
                                            self.catalog.text("assistant-copy-message");
                                        let copy = ui.add(
                                            egui::Button::new("")
                                                .min_size(Vec2::splat(24.0))
                                                .frame(false),
                                        );
                                        if copy.hovered() {
                                            ui.painter().rect_filled(
                                                copy.rect,
                                                egui::CornerRadius::same(5),
                                                palette.panel2,
                                            );
                                        }
                                        theme::paint_icon(
                                            ui.painter(),
                                            shrink_to_icon(copy.rect, 13.0),
                                            Icon::Copy,
                                            if copy.hovered() {
                                                palette.text
                                            } else {
                                                palette.dim
                                            },
                                            palette.accent,
                                            1.6,
                                        );
                                        name_widget(&copy, true, &copy_label);
                                        if copy.on_hover_text(copy_label).clicked() {
                                            ui.ctx().copy_text(message.text.clone());
                                        }
                                    });
                                });
                            ui.add_space(8.0);
                        }
                        if self.assistant_pending_execution.is_some() {
                            ui.weak(self.catalog.text("assistant-progress-executing"));
                        } else if self.assistant_chat_task.is_some() {
                            ui.weak(self.catalog.text("assistant-progress-requesting"));
                        }
                    });
            });
        if let Some(proposal) = self.assistant_proposal.clone() {
            let mut confirm_clicked = false;
            let mut cancel_clicked = false;
            egui::Frame::new()
                .fill(palette.panel2)
                .stroke(Stroke::new(1.0_f32, palette.accent))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.strong(self.catalog.text("assistant-review-title"));
                    ui.small(self.catalog.format(
                        "assistant-review-meta",
                        &BTreeMap::from([
                            ("revision", proposal.provenance_revision().to_string()),
                            (
                                "reads",
                                proposal.authoritative_dependencies().len().to_string(),
                            ),
                            ("writes", proposal.authoritative_writes().len().to_string()),
                            ("commands", proposal.cost().commands.to_string()),
                            ("assumptions", proposal.assumptions().len().to_string()),
                        ]),
                    ));
                    ui.small(self.catalog.text("assistant-risk-standard"));
                    ui.weak(self.catalog.text("assistant-review-observational"));
                    egui::ScrollArea::vertical()
                        .id_salt("assistant-proposal-diff")
                        .max_height(140.0)
                        .show(ui, |ui| {
                            for entry in proposal.authoritative_diff() {
                                ui.monospace(self.assistant_proposal_target_label(&entry.target));
                                ui.monospace(self.catalog.format(
                                    "assistant-diff-row",
                                    &BTreeMap::from([
                                        (
                                            "before",
                                            self.assistant_proposal_value_label(&entry.before),
                                        ),
                                        (
                                            "after",
                                            self.assistant_proposal_value_label(&entry.after),
                                        ),
                                    ]),
                                ));
                            }
                        });
                    ui.horizontal(|ui| {
                        confirm_clicked =
                            ui.button(self.catalog.text("assistant-confirm")).clicked();
                        cancel_clicked = ui.button(self.catalog.text("assistant-cancel")).clicked();
                    });
                });
            if confirm_clicked {
                self.confirm_assistant_proposal();
            } else if cancel_clicked {
                self.cancel_assistant_proposal();
            }
        }
        if let Some(verification) = self.assistant_verification.clone() {
            let can_undo = self.assistant_change_can_undo();
            let mut undo_clicked = false;
            egui::Frame::new()
                .fill(palette.accent_wash(if palette.dark { 32 } else { 20 }))
                .stroke(Stroke::new(1.0_f32, palette.accent))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(self.catalog.text("assistant-result-title"));
                        ui.small(self.catalog.format(
                            "assistant-verification",
                            &BTreeMap::from([
                                ("revision", verification.revision_id.to_string()),
                                ("writes", verification.verified_write_count.to_string()),
                            ]),
                        ));
                    });
                    let undo_label = self.catalog.text("assistant-undo-change");
                    undo_clicked = ui
                        .add_enabled(can_undo, egui::Button::new(&undo_label))
                        .on_hover_text(self.catalog.text("shortcut-undo"))
                        .clicked();
                });
            if undo_clicked {
                self.dispatch_command(AppCommand::Undo);
            }
        }
        let enter_without_shift =
            ui.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift);
        let input_label = self.catalog.text("assistant-input-hint");
        let input = ui.add(
            egui::TextEdit::multiline(&mut self.assistant_input)
                .id_salt("assistant-chat-input")
                .hint_text(&input_label)
                .desired_width(f32::INFINITY)
                .desired_rows(
                    if self.assistant_workspace_mode == AssistantWorkspaceMode::Tab {
                        4
                    } else {
                        3
                    },
                ),
        );
        let send_shortcut = input.has_focus() && enter_without_shift;
        input.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &input_label)
        });
        input.on_hover_text(self.catalog.text("assistant-send-shortcut"));

        let enabled = self.assistant_chat_task.is_none()
            && self.assistant_pending_execution.is_none()
            && !self.assistant_input.trim().is_empty();
        let send_clicked = ui
            .allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 28.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let send_label = self.catalog.text("assistant-send");
                    let send = ui.add_enabled(
                        enabled,
                        egui::Button::new("")
                            .min_size(Vec2::splat(28.0))
                            .fill(if enabled {
                                palette.accent
                            } else {
                                palette.panel2
                            })
                            .stroke(Stroke::NONE)
                            .corner_radius(egui::CornerRadius::same(8)),
                    );
                    theme::paint_icon(
                        ui.painter(),
                        shrink_to_icon(send.rect, 14.0),
                        Icon::Send,
                        if enabled {
                            palette.accent_ink
                        } else {
                            palette.faint
                        },
                        if enabled {
                            palette.accent_ink
                        } else {
                            palette.faint
                        },
                        1.8,
                    );
                    name_widget(&send, enabled, &send_label);
                    send.on_hover_text(self.catalog.text("assistant-send-shortcut"))
                        .clicked()
                },
            )
            .inner;
        if send_clicked || send_shortcut {
            self.send_assistant_message(ui.ctx());
        }

        egui::CollapsingHeader::new(self.catalog.text("assistant-advanced-tools"))
            .default_open(false)
            .show(ui, |ui| {
                egui::ComboBox::from_label(self.catalog.text("assistant-intent"))
                    .selected_text(self.catalog.text(match self.assistant_intent_kind {
                        AssistantIntentKind::CreateEvaluatorInput => {
                            "assistant-intent-create-evaluator-input"
                        }
                        AssistantIntentKind::CreateEvaluatorExpression => {
                            "assistant-intent-create-evaluator-expression"
                        }
                        AssistantIntentKind::CreateEvaluatorRule => {
                            "assistant-intent-create-evaluator-rule"
                        }
                        AssistantIntentKind::CreateRuleOverride => {
                            "assistant-intent-create-rule-override"
                        }
                        AssistantIntentKind::DeleteRuleOverride => {
                            "assistant-intent-delete-rule-override"
                        }
                        AssistantIntentKind::CreateFeatureParameterBinding => {
                            "assistant-intent-create-feature-parameter-binding"
                        }
                        AssistantIntentKind::DeleteFeatureParameterBinding => {
                            "assistant-intent-delete-feature-parameter-binding"
                        }
                        AssistantIntentKind::CreatePersistentDimension => {
                            "assistant-intent-create-persistent-dimension"
                        }
                        AssistantIntentKind::CreateSpace => "assistant-intent-create-space",
                        AssistantIntentKind::CreateClearanceVolume => {
                            "assistant-intent-create-clearance-volume"
                        }
                        AssistantIntentKind::CreateJoint => "assistant-intent-create-joint",
                        AssistantIntentKind::CloneProfileDefinitionAndRepoint => {
                            "assistant-intent-clone-profile-definition"
                        }
                        AssistantIntentKind::ConvertEmptyGroupToComponent => {
                            "assistant-intent-convert-empty-group"
                        }
                        AssistantIntentKind::RecomputeFeatureParameter => {
                            "assistant-intent-recompute-feature-parameter"
                        }
                        AssistantIntentKind::DeleteJoint => "assistant-intent-delete-joint",
                        AssistantIntentKind::DeleteSpace => "assistant-intent-delete-space",
                        AssistantIntentKind::DeleteClearanceVolume => {
                            "assistant-intent-delete-clearance-volume"
                        }
                        AssistantIntentKind::DeletePersistentDimension => {
                            "assistant-intent-delete-persistent-dimension"
                        }
                        AssistantIntentKind::RuleDimension => "assistant-intent-rule",
                        AssistantIntentKind::EvaluatorName => "assistant-intent-evaluator-name",
                        AssistantIntentKind::EvaluatorExpression => {
                            "assistant-intent-evaluator-expression"
                        }
                        AssistantIntentKind::RuleOutputs => "assistant-intent-rule-outputs",
                        AssistantIntentKind::FeatureDimension => "assistant-intent-feature",
                        AssistantIntentKind::BottleControlDimension => {
                            "assistant-intent-bottle-control-dimension"
                        }
                        AssistantIntentKind::BottleEdgeFinishKind => {
                            "assistant-intent-bottle-finish-kind"
                        }
                        AssistantIntentKind::ProfilePoints => "assistant-intent-profile-points",
                        AssistantIntentKind::DefinitionName => "assistant-intent-definition-name",
                        AssistantIntentKind::OccurrenceVisibility => {
                            "assistant-intent-occurrence-visibility"
                        }
                        AssistantIntentKind::TagVisibility => "assistant-intent-tag-visibility",
                        AssistantIntentKind::OccurrenceTag => "assistant-intent-occurrence-tag",
                        AssistantIntentKind::OccurrenceDefinition => {
                            "assistant-intent-occurrence-definition"
                        }
                        AssistantIntentKind::OccurrenceParent => {
                            "assistant-intent-occurrence-parent"
                        }
                        AssistantIntentKind::OccurrenceTranslation => {
                            "assistant-intent-occurrence-translation"
                        }
                        AssistantIntentKind::GroupTranslation => {
                            "assistant-intent-group-translation"
                        }
                        AssistantIntentKind::GroupParent => "assistant-intent-group-parent",
                        AssistantIntentKind::CollectionOccurrences => {
                            "assistant-intent-collection-occurrences"
                        }
                        AssistantIntentKind::CreateTag => "assistant-intent-create-tag",
                        AssistantIntentKind::DeleteTag => "assistant-intent-delete-tag",
                        AssistantIntentKind::CreateCollection => {
                            "assistant-intent-create-collection"
                        }
                        AssistantIntentKind::DeleteCollection => {
                            "assistant-intent-delete-collection"
                        }
                        AssistantIntentKind::DeleteGroup => "assistant-intent-delete-group",
                        AssistantIntentKind::DeleteOccurrence => {
                            "assistant-intent-delete-occurrence"
                        }
                        AssistantIntentKind::CreateDefinition => {
                            "assistant-intent-create-definition"
                        }
                        AssistantIntentKind::DeleteDefinition => {
                            "assistant-intent-delete-definition"
                        }
                        AssistantIntentKind::CreateProfileFeature => {
                            "assistant-intent-create-profile-feature"
                        }
                        AssistantIntentKind::DeleteProfileFeature => {
                            "assistant-intent-delete-profile-feature"
                        }
                        AssistantIntentKind::CreateGroup => "assistant-intent-create-group",
                        AssistantIntentKind::CreateOccurrence => {
                            "assistant-intent-create-occurrence"
                        }
                    }))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateEvaluatorInput,
                            self.catalog.text("assistant-intent-create-evaluator-input"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateEvaluatorExpression,
                            self.catalog
                                .text("assistant-intent-create-evaluator-expression"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateEvaluatorRule,
                            self.catalog.text("assistant-intent-create-evaluator-rule"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateRuleOverride,
                            self.catalog.text("assistant-intent-create-rule-override"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteRuleOverride,
                            self.catalog.text("assistant-intent-delete-rule-override"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateFeatureParameterBinding,
                            self.catalog
                                .text("assistant-intent-create-feature-parameter-binding"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteFeatureParameterBinding,
                            self.catalog
                                .text("assistant-intent-delete-feature-parameter-binding"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreatePersistentDimension,
                            self.catalog
                                .text("assistant-intent-create-persistent-dimension"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateSpace,
                            self.catalog.text("assistant-intent-create-space"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateClearanceVolume,
                            self.catalog
                                .text("assistant-intent-create-clearance-volume"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateJoint,
                            self.catalog.text("assistant-intent-create-joint"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CloneProfileDefinitionAndRepoint,
                            self.catalog
                                .text("assistant-intent-clone-profile-definition"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::ConvertEmptyGroupToComponent,
                            self.catalog.text("assistant-intent-convert-empty-group"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::RecomputeFeatureParameter,
                            self.catalog
                                .text("assistant-intent-recompute-feature-parameter"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteJoint,
                            self.catalog.text("assistant-intent-delete-joint"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteSpace,
                            self.catalog.text("assistant-intent-delete-space"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteClearanceVolume,
                            self.catalog
                                .text("assistant-intent-delete-clearance-volume"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeletePersistentDimension,
                            self.catalog
                                .text("assistant-intent-delete-persistent-dimension"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::RuleDimension,
                            self.catalog.text("assistant-intent-rule"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::EvaluatorName,
                            self.catalog.text("assistant-intent-evaluator-name"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::EvaluatorExpression,
                            self.catalog.text("assistant-intent-evaluator-expression"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::RuleOutputs,
                            self.catalog.text("assistant-intent-rule-outputs"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::FeatureDimension,
                            self.catalog.text("assistant-intent-feature"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::BottleControlDimension,
                            self.catalog
                                .text("assistant-intent-bottle-control-dimension"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::BottleEdgeFinishKind,
                            self.catalog.text("assistant-intent-bottle-finish-kind"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::ProfilePoints,
                            self.catalog.text("assistant-intent-profile-points"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DefinitionName,
                            self.catalog.text("assistant-intent-definition-name"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::OccurrenceVisibility,
                            self.catalog.text("assistant-intent-occurrence-visibility"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::TagVisibility,
                            self.catalog.text("assistant-intent-tag-visibility"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::OccurrenceTranslation,
                            self.catalog.text("assistant-intent-occurrence-translation"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::OccurrenceTag,
                            self.catalog.text("assistant-intent-occurrence-tag"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::OccurrenceDefinition,
                            self.catalog.text("assistant-intent-occurrence-definition"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::OccurrenceParent,
                            self.catalog.text("assistant-intent-occurrence-parent"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::GroupTranslation,
                            self.catalog.text("assistant-intent-group-translation"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::GroupParent,
                            self.catalog.text("assistant-intent-group-parent"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CollectionOccurrences,
                            self.catalog.text("assistant-intent-collection-occurrences"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateTag,
                            self.catalog.text("assistant-intent-create-tag"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteTag,
                            self.catalog.text("assistant-intent-delete-tag"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateCollection,
                            self.catalog.text("assistant-intent-create-collection"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteCollection,
                            self.catalog.text("assistant-intent-delete-collection"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteGroup,
                            self.catalog.text("assistant-intent-delete-group"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteOccurrence,
                            self.catalog.text("assistant-intent-delete-occurrence"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateDefinition,
                            self.catalog.text("assistant-intent-create-definition"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteDefinition,
                            self.catalog.text("assistant-intent-delete-definition"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateProfileFeature,
                            self.catalog.text("assistant-intent-create-profile-feature"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::DeleteProfileFeature,
                            self.catalog.text("assistant-intent-delete-profile-feature"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateGroup,
                            self.catalog.text("assistant-intent-create-group"),
                        );
                        ui.selectable_value(
                            &mut self.assistant_intent_kind,
                            AssistantIntentKind::CreateOccurrence,
                            self.catalog.text("assistant-intent-create-occurrence"),
                        );
                    });
                egui::Grid::new("assistant-intent-inputs").show(ui, |ui| {
                    ui.label(self.catalog.text("assistant-target"));
                    ui.text_edit_singleline(&mut self.assistant_target_input);
                    ui.end_row();
                    ui.label(self.catalog.text("assistant-value-label"))
                        .on_hover_text(self.catalog.text("assistant-value"));
                    ui.text_edit_singleline(&mut self.assistant_value_input);
                    ui.end_row();
                });
                if ui.button(self.catalog.text("assistant-preview")).clicked()
                    && self.prepare_assistant_from_inputs()
                    && self
                        .assistant_proposal
                        .as_ref()
                        .is_some_and(Self::assistant_proposal_is_low_risk)
                {
                    self.confirm_assistant_proposal();
                }
            });

        ui.separator();
    }

    fn show_bottle_workflow(&mut self, ui: &mut egui::Ui) {
        section_header(ui, self.palette(), "M6 editable bottle");
        if ui.button("Create editable bottle").clicked() {
            self.create_bottle();
        }
        let Some(definition_id) = self.selected_bottle_definition() else {
            ui.small("Select an editable bottle to inspect exact authority or edit parameters.");
            ui.separator();
            return;
        };
        if self
            .bottle_editor
            .as_ref()
            .is_none_or(|editor| editor.definition_id != definition_id)
        {
            self.bottle_editor =
                Self::bottle_editor_inputs(&self.document.current(), definition_id);
        }

        ui.small("Push/Pull drag: body=scale, shoulder=flatten, neck/rim=stretch");
        let mut apply = false;
        if let Some(editor) = self.bottle_editor.as_mut() {
            egui::Grid::new("bottle-parameters").show(ui, |ui| {
                ui.label("Body radius / scale (mm)");
                ui.text_edit_singleline(&mut editor.body_radius);
                ui.end_row();
                ui.label("Body height / stretch (mm)");
                ui.text_edit_singleline(&mut editor.body_height);
                ui.end_row();
                ui.label("Shoulder rise / flatten (mm)");
                ui.text_edit_singleline(&mut editor.shoulder_rise);
                ui.end_row();
                ui.label("Shell thickness (mm)");
                ui.text_edit_singleline(&mut editor.thickness);
                ui.end_row();
                ui.label("Finish amount (mm)");
                ui.text_edit_singleline(&mut editor.finish_amount);
                ui.end_row();
            });
            egui::ComboBox::from_label("Shoulder finish")
                .selected_text(match editor.finish_kind {
                    BottleEdgeFinishKind::Fillet => "Fillet",
                    BottleEdgeFinishKind::Chamfer => "Chamfer",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut editor.finish_kind,
                        BottleEdgeFinishKind::Fillet,
                        "Fillet",
                    );
                    ui.selectable_value(
                        &mut editor.finish_kind,
                        BottleEdgeFinishKind::Chamfer,
                        "Chamfer",
                    );
                });
            apply = ui.button("Apply bottle parameters atomically").clicked();
        }
        if apply && let Some(editor) = self.bottle_editor.clone() {
            self.set_bottle_parameters(&editor);
        }

        let report = self.bottle_authority_report(definition_id);
        if let Some(report) = &report {
            ui.strong(if report.current && report.validation_passed {
                "Validation: accepted current exact result"
            } else {
                "Validation: stale or rejected — exports blocked"
            });
            ui.small(format!(
                "Canonical authority: {}\nEvaluated authority: {}\nViewport: {}\nLoss: {}\nDurable references: {}",
                report.canonical_authority,
                report.evaluated_authority,
                report.render_representation,
                report.conversion_loss,
                report.durable_reference_count
            ));
        } else {
            ui.label(
                "Exact authority: evaluating / unavailable; proxy fallback is not authoritative",
            );
        }
        let export_ready = report.is_some_and(|report| report.current && report.validation_passed);
        ui.horizontal(|ui| {
            let exact = ui
                .add_enabled(export_ready, egui::Button::new("Export STEP + loss report"))
                .clicked();
            let mesh = ui
                .add_enabled(export_ready, egui::Button::new("Export mesh + loss report"))
                .clicked();
            if exact
                && let Some(path) = self.dialogs.pick_export_path(ExportRequest {
                    filter_label: "ISO 10303 STEP exact model",
                    extension: "step",
                    suggested_name: "editable-bottle.step",
                })
            {
                self.export_bottle_step_to(definition_id, &path);
            }
            if mesh
                && let Some(path) = self.dialogs.pick_export_path(ExportRequest {
                    filter_label: "Wavefront OBJ mesh",
                    extension: "obj",
                    suggested_name: "editable-bottle.obj",
                })
            {
                self.export_bottle_mesh_to(definition_id, &path);
            }
        });
        ui.separator();
    }

    fn show_beam_m4ae(&mut self, ui: &mut egui::Ui) {
        section_header(ui, self.palette(), "Beam A / M5 exact fabrication");
        if ui.button("Load / reset Beam A").clicked() {
            self.load_beam_m4ae();
        }
        if self.beam_workspace.is_some() {
            ui.horizontal(|ui| {
                ui.label("Zone 1 gap (mm)");
                ui.text_edit_singleline(&mut self.beam_zone1_gap_input);
                if ui.button("Apply Beam A change").clicked()
                    && let Ok(value) = self.beam_zone1_gap_input.trim().parse::<f64>()
                {
                    self.set_beam_zone1_gap_mm(value);
                }
            });
            if let Some(slice) = self.beam_slice() {
                ui.label(match slice.validation {
                    BeamValidationVerdict::Green => "Validation: Passed (collision / joints)",
                    BeamValidationVerdict::Error => {
                        "Validation: Failed / NotEvaluated / Unavailable"
                    }
                });
                ui.small(format!(
                    "{} · input {} · diagnostics {} · Exact {} · Tolerant {}",
                    slice.validation_report.invocation.contract_id,
                    &slice.validation_report.invocation.input_digest[..12],
                    slice.validation_report.diagnostics.len(),
                    slice.validation_report.evidence_counts.exact,
                    slice.validation_report.evidence_counts.tolerant
                ));
                egui::CollapsingHeader::new("12 groove positions")
                    .default_open(true)
                    .show(ui, |ui| {
                        for item in &slice.positions {
                            ui.label(format!(
                                "G{:02}: {}/{}/{} mm",
                                item.number,
                                format_height(item.start_mm),
                                format_height(item.end_mm),
                                format_height(item.centre_mm)
                            ));
                        }
                    });
                ui.strong("Full BOM");
                for row in &slice.full_bom.rows {
                    ui.label(format!(
                        "{} · {} · {} × {} × {} mm · qty {} · {:?}",
                        row.stable_row_id,
                        row.material_key,
                        format_height(row.dimensions.length_mm),
                        format_height(row.dimensions.width_mm),
                        format_height(row.dimensions.height_mm),
                        row.quantity,
                        row.validation_state
                    ));
                }
                ui.strong("Dimension projections");
                for chain in &slice.dimension_sheet.chains {
                    ui.label(format!(
                        "{}: {}",
                        chain.stable_chain_id,
                        chain.grouped_labels.join(", ")
                    ));
                }
            }
            let m5_ready = self.beam_m5_products.is_some();
            if let Some(products) = self.beam_m5_products.as_deref() {
                ui.strong("Worker-backed exact piece drawing");
                ui.small(format!(
                    "{} B-Rep pieces · {} durable notch-face references · {} manufacturing operations",
                    products.packages.len(),
                    products.stable_reference_count(),
                    products.manufacturing.operations.len()
                ));
                if let Some(outline) = products.drawing.outlines.first() {
                    let desired = Vec2::new(ui.available_width().max(120.0), 96.0);
                    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
                    let min_x = outline
                        .points_mm
                        .iter()
                        .map(|point| point[0])
                        .reduce(f64::min)
                        .unwrap_or(0.0);
                    let max_x = outline
                        .points_mm
                        .iter()
                        .map(|point| point[0])
                        .reduce(f64::max)
                        .unwrap_or(1.0);
                    let min_y = outline
                        .points_mm
                        .iter()
                        .map(|point| point[1])
                        .reduce(f64::min)
                        .unwrap_or(0.0);
                    let max_y = outline
                        .points_mm
                        .iter()
                        .map(|point| point[1])
                        .reduce(f64::max)
                        .unwrap_or(1.0);
                    let scale_x = f64::from(rect.width()) / (max_x - min_x).max(1.0);
                    let scale_y = f64::from(rect.height()) / (max_y - min_y).max(1.0);
                    let points = outline
                        .points_mm
                        .iter()
                        .map(|point| {
                            Pos2::new(
                                rect.left() + ((point[0] - min_x) * scale_x) as f32,
                                rect.bottom() - ((point[1] - min_y) * scale_y) as f32,
                            )
                        })
                        .collect::<Vec<_>>();
                    ui.painter().add(egui::Shape::closed_line(
                        points,
                        Stroke::new(1.5_f32, Color32::from_rgb(240, 190, 92)),
                    ));
                }
            } else {
                ui.label("Exact notch worker: evaluating / unavailable");
            }
            ui.horizontal(|ui| {
                let drawing = ui
                    .add_enabled(m5_ready, egui::Button::new("Export piece drawing (SVG)"))
                    .clicked();
                let manufacturing = ui
                    .add_enabled(
                        m5_ready,
                        egui::Button::new("Export manufacturing operations"),
                    )
                    .clicked();
                if drawing
                    && let Some(path) = self.dialogs.pick_export_path(ExportRequest {
                        filter_label: "SVG piece drawing",
                        extension: "svg",
                        suggested_name: "beam-a-piece-drawing.svg",
                    })
                {
                    self.export_beam_drawing_to(&path);
                }
                if manufacturing
                    && let Some(path) = self.dialogs.pick_export_path(ExportRequest {
                        filter_label: "Ketchup manufacturing operations",
                        extension: "kfm",
                        suggested_name: "beam-a-manufacturing.kfm",
                    })
                {
                    self.export_beam_manufacturing_to(&path);
                }
            });
        }
        ui.separator();
    }

    fn parameter_expression_nodes(&self) -> Vec<(NodeId, String, String)> {
        self.document
            .current()
            .evaluator_nodes()
            .filter_map(|node| match node.kind() {
                EvaluatorNodeKind::Expression { source, .. }
                | EvaluatorNodeKind::Rule { source, .. } => {
                    Some((node.id(), node.name().to_owned(), source.clone()))
                }
                EvaluatorNodeKind::Parameter { .. } => None,
            })
            .collect()
    }

    fn apply_parameter_expression(&mut self) -> bool {
        let Some(node_id) = self.parameter_editor_node else {
            return false;
        };
        let snapshot = self.document.current();
        let current_provenance = (
            snapshot.document_id(),
            snapshot.revision_id(),
            snapshot.canonical_digest(),
        );
        if self.parameter_provenance.as_ref() != Some(&current_provenance) {
            self.parameter_provenance = Some(current_provenance);
            self.digest = self.catalog.text("error-parameter-stale");
            return false;
        }
        let batch = CommandBatch::new(vec![
            CanonicalCommand::SetNodeExpression {
                id: node_id,
                expression: self.parameter_expression_input.clone(),
            },
            CanonicalCommand::RecomputeFeatureParameters {
                identity: EvaluationIdentity::default(),
            },
        ]);
        let proposal = match self.document.prepare_proposal(batch) {
            Ok(proposal) => proposal,
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-parameter-expression",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                return false;
            }
        };
        if proposal.document_id() != snapshot.document_id()
            || proposal.provenance_revision() != snapshot.revision_id()
            || proposal.provenance_digest() != snapshot.canonical_digest()
        {
            self.digest = self.catalog.text("error-parameter-stale");
            return false;
        }
        match self.document.commit_verified_proposal(&proposal) {
            Ok(committed) => {
                self.parameter_last_recomputed_nodes =
                    committed.revision().recomputed_nodes().clone();
                self.parameter_canonical_source = self.parameter_expression_input.clone();
                let committed_snapshot = committed.revision().snapshot();
                self.parameter_provenance = Some((
                    committed_snapshot.document_id(),
                    committed_snapshot.revision_id(),
                    committed_snapshot.canonical_digest(),
                ));
                let value = committed
                    .revision()
                    .evaluation()
                    .and_then(|report| report.node(node_id))
                    .and_then(|node| match node.status {
                        EvaluationStatus::Evaluated(value) => Some(format_height(value)),
                        EvaluationStatus::Error(_) => None,
                    })
                    .unwrap_or_default();
                self.digest = self.catalog.format(
                    "digest-parameter-applied",
                    &BTreeMap::from([("node", node_id.0.to_string()), ("value", value)]),
                );
                self.status_key = "status-ready";
                true
            }
            Err(ProposalCommitError::Stale(_)) => {
                self.digest = self.catalog.text("error-parameter-stale");
                false
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-parameter-expression",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                false
            }
        }
    }

    fn show_parameter_editor(&mut self, ui: &mut egui::Ui) {
        let nodes = self.parameter_expression_nodes();
        if nodes.is_empty() {
            self.parameter_editor_node = None;
            self.parameter_expression_input.clear();
            self.parameter_canonical_source.clear();
            self.parameter_provenance = None;
            return;
        }
        let selected_is_current = self
            .parameter_editor_node
            .is_some_and(|selected| nodes.iter().any(|(id, _, _)| *id == selected));
        if !selected_is_current {
            self.parameter_editor_node = Some(nodes[0].0);
        }
        let mut selected = self
            .parameter_editor_node
            .expect("an editable evaluator node was selected");
        let previous_selected = selected;
        let selected_name = nodes
            .iter()
            .find(|(id, _, _)| *id == selected)
            .map(|(_, name, _)| name.clone())
            .expect("the selected evaluator node is present");

        section_header(ui, self.palette(), &self.catalog.text("parameters-title"));
        let selector_label = self.catalog.text("parameters-node");
        ui.label(&selector_label);
        let selector = egui::ComboBox::from_id_salt("parameter-expression-node")
            .width(ui.available_width())
            .selected_text(selected_name)
            .show_ui(ui, |ui| {
                for (id, name, _) in &nodes {
                    ui.selectable_value(&mut selected, *id, name);
                }
            });
        selector.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &selector_label)
        });
        self.parameter_editor_node = Some(selected);
        let canonical_source = nodes
            .iter()
            .find(|(id, _, _)| *id == selected)
            .map(|(_, _, source)| source.clone())
            .expect("the selected evaluator node is present");
        if selected != previous_selected || canonical_source != self.parameter_canonical_source {
            self.parameter_expression_input = canonical_source.clone();
            self.parameter_canonical_source = canonical_source;
            let snapshot = self.document.current();
            self.parameter_provenance = Some((
                snapshot.document_id(),
                snapshot.revision_id(),
                snapshot.canonical_digest(),
            ));
        }

        let input_label = self.catalog.text("parameters-expression");
        ui.label(&input_label);
        let input = ui.add(
            egui::TextEdit::singleline(&mut self.parameter_expression_input)
                .hint_text(self.catalog.text("parameters-expression-hint")),
        );
        input.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &input_label)
        });
        ui.small(self.catalog.text("parameters-expression-help"));

        let snapshot = self.document.current();
        if let Ok(report) = snapshot.evaluate(&EvaluationIdentity::default())
            && let Some(node) = report.node(selected)
            && let EvaluationStatus::Evaluated(value) = node.status
        {
            ui.label(self.catalog.format(
                "parameters-result",
                &BTreeMap::from([("value", format_height(value))]),
            ));
        }
        if ui.button(self.catalog.text("parameters-apply")).clicked() {
            self.apply_parameter_expression();
        }
        ui.separator();
    }

    fn show_pocket_properties(&mut self, ui: &mut egui::Ui) {
        let Some((feature_id, depth)) = self.selected_pocket() else {
            self.pocket_editor_feature = None;
            self.pocket_depth_input.clear();
            return;
        };
        if self.pocket_editor_feature != Some(feature_id) {
            self.pocket_editor_feature = Some(feature_id);
            self.pocket_depth_input = depth.source_token().to_owned();
        }
        section_header(
            ui,
            self.palette(),
            &self.catalog.text("pocket-properties-title"),
        );
        ui.horizontal(|ui| {
            ui.label(self.catalog.text("pocket-properties-depth"));
            ui.text_edit_singleline(&mut self.pocket_depth_input);
            ui.label(self.catalog.text("unit-mm"));
        });
        let apply = ui
            .button(self.catalog.text("pocket-properties-apply"))
            .clicked();
        if apply {
            if let Some(depth_mm) = parse_distance_mm(&self.pocket_depth_input) {
                if self.set_selected_pocket_depth(depth_mm) {
                    self.pocket_depth_input = format_height(depth_mm);
                }
            } else {
                self.digest = self.catalog.text("digest-pocket-invalid-depth");
            }
        }
        ui.separator();
    }

    fn show_outliner_without_assistant(&mut self, ui: &mut egui::Ui) {
        self.show_bottle_workflow(ui);
        self.show_beam_m4ae(ui);
        self.show_pocket_properties(ui);
        let groups = self.outliner_groups();
        let entries = self.outliner_query();
        section_header(ui, self.palette(), &self.catalog.text("dock-outliner"));
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - 64.0).max(120.0))
            .show(ui, |ui| {
                for group in groups {
                    let label = self.catalog.format(
                        "outliner-group",
                        &BTreeMap::from([
                            ("name", group.name),
                            ("count", group.member_count.to_string()),
                        ]),
                    );
                    let response =
                        ui.selectable_label(self.selection.selected_group == Some(group.id), label);
                    if response.double_clicked() {
                        self.enter_group_context(group.id);
                    } else if response.clicked() {
                        self.select_group(group.id);
                    }
                }
                if !entries.is_empty() {
                    ui.separator();
                }
                for mut definition in entries {
                    if definition.occurrences.len() == 1 {
                        let occurrence = definition.occurrences.remove(0);
                        let default_name = format!("{} #1", definition.name);
                        let name = if occurrence.name == default_name {
                            definition.name
                        } else {
                            occurrence.name.clone()
                        };
                        let mut arguments = BTreeMap::from([
                            ("name", name),
                            ("dimensions", definition.specification),
                            (
                                "visibility",
                                if occurrence.visible { "◉" } else { "○" }.to_owned(),
                            ),
                        ]);
                        let key = if let Some(group_id) = occurrence.parent {
                            arguments.insert("group", group_id.0.to_string());
                            "outliner-object-grouped"
                        } else {
                            "outliner-object"
                        };
                        let response = ui.selectable_label(
                            self.selection.contains(&occurrence.instance_path),
                            self.catalog.format(key, &arguments),
                        );
                        if response.double_clicked() {
                            self.enter_occurrence_context(occurrence.instance_path.clone());
                        } else if response.clicked() {
                            let additive = ui.input(|input| input.modifiers.shift);
                            self.select_from_outliner(occurrence.instance_path, additive);
                        }
                    } else {
                        let count = definition.occurrences.len();
                        let heading = self.catalog.format(
                            "outliner-component",
                            &BTreeMap::from([
                                ("name", definition.name),
                                ("count", count.to_string()),
                                ("dimensions", definition.specification),
                            ]),
                        );
                        let definition_id = definition.id;
                        let response = egui::CollapsingHeader::new(heading)
                            .id_salt(definition_id.0)
                            .show(ui, |ui| {
                                for occurrence in definition.occurrences {
                                    let mut arguments = BTreeMap::from([
                                        ("name", occurrence.name),
                                        (
                                            "visibility",
                                            if occurrence.visible { "◉" } else { "○" }.to_owned(),
                                        ),
                                    ]);
                                    let key = if let Some(group_id) = occurrence.parent {
                                        arguments.insert("group", group_id.0.to_string());
                                        "outliner-instance-grouped"
                                    } else {
                                        "outliner-instance"
                                    };
                                    let row = ui.selectable_label(
                                        self.selection.contains(&occurrence.instance_path),
                                        self.catalog.format(key, &arguments),
                                    );
                                    if row.double_clicked() {
                                        self.enter_occurrence_context(
                                            occurrence.instance_path.clone(),
                                        );
                                    } else if row.clicked() {
                                        let additive = ui.input(|input| input.modifiers.shift);
                                        self.select_from_outliner(
                                            occurrence.instance_path,
                                            additive,
                                        );
                                    }
                                }
                            });
                        if response.header_response.clicked() {
                            let additive = ui.input(|input| input.modifiers.shift);
                            self.select_definition(definition_id, additive);
                        }
                    }
                    ui.add_space(2.0);
                }
            });
        ui.separator();
        section_header(ui, self.palette(), &self.catalog.text("dock-tags"));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.command_enabled(AppCommand::MakeUnique),
                    egui::Button::new(self.catalog.text("model-make-unique")),
                )
                .clicked()
            {
                self.dispatch_command(AppCommand::MakeUnique);
            }
        });
        ui.label(self.catalog.format(
            "tags-visibility",
            &BTreeMap::from([
                ("hidden", self.hidden_occurrence_count().to_string()),
                ("total", self.active_box_count().to_string()),
            ]),
        ));
        ui.horizontal(|ui| {
            self.command_button(ui, AppCommand::Hide);
            self.command_button(ui, AppCommand::Unhide);
        });
    }

    fn show_stl_import_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.pending_stl_import.as_ref() else {
            return;
        };
        let path = pending.path.clone();
        let mut unit = pending.unit;
        let mut import = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-import-stl-title"))
            .id(egui::Id::new("stl-import-units"))
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    "dialog-import-stl-source",
                    &BTreeMap::from([("path", path.display().to_string())]),
                ));
                ui.label(self.catalog.text("dialog-import-stl-units"));
                for (candidate, key) in [
                    (ImportLengthUnit::Millimetre, "unit-millimetre"),
                    (ImportLengthUnit::Centimetre, "unit-centimetre"),
                    (ImportLengthUnit::Metre, "unit-metre"),
                    (ImportLengthUnit::Inch, "unit-inch"),
                    (ImportLengthUnit::Foot, "unit-foot"),
                ] {
                    ui.radio_value(&mut unit, candidate, self.catalog.text(key));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    import = ui
                        .button(self.catalog.text("dialog-import-stl-confirm"))
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-import-stl-cancel"))
                        .clicked();
                });
            });
        if let Some(pending) = self.pending_stl_import.as_mut() {
            pending.unit = unit;
        }
        if cancel {
            self.pending_stl_import = None;
            self.digest = self.catalog.text("digest-cancelled");
        } else if import {
            let pending = self
                .pending_stl_import
                .take()
                .expect("the STL review window has a pending import");
            self.import_stl_from(&pending);
        }
    }

    fn show_shortcuts_window(&mut self, context: &egui::Context) {
        if !self.shortcuts_open {
            return;
        }
        let mut open = true;
        egui::Window::new(self.catalog.text("shortcuts-title"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                for spec in CommandRegistry::COMMANDS
                    .iter()
                    .filter(|spec| spec.shortcut_key != "shortcut-none")
                {
                    ui.label(self.catalog.format(
                        "shortcuts-row",
                        &BTreeMap::from([
                            ("command", self.catalog.text(spec.label_key)),
                            ("shortcut", self.catalog.text(spec.shortcut_key)),
                        ]),
                    ));
                }
                ui.separator();
                if ui.button(self.catalog.text("shortcuts-close")).clicked() {
                    self.shortcuts_open = false;
                }
            });
        if !open {
            self.shortcuts_open = false;
        }
    }

    fn show_status_bar(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 3.0, palette.accent);
            ui.label(
                egui::RichText::new(self.catalog.text(self.active_tool.label_key()))
                    .strong()
                    .color(palette.text),
            );

            // Everything after the tool name is running commentary, so it is set
            // in the tertiary tone and truncated rather than allowed to push the
            // measured chips off the right edge.
            let mut context = vec![self.catalog.format(
                "status-selected",
                &BTreeMap::from([("count", self.selection_count().to_string())]),
            )];
            if let Some(edit_context) = self.selection.edit_context.last() {
                let (key, id) = match edit_context {
                    EditContext::Group(id) => ("status-editing-group", id.0),
                    EditContext::Definition { definition_id, .. } => {
                        ("status-editing-component", definition_id.0)
                    }
                };
                context.push(
                    self.catalog
                        .format(key, &BTreeMap::from([("id", id.to_string())])),
                );
            }
            context.push(self.digest.clone());
            ui.add(
                egui::Label::new(
                    egui::RichText::new(context.join("  \u{b7}  "))
                        .size(11.5)
                        .color(palette.faint),
                )
                .truncate(),
            );

            // The measured facts are pinned right, in the same mono pill the
            // viewport readouts use, so they line up down the whole session.
            let mut chips = vec![
                self.catalog.text("status-snap-on"),
                self.catalog
                    .format("status-grid", &BTreeMap::from([("step", "10".to_owned())])),
                self.catalog.text("status-refs-guaranteed"),
            ];
            chips.push(if self.exact_results.is_empty() {
                self.catalog.text("status-exact-unavailable")
            } else {
                self.catalog.format(
                    "status-exact-current",
                    &BTreeMap::from([
                        ("bodies", self.exact_render_body_count().to_string()),
                        ("refs", self.exact_stable_reference_count().to_string()),
                    ]),
                )
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for text in chips.into_iter().rev() {
                    status_chip(ui, palette, &text);
                }
            });
        });
    }
}

fn assistant_model_catalog_text() -> String {
    std::env::var_os("KETCHUP_ASSISTANT_MODELS")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe().ok().and_then(|path| {
                path.parent()
                    .map(|parent| parent.join("assistant-models.yaml"))
            })
        })
        .and_then(|path| std::fs::read_to_string(path).ok())
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

fn assistant_sidecar_command(
    distribution: AssistantDistribution,
) -> Result<(PathBuf, Vec<std::ffi::OsString>), String> {
    match distribution {
        AssistantDistribution::PrivateOauth => {
            let path = std::env::var_os("KETCHUP_PRIVATE_ASSISTANT")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::current_exe().ok().and_then(|path| {
                        path.parent()
                            .map(|parent| parent.join("KetchupPrivateAssistant.exe"))
                    })
                })
                .filter(|path| path.is_file())
                .ok_or_else(|| "KetchupPrivateAssistant.exe was not found".to_owned())?;
            Ok((path, Vec::new()))
        }
        AssistantDistribution::PublicApi => {
            let script = std::env::var_os("KETCHUP_PUBLIC_ASSISTANT")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .map(|root| root.join("sdk/python/ketchup_assistant.py"))
                })
                .filter(|path| path.is_file())
                .ok_or_else(|| "sdk/python/ketchup_assistant.py was not found".to_owned())?;
            let python = std::env::var_os("KETCHUP_PYTHON")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(if cfg!(windows) {
                        "python.exe"
                    } else {
                        "python3"
                    })
                });
            Ok((python, vec![script.into_os_string()]))
        }
    }
}

fn exact_worker_candidates() -> Vec<PathBuf> {
    let executable_name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let Some(current) = std::env::current_exe().ok() else {
        return Vec::new();
    };
    let Some(parent) = current.parent() else {
        return Vec::new();
    };
    let mut candidates = vec![parent.join(executable_name)];
    if let Some(grandparent) = parent.parent() {
        candidates.push(grandparent.join(executable_name));
    }
    candidates
}

impl eframe::App for KetchupApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(context);
    }
}

impl KetchupApp {
    fn show_smart_push_pull_chooser(&mut self, context: &egui::Context) {
        let Some(chooser) = self.smart_push_pull_chooser.as_ref() else {
            return;
        };
        let mut selected = chooser.selected;
        let snapshot = match &chooser.planning {
            SmartPushPullPlanning::Append => self.document.current(),
            SmartPushPullPlanning::TipReplacement(parent) => parent.snapshot().clone(),
        };
        let target_labels = chooser
            .targets
            .iter()
            .map(|target| {
                let occurrence_id = target.instance_path.root_occurrence();
                let occurrence = snapshot.occurrence(occurrence_id).map_or_else(
                    || occurrence_id.0.to_string(),
                    |item| item.name().to_owned(),
                );
                let feature_id = target
                    .extrusion_feature_id
                    .expect("a circular cut choice always has an extrusion");
                let feature = snapshot
                    .feature(feature_id)
                    .map_or_else(|| feature_id.0.to_string(), |item| item.name().to_owned());
                (
                    SmartPushPullChoice::CircularCut(occurrence_id),
                    self.catalog.format(
                        "choice-smart-push-pull-cut-target",
                        &BTreeMap::from([
                            ("feature", feature),
                            ("feature_id", feature_id.0.to_string()),
                            ("occurrence", occurrence),
                            ("occurrence_id", occurrence_id.0.to_string()),
                        ]),
                    ),
                )
            })
            .collect::<Vec<_>>();
        let title = self.catalog.text("choice-smart-push-pull-source");
        let explanation = self.catalog.text("choice-smart-push-pull-explanation");
        let new_feature = self.catalog.text("choice-smart-push-pull-new-feature");
        let continue_label = self.catalog.text("choice-smart-push-pull-continue");
        let cancel_label = self.catalog.text("choice-smart-push-pull-cancel");
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(title)
            .id(egui::Id::new("smart-push-pull-chooser"))
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(explanation);
                ui.separator();
                ui.radio_value(&mut selected, SmartPushPullChoice::NewFeature, new_feature);
                for (choice, label) in target_labels {
                    ui.radio_value(&mut selected, choice, label);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    confirm = ui.button(continue_label).clicked();
                    cancel = ui.button(cancel_label).clicked();
                });
            });
        if let Some(chooser) = self.smart_push_pull_chooser.as_mut() {
            chooser.selected = selected;
        }
        if cancel {
            self.cancel_preview();
            self.digest = self.catalog.text("digest-cancelled");
        } else if confirm {
            self.confirm_smart_push_pull_choice();
        }
    }

    /// Draw the whole designed shell into an `egui` context.
    ///
    /// This is the single entry point used both by the windowed `eframe`
    /// integration and by the offscreen [`crate::testing::HeadlessShell`].
    pub fn ui(&mut self, context: &egui::Context) {
        self.refresh_exact_products(context);
        self.refresh_beam_m5_products(context);
        let palette = self.palette();
        apply_shell_style(context, palette);
        self.handle_shortcuts(context);

        // Chrome surfaces carry a hairline on the edge they meet the viewport
        // on, so the shell reads as panels around a canvas rather than as one
        // flat sheet.
        let hairline = Stroke::new(1.0_f32, palette.line);
        let chrome = |bottom: bool, top: bool| {
            egui::Frame::new()
                .fill(palette.chrome)
                .inner_margin(egui::Margin::symmetric(12, 0))
                .stroke(Stroke::NONE)
                .outer_margin(egui::Margin::ZERO)
                .shadow(egui::epaint::Shadow::NONE)
                .stroke(if bottom || top {
                    hairline
                } else {
                    Stroke::NONE
                })
        };
        egui::TopBottomPanel::top("top-bar")
            .exact_height(46.0)
            .frame(chrome(true, false))
            .show(context, |ui| self.show_top_bar(ui));
        egui::TopBottomPanel::top("menu-bar")
            .exact_height(32.0)
            .frame(chrome(true, false))
            .show(context, |ui| self.show_menu_bar(ui));
        egui::TopBottomPanel::bottom("status-bar")
            .exact_height(32.0)
            .frame(chrome(false, true))
            .show(context, |ui| self.show_status_bar(ui));
        egui::SidePanel::left("tool-rail")
            .resizable(false)
            .exact_width(TOOL_RAIL_WIDTH)
            .frame(
                egui::Frame::new()
                    .fill(palette.chrome)
                    .inner_margin(egui::Margin::symmetric(0, 8))
                    .stroke(hairline),
            )
            .show(context, |ui| self.show_tool_rail(ui));
        if self.assistant_workspace_mode == AssistantWorkspaceMode::Dock {
            egui::SidePanel::right("right-dock")
                .resizable(true)
                .default_width(440.0)
                .width_range(380.0..=720.0)
                .frame(
                    egui::Frame::new()
                        .fill(palette.chrome)
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .stroke(hairline),
                )
                .show(context, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    self.show_parameter_editor(ui);
                    self.show_assistant(ui);
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(palette.viewport_outer))
                .show(context, |ui| self.viewport(ui));
        } else {
            egui::SidePanel::right("right-dock")
                .resizable(true)
                .default_width(340.0)
                .width_range(280.0..=520.0)
                .frame(
                    egui::Frame::new()
                        .fill(palette.chrome)
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .stroke(hairline),
                )
                .show(context, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    self.show_parameter_editor(ui);
                    self.show_outliner_without_assistant(ui);
                });
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(palette.chrome)
                        .inner_margin(egui::Margin::same(16)),
                )
                .show(context, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    self.show_assistant(ui);
                });
        }
        self.show_smart_push_pull_chooser(context);
        self.show_stl_import_window(context);
        self.show_shortcuts_window(context);
        self.poll_assistant_chat(context);
    }
}

/// One shared type scale and spacing rhythm for every surface of the shell.
///
/// Before this existed each panel picked its own font size, so a section title
/// was nearly twice the size of the text under it. The whole shell now derives
/// from four sizes: title, body, small and monospace.
fn apply_shell_style(context: &egui::Context, palette: Palette) {
    let mut visuals = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(palette.text);
    visuals.panel_fill = palette.chrome;
    visuals.window_fill = palette.panel;
    visuals.window_stroke = Stroke::new(1.0_f32, palette.line);
    visuals.extreme_bg_color = palette.bg;
    visuals.selection.bg_fill = palette.accent_wash(90);
    visuals.selection.stroke = Stroke::new(1.0_f32, palette.accent);
    visuals.hyperlink_color = palette.accent;
    visuals.widgets.noninteractive.bg_fill = palette.panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, palette.line);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, palette.dim);
    visuals.widgets.inactive.weak_bg_fill = palette.panel;
    visuals.widgets.inactive.bg_fill = palette.panel;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, palette.line);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, palette.text);
    visuals.widgets.hovered.weak_bg_fill = palette.panel2;
    visuals.widgets.hovered.bg_fill = palette.panel2;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, palette.line);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, palette.text);
    visuals.widgets.active.weak_bg_fill = palette.panel2;
    visuals.widgets.active.bg_fill = palette.panel2;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, palette.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, palette.text);
    visuals.widgets.open.weak_bg_fill = palette.panel2;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, palette.line);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(6);
    }

    let mut style = (*context.style()).clone();
    style.text_styles = [
        (
            egui::TextStyle::Heading,
            egui::FontId::proportional(SHELL_TITLE_SIZE),
        ),
        (
            egui::TextStyle::Body,
            egui::FontId::proportional(SHELL_BODY_SIZE),
        ),
        (
            egui::TextStyle::Button,
            egui::FontId::proportional(SHELL_BODY_SIZE),
        ),
        (
            egui::TextStyle::Small,
            egui::FontId::proportional(SHELL_SMALL_SIZE),
        ),
        (
            egui::TextStyle::Monospace,
            egui::FontId::monospace(SHELL_MONO_SIZE),
        ),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(6.0, 5.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    style.spacing.interact_size.y = 22.0;
    style.spacing.combo_width = 180.0;
    style.spacing.text_edit_width = 160.0;
    style.visuals = visuals;
    context.set_style(style);
}

/// Draw a dock section title so every section reads at the same weight.
fn section_header(ui: &mut egui::Ui, palette: Palette, title: &str) {
    ui.add_space(9.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title.to_uppercase())
                .size(SHELL_SECTION_SIZE)
                .strong()
                .color(palette.dim),
        );
    });
    ui.add_space(4.0);
}

/// One measured fact in the status bar, as a dotted monospace pill.
fn status_chip(ui: &mut egui::Ui, palette: Palette, text: &str) {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), egui::FontId::monospace(10.5), palette.dim);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + 26.0, galley.size().y + 6.0),
        Sense::hover(),
    );
    let painter = ui.painter();
    let corner = egui::CornerRadius::same(5);
    painter.rect_filled(rect, corner, palette.panel);
    painter.rect_stroke(
        rect,
        corner,
        Stroke::new(1.0_f32, palette.line),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(
        Pos2::new(rect.left() + 9.0, rect.center().y),
        2.5,
        palette.accent,
    );
    painter.galley(
        Pos2::new(rect.left() + 17.0, rect.center().y - galley.size().y * 0.5),
        galley,
        palette.dim,
    );
}

/// A hairline divider between two clusters inside a horizontal bar.
fn vertical_rule(ui: &mut egui::Ui, palette: Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, palette.line);
}

/// Wrap `content` in the shell's segmented-control shell: a raised, outlined,
/// rounded tray whose children are flush pills.
fn segmented(ui: &mut egui::Ui, palette: Palette, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0_f32, palette.line))
        .corner_radius(egui::CornerRadius::same(9))
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.spacing_mut().button_padding.x = 10.0;
            ui.horizontal(content);
        });
}

/// The square a glyph of `size` occupies inside a larger hit target.
fn shrink_to_icon(button: Rect, size: f32) -> Rect {
    Rect::from_center_size(button.center(), Vec2::splat(size))
}

/// Give a widget an accessible name.
///
/// Icon-only controls paint a glyph, which is useless both to a screen reader
/// and to an acceptance test. This publishes the command's localized name to
/// the accessibility tree instead.
fn name_widget(response: &egui::Response, enabled: bool, name: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
}

/// Which drawing represents a command in the rail and the menus.
///
/// One glyph can serve several commands — Zoom Fit and the Zoom tool share the
/// magnifier — so this is a mapping rather than a field on the command spec.
const fn command_icon(id: AppCommand) -> Icon {
    match id {
        AppCommand::Line => Icon::Line,
        AppCommand::Rectangle => Icon::Rectangle,
        AppCommand::Circle => Icon::Circle,
        AppCommand::Arc => Icon::Arc,
        AppCommand::Revolve => Icon::Orbit,
        AppCommand::Shell => Icon::PushPull,
        AppCommand::Fillet | AppCommand::Chamfer => Icon::Tape,
        AppCommand::PushPull => Icon::PushPull,
        AppCommand::Move => Icon::Move,
        AppCommand::Measure => Icon::Tape,
        AppCommand::Orbit => Icon::Orbit,
        AppCommand::Pan => Icon::Pan,
        AppCommand::Delete => Icon::Eraser,
        AppCommand::ZoomFit => Icon::Zoom,
        AppCommand::Undo => Icon::Undo,
        AppCommand::Redo => Icon::Redo,
        _ => Icon::Select,
    }
}

fn create_box_batch(
    definition_id: DefinitionId,
    feature_ids: [FeatureId; 2],
    occurrence_id: OccurrenceId,
    names: [&str; 4],
    origin_mm: Vec3,
    size_mm: Vec3,
) -> CommandBatch {
    let [profile_id, extrusion_id] = feature_ids;
    let [name, profile_name, extrusion_name, occurrence_name] = names;
    CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: name.to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: profile_id,
            definition_id,
            name: profile_name.to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![
                    [0.0, 0.0],
                    [size_mm.x, 0.0],
                    [size_mm.x, size_mm.y],
                    [0.0, size_mm.y],
                ],
            },
        },
        CanonicalCommand::CreateFeature {
            id: extrusion_id,
            definition_id,
            name: extrusion_name.to_owned(),
            kind: FeatureKind::Extrusion {
                profile: profile_id,
                height: Dimension::new(format_height(size_mm.z), size_mm.z)
                    .expect("validated box height is canonical"),
            },
        },
        CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name: occurrence_name.to_owned(),
            transform: Transform::from_translation(origin_mm.x, origin_mm.y, origin_mm.z)
                .expect("validated box origin is canonical"),
            parent: None,
            tag: None,
            visible: true,
        },
    ])
}

fn alignment_coordinate(item: &RenderBox, axis: Axis, mode: AlignMode) -> f64 {
    let (origin, size) = match axis {
        Axis::X => (item.origin_mm.x, item.size_mm.x),
        Axis::Y => (item.origin_mm.y, item.size_mm.y),
        Axis::Z => (item.origin_mm.z, item.size_mm.z),
    };
    match mode {
        AlignMode::Minimum => origin,
        AlignMode::Center => origin + size * 0.5,
        AlignMode::Maximum => origin + size,
    }
}

fn axis_vector(axis: Axis, value: f64) -> Vec3 {
    match axis {
        Axis::X => Vec3::new(value, 0.0, 0.0),
        Axis::Y => Vec3::new(0.0, value, 0.0),
        Axis::Z => Vec3::new(0.0, 0.0, value),
    }
}

const fn alignment_axis_label(axis: Axis) -> &'static str {
    match axis {
        Axis::X => "X",
        Axis::Y => "Y",
        Axis::Z => "Z",
    }
}

const fn alignment_mode_label(mode: AlignMode) -> &'static str {
    match mode {
        AlignMode::Minimum => "minimum",
        AlignMode::Center => "center",
        AlignMode::Maximum => "maximum",
    }
}

fn translated_transform(transform: Transform, delta_mm: Vec3) -> Result<Transform, ()> {
    let mut matrix = *transform.matrix();
    matrix[3] += delta_mm.x;
    matrix[7] += delta_mm.y;
    matrix[11] += delta_mm.z;
    Transform::from_matrix(matrix).map_err(|_| ())
}

fn rotate_transform_90(transform: Transform, local_box: ProjectedBox) -> Result<Transform, ()> {
    let center_x = local_box.origin_mm.x + local_box.size_mm.x * 0.5;
    let center_y = local_box.origin_mm.y + local_box.size_mm.y * 0.5;
    let local_rotation = Transform::from_matrix([
        0.0,
        -1.0,
        0.0,
        center_x + center_y,
        1.0,
        0.0,
        0.0,
        center_y - center_x,
        0.0,
        0.0,
        1.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ])
    .map_err(|_| ())?;
    Ok(transform.compose(local_rotation))
}

fn push_pull_batch(
    snapshot: &Snapshot,
    selection: &SelectionId,
    item: &RenderBox,
    new_extent_mm: f64,
    source_token: String,
) -> Option<CommandBatch> {
    let resolved = snapshot
        .resolve_instance_path(&selection.instance_path)
        .ok()?;
    if resolved.definition_id != selection.definition_id
        || item.definition_id != selection.definition_id
        || item.instance_path != selection.instance_path
    {
        return None;
    }
    let profile = snapshot.feature(item.profile_feature_id)?;
    if profile.definition_id() != selection.definition_id {
        return None;
    }
    if let Some(extrusion_id) = item.extrusion_feature_id {
        let extrusion = snapshot.feature(extrusion_id)?;
        if extrusion.definition_id() != selection.definition_id
            || !matches!(
                extrusion.kind(),
                FeatureKind::Extrusion { profile, .. } if *profile == item.profile_feature_id
            )
        {
            return None;
        }
    }
    let ElementId::Face { axis, side } = selection.element else {
        return None;
    };
    let mut commands = Vec::new();
    match axis {
        Axis::Z => {
            let dimension = Dimension::new(source_token, new_extent_mm).ok()?;
            if let Some(extrusion_id) = item.extrusion_feature_id {
                commands.push(CanonicalCommand::SetFeatureDimension {
                    id: extrusion_id,
                    dimension,
                });
            } else {
                let extrusion_id = FeatureId(
                    snapshot
                        .features()
                        .map(|feature| feature.id().0)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)?,
                );
                commands.push(CanonicalCommand::CreateFeature {
                    id: extrusion_id,
                    definition_id: selection.definition_id,
                    name: "Extrusion".to_owned(),
                    kind: FeatureKind::Extrusion {
                        profile: item.profile_feature_id,
                        height: dimension,
                    },
                });
            }
        }
        Axis::X | Axis::Y => {
            let FeatureKind::Profile { points_mm } = profile.kind() else {
                return None;
            };
            let coordinate = |point: &[f64; 2]| match axis {
                Axis::X => point[0],
                Axis::Y => point[1],
                Axis::Z => unreachable!(),
            };
            let minimum = points_mm.iter().map(coordinate).min_by(f64::total_cmp)?;
            let maximum = points_mm.iter().map(coordinate).max_by(f64::total_cmp)?;
            let old_extent = maximum - minimum;
            let mut resized = points_mm.clone();
            for point in &mut resized {
                let normalized = (coordinate(point) - minimum) / old_extent;
                let value = minimum + normalized * new_extent_mm;
                match axis {
                    Axis::X => point[0] = value,
                    Axis::Y => point[1] = value,
                    Axis::Z => unreachable!(),
                }
            }
            commands.push(CanonicalCommand::SetProfilePoints {
                id: item.profile_feature_id,
                points_mm: resized,
            });
        }
    }
    if side == Side::Minimum {
        if !selection.instance_path.is_root() {
            return None;
        }
        let occurrence_id = selection.instance_path.root_occurrence();
        let occurrence = snapshot.occurrence(occurrence_id)?;
        let delta = match axis {
            Axis::X => Vec3::new(item.size_mm.x - new_extent_mm, 0.0, 0.0),
            Axis::Y => Vec3::new(0.0, item.size_mm.y - new_extent_mm, 0.0),
            Axis::Z => Vec3::new(0.0, 0.0, item.size_mm.z - new_extent_mm),
        };
        commands.push(CanonicalCommand::SetOccurrenceTransform {
            id: occurrence_id,
            transform: translated_transform(occurrence.transform(), delta).ok()?,
        });
    }
    Some(CommandBatch::new(commands))
}

fn box_faces() -> [BoxFace; 6] {
    [
        BoxFace {
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Minimum,
            },
            corners: [0, 1, 3, 2],
            color: Color32::from_rgb(66, 74, 88),
        },
        BoxFace {
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
            corners: [4, 6, 7, 5],
            color: Color32::from_rgb(126, 145, 166),
        },
        BoxFace {
            element: ElementId::Face {
                axis: Axis::X,
                side: Side::Minimum,
            },
            corners: [0, 2, 6, 4],
            color: Color32::from_rgb(82, 96, 113),
        },
        BoxFace {
            element: ElementId::Face {
                axis: Axis::X,
                side: Side::Maximum,
            },
            corners: [1, 5, 7, 3],
            color: Color32::from_rgb(78, 91, 107),
        },
        BoxFace {
            element: ElementId::Face {
                axis: Axis::Y,
                side: Side::Minimum,
            },
            corners: [0, 4, 5, 1],
            color: Color32::from_rgb(94, 108, 126),
        },
        BoxFace {
            element: ElementId::Face {
                axis: Axis::Y,
                side: Side::Maximum,
            },
            corners: [2, 3, 7, 6],
            color: Color32::from_rgb(88, 102, 119),
        },
    ]
}

fn exact_face_element(role: ExactFaceRole) -> Option<ElementId> {
    match role {
        ExactFaceRole::Top | ExactFaceRole::BoxShellRim => Some(ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        }),
        ExactFaceRole::Bottom | ExactFaceRole::BoxShellOuterBottom => Some(ElementId::Face {
            axis: Axis::Z,
            side: Side::Minimum,
        }),
        ExactFaceRole::PocketFloor => Some(ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        }),
        ExactFaceRole::East
        | ExactFaceRole::CutEast
        | ExactFaceRole::PocketEast
        | ExactFaceRole::BoxShellOuterEast => Some(ElementId::Face {
            axis: Axis::X,
            side: Side::Maximum,
        }),
        ExactFaceRole::CutWest | ExactFaceRole::PocketWest => Some(ElementId::Face {
            axis: Axis::X,
            side: Side::Minimum,
        }),
        ExactFaceRole::CutSouth | ExactFaceRole::PocketSouth => Some(ElementId::Face {
            axis: Axis::Y,
            side: Side::Minimum,
        }),
        ExactFaceRole::CutNorth | ExactFaceRole::PocketNorth => Some(ElementId::Face {
            axis: Axis::Y,
            side: Side::Maximum,
        }),
        ExactFaceRole::CircleSide
        | ExactFaceRole::ArcSide
        | ExactFaceRole::CutCircle
        | ExactFaceRole::RevolveBottom
        | ExactFaceRole::RevolveBody
        | ExactFaceRole::RevolveShoulder
        | ExactFaceRole::RevolveNeck
        | ExactFaceRole::RevolveMouth
        | ExactFaceRole::RevolveSide0
        | ExactFaceRole::RevolveSide1
        | ExactFaceRole::RevolveStart
        | ExactFaceRole::RevolveEnd
        | ExactFaceRole::ShellOuterBottom
        | ExactFaceRole::ShellOuterBody
        | ExactFaceRole::ShellOuterShoulder
        | ExactFaceRole::ShellOuterNeck
        | ExactFaceRole::ShellRim
        | ExactFaceRole::ShellInnerBottom
        | ExactFaceRole::ShellInnerBody
        | ExactFaceRole::ShellInnerShoulder
        | ExactFaceRole::ShellInnerNeck
        | ExactFaceRole::PlanarOffsetFace
        | ExactFaceRole::SweepStart
        | ExactFaceRole::SweepEnd
        | ExactFaceRole::SweepSide0
        | ExactFaceRole::SweepSide1
        | ExactFaceRole::SweepSide2
        | ExactFaceRole::SweepSide3
        | ExactFaceRole::LoftStart
        | ExactFaceRole::LoftEnd
        | ExactFaceRole::LoftSide => None,
    }
}

fn overlap_signature(pick: &PickResult) -> Vec<SelectionId> {
    pick.overlapping
        .iter()
        .map(|hit| hit.reference.clone())
        .collect()
}

fn exact_surface_element(normal: Vec3) -> Option<ElementId> {
    let components = [normal.x.abs(), normal.y.abs(), normal.z.abs()];
    let (axis_index, magnitude) = components
        .into_iter()
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(&right.1))?;
    if magnitude <= 1.0e-9 {
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

fn current_unix_time_ms() -> Result<u64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    u64::try_from(millis).map_err(|_| "system time is outside the supported range".to_owned())
}

fn exact_step_loss_report(package: &ExactRevolvePackage) -> String {
    format!(
        "authority=accepted exact OCCT B-Rep\nformat=ISO 10303 STEP\neditability_loss=canonical Ketchup features, rules, dimensions, and Undo history are not preserved\ntopology_loss=exact B-Rep topology is preserved, but durable Ketchup subshape identity is not preserved\ntolerance_loss=no tessellation loss; receiving systems may apply a different modeling tolerance\nsource_tolerance={}\nsource_digest={}\nbackend={}\nresult_fingerprint={}\n",
        package.identity.tolerance,
        package.identity.source_digest,
        package.identity.backend,
        package.identity.result_fingerprint,
    )
}

fn exact_model_step_loss_report(
    snapshot: &Snapshot,
    model: &[(ExactBodyPackage, Transform)],
) -> String {
    let fingerprints = model
        .iter()
        .map(|(package, _)| package.result_key().result_fingerprint)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "authority=accepted exact OCCT B-Rep\nformat=ISO 10303 STEP\nconversion=current-visible-exact-model-to-world-space-brep\neditability_loss=canonical Ketchup features, rules, dimensions, hierarchy, and Undo history are not preserved\ntopology_loss=exact B-Rep topology is preserved, but durable Ketchup subshape and occurrence identity are not preserved\ntolerance_loss=no tessellation loss; receiving systems may apply a different modeling tolerance\nsource_digest={}\noccurrence_count={}\nresult_fingerprints={fingerprints}\n",
        snapshot.canonical_digest(),
        model.len(),
    )
}

fn exact_stl_export_evidence(path: &Path, bundle: &ExactStlExport) -> Vec<u8> {
    export_bundle_evidence(
        b"ketchup.current-model-stl-export.v1",
        path,
        bundle.mesh_stl.as_bytes(),
        &path.with_extension("stl.loss.txt"),
        bundle.loss_report.as_bytes(),
    )
}

fn export_bundle_evidence(
    domain: &[u8],
    primary_path: &Path,
    primary: &[u8],
    report_path: &Path,
    report: &[u8],
) -> Vec<u8> {
    let mut evidence = domain.to_vec();
    for (path, artifact) in [(primary_path, primary), (report_path, report)] {
        let path = path.to_string_lossy();
        evidence.extend_from_slice(&(path.len() as u64).to_le_bytes());
        evidence.extend_from_slice(path.as_bytes());
        evidence.extend_from_slice(&(artifact.len() as u64).to_le_bytes());
        evidence.extend_from_slice(artifact);
    }
    evidence
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExportBundlePrecondition {
    primary_sha256: Option<String>,
    report_sha256: Option<String>,
}

impl ExportBundlePrecondition {
    fn capture(primary_path: &Path, report_path: &Path) -> Result<Self, String> {
        Ok(Self {
            primary_sha256: export_target_sha256(primary_path)?,
            report_sha256: export_target_sha256(report_path)?,
        })
    }
}

fn export_target_sha256(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    std::fs::read(path)
        .map(|bytes| Some(ketchup_core::graph::sha256_hex(&bytes)))
        .map_err(|error| error.to_string())
}

fn empty_export_temp_path(path: &Path, prefix: &str) -> Result<tempfile::TempPath, String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(prefix)
        .tempfile_in(parent)
        .map_err(|error| error.to_string())?
        .into_temp_path();
    std::fs::remove_file(&temporary).map_err(|error| error.to_string())?;
    Ok(temporary)
}

fn persist_export_backup_noclobber(backup: tempfile::TempPath, path: &Path) -> Result<(), String> {
    match backup.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) => {
            let persist_error = error.error;
            let preserved = error
                .path
                .keep()
                .map_err(|keep_error| keep_error.error.to_string())?;
            Err(format!(
                "{persist_error}; backup preserved at {}",
                preserved.display()
            ))
        }
    }
}

fn export_backup(
    path: &Path,
    expected_sha256: Option<&str>,
) -> Result<Option<tempfile::TempPath>, String> {
    let current_sha256 = export_target_sha256(path)?;
    if current_sha256.as_deref() != expected_sha256 {
        return Err(format!(
            "export target {} changed after authorization",
            path.display()
        ));
    }
    if expected_sha256.is_none() {
        return Ok(None);
    }
    let backup = empty_export_temp_path(path, ".ketchup-export-backup-")?;
    std::fs::rename(path, &backup).map_err(|error| error.to_string())?;
    let moved_sha256 = export_target_sha256(&backup);
    if moved_sha256.as_ref().ok().and_then(Option::as_deref) == expected_sha256 {
        return Ok(Some(backup));
    }
    let mismatch = moved_sha256
        .err()
        .unwrap_or_else(|| format!("export target {} changed during backup", path.display()));
    let restore = persist_export_backup_noclobber(backup, path);
    Err(match restore {
        Ok(()) => mismatch,
        Err(restore) => format!("{mismatch}; concurrent target restore failed: {restore}"),
    })
}

fn restore_export_backup(
    path: &Path,
    backup: &mut Option<tempfile::TempPath>,
    published_sha256: Option<&str>,
) -> Result<(), String> {
    let quarantined = if path.exists() {
        let quarantined = empty_export_temp_path(path, ".ketchup-export-rollback-")?;
        std::fs::rename(path, &quarantined).map_err(|error| error.to_string())?;
        Some(quarantined)
    } else {
        None
    };
    if let Some(quarantined) = quarantined {
        let quarantined_sha256 = export_target_sha256(&quarantined);
        if quarantined_sha256.as_ref().ok().and_then(Option::as_deref) != published_sha256 {
            let concurrent_restore = persist_export_backup_noclobber(quarantined, path);
            let original_preserved = backup
                .take()
                .map(tempfile::TempPath::keep)
                .transpose()
                .map_err(|error| error.error.to_string())?;
            return Err(match (concurrent_restore, original_preserved) {
                (Ok(()), Some(original)) => format!(
                    "cannot roll back {} because it changed concurrently; original preserved at {}",
                    path.display(),
                    original.display()
                ),
                (Ok(()), None) => format!(
                    "cannot roll back {} because it changed concurrently",
                    path.display()
                ),
                (Err(concurrent), Some(original)) => format!(
                    "cannot roll back {} because it changed concurrently; {concurrent}; original preserved at {}",
                    path.display(),
                    original.display()
                ),
                (Err(concurrent), None) => format!(
                    "cannot roll back {} because it changed concurrently; {concurrent}",
                    path.display()
                ),
            });
        }
    }
    if let Some(saved) = backup.take() {
        persist_export_backup_noclobber(saved, path)?;
    }
    Ok(())
}

fn rollback_export_bundle(
    primary_path: &Path,
    primary_backup: &mut Option<tempfile::TempPath>,
    primary_published_sha256: Option<&str>,
    report_path: &Path,
    report_backup: &mut Option<tempfile::TempPath>,
    report_published_sha256: Option<&str>,
) -> Result<(), String> {
    let primary = restore_export_backup(primary_path, primary_backup, primary_published_sha256);
    let report = restore_export_backup(report_path, report_backup, report_published_sha256);
    match (primary, report) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(primary), Ok(())) => Err(format!("primary rollback failed: {primary}")),
        (Ok(()), Err(report)) => Err(format!("loss-report rollback failed: {report}")),
        (Err(primary), Err(report)) => Err(format!(
            "primary rollback failed: {primary}; loss-report rollback failed: {report}"
        )),
    }
}

fn persist_export_temporary(temporary: tempfile::TempPath, path: &Path) -> Result<(), String> {
    temporary
        .persist_noclobber(path)
        .map(|_| ())
        .map_err(|error| error.error.to_string())
}

fn write_export_bundle(
    primary_path: &Path,
    primary: &[u8],
    report_path: &Path,
    report: &[u8],
    precondition: &ExportBundlePrecondition,
) -> Result<(), String> {
    let primary_parent = primary_path.parent().unwrap_or_else(|| Path::new("."));
    let report_parent = report_path.parent().unwrap_or_else(|| Path::new("."));
    if primary_parent != report_parent {
        return Err("export artifact and loss report must share a directory".to_owned());
    }
    if primary_path.is_dir() || report_path.is_dir() {
        return Err("export target must be a regular file path".to_owned());
    }
    let mut primary_temporary = tempfile::Builder::new()
        .prefix(".ketchup-export-primary-")
        .tempfile_in(primary_parent)
        .map_err(|error| error.to_string())?;
    let mut report_temporary = tempfile::Builder::new()
        .prefix(".ketchup-export-report-")
        .tempfile_in(primary_parent)
        .map_err(|error| error.to_string())?;
    primary_temporary
        .write_all(primary)
        .and_then(|()| primary_temporary.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    report_temporary
        .write_all(report)
        .and_then(|()| report_temporary.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    let primary_temporary = primary_temporary.into_temp_path();
    let report_temporary = report_temporary.into_temp_path();

    let primary_published_sha256 = ketchup_core::graph::sha256_hex(primary);
    let mut primary_backup = export_backup(primary_path, precondition.primary_sha256.as_deref())?;
    let mut report_backup = match export_backup(report_path, precondition.report_sha256.as_deref())
    {
        Ok(backup) => backup,
        Err(error) => {
            let rollback = restore_export_backup(primary_path, &mut primary_backup, None);
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback) => format!("{error}; primary rollback failed: {rollback}"),
            });
        }
    };
    if let Err(error) = persist_export_temporary(primary_temporary, primary_path) {
        let rollback = rollback_export_bundle(
            primary_path,
            &mut primary_backup,
            None,
            report_path,
            &mut report_backup,
            None,
        );
        return Err(match rollback {
            Ok(()) => error,
            Err(rollback) => format!("{error}; {rollback}"),
        });
    }
    if let Err(error) = persist_export_temporary(report_temporary, report_path) {
        let rollback = rollback_export_bundle(
            primary_path,
            &mut primary_backup,
            Some(&primary_published_sha256),
            report_path,
            &mut report_backup,
            None,
        );
        return Err(match rollback {
            Ok(()) => error,
            Err(rollback) => format!("{error}; {rollback}"),
        });
    }
    Ok(())
}

fn exact_mesh_export_evidence(bundle: &ExactMeshExport) -> Vec<u8> {
    let mut evidence = b"ketchup.exact-mesh-export.v1".to_vec();
    for artifact in [&bundle.mesh_obj, &bundle.loss_report] {
        evidence.extend_from_slice(&(artifact.len() as u64).to_le_bytes());
        evidence.extend_from_slice(artifact.as_bytes());
    }
    evidence
}

fn write_exact_mesh_export(path: &Path, bundle: ExactMeshExport) -> Result<(), String> {
    std::fs::write(path, bundle.mesh_obj).map_err(|error| error.to_string())?;
    std::fs::write(path.with_extension("obj.loss.txt"), bundle.loss_report)
        .map_err(|error| error.to_string())
}

fn transform_model_point(transform: Transform, point: Vec3) -> Vec3 {
    let matrix = transform.matrix();
    Vec3::new(
        matrix[0] * point.x + matrix[1] * point.y + matrix[2] * point.z + matrix[3],
        matrix[4] * point.x + matrix[5] * point.y + matrix[6] * point.z + matrix[7],
        matrix[8] * point.x + matrix[9] * point.y + matrix[10] * point.z + matrix[11],
    )
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

fn point_depth(point: Vec3, forward: Vec3) -> f64 {
    point.x * forward.x + point.y * forward.y + point.z * forward.z
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

fn face_color_from_normal(normal: Vec3) -> Color32 {
    let element = face_element_from_normal(normal);
    box_faces()
        .into_iter()
        .find(|face| face.element == element)
        .map_or(Color32::from_rgb(94, 108, 126), |face| face.color)
}

fn face_is_visible(element: &ElementId, forward: Vec3) -> bool {
    let ElementId::Face { axis, side } = element else {
        return false;
    };
    let direction = match axis {
        Axis::X => forward.x,
        Axis::Y => forward.y,
        Axis::Z => forward.z,
    };
    let outward_dot_forward = match side {
        Side::Minimum => -direction,
        Side::Maximum => direction,
    };
    outward_dot_forward < -1.0e-9
}

fn projected_face_has_area(corners: [usize; 4], projected: &[Pos2; 8]) -> bool {
    let points = corners.map(|index| projected[index]);
    projected_polygon_has_area(&points)
}

fn projected_polygon_has_area(points: &[Pos2]) -> bool {
    let twice_area = (0..points.len())
        .map(|index| {
            let current = points[index];
            let next = points[(index + 1) % points.len()];
            current.x * next.y - next.x * current.y
        })
        .sum::<f32>();
    twice_area.abs() >= 1.0
}

fn adaptive_grid_step(pixels_per_mm: f64) -> f64 {
    const TARGET_SPACING_PX: f64 = 32.0;
    let desired = (TARGET_SPACING_PX / pixels_per_mm.max(1.0e-12)).max(GRID_STEP_MM);
    let magnitude = 10.0_f64.powf(desired.log10().floor());
    let normalized = desired / magnitude;
    let factor = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    factor * magnitude
}

fn projected_bounds(points: &[Vec3], project: impl Fn(Vec3) -> Pos2) -> Rect {
    points
        .iter()
        .map(|point| Rect::from_min_max(project(*point), project(*point)))
        .reduce(|left, right| left.union(right))
        .unwrap_or(Rect::ZERO)
}

fn dot(left: Vec3, right: Vec3) -> f64 {
    left.x * right.x + left.y * right.y + left.z * right.z
}

fn vector_length(vector: Vec3) -> f64 {
    (vector.x * vector.x + vector.y * vector.y + vector.z * vector.z).sqrt()
}

fn snapped_move_delta(start: Vec3, end: Vec3, constrain_axis: bool) -> Vec3 {
    let mut delta = Vec3::new(end.x - start.x, end.y - start.y, 0.0);
    if constrain_axis {
        if delta.x.abs() >= delta.y.abs() {
            delta.y = 0.0;
        } else {
            delta.x = 0.0;
        }
    }
    Vec3::new(
        (delta.x / GRID_STEP_MM).round() * GRID_STEP_MM,
        (delta.y / GRID_STEP_MM).round() * GRID_STEP_MM,
        0.0,
    )
}

fn parse_move_vector(input: &str) -> Option<Vec3> {
    let trimmed = input.trim();
    let numeric = trimmed
        .strip_suffix("mm")
        .or_else(|| trimmed.strip_suffix("MM"))
        .unwrap_or(trimmed);
    let values = numeric
        .split([',', ';', 'x', 'X', '*'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let vector = match values.as_slice() {
        [x, y] => Vec3::new(*x, *y, 0.0),
        [x, y, z] => Vec3::new(*x, *y, *z),
        _ => return None,
    };
    (vector.x.is_finite()
        && vector.y.is_finite()
        && vector.z.is_finite()
        && vector_length(vector) > 0.0)
        .then_some(vector)
}

fn format_vector_mm(vector: Vec3) -> String {
    format!(
        "{},{},{} mm",
        format_height(vector.x),
        format_height(vector.y),
        format_height(vector.z)
    )
}

fn parse_rectangle_dimensions(input: &str) -> Option<[f64; 2]> {
    let values = input
        .split([',', ';', 'x', 'X', '*'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    match values.as_slice() {
        [width, depth]
            if width.is_finite() && depth.is_finite() && *width > 0.01 && *depth > 0.01 =>
        {
            Some([*width, *depth])
        }
        _ => None,
    }
}

fn push_pull_distance_from_pointer(drag: PushPullDrag, pointer: Pos2) -> f64 {
    let pointer_delta = pointer - drag.pointer_start;
    let raw_distance =
        f64::from(pointer_delta.dot(drag.screen_normal)) / f64::from(drag.pixels_per_mm);
    ((raw_distance / GRID_STEP_MM).round() * GRID_STEP_MM).max(-drag.extent_start_mm + 0.01)
}

fn tangent_points(anchor: Vec3, center: Vec3, radius: f64) -> Vec<Vec3> {
    let delta = anchor - center;
    let distance_squared = delta.x * delta.x + delta.y * delta.y;
    if !radius.is_finite() || radius <= 0.0 || distance_squared <= radius * radius + 1.0e-9 {
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
    if length <= 1.0e-12 {
        0.0
    } else {
        (chord.x * (point.y - start.y) - chord.y * (point.x - start.x)) / length
    }
}

fn arc_geometry(start: Vec3, end: Vec3, bulge: Vec3) -> Option<ArcGeometry> {
    if (start.z - end.z).abs() > 1.0e-6 || (start.z - bulge.z).abs() > 1.0e-6 {
        return None;
    }
    let determinant = 2.0
        * (start.x * (end.y - bulge.y) + end.x * (bulge.y - start.y) + bulge.x * (start.y - end.y));
    if !determinant.is_finite() || determinant.abs() <= 1.0e-9 {
        return None;
    }
    let start_squared = start.x * start.x + start.y * start.y;
    let end_squared = end.x * end.x + end.y * end.y;
    let bulge_squared = bulge.x * bulge.x + bulge.y * bulge.y;
    let center = Vec3::new(
        (start_squared * (end.y - bulge.y)
            + end_squared * (bulge.y - start.y)
            + bulge_squared * (start.y - end.y))
            / determinant,
        (start_squared * (bulge.x - end.x)
            + end_squared * (start.x - bulge.x)
            + bulge_squared * (end.x - start.x))
            / determinant,
        start.z,
    );
    let radius = vector_length(center - start);
    let end_radius = vector_length(center - end);
    if !radius.is_finite()
        || radius <= 0.01
        || (radius - end_radius).abs() > 1.0e-8 * radius.max(end_radius).max(1.0)
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

fn arc_polyline(arc: ArcGeometry, segments: usize) -> Vec<Vec3> {
    let radius = vector_length(arc.start - arc.center);
    let start_angle = (arc.start.y - arc.center.y).atan2(arc.start.x - arc.center.x);
    let end_angle = (arc.end.y - arc.center.y).atan2(arc.end.x - arc.center.x);
    let mut sweep = end_angle - start_angle;
    if arc.clockwise {
        while sweep >= 0.0 {
            sweep -= std::f64::consts::TAU;
        }
    } else {
        while sweep <= 0.0 {
            sweep += std::f64::consts::TAU;
        }
    }
    (0..=segments)
        .map(|index| {
            let angle = start_angle + sweep * index as f64 / segments as f64;
            Vec3::new(
                arc.center.x + radius * angle.cos(),
                arc.center.y + radius * angle.sin(),
                arc.start.z,
            )
        })
        .collect()
}

fn exact_arc_profile_geometry(
    segments: &[ProfileSegment],
    closed: bool,
) -> Option<ExactArcProfileGeometry> {
    let [
        ProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        },
        ProfileSegment::Line {
            start_mm: line_start,
            end_mm: line_end,
        },
    ] = segments
    else {
        return None;
    };
    (closed && end_mm == line_start && start_mm == line_end)
        .then_some((*start_mm, *end_mm, *center_mm, *clockwise))
}

fn exact_circle_geometry(segments: &[ProfileSegment], closed: bool) -> Option<([f64; 2], f64)> {
    let [
        ProfileSegment::CircularArc {
            start_mm: first_start,
            end_mm: first_end,
            center_mm: first_center,
            clockwise: first_clockwise,
        },
        ProfileSegment::CircularArc {
            start_mm: second_start,
            end_mm: second_end,
            center_mm: second_center,
            clockwise: second_clockwise,
        },
    ] = segments
    else {
        return None;
    };
    if !closed
        || first_start != second_end
        || first_end != second_start
        || first_center != second_center
        || first_clockwise != second_clockwise
    {
        return None;
    }
    let first_vector = [
        first_start[0] - first_center[0],
        first_start[1] - first_center[1],
    ];
    let end_vector = [
        first_end[0] - first_center[0],
        first_end[1] - first_center[1],
    ];
    if first_vector[0] != -end_vector[0] || first_vector[1] != -end_vector[1] {
        return None;
    }
    let radius = first_vector[0].hypot(first_vector[1]);
    (radius.is_finite() && radius > 0.0).then_some((*first_center, radius))
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

fn format_signed_mm(distance: f64) -> String {
    if distance > 0.0 {
        format!("+{} mm", format_height(distance))
    } else {
        format!("{} mm", format_height(distance))
    }
}

fn format_height(height: f64) -> String {
    let formatted = format!("{height:.2}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn face_extent(item: &RenderBox, element: Option<&ElementId>) -> Option<f64> {
    match element? {
        ElementId::Face { axis: Axis::X, .. } => Some(item.size_mm.x),
        ElementId::Face { axis: Axis::Y, .. } => Some(item.size_mm.y),
        ElementId::Face { axis: Axis::Z, .. } => Some(item.size_mm.z),
        _ => None,
    }
}

fn resize_box_from_face(
    item: &RenderBox,
    element: &ElementId,
    new_extent_mm: f64,
) -> Option<RenderBox> {
    let mut item = item.clone();
    if !new_extent_mm.is_finite() || new_extent_mm <= 0.01 {
        return None;
    }
    let ElementId::Face { axis, side } = element else {
        return None;
    };
    let (origin, extent) = match axis {
        Axis::X => (&mut item.origin_mm.x, &mut item.size_mm.x),
        Axis::Y => (&mut item.origin_mm.y, &mut item.size_mm.y),
        Axis::Z => (&mut item.origin_mm.z, &mut item.size_mm.z),
    };
    if *side == Side::Minimum {
        *origin += *extent - new_extent_mm;
    }
    *extent = new_extent_mm;
    Some(item)
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
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;
    use ketchup_core::document::ProposalGoal;
    use ketchup_core::graph::{EvaluatorNodeKind, PortSpec};

    #[test]
    fn export_rollback_preserves_concurrent_destination_and_original_backup() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("model.step");
        std::fs::write(&target, b"original").unwrap();
        let original_sha256 = export_target_sha256(&target).unwrap();
        let mut backup = export_backup(&target, original_sha256.as_deref()).unwrap();
        let backup_path = backup.as_ref().unwrap().to_path_buf();
        std::fs::write(&target, b"concurrent writer").unwrap();

        let error = restore_export_backup(&target, &mut backup, None).unwrap_err();

        assert!(error.contains("changed concurrently"));
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent writer");
        assert_eq!(std::fs::read(backup_path).unwrap(), b"original");
        assert!(backup.is_none());
    }

    #[test]
    fn export_rollback_replaces_only_its_own_published_artifact() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("model.step");
        std::fs::write(&target, b"original").unwrap();
        let original_sha256 = export_target_sha256(&target).unwrap();
        let mut backup = export_backup(&target, original_sha256.as_deref()).unwrap();
        let published = b"ketchup export";
        std::fs::write(&target, published).unwrap();
        let published_sha256 = ketchup_core::graph::sha256_hex(published);

        restore_export_backup(&target, &mut backup, Some(&published_sha256)).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert!(backup.is_none());
    }

    #[test]
    fn only_body_producing_features_are_export_candidates() {
        assert!(!FeatureKind::Profile { points_mm: vec![] }.produces_body());
        assert!(
            !FeatureKind::BottleProfileControl {
                profile: FeatureId(1),
                body_radius: Dimension::new("1", 1.0).unwrap(),
                body_height: Dimension::new("1", 1.0).unwrap(),
                shoulder_rise: Dimension::new("1", 1.0).unwrap(),
            }
            .produces_body()
        );
        assert!(
            FeatureKind::Extrusion {
                profile: FeatureId(1),
                height: Dimension::new("1", 1.0).unwrap(),
            }
            .produces_body()
        );
    }

    fn apply_reviewed_model_intent(app: &mut KetchupApp, intent: AssistantModelIntent) -> bool {
        app.prepare_assistant_model_intent(intent) && app.confirm_assistant_proposal()
    }

    fn select_initial_top_face(app: &mut KetchupApp) {
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
    }

    #[test]
    fn manual_and_assistant_push_pull_share_the_identical_canonical_batch() {
        let mut app = KetchupApp::new();
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        app.set_push_pull_distance_input("15");
        assert!(app.start_preview());
        let manual = app.smart_push_pull_proposal.as_ref().unwrap().clone();
        app.cancel_preview();

        assert!(
            app.prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
                target: FeatureId(2),
                value_text: "35".to_owned(),
            })
        );
        let assistant = app.assistant_proposal.as_ref().unwrap();
        assert_eq!(manual.batch(), assistant.batch());
        assert_eq!(manual.command_digest(), assistant.command_digest());
        assert_eq!(manual.principal(), ProposalPrincipal::ManualClient);
        assert_eq!(assistant.principal(), ProposalPrincipal::LocalAssistant);
        assert_eq!(app.document_revision(), manual.provenance_revision());
    }

    #[test]
    fn assistant_conversation_changes_participate_in_document_dirty_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chat-dirty.ketchup");
        let mut app = KetchupApp::new().with_dialogs(Box::new(
            dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
        ));

        assert!(app.save_document_to(&path));
        assert!(!app.is_dirty());
        app.assistant_messages.push(AssistantChatMessage {
            role: AssistantMessageRole::User,
            text: "Create a shelf.".to_owned(),
            source: "test".to_owned(),
        });
        assert!(app.is_dirty());
        assert!(app.save_document_to(&path));
        assert!(!app.is_dirty());
        app.new_assistant_chat();
        assert!(app.is_dirty());
    }

    #[test]
    fn new_chat_cancels_the_active_assistant_request() {
        let mut app = KetchupApp::new();
        let cancellation = AssistantCancellation::default();
        let (_sender, receiver) = mpsc::channel();
        app.assistant_chat_task = Some(AssistantChatTask {
            receiver,
            cancellation: cancellation.clone(),
            document_id: app.document.current().document_id(),
            revision_id: app.document.current().revision_id(),
            canonical_digest: app.document.current().canonical_digest(),
            source: "test".to_owned(),
        });

        app.new_assistant_chat();

        assert!(cancellation.is_cancelled());
        assert!(app.assistant_chat_task.is_none());
    }

    #[test]
    fn new_and_open_cancel_active_assistant_requests() {
        let mut app = KetchupApp::new();
        let new_cancellation = AssistantCancellation::default();
        let (_new_sender, new_receiver) = mpsc::channel();
        app.assistant_chat_task = Some(AssistantChatTask {
            receiver: new_receiver,
            cancellation: new_cancellation.clone(),
            document_id: app.document.current().document_id(),
            revision_id: app.document.current().revision_id(),
            canonical_digest: app.document.current().canonical_digest(),
            source: "test".to_owned(),
        });

        app.new_document();

        assert!(new_cancellation.is_cancelled());
        assert!(app.assistant_chat_task.is_none());

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("assistant-open-cancel.ketchup");
        assert!(app.save_document_to(&path));
        let open_cancellation = AssistantCancellation::default();
        let (_open_sender, open_receiver) = mpsc::channel();
        app.assistant_chat_task = Some(AssistantChatTask {
            receiver: open_receiver,
            cancellation: open_cancellation.clone(),
            document_id: app.document.current().document_id(),
            revision_id: app.document.current().revision_id(),
            canonical_digest: app.document.current().canonical_digest(),
            source: "test".to_owned(),
        });

        assert!(app.open_document_from(&path));

        assert!(open_cancellation.is_cancelled());
        assert!(app.assistant_chat_task.is_none());
    }

    #[test]
    fn assistant_progress_phases_are_accessible_with_deterministic_channels() {
        let mut app = KetchupApp::new();
        let requesting = app.catalog.text("assistant-progress-requesting");
        let executing = app.catalog.text("assistant-progress-executing");
        let (_sender, receiver) = mpsc::channel();
        app.assistant_chat_task = Some(AssistantChatTask {
            receiver,
            cancellation: AssistantCancellation::default(),
            document_id: app.document.current().document_id(),
            revision_id: app.document.current().revision_id(),
            canonical_digest: app.document.current().canonical_digest(),
            source: "test".to_owned(),
        });
        let mut harness = Harness::builder()
            .with_size(Vec2::new(1600.0, 1000.0))
            .build_state(|context, app: &mut KetchupApp| app.ui(context), app);

        harness.run();

        assert!(
            harness
                .query_all_by(|node| {
                    !node.is_hidden()
                        && (node.label().as_deref() == Some(&requesting)
                            || node.value().as_deref() == Some(&requesting))
                })
                .next()
                .is_some()
        );

        let state = harness.state_mut();
        state.assistant_chat_task = None;
        state.assistant_pending_execution = Some(AssistantPendingExecution {
            result: AssistantChatResult {
                message: "Moved it.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: Vec::new(),
                    translations: vec![
                        ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                            occurrence_id: 1,
                            delta_mm: [25.0, 0.0, 0.0],
                        },
                    ],
                    linear_arrays: Vec::new(),
                }),
            },
            document_id: state.document.current().document_id(),
            revision_id: state.document.current().revision_id(),
            canonical_digest: state.document.current().canonical_digest(),
            source: "test".to_owned(),
        });
        harness.run();

        assert!(
            harness
                .query_all_by(|node| {
                    !node.is_hidden()
                        && (node.label().as_deref() == Some(&executing)
                            || node.value().as_deref() == Some(&executing))
                })
                .next()
                .is_some()
        );
    }

    #[test]
    fn new_chat_discards_a_pending_assistant_execution_before_commit() {
        let mut app = KetchupApp::new();
        let revision = app.document.current().revision_id();
        app.assistant_pending_execution = Some(AssistantPendingExecution {
            result: AssistantChatResult {
                message: "Moved it.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: Vec::new(),
                    translations: vec![
                        ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                            occurrence_id: 1,
                            delta_mm: [25.0, 0.0, 0.0],
                        },
                    ],
                    linear_arrays: Vec::new(),
                }),
            },
            document_id: app.document.current().document_id(),
            revision_id: revision,
            canonical_digest: app.document.current().canonical_digest(),
            source: "test".to_owned(),
        });

        app.new_assistant_chat();
        app.poll_assistant_chat(&egui::Context::default());

        assert!(app.assistant_pending_execution.is_none());
        assert_eq!(app.document.current().revision_id(), revision);
        assert!(app.assistant_verification.is_none());
    }

    #[test]
    fn assistant_model_change_requires_explicit_confirmation_after_validation() {
        let mut app = KetchupApp::new();
        let revision = app.document.current().revision_id();
        let (sender, receiver) = mpsc::channel();
        app.assistant_chat_task = Some(AssistantChatTask {
            receiver,
            cancellation: AssistantCancellation::default(),
            document_id: app.document.current().document_id(),
            revision_id: revision,
            canonical_digest: app.document.current().canonical_digest(),
            source: "test".to_owned(),
        });
        sender
            .send(Ok(AssistantChatResult {
                message: "Moved it.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: Vec::new(),
                    translations: vec![
                        ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                            occurrence_id: 1,
                            delta_mm: [25.0, 0.0, 0.0],
                        },
                    ],
                    linear_arrays: Vec::new(),
                }),
            }))
            .unwrap();
        let context = egui::Context::default();

        app.poll_assistant_chat(&context);

        assert!(app.assistant_chat_task.is_none());
        assert!(app.assistant_pending_execution.is_some());
        assert_eq!(app.document.current().revision_id(), revision);

        app.poll_assistant_chat(&context);

        assert!(app.assistant_pending_execution.is_none());
        assert_eq!(app.document.current().revision_id(), revision);
        assert!(app.assistant_proposal.is_some());
        assert!(app.assistant_verification.is_none());
        assert!(app.assistant_messages.iter().any(|message| {
            message.role == AssistantMessageRole::Assistant && message.text == "Moved it."
        }));

        assert!(app.confirm_assistant_proposal());
        assert_eq!(app.document.current().revision_id(), revision + 1);
        assert!(app.assistant_verification.is_some());
    }

    #[test]
    fn assistant_replace_scene_clears_collection_references_in_the_same_undo_step() {
        let mut app = KetchupApp::new();
        let collection = CollectionId(1);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateCollection {
                    id: collection,
                    name: "Original selection".to_owned(),
                },
                CanonicalCommand::SetCollectionOccurrences {
                    id: collection,
                    occurrence_ids: vec![OccurrenceId(1)],
                },
            ]))
            .unwrap();
        let before = app.document.current();

        assert!(apply_reviewed_model_intent(
            &mut app,
            AssistantModelIntent {
                replace_scene: true,
                boxes: vec![AssistantBoxIntent {
                    name: "Replacement".to_owned(),
                    size_mm: [100.0, 80.0, 60.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                }],
                translations: Vec::new(),
                linear_arrays: Vec::new(),
            }
        ));

        let replaced = app.document.current();
        assert_eq!(replaced.revision_id(), before.revision_id() + 1);
        assert_eq!(replaced.occurrences().count(), 1);
        assert_eq!(replaced.definitions().count(), 1);
        assert_eq!(
            replaced
                .collection(collection)
                .unwrap()
                .occurrence_ids()
                .count(),
            0
        );
        assert!(app.undo());
        assert_eq!(
            app.document
                .current()
                .collection(collection)
                .unwrap()
                .occurrence_ids()
                .collect::<Vec<_>>(),
            vec![OccurrenceId(1)]
        );
        assert_eq!(
            app.document.current().canonical_digest(),
            before.canonical_digest()
        );
    }

    #[test]
    fn stale_assistant_model_result_is_reported_without_mutating_the_newer_document() {
        let mut app = KetchupApp::new();
        let request_document_id = app.document.current().document_id();
        let request_revision_id = app.document.current().revision_id();
        let request_digest = app.document.current().canonical_digest();
        let (sender, receiver) = mpsc::channel();
        let cancellation = AssistantCancellation::default();
        app.assistant_chat_task = Some(AssistantChatTask {
            receiver,
            cancellation,
            document_id: request_document_id,
            revision_id: request_revision_id,
            canonical_digest: request_digest,
            source: "test".to_owned(),
        });
        assert!(
            app.prepare_assistant_intent(WorkflowIntent::SetOccurrenceTranslation {
                target: OccurrenceId(1),
                x_mm_text: "10".to_owned(),
                y_mm_text: "0".to_owned(),
                z_mm_text: "0".to_owned(),
            })
        );
        assert!(app.confirm_assistant_proposal());
        let changed_revision = app.document.current().revision_id();
        let changed_digest = app.document.current().canonical_digest();
        let undo_steps = app.document.visible_undo_steps();
        sender
            .send(Ok(AssistantChatResult {
                message: "Moved it.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: Vec::new(),
                    translations: vec![
                        ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                            occurrence_id: 1,
                            delta_mm: [100.0, 0.0, 0.0],
                        },
                    ],
                    linear_arrays: Vec::new(),
                }),
            }))
            .unwrap();

        let context = egui::Context::default();
        app.poll_assistant_chat(&context);
        app.poll_assistant_chat(&context);

        assert_eq!(app.document.current().revision_id(), changed_revision);
        assert_eq!(app.document.current().canonical_digest(), changed_digest);
        assert_eq!(app.document.visible_undo_steps(), undo_steps);
        assert!(app.assistant_messages.iter().any(|message| {
            message.role == AssistantMessageRole::Error
                && message.text == app.catalog.text("assistant-error-stale-response")
        }));
    }

    #[test]
    fn assistant_conversation_round_trips_with_its_document() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chat-model.ketchup");
        let mut app = KetchupApp::new().with_dialogs(Box::new(
            dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
        ));
        app.assistant_messages = vec![
            AssistantChatMessage {
                role: AssistantMessageRole::User,
                text: "Posuň hranol o 100 mm.".to_owned(),
                source: "Codex OAuth · gpt-test".to_owned(),
            },
            AssistantChatMessage {
                role: AssistantMessageRole::Assistant,
                text: "Hranol som posunul.".to_owned(),
                source: "Codex OAuth · gpt-test".to_owned(),
            },
        ];

        assert!(app.save_document_to(&path));
        let mut reopened = KetchupApp::new().with_dialogs(Box::new(
            dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
        ));
        assert!(reopened.open_document_from(&path));
        assert_eq!(reopened.assistant_messages, app.assistant_messages);
        assert_eq!(reopened.document_path.as_deref(), Some(path.as_path()));

        reopened.new_assistant_chat();
        assert!(reopened.assistant_messages.is_empty());
        assert!(reopened.save_document_to(&path));
        let mut cleared = KetchupApp::new();
        assert!(cleared.open_document_from(&path));
        assert!(cleared.assistant_messages.is_empty());
    }

    // Palette contrast is proved once for all four appearances in
    // `theme::tests::every_palette_keeps_text_and_accent_legible`, so this file
    // no longer keeps a second copy of the thresholds for one hardcoded set.

    #[test]
    fn switching_theme_repaints_the_shell_without_touching_the_document() {
        let mut app = KetchupApp::new();
        let before_revision = app.document_revision();
        let before_digest = app.canonical_digest();
        let graphite = app.palette();

        for kind in ThemeKind::ALL {
            app.set_theme(kind);
            assert_eq!(app.theme(), kind);
            assert_eq!(app.palette(), Palette::of(kind));
            assert_eq!(
                app.document_revision(),
                before_revision,
                "changing appearance must not commit a canonical batch"
            );
            assert_eq!(
                app.canonical_digest(),
                before_digest,
                "changing appearance must not change the model"
            );
        }

        app.set_theme(ThemeKind::Graphite);
        assert_eq!(app.palette(), graphite);
        assert!(
            !app.can_undo(),
            "appearance must never enter the undo stack"
        );
    }

    fn lossy_legacy_document() -> Vec<u8> {
        let mut bytes = b"KETCHUPDOC".to_vec();
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&7_u64.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&42_u64.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.push(b'x');
        bytes.extend_from_slice(&3.5_f64.to_bits().to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes
    }

    fn current_bottle_package(
        app: &KetchupApp,
        definition_id: DefinitionId,
    ) -> Arc<ExactBodyPackage> {
        use ketchup_core::bottle_m6::{SHELL_FACE_ROLES, build_revolve_package};
        use ketchup_core::exact_product::canonical_reference_lineage_digest;

        let snapshot = app.document.current();
        let request = ExactRevolveRequest::from_snapshot(&snapshot, definition_id).unwrap();
        let points = request.points_mm();
        let max_radius = points.iter().map(|point| point[0]).fold(0.0_f64, f64::max);
        let evidence = SHELL_FACE_ROLES
            .map(|role| {
                (
                    role,
                    canonical_reference_lineage_digest(
                        request.document_id,
                        request.producer_feature_id(),
                        role.semantic_role(),
                        role.source_element_id(),
                        role.expected_type(),
                    ),
                    format!("geometry-{role:?}"),
                )
            })
            .to_vec();
        Arc::new(
            build_revolve_package(
                &request,
                "exact-input".to_owned(),
                "result".to_owned(),
                "OCCT-test".to_owned(),
                "linear=1e-7mm".to_owned(),
                [
                    [-max_radius, -max_radius, points[0][1]],
                    [max_radius, max_radius, points[5][1]],
                ],
                evidence,
            )
            .unwrap()
            .into(),
        )
    }

    fn through_cut_document() -> DocumentStore {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(10),
                    name: "Exact cut body".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(11),
                    definition_id: DefinitionId(10),
                    name: "Outer profile".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(12),
                    definition_id: DefinitionId(10),
                    name: "Base extrusion".to_owned(),
                    kind: FeatureKind::Extrusion {
                        profile: FeatureId(11),
                        height: Dimension::from_decimal("10").unwrap(),
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(13),
                    definition_id: DefinitionId(10),
                    name: "Cut profile".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(14),
                    definition_id: DefinitionId(10),
                    name: "Through cut".to_owned(),
                    kind: FeatureKind::ThroughCut {
                        target: FeatureId(12),
                        profile: FeatureId(13),
                    },
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(10),
                    definition_id: DefinitionId(10),
                    name: "Cut body occurrence".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        document.discard_history_before_current();
        document
    }

    fn exact_worker_executable() -> PathBuf {
        let executable_name = if cfg!(windows) {
            "ketchup-exact-worker.exe"
        } else {
            "ketchup-exact-worker"
        };
        std::env::current_exe()
            .unwrap()
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join(executable_name)
    }

    fn current_box_package(
        app: &KetchupApp,
    ) -> Arc<ketchup_core::exact_product::ExactRenderPackage> {
        use ketchup_core::exact_product::{
            build_box_render_package, canonical_reference_lineage_digest,
        };

        let snapshot = app.document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, INITIAL_BOX_DEFINITION)
            .expect("the default box has an exact request");
        let evidence = [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
        ]
        .map(|role| {
            (
                role,
                canonical_reference_lineage_digest(
                    request.document_id,
                    request.producer_feature_id(),
                    role.semantic_role(),
                    role.source_element_id(),
                    "planar_face",
                ),
                format!("geometry-{role:?}"),
            )
        });
        Arc::new(
            build_box_render_package(
                &request,
                "exact-input".to_owned(),
                "result".to_owned(),
                "backend".to_owned(),
                "tolerance".to_owned(),
                [[0.0; 3], request.dimensions_mm()],
                evidence,
            )
            .expect("the exact package matches the default box"),
        )
    }

    #[test]
    fn exact_bottle_can_start_preview_and_commit_the_standard_move_tool() {
        let mut app = KetchupApp::new();
        assert!(app.create_bottle());
        let definition_id = app.selected_bottle_definition().unwrap();
        let bottle_path = app.selection.occurrences.iter().next().unwrap().clone();
        let package = current_bottle_package(&app, definition_id);
        let snapshot = app.document.current();
        app.exact_results
            .insert_current(&snapshot, package)
            .unwrap();
        assert!(
            app.active_boxes()
                .iter()
                .all(|item| item.instance_path != bottle_path),
            "the exact bottle intentionally has no canonical box proxy"
        );

        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1_000.0, 800.0));
        let pointer = app.project(Vec3::new(120.0, 0.0, 50.0), rect);
        app.update_viewport_inference(Some(pointer), rect);
        assert_eq!(
            app.hovered
                .as_ref()
                .map(|selection| &selection.instance_path),
            Some(&bottle_path)
        );
        assert!(app.begin_move_drag_at(pointer, rect, false));
        let mut drag = app.move_drag.take().unwrap();
        drag.delta_mm = Vec3::new(25.0, 10.0, 0.0);
        let overrides = app.document.current().scene_query();
        app.move_drag = Some(drag.clone());
        let preview = app.move_preview_transform_overrides();
        let original = overrides
            .iter()
            .find(|occurrence| occurrence.instance_path == bottle_path)
            .unwrap()
            .transform;
        assert_eq!(
            preview[&bottle_path].matrix()[3],
            original.matrix()[3] + 25.0
        );
        assert_eq!(
            preview[&bottle_path].matrix()[7],
            original.matrix()[7] + 10.0
        );
        let render_snapshot = app.document.current();
        let render_plan = InstancedRenderPlan::from_snapshot_with_transform_overrides(
            &render_snapshot,
            &app.exact_results,
            &mut app.render_cache,
            &preview,
        );
        let bottle_instance = &render_plan
            .batches()
            .iter()
            .find(|batch| batch.definition_id == definition_id)
            .unwrap()
            .instances[0];
        assert_eq!(
            bottle_instance.transform[3],
            (original.matrix()[3] + 25.0) as f32
        );
        assert_eq!(
            bottle_instance.transform[7],
            (original.matrix()[7] + 10.0) as f32
        );

        app.move_drag = None;
        assert!(app.commit_move_drag(&drag));
        let moved = app
            .document
            .current()
            .world_transform_for_occurrence(bottle_path.root_occurrence())
            .unwrap();
        assert_eq!(moved.matrix()[3], original.matrix()[3] + 25.0);
        assert_eq!(moved.matrix()[7], original.matrix()[7] + 10.0);
    }

    #[test]
    fn bottle_numeric_workflow_is_atomic_and_round_trips_losslessly() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("editable-bottle.ketchup");
        let mut app = KetchupApp::new();
        let undo_before_create = app.document.visible_undo_steps();

        assert!(app.create_bottle());
        assert_eq!(app.document.visible_undo_steps(), undo_before_create + 1);
        let definition_id = app.selected_bottle_definition().unwrap();
        let revision_before_edit = app.document.current().revision_id();
        let undo_before_edit = app.document.visible_undo_steps();

        assert!(app.set_bottle_parameters(&BottleEditorInputs {
            definition_id,
            body_radius: "34 mm".to_owned(),
            body_height: "125".to_owned(),
            shoulder_rise: "16.5".to_owned(),
            thickness: "2.5".to_owned(),
            finish_amount: "1.5".to_owned(),
            finish_kind: BottleEdgeFinishKind::Chamfer,
        }));
        assert_ne!(app.document.current().revision_id(), revision_before_edit);
        assert_eq!(app.document.visible_undo_steps(), undo_before_edit + 1);
        let edited = app.document.current();
        let ids = KetchupApp::bottle_feature_ids(&edited, definition_id).unwrap();
        let FeatureKind::BottleProfileControl {
            body_radius,
            body_height,
            shoulder_rise,
            ..
        } = edited.feature(ids.control).unwrap().kind()
        else {
            panic!("bottle control feature missing");
        };
        assert_eq!(body_radius.source_token(), "34 mm");
        assert_eq!(body_height.millimetres(), 125.0);
        assert_eq!(shoulder_rise.millimetres(), 16.5);
        assert!(matches!(
            edited.feature(ids.finish).unwrap().kind(),
            FeatureKind::BottleEdgeFinish {
                kind: BottleEdgeFinishKind::Chamfer,
                ..
            }
        ));

        let digest_before_rejection = edited.canonical_digest();
        let revision_before_rejection = edited.revision_id();
        let undo_before_rejection = app.document.visible_undo_steps();
        assert!(!app.set_bottle_parameters(&BottleEditorInputs {
            definition_id,
            body_radius: "34".to_owned(),
            body_height: "125".to_owned(),
            shoulder_rise: "16.5".to_owned(),
            thickness: "7".to_owned(),
            finish_amount: "1.5".to_owned(),
            finish_kind: BottleEdgeFinishKind::Fillet,
        }));
        assert_eq!(
            app.document.current().canonical_digest(),
            digest_before_rejection
        );
        assert_eq!(
            app.document.current().revision_id(),
            revision_before_rejection
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before_rejection);

        assert!(app.save_document_to(&path));
        let expected = app.document.current();
        let mut reopened = KetchupApp::new();
        assert!(reopened.open_document_from(&path));
        let actual = reopened.document.current();
        assert_eq!(actual.canonical_digest(), expected.canonical_digest());
        assert!(ExactRevolveRequest::from_snapshot(&actual, definition_id).is_ok());
        assert!(KetchupApp::bottle_editor_inputs(&actual, definition_id).is_some());
    }

    #[test]
    fn accepted_bottle_result_drives_render_pick_authority_and_fail_closed_exports() {
        let directory = tempfile::tempdir().unwrap();
        let exact_path = directory.path().join("bottle.kbex");
        let mesh_path = directory.path().join("bottle.obj");
        let stale_path = directory.path().join("stale.kbex");
        let mut app = KetchupApp::new().with_dialogs(Box::new(
            dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(61),
        ));
        assert!(app.create_bottle());
        let definition_id = app.selected_bottle_definition().unwrap();
        let package = current_bottle_package(&app, definition_id);
        let snapshot = app.document.current();
        app.exact_results
            .insert_current(&snapshot, package)
            .unwrap();

        let report = app.bottle_authority_report(definition_id).unwrap();
        assert!(report.current);
        assert!(report.validation_passed);
        assert_eq!(report.durable_reference_count, 9);
        assert_eq!(app.exact_render_body_count(), 1);
        assert_eq!(app.exact_stable_reference_count(), 9);
        let picked = app
            .exact_pick_durable(
                Ray::new(Vec3::new(130.0, 0.0, 50.0), Vec3::new(-1.0, 0.0, 0.0)).unwrap(),
            )
            .expect("accepted bottle mesh must remain exactly pickable");
        assert_eq!(picked.body.role(), Some(ExactFaceRole::ShellOuterBody));

        assert!(app.export_bottle_exact_recipe_to(definition_id, &exact_path));
        assert!(app.export_bottle_mesh_to(definition_id, &mesh_path));
        let exact = std::fs::read_to_string(&exact_path).unwrap();
        let mesh = std::fs::read_to_string(&mesh_path).unwrap();
        let loss = std::fs::read_to_string(mesh_path.with_extension("obj.loss.txt")).unwrap();
        assert!(exact.starts_with("KETCHUP_EXACT_BOTTLE_RECIPE_V1\n"));
        assert!(exact.contains("result_fingerprint=result"));
        assert!(mesh.contains("# authority=accepted exact OCCT B-Rep"));
        assert!(mesh.contains("g shell.outer.body"));
        assert!(loss.contains("editability_loss="));
        assert!(loss.contains("topology_loss="));
        assert!(loss.contains("tolerance_loss="));

        let ids = KetchupApp::bottle_feature_ids(&app.document.current(), definition_id).unwrap();
        let undo_before_drag = app.document.visible_undo_steps();
        assert!(app.commit_bottle_direct_drag(
            BottleDirectDrag {
                definition_id,
                feature_id: ids.control,
                control: BottleControlDimension::BodyRadius,
                pointer_start: Pos2::ZERO,
                value_start_mm: 30.0,
                screen_direction: Vec2::X,
                pixels_per_mm: 1.0,
            },
            33.0,
        ));
        assert_eq!(app.document.visible_undo_steps(), undo_before_drag + 1);
        assert_eq!(app.exact_render_body_count(), 0);
        assert!(!app.bottle_authority_report(definition_id).unwrap().current);
        assert!(!app.export_bottle_exact_recipe_to(definition_id, &stale_path));
        assert!(!stale_path.exists());
    }

    #[test]
    fn lossy_mesh_export_requires_payload_bound_receipt_before_any_artifact_write() {
        let directory = tempfile::tempdir().unwrap();
        let mesh_path = directory.path().join("protected.obj");
        let loss_path = mesh_path.with_extension("obj.loss.txt");
        let original_mesh = b"preserve mesh until approval".to_vec();
        let original_loss = b"preserve loss report until approval".to_vec();
        std::fs::write(&mesh_path, &original_mesh).unwrap();
        std::fs::write(&loss_path, &original_loss).unwrap();
        let script = dialogs::ScriptedFileDialogs::new()
            .queue_refused_high_risk()
            .queue_high_risk_approval(73);
        let mut app = KetchupApp::new().with_dialogs(Box::new(script.clone()));
        assert!(app.create_bottle());
        let definition_id = app.selected_bottle_definition().unwrap();
        let package = current_bottle_package(&app, definition_id);
        let snapshot = app.document.current();
        app.exact_results
            .insert_current(&snapshot, package)
            .unwrap();
        let canonical_before = app.document.current().canonical_digest();
        let revision_before = app.document.current().revision_id();
        let undo_before = app.document.visible_undo_steps();

        assert!(!app.export_bottle_mesh_to(definition_id, &mesh_path));
        assert_eq!(std::fs::read(&mesh_path).unwrap(), original_mesh);
        assert_eq!(std::fs::read(&loss_path).unwrap(), original_loss);
        assert!(app.last_side_effect_receipt().is_none());

        assert!(app.export_bottle_mesh_to(definition_id, &mesh_path));
        let receipt = app
            .last_side_effect_receipt()
            .expect("approved lossy export returns an authorization receipt");
        assert_eq!(receipt.approving_human(), 73);
        assert_eq!(receipt.revision_id(), revision_before);
        assert_eq!(receipt.operation(), "export-lossy-obj-with-loss-report");
        assert_eq!(receipt.scope().class(), HighRiskClass::LossyConversion);
        assert_eq!(
            receipt.scope().path(),
            Some(mesh_path.display().to_string().as_str())
        );
        assert_ne!(std::fs::read(&mesh_path).unwrap(), original_mesh);
        assert_ne!(std::fs::read(&loss_path).unwrap(), original_loss);
        assert_eq!(app.document.current().canonical_digest(), canonical_before);
        assert_eq!(app.document.current().revision_id(), revision_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);
        assert_eq!(script.high_risk_prompts().len(), 2);
        assert!(script.high_risk_prompts()[0].contains("Payload SHA-256:"));
    }

    #[test]
    fn current_exact_occurrence_suppresses_only_the_non_preview_proxy() {
        let mut app = KetchupApp::new();
        let package = current_box_package(&app);
        let snapshot = app.document.current();
        app.exact_results
            .insert_current(&snapshot, Arc::new((*package).clone().into()))
            .unwrap();
        let exact_projection = app.exact_projection(&snapshot);

        assert!(exact_projection.contains_occurrence(&InstancePath::root(OccurrenceId(1))));
        assert!(app.viewport_boxes(&exact_projection).is_empty());

        app.selection.primary = Some(SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        });
        app.set_push_pull_distance_input("5");
        assert!(app.start_preview());
        assert_eq!(app.viewport_boxes(&exact_projection).len(), 1);
    }

    #[test]
    fn exact_occurrence_reference_and_mesh_export_use_the_canonical_world_transform() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("transformed.obj");
        let mut app = KetchupApp::new().with_dialogs(Box::new(
            dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(62),
        ));
        let transform = Transform::from_matrix([
            0.0, -1.0, 0.0, 10.0, 1.0, 0.0, 0.0, 20.0, 0.0, 0.0, 1.0, 30.0, 0.0, 0.0, 0.0, 1.0,
        ])
        .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: OccurrenceId(1),
                    transform,
                },
            ]))
            .unwrap();
        let package = current_box_package(&app);
        let snapshot = app.document.current();
        app.exact_results
            .insert_current(&snapshot, Arc::new((*package).clone().into()))
            .unwrap();
        let instance_path = InstancePath::root(OccurrenceId(1));

        let reference = app
            .exact_reference_for_occurrence(&instance_path, ExactFaceRole::Top)
            .unwrap();
        assert_eq!(reference.instance_path, instance_path);
        assert_eq!(reference.body.role(), Some(ExactFaceRole::Top));
        assert!(app.export_exact_occurrence_mesh_to(&instance_path, &path));

        let mesh = std::fs::read_to_string(&path).unwrap();
        let first_vertex = mesh
            .lines()
            .find_map(|line| line.strip_prefix("v "))
            .unwrap()
            .split_whitespace()
            .map(|value| value.parse::<f64>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(first_vertex, vec![10.0, 20.0, 30.0]);
        assert!(mesh.contains("g extrusion.top"));
        let loss = std::fs::read_to_string(path.with_extension("obj.loss.txt")).unwrap();
        assert!(loss.contains("exact-body-to-world-space-mesh"));
        assert!(loss.contains("producer_feature_id=2"));
    }

    #[test]
    fn hovering_and_selecting_a_canonical_mesh_body_paints_its_outline() {
        let mut app = KetchupApp::new();
        assert!(apply_reviewed_model_intent(
            &mut app,
            AssistantModelIntent {
                replace_scene: true,
                boxes: vec![AssistantBoxIntent {
                    name: "Grooved beam".to_owned(),
                    size_mm: [400.0, 100.0, 100.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: vec![
                        ketchup_core::assistant_sidecar::AssistantSubtractionIntent {
                            size_mm: [40.0, 100.0, 30.0],
                            origin_mm: [180.0, 0.0, 70.0],
                        }
                    ],
                }],
                translations: Vec::new(),
                linear_arrays: Vec::new(),
            }
        ));
        let snapshot = app.document.current();
        let occurrence = snapshot.occurrences().next().unwrap().id();
        assert!(
            definition_mesh_body(&snapshot, snapshot.definitions().next().unwrap().id()).is_some(),
            "the subtracted box is stored as a canonical mesh body"
        );
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
        app.zoom_fit();

        let unselected = selection_stroke_segments(&context, &mut app);
        app.selection.select_occurrence(occurrence, false);
        let selected = selection_stroke_segments(&context, &mut app);

        assert_eq!(unselected, 0);
        assert!(
            selected > 0,
            "a selected canonical mesh body must paint its selection outline"
        );
    }

    fn selection_stroke_segments(context: &egui::Context, app: &mut KetchupApp) -> usize {
        let output = context.run(egui::RawInput::default(), |context| app.ui(context));
        let mut count = 0;
        for clipped in output.shapes {
            count_selection_segments(&clipped.shape, &mut count);
        }
        count
    }

    fn count_selection_segments(shape: &egui::Shape, count: &mut usize) {
        match shape {
            egui::Shape::LineSegment { stroke, .. } => {
                if stroke.color == Color32::from_rgb(240, 78, 35) {
                    *count += 1;
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    count_selection_segments(shape, count);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn running_app_uses_one_exact_cut_body_for_render_pick_and_export() {
        let executable = exact_worker_executable();
        assert!(
            executable.is_file(),
            "build workspace all-targets so the exact worker exists at {}",
            executable.display()
        );
        let mut app = KetchupApp::new().with_dialogs(Box::new(
            dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(63),
        ));
        app.document = through_cut_document();
        app.document
            .configure_human_confirmation_policy(app.confirmation_surface.verifying_key(), 1)
            .unwrap();
        app.reset_document_presentation();
        app.connect_exact_worker(&executable).unwrap();
        let before = app.document.current();
        let context = egui::Context::default();

        for _ in 0..200 {
            app.refresh_exact_products(&context);
            if app.exact_render_body_count() == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(app.exact_render_body_count(), 1);
        assert_eq!(app.document.current().revision_id(), before.revision_id());
        assert_eq!(
            app.document.current().canonical_digest(),
            before.canonical_digest()
        );
        let projection = app.exact_projection(&app.document.current());
        assert!(
            projection
                .exact_surface_pick(
                    Ray::new(Vec3::new(5.0, 5.0, 20.0), Vec3::new(0.0, 0.0, -1.0)).unwrap()
                )
                .is_none(),
            "the exact through-hole must not be filled by an axis-aligned proxy"
        );
        let wall = app
            .exact_pick_durable(
                Ray::new(Vec3::new(5.0, 5.0, 5.0), Vec3::new(1.0, 0.0, 0.0)).unwrap(),
            )
            .expect("the cut wall must remain durably pickable");
        assert_eq!(wall.body.role(), Some(ExactFaceRole::CutEast));

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("through-cut.obj");
        assert!(app.export_exact_occurrence_mesh_to(&InstancePath::root(OccurrenceId(10)), &path));
        let mesh = std::fs::read_to_string(&path).unwrap();
        assert!(mesh.contains("g through_cut.wall.east"));
        assert_eq!(
            mesh.lines().filter(|line| line.starts_with("f ")).count(),
            32
        );
        let loss = std::fs::read_to_string(path.with_extension("obj.loss.txt")).unwrap();
        assert!(loss.contains("authority=accepted exact OCCT B-Rep"));

        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: FeatureId(12),
                    dimension: Dimension::from_decimal("11").unwrap(),
                },
            ]))
            .unwrap();
        assert_eq!(app.exact_render_body_count(), 0);
        let stale_projection = app.exact_projection(&app.document.current());
        assert!(!stale_projection.contains_occurrence(&InstancePath::root(OccurrenceId(10))));
        assert!(app.viewport_boxes(&stale_projection).is_empty());
        let stale_path = directory.path().join("stale-through-cut.obj");
        assert!(
            !app.export_exact_occurrence_mesh_to(
                &InstancePath::root(OccurrenceId(10)),
                &stale_path
            )
        );
        assert!(!stale_path.exists());
    }

    #[test]
    fn orbit_passes_both_poles_without_a_pitch_limit() {
        let mut app = KetchupApp::new();

        app.orbit(Vec2::new(0.0, 400.0));
        assert!(app.pitch > 1.2);

        app.orbit(Vec2::new(0.0, -800.0));
        assert!(app.pitch < -1.2);
    }

    #[test]
    fn creating_a_second_box_has_stable_identity_and_undo_redo_visibility() {
        let mut app = KetchupApp::new();
        assert_eq!(app.active_box_count(), 1);

        assert!(app.create_box());
        assert_eq!(app.active_box_count(), 2);
        let created = app.selected_reference().unwrap();
        assert_eq!(created.definition_id, DefinitionId(2));
        assert_eq!(created.instance_path, InstancePath::root(OccurrenceId(2)));
        assert_eq!(app.box_height_mm(created.definition_id), Some(20.0));

        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let second_top = app.project(Vec3::new(125.0, 85.0, 20.0), rect);
        let picked = app.exact_pick_at_screen(second_top, rect).unwrap();
        assert_eq!(picked.definition_id, DefinitionId(2));
        assert_eq!(picked.instance_path, InstancePath::root(OccurrenceId(2)));

        assert!(app.undo());
        assert_eq!(app.active_box_count(), 1);
        assert_eq!(app.selected_reference(), None);

        assert!(app.redo());
        assert_eq!(app.active_box_count(), 2);
    }

    #[test]
    fn move_rotate_delete_are_independent_undoable_scene_operations() {
        let mut app = KetchupApp::new();
        let selected = SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        app.selection.primary = Some(selected);

        assert!(!app.move_selected(Vec3::ZERO));
        assert!(app.move_selected(Vec3::new(10.0, -5.0, 3.0)));
        assert_eq!(app.active_boxes()[0].origin_mm, Vec3::new(10.0, -5.0, 3.0));
        assert!(app.undo());
        assert_eq!(app.active_boxes()[0].origin_mm, Vec3::ZERO);
        assert!(app.redo());
        assert_eq!(app.active_boxes()[0].origin_mm, Vec3::new(10.0, -5.0, 3.0));

        assert!(app.rotate_selected_90());
        assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(60.0, 100.0, 20.0));
        assert_eq!(app.active_boxes()[0].origin_mm, Vec3::new(30.0, -25.0, 3.0));
        assert!(app.undo());
        assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(100.0, 60.0, 20.0));
        assert!(app.redo());
        assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(60.0, 100.0, 20.0));

        assert!(app.delete_selected());
        assert_eq!(app.active_box_count(), 0);
        assert_eq!(app.selected_reference(), None);
        assert!(app.undo());
        assert_eq!(app.active_box_count(), 1);
        assert_eq!(app.selected_reference(), None);
        assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(60.0, 100.0, 20.0));
        assert!(app.redo());
        assert_eq!(app.active_box_count(), 0);
        assert_eq!(app.selected_reference(), None);
    }

    #[test]
    fn push_pull_drag_is_signed_along_the_face_normal() {
        let drag = PushPullDrag {
            pointer_start: Pos2::new(100.0, 100.0),
            extent_start_mm: 20.0,
            screen_normal: Vec2::new(1.0, 0.0),
            pixels_per_mm: 2.0,
        };

        assert_eq!(
            push_pull_distance_from_pointer(drag, Pos2::new(120.0, 100.0)),
            10.0
        );
        assert_eq!(
            push_pull_distance_from_pointer(drag, Pos2::new(80.0, 100.0)),
            -10.0
        );
    }

    #[test]
    fn push_pull_distance_accepts_units_and_moves_inward() {
        let mut app = KetchupApp::new();
        select_initial_top_face(&mut app);
        app.set_push_pull_distance_input("-5 mm");

        assert!(app.start_preview());
        assert_eq!(app.preview_box.as_ref().unwrap().box_data.size_mm.z, 15.0);
        assert!(app.confirm_preview());
        assert_eq!(app.document_height_mm(), 15.0);
        assert!(app.undo());
        assert_eq!(app.document_height_mm(), 20.0);
    }

    #[test]
    fn push_pull_minimum_side_keeps_the_opposite_face_fixed() {
        let mut app = KetchupApp::new();
        app.selection.primary = Some(SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::X,
                side: Side::Minimum,
            },
        });
        let old_maximum = app.active_boxes()[0].origin_mm.x + app.active_boxes()[0].size_mm.x;

        app.set_push_pull_distance_input("30");
        assert!(app.start_preview());
        let preview = app.preview_box.as_ref().unwrap().box_data.clone();
        assert_eq!(preview.origin_mm.x, -30.0);
        assert_eq!(preview.size_mm.x, 130.0);
        assert_eq!(preview.origin_mm.x + preview.size_mm.x, old_maximum);

        assert!(app.confirm_preview());
        assert_eq!(app.active_boxes()[0], preview);
        assert!(app.undo());
        assert_eq!(
            app.active_boxes()[0].origin_mm.x + app.active_boxes()[0].size_mm.x,
            old_maximum
        );
        assert_eq!(app.active_boxes()[0].size_mm.x, 100.0);
    }

    #[test]
    fn assistant_evaluator_rename_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let node = NodeId(20);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateEvaluatorNode {
                    id: node,
                    name: "width".to_owned(),
                    dimension: Dimension::from_decimal("600").unwrap(),
                    dependencies: Vec::new(),
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::EvaluatorName;
        app.assistant_target_input = node.0.to_string();
        app.assistant_value_input = String::new();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);

        app.assistant_value_input = "cabinet width".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::RenameEvaluatorNode(node));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Text("width".to_owned())
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Text("cabinet width".to_owned())
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document.current().evaluator_node(node).unwrap().name(),
            "cabinet width"
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document.current().evaluator_node(node).unwrap().name(),
            "width"
        );
    }

    #[test]
    fn assistant_evaluator_expression_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let input = NodeId(20);
        let expression = NodeId(21);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateEvaluatorNode {
                    id: input,
                    name: "width".to_owned(),
                    dimension: Dimension::from_decimal("600").unwrap(),
                    dependencies: Vec::new(),
                },
                CanonicalCommand::CreateExpressionNode {
                    id: expression,
                    name: "double width".to_owned(),
                    expression: "$20 * 2".to_owned(),
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::EvaluatorExpression;
        app.assistant_target_input = expression.0.to_string();
        app.assistant_value_input = "(".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);

        app.assistant_value_input = "$20 * 3".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::SetEvaluatorExpression(expression)
        );
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Text("$20 * 2".to_owned())
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Text("$20 * 3".to_owned())
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document
                .current()
                .evaluator_node(expression)
                .unwrap()
                .kind()
                .source(),
            "$20 * 3"
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document
                .current()
                .evaluator_node(expression)
                .unwrap()
                .kind()
                .source(),
            "$20 * 2"
        );
    }

    #[test]
    fn assistant_tag_visibility_review_is_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let tag = TagId(7);
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
                id: tag,
                name: "Hardware".to_owned(),
                visible: true,
            }]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::TagVisibility;
        app.assistant_target_input = tag.0.to_string();
        app.assistant_value_input = "yes".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);

        assert!(
            app.prepare_assistant_intent(WorkflowIntent::SetTagVisibility {
                target: tag,
                visible: false,
            })
        );
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::SetTagVisibility(tag));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Boolean(true)
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Boolean(false)
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(!app.document.current().tag(tag).unwrap().visible());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().tag(tag).unwrap().visible());
    }

    #[test]
    fn assistant_occurrence_tag_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let tag = TagId(8);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateTag {
                    id: tag,
                    name: "Fixtures".to_owned(),
                    visible: true,
                },
                CanonicalCommand::SetOccurrenceTag {
                    id: OccurrenceId(1),
                    tag: Some(tag),
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::OccurrenceTag;
        app.assistant_target_input = "1".to_owned();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "none".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::SetOccurrenceTag(OccurrenceId(1))
        );
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Tag(Some(tag))
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Tag(None)
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document
                .current()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .tag(),
            None
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document
                .current()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .tag(),
            Some(tag)
        );
    }

    #[test]
    fn assistant_occurrence_repoint_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let definition = DefinitionId(9);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: "Alternate".to_owned(),
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::OccurrenceDefinition;
        app.assistant_target_input = "1".to_owned();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = definition.0.to_string();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::RepointOccurrence(OccurrenceId(1))
        );
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Definition(INITIAL_BOX_DEFINITION)
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Definition(definition)
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document
                .current()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .definition_id(),
            definition
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document
                .current()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .definition_id(),
            INITIAL_BOX_DEFINITION
        );
    }

    #[test]
    fn assistant_occurrence_parent_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let group = GroupId(10);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateGroup {
                    id: group,
                    name: "Assembly".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                },
                CanonicalCommand::SetOccurrenceParent {
                    id: OccurrenceId(1),
                    parent: Some(group),
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::OccurrenceParent;
        app.assistant_target_input = "1".to_owned();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "none".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::SetOccurrenceParent(OccurrenceId(1))
        );
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Group(Some(group))
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Group(None)
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document
                .current()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .parent(),
            None
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document
                .current()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .parent(),
            Some(group)
        );
    }

    #[test]
    fn assistant_group_parent_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let group = GroupId(10);
        let parent = GroupId(11);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateGroup {
                    id: group,
                    name: "Assembly".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                },
                CanonicalCommand::CreateGroup {
                    id: parent,
                    name: "Parent".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::GroupParent;
        app.assistant_target_input = group.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = parent.0.to_string();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::SetGroupParent(group));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Group(None)
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Group(Some(parent))
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document.current().group(group).unwrap().parent(),
            Some(parent)
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(app.document.current().group(group).unwrap().parent(), None);
    }

    #[test]
    fn assistant_group_translation_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let group = GroupId(10);
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
                id: group,
                name: "Assembly".to_owned(),
                transform: Transform::identity(),
                parent: None,
            }]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::GroupTranslation;
        app.assistant_target_input = group.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "4.5, -2, 11.25".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        let expected = Transform::from_translation(4.5, -2.0, 11.25).unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::SetGroupTranslation(group));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Transform(Transform::identity())
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Transform(expected)
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document.current().group(group).unwrap().transform(),
            expected
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document.current().group(group).unwrap().transform(),
            Transform::identity()
        );
    }

    #[test]
    fn assistant_bottle_control_dimension_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        assert!(app.create_bottle());
        let definition_id = app.selected_bottle_definition().unwrap();
        let control = KetchupApp::bottle_feature_ids(&app.document.current(), definition_id)
            .unwrap()
            .control;
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::BottleControlDimension;
        app.assistant_target_input = control.0.to_string();
        app.assistant_value_input = "waist=32".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);

        app.assistant_value_input = "body_radius=32".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::SetBottleControlDimension(control, BottleControlDimension::BodyRadius)
        );
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Dimension(Dimension::from_decimal("30").unwrap())
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Dimension(Dimension::from_decimal("32").unwrap())
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(matches!(
            app.document.current().feature(control).unwrap().kind(),
            FeatureKind::BottleProfileControl { body_radius, .. }
                if body_radius.millimetres() == 32.0
        ));
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(matches!(
            app.document.current().feature(control).unwrap().kind(),
            FeatureKind::BottleProfileControl { body_radius, .. }
                if body_radius.millimetres() == 30.0
        ));
    }

    #[test]
    fn assistant_bottle_finish_kind_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        assert!(app.create_bottle());
        let definition_id = app.selected_bottle_definition().unwrap();
        let finish = KetchupApp::bottle_feature_ids(&app.document.current(), definition_id)
            .unwrap()
            .finish;
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::BottleEdgeFinishKind;
        app.assistant_target_input = finish.0.to_string();
        app.assistant_value_input = "round".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);

        app.assistant_value_input = "chamfer".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::SetBottleEdgeFinishKind(finish)
        );
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::BottleEdgeFinishKind(BottleEdgeFinishKind::Fillet)
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::BottleEdgeFinishKind(BottleEdgeFinishKind::Chamfer)
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(matches!(
            app.document.current().feature(finish).unwrap().kind(),
            FeatureKind::BottleEdgeFinish {
                kind: BottleEdgeFinishKind::Chamfer,
                ..
            }
        ));
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(matches!(
            app.document.current().feature(finish).unwrap().kind(),
            FeatureKind::BottleEdgeFinish {
                kind: BottleEdgeFinishKind::Fillet,
                ..
            }
        ));
    }

    #[test]
    fn assistant_profile_points_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let definition = DefinitionId(50);
        let profile = FeatureId(51);
        let original = vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]];
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: "Assistant profile".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: profile,
                    definition_id: definition,
                    name: "Profile".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: original.clone(),
                    },
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::ProfilePoints;
        app.assistant_target_input = profile.0.to_string();
        app.assistant_value_input = "0,0; invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);

        let requested = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]];
        app.assistant_value_input = "0,0; 12,0; 12,8; 0,8".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::SetProfilePoints(profile));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::ProfilePoints(original.clone())
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::ProfilePoints(requested.clone())
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(matches!(
            app.document.current().feature(profile).unwrap().kind(),
            FeatureKind::Profile { points_mm } if points_mm == &requested
        ));
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(matches!(
            app.document.current().feature(profile).unwrap().kind(),
            FeatureKind::Profile { points_mm } if points_mm == &original
        ));
    }

    #[test]
    fn assistant_rule_outputs_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let rule = NodeId(30);
        let output = |key: &str| {
            RuleOutput::new(SlotSegment::new(rule, "result", key).unwrap(), Vec::new()).unwrap()
        };
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "layout".to_owned(),
                expression: "1".to_owned(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("result").unwrap()],
                outputs: vec![output("left")],
                override_parameters: Vec::new(),
            }]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::RuleOutputs;
        app.assistant_target_input = rule.0.to_string();
        app.assistant_value_input = "result".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);

        let requested = vec![output("center"), output("right")];
        app.assistant_value_input = "result:center; result:right".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::SetRuleOutputs(rule));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::RuleOutputs(vec![output("left")])
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::RuleOutputs(requested.clone())
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(matches!(
            app.document.current().evaluator_node(rule).unwrap().kind(),
            EvaluatorNodeKind::Rule { outputs, .. } if outputs == &requested
        ));
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(matches!(
            app.document.current().evaluator_node(rule).unwrap().kind(),
            EvaluatorNodeKind::Rule { outputs, .. } if outputs == &vec![output("left")]
        ));
    }

    #[test]
    fn assistant_create_tag_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let tag = TagId(24);
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateTag;
        app.assistant_target_input = tag.0.to_string();
        app.assistant_value_input = "visible:Reviewed".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "true:Reviewed tag".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateTag(tag));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::TagState {
                name: "Reviewed tag".to_owned(),
                visible: true,
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let snapshot = app.document.current();
        let created = snapshot.tag(tag).unwrap();
        assert_eq!(created.name(), "Reviewed tag");
        assert!(created.visible());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().tag(tag).is_none());
    }

    #[test]
    fn assistant_create_collection_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let collection = CollectionId(24);
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateCollection;
        app.assistant_target_input = collection.0.to_string();
        app.assistant_value_input = String::new();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "Reviewed selection".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateCollection(collection));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Text("Reviewed selection".to_owned())
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document
                .current()
                .collection(collection)
                .unwrap()
                .name(),
            "Reviewed selection"
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().collection(collection).is_none());
    }

    #[test]
    fn assistant_delete_collection_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let collection = CollectionId(24);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateCollection {
                    id: collection,
                    name: "Reviewed selection".to_owned(),
                },
                CanonicalCommand::SetCollectionOccurrences {
                    id: collection,
                    occurrence_ids: vec![OccurrenceId(1)],
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteCollection;
        app.assistant_target_input = collection.0.to_string();
        app.assistant_value_input.clear();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::DeleteCollection(collection));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::CollectionState {
                name: "Reviewed selection".to_owned(),
                occurrence_ids: vec![OccurrenceId(1)],
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().collection(collection).is_none());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        let snapshot = app.document.current();
        let restored = snapshot.collection(collection).unwrap();
        assert_eq!(restored.name(), "Reviewed selection");
        assert_eq!(
            restored.occurrence_ids().collect::<Vec<_>>(),
            vec![OccurrenceId(1)]
        );
    }

    #[test]
    fn assistant_delete_tag_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let tag = TagId(24);
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
                id: tag,
                name: "Reviewed tag".to_owned(),
                visible: false,
            }]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteTag;
        app.assistant_target_input = tag.0.to_string();
        app.assistant_value_input.clear();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::DeleteTag(tag));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::TagState {
                name: "Reviewed tag".to_owned(),
                visible: false,
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().tag(tag).is_none());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        let snapshot = app.document.current();
        let restored = snapshot.tag(tag).unwrap();
        assert_eq!(restored.name(), "Reviewed tag");
        assert!(!restored.visible());
    }

    #[test]
    fn assistant_delete_group_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let group = GroupId(24);
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
                id: group,
                name: "Reviewed group".to_owned(),
                transform: Transform::identity(),
                parent: None,
            }]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteGroup;
        app.assistant_target_input = group.0.to_string();
        app.assistant_value_input.clear();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::DeleteGroup(group));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::GroupState {
                name: "Reviewed group".to_owned(),
                transform: Transform::identity(),
                parent: None,
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().group(group).is_none());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        let snapshot = app.document.current();
        let restored = snapshot.group(group).unwrap();
        assert_eq!(restored.name(), "Reviewed group");
        assert_eq!(restored.transform(), Transform::identity());
        assert_eq!(restored.parent(), None);
    }

    #[test]
    fn assistant_delete_occurrence_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let occurrence = OccurrenceId(1);
        let snapshot = app.document.current();
        let existing = snapshot.occurrence(occurrence).unwrap();
        let expected_definition = existing.definition_id();
        let expected_name = existing.name().to_owned();
        let expected_transform = existing.transform();
        let expected_parent = existing.parent();
        let expected_tag = existing.tag();
        let expected_visible = existing.visible();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteOccurrence;
        app.assistant_target_input = occurrence.0.to_string();
        app.assistant_value_input.clear();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::DeleteOccurrence(occurrence));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::OccurrenceState {
                definition: expected_definition,
                name: expected_name.clone(),
                transform: expected_transform,
                parent: expected_parent,
                tag: expected_tag,
                visible: expected_visible,
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().occurrence(occurrence).is_none());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        let snapshot = app.document.current();
        let restored = snapshot.occurrence(occurrence).unwrap();
        assert_eq!(restored.definition_id(), expected_definition);
        assert_eq!(restored.name(), expected_name);
        assert_eq!(restored.transform(), expected_transform);
        assert_eq!(restored.parent(), expected_parent);
        assert_eq!(restored.tag(), expected_tag);
        assert_eq!(restored.visible(), expected_visible);
    }

    #[test]
    fn assistant_create_definition_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let definition = DefinitionId(24);
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateDefinition;
        app.assistant_target_input = definition.0.to_string();
        app.assistant_value_input = String::new();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "Reviewed component".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateDefinition(definition));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Text("Reviewed component".to_owned())
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document
                .current()
                .definition(definition)
                .unwrap()
                .name(),
            "Reviewed component"
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().definition(definition).is_none());
    }

    #[test]
    fn assistant_create_group_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let group = GroupId(24);
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateGroup;
        app.assistant_target_input = group.0.to_string();
        app.assistant_value_input = String::new();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "Reviewed root group".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateGroup(group));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::GroupState {
                name: "Reviewed root group".to_owned(),
                transform: Transform::identity(),
                parent: None,
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let snapshot = app.document.current();
        let created = snapshot.group(group).unwrap();
        assert_eq!(created.name(), "Reviewed root group");
        assert_eq!(created.transform(), Transform::identity());
        assert_eq!(created.parent(), None);
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().group(group).is_none());
    }

    #[test]
    fn assistant_create_occurrence_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let occurrence = OccurrenceId(24);
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateOccurrence;
        app.assistant_target_input = occurrence.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "1:Reviewed occurrence".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateOccurrence(occurrence));
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::OccurrenceState {
                definition: INITIAL_BOX_DEFINITION,
                name: "Reviewed occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let snapshot = app.document.current();
        let created = snapshot.occurrence(occurrence).unwrap();
        assert_eq!(created.definition_id(), INITIAL_BOX_DEFINITION);
        assert_eq!(created.name(), "Reviewed occurrence");
        assert_eq!(created.transform(), Transform::identity());
        assert_eq!(created.parent(), None);
        assert_eq!(created.tag(), None);
        assert!(created.visible());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().occurrence(occurrence).is_none());
    }

    #[test]
    fn assistant_create_profile_feature_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let feature = FeatureId(24);
        let points_mm = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
        let feature_ids_before = app
            .document
            .current()
            .definition(INITIAL_BOX_DEFINITION)
            .unwrap()
            .feature_ids()
            .to_vec();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateProfileFeature;
        app.assistant_target_input = feature.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "1:Reviewed profile:0,0;20,0;20,10;0,10".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateProfileFeature(feature));
        assert_eq!(proposal.authoritative_writes().len(), 2);
        let feature_diff = proposal
            .authoritative_diff()
            .iter()
            .find(|entry| {
                entry.target == ketchup_core::document::AuthoritativeDependency::Feature(feature)
            })
            .unwrap();
        assert_eq!(feature_diff.before, ProposalValue::Missing);
        assert_eq!(
            feature_diff.after,
            ProposalValue::ProfileFeatureState {
                definition: INITIAL_BOX_DEFINITION,
                name: "Reviewed profile".to_owned(),
                points_mm: points_mm.clone(),
            }
        );
        let definition_diff = proposal
            .authoritative_diff()
            .iter()
            .find(|entry| {
                entry.target
                    == ketchup_core::document::AuthoritativeDependency::Definition(
                        INITIAL_BOX_DEFINITION,
                    )
            })
            .unwrap();
        let mut feature_ids_after = feature_ids_before.clone();
        feature_ids_after.push(feature);
        assert_eq!(
            definition_diff.before,
            ProposalValue::DefinitionFeatures(feature_ids_before.clone())
        );
        assert_eq!(
            definition_diff.after,
            ProposalValue::DefinitionFeatures(feature_ids_after)
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let snapshot = app.document.current();
        let created = snapshot.feature(feature).unwrap();
        assert_eq!(created.definition_id(), INITIAL_BOX_DEFINITION);
        assert_eq!(created.name(), "Reviewed profile");
        assert!(matches!(
            created.kind(),
            FeatureKind::Profile { points_mm: created_points } if created_points == &points_mm
        ));
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().feature(feature).is_none());
        assert_eq!(
            app.document
                .current()
                .definition(INITIAL_BOX_DEFINITION)
                .unwrap()
                .feature_ids(),
            feature_ids_before
        );
    }

    #[test]
    fn assistant_delete_profile_feature_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let feature = FeatureId(24);
        let points_mm = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: feature,
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Reviewed profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: points_mm.clone(),
                },
            }]))
            .unwrap();
        let feature_ids_before = app
            .document
            .current()
            .definition(INITIAL_BOX_DEFINITION)
            .unwrap()
            .feature_ids()
            .to_vec();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteProfileFeature;
        app.assistant_target_input = feature.0.to_string();
        app.assistant_value_input.clear();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::DeleteProfileFeature(feature));
        assert_eq!(proposal.authoritative_writes().len(), 2);
        let feature_diff = proposal
            .authoritative_diff()
            .iter()
            .find(|entry| {
                entry.target == ketchup_core::document::AuthoritativeDependency::Feature(feature)
            })
            .unwrap();
        assert_eq!(
            feature_diff.before,
            ProposalValue::ProfileFeatureState {
                definition: INITIAL_BOX_DEFINITION,
                name: "Reviewed profile".to_owned(),
                points_mm: points_mm.clone(),
            }
        );
        assert_eq!(feature_diff.after, ProposalValue::Missing);
        let definition_diff = proposal
            .authoritative_diff()
            .iter()
            .find(|entry| {
                entry.target
                    == ketchup_core::document::AuthoritativeDependency::Definition(
                        INITIAL_BOX_DEFINITION,
                    )
            })
            .unwrap();
        let mut feature_ids_after = feature_ids_before.clone();
        feature_ids_after.retain(|candidate| *candidate != feature);
        assert_eq!(
            definition_diff.before,
            ProposalValue::DefinitionFeatures(feature_ids_before.clone())
        );
        assert_eq!(
            definition_diff.after,
            ProposalValue::DefinitionFeatures(feature_ids_after)
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().feature(feature).is_none());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        let snapshot = app.document.current();
        let restored = snapshot.feature(feature).unwrap();
        assert_eq!(restored.definition_id(), INITIAL_BOX_DEFINITION);
        assert_eq!(restored.name(), "Reviewed profile");
        assert_eq!(
            restored.kind(),
            &FeatureKind::Profile {
                points_mm: points_mm.clone(),
            }
        );
        assert_eq!(
            snapshot
                .definition(INITIAL_BOX_DEFINITION)
                .unwrap()
                .feature_ids(),
            feature_ids_before
        );
    }

    #[test]
    fn assistant_create_evaluator_input_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let target = NodeId(99);
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateEvaluatorInput;
        app.assistant_target_input = target.0.to_string();
        app.assistant_value_input = "missing delimiter".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "Reviewed depth:42.5".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateEvaluatorInput(target));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::EvaluatorInputState {
                name: "Reviewed depth".to_owned(),
                dimension: Dimension::from_decimal("42.5").unwrap(),
                dependencies: Vec::new(),
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let snapshot = app.document.current();
        let created = snapshot.evaluator_node(target).unwrap();
        assert_eq!(created.name(), "Reviewed depth");
        assert_eq!(
            created.dimension(),
            Some(&Dimension::from_decimal("42.5").unwrap())
        );
        assert!(created.dependencies().is_empty());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().evaluator_node(target).is_none());
    }

    #[test]
    fn assistant_create_evaluator_expression_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateEvaluatorNode {
                    id: NodeId(1),
                    name: "Reviewed source".to_owned(),
                    dimension: Dimension::from_decimal("21").unwrap(),
                    dependencies: Vec::new(),
                },
            ]))
            .unwrap();
        let target = NodeId(100);
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateEvaluatorExpression;
        app.assistant_target_input = target.0.to_string();
        app.assistant_value_input = "missing delimiter".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "Reviewed double:$1 * 2".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::CreateEvaluatorExpression(target)
        );
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::EvaluatorExpressionState {
                name: "Reviewed double".to_owned(),
                expression: "$1 * 2".to_owned(),
                dependencies: vec![NodeId(1)],
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let snapshot = app.document.current();
        let created = snapshot.evaluator_node(target).unwrap();
        assert_eq!(created.name(), "Reviewed double");
        assert_eq!(created.kind().source(), "$1 * 2");
        assert_eq!(created.dependencies(), &[NodeId(1)]);
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().evaluator_node(target).is_none());
    }

    #[test]
    fn assistant_create_evaluator_rule_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateEvaluatorNode {
                    id: NodeId(1),
                    name: "Reviewed source".to_owned(),
                    dimension: Dimension::from_decimal("21").unwrap(),
                    dependencies: Vec::new(),
                },
            ]))
            .unwrap();
        let target = NodeId(100);
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateEvaluatorRule;
        app.assistant_target_input = target.0.to_string();
        app.assistant_value_input = "missing delimiter".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "Reviewed rule:$1 * 2".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateEvaluatorRule(target));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::EvaluatorRuleState {
                name: "Reviewed rule".to_owned(),
                expression: "$1 * 2".to_owned(),
                dependencies: vec![NodeId(1)],
                input_ports: Vec::new(),
                output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
                outputs: Vec::new(),
                override_parameters: Vec::new(),
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let snapshot = app.document.current();
        let created = snapshot.evaluator_node(target).unwrap();
        assert_eq!(created.name(), "Reviewed rule");
        assert_eq!(created.kind().source(), "$1 * 2");
        assert_eq!(created.dependencies(), &[NodeId(1)]);
        assert!(created.input_ports().is_empty());
        assert_eq!(
            created.output_ports(),
            &[ketchup_core::document::PortSpec::number("result").unwrap()]
        );
        assert!(created.allowed_parameters().is_empty());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().evaluator_node(target).is_none());
    }

    #[test]
    fn assistant_create_rule_override_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let rule = NodeId(101);
        let target = 102;
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "Reviewed override source".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(rule, "result", "left").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
            }]))
            .unwrap();
        let identity = DerivedIdentity::new(
            rule,
            SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
        )
        .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateRuleOverride;
        app.assistant_target_input = target.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "101:result:left:offset:2.5".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateRuleOverride(target));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::RuleOverrideState {
                target: identity.clone(),
                parameter: "offset".to_owned(),
                value: 2.5,
                health: SlotResolution::Resolved,
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let snapshot = app.document.current();
        let created = snapshot.override_by_id(target).unwrap();
        assert_eq!(created.target, identity);
        assert_eq!(created.parameter, "offset");
        assert_eq!(created.value(), 2.5);
        assert_eq!(created.health, SlotResolution::Resolved);
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().override_by_id(target).is_none());
    }

    #[test]
    fn assistant_create_feature_parameter_binding_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let definition = DefinitionId(200);
        let profile = FeatureId(201);
        let feature = FeatureId(202);
        let rule = NodeId(203);
        let target = FeatureParameterTarget {
            feature_id: feature,
            slot: FeatureParameterSlot::Height,
        };
        let derived_from = DerivedIdentity::new(
            rule,
            SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
        )
        .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: "Bound box".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: profile,
                    definition_id: definition,
                    name: "Bound profile".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: feature,
                    definition_id: definition,
                    name: "Bound extrusion".to_owned(),
                    kind: FeatureKind::Extrusion {
                        profile,
                        height: Dimension::from_decimal("20").unwrap(),
                    },
                },
                CanonicalCommand::CreateRuleNode {
                    id: rule,
                    name: "Binding source".to_owned(),
                    expression: "1".to_owned(),
                    input_ports: Vec::new(),
                    output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
                    outputs: vec![
                        RuleOutput::new(
                            SlotSegment::new(rule, "result", "left").unwrap(),
                            Vec::new(),
                        )
                        .unwrap(),
                    ],
                    override_parameters: Vec::new(),
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateFeatureParameterBinding;
        app.assistant_target_input = feature.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "height:203:result:left".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::CreateFeatureParameterBinding(target)
        );
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::FeatureParameterBindingState {
                target,
                derived_from: derived_from.clone(),
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document
                .current()
                .feature_parameter_binding(target)
                .unwrap()
                .derived_from,
            derived_from
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(
            app.document
                .current()
                .feature_parameter_binding(target)
                .is_none()
        );
    }

    #[test]
    fn assistant_delete_feature_parameter_binding_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let definition = DefinitionId(204);
        let profile = FeatureId(205);
        let feature = FeatureId(206);
        let rule = NodeId(207);
        let target = FeatureParameterTarget {
            feature_id: feature,
            slot: FeatureParameterSlot::Height,
        };
        let derived_from = DerivedIdentity::new(
            rule,
            SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
        )
        .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: "Bound box deletion".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: profile,
                    definition_id: definition,
                    name: "Bound profile deletion".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: feature,
                    definition_id: definition,
                    name: "Bound extrusion deletion".to_owned(),
                    kind: FeatureKind::Extrusion {
                        profile,
                        height: Dimension::from_decimal("20").unwrap(),
                    },
                },
                CanonicalCommand::CreateRuleNode {
                    id: rule,
                    name: "Binding deletion source".to_owned(),
                    expression: "1".to_owned(),
                    input_ports: Vec::new(),
                    output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
                    outputs: vec![
                        RuleOutput::new(
                            SlotSegment::new(rule, "result", "left").unwrap(),
                            Vec::new(),
                        )
                        .unwrap(),
                    ],
                    override_parameters: Vec::new(),
                },
                CanonicalCommand::UpsertFeatureParameterBinding(
                    ketchup_core::document::FeatureParameterBinding {
                        target,
                        derived_from: derived_from.clone(),
                    },
                ),
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteFeatureParameterBinding;
        app.assistant_target_input = feature.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "height".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::DeleteFeatureParameterBinding(target)
        );
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::FeatureParameterBindingState {
                target,
                derived_from: derived_from.clone(),
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(
            app.document
                .current()
                .feature_parameter_binding(target)
                .is_none()
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document
                .current()
                .feature_parameter_binding(target)
                .unwrap()
                .derived_from,
            derived_from
        );
    }

    #[test]
    fn assistant_recompute_feature_parameter_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let definition = DefinitionId(208);
        let profile = FeatureId(209);
        let feature = FeatureId(210);
        let rule = NodeId(211);
        let target = FeatureParameterTarget {
            feature_id: feature,
            slot: FeatureParameterSlot::Height,
        };
        let derived_from = DerivedIdentity::new(
            rule,
            SlotPath::new(vec![SlotSegment::new(rule, "result", "height").unwrap()]).unwrap(),
        )
        .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: "Recomputed box".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: profile,
                    definition_id: definition,
                    name: "Recomputed profile".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: feature,
                    definition_id: definition,
                    name: "Recomputed extrusion".to_owned(),
                    kind: FeatureKind::Extrusion {
                        profile,
                        height: Dimension::from_decimal("20").unwrap(),
                    },
                },
                CanonicalCommand::CreateRuleNode {
                    id: rule,
                    name: "Recompute source".to_owned(),
                    expression: "42".to_owned(),
                    input_ports: Vec::new(),
                    output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
                    outputs: vec![
                        RuleOutput::new(
                            SlotSegment::new(rule, "result", "height").unwrap(),
                            Vec::new(),
                        )
                        .unwrap(),
                    ],
                    override_parameters: Vec::new(),
                },
                CanonicalCommand::UpsertFeatureParameterBinding(
                    ketchup_core::document::FeatureParameterBinding {
                        target,
                        derived_from,
                    },
                ),
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::RecomputeFeatureParameter;
        app.assistant_target_input = feature.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "height".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::RecomputeFeatureParameter(target)
        );
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Dimension(Dimension::from_decimal("20").unwrap())
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Dimension(Dimension::from_decimal("42").unwrap())
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(matches!(
            app.document.current().feature(feature).unwrap().kind(),
            FeatureKind::Extrusion { height, .. }
                if height.source_token() == "42" && height.millimetres() == 42.0
        ));
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(matches!(
            app.document.current().feature(feature).unwrap().kind(),
            FeatureKind::Extrusion { height, .. }
                if height.source_token() == "20" && height.millimetres() == 20.0
        ));
    }

    #[test]
    fn assistant_clone_profile_definition_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let source_definition = DefinitionId(300);
        let source_feature = FeatureId(301);
        let occurrence = OccurrenceId(302);
        let new_definition = DefinitionId(303);
        let new_feature = FeatureId(304);
        let points_mm = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 6.0]];
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: source_definition,
                    name: "Clone source".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: source_feature,
                    definition_id: source_definition,
                    name: "Source profile".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: points_mm.clone(),
                    },
                },
                CanonicalCommand::CreateOccurrence {
                    id: occurrence,
                    definition_id: source_definition,
                    name: "Clone occurrence".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CloneProfileDefinitionAndRepoint;
        app.assistant_target_input = occurrence.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.canonical_digest(), digest_before);

        app.assistant_value_input = format!(
            "{}:{}:{}:{}:Independent profile",
            source_definition.0, source_feature.0, new_definition.0, new_feature.0
        );
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::CloneProfileDefinitionAndRepoint(occurrence)
        );
        assert_eq!(proposal.authoritative_writes().len(), 3);
        let feature_diff = proposal
            .authoritative_diff()
            .iter()
            .find(|entry| {
                entry.target
                    == ketchup_core::document::AuthoritativeDependency::Feature(new_feature)
            })
            .unwrap();
        assert_eq!(feature_diff.before, ProposalValue::Missing);
        assert_eq!(
            feature_diff.after,
            ProposalValue::ProfileFeatureState {
                definition: new_definition,
                name: "Source profile".to_owned(),
                points_mm: points_mm.clone(),
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document
                .current()
                .occurrence(occurrence)
                .unwrap()
                .definition_id(),
            new_definition
        );
        assert!(matches!(
            app.document.current().feature(new_feature).unwrap().kind(),
            FeatureKind::Profile { points_mm: cloned } if cloned == &points_mm
        ));
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document
                .current()
                .occurrence(occurrence)
                .unwrap()
                .definition_id(),
            source_definition
        );
        assert!(app.document.current().definition(new_definition).is_none());
        assert!(app.document.current().feature(new_feature).is_none());
    }

    #[test]
    fn assistant_convert_empty_group_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let group = GroupId(300);
        let new_definition = DefinitionId(301);
        let new_occurrence = OccurrenceId(302);
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
                id: group,
                name: "Reviewed empty group".to_owned(),
                transform: Transform::from_translation(1.0, 2.0, 3.0).unwrap(),
                parent: None,
            }]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::ConvertEmptyGroupToComponent;
        app.assistant_target_input = group.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.canonical_digest(), digest_before);

        app.assistant_value_input = format!(
            "{}:{}:Reviewed component",
            new_definition.0, new_occurrence.0
        );
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::ConvertEmptyGroupToComponent(group)
        );
        assert_eq!(proposal.authoritative_writes().len(), 3);
        let group_diff = proposal
            .authoritative_diff()
            .iter()
            .find(|entry| {
                entry.target == ketchup_core::document::AuthoritativeDependency::GroupSubtree(group)
            })
            .unwrap();
        assert!(matches!(
            group_diff.before,
            ProposalValue::GroupState { ref name, .. } if name == "Reviewed empty group"
        ));
        assert_eq!(group_diff.after, ProposalValue::Missing);
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().group(group).is_none());
        assert!(app.document.current().definition(new_definition).is_some());
        assert_eq!(
            app.document
                .current()
                .occurrence(new_occurrence)
                .unwrap()
                .transform(),
            Transform::from_translation(1.0, 2.0, 3.0).unwrap()
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().group(group).is_some());
        assert!(app.document.current().definition(new_definition).is_none());
        assert!(app.document.current().occurrence(new_occurrence).is_none());
    }

    #[test]
    fn assistant_create_joint_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let rule = NodeId(500);
        let target = JointId(222);
        let output = |key| {
            RuleOutput::new(SlotSegment::new(rule, "result", key).unwrap(), Vec::new()).unwrap()
        };
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "joint participants".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![PortSpec::number("result").unwrap()],
                outputs: vec![output("left"), output("right")],
                override_parameters: Vec::new(),
            }]))
            .unwrap();
        let participant = |key| {
            DerivedIdentity::new(
                rule,
                SlotPath::new(vec![SlotSegment::new(rule, "result", key).unwrap()]).unwrap(),
            )
            .unwrap()
        };
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateJoint;
        app.assistant_target_input = target.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert_eq!(app.canonical_digest(), digest_before);

        app.assistant_value_input = "500,result,left:500,result,right:1,2,3:4,5,6".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateJoint(target));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::JointState {
                participant_a: participant("left"),
                participant_b: participant("right"),
                volume_min: [1.0, 2.0, 3.0],
                volume_max: [4.0, 5.0, 6.0],
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let joint = ketchup_core::prismatic::CanonicalJoint::new(
            target,
            participant("left"),
            participant("right"),
            ketchup_core::prismatic::Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0])
                .unwrap(),
        )
        .unwrap();
        assert_eq!(app.document.current().joint(target), Some(&joint));
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().joint(target).is_none());
    }

    #[test]
    fn assistant_delete_joint_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let target = JointId(212);
        let participant = |key| {
            DerivedIdentity::new(
                NodeId(213),
                SlotPath::new(vec![SlotSegment::new(NodeId(213), "result", key).unwrap()]).unwrap(),
            )
            .unwrap()
        };
        let joint = ketchup_core::prismatic::CanonicalJoint::new(
            target,
            participant("left"),
            participant("right"),
            ketchup_core::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 2.0, 3.0])
                .unwrap(),
        )
        .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertJoint(
                joint.clone(),
            )]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteJoint;
        app.assistant_target_input = target.0.to_string();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::DeleteJoint(target));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::JointState {
                participant_a: joint.participant_a().clone(),
                participant_b: joint.participant_b().clone(),
                volume_min: [0.0, 0.0, 0.0],
                volume_max: [1.0, 2.0, 3.0],
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().joint(target).is_none());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(app.document.current().joint(target), Some(&joint));
    }

    #[test]
    fn assistant_create_space_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let target = SpaceId(219);
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateSpace;
        app.assistant_target_input = target.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert_eq!(app.canonical_digest(), digest_before);

        app.assistant_value_input = "maintenance access:1,2,3:4,5,6".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateSpace(target));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::SpaceState {
                purpose: "maintenance access".to_owned(),
                volume_min: [1.0, 2.0, 3.0],
                volume_max: [4.0, 5.0, 6.0],
                adjacent_to: Vec::new(),
                accessible_to: Vec::new(),
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let space = ketchup_core::space::CanonicalSpace::new(
            target,
            "maintenance access",
            ketchup_core::prismatic::Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0])
                .unwrap(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(app.document.current().space(target), Some(&space));
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().space(target).is_none());
    }

    #[test]
    fn assistant_create_clearance_volume_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let owner = SpaceId(220);
        let target = ClearanceVolumeId(221);
        let space = ketchup_core::space::CanonicalSpace::new(
            owner,
            "equipment",
            ketchup_core::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [5.0, 5.0, 5.0])
                .unwrap(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
                space,
            )]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreateClearanceVolume;
        app.assistant_target_input = target.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert_eq!(app.canonical_digest(), digest_before);

        app.assistant_value_input = "220:maintenance envelope:1,2,3:4,5,6:0.01:required".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::CreateClearanceVolume(target));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::ClearanceVolumeState {
                owner: ClearanceOwner::Space(owner),
                reason: "maintenance envelope".to_owned(),
                volume_min: [1.0, 2.0, 3.0],
                volume_max: [4.0, 5.0, 6.0],
                coordinate_frame: ketchup_core::space::ClearanceCoordinateFrame::World,
                tolerance_mm: 0.01,
                severity: ClearanceSeverity::Required,
                derived_from: None,
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let clearance = ketchup_core::space::CanonicalClearanceVolume::new(
            target,
            ClearanceOwner::Space(owner),
            "maintenance envelope",
            ketchup_core::prismatic::Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0])
                .unwrap(),
            TolerancePolicy::new(0.01).unwrap(),
            ClearanceSeverity::Required,
            None,
        )
        .unwrap();
        assert_eq!(
            app.document.current().clearance_volume(target),
            Some(&clearance)
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(app.document.current().clearance_volume(target).is_none());
    }

    #[test]
    fn assistant_delete_space_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let target = SpaceId(214);
        let space = ketchup_core::space::CanonicalSpace::new(
            target,
            "maintenance access",
            ketchup_core::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 2.0, 3.0])
                .unwrap(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
                space.clone(),
            )]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteSpace;
        app.assistant_target_input = target.0.to_string();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::DeleteSpace(target));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::SpaceState {
                purpose: "maintenance access".to_owned(),
                volume_min: [0.0, 0.0, 0.0],
                volume_max: [1.0, 2.0, 3.0],
                adjacent_to: Vec::new(),
                accessible_to: Vec::new(),
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().space(target).is_none());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(app.document.current().space(target), Some(&space));
    }

    #[test]
    fn assistant_delete_clearance_volume_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let owner = SpaceId(215);
        let target = ClearanceVolumeId(216);
        let space = ketchup_core::space::CanonicalSpace::new(
            owner,
            "equipment",
            ketchup_core::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [5.0, 5.0, 5.0])
                .unwrap(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let clearance = ketchup_core::space::CanonicalClearanceVolume::new(
            target,
            ketchup_core::space::ClearanceOwner::Space(owner),
            "maintenance envelope",
            ketchup_core::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 2.0, 3.0])
                .unwrap(),
            ketchup_core::prismatic::TolerancePolicy::new(0.01).unwrap(),
            ketchup_core::space::ClearanceSeverity::Required,
            None,
        )
        .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::UpsertSpace(space),
                CanonicalCommand::UpsertClearanceVolume(clearance.clone()),
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteClearanceVolume;
        app.assistant_target_input = target.0.to_string();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::DeleteClearanceVolume(target));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::ClearanceVolumeState {
                owner: ketchup_core::space::ClearanceOwner::Space(owner),
                reason: "maintenance envelope".to_owned(),
                volume_min: [0.0, 0.0, 0.0],
                volume_max: [1.0, 2.0, 3.0],
                coordinate_frame: ketchup_core::space::ClearanceCoordinateFrame::World,
                tolerance_mm: 0.01,
                severity: ketchup_core::space::ClearanceSeverity::Required,
                derived_from: None,
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().clearance_volume(target).is_none());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document.current().clearance_volume(target),
            Some(&clearance)
        );
    }

    #[test]
    fn assistant_delete_persistent_dimension_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let target = PersistentDimensionId(217);
        let dimension_target =
            PersistentDimensionTarget::FeatureParameter(FeatureParameterTarget {
                feature_id: FeatureId(2),
                slot: FeatureParameterSlot::ProfileWidth,
            });
        let presentation =
            DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 2).unwrap();
        let dimension = PersistentDimension::new(
            target,
            "Cabinet width",
            dimension_target.clone(),
            presentation,
        )
        .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::UpsertPersistentDimension(dimension.clone()),
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeletePersistentDimension;
        app.assistant_target_input = target.0.to_string();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::DeletePersistentDimension(target)
        );
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::PersistentDimensionState {
                name: "Cabinet width".to_owned(),
                target: dimension_target,
                presentation,
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(
            app.document
                .current()
                .persistent_dimension(target)
                .is_none()
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document.current().persistent_dimension(target),
            Some(&dimension)
        );
    }

    #[test]
    fn assistant_create_persistent_dimension_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let target = PersistentDimensionId(218);
        let dimension_target = FeatureParameterTarget {
            feature_id: FeatureId(2),
            slot: FeatureParameterSlot::Height,
        };
        let presentation =
            DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 2).unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CreatePersistentDimension;
        app.assistant_target_input = target.0.to_string();
        app.assistant_value_input = "invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert_eq!(app.canonical_digest(), digest_before);

        app.assistant_value_input = "Reviewed height:2:height:cm:2".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::CreatePersistentDimension(target)
        );
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Missing
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::PersistentDimensionState {
                name: "Reviewed height".to_owned(),
                target: PersistentDimensionTarget::FeatureParameter(dimension_target),
                presentation,
            }
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        let dimension = PersistentDimension::new(
            target,
            "Reviewed height",
            PersistentDimensionTarget::FeatureParameter(dimension_target),
            presentation,
        )
        .unwrap();
        assert_eq!(
            app.document.current().persistent_dimension(target),
            Some(&dimension)
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert!(
            app.document
                .current()
                .persistent_dimension(target)
                .is_none()
        );
    }

    #[test]
    fn assistant_delete_rule_override_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let rule = NodeId(101);
        let target = 102;
        let identity = DerivedIdentity::new(
            rule,
            SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
        )
        .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateRuleNode {
                    id: rule,
                    name: "Reviewed override source".to_owned(),
                    expression: "1".to_owned(),
                    input_ports: Vec::new(),
                    output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
                    outputs: vec![
                        RuleOutput::new(
                            SlotSegment::new(rule, "result", "left").unwrap(),
                            Vec::new(),
                        )
                        .unwrap(),
                    ],
                    override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
                },
                CanonicalCommand::UpsertOverride(
                    ketchup_core::document::CanonicalOverride::new(
                        target,
                        identity.clone(),
                        "offset",
                        2.5,
                        SlotResolution::Resolved,
                    )
                    .unwrap(),
                ),
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteRuleOverride;
        app.assistant_target_input = target.to_string();
        app.assistant_value_input.clear();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::DeleteRuleOverride(target));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::RuleOverrideState {
                target: identity.clone(),
                parameter: "offset".to_owned(),
                value: 2.5,
                health: SlotResolution::Resolved,
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().override_by_id(target).is_none());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        let restored = app
            .document
            .current()
            .override_by_id(target)
            .unwrap()
            .clone();
        assert_eq!(restored.target, identity);
        assert_eq!(restored.parameter, "offset");
        assert_eq!(restored.value(), 2.5);
        assert_eq!(restored.health, SlotResolution::Resolved);
    }

    #[test]
    fn assistant_delete_definition_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let definition = DefinitionId(99);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: "Reviewed empty definition".to_owned(),
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::DeleteDefinition;
        app.assistant_target_input = definition.0.to_string();
        app.assistant_value_input.clear();

        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(proposal.goal(), ProposalGoal::DeleteDefinition(definition));
        assert_eq!(proposal.authoritative_writes().len(), 1);
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::DefinitionState {
                name: "Reviewed empty definition".to_owned(),
                feature_ids: Vec::new(),
                local_occurrence_ids: Vec::new(),
                local_group_ids: Vec::new(),
            }
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Missing
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert!(app.document.current().definition(definition).is_none());
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        let snapshot = app.document.current();
        let restored = snapshot.definition(definition).unwrap();
        assert_eq!(restored.name(), "Reviewed empty definition");
        assert!(restored.feature_ids().is_empty());
    }

    #[test]
    fn assistant_collection_membership_review_is_typed_observational_and_undoable() {
        let mut app = KetchupApp::new();
        let collection = CollectionId(12);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateCollection {
                    id: collection,
                    name: "Selection set".to_owned(),
                },
            ]))
            .unwrap();
        let revision_before = app.document_revision();
        let digest_before = app.canonical_digest();
        let undo_before = app.document.visible_undo_steps();
        app.assistant_intent_kind = AssistantIntentKind::CollectionOccurrences;
        app.assistant_target_input = collection.0.to_string();
        app.assistant_value_input = "1, invalid".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "1, 1".to_owned();
        assert!(!app.prepare_assistant_from_inputs());
        assert!(app.assistant_proposal().is_none());
        assert_eq!(app.document_revision(), revision_before);

        app.assistant_value_input = "1".to_owned();
        assert!(app.prepare_assistant_from_inputs());
        let proposal = app.assistant_proposal().unwrap();
        assert_eq!(
            proposal.goal(),
            ProposalGoal::SetCollectionOccurrences(collection)
        );
        assert_eq!(
            proposal.authoritative_diff()[0].before,
            ProposalValue::Occurrences(Vec::new())
        );
        assert_eq!(
            proposal.authoritative_diff()[0].after,
            ProposalValue::Occurrences(vec![OccurrenceId(1)])
        );
        assert_eq!(app.document_revision(), revision_before);
        assert_eq!(app.canonical_digest(), digest_before);
        assert_eq!(app.document.visible_undo_steps(), undo_before);

        assert!(app.confirm_assistant_proposal());
        assert_eq!(
            app.document
                .current()
                .collection(collection)
                .unwrap()
                .occurrence_ids()
                .collect::<Vec<_>>(),
            vec![OccurrenceId(1)]
        );
        assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
        assert!(app.undo());
        assert_eq!(
            app.document
                .current()
                .collection(collection)
                .unwrap()
                .occurrence_ids()
                .count(),
            0
        );
    }

    #[test]
    fn push_pull_uses_the_projected_feature_pair_and_preserves_local_profile_origin() {
        let mut app = KetchupApp::new();
        let offset_points = vec![[10.0, 20.0], [110.0, 20.0], [110.0, 80.0], [10.0, 80.0]];
        let unrelated_points = vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0]];
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetProfilePoints {
                    id: FeatureId(1),
                    points_mm: offset_points,
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: INITIAL_BOX_DEFINITION,
                    name: "Unrelated profile".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: unrelated_points.clone(),
                    },
                },
            ]))
            .unwrap();
        let selection = SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::X,
                side: Side::Maximum,
            },
        };
        app.selection.primary = Some(selection.clone());
        let item = app.active_boxes()[0].clone();
        assert_eq!(item.profile_feature_id, FeatureId(1));
        assert_eq!(item.extrusion_feature_id, Some(FeatureId(2)));
        assert_eq!(item.origin_mm, Vec3::new(10.0, 20.0, 0.0));
        assert!(
            push_pull_batch(
                &app.document.current(),
                &SelectionId {
                    definition_id: DefinitionId(999),
                    ..selection
                },
                &item,
                150.0,
                "150".to_owned(),
            )
            .is_none()
        );

        app.set_push_pull_distance_input("50");
        assert!(app.start_preview());
        assert!(app.confirm_preview());
        let snapshot = app.document.current();
        let FeatureKind::Profile { points_mm } = snapshot.feature(FeatureId(1)).unwrap().kind()
        else {
            panic!("linked profile must remain a profile");
        };
        assert_eq!(
            points_mm,
            &vec![[10.0, 20.0], [160.0, 20.0], [160.0, 80.0], [10.0, 80.0]]
        );
        let FeatureKind::Profile { points_mm } = snapshot.feature(FeatureId(3)).unwrap().kind()
        else {
            panic!("unrelated feature must remain a profile");
        };
        assert_eq!(points_mm, &unrelated_points);
    }

    #[test]
    fn push_pull_preview_fails_closed_after_the_source_revision_changes() {
        let mut app = KetchupApp::new();
        select_initial_top_face(&mut app);
        app.set_push_pull_distance_input("5");
        assert!(app.start_preview());
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: OccurrenceId(1),
                    transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
                },
            ]))
            .unwrap();
        let current = app.active_boxes()[0].clone();
        assert!(!app.has_preview());
        assert_eq!(app.render_box(current.clone()), current);
        assert!(!app.confirm_preview());
    }

    #[test]
    fn linear_pattern_preview_adds_virtual_viewport_occurrences_only() {
        let mut app = KetchupApp::new();
        let revision = app.document_revision();
        let digest = app.canonical_digest();
        assert!(app.preview_linear_pattern(OccurrenceId(1), Axis::Z, 50.0, 4));

        let snapshot = app.document.current();
        let exact_projection = app.exact_projection(&snapshot);
        let boxes = app.viewport_boxes(&exact_projection);
        assert_eq!(boxes.len(), 4);
        assert_eq!(boxes[3].origin_mm, Vec3::new(0.0, 0.0, 150.0));
        assert_eq!(app.active_box_count(), 1);
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
    }

    #[test]
    fn rectangle_sketch_creates_a_profile_then_push_pull_adds_the_extrusion() {
        let mut app = KetchupApp::new();

        assert!(
            app.complete_rectangle_sketch(Vec3::new(40.0, 25.0, 0.0), Vec3::new(-10.0, -5.0, 0.0),)
        );
        assert_eq!(app.active_box_count(), 2);
        let profile = app.active_boxes()[1].clone();
        assert_eq!(profile.origin_mm, Vec3::new(-10.0, -5.0, 0.0));
        assert_eq!(profile.size_mm, Vec3::new(50.0, 30.0, 0.0));
        assert_eq!(profile.extrusion_feature_id, None);
        let profile_digest = app.canonical_digest();

        app.set_push_pull_distance_input("30");
        assert!(app.start_preview());
        assert!(app.confirm_preview());
        let solid = app.active_boxes()[1].clone();
        assert_eq!(solid.size_mm, Vec3::new(50.0, 30.0, 30.0));
        assert!(solid.extrusion_feature_id.is_some());

        assert!(app.undo());
        assert_eq!(app.canonical_digest(), profile_digest);
        assert_eq!(app.active_boxes()[1].size_mm.z, 0.0);
        assert!(app.redo());
        assert_eq!(app.active_boxes()[1].size_mm.z, 30.0);
    }

    #[test]
    fn cut_through_adds_a_bounded_profile_to_the_selected_solid_as_one_undo_step() {
        let mut app = KetchupApp::new();
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );

        app.dispatch_command(AppCommand::CutThrough);
        assert_eq!(app.active_tool, ActiveTool::CutThrough);
        app.sketch_start = Some(Vec3::new(20.0, 15.0, 20.0));
        app.sketch_cursor = Some(Vec3::new(21.0, 16.0, 20.0));
        app.value_input = "30,20".to_owned();
        assert!(app.apply_value_input());

        let snapshot = app.document.current();
        assert!(matches!(
            snapshot.feature(FeatureId(3)).unwrap().kind(),
            FeatureKind::Profile { points_mm }
                if points_mm == &vec![[20.0, 15.0], [50.0, 15.0], [50.0, 35.0], [20.0, 35.0]]
        ));
        assert!(matches!(
            snapshot.feature(FeatureId(4)).unwrap().kind(),
            FeatureKind::ThroughCut {
                target: FeatureId(2),
                profile: FeatureId(3),
            }
        ));
        assert_eq!(app.document.visible_undo_steps(), 1);
        assert!(ExactFeatureChainRequest::from_snapshot(&snapshot, INITIAL_BOX_DEFINITION).is_ok());
        let reopened = ketchup_core::persistence::load(&ketchup_core::persistence::save(&snapshot))
            .unwrap()
            .snapshot();
        assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());
        assert!(matches!(
            reopened.feature(FeatureId(4)).unwrap().kind(),
            FeatureKind::ThroughCut { .. }
        ));

        assert!(app.undo());
        assert!(app.document.current().feature(FeatureId(3)).is_none());
        assert!(app.document.current().feature(FeatureId(4)).is_none());
        assert!(app.redo());
        assert!(matches!(
            app.document.current().feature(FeatureId(4)).unwrap().kind(),
            FeatureKind::ThroughCut { .. }
        ));
    }

    #[test]
    fn pocket_previews_then_commits_and_edits_depth_as_canonical_undo_steps() {
        let mut app = KetchupApp::new();
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        let original_digest = app.canonical_digest();

        app.dispatch_command(AppCommand::Pocket);
        app.sketch_start = Some(Vec3::new(20.0, 15.0, 20.0));
        app.sketch_cursor = Some(Vec3::new(21.0, 16.0, 20.0));
        app.value_input = "30,20".to_owned();
        assert!(app.apply_value_input());
        assert!(app.has_pocket_preview());
        assert_eq!(app.canonical_digest(), original_digest);
        assert!(!app.can_undo());

        app.value_input = "8".to_owned();
        assert!(app.apply_value_input());
        assert!(!app.has_pocket_preview());
        let snapshot = app.document.current();
        assert!(matches!(
            snapshot.feature(FeatureId(3)).unwrap().kind(),
            FeatureKind::Profile { points_mm }
                if points_mm == &vec![[20.0, 15.0], [50.0, 15.0], [50.0, 35.0], [20.0, 35.0]]
        ));
        assert!(matches!(
            snapshot.feature(FeatureId(4)).unwrap().kind(),
            FeatureKind::Pocket {
                target: FeatureId(2),
                profile: FeatureId(3),
                depth,
            } if depth.millimetres() == 8.0
        ));
        assert_eq!(app.document.visible_undo_steps(), 1);
        let reopened = ketchup_core::persistence::load(&ketchup_core::persistence::save(&snapshot))
            .unwrap()
            .snapshot();
        assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());

        assert!(app.set_selected_pocket_depth(12.0));
        assert_eq!(app.document.visible_undo_steps(), 2);
        assert!(matches!(
            app.document.current().feature(FeatureId(4)).unwrap().kind(),
            FeatureKind::Pocket { depth, .. } if depth.millimetres() == 12.0
        ));
        assert!(app.undo());
        assert!(matches!(
            app.document.current().feature(FeatureId(4)).unwrap().kind(),
            FeatureKind::Pocket { depth, .. } if depth.millimetres() == 8.0
        ));
        assert!(app.undo());
        assert_eq!(app.canonical_digest(), original_digest);
        assert!(app.redo());
        assert!(matches!(
            app.document.current().feature(FeatureId(4)).unwrap().kind(),
            FeatureKind::Pocket { depth, .. } if depth.millimetres() == 8.0
        ));
    }

    #[test]
    fn pocket_preview_fails_closed_for_invalid_or_stale_depth() {
        let mut app = KetchupApp::new();
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        app.dispatch_command(AppCommand::Pocket);
        let digest = app.canonical_digest();
        assert!(!app.prepare_pocket_preview(
            Vec3::new(20.0, 15.0, 20.0),
            Vec3::new(50.0, 35.0, 20.0),
            20.0,
        ));
        assert_eq!(app.canonical_digest(), digest);

        assert!(app.prepare_pocket_preview(
            Vec3::new(20.0, 15.0, 20.0),
            Vec3::new(50.0, 35.0, 20.0),
            8.0,
        ));
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: OccurrenceId(1),
                    transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
                },
            ]))
            .unwrap();
        assert!(!app.has_pocket_preview());
        assert!(!app.confirm_pocket_preview());
        assert!(app.document.current().feature(FeatureId(3)).is_none());
    }

    #[test]
    fn cut_through_rejects_a_profile_that_touches_the_target_boundary() {
        let mut app = KetchupApp::new();
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        app.dispatch_command(AppCommand::CutThrough);
        let digest = app.canonical_digest();

        assert!(!app.complete_rectangle_sketch(
            Vec3::new(0.0, 15.0, 20.0),
            Vec3::new(50.0, 35.0, 20.0),
        ));
        assert_eq!(app.canonical_digest(), digest);
        assert!(!app.can_undo());
    }

    #[test]
    fn cut_through_stays_disabled_for_an_exact_unsupported_offset_profile() {
        let mut app = KetchupApp::new();
        assert!(app.create_closed_polyline(vec![
            [10.0, 10.0],
            [80.0, 10.0],
            [80.0, 50.0],
            [10.0, 50.0],
        ]));
        app.set_push_pull_distance_input("20");
        assert!(app.start_preview());
        assert!(app.confirm_preview());
        let digest = app.canonical_digest();

        assert!(!app.command_enabled(AppCommand::CutThrough));
        app.dispatch_command(AppCommand::CutThrough);
        assert_ne!(app.active_tool, ActiveTool::CutThrough);
        assert_eq!(app.canonical_digest(), digest);
    }

    #[test]
    fn closed_polyline_path_preserves_points_and_rejects_invalid_input_atomically() {
        let mut app = KetchupApp::new();
        let points_mm = vec![
            [-12.5, 4.25],
            [80.0, 4.25],
            [95.5, 40.0],
            [35.0, 72.75],
            [-12.5, 40.0],
        ];
        assert!(app.create_closed_polyline(points_mm.clone()));
        let created = app.active_boxes()[1].clone();
        assert_eq!(created.extrusion_feature_id, None);
        assert_eq!(created.size_mm.z, 0.0);
        assert!(matches!(
            app.document
                .current()
                .feature(created.profile_feature_id)
                .unwrap()
                .kind(),
            FeatureKind::Profile { points_mm: stored } if stored == &points_mm
        ));

        let digest = app.canonical_digest();
        let revision = app.document_revision();
        assert!(!app.create_closed_polyline(vec![
            [0.0, 0.0],
            [0.0, 10.0],
            [10.0, 10.0],
            [10.0, 0.0],
        ]));
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.document_revision(), revision);
    }

    #[test]
    fn push_pull_keeps_the_opposite_face_fixed_on_screen() {
        let mut app = KetchupApp::new();
        select_initial_top_face(&mut app);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let bottom = Vec3::new(0.0, 0.0, 0.0);
        let top = Vec3::new(0.0, 0.0, app.document_height_mm());
        let bottom_before = app.project(bottom, rect);
        let top_before = app.project(top, rect);

        app.set_push_pull_distance_input("20");
        assert!(app.start_preview());

        assert_eq!(app.project(bottom, rect), bottom_before);
        assert_ne!(app.project(Vec3::new(0.0, 0.0, 40.0), rect), top_before);
    }

    #[test]
    fn confirmed_push_pull_can_be_undone_and_redone() {
        let mut app = KetchupApp::new();
        select_initial_top_face(&mut app);
        assert!(!app.can_undo());

        app.set_push_pull_distance_input("22");
        assert!(app.start_preview());
        assert!(app.confirm_preview());
        assert_eq!(app.document_height_mm(), 42.0);

        assert!(app.undo());
        assert_eq!(app.document_height_mm(), 20.0);
        assert!(app.can_redo());

        assert!(app.redo());
        assert_eq!(app.document_height_mm(), 42.0);
    }

    #[test]
    fn typed_push_pull_values_correct_the_last_one_instead_of_stacking() {
        let mut app = KetchupApp::new();
        let base_height = app.document_height_mm();
        app.active_tool = ActiveTool::PushPull;
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );

        let base_digest = app.canonical_digest();
        app.value_input = "20".to_owned();
        assert!(app.apply_value_input());
        let original_revision = app.document_revision();
        assert_eq!(app.document_height_mm(), base_height + 20.0);
        assert_eq!(app.document.visible_undo_steps(), 1);

        app.value_input = "25".to_owned();
        assert!(app.apply_value_input());
        assert_eq!(app.document_revision(), original_revision + 1);
        assert_eq!(app.document_height_mm(), base_height + 25.0);
        assert_eq!(app.document.visible_undo_steps(), 1);

        app.value_input = "0".to_owned();
        assert!(app.apply_value_input());
        assert_eq!(app.document_revision(), original_revision + 2);
        assert_eq!(app.document_height_mm(), base_height);
        assert_eq!(app.document.visible_undo_steps(), 1);

        assert!(app.undo());
        assert_eq!(app.document_height_mm(), base_height);
        assert_eq!(app.canonical_digest(), base_digest);
        assert!(!app.can_undo(), "corrections must stay one undo step");
    }

    #[test]
    fn rejected_push_pull_correction_preserves_the_last_valid_operation() {
        let mut app = KetchupApp::new();
        app.active_tool = ActiveTool::PushPull;
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        app.value_input = "20".to_owned();
        assert!(app.apply_value_input());
        let valid_height = app.document_height_mm();
        let valid_revision = app.document_revision();
        let valid_digest = app.canonical_digest();
        let valid_undo_steps = app.document.visible_undo_steps();

        app.value_input = "-100".to_owned();
        assert!(!app.apply_value_input());
        assert_eq!(app.document_height_mm(), valid_height);
        assert_eq!(app.document_revision(), valid_revision);
        assert_eq!(app.canonical_digest(), valid_digest);
        assert_eq!(app.document.visible_undo_steps(), valid_undo_steps);
        assert_eq!(
            app.last_push_pull
                .as_ref()
                .map(|operation| operation.canonical_digest.as_str()),
            Some(valid_digest.as_str())
        );
    }

    #[test]
    fn viewport_selection_keeps_group_commands_available() {
        let mut app = KetchupApp::new();
        app.select_all();
        assert!(app.copy_selection_to_clipboard());
        assert!(app.paste_clipboard());
        app.select_all();
        assert!(app.group_selected());

        let target = SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        app.clear_selection();
        app.select_from_viewport(Some(target), false);

        assert_eq!(app.selection_count(), 2);
        assert!(app.selection.primary.is_none());
        assert!(app.selected_group_id().is_some());
        assert!(app.command_enabled(AppCommand::Ungroup));
        assert!(app.command_enabled(AppCommand::MakeComponent));
    }

    #[test]
    fn moved_group_behaves_as_one_object_and_explodes_without_geometry_shift() {
        let mut app = KetchupApp::new();
        assert!(app.create_box());
        app.select_all();
        assert!(app.group_selected());
        let group_id = app.selection.selected_group.unwrap();
        let ids = [OccurrenceId(1), OccurrenceId(2)];
        let before = ids.map(|id| {
            app.document
                .current()
                .world_transform_for_occurrence(id)
                .unwrap()
        });
        let revision_before_move = app.document_revision();

        assert!(app.move_selected(Vec3::new(40.0, -20.0, 15.0)));
        assert_eq!(app.document_revision(), revision_before_move + 1);
        assert_eq!(app.selection.selected_group, Some(group_id));
        let moved = ids.map(|id| {
            app.document
                .current()
                .world_transform_for_occurrence(id)
                .unwrap()
        });
        for (before, moved) in before.into_iter().zip(moved) {
            assert_eq!(moved.matrix()[3], before.matrix()[3] + 40.0);
            assert_eq!(moved.matrix()[7], before.matrix()[7] - 20.0);
            assert_eq!(moved.matrix()[11], before.matrix()[11] + 15.0);
        }

        let moved = ids.map(|id| {
            app.document
                .current()
                .world_transform_for_occurrence(id)
                .unwrap()
        });
        assert!(app.ungroup_selected());
        assert_eq!(app.group_count(), 0);
        let exploded = ids.map(|id| {
            app.document
                .current()
                .world_transform_for_occurrence(id)
                .unwrap()
        });
        assert_eq!(exploded, moved);
    }

    #[test]
    fn gpu_projection_matches_cpu_projection_inside_callback_viewport() {
        let mut app = KetchupApp::new();
        let rect = Rect::from_min_size(Pos2::new(87.0, 163.0), Vec2::new(1_927.0, 1_184.0));

        for mode in [ProjectionMode::Perspective, ProjectionMode::Parallel] {
            app.projection_mode = mode;
            let matrix = app.world_to_clip(rect);
            for point in box_corners(BOX_WIDTH_MM, BOX_DEPTH_MM, app.document_height_mm()) {
                let clip_x = matrix[0] * point.x as f32
                    + matrix[4] * point.y as f32
                    + matrix[8] * point.z as f32
                    + matrix[12];
                let clip_y = matrix[1] * point.x as f32
                    + matrix[5] * point.y as f32
                    + matrix[9] * point.z as f32
                    + matrix[13];
                // The rasterizer divides by clip w before it maps to the
                // viewport, so the check has to divide as well or it would
                // only ever be valid for the parallel projection.
                let clip_w = matrix[3] * point.x as f32
                    + matrix[7] * point.y as f32
                    + matrix[11] * point.z as f32
                    + matrix[15];
                let gpu_screen = Pos2::new(
                    rect.center().x + (clip_x / clip_w) * rect.width() * 0.5,
                    rect.center().y - (clip_y / clip_w) * rect.height() * 0.5,
                );
                let cpu_screen = app.project(point, rect);
                assert!((gpu_screen - cpu_screen).length() < 0.01, "{mode:?}");
            }
        }
    }

    #[test]
    fn viewport_omits_edge_on_faces_that_collapse_to_a_line() {
        let mut app = KetchupApp::new();
        // Only a parallel projection collapses an edge-on face to a line; a
        // converging one always leaves a sliver of area.
        app.projection_mode = ProjectionMode::Parallel;
        app.yaw = std::f32::consts::FRAC_PI_2;
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let projected = box_corners(BOX_WIDTH_MM, BOX_DEPTH_MM, app.document_height_mm())
            .map(|point| app.project(point, rect));
        let forward = Vec3::new(
            -f64::from(app.yaw.sin() * app.pitch.sin()),
            -f64::from(app.yaw.cos() * app.pitch.sin()),
            -f64::from(app.pitch.cos()),
        );

        assert_eq!(
            box_faces()
                .into_iter()
                .filter(|face| {
                    face_is_visible(&face.element, forward)
                        && projected_face_has_area(face.corners, &projected)
                })
                .count(),
            2
        );
    }

    #[test]
    fn viewport_draws_only_the_three_camera_facing_box_faces() {
        let app = KetchupApp::new();
        let forward = Vec3::new(
            -f64::from(app.yaw.sin() * app.pitch.sin()),
            -f64::from(app.yaw.cos() * app.pitch.sin()),
            -f64::from(app.pitch.cos()),
        );

        assert_eq!(
            box_faces()
                .into_iter()
                .filter(|face| face_is_visible(&face.element, forward))
                .count(),
            3
        );
    }

    #[test]
    fn viewport_click_routes_through_exact_spatial_query() {
        let app = KetchupApp::new();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let selected = app.exact_pick_at_screen(rect.center(), rect).unwrap();
        assert_eq!(selected.definition_id, INITIAL_BOX_DEFINITION);
        assert_eq!(selected.instance_path, InstancePath::root(OccurrenceId(1)));
        assert_eq!(
            selected.element,
            ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            }
        );
    }

    #[test]
    fn picking_chooses_the_frontmost_body_across_mesh_and_box_geometry() {
        let mut app = KetchupApp::new();
        assert!(apply_reviewed_model_intent(
            &mut app,
            AssistantModelIntent {
                replace_scene: true,
                boxes: vec![
                    AssistantBoxIntent {
                        name: "Grooved behind".to_owned(),
                        size_mm: [100.0, 60.0, 20.0],
                        origin_mm: [0.0, 0.0, 0.0],
                        subtract_boxes: vec![
                            ketchup_core::assistant_sidecar::AssistantSubtractionIntent {
                                size_mm: [10.0, 60.0, 5.0],
                                origin_mm: [45.0, 0.0, 15.0],
                            }
                        ],
                    },
                    AssistantBoxIntent {
                        name: "Plain in front".to_owned(),
                        size_mm: [100.0, 60.0, 20.0],
                        origin_mm: [0.0, 0.0, 40.0],
                        subtract_boxes: Vec::new(),
                    },
                ],
                translations: Vec::new(),
                linear_arrays: Vec::new(),
            }
        ));
        app.projection_mode = ProjectionMode::Parallel;
        app.yaw = 0.0;
        app.pitch = 0.0;
        app.camera_target_z = 30.0;
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let pointer = app.project(Vec3::new(50.0, 30.0, 60.0), rect);

        let selected = app.exact_pick_at_screen(pointer, rect).unwrap();
        assert_eq!(selected.instance_path, InstancePath::root(OccurrenceId(3)));
    }

    #[test]
    fn repeated_large_scene_picks_reuse_revision_bound_spatial_indices() {
        let mut app = KetchupApp::new();
        let source = app
            .document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .clone();
        let commands = (2_u32..=480)
            .map(|id| CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(u64::from(id)),
                definition_id: source.definition_id(),
                name: format!("Stacked {id}"),
                transform: Transform::from_translation(
                    f64::from((id - 1) % 24) * 120.0,
                    0.0,
                    f64::from((id - 1) / 24) * 280.0,
                )
                .unwrap(),
                parent: None,
                tag: None,
                visible: true,
            })
            .collect();
        app.document
            .apply_batch(&CommandBatch::new(commands))
            .unwrap();
        app.zoom_fit();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
        let pointer = app.project(Vec3::new(50.0, 30.0, 20.0), rect);

        assert!(app.exact_pick_at_screen(pointer, rect).is_some());
        let cache_ptrs = app.interaction_projection_cache_ptrs().unwrap();
        for _ in 0..480 {
            assert!(app.exact_pick_at_screen(pointer, rect).is_some());
            assert_eq!(app.interaction_projection_cache_ptrs(), Some(cache_ptrs));
        }
    }

    #[test]
    fn parallel_view_picks_a_hundred_metre_body_after_zoom_fit() {
        let mut app = KetchupApp::new();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
        app.viewport_rect = Some(rect);
        assert!(app.create_box_at(Vec3::new(0.0, 200.0, 0.0), Vec3::new(100_000.0, 60.0, 20.0),));
        app.zoom_fit();
        app.refresh_camera_distance();
        let visible_top = app.project(Vec3::new(75_000.0, 230.0, 20.0), rect);

        let selected = app
            .exact_pick_at_screen(visible_top, rect)
            .expect("a visible point on a 100 m body must remain pickable");

        assert_eq!(selected.definition_id, DefinitionId(2));
        assert_eq!(selected.instance_path, InstancePath::root(OccurrenceId(2)));
    }

    #[test]
    fn viewport_picks_the_geometry_currently_shown_in_preview() {
        let mut app = KetchupApp::new();
        select_initial_top_face(&mut app);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        app.set_push_pull_distance_input("40");
        assert!(app.start_preview());
        let top_center = app.project(Vec3::new(50.0, 30.0, 60.0), rect);

        let selected = app.exact_pick_at_screen(top_center, rect).unwrap();

        assert_eq!(
            selected.element,
            ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            }
        );
    }

    #[test]
    fn outliner_and_viewport_share_multiselection_without_document_mutation() {
        let mut app = KetchupApp::new();
        assert!(app.create_box());
        let revision = app.document_revision();
        let outliner_ids = app
            .outliner_query()
            .into_iter()
            .flat_map(|definition| definition.occurrences)
            .map(|occurrence| occurrence.instance_path)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            outliner_ids,
            BTreeSet::from([
                InstancePath::root(OccurrenceId(1)),
                InstancePath::root(OccurrenceId(2)),
            ])
        );

        app.clear_selection();
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        assert!(app.selection.contains(&InstancePath::root(OccurrenceId(1))));
        assert_eq!(app.selection_count(), 1);

        app.select_from_viewport(
            Some(SelectionId {
                definition_id: DefinitionId(2),
                instance_path: InstancePath::root(OccurrenceId(2)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            }),
            true,
        );
        assert_eq!(app.selection_count(), 2);
        assert!(app.selection.contains(&InstancePath::root(OccurrenceId(1))));
        assert!(app.selection.contains(&InstancePath::root(OccurrenceId(2))));

        app.select_from_viewport(
            Some(SelectionId {
                definition_id: DefinitionId(2),
                instance_path: InstancePath::root(OccurrenceId(2)),
                element: ElementId::Face {
                    axis: Axis::X,
                    side: Side::Maximum,
                },
            }),
            true,
        );
        assert_eq!(app.selection_count(), 1);

        app.select_from_viewport(None, false);
        assert_eq!(app.selection_count(), 0);
        app.orbit(Vec2::new(18.0, -9.0));
        assert_eq!(app.document_revision(), revision);
    }

    #[test]
    fn shared_definition_push_pull_previews_each_occurrence_and_explains_impact() {
        let mut app = KetchupApp::new();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(1),
                    name: "Box-1 #2".to_owned(),
                    transform: Transform::from_translation(250.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        app.push_pull_distance_input = "60".to_owned();

        assert!(app.start_preview());
        let rendered = app
            .active_boxes()
            .into_iter()
            .map(|item| app.render_box(item))
            .collect::<Vec<_>>();
        assert_eq!(rendered[0].origin_mm, Vec3::ZERO);
        assert_eq!(rendered[1].origin_mm, Vec3::new(250.0, 0.0, 0.0));
        assert_eq!(rendered[0].size_mm.z, 80.0);
        assert_eq!(rendered[1].size_mm.z, 80.0);
        assert!(app.digest.contains("2 occurrence(s) follow"));

        app.cancel_preview();
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::X,
                    side: Side::Minimum,
                },
            },
            false,
        );
        app.push_pull_distance_input = "30".to_owned();
        assert!(app.start_preview());
        let rendered = app
            .active_boxes()
            .into_iter()
            .map(|item| app.render_box(item))
            .collect::<Vec<_>>();
        assert_eq!(rendered[0].origin_mm.x, -30.0);
        assert_eq!(rendered[1].origin_mm.x, 250.0);
        assert_eq!(rendered[0].size_mm.x, 130.0);
        assert_eq!(rendered[1].size_mm.x, 130.0);
    }

    #[test]
    fn exact_rectangle_and_push_pull_are_atomic_undo_steps() {
        let mut app = KetchupApp::new();
        app.dispatch_command(AppCommand::Rectangle);
        app.sketch_start = Some(Vec3::new(40.0, 30.0, 20.0));
        app.sketch_cursor = Some(Vec3::new(20.0, 10.0, 20.0));
        app.value_input = "300,200".to_owned();

        assert!(app.apply_value_input());
        assert_eq!(app.active_box_count(), 2);
        let created = app.active_boxes()[1].clone();
        assert_eq!(created.origin_mm, Vec3::new(-260.0, -170.0, 20.0));
        assert_eq!(created.size_mm, Vec3::new(300.0, 200.0, 0.0));
        assert_eq!(app.document.visible_undo_steps(), 1);

        app.dispatch_command(AppCommand::PushPull);
        app.value_input = "55".to_owned();
        assert!(app.apply_value_input());
        assert_eq!(app.active_boxes()[1].size_mm.z, 55.0);
        assert_eq!(app.document.visible_undo_steps(), 2);

        assert!(app.undo());
        assert_eq!(app.active_boxes()[1].size_mm.z, 0.0);
        assert!(app.undo());
        assert_eq!(app.active_box_count(), 1);
        assert!(app.redo());
        assert!(app.redo());
        assert_eq!(app.active_boxes()[1].size_mm.z, 55.0);
    }

    #[test]
    fn move_and_ctrl_copy_commit_occurrence_only_batches_visible_in_outliner() {
        let mut app = KetchupApp::new();
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        let definition_count = app.document.current().definitions().count();

        assert!(app.move_selected(Vec3::new(30.0, 20.0, 0.0)));
        assert_eq!(app.document.visible_undo_steps(), 1);
        assert_eq!(app.outliner_query()[0].occurrences[0].position, "30,20");
        assert_eq!(
            app.document.current().definitions().count(),
            definition_count
        );

        assert!(app.copy_selected(Vec3::new(50.0, 0.0, 0.0)));
        let snapshot = app.document.current();
        assert_eq!(snapshot.definitions().count(), definition_count);
        assert_eq!(snapshot.occurrences().count(), 2);
        assert_eq!(
            snapshot
                .occurrence(OccurrenceId(1))
                .unwrap()
                .definition_id(),
            snapshot
                .occurrence(OccurrenceId(2))
                .unwrap()
                .definition_id()
        );
        assert_eq!(snapshot.scene_query()[0].shared_occurrence_count, 2);
        assert_eq!(
            app.selected_move_reference().unwrap().instance_path,
            InstancePath::root(OccurrenceId(2))
        );
        assert_eq!(app.outliner_query()[0].occurrences[1].position, "80,20");
        assert_eq!(app.document.visible_undo_steps(), 2);

        assert!(app.undo());
        assert_eq!(app.active_box_count(), 1);
        assert!(app.redo());
        assert_eq!(app.active_box_count(), 2);
        assert_eq!(app.outliner_query()[0].occurrences[1].position, "80,20");
    }

    #[test]
    fn move_vcb_accepts_last_direction_distance_and_exact_vector() {
        let mut app = KetchupApp::new();
        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        app.dispatch_command(AppCommand::Move);
        assert!(app.move_selected(Vec3::new(30.0, 40.0, 0.0)));
        app.value_input = "100 mm".to_owned();
        assert!(app.apply_value_input());
        let transform = app
            .document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .transform();
        assert_eq!(transform.matrix()[3], 60.0);
        assert_eq!(transform.matrix()[7], 80.0);
        assert_eq!(app.document.visible_undo_steps(), 2);

        app.value_input = "10,-20,5".to_owned();
        assert!(app.apply_value_input());
        let transform = app
            .document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .transform();
        assert_eq!(transform.matrix()[3], 70.0);
        assert_eq!(transform.matrix()[7], 60.0);
        assert_eq!(transform.matrix()[11], 5.0);
    }

    #[test]
    fn adaptive_grid_keeps_metric_lines_readable_across_camera_scales() {
        assert_eq!(adaptive_grid_step(8.0), 10.0);
        assert_eq!(adaptive_grid_step(0.01), 5_000.0);
        assert_eq!(adaptive_grid_step(0.000_01), 5_000_000.0);
        for scale in [8.0, 1.0, 0.01, 0.000_01] {
            let screen_spacing = adaptive_grid_step(scale) * scale;
            assert!(screen_spacing >= 32.0);
            assert!(screen_spacing <= 80.0);
        }
    }

    #[test]
    fn gpu_scene_is_painted_after_the_ground_grid() {
        let mut app = KetchupApp::new();
        let snapshot = app.document.current();
        let plan = Arc::new(InstancedRenderPlan::from_snapshot(
            &snapshot,
            &app.exact_results,
            &mut app.render_cache,
        ));
        let context = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let output = context.run(egui::RawInput::default(), |context| {
            let painter = context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("scene-base-layer-order"),
            ));
            app.paint_scene_base_layers(&painter, rect, Some(Arc::clone(&plan)));
        });

        assert!(output.shapes.len() > 1);
        assert!(matches!(
            output.shapes.last().map(|shape| &shape.shape),
            Some(egui::Shape::Callback(_))
        ));
    }

    #[test]
    fn adjacent_projected_triangles_share_a_fill_underlay_and_keep_antialiased_outlines() {
        let app = KetchupApp::new();
        let selection = SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        let faces = vec![
            ProjectedFace {
                selection: selection.clone(),
                polygon: ProjectedPolygon::Triangle([
                    Pos2::new(10.0, 10.0),
                    Pos2::new(110.0, 10.0),
                    Pos2::new(10.0, 110.0),
                ]),
                color: Color32::GRAY,
                depth: 0.0,
                previewed: false,
                out_of_context: false,
            },
            ProjectedFace {
                selection,
                polygon: ProjectedPolygon::Triangle([
                    Pos2::new(110.0, 10.0),
                    Pos2::new(110.0, 110.0),
                    Pos2::new(10.0, 110.0),
                ]),
                color: Color32::GRAY,
                depth: 0.0,
                previewed: false,
                out_of_context: false,
            },
        ];
        let context = egui::Context::default();
        let output = context.run(egui::RawInput::default(), |context| {
            let painter = context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("projected-face-fill"),
            ));
            app.paint_projected_faces(&painter, &faces);
        });

        assert_eq!(output.shapes.len(), 3);
        let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
            panic!("projected faces must start with one shared mesh underlay");
        };
        assert_eq!(underlay.vertices.len(), 6);
        assert_eq!(underlay.indices.len(), 6);
        assert!(
            output.shapes[1..]
                .iter()
                .all(|shape| matches!(shape.shape, egui::Shape::Path(_)))
        );
    }

    #[test]
    fn move_drag_snaps_to_grid_and_shift_constrains_dominant_axis() {
        let start = Vec3::new(3.0, 4.0, 20.0);
        assert_eq!(
            snapped_move_delta(start, Vec3::new(31.0, 28.0, 20.0), false),
            Vec3::new(30.0, 20.0, 0.0)
        );
        assert_eq!(
            snapped_move_delta(start, Vec3::new(31.0, 28.0, 20.0), true),
            Vec3::new(30.0, 0.0, 0.0)
        );
    }

    #[test]
    fn group_and_ungroup_preserve_world_placement_as_atomic_batches() {
        let mut app = KetchupApp::new();
        assert!(app.create_box());
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
        let before = app
            .document
            .current()
            .scene_query()
            .into_iter()
            .map(|item| (item.occurrence_id, item.transform))
            .collect::<BTreeMap<_, _>>();
        let undo_steps = app.document.visible_undo_steps();

        assert!(app.group_selected());
        let group_id = app.selection.selected_group.unwrap();
        let grouped = app.document.current();
        assert_eq!(app.document.visible_undo_steps(), undo_steps + 1);
        assert_eq!(grouped.groups().count(), 1);
        assert_eq!(
            grouped.occurrence(OccurrenceId(1)).unwrap().parent(),
            Some(group_id)
        );
        assert_eq!(
            grouped.occurrence(OccurrenceId(2)).unwrap().parent(),
            Some(group_id)
        );
        assert_eq!(
            grouped
                .scene_query()
                .into_iter()
                .map(|item| (item.occurrence_id, item.transform))
                .collect::<BTreeMap<_, _>>(),
            before
        );

        assert!(app.undo());
        assert_eq!(app.document.current().groups().count(), 0);
        assert!(app.redo());
        assert!(app.select_group(group_id));
        assert!(app.ungroup_selected());
        assert_eq!(app.document.current().groups().count(), 0);
        assert_eq!(
            app.document
                .current()
                .scene_query()
                .into_iter()
                .map(|item| (item.occurrence_id, item.transform))
                .collect::<BTreeMap<_, _>>(),
            before
        );
        assert!(app.undo());
        assert_eq!(app.document.current().groups().count(), 1);
        assert!(app.redo());
        assert_eq!(app.document.current().groups().count(), 0);
    }

    #[test]
    fn component_copies_keep_distinct_nested_paths_and_composed_world_positions() {
        let mut app = KetchupApp::new();
        assert!(app.create_box());
        let before = app
            .active_boxes()
            .into_iter()
            .map(|item| item.origin_mm)
            .collect::<Vec<_>>();
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
        assert!(app.group_selected());
        assert!(app.make_component());

        let converted = app.active_boxes();
        assert_eq!(
            converted
                .iter()
                .map(|item| item.origin_mm)
                .collect::<Vec<_>>(),
            before
        );
        assert!(converted.iter().all(|item| !item.instance_path.is_root()));
        let component_path = app.selected_move_reference().unwrap().instance_path;
        assert!(component_path.is_root());
        assert!(app.copy_selected(Vec3::new(200.0, 0.0, 0.0)));

        let boxes = app.active_boxes();
        let paths = boxes
            .iter()
            .map(|item| item.instance_path.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(paths.len(), 4);
        assert_eq!(
            paths
                .iter()
                .map(InstancePath::root_occurrence)
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        let mut expected = before.clone();
        expected.extend(
            before
                .iter()
                .map(|origin| *origin + Vec3::new(200.0, 0.0, 0.0)),
        );
        let actual = boxes.iter().map(|item| item.origin_mm).collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn nested_edits_require_a_matching_snapshot_bound_context() {
        let mut app = KetchupApp::new();
        assert!(app.create_box());
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
        assert!(app.group_selected());
        assert!(app.make_component());
        let nested = app.active_boxes()[0].clone();
        let selection = SelectionId {
            definition_id: nested.definition_id,
            instance_path: nested.instance_path.clone(),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        app.selection.select_exact(selection.clone(), false);
        let before_revision = app.document_revision();
        let before_digest = app.document.current().canonical_digest();
        app.set_push_pull_distance_input("5");
        assert!(!app.start_preview());
        assert!(!app.move_selected(Vec3::new(10.0, 0.0, 0.0)));
        assert_eq!(app.document_revision(), before_revision);
        assert_eq!(app.document.current().canonical_digest(), before_digest);

        let component_path = InstancePath::root(nested.instance_path.root_occurrence());
        assert!(app.enter_occurrence_context(component_path));
        app.selection.select_exact(selection.clone(), false);
        assert!(app.start_preview());
        assert!(app.preview_action_digest().is_some());
        assert!(app.confirm_preview());
        let committed_digest = app.document.current().canonical_digest();
        assert_ne!(committed_digest, before_digest);
        assert_eq!(app.document_revision(), before_revision + 1);
        assert!(app.undo());
        assert_eq!(app.document.current().canonical_digest(), before_digest);
        assert!(app.redo());
        assert_eq!(app.document.current().canonical_digest(), committed_digest);

        app.selection.select_exact(selection, false);
        app.set_push_pull_distance_input("5");
        assert!(app.start_preview());
        let stale_revision = app.document_revision();
        let stale_digest = app.document.current().canonical_digest();
        assert!(app.exit_edit_context());
        assert!(!app.confirm_preview());
        assert_eq!(app.document_revision(), stale_revision);
        assert_eq!(app.document.current().canonical_digest(), stale_digest);
    }

    #[test]
    fn edit_context_blocks_selection_leakage_and_exits_one_level_at_a_time() {
        let mut app = KetchupApp::new();
        assert!(app.create_box());
        assert!(app.create_box());
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
        assert!(app.group_selected());
        let group_id = app.selection.selected_group.unwrap();

        app.clear_selection();
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        assert_eq!(app.selection.selected_group, Some(group_id));
        assert!(app.enter_occurrence_context(InstancePath::root(OccurrenceId(1))));
        assert_eq!(
            app.selection.edit_context,
            vec![EditContext::Group(group_id)]
        );

        app.select_from_outliner(InstancePath::root(OccurrenceId(3)), false);
        assert_eq!(app.selection_count(), 0);
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        assert_eq!(app.selection_count(), 1);
        assert!(app.enter_occurrence_context(InstancePath::root(OccurrenceId(1))));
        assert!(matches!(
            app.selection.edit_context.last(),
            Some(EditContext::Definition {
                definition_id: DefinitionId(1),
                instance_path,
            }) if *instance_path == InstancePath::root(OccurrenceId(1))
        ));

        app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
        assert_eq!(app.selection_count(), 0);
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        assert_eq!(app.selection_count(), 1);
        app.clear_selection();
        assert!(app.exit_edit_context());
        assert_eq!(
            app.selection.edit_context,
            vec![EditContext::Group(group_id)]
        );
        assert!(app.exit_edit_context());
        assert!(app.selection.edit_context.is_empty());
    }

    #[test]
    fn organized_component_hierarchy_round_trips_with_stable_identity() {
        let mut app = KetchupApp::new();
        app.selection.select_exact(
            SelectionId {
                definition_id: DefinitionId(1),
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        assert!(app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
        assert!(app.group_selected());
        let group_id = app.selection.selected_group.unwrap();
        assert!(app.enter_group_context(group_id));
        app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
        assert!(app.make_unique());

        let expected = app.document.current();
        let loaded = ketchup_core::persistence::load(&ketchup_core::persistence::save(&expected))
            .unwrap()
            .snapshot();
        assert_eq!(loaded.canonical_digest(), expected.canonical_digest());
        assert_eq!(
            loaded.occurrence(OccurrenceId(1)).unwrap().parent(),
            Some(group_id)
        );
        assert_eq!(
            loaded.occurrence(OccurrenceId(2)).unwrap().parent(),
            Some(group_id)
        );
        assert_ne!(
            loaded.occurrence(OccurrenceId(1)).unwrap().definition_id(),
            loaded.occurrence(OccurrenceId(2)).unwrap().definition_id()
        );
    }

    #[test]
    fn review_only_open_preserves_the_active_document_and_its_history() {
        let directory = tempfile::tempdir().unwrap();
        let active_path = directory.path().join("active.ketchup");
        let review_path = directory.path().join("legacy.ketchup");
        std::fs::write(&review_path, lossy_legacy_document()).unwrap();

        let mut app = KetchupApp::new();
        assert!(app.save_document_to(&active_path));
        let node_id = ketchup_core::document::NodeId(900);
        let active_revision = app
            .document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateEvaluatorNode {
                    id: node_id,
                    name: "active parameter".to_owned(),
                    dimension: Dimension::new("2", 2.0).unwrap(),
                    dependencies: vec![],
                },
            ]))
            .unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: node_id,
                    dimension: Dimension::new("3", 3.0).unwrap(),
                },
            ]))
            .unwrap();
        assert!(app.undo());
        assert_eq!(app.document.visible_undo_steps(), 1);
        assert_eq!(app.document.visible_redo_steps(), 1);

        let before = app.document.current();
        assert_eq!(
            before.canonical_digest(),
            active_revision.snapshot().canonical_digest()
        );
        let before_document_id = before.document_id();
        let before_revision = before.revision_id();
        let before_digest = before.canonical_digest();
        let before_canonical_bytes = ketchup_core::persistence::save(&before);
        let before_evaluation = before.evaluate(&Default::default()).unwrap();
        let before_path = app.document_path.clone();
        let before_dirty = app.is_dirty();
        let before_undo_steps = app.document.visible_undo_steps();
        let before_redo_steps = app.document.visible_redo_steps();
        let before_revision_count = app.document.revision_count();
        let before_evaluation_registry = app.document.evaluation_registry_len();

        assert!(!app.open_document_from(&review_path));
        assert!(app.has_review_candidate());
        assert!(!app.review_candidate.as_ref().unwrap().is_editable());

        let after = app.document.current();
        assert_eq!(after.document_id(), before_document_id);
        assert_eq!(after.revision_id(), before_revision);
        assert_eq!(after.canonical_digest(), before_digest);
        assert_eq!(
            ketchup_core::persistence::save(&after),
            before_canonical_bytes
        );
        assert_eq!(
            after.evaluate(&Default::default()).unwrap(),
            before_evaluation
        );
        assert_eq!(app.document_path, before_path);
        assert_eq!(app.is_dirty(), before_dirty);
        assert_eq!(app.document.visible_undo_steps(), before_undo_steps);
        assert_eq!(app.document.visible_redo_steps(), before_redo_steps);
        assert_eq!(app.document.revision_count(), before_revision_count);
        assert_eq!(
            app.document.evaluation_registry_len(),
            before_evaluation_registry
        );
    }

    #[test]
    fn lossless_schema_three_open_replaces_the_document_and_clears_history_and_review() {
        let directory = tempfile::tempdir().unwrap();
        let review_path = directory.path().join("legacy.ketchup");
        let lossless_path = directory.path().join("lossless.ketchup");
        std::fs::write(&review_path, lossy_legacy_document()).unwrap();

        let mut source = KetchupApp::new();
        assert!(source.create_box());
        assert!(source.create_box());
        let expected = source.document.current();
        let expected_bytes = ketchup_core::persistence::save(&expected);
        assert!(source.save_document_to(&lossless_path));

        let mut app = KetchupApp::new();
        assert!(app.create_box());
        let replaced_document_id = app.document.current().document_id();
        assert!(app.document.visible_undo_steps() > 0);
        assert!(!app.open_document_from(&review_path));
        assert!(app.has_review_candidate());

        assert!(app.open_document_from(&lossless_path));

        let opened = app.document.current();
        assert_ne!(opened.document_id(), replaced_document_id);
        assert_eq!(opened.document_id(), expected.document_id());
        assert_eq!(opened.revision_id(), expected.revision_id());
        assert_eq!(opened.canonical_digest(), expected.canonical_digest());
        assert_eq!(ketchup_core::persistence::save(&opened), expected_bytes);
        assert_eq!(app.document_path.as_deref(), Some(lossless_path.as_path()));
        assert!(!app.is_dirty());
        assert_eq!(app.document.visible_undo_steps(), 0);
        assert_eq!(app.document.visible_redo_steps(), 0);
        assert_eq!(app.document.revision_count(), 1);
        assert!(!app.has_review_candidate());
    }

    #[test]
    fn file_workflow_round_trips_composed_model_and_tracks_dirty_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("composed.ketchup");
        let mut app = KetchupApp::new();
        assert!(!app.is_dirty());

        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        assert!(app.copy_selected(Vec3::new(150.0, 25.0, 0.0)));
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
        assert!(app.group_selected());
        let expected = app.document.current();
        assert!(app.is_dirty());
        assert!(app.save_document_to(&path));
        assert!(!app.is_dirty());
        assert_eq!(app.document_path.as_deref(), Some(path.as_path()));

        let mut reopened = KetchupApp::new().with_dialogs(Box::new(
            dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
        ));
        assert!(reopened.open_document_from(&path));
        let actual = reopened.document.current();
        assert_eq!(actual.canonical_digest(), expected.canonical_digest());
        assert_eq!(actual.revision_id(), expected.revision_id());
        assert_eq!(actual.document_id(), expected.document_id());
        assert_eq!(actual.units(), expected.units());
        assert_eq!(actual.definitions().count(), 1);
        assert_eq!(actual.occurrences().count(), 2);
        assert_eq!(actual.groups().count(), 1);
        assert_eq!(actual.scene_query()[0].shared_occurrence_count, 2);
        assert_eq!(
            actual.occurrence(OccurrenceId(1)).unwrap().parent(),
            actual.occurrence(OccurrenceId(2)).unwrap().parent()
        );
        assert_eq!(
            actual.occurrence(OccurrenceId(2)).unwrap().transform(),
            expected.occurrence(OccurrenceId(2)).unwrap().transform()
        );
        assert_eq!(
            actual.feature(FeatureId(2)).unwrap().kind(),
            expected.feature(FeatureId(2)).unwrap().kind()
        );
        assert!(!reopened.is_dirty());
        assert_eq!(reopened.document.visible_undo_steps(), 0);

        reopened.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        assert!(reopened.move_selected(Vec3::new(10.0, 0.0, 0.0)));
        assert!(reopened.is_dirty());
        assert!(reopened.undo());
        assert!(!reopened.is_dirty());
        assert!(reopened.redo());
        assert!(reopened.is_dirty());
        assert!(reopened.save_document_to(&path));
        let saved_again = ketchup_core::persistence::load_file(&path)
            .unwrap()
            .snapshot();
        assert_eq!(
            saved_again.canonical_digest(),
            reopened.document.current().canonical_digest()
        );
    }

    #[test]
    fn failed_open_and_save_preserve_the_active_document_and_file_identity() {
        let directory = tempfile::tempdir().unwrap();
        let malformed = directory.path().join("malformed.ketchup");
        std::fs::write(&malformed, b"not a ketchup document").unwrap();
        let mut app = KetchupApp::new();
        assert!(app.create_box());
        let before_digest = app.document.current().canonical_digest();
        let before_path = app.document_path.clone();
        let before_saved_digest = app.saved_digest.clone();

        assert!(!app.open_document_from(&malformed));
        assert_eq!(app.document.current().canonical_digest(), before_digest);
        assert_eq!(app.document_path, before_path);
        assert_eq!(app.saved_digest, before_saved_digest);
        assert!(app.digest.contains("active model was not changed"));

        assert!(!app.save_document_to(directory.path()));
        assert_eq!(app.document.current().canonical_digest(), before_digest);
        assert_eq!(app.document_path, before_path);
        assert_eq!(app.saved_digest, before_saved_digest);
        assert!(app.is_dirty());
        assert!(app.digest.contains("active model remains unsaved"));
    }

    #[test]
    fn command_registry_exposes_only_complete_modeling_tools() {
        let mut app = KetchupApp::new();
        assert!(app.command_enabled(AppCommand::Select));
        assert!(app.command_enabled(AppCommand::Orbit));
        assert!(app.command_enabled(AppCommand::Rectangle));
        assert!(app.command_enabled(AppCommand::PushPull));
        assert!(app.command_enabled(AppCommand::Move));

        app.dispatch_command(AppCommand::Rectangle);
        assert_eq!(app.active_tool, ActiveTool::Rectangle);
        assert!(app.sketch_mode);
        app.dispatch_command(AppCommand::PushPull);
        assert_eq!(app.active_tool, ActiveTool::PushPull);
        assert!(!app.sketch_mode);
    }
}
