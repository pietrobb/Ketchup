#![forbid(unsafe_code)]

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use ketchup_core::beam_m4ae::{
    BeamChangeSummary, BeamSlice, BeamValidationVerdict, BeamWorkspace, GroovePosition, GroupedBom,
};
use ketchup_core::beam_m5::{BeamExactPiecePackage, BeamM5Products};
use ketchup_core::bottle_m6::{BottleAuthorityReport, ExactRevolveRequest};
use ketchup_core::document::{
    BottleControlDimension, BottleEdgeFinishKind, CanonicalCommand, CommandBatch, DefinitionId,
    Dimension, DocumentId, DocumentStore, FeatureId, FeatureKind, GroupId, InstancePath, NodeId,
    OccurrenceId, Proposal, ProposalGoal, ProposalValue, Snapshot, Transform,
};
use ketchup_core::exact_product::{
    AssemblySelectionTarget, ExactBodyPackage, ExactBodyView, ExactFaceRole,
    ExactFeatureChainRequest, ExactMeshExport, ExactResultRegistry,
};
use ketchup_core::fabrication::{FullBomProjection, PieceDimensionSheet};
use ketchup_core::graph::DerivedIdentity;
use ketchup_core::intent::{IntentRequest, WorkflowIntent, propose_intent};
use ketchup_core::validation::ValidationReport;
use ketchup_interaction::{
    Axis, ElementId, LocaleCatalog, Ray, SelectionId, Side, Vec3,
    exact_projection::ExactInteractionProjection,
    projection::{CanonicalInteractionProjection, ProjectedBox},
};
use ketchup_scheduler::ExactWorkerSupervisor;
pub mod dialogs;

use dialogs::{DiscardRequest, ExportRequest, FileDialogs, NativeFileDialogs, SaveRequest};

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};
use std::time::{Duration, Instant};

const INITIAL_BOX_DEFINITION: DefinitionId = DefinitionId(1);
const BOX_WIDTH_MM: f64 = 100.0;
const BOX_DEPTH_MM: f64 = 60.0;
const GRID_STEP_MM: f64 = 10.0;

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
    extrusion_feature_id: FeatureId,
    instance_path: InstancePath,
    origin_mm: Vec3,
    size_mm: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
struct EphemeralBoxPreview {
    source_revision: u64,
    selection_state: Option<SelectionId>,
    target: SelectionId,
    command_digest: String,
    box_data: RenderBox,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveTool {
    Select,
    Line,
    Rectangle,
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
    Select,
    Line,
    Rectangle,
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
    ZoomFit,
    Shortcuts,
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
    const COMMANDS: [CommandSpec; 30] = [
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
pub enum AssistantIntentKind {
    RuleDimension,
    FeatureDimension,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantVerification {
    pub revision_id: u64,
    pub command_digest: String,
    pub result_digest: String,
    pub verified_write_count: usize,
}

pub struct KetchupApp {
    document: DocumentStore,
    review_candidate: Option<ketchup_core::persistence::LoadOutcome>,
    document_path: Option<PathBuf>,
    saved_digest: String,
    catalog: LocaleCatalog,
    push_pull_distance_input: String,
    preview: Option<CommandBatch>,
    preview_box: Option<EphemeralBoxPreview>,
    preview_definition_id: Option<DefinitionId>,
    status_key: &'static str,
    yaw: f32,
    pitch: f32,
    camera_target_z: f64,
    zoom: f32,
    pan: Vec2,
    selection: SelectionState,
    hovered: Option<SelectionId>,
    active_tool: ActiveTool,
    digest: String,
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
    sketch_cursor: Option<Vec3>,
    sketch_height_input: String,
    value_input: String,
    focus_value_box: bool,
    occurrence_clipboard: Vec<OccurrenceId>,
    measure_start: Option<Vec3>,
    measure_cursor: Option<Vec3>,
    measure_end: Option<Vec3>,
    shortcuts_open: bool,
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
}

impl Default for KetchupApp {
    fn default() -> Self {
        Self::new()
    }
}

impl KetchupApp {
    #[must_use]
    pub fn new() -> Self {
        let catalog = LocaleCatalog::english();
        let mut document = DocumentStore::new();
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
            review_candidate: None,
            document_path: None,
            saved_digest,
            catalog,
            push_pull_distance_input: String::new(),
            preview: None,
            preview_box: None,
            preview_definition_id: None,
            status_key: "status-ready",
            yaw: -0.65,
            pitch: -0.5,
            camera_target_z: 10.0,
            zoom: 2.8,
            pan: Vec2::ZERO,
            selection: SelectionState::default(),
            hovered: None,
            active_tool: ActiveTool::Select,
            digest,
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
            sketch_cursor: None,
            sketch_height_input: "20".to_owned(),
            value_input: String::new(),
            focus_value_box: false,
            occurrence_clipboard: Vec::new(),
            measure_start: None,
            measure_cursor: None,
            measure_end: None,
            shortcuts_open: false,
            viewport_rect: None,
            dialogs: Box::new(NativeFileDialogs),
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
        }
    }

    /// Answer file dialogs from `dialogs` instead of the operating system.
    #[must_use]
    pub fn with_dialogs(mut self, dialogs: Box<dyn FileDialogs>) -> Self {
        self.dialogs = dialogs;
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

    fn reset_document_presentation(&mut self) {
        self.preview = None;
        self.preview_box = None;
        self.preview_definition_id = None;
        self.selection = SelectionState::default();
        self.hovered = None;
        self.active_tool = ActiveTool::Select;
        self.assistant_proposal = None;
        self.assistant_verification = None;
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
        self.sketch_cursor = None;
        self.value_input.clear();
        self.focus_value_box = false;
        if let Some(task) = self.exact_task.take() {
            task.cancelled.store(true, Ordering::Release);
        }
        self.exact_results.clear();
        self.exact_source = None;
        self.exact_retry_at = None;
        self.status_key = "status-ready";
    }

    fn new_document(&mut self) {
        let dialogs = std::mem::replace(&mut self.dialogs, Box::new(NativeFileDialogs));
        *self = Self::new().with_dialogs(dialogs);
        self.digest = self.catalog.text("digest-new-document");
    }

    fn open_document_from(&mut self, path: &Path) -> bool {
        match ketchup_core::persistence::load_file(path) {
            Ok(outcome) => {
                if !outcome.is_editable() {
                    self.review_candidate = Some(outcome);
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
                let Ok(mut document) = outcome.into_editable() else {
                    unreachable!("editable load outcome must contain an editable document");
                };
                document.discard_history_before_current();
                self.document = document;
                self.review_candidate = None;
                self.document_path = Some(path.to_owned());
                self.saved_digest = self.document.current().canonical_digest();
                self.reset_document_presentation();
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

    fn save_document_to(&mut self, path: &Path) -> bool {
        let snapshot = self.document.current();
        match ketchup_core::persistence::save_atomic(path, &snapshot) {
            Ok(()) => {
                self.document_path = Some(path.to_owned());
                self.saved_digest = snapshot.canonical_digest();
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
                .with_min_inner_size([760.0, 480.0]),
            ..Default::default()
        }
    }

    pub fn load_beam_m4ae(&mut self) -> bool {
        match BeamWorkspace::load() {
            Ok(workspace) => {
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
            .and_then(|bundle| write_exact_mesh_export(path, bundle));
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
            .and_then(|bytes| std::fs::write(path, bytes).map_err(|error| error.to_string()));
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

    /// Whether the active document carries unsaved changes.
    #[must_use]
    pub fn has_review_candidate(&self) -> bool {
        self.review_candidate.is_some()
    }

    pub fn is_dirty(&self) -> bool {
        self.document.current().canonical_digest() != self.saved_digest
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
        match propose_intent(&self.document, IntentRequest::m7a(intent)) {
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

    pub fn confirm_assistant_proposal(&mut self) -> bool {
        let Some(proposal) = self.assistant_proposal.take() else {
            return false;
        };
        match self.document.commit_verified_proposal(&proposal) {
            Ok(committed) => {
                let verification = AssistantVerification {
                    revision_id: committed.revision().id(),
                    command_digest: committed.command_digest().to_owned(),
                    result_digest: committed.result_digest().to_owned(),
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

    #[must_use]
    pub const fn assistant_verification(&self) -> Option<&AssistantVerification> {
        self.assistant_verification.as_ref()
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

    #[must_use]
    pub fn exact_render_body_count(&self) -> usize {
        let snapshot = self.document.current();
        self.exact_results
            .values()
            .filter(|package| package.is_current(&snapshot))
            .count()
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

    #[must_use]
    pub fn exact_pick_durable(&self, ray: Ray) -> Option<AssemblySelectionTarget> {
        let snapshot = self.document.current();
        self.exact_projection(&snapshot)
            .exact_pick(ray)
            .map(|hit| hit.target)
    }

    fn active_boxes(&self) -> Vec<RenderBox> {
        let snapshot = self.document.current();
        CanonicalInteractionProjection::from_snapshot(&snapshot)
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
                let exact_bounds = self
                    .exact_results
                    .get(&occurrence.body.definition_id)
                    .filter(|_| translation_only)
                    .filter(|package| package.is_current(&snapshot))
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
                    extrusion_feature_id: occurrence.body.extrusion_feature_id?,
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

    fn occurrence_in_active_context(&self, instance_path: &InstancePath) -> bool {
        let snapshot = self.document.current();
        let Some(occurrence) = snapshot.occurrence(instance_path.root_occurrence()) else {
            return false;
        };
        if snapshot.resolve_instance_path(instance_path).is_err() {
            return false;
        }
        match self.selection.edit_context.last() {
            None => true,
            Some(EditContext::Group(group_id)) => occurrence.parent() == Some(*group_id),
            Some(EditContext::Definition {
                definition_id,
                instance_path: context_path,
            }) => {
                snapshot
                    .resolve_instance_path(context_path)
                    .is_ok_and(|resolved| resolved.definition_id == *definition_id)
                    && instance_path.root_occurrence() == context_path.root_occurrence()
                    && instance_path.steps().starts_with(context_path.steps())
            }
        }
    }

    fn select_group(&mut self, group_id: GroupId) -> bool {
        let snapshot = self.document.current();
        let Some(group) = snapshot.group(group_id) else {
            return false;
        };
        let ids = snapshot
            .occurrences()
            .filter(|occurrence| occurrence.parent() == Some(group_id))
            .map(|occurrence| InstancePath::root(occurrence.id()))
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
        let ids = snapshot
            .scene_query()
            .into_iter()
            .filter(|item| {
                item.definition_id == definition_id
                    && self.occurrence_in_active_context(&item.instance_path)
            })
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
            .document
            .current()
            .scene_query()
            .into_iter()
            .filter(|item| item.visible && self.occurrence_in_active_context(&item.instance_path))
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
        snapshot
            .definitions()
            .map(|definition| {
                let size = projection
                    .occurrences()
                    .iter()
                    .find(|occurrence| occurrence.body.definition_id == definition.id())
                    .and_then(|occurrence| occurrence.local_box.map(|local_box| local_box.size_mm))
                    .unwrap_or(Vec3::ZERO);
                let occurrences = snapshot
                    .scene_query()
                    .into_iter()
                    .filter(|item| item.definition_id == definition.id())
                    .map(|item| {
                        let matrix = item.transform.matrix();
                        OutlinerOccurrence {
                            instance_path: item.instance_path,
                            name: item.occurrence_name,
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
            if tool == ActiveTool::Rectangle {
                self.sketch_mode = true;
                self.status_key = "status-sketch-first-point";
            } else if tool == ActiveTool::Measure {
                self.status_key = "status-measure-first-point";
            }
            self.digest = self.catalog.format(
                "digest-tool-active",
                &BTreeMap::from([("tool", self.catalog.text(tool.label_key()))]),
            );
            return;
        }
        match id {
            AppCommand::New | AppCommand::Open | AppCommand::Save | AppCommand::SaveAs => {
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
            AppCommand::ZoomFit => self.zoom_fit(),
            AppCommand::Shortcuts => self.shortcuts_open = true,
            AppCommand::Select
            | AppCommand::Line
            | AppCommand::Rectangle
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
        self.zoom = (fit * 0.82).clamp(0.8, 8.0);
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
        if group.transform() != Transform::identity() {
            return false;
        }
        let parent = group.parent();
        let ids = snapshot
            .occurrences()
            .filter(|occurrence| occurrence.parent() == Some(group_id))
            .map(|occurrence| occurrence.id())
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return false;
        }
        let mut commands = ids
            .iter()
            .copied()
            .map(|id| CanonicalCommand::SetOccurrenceParent { id, parent })
            .collect::<Vec<_>>();
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
            .extend(ids.iter().copied().map(InstancePath::root));
        self.digest = self.catalog.format(
            "digest-ungrouped",
            &BTreeMap::from([("count", ids.len().to_string())]),
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
                kind: FeatureKind::Revolve { profile: control },
            },
            CanonicalCommand::CreateFeature {
                id: shell,
                definition_id,
                name: "Bottle shell".to_owned(),
                kind: FeatureKind::Shell {
                    target: revolve,
                    thickness: Dimension::new("2", 2.0).expect("built-in thickness is valid"),
                },
            },
            CanonicalCommand::CreateFeature {
                id: finish,
                definition_id,
                name: "Bottle shoulder finish".to_owned(),
                kind: FeatureKind::BottleEdgeFinish {
                    target: shell,
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
            .and_then(|bundle| write_exact_mesh_export(path, bundle));
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
            .and_then(|bundle| write_exact_mesh_export(path, bundle));
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

    pub fn start_preview(&mut self) -> bool {
        let selection = self.selection.primary.clone().unwrap_or(SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        });
        if !selection.instance_path.is_root() {
            return false;
        }
        let Some(distance_mm) = parse_distance_mm(&self.push_pull_distance_input) else {
            return false;
        };
        let Some(item) = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)
        else {
            return false;
        };
        let Some(current_extent_mm) = face_extent(&item, Some(&selection.element)) else {
            return false;
        };
        let new_extent_mm = current_extent_mm + distance_mm;
        let Some(after) = resize_box_from_face(&item, &selection.element, new_extent_mm) else {
            return false;
        };
        let Some(batch) = push_pull_batch(
            &self.document.current(),
            &selection,
            &item,
            new_extent_mm,
            format_height(new_extent_mm),
        ) else {
            return false;
        };
        let command_digest = batch.digest();
        self.preview = Some(batch);
        self.preview_box = Some(EphemeralBoxPreview {
            source_revision: self.document.current().revision_id(),
            selection_state: self.selection.primary.clone(),
            target: selection.clone(),
            command_digest,
            box_data: after,
        });
        self.preview_definition_id = Some(selection.definition_id);
        self.status_key = "status-preview";
        let snapshot = self.document.current();
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
        let expected_target = self.selection.primary.clone().unwrap_or(SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        });
        self.preview_box.as_ref().is_some_and(|preview| {
            preview.source_revision == self.document.current().revision_id()
                && preview.selection_state == self.selection.primary
                && preview.target == expected_target
                && self
                    .preview
                    .as_ref()
                    .is_some_and(|batch| batch.digest() == preview.command_digest)
        })
    }

    pub fn cancel_preview(&mut self) {
        self.clear_ephemeral_edit_state();
        self.status_key = "status-ready";
    }

    fn clear_ephemeral_edit_state(&mut self) {
        self.preview = None;
        self.preview_box = None;
        self.preview_definition_id = None;
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
                } => {
                    snapshot.definition(*definition_id).is_none()
                        || !snapshot
                            .resolve_instance_path(instance_path)
                            .is_ok_and(|resolved| resolved.definition_id == *definition_id)
                }
            })
        {
            self.selection.edit_context.pop();
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

    pub fn undo(&mut self) -> bool {
        if self.document.undo().is_none() {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.reconcile_selection();
        self.status_key = "status-undo";
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.document.redo().is_none() {
            return false;
        }
        self.clear_ephemeral_edit_state();
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
        if self.document.apply_batch(&batch).is_err() {
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
        let selection = self.selection.primary.clone().unwrap_or(SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        });
        let snapshot = self.document.current();
        let occurrence = snapshot.occurrence(selection.instance_path.root_occurrence())?;
        let definition = snapshot.definition(occurrence.definition_id())?;
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
                    && drag.selection.instance_path == item.instance_path
            });
        push_pull_preview || move_preview
    }

    fn viewport_boxes(&self, exact_projection: &ExactInteractionProjection) -> Vec<RenderBox> {
        let mut boxes = self.active_boxes();
        if let Some(drag) = self.move_drag.as_ref().or(self.move_anchor.as_ref())
            && self.move_preview_is_current(drag)
            && let Some(index) = boxes
                .iter()
                .position(|item| item.instance_path == drag.selection.instance_path)
        {
            let mut preview = boxes[index].clone();
            preview.origin_mm = preview.origin_mm + drag.delta_mm;
            if drag.copy {
                boxes.push(preview);
            } else {
                boxes[index] = preview;
            }
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
    pub fn measured_distance_mm(&self) -> Option<f64> {
        let (start, end) = (self.measure_start?, self.measure_end?);
        Some(vector_length(Vec3::new(
            end.x - start.x,
            end.y - start.y,
            end.z - start.z,
        )))
    }

    fn cancel_rectangle_sketch(&mut self) {
        self.sketch_mode = false;
        self.sketch_start = None;
        self.sketch_cursor = None;
        self.status_key = "status-ready";
    }

    fn complete_rectangle_sketch(&mut self, start: Vec3, end: Vec3) -> bool {
        let Ok(height) = self.sketch_height_input.parse::<f64>() else {
            return false;
        };
        let origin = Vec3::new(start.x.min(end.x), start.y.min(end.y), start.z);
        let size = Vec3::new((end.x - start.x).abs(), (end.y - start.y).abs(), height);
        let created = self.create_box_at(origin, size);
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
            return self.complete_exact_rectangle();
        }
        if self.active_tool == ActiveTool::PushPull {
            let selection = self.selection.primary.clone().or_else(|| {
                self.last_push_pull
                    .as_ref()
                    .map(|operation| operation.selection.clone())
            });
            let Some(selection) = selection else {
                self.digest = self.catalog.text("digest-nothing-to-apply");
                return false;
            };
            self.selection.select_exact(selection.clone(), false);
            self.push_pull_distance_input = self.value_input.clone();
            if self.start_preview() && self.confirm_preview() {
                self.last_push_pull = Some(LastPushPull {
                    selection: selection.clone(),
                });
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
            ActiveTool::Rectangle => "value-label-width-depth",
            ActiveTool::PushPull | ActiveTool::Move | ActiveTool::Measure => "value-label-distance",
            _ => "value-label-dimensions",
        }
    }

    fn view_ray(&self, pointer: Pos2, rect: Rect) -> Option<Ray> {
        let yaw_sin = f64::from(self.yaw.sin());
        let yaw_cos = f64::from(self.yaw.cos());
        let pitch_sin = f64::from(self.pitch.sin());
        let pitch_cos = f64::from(self.pitch.cos());
        let right = Vec3::new(yaw_cos, -yaw_sin, 0.0);
        let up = Vec3::new(yaw_sin * pitch_cos, yaw_cos * pitch_cos, -pitch_sin);
        let forward = Vec3::new(-yaw_sin * pitch_sin, -yaw_cos * pitch_sin, -pitch_cos);
        let scale = f64::from(self.zoom) * f64::from(rect.width().min(rect.height())) / 420.0;
        let horizontal = f64::from(pointer.x - rect.center().x - self.pan.x) / scale;
        let vertical = f64::from(rect.center().y + self.pan.y - pointer.y) / scale;
        let model_center = Vec3::new(BOX_WIDTH_MM * 0.5, BOX_DEPTH_MM * 0.5, self.camera_target_z);
        let view_plane_point = model_center + right * horizontal + up * vertical;
        Ray::new(view_plane_point - forward * 10_000.0, forward).ok()
    }

    fn screen_to_plane(&self, pointer: Pos2, rect: Rect, plane_z: f64) -> Option<Vec3> {
        let ray = self.view_ray(pointer, rect)?;
        if ray.direction.z.abs() <= 1.0e-9 {
            return None;
        }
        let distance = (plane_z - ray.origin.z) / ray.direction.z;
        (distance >= 0.0).then(|| ray.origin + ray.direction * distance)
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
        self.catalog.format(
            "hover-face",
            &BTreeMap::from([
                ("name", definition.name().to_owned()),
                (
                    "face",
                    self.catalog.text(match hovered.element {
                        ElementId::Face {
                            axis: Axis::Z,
                            side: Side::Maximum,
                        } => "face-top",
                        ElementId::Face {
                            axis: Axis::Z,
                            side: Side::Minimum,
                        } => "face-bottom",
                        _ => "face-side",
                    }),
                ),
            ]),
        )
    }

    fn viewport_overlays(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let painter = ui.painter();
        let glass = Color32::from_black_alpha(150);
        let line = Color32::from_rgb(43, 48, 54);
        let text = Color32::from_rgb(233, 235, 238);
        let dim = Color32::from_rgb(152, 160, 169);
        let mono = egui::FontId::monospace(11.0);

        let digest_rect = Rect::from_min_size(
            rect.left_top() + Vec2::new(12.0, 12.0),
            Vec2::new(390.0_f32.min(rect.width() - 24.0), 28.0),
        );
        painter.rect_filled(digest_rect, 8.0, glass);
        painter.rect_stroke(
            digest_rect,
            8.0,
            Stroke::new(1.0_f32, line),
            egui::StrokeKind::Inside,
        );
        painter.circle_filled(
            digest_rect.left_center() + Vec2::new(12.0, 0.0),
            3.0,
            Color32::from_rgb(240, 78, 35),
        );
        painter.text(
            digest_rect.left_center() + Vec2::new(23.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &self.digest,
            egui::FontId::proportional(12.0),
            text,
        );

        let camera = self.catalog.format(
            "camera-readout",
            &BTreeMap::from([
                ("distance", format_height(420.0 / f64::from(self.zoom))),
                ("azimuth", format_height(f64::from(self.yaw.to_degrees()))),
                (
                    "elevation",
                    format_height(f64::from(self.pitch.to_degrees())),
                ),
            ]),
        );
        let readout_rect = Rect::from_min_size(
            Pos2::new(rect.right() - 300.0, rect.top() + 12.0),
            Vec2::new(288.0, 48.0),
        );
        painter.rect_filled(readout_rect, 7.0, glass);
        painter.text(
            readout_rect.right_top() + Vec2::new(-10.0, 8.0),
            egui::Align2::RIGHT_TOP,
            camera,
            mono.clone(),
            dim,
        );
        painter.text(
            readout_rect.right_bottom() + Vec2::new(-10.0, -8.0),
            egui::Align2::RIGHT_BOTTOM,
            self.hover_readout(),
            mono.clone(),
            text,
        );

        let hint_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 14.0, rect.bottom() - 54.0),
            Vec2::new((rect.width() * 0.46).max(240.0), 40.0),
        );
        painter.rect_filled(hint_rect, 9.0, glass);
        painter.text(
            hint_rect.left_center() + Vec2::new(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            self.catalog.text(self.active_tool.hint_key()),
            egui::FontId::proportional(12.0),
            text,
        );

        let value_rect = Rect::from_min_size(
            Pos2::new(rect.right() - 318.0, rect.bottom() - 52.0),
            Vec2::new(304.0, 38.0),
        );
        painter.rect_filled(value_rect, 10.0, Color32::from_black_alpha(180));
        painter.rect_stroke(
            value_rect,
            10.0,
            Stroke::new(1.0_f32, line),
            egui::StrokeKind::Inside,
        );
        let label_rect = Rect::from_min_size(value_rect.min, Vec2::new(114.0, 38.0));
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            self.catalog.text(self.value_label_key()),
            egui::FontId::proportional(11.0),
            dim,
        );
        let input_rect = Rect::from_min_max(
            Pos2::new(label_rect.right() + 1.0, value_rect.top() + 2.0),
            Pos2::new(value_rect.right() - 4.0, value_rect.bottom() - 2.0),
        );
        let response = ui.put(
            input_rect,
            egui::TextEdit::singleline(&mut self.value_input)
                .id_salt("value-box-input")
                .hint_text(self.catalog.text("value-placeholder"))
                .font(egui::TextStyle::Monospace)
                .frame(false),
        );
        if self.focus_value_box {
            response.request_focus();
            self.focus_value_box = false;
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
        let desired = ui.available_size().max(Vec2::new(320.0, 280.0));
        let (response, painter) = ui.allocate_painter(desired, Sense::click_and_drag());
        self.viewport_rect = Some(response.rect);
        painter.rect_filled(response.rect, 0.0, Color32::from_rgb(24, 28, 36));
        painter.rect_stroke(
            response.rect,
            0.0,
            Stroke::new(1.0_f32, Color32::from_rgb(62, 70, 84)),
            egui::StrokeKind::Inside,
        );

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
                    || self.rectangle_plane_z(pointer, response.rect),
                    |start| start.z,
                );
                if let Some(point) = self.screen_to_plane(pointer, response.rect, plane_z) {
                    if let Some(start) = self.sketch_start {
                        self.complete_rectangle_sketch(start, point);
                    } else {
                        self.sketch_start = Some(point);
                        self.sketch_cursor = Some(point);
                        self.value_input.clear();
                        self.status_key = "status-sketch-second-point";
                    }
                }
            } else if self.active_tool == ActiveTool::Select {
                let additive = ui.input(|input| input.modifiers.shift);
                let target = self.exact_pick_at_screen(pointer, response.rect);
                self.select_from_viewport(target, additive);
            } else if self.active_tool == ActiveTool::PushPull {
                if !self.begin_bottle_direct_drag(pointer, response.rect) {
                    let target = self.exact_pick_at_screen(pointer, response.rect);
                    self.select_from_viewport(target, false);
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
                            self.translate_occurrence(
                                &anchor.selection,
                                anchor.delta_mm,
                                anchor.copy,
                            );
                        } else {
                            self.move_anchor = Some(anchor);
                        }
                    }
                } else {
                    let target =
                        self.exact_pick_at_screen(pointer, response.rect)
                            .filter(|selection| {
                                self.occurrence_in_active_context(&selection.instance_path)
                            });
                    if target.is_some() {
                        self.select_from_viewport(target.clone(), false);
                    } else {
                        self.digest = self.catalog.text("digest-move-start-missed");
                    }
                    if let Some(selection) = target
                        && let Some(item) = self
                            .active_boxes()
                            .into_iter()
                            .find(|item| item.instance_path == selection.instance_path)
                        && let Some(pointer_start_world) =
                            self.screen_to_plane(pointer, response.rect, item.origin_mm.z)
                    {
                        self.value_input = "0".to_owned();
                        let snapshot = self.document.current();
                        self.move_drag = Some(MoveDrag {
                            source_document_id: snapshot.document_id(),
                            source_revision: snapshot.revision_id(),
                            selection,
                            pointer_start_world,
                            plane_z: item.origin_mm.z,
                            delta_mm: Vec3::ZERO,
                            copy: ui.input(|input| input.modifiers.command),
                        });
                    }
                }
            } else if self.active_tool == ActiveTool::Measure {
                let plane_z = self.measure_anchor().map_or_else(
                    || self.rectangle_plane_z(pointer, response.rect),
                    |start| start.z,
                );
                if let Some(point) = self.screen_to_plane(pointer, response.rect, plane_z) {
                    self.add_measured_point(point);
                }
            }
        }

        if self.active_tool == ActiveTool::Select
            && response.double_clicked()
            && let Some(pointer) = response.interact_pointer_pos()
            && let Some(target) = self.exact_pick_at_screen(pointer, response.rect)
        {
            self.enter_occurrence_context(target.instance_path);
        }

        self.hovered = response
            .hover_pos()
            .and_then(|pointer| self.exact_pick_at_screen(pointer, response.rect));

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
                    self.translate_occurrence(&drag.selection, drag.delta_mm, drag.copy);
                } else {
                    self.move_anchor = Some(drag);
                    self.digest = self.catalog.text("digest-move-anchor-set");
                }
            } else if let Some(drag) = self.bottle_direct_drag.take() {
                let value = parse_distance_mm(&self.value_input).unwrap_or(drag.value_start_mm);
                self.commit_bottle_direct_drag(drag, value);
            } else if self.push_pull_drag.take().is_some() && self.has_preview() {
                self.confirm_preview();
            }
        }
        if self.sketch_mode
            && self.sketch_start.is_some()
            && response.hovered()
            && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
        {
            let plane_z = self.sketch_start.map_or(0.0, |start| start.z);
            self.sketch_cursor = self.screen_to_plane(pointer, response.rect, plane_z);
            // Once the user starts typing, the value box owns the value: the
            // focus request only takes effect next frame, so the freshly typed
            // text would otherwise be overwritten by the hovered dimensions.
            if !ui.ctx().wants_keyboard_input()
                && !self.focus_value_box
                && let (Some(start), Some(cursor)) = (self.sketch_start, self.sketch_cursor)
            {
                self.value_input = format!(
                    "{},{}",
                    format_height((cursor.x - start.x).abs()),
                    format_height((cursor.y - start.y).abs())
                );
            }
        }
        if let Some(start) = self.measure_anchor()
            && response.hovered()
            && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
            && let Some(cursor) = self.screen_to_plane(pointer, response.rect, start.z)
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
            self.zoom = (self.zoom * (1.0 + scroll * 0.001)).clamp(0.8, 8.0);
        }
        let forward = Vec3::new(
            -f64::from(self.yaw.sin() * self.pitch.sin()),
            -f64::from(self.yaw.cos() * self.pitch.sin()),
            -f64::from(self.pitch.cos()),
        );
        let snapshot = self.document.current();
        let exact_projection = self.exact_projection(&snapshot);
        let mut faces = Vec::new();
        for item in self.viewport_boxes(&exact_projection) {
            let item = self.render_box(item);
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
                let depth = face
                    .corners
                    .iter()
                    .map(|index| point_depth(corners[*index], forward))
                    .sum::<f64>()
                    / 4.0;
                faces.push(ProjectedFace {
                    selection,
                    polygon: ProjectedPolygon::Quad(points),
                    color: face.color,
                    depth,
                    previewed: self.has_preview()
                        && self.preview_definition_id == Some(item.definition_id)
                        && matches!(
                            face.element,
                            ElementId::Face {
                                axis: Axis::Z,
                                side: Side::Maximum,
                            }
                        ),
                    out_of_context: !self.occurrence_in_active_context(&item.instance_path),
                });
            }
        }
        let canonical_projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
        for occurrence in canonical_projection
            .occurrences()
            .iter()
            .filter(|occurrence| {
                occurrence.visible
                    && exact_projection.contains_occurrence(&occurrence.instance_path)
            })
        {
            let Some(package) = self.exact_results.get(&occurrence.body.definition_id) else {
                continue;
            };
            let transform = occurrence.canonical_world_transform;
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
                    out_of_context: !self.occurrence_in_active_context(&occurrence.instance_path),
                });
            }
        }
        faces.sort_by(|left, right| right.depth.total_cmp(&left.depth));

        for face in &faces {
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
            painter.add(egui::Shape::convex_polygon(
                face.polygon.points().to_vec(),
                color,
                Stroke::NONE,
            ));
        }

        let edge_stroke = Stroke::new(1.25_f32, Color32::from_rgb(182, 192, 207));
        for face in &faces {
            let points = face.polygon.points();
            for edge in 0..points.len() {
                painter.line_segment(
                    [points[edge], points[(edge + 1) % points.len()]],
                    edge_stroke,
                );
            }
        }

        let selection_stroke = Stroke::new(1.8_f32, Color32::from_rgb(240, 78, 35));
        for face in faces.iter().filter(|face| {
            if self.active_tool == ActiveTool::PushPull {
                self.selection.primary.as_ref() == Some(&face.selection)
            } else {
                self.selection.contains(&face.selection.instance_path)
            }
        }) {
            let points = face.polygon.points();
            for edge in 0..points.len() {
                painter.line_segment(
                    [points[edge], points[(edge + 1) % points.len()]],
                    selection_stroke,
                );
            }
        }

        if let (Some(start), Some(cursor)) = (self.sketch_start, self.sketch_cursor) {
            let ground = [
                start,
                Vec3::new(cursor.x, start.y, 0.0),
                cursor,
                Vec3::new(start.x, cursor.y, 0.0),
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
        self.viewport_overlays(ui, response.rect);
    }

    fn exact_pick_at_screen(&self, pointer: Pos2, rect: Rect) -> Option<SelectionId> {
        let ray = self.view_ray(pointer, rect)?;
        let snapshot = self.document.current();
        let exact_projection = self.exact_projection(&snapshot);
        if let Some(hit) = exact_projection.exact_surface_pick(ray) {
            if !self.occurrence_in_active_context(&hit.instance_path) {
                return None;
            }
            let element = hit
                .durable_target
                .as_ref()
                .and_then(|target| target.body.role())
                .and_then(exact_face_element)
                .or_else(|| exact_surface_element(hit.outward_normal))?;
            return Some(SelectionId {
                definition_id: hit.definition_id,
                instance_path: hit.instance_path,
                element,
            });
        }

        let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
        let scene = projection
            .scene_where(|occurrence| {
                self.occurrence_in_active_context(&occurrence.instance_path)
                    && !exact_projection.contains_occurrence(&occurrence.instance_path)
            })
            .ok()?;
        let scale = f64::from(self.zoom) * f64::from(rect.width().min(rect.height())) / 420.0;
        scene
            .exact_pick(ray, 8.0 / scale)
            .map(|result| result.primary.reference)
    }

    fn project(&self, point: Vec3, rect: Rect) -> Pos2 {
        let centered = Vec3::new(
            point.x - BOX_WIDTH_MM * 0.5,
            point.y - BOX_DEPTH_MM * 0.5,
            point.z - self.camera_target_z,
        );
        let yaw_sin = f64::from(self.yaw.sin());
        let yaw_cos = f64::from(self.yaw.cos());
        let pitch_sin = f64::from(self.pitch.sin());
        let pitch_cos = f64::from(self.pitch.cos());
        let rotated_x = centered.x * yaw_cos - centered.y * yaw_sin;
        let yaw_y = centered.x * yaw_sin + centered.y * yaw_cos;
        let rotated_y = yaw_y * pitch_cos - centered.z * pitch_sin;
        let scale = f64::from(self.zoom) * f64::from(rect.width().min(rect.height())) / 420.0;
        Pos2::new(
            rect.center().x + self.pan.x + (rotated_x * scale) as f32,
            rect.center().y + self.pan.y - (rotated_y * scale) as f32,
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
        let push_pull = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::P));
        let move_tool = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::M));
        let measure = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::T));
        let zoom_fit = !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::Z));
        let shortcuts = context.input(|input| input.key_pressed(egui::Key::F1));
        let group = !context.wants_keyboard_input()
            && context.input(|input| {
                input.modifiers.command && !input.modifiers.shift && input.key_pressed(egui::Key::G)
            });
        let ungroup = !context.wants_keyboard_input()
            && context.input(|input| {
                input.modifiers.command && input.modifiers.shift && input.key_pressed(egui::Key::G)
            });
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
        } else if push_pull {
            self.dispatch_command(AppCommand::PushPull);
        } else if move_tool {
            self.dispatch_command(AppCommand::Move);
        } else if measure {
            self.dispatch_command(AppCommand::Measure);
        } else if zoom_fit {
            self.dispatch_command(AppCommand::ZoomFit);
        } else if shortcuts {
            self.dispatch_command(AppCommand::Shortcuts);
        } else if escape {
            if self.measure_start.is_some() {
                self.clear_measurement();
                self.digest = self.catalog.text("digest-measure-cleared");
                self.status_key = "status-measure-first-point";
            } else if self.has_preview() || self.sketch_mode {
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
        ui.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new(self.catalog.text("brand-mark"))
                    .strong()
                    .color(Color32::WHITE)
                    .background_color(Color32::from_rgb(240, 78, 35)),
            );
            ui.strong(self.catalog.text("app-title"));
            ui.separator();
            ui.label(self.document_title());
            self.command_button(ui, AppCommand::Undo);
            self.command_button(ui, AppCommand::Redo);
            ui.add_space(ui.available_width().max(0.0) * 0.2);
            self.command_button(ui, AppCommand::ViewIso);
            self.command_button(ui, AppCommand::ViewTop);
            self.command_button(ui, AppCommand::ViewFront);
            self.command_button(ui, AppCommand::ZoomFit);
            ui.monospace(self.catalog.text("unit-mm"));
            ui.add_enabled(false, egui::Button::new(self.catalog.text("theme-light")));
        });
    }

    fn show_menu_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button(self.catalog.text("menu-file"), |ui| {
                self.menu_command(ui, AppCommand::New);
                self.menu_command(ui, AppCommand::Open);
                self.menu_command(ui, AppCommand::Save);
                self.menu_command(ui, AppCommand::SaveAs);
                ui.separator();
                self.disabled_menu_item(ui, "file-export-exact");
                self.disabled_menu_item(ui, "file-export-mesh");
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
                self.menu_command(ui, AppCommand::Hide);
                self.menu_command(ui, AppCommand::Unhide);
            });
            ui.menu_button(self.catalog.text("menu-draw"), |ui| {
                self.menu_command(ui, AppCommand::Line);
                self.menu_command(ui, AppCommand::Rectangle);
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
        const TOOLS: [AppCommand; 8] = [
            AppCommand::Select,
            AppCommand::Line,
            AppCommand::Rectangle,
            AppCommand::PushPull,
            AppCommand::Move,
            AppCommand::Measure,
            AppCommand::Orbit,
            AppCommand::Pan,
        ];
        ui.vertical_centered(|ui| {
            for id in TOOLS {
                if id == AppCommand::Orbit {
                    ui.separator();
                }
                let spec = CommandRegistry::spec(id);
                let icon = self.catalog.text(match id {
                    AppCommand::Select => "icon-select",
                    AppCommand::Line => "icon-line",
                    AppCommand::Rectangle => "icon-rectangle",
                    AppCommand::PushPull => "icon-push-pull",
                    AppCommand::Move => "icon-move",
                    AppCommand::Measure => "icon-measure",
                    AppCommand::Orbit => "icon-orbit",
                    AppCommand::Pan => "icon-pan",
                    _ => unreachable!(),
                });
                let active = spec.tool == Some(self.active_tool);
                let button = egui::Button::new(icon).selected(active);
                let enabled = self.command_enabled(id);
                let label = self.catalog.text(spec.label_key);
                let response = ui.add_enabled(enabled, button);
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
            ui.add_space((ui.available_height() - 48.0).max(0.0));
            let enabled = self.command_enabled(AppCommand::Delete);
            let response =
                ui.add_enabled(enabled, egui::Button::new(self.catalog.text("icon-delete")));
            name_widget(&response, enabled, &self.command_label(AppCommand::Delete));
            if response
                .on_hover_text(self.catalog.text("tooltip-delete"))
                .clicked()
            {
                self.dispatch_command(AppCommand::Delete);
            }
        });
    }

    fn prepare_assistant_from_inputs(&mut self) -> bool {
        let Ok(target) = self.assistant_target_input.trim().parse::<u64>() else {
            self.digest = self.catalog.text("assistant-error-target");
            return false;
        };
        let value_text = self.assistant_value_input.clone();
        let intent = match self.assistant_intent_kind {
            AssistantIntentKind::RuleDimension => WorkflowIntent::SetRuleDimension {
                target: NodeId(target),
                value_text,
            },
            AssistantIntentKind::FeatureDimension => WorkflowIntent::SetFeatureDimension {
                target: FeatureId(target),
                value_text,
            },
        };
        self.prepare_assistant_intent(intent)
    }

    fn assistant_value_text(&self, value: &ProposalValue) -> String {
        match value {
            ProposalValue::Missing => self.catalog.text("assistant-value-missing"),
            ProposalValue::Dimension(value) => self.catalog.format(
                "assistant-value-dimension",
                &BTreeMap::from([("value", value.source_token().to_owned())]),
            ),
            ProposalValue::Digest(digest) => self.catalog.format(
                "assistant-value-digest",
                &BTreeMap::from([("digest", digest.chars().take(12).collect())]),
            ),
        }
    }

    fn show_assistant(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.catalog.text("assistant-title"));
        ui.small(self.catalog.text("assistant-boundary"));
        egui::ComboBox::from_label(self.catalog.text("assistant-intent"))
            .selected_text(self.catalog.text(match self.assistant_intent_kind {
                AssistantIntentKind::RuleDimension => "assistant-intent-rule",
                AssistantIntentKind::FeatureDimension => "assistant-intent-feature",
            }))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.assistant_intent_kind,
                    AssistantIntentKind::RuleDimension,
                    self.catalog.text("assistant-intent-rule"),
                );
                ui.selectable_value(
                    &mut self.assistant_intent_kind,
                    AssistantIntentKind::FeatureDimension,
                    self.catalog.text("assistant-intent-feature"),
                );
            });
        egui::Grid::new("assistant-intent-inputs").show(ui, |ui| {
            ui.label(self.catalog.text("assistant-target"));
            ui.text_edit_singleline(&mut self.assistant_target_input);
            ui.end_row();
            ui.label(self.catalog.text("assistant-value"));
            ui.text_edit_singleline(&mut self.assistant_value_input);
            ui.end_row();
        });
        if ui.button(self.catalog.text("assistant-preview")).clicked() {
            self.prepare_assistant_from_inputs();
        }

        let mut confirm = false;
        let mut cancel = false;
        if let Some(proposal) = self.assistant_proposal.as_ref() {
            ui.separator();
            ui.strong(self.catalog.text("assistant-review-title"));
            let goal = self.catalog.text(match proposal.goal() {
                ProposalGoal::SetRuleDimension(_) => "assistant-goal-rule",
                ProposalGoal::SetFeatureDimension(_) => "assistant-goal-feature",
                ProposalGoal::CanonicalPreview => "assistant-goal-canonical",
            });
            ui.label(goal);
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
            ui.small(self.catalog.text("assistant-confirmation-review"));
            for entry in proposal.authoritative_diff() {
                ui.monospace(self.catalog.format(
                    "assistant-diff-row",
                    &BTreeMap::from([
                        ("before", self.assistant_value_text(&entry.before)),
                        ("after", self.assistant_value_text(&entry.after)),
                    ]),
                ));
            }
            ui.horizontal(|ui| {
                confirm = ui.button(self.catalog.text("assistant-confirm")).clicked();
                cancel = ui.button(self.catalog.text("assistant-cancel")).clicked();
            });
        }
        if confirm {
            self.confirm_assistant_proposal();
        } else if cancel {
            self.cancel_assistant_proposal();
        }

        if let Some(verification) = self.assistant_verification.as_ref() {
            ui.small(self.catalog.format(
                "assistant-verification",
                &BTreeMap::from([
                    ("revision", verification.revision_id.to_string()),
                    ("writes", verification.verified_write_count.to_string()),
                ]),
            ));
        }
        ui.separator();
    }

    fn show_bottle_workflow(&mut self, ui: &mut egui::Ui) {
        ui.heading("M6 editable bottle");
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
                .add_enabled(export_ready, egui::Button::new("Export exact recipe"))
                .clicked();
            let mesh = ui
                .add_enabled(export_ready, egui::Button::new("Export mesh + loss report"))
                .clicked();
            if exact
                && let Some(path) = self.dialogs.pick_export_path(ExportRequest {
                    filter_label: "Ketchup exact bottle recipe",
                    extension: "kbex",
                    suggested_name: "editable-bottle.kbex",
                })
            {
                self.export_bottle_exact_recipe_to(definition_id, &path);
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
        ui.heading("Beam A / M5 exact fabrication");
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

    fn show_outliner(&mut self, ui: &mut egui::Ui) {
        self.show_assistant(ui);
        self.show_bottle_workflow(ui);
        self.show_beam_m4ae(ui);
        let groups = self.outliner_groups();
        let entries = self.outliner_query();
        let occurrence_count = entries
            .iter()
            .map(|entry| entry.occurrences.len())
            .sum::<usize>();
        ui.horizontal(|ui| {
            ui.strong(self.catalog.text("dock-outliner"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.monospace(self.catalog.format(
                    "outliner-meta",
                    &BTreeMap::from([
                        ("occurrences", occurrence_count.to_string()),
                        ("definitions", entries.len().to_string()),
                    ]),
                ));
            });
        });
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
                for definition in entries {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        let additive = ui.input(|input| input.modifiers.shift);
                        let count = definition.occurrences.len();
                        let heading = self.catalog.format(
                            "outliner-definition",
                            &BTreeMap::from([
                                ("name", definition.name.clone()),
                                ("count", count.to_string()),
                            ]),
                        );
                        if ui.button(heading).clicked() {
                            self.select_definition(definition.id, additive);
                        }
                        ui.monospace(definition.specification);
                        for occurrence in definition.occurrences {
                            let mut arguments = BTreeMap::from([
                                ("name", occurrence.name),
                                ("position", occurrence.position),
                                (
                                    "visibility",
                                    self.catalog.text(if occurrence.visible {
                                        "visibility-shown"
                                    } else {
                                        "visibility-hidden"
                                    }),
                                ),
                            ]);
                            let key = if let Some(group_id) = occurrence.parent {
                                arguments.insert("group", group_id.0.to_string());
                                "outliner-occurrence-grouped"
                            } else {
                                "outliner-occurrence"
                            };
                            let label = self.catalog.format(key, &arguments);
                            let response = ui.selectable_label(
                                self.selection.contains(&occurrence.instance_path),
                                label,
                            );
                            if response.double_clicked() {
                                self.enter_occurrence_context(occurrence.instance_path.clone());
                            } else if response.clicked() {
                                let additive = ui.input(|input| input.modifiers.shift);
                                self.select_from_outliner(occurrence.instance_path, additive);
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(self.catalog.text("dock-tags"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
        ui.horizontal_centered(|ui| {
            ui.label(self.catalog.format(
                "status-tool",
                &BTreeMap::from([("tool", self.catalog.text(self.active_tool.label_key()))]),
            ));
            ui.label(self.catalog.format(
                "status-selected",
                &BTreeMap::from([("count", self.selection_count().to_string())]),
            ));
            if let Some(edit_context) = self.selection.edit_context.last() {
                let (key, id) = match edit_context {
                    EditContext::Group(id) => ("status-editing-group", id.0),
                    EditContext::Definition { definition_id, .. } => {
                        ("status-editing-component", definition_id.0)
                    }
                };
                ui.label(
                    self.catalog
                        .format(key, &BTreeMap::from([("id", id.to_string())])),
                );
            }
            ui.add_space(ui.available_width().max(0.0) * 0.25);
            ui.label(self.catalog.text("status-snap-on"));
            ui.label(
                self.catalog
                    .format("status-grid", &BTreeMap::from([("step", "10".to_owned())])),
            );
            ui.label(self.catalog.text("status-refs-guaranteed"));
            if self.exact_results.is_empty() {
                ui.label(self.catalog.text("status-exact-unavailable"));
            } else {
                ui.label(self.catalog.format(
                    "status-exact-current",
                    &BTreeMap::from([
                        ("bodies", self.exact_render_body_count().to_string()),
                        ("refs", self.exact_stable_reference_count().to_string()),
                    ]),
                ));
            }
        });
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
    /// Draw the whole designed shell into an `egui` context.
    ///
    /// This is the single entry point used both by the windowed `eframe`
    /// integration and by the offscreen [`crate::testing::HeadlessShell`].
    pub fn ui(&mut self, context: &egui::Context) {
        self.refresh_exact_products(context);
        self.refresh_beam_m5_products(context);
        context.set_visuals(egui::Visuals::dark());
        self.handle_shortcuts(context);

        let panel_frame = egui::Frame::new()
            .fill(Color32::from_rgb(23, 25, 28))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(43, 48, 54)));
        egui::TopBottomPanel::top("top-bar")
            .exact_height(46.0)
            .frame(panel_frame)
            .show(context, |ui| self.show_top_bar(ui));
        egui::TopBottomPanel::top("menu-bar")
            .exact_height(28.0)
            .frame(panel_frame)
            .show(context, |ui| self.show_menu_bar(ui));
        egui::TopBottomPanel::bottom("status-bar")
            .exact_height(26.0)
            .frame(panel_frame)
            .show(context, |ui| self.show_status_bar(ui));
        egui::SidePanel::left("tool-rail")
            .resizable(false)
            .exact_width(56.0)
            .frame(panel_frame)
            .show(context, |ui| self.show_tool_rail(ui));
        egui::SidePanel::right("right-dock")
            .resizable(false)
            .exact_width(320.0)
            .frame(panel_frame)
            .show(context, |ui| self.show_outliner(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(15, 17, 19)))
            .show(context, |ui| self.viewport(ui));
        self.show_shortcuts_window(context);
    }
}

/// Give a widget an accessible name.
///
/// Icon-only controls paint a glyph, which is useless both to a screen reader
/// and to an acceptance test. This publishes the command's localized name to
/// the accessibility tree instead.
fn name_widget(response: &egui::Response, enabled: bool, name: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
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
    if !selection.instance_path.is_root() {
        return None;
    }
    let occurrence_id = selection.instance_path.root_occurrence();
    let occurrence = snapshot.occurrence(occurrence_id)?;
    if occurrence.definition_id() != selection.definition_id
        || item.definition_id != selection.definition_id
        || item.instance_path != selection.instance_path
    {
        return None;
    }
    let profile = snapshot.feature(item.profile_feature_id)?;
    let extrusion = snapshot.feature(item.extrusion_feature_id)?;
    if profile.definition_id() != selection.definition_id
        || extrusion.definition_id() != selection.definition_id
        || !matches!(
            extrusion.kind(),
            FeatureKind::Extrusion { profile, .. } if *profile == item.profile_feature_id
        )
    {
        return None;
    }
    let ElementId::Face { axis, side } = selection.element else {
        return None;
    };
    let mut commands = Vec::new();
    match axis {
        Axis::Z => {
            commands.push(CanonicalCommand::SetFeatureDimension {
                id: item.extrusion_feature_id,
                dimension: Dimension::new(source_token, new_extent_mm).ok()?,
            });
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
        ExactFaceRole::Top => Some(ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        }),
        ExactFaceRole::Bottom => Some(ElementId::Face {
            axis: Axis::Z,
            side: Side::Minimum,
        }),
        ExactFaceRole::East | ExactFaceRole::CutEast => Some(ElementId::Face {
            axis: Axis::X,
            side: Side::Maximum,
        }),
        ExactFaceRole::CutWest => Some(ElementId::Face {
            axis: Axis::X,
            side: Side::Minimum,
        }),
        ExactFaceRole::CutSouth => Some(ElementId::Face {
            axis: Axis::Y,
            side: Side::Minimum,
        }),
        ExactFaceRole::CutNorth => Some(ElementId::Face {
            axis: Axis::Y,
            side: Side::Maximum,
        }),
        ExactFaceRole::RevolveBottom
        | ExactFaceRole::RevolveBody
        | ExactFaceRole::RevolveShoulder
        | ExactFaceRole::RevolveNeck
        | ExactFaceRole::RevolveMouth
        | ExactFaceRole::ShellOuterBottom
        | ExactFaceRole::ShellOuterBody
        | ExactFaceRole::ShellOuterShoulder
        | ExactFaceRole::ShellOuterNeck
        | ExactFaceRole::ShellRim
        | ExactFaceRole::ShellInnerBottom
        | ExactFaceRole::ShellInnerBody
        | ExactFaceRole::ShellInnerShoulder
        | ExactFaceRole::ShellInnerNeck => None,
    }
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

fn projected_bounds(points: &[Vec3], project: impl Fn(Vec3) -> Pos2) -> Rect {
    points
        .iter()
        .map(|point| Rect::from_min_max(project(*point), project(*point)))
        .reduce(|left, right| left.union(right))
        .unwrap_or(Rect::ZERO)
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
        let mut app = KetchupApp::new();
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
        assert!(loss.contains("feature editability"));

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
        let mut app = KetchupApp::new();
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
    fn running_app_uses_one_exact_cut_body_for_render_pick_and_export() {
        let executable = exact_worker_executable();
        assert!(
            executable.is_file(),
            "build workspace all-targets so the exact worker exists at {}",
            executable.display()
        );
        let mut app = KetchupApp::new();
        app.document = through_cut_document();
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
        assert_eq!(item.extrusion_feature_id, FeatureId(2));
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
    fn rectangle_sketch_creates_an_undoable_solid() {
        let mut app = KetchupApp::new();
        app.sketch_height_input = "30".to_owned();

        assert!(
            app.complete_rectangle_sketch(Vec3::new(40.0, 25.0, 0.0), Vec3::new(-10.0, -5.0, 0.0),)
        );
        assert_eq!(app.active_box_count(), 2);
        assert_eq!(app.active_boxes()[1].origin_mm, Vec3::new(-10.0, -5.0, 0.0));
        assert_eq!(app.active_boxes()[1].size_mm, Vec3::new(50.0, 30.0, 30.0));

        assert!(app.undo());
        assert_eq!(app.active_box_count(), 1);
        assert!(app.redo());
        assert_eq!(app.active_boxes()[1].size_mm, Vec3::new(50.0, 30.0, 30.0));
    }

    #[test]
    fn push_pull_keeps_the_opposite_face_fixed_on_screen() {
        let mut app = KetchupApp::new();
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
    fn viewport_omits_edge_on_faces_that_collapse_to_a_line() {
        let mut app = KetchupApp::new();
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
    fn viewport_picks_the_geometry_currently_shown_in_preview() {
        let mut app = KetchupApp::new();
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
        assert_eq!(created.size_mm, Vec3::new(300.0, 200.0, 20.0));
        assert_eq!(app.document.visible_undo_steps(), 1);

        app.dispatch_command(AppCommand::PushPull);
        app.value_input = "55".to_owned();
        assert!(app.apply_value_input());
        assert_eq!(app.active_boxes()[1].size_mm.z, 75.0);
        assert_eq!(app.document.visible_undo_steps(), 2);

        assert!(app.undo());
        assert_eq!(app.active_boxes()[1].size_mm.z, 20.0);
        assert!(app.undo());
        assert_eq!(app.active_box_count(), 1);
        assert!(app.redo());
        assert!(app.redo());
        assert_eq!(app.active_boxes()[1].size_mm.z, 75.0);
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
    fn nested_mutations_are_rejected_without_revision_or_digest_changes() {
        let mut app = KetchupApp::new();
        assert!(app.create_box());
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
        assert!(app.group_selected());
        assert!(app.make_component());
        let nested = app.active_boxes()[0].clone();
        app.selection.select_exact(
            SelectionId {
                definition_id: nested.definition_id,
                instance_path: nested.instance_path,
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        let revision = app.document_revision();
        let digest = app.document.current().canonical_digest();
        app.set_push_pull_distance_input("5");
        assert!(!app.start_preview());
        assert!(!app.move_selected(Vec3::new(10.0, 0.0, 0.0)));
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.document.current().canonical_digest(), digest);
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

        let mut reopened = KetchupApp::new();
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
