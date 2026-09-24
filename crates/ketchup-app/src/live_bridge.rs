//! Opt-in live GUI bridge v1. No independent document/session or scripting engine.
//! Wire: u32 big-endian byte length, then UTF-8 JSON `Envelope`; responses use
//! the same framing. Both directions are capped at 32 KiB. At most four active clients,
//! one in-flight request per client, queue capacity 8, at most 4 requests per UI frame.
//! Authentication is required on EVERY request. Never publish credentials in
//! logs, command lines, environment variables, documents, or ordinary UI output.
//! Only a trusted embedding host should use `live_bridge_credentials` and carry
//! its result over an independently trusted out-of-band channel.
//! Proposals are connection-local; disconnect drops them. Mutations never replay:
//! an optional `expected` stamp only guards against a concurrent human edit.
//! Images are callback-correlated CAD-only PNG thumbnails; geometry completeness is not claimed.

use crate::{ActiveTool, AppCommand, KetchupApp, SelectionId, WorkRecoveryMutationError};
use eframe::egui;
use ketchup_application::{
    batch_task::{
        OccurrenceBatchDocument, OccurrenceBatchError, OccurrenceBatchOperation,
        OccurrenceBatchState, OccurrenceBatchTask,
    },
    evaluation::{
        EvaluationReport, ExactEvaluationSelection, ExactEvaluationTask, exact_source,
        exact_worker_candidates, materialize_exact_products, plan_incremental_exact_evaluation,
        start_exact_evaluation_scoped_with_cancellation,
    },
    model_query::{EditContextRequest, EntityKind, ModelQuery, PageRequest},
    validation::{
        AssistantValidationSelection, assistant_validation_context_with_worker_cancellation,
    },
};
use ketchup_core::{
    assistant_sidecar::{
        AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
        AssistantInstancePath, AssistantRejectionDiagnostic,
    },
    document::{
        CommandBatch, DocumentStore, OccurrenceId, Proposal, Snapshot, VerifiedProposalCommit,
    },
    exact_product::ExactResultRegistry,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque, hash_map::RandomState},
    hash::BuildHasher,
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub mod bootstrap;
pub mod consent;
mod image;
#[cfg(test)]
mod tests;
mod transport;
pub const MAX_FRAME_BYTES: usize = 32 * 1024;
pub const MAX_IMAGE_FRAME_BYTES: usize = 12 * 1024 * 1024;
pub const MIN_IMAGE_SIDE_PX: u32 = 512;
pub const MAX_IMAGE_SIDE_PX: u32 = 1600;
pub const QUEUE_CAPACITY: usize = 8;
pub const MAX_SELECTION: usize = 100;
pub const MAX_APPLY_VERIFY_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_APPLY_VERIFY_TIMEOUT_MS: u64 = 60_000;
const fn default_apply_verify_timeout_ms() -> u64 {
    DEFAULT_APPLY_VERIFY_TIMEOUT_MS
}
pub const MAX_BATCH_JOBS: usize = 16;
pub const IMAGE_PROTOCOL_VERSION: u32 = 4;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub document_id: u64,
    pub revision: u64,
    pub canonical_digest: String,
    pub mutation_epoch: u64,
}

/// Intentionally no Debug or Serialize implementation: trusted API only.
pub struct Credentials {
    pub address: SocketAddr,
    pub token: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u32,
    pub id: u64,
    pub token: String,
    pub request: Request,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApplyAndVerifySave {
    Current {},
    Path { path: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status {},
    Summary {},
    EditContext {
        #[serde(default)]
        expected: Option<Stamp>,
        targets: Vec<AssistantInstancePath>,
    },
    Query {
        #[serde(default)]
        expected: Option<Stamp>,
        query: PageRequest,
    },
    Detail {
        #[serde(default)]
        expected: Option<Stamp>,
        kind: EntityKind,
        entity_id: u64,
    },
    WorksetCreate {
        #[serde(default)]
        expected: Option<Stamp>,
        query: PageRequest,
    },
    WorksetStatus {
        #[serde(default)]
        expected: Option<Stamp>,
        handle: String,
    },
    BatchJobStart {
        #[serde(default)]
        expected: Option<Stamp>,
        workset_handle: String,
        operation: OccurrenceBatchOperation,
    },
    BatchJobStatus {
        #[serde(default)]
        expected: Option<Stamp>,
        handle: String,
    },
    BatchJobStep {
        #[serde(default)]
        expected: Option<Stamp>,
        handle: String,
    },
    BatchJobCancel {
        #[serde(default)]
        expected: Option<Stamp>,
        handle: String,
    },
    Propose {
        #[serde(default)]
        expected: Option<Stamp>,
        #[serde(default)]
        selection: Option<Vec<u64>>,
        program: AssistantCadEditProgram,
    },
    Commit {
        #[serde(default)]
        expected: Option<Stamp>,
        proposal_id: u64,
    },
    /// Plan, evaluate exact geometry, run collision (plus any extra
    /// `validators`, e.g. gravity_support on demand) and publish as one Undo step. `expected` and
    /// `selection` are optional guards against concurrent human edits.
    ApplyAndVerify {
        #[serde(default)]
        expected: Option<Stamp>,
        #[serde(default)]
        selection: Option<Vec<u64>>,
        program: AssistantCadEditProgram,
        #[serde(default)]
        validators: Vec<String>,
        #[serde(default = "default_apply_verify_timeout_ms")]
        timeout_ms: u64,
        #[serde(default)]
        save: Option<ApplyAndVerifySave>,
    },
    Undo {
        #[serde(default)]
        expected: Option<Stamp>,
    },
    Redo {
        #[serde(default)]
        expected: Option<Stamp>,
    },
    Save {
        #[serde(default)]
        expected: Option<Stamp>,
    },
    SaveAs {
        #[serde(default)]
        expected: Option<Stamp>,
        path: String,
    },
    Open {
        #[serde(default)]
        expected: Option<Stamp>,
        path: String,
    },
    Selection {
        #[serde(default)]
        expected: Option<Stamp>,
        occurrence_ids: Vec<u64>,
    },
    View {
        #[serde(default)]
        expected: Option<Stamp>,
        view: View,
    },
    Image {
        #[serde(default)]
        expected: Option<Stamp>,
        image_protocol_version: u32,
        capture_mode: CaptureMode,
        max_side_px: u32,
        #[serde(default)]
        framing: ImageFraming,
        #[serde(default)]
        detail_target: Option<ImageDetailTarget>,
    },
    Disconnect {},
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    #[default]
    Offscreen,
    VisibleViewport,
}

impl CaptureMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Offscreen => "offscreen",
            Self::VisibleViewport => "visible_viewport",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageFraming {
    #[default]
    Viewport,
    Selection,
    DetailSelection,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImageDetailTarget {
    pub occurrence_id: u64,
    pub kind: EntityKind,
    pub entity_id: u64,
}

impl ImageFraming {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Viewport => "viewport",
            Self::Selection => "selection",
            Self::DetailSelection => "detail_selection",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Iso,
    Top,
    Front,
    ZoomFit,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    pub ok: bool,
    pub stamp: Option<Stamp>,
    pub result: Option<Value>,
    pub error: Option<String>,
}
impl Response {
    fn error(id: u64, code: &str) -> Self {
        Self {
            version: 1,
            id,
            ok: false,
            stamp: None,
            result: None,
            error: Some(code.into()),
        }
    }

    fn capability_gap(id: u64, diagnostic: &AssistantRejectionDiagnostic) -> Self {
        Self {
            version: 1,
            id,
            ok: false,
            stamp: None,
            result: Some(json!({
                "kind": "capability_gap",
                "capability": diagnostic.code,
                "operation": diagnostic.operation,
                "retryable": false,
                "published": false,
            })),
            error: Some("capability_gap".into()),
        }
    }
}

struct Queued {
    session: u64,
    id: u64,
    request: Request,
    connection_closed: bool,
    cancelled: Arc<AtomicBool>,
    reply: mpsc::SyncSender<Response>,
}
/// GUI selection the caller asserted, re-checked right before publication.
type SelectionGuard = Option<(Vec<u64>, Option<SelectionId>)>;
struct Pending {
    id: u64,
    epoch: u64,
    selection: SelectionGuard,
    proposal: Proposal,
}
enum PlanRejection {
    Code(&'static str),
    CapabilityGap(Box<AssistantRejectionDiagnostic>),
}
impl From<&'static str> for PlanRejection {
    fn from(code: &'static str) -> Self {
        Self::Code(code)
    }
}
pub(crate) struct ApplyAndVerifyRequest {
    expected: Option<Stamp>,
    selection: Option<Vec<u64>>,
    program: AssistantCadEditProgram,
    validators: Vec<String>,
    timeout_ms: u64,
    save: Option<ApplyAndVerifySave>,
}
/// Everything decided on the UI thread before the off-thread exact/validation work.
struct ApplyAndVerifyPlan {
    before: Stamp,
    selection: SelectionGuard,
    proposal: Proposal,
    candidate: Snapshot,
    save_path: Option<PathBuf>,
    timeout_ms: u64,
    started: Instant,
    planned_at: Instant,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplyAndVerifyFault {
    Planning,
    Candidate,
    Exact,
    Validation,
    Publication,
}
struct PreparedApplyAndVerify {
    candidate_exact: ExactResultRegistry,
    candidate_topology: ExactResultRegistry,
    exact_report: EvaluationReport,
    validation: Value,
    exact_at: Instant,
    validated_at: Instant,
    _exact_task: ExactEvaluationTask,
}
struct ApplyAndVerifyJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    worker_cancelled: Arc<AtomicBool>,
    plan: ApplyAndVerifyPlan,
    receiver: mpsc::Receiver<Result<PreparedApplyAndVerify, &'static str>>,
}
struct BatchJob {
    handle: String,
    task: OccurrenceBatchTask,
}

struct ClientState {
    query: ModelQuery,
    pending: Option<Pending>,
    next_proposal: u64,
    batch_jobs: VecDeque<BatchJob>,
    batch_job_key: RandomState,
    next_batch_job: u64,
}

impl Default for ClientState {
    fn default() -> Self {
        Self {
            query: ModelQuery::default(),
            pending: None,
            next_proposal: 1,
            batch_jobs: VecDeque::new(),
            batch_job_key: RandomState::new(),
            next_batch_job: 1,
        }
    }
}

impl OccurrenceBatchDocument for KetchupApp {
    fn batch_snapshot(&self) -> Snapshot {
        self.document.current()
    }

    fn batch_mutation_epoch(&self) -> u64 {
        self.document.mutation_epoch()
    }

    fn batch_plan(&self, batch: CommandBatch) -> Result<Proposal, OccurrenceBatchError> {
        <DocumentStore as OccurrenceBatchDocument>::batch_plan(&self.document, batch)
    }

    fn batch_commit(
        &mut self,
        proposal: &Proposal,
    ) -> Result<VerifiedProposalCommit, OccurrenceBatchError> {
        self.commit_verified_proposal_with_work_recovery(proposal)
            .map_err(|_| OccurrenceBatchError::HostTransaction)
    }
}

pub(crate) struct LiveBridge {
    address: SocketAddr,
    token: String,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    #[cfg(test)]
    active_connections: Arc<AtomicUsize>,
    queue: mpsc::Receiver<Queued>,
    query: ModelQuery,
    observed: Option<Stamp>,
    session: u64,
    client_states: BTreeMap<u64, ClientState>,
    pending: Option<Pending>,
    next_proposal: u64,
    apply_and_verify_job: Option<ApplyAndVerifyJob>,
    #[cfg(test)]
    apply_and_verify_fault: Option<ApplyAndVerifyFault>,
    batch_jobs: VecDeque<BatchJob>,
    batch_job_key: RandomState,
    next_batch_job: u64,
    image: image::ImageState,
}
impl Drop for LiveBridge {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        // Idle/frame reads poll stop; writes have a total two-second deadline. Never join on the UI.
        self.worker.take();
    }
}

impl KetchupApp {
    /// Explicit opt-in by a trusted UI-thread host; binds ONLY 127.0.0.1:0.
    /// Generates fresh credentials; launcher-supplied tokens use the separate bootstrap API.
    pub fn enable_live_bridge(&mut self, context: &egui::Context) -> io::Result<SocketAddr> {
        if self.live_bridge.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "bridge already enabled",
            ));
        }
        let bridge = transport::start(context.clone())?;
        let address = bridge.address;
        self.live_bridge = Some(bridge);
        Ok(address)
    }

    /// Secret-bearing out-of-band accessor for trusted host code only.
    pub fn live_bridge_credentials(&self) -> Option<Credentials> {
        self.live_bridge.as_ref().map(|b| Credentials {
            address: b.address,
            token: b.token.clone(),
        })
    }

    pub fn disable_live_bridge(&mut self) {
        self.live_bridge = None;
    }

    pub fn live_bridge_stamp(&self) -> Stamp {
        let snapshot = self.document.current();
        Stamp {
            document_id: snapshot.document_id().0,
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            mutation_epoch: self.document.mutation_epoch(),
        }
    }

    pub(crate) fn poll_live_bridge(&mut self, context: &egui::Context) {
        let Some(mut bridge) = self.live_bridge.take() else {
            return;
        };
        let mut revoke_consent = false;
        let ui_busy = context.wants_keyboard_input() || context.is_using_pointer();
        bridge.poll_apply_and_verify_job(self, context, ui_busy);
        for _ in 0..4 {
            let Ok(queued) = bridge.queue.try_recv() else {
                break;
            };
            if queued.connection_closed {
                bridge.remove_client(queued.session);
                if self.live_consent_attached {
                    revoke_consent = true;
                    break;
                }
                continue;
            }
            if queued.cancelled.load(Ordering::Acquire) {
                continue;
            }
            if bridge.session != queued.session
                && (bridge.image.is_pending() || bridge.apply_and_verify_job.is_some())
            {
                let _ = queued.reply.try_send(Response::error(queued.id, "busy"));
                continue;
            }
            bridge.activate_client(queued.session);
            let stamp = self.live_bridge_stamp();
            if bridge.observed.as_ref() != Some(&stamp) {
                bridge.query.invalidate();
                for state in bridge.client_states.values_mut() {
                    state.query.invalidate();
                }
                bridge.observed = Some(stamp);
            }
            if matches!(queued.request, Request::Image { .. }) {
                bridge.request_image(self, context, queued);
                continue;
            }
            let Queued {
                session,
                id,
                request,
                cancelled,
                reply,
                ..
            } = queued;
            let request = match request {
                Request::ApplyAndVerify {
                    expected,
                    selection,
                    program,
                    validators,
                    timeout_ms,
                    save,
                } => {
                    bridge.start_queued_apply_and_verify(
                        self,
                        context,
                        id,
                        reply,
                        cancelled,
                        ApplyAndVerifyRequest {
                            expected,
                            selection,
                            program,
                            validators,
                            timeout_ms,
                            save,
                        },
                        ui_busy,
                    );
                    continue;
                }
                request => request,
            };
            let disconnect = matches!(request, Request::Disconnect {});
            let result = bridge.execute_authorized(self, request, ui_busy, &cancelled);
            let mut response = match result {
                Ok(value) => Response {
                    version: 1,
                    id,
                    ok: true,
                    stamp: None,
                    result: Some(value),
                    error: None,
                },
                Err(code) => Response::error(id, code),
            };
            response.stamp = Some(self.live_bridge_stamp());
            let _ = reply.try_send(response);
            context.request_repaint();
            if disconnect {
                bridge.remove_client(session);
            }
            if disconnect && self.live_consent_attached {
                revoke_consent = true;
                break;
            }
        }
        if revoke_consent {
            self.live_consent_attached = false;
        } else {
            self.live_bridge = Some(bridge);
        }
    }
}

impl LiveBridge {
    fn activate_client(&mut self, session: u64) {
        if self.session == session {
            return;
        }
        if self.session != 0 {
            let state = ClientState {
                query: std::mem::take(&mut self.query),
                pending: self.pending.take(),
                next_proposal: std::mem::replace(&mut self.next_proposal, 1),
                batch_jobs: std::mem::take(&mut self.batch_jobs),
                batch_job_key: std::mem::replace(&mut self.batch_job_key, RandomState::new()),
                next_batch_job: std::mem::replace(&mut self.next_batch_job, 1),
            };
            self.client_states.insert(self.session, state);
        }
        let state = self.client_states.remove(&session).unwrap_or_default();
        self.query = state.query;
        self.pending = state.pending;
        self.next_proposal = state.next_proposal;
        self.batch_jobs = state.batch_jobs;
        self.batch_job_key = state.batch_job_key;
        self.next_batch_job = state.next_batch_job;
        self.session = session;
    }

    fn remove_client(&mut self, session: u64) {
        if self.session != session {
            self.client_states.remove(&session);
            return;
        }
        self.session = 0;
        self.query = ModelQuery::default();
        self.pending = None;
        self.next_proposal = 1;
        self.batch_jobs.clear();
        self.batch_job_key = RandomState::new();
        self.next_batch_job = 1;
        self.image.revoke();
    }

    pub(super) fn invalidate_document_context(&mut self) {
        self.observed = None;
        self.client_states.clear();
        self.pending = None;
        self.batch_jobs.clear();
        self.batch_job_key = RandomState::new();
        self.next_batch_job = 1;
        self.query.invalidate();
        self.image.revoke();
    }

    fn batch_job_handle(&self, id: u64) -> String {
        format!("batch-{id:016x}-{:016x}", self.batch_job_key.hash_one(id))
    }

    // Deliberately inspect raw state: validity-filtered preview helpers can hide stale human work.
    fn busy(app: &KetchupApp) -> bool {
        app.preview.is_some()
            || app.preview_box.is_some()
            || app.preview_definition_id.is_some()
            || app.smart_push_pull_proposal.is_some()
            || app.smart_push_pull_planning.is_some()
            || app.smart_push_pull_chooser.is_some()
            || app.occurrence_operation_preview.is_some()
            || app.solid_tool_target.is_some()
            || app.revolve_tool.is_some()
            || app.revolve_preview.is_some()
            || app.planar_offset_preview.is_some()
            || matches!(app.active_tool, ActiveTool::Helix | ActiveTool::Thread)
            || app.sweep_preview.is_some()
            || app.loft_input_sections.is_some()
            || app.loft_preview.is_some()
            || app.general_finish_preview.is_some()
            || app.pocket_preview.is_some()
            || app.pocket_editor_feature.is_some()
            || app.parameter_editor_node.is_some()
            || app.parameter_provenance.is_some()
            || app.assistant_proposal.is_some()
            || app.assistant_pending_execution.is_some()
            || app.assistant_chat_task.is_some()
            || app.push_pull_drag.is_some()
            || app.push_pull_anchor.is_some()
            || app.transform_gesture_active()
            || app.camera_drag_active
            || app.camera_wheel_active
            || app.zoom_window_start.is_some()
            || app.zoom_window_cursor.is_some()
            || app.sketch_mode
            || app.sketch_start.is_some()
            || app.sketch_end.is_some()
            || app.sketch_cursor.is_some()
            || app.line_chain_origin.is_some()
            || !app.line_chain_points.is_empty()
            || !app.line_chain_items.is_empty()
            || app.focus_value_box
            || app.measure_start.is_some()
            || app.measure_cursor.is_some()
            || app.measure_end.is_some()
            || app.pending_definition_rename.is_some()
            || app.pending_occurrence_rename.is_some()
            || app.pending_component_replacement.is_some()
            || app.pending_tag_creation.is_some()
            || app.pending_tag_deletion.is_some()
            || app.pending_tag_clear.is_some()
            || app.pending_tag_rename.is_some()
            || app.pending_tag_assignment.is_some()
            || app.pending_occurrence_align.is_some()
            || app.pending_occurrence_distribution.is_some()
            || app.pending_linear_pattern.is_some()
            || app.pending_rectangular_pattern.is_some()
            || app.pending_circular_pattern.is_some()
            || app.mesh_conversion_active()
            || app.pending_stl_import.is_some()
            || app.pending_dxf_import.is_some()
            || app.pending_step_import.is_some()
            || app.pending_iges_import.is_some()
            || app.pending_sketchup_scene_import.is_some()
            || app.pending_glb_import.is_some()
            || app.migration_review_plan.is_some()
            || app.assembly_preview_pending()
            || app.body_preview_pending()
            || app.feature_history_preview_pending()
            || app.face_workflow.xray_preview()
            || Self::fixture_busy(app)
    }
    fn fixture_busy(_app: &KetchupApp) -> bool {
        false
    }
    fn available(app: &KetchupApp, ui_busy: bool) -> Result<(), &'static str> {
        if app.review_candidate.is_some() {
            return Err("read_only_document");
        }
        if ui_busy || Self::busy(app) {
            return Err("busy");
        }
        Ok(())
    }
    /// Every document mutation (including Undo/Redo and Open) draws a fresh
    /// process-unique epoch, so the epoch alone detects any intervening change.
    fn guard(app: &KetchupApp, expected: &Option<Stamp>) -> Result<(), &'static str> {
        match expected {
            Some(stamp) if stamp.mutation_epoch != app.document.mutation_epoch() => {
                Err("stale_document")
            }
            _ => Ok(()),
        }
    }

    fn selection_guard(
        app: &KetchupApp,
        selection: Option<Vec<u64>>,
    ) -> Result<SelectionGuard, &'static str> {
        let Some(selection) = selection else {
            return Ok(None);
        };
        let selection = Self::validate_ids(&selection)?;
        if selection != Self::selection(app)? {
            return Err("selection_changed");
        }
        Ok(Some((selection, app.selection.primary.clone())))
    }

    fn check_selection_guard(app: &KetchupApp, guard: &SelectionGuard) -> Result<(), &'static str> {
        match guard {
            Some((ids, primary))
                if *ids != Self::selection(app)? || *primary != app.selection.primary =>
            {
                Err("selection_changed")
            }
            _ => Ok(()),
        }
    }

    fn selection(app: &KetchupApp) -> Result<Vec<u64>, &'static str> {
        if !app.selection.edit_context.is_empty()
            || app.selection.selected_group.is_some()
            || !app.selection.topological.is_empty()
            || app.selected_instance_paths().iter().any(|p| !p.is_root())
        {
            return Err("unsupported_selection_scope");
        }
        let selected = app.selected_occurrence_ids();
        if selected.len() > MAX_SELECTION {
            return Err("selection_limit");
        }
        let ids: Vec<_> = selected.iter().map(|id| id.0).collect();
        Self::root_ids(app, &ids)?;
        Ok(ids)
    }

    fn root_ids(app: &KetchupApp, ids: &[u64]) -> Result<(), &'static str> {
        if ids.is_empty() {
            return Ok(());
        }
        let snapshot = app.document.current();
        for id in ids {
            let occurrence = snapshot
                .occurrence(OccurrenceId(*id))
                .ok_or("entity_not_found")?;
            if occurrence.parent().is_some() {
                return Err("unsupported_selection_scope");
            }
        }
        let selectable: BTreeSet<_> = app
            .active_scene_query()
            .into_iter()
            .filter(|o| o.visible && o.parent.is_none() && o.instance_path.is_root())
            .map(|o| o.instance_path.root_occurrence().0)
            .collect();
        if ids.iter().any(|id| !selectable.contains(id)) {
            return Err("unsupported_selection_scope");
        }
        Ok(())
    }
    fn program_scope(
        app: &KetchupApp,
        program: &AssistantCadEditProgram,
    ) -> Result<(), &'static str> {
        for operation in &program.operations {
            let selector = match operation {
                AssistantCadEditOperation::Delete { selector, .. }
                | AssistantCadEditOperation::Transform { selector, .. }
                | AssistantCadEditOperation::SetColor { selector, .. }
                | AssistantCadEditOperation::SetGrounded { selector, .. }
                | AssistantCadEditOperation::SetOccurrenceClassification { selector, .. }
                | AssistantCadEditOperation::Copy { selector, .. }
                | AssistantCadEditOperation::LinearPattern { selector, .. }
                | AssistantCadEditOperation::CircularPattern { selector, .. }
                | AssistantCadEditOperation::Mirror { selector, .. } => Some(selector),
                AssistantCadEditOperation::CreateSketch { .. }
                | AssistantCadEditOperation::CreateProgramSketch { .. }
                | AssistantCadEditOperation::CreatePart { .. }
                | AssistantCadEditOperation::CreatePanel { .. }
                | AssistantCadEditOperation::CreateDowelJoint { .. }
                | AssistantCadEditOperation::CreateProgramDowelJoint { .. }
                | AssistantCadEditOperation::CreatePhysicalDowelJoint { .. }
                | AssistantCadEditOperation::DeletePhysicalDowelJoint { .. }
                | AssistantCadEditOperation::CreateTag { .. }
                | AssistantCadEditOperation::SetOccurrenceTag { .. }
                | AssistantCadEditOperation::SetTagVisibility { .. }
                | AssistantCadEditOperation::CreateSpatialPath { .. }
                | AssistantCadEditOperation::CreateHelixPath { .. }
                | AssistantCadEditOperation::CreateConstructionPoint { .. }
                | AssistantCadEditOperation::CreateConstructionAxis { .. }
                | AssistantCadEditOperation::CreateConstructionPlane { .. }
                | AssistantCadEditOperation::CreateHelix { .. }
                | AssistantCadEditOperation::CreateThread { .. }
                | AssistantCadEditOperation::FilletEdges { .. }
                | AssistantCadEditOperation::ChamferEdges { .. }
                | AssistantCadEditOperation::AppendFeature { .. }
                | AssistantCadEditOperation::AppendProgramPocket { .. }
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
            };
            if let Some(AssistantCadEntitySelector::Occurrences { occurrence_ids }) = selector {
                Self::root_ids(app, occurrence_ids)?;
            }
        }
        Ok(())
    }
    fn require_request_authority(cancelled: &AtomicBool) -> Result<(), &'static str> {
        if cancelled.load(Ordering::Acquire) {
            Err("request_cancelled")
        } else {
            Ok(())
        }
    }

    fn validate_ids(ids: &[u64]) -> Result<Vec<u64>, &'static str> {
        if ids.len() > MAX_SELECTION || ids.contains(&0) {
            return Err("invalid_selection");
        }
        let sorted: BTreeSet<_> = ids.iter().copied().collect();
        if sorted.len() != ids.len() {
            return Err("invalid_selection");
        }
        Ok(sorted.into_iter().collect())
    }

    fn mandatory_validation_selection(
        validators: &[String],
    ) -> Result<AssistantValidationSelection, &'static str> {
        let mut validator_names = vec!["collision"];
        for name in validators {
            if !validator_names.contains(&name.as_str()) {
                validator_names.push(name.as_str());
            }
        }
        let selection = AssistantValidationSelection::only(&validator_names);
        if !selection.is_valid() {
            return Err("unknown_validator");
        }
        Ok(selection)
    }

    fn is_capability_gap(diagnostic: &AssistantRejectionDiagnostic) -> bool {
        diagnostic.code.ends_with("_unsupported")
    }

    fn reply_capability_gap(
        app: &KetchupApp,
        id: u64,
        reply: &mpsc::SyncSender<Response>,
        diagnostic: &AssistantRejectionDiagnostic,
    ) {
        let mut response = Response::capability_gap(id, diagnostic);
        response.stamp = Some(app.live_bridge_stamp());
        let _ = reply.try_send(response);
    }

    fn reply(
        app: &KetchupApp,
        id: u64,
        reply: &mpsc::SyncSender<Response>,
        result: Result<Value, &'static str>,
    ) {
        let mut response = match result {
            Ok(value) => Response {
                version: 1,
                id,
                ok: true,
                stamp: None,
                result: Some(value),
                error: None,
            },
            Err(code) => Response::error(id, code),
        };
        response.stamp = Some(app.live_bridge_stamp());
        let _ = reply.try_send(response);
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_apply_and_verify_candidate(
        candidate: &Snapshot,
        container_data: &ketchup_core::persistence::ContainerData,
        render: &ExactResultRegistry,
        topology: &ExactResultRegistry,
        worker_path: Option<PathBuf>,
        exact_selection: &ExactEvaluationSelection,
        validation_selection: &AssistantValidationSelection,
        started: Instant,
        timeout_ms: u64,
        cancelled: Arc<AtomicBool>,
        #[cfg(test)] fault: Option<ApplyAndVerifyFault>,
    ) -> Result<PreparedApplyAndVerify, &'static str> {
        let deadline = Duration::from_millis(timeout_ms);
        #[cfg(test)]
        if fault == Some(ApplyAndVerifyFault::Exact) {
            return Err("exact_evaluation_rejected");
        }
        let scope = match exact_selection {
            ExactEvaluationSelection::Full => None,
            ExactEvaluationSelection::Scoped(producers) => Some(producers),
        };
        let exact_task = start_exact_evaluation_scoped_with_cancellation(
            candidate.clone(),
            container_data,
            render,
            topology,
            worker_path.clone(),
            scope,
            Arc::clone(&cancelled),
            || {},
        );
        let remaining = deadline
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or("job_timeout")?;
        let products = exact_task.wait(remaining).map_err(|error| {
            // A wait timeout also raises the shared cancel flag; report it as a timeout.
            if error.contains("timed out") {
                "job_timeout"
            } else if cancelled.load(Ordering::Acquire) {
                "request_cancelled"
            } else if error.contains("cancelled") {
                "request_cancelled"
            } else if error.contains("disconnected") {
                "exact_worker_disconnected"
            } else if error.contains("unavailable") {
                "exact_worker_unavailable"
            } else {
                "exact_evaluation_rejected"
            }
        })?;
        Self::require_request_authority(&cancelled)?;
        let (candidate_exact, candidate_topology, exact_report) =
            materialize_exact_products(candidate, render, topology, &exact_task, products)
                .map_err(|_| "exact_evaluation_rejected")?;
        if !exact_report.complete || !exact_report.topology_complete {
            return Err("exact_evaluation_incomplete");
        }
        let exact_at = Instant::now();
        let remaining = deadline
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or("job_timeout")?;
        #[cfg(test)]
        if fault == Some(ApplyAndVerifyFault::Validation) {
            return Err("validation_failed");
        }
        let validation = assistant_validation_context_with_worker_cancellation(
            candidate,
            &candidate_exact,
            validation_selection,
            container_data,
            worker_path,
            remaining,
            Arc::clone(&cancelled),
        );
        if started.elapsed() > deadline {
            cancelled.store(true, Ordering::Release);
            return Err("job_timeout");
        }
        Self::require_request_authority(&cancelled)?;
        if validation["state"] == "failed" {
            return Err("validation_failed");
        }
        if validation["state"] != "passed" || validation["complete"] != true {
            return Err("validation_incomplete");
        }
        Ok(PreparedApplyAndVerify {
            candidate_exact,
            candidate_topology,
            exact_report,
            validation,
            exact_at,
            validated_at: Instant::now(),
            _exact_task: exact_task,
        })
    }

    fn worker_path(app: &KetchupApp) -> Option<PathBuf> {
        app.exact_worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        })
    }

    /// UI-thread preflight shared by the queued bridge path and the synchronous
    /// built-in assistant path. Mutates nothing.
    fn plan_apply_and_verify(
        app: &KetchupApp,
        request: ApplyAndVerifyRequest,
        ui_busy: bool,
        #[cfg(test)] fault: Option<ApplyAndVerifyFault>,
    ) -> Result<
        (
            ApplyAndVerifyPlan,
            ExactEvaluationSelection,
            AssistantValidationSelection,
        ),
        PlanRejection,
    > {
        let ApplyAndVerifyRequest {
            expected,
            selection,
            program,
            validators,
            timeout_ms,
            save,
        } = request;
        let started = Instant::now();
        if timeout_ms == 0 || timeout_ms > MAX_APPLY_VERIFY_TIMEOUT_MS {
            return Err("invalid_job_timeout".into());
        }
        let save_path = match save {
            None => None,
            Some(ApplyAndVerifySave::Current {}) => {
                Some(app.document_path.clone().ok_or("save_path_required")?)
            }
            Some(ApplyAndVerifySave::Path { path }) => {
                if path.is_empty()
                    || path.len() > 4096
                    || path.contains('\0')
                    || !Path::new(&path).is_absolute()
                {
                    return Err("invalid_path".into());
                }
                Some(PathBuf::from(path))
            }
        };
        let validation_selection = Self::mandatory_validation_selection(&validators)?;
        Self::guard(app, &expected)?;
        Self::available(app, ui_busy)?;
        let selection = Self::selection_guard(app, selection)?;
        program.validate().map_err(|_| "invalid_program")?;
        Self::program_scope(app, &program)?;
        #[cfg(test)]
        if fault == Some(ApplyAndVerifyFault::Planning) {
            return Err("planning_rejected".into());
        }
        let proposal = app
            .derive_assistant_cad_edit_proposal(&program)
            .map_err(|diagnostic| {
                if Self::is_capability_gap(&diagnostic) {
                    PlanRejection::CapabilityGap(diagnostic)
                } else {
                    PlanRejection::Code("planning_rejected")
                }
            })?;
        #[cfg(test)]
        if fault == Some(ApplyAndVerifyFault::Candidate) {
            return Err("candidate_rejected".into());
        }
        let candidate = app
            .document
            .preview_verified_proposal(&proposal)
            .map_err(|_| "candidate_rejected")?;
        let exact_selection = plan_incremental_exact_evaluation(
            &app.document.current(),
            &candidate,
            app.exact_source.as_ref(),
        )
        .map_err(|_| "planning_rejected")?
        .selection;
        let plan = ApplyAndVerifyPlan {
            before: app.live_bridge_stamp(),
            selection,
            proposal,
            candidate,
            save_path,
            timeout_ms,
            started,
            planned_at: Instant::now(),
        };
        Ok((plan, exact_selection, validation_selection))
    }

    #[allow(clippy::too_many_arguments)]
    fn start_queued_apply_and_verify(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        id: u64,
        reply: mpsc::SyncSender<Response>,
        cancelled: Arc<AtomicBool>,
        request: ApplyAndVerifyRequest,
        ui_busy: bool,
    ) {
        if self.apply_and_verify_job.is_some() {
            Self::reply(app, id, &reply, Err("apply_and_verify_busy"));
            return;
        }
        let (plan, exact_selection, validation_selection) = match Self::plan_apply_and_verify(
            app,
            request,
            ui_busy,
            #[cfg(test)]
            self.apply_and_verify_fault,
        ) {
            Ok(planned) => planned,
            Err(PlanRejection::CapabilityGap(diagnostic)) => {
                Self::reply_capability_gap(app, id, &reply, &diagnostic);
                return;
            }
            Err(PlanRejection::Code(code)) => {
                Self::reply(app, id, &reply, Err(code));
                return;
            }
        };
        let container_data = app.container_data.clone();
        let render = app.exact_results.clone();
        let topology = app.topology_results.clone();
        let worker_path = Self::worker_path(app);
        let candidate = plan.candidate.clone();
        let (started, timeout_ms) = (plan.started, plan.timeout_ms);
        let worker_cancelled = Arc::new(AtomicBool::new(false));
        let thread_cancelled = Arc::clone(&worker_cancelled);
        #[cfg(test)]
        let fault = self.apply_and_verify_fault;
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-live-apply-verify".into())
            .spawn(move || {
                let result = Self::evaluate_apply_and_verify_candidate(
                    &candidate,
                    &container_data,
                    &render,
                    &topology,
                    worker_path,
                    &exact_selection,
                    &validation_selection,
                    started,
                    timeout_ms,
                    thread_cancelled,
                    #[cfg(test)]
                    fault,
                );
                let _ = sender.try_send(result);
                repaint.request_repaint();
            });
        if spawn.is_err() {
            Self::reply(app, id, &reply, Err("job_worker_unavailable"));
            return;
        }
        self.apply_and_verify_job = Some(ApplyAndVerifyJob {
            id,
            reply,
            cancelled,
            worker_cancelled,
            plan,
            receiver,
        });
        context.request_repaint_after(Duration::from_millis(10));
    }

    fn publish_apply_and_verify(
        app: &mut KetchupApp,
        plan: &ApplyAndVerifyPlan,
        prepared: PreparedApplyAndVerify,
        ui_busy: bool,
        cancelled: &AtomicBool,
        #[cfg(test)] fault: Option<ApplyAndVerifyFault>,
    ) -> Result<Value, &'static str> {
        let ApplyAndVerifyPlan {
            before,
            selection,
            proposal,
            candidate,
            save_path,
            timeout_ms,
            started,
            planned_at,
        } = plan;
        let deadline = Duration::from_millis(*timeout_ms);
        if started.elapsed() > deadline {
            cancelled.store(true, Ordering::Release);
            return Err("job_timeout");
        }
        Self::require_request_authority(cancelled)?;
        // The candidate was built on `before`; a human edit since then wins.
        if app.document.mutation_epoch() != before.mutation_epoch {
            return Err("stale_document");
        }
        Self::available(app, ui_busy)?;
        Self::check_selection_guard(app, selection)?;

        let PreparedApplyAndVerify {
            candidate_exact,
            candidate_topology,
            exact_report,
            validation,
            exact_at,
            validated_at,
            _exact_task,
        } = prepared;
        let diff_targets = proposal
            .authoritative_diff()
            .iter()
            .map(|entry| format!("{:?}", entry.target))
            .collect::<Vec<_>>();
        let exact_references = candidate_exact
            .values()
            .flat_map(|package| package.references())
            .cloned()
            .collect::<Vec<_>>();
        #[cfg(test)]
        if fault == Some(ApplyAndVerifyFault::Publication) {
            return Err("commit_rejected");
        }
        let committed = app
            .complete_mutation_and_exact_results_with_work_recovery(
                |document, exact_results, topology_results| {
                    let committed = document
                        .commit_verified_proposal(proposal)
                        .map_err(|_| "commit_rejected")?;
                    for reference in exact_references {
                        document
                            .register_exact_reference_evidence(reference)
                            .map_err(|_| "exact_reference_rejected")?;
                    }
                    document
                        .register_exact_reference_evidence(&candidate_exact)
                        .map_err(|_| "exact_reference_rejected")?;
                    *exact_results = candidate_exact;
                    *topology_results = candidate_topology;
                    Ok(committed)
                },
            )
            .map_err(|error| match error {
                WorkRecoveryMutationError::Mutation(code) => code,
                WorkRecoveryMutationError::Recovery(_) => "recovery_rejected",
            })?;
        let after = app.live_bridge_stamp();
        app.exact_source = Some(exact_source(candidate));
        app.exact_retry_at = None;
        app.invalidate_pending_import_reviews();
        app.clear_ephemeral_edit_state();
        app.reconcile_selection();
        app.status_key = "status-ready";
        let committed_at = Instant::now();

        let (saved, save_state, save_error) = if let Some(path) = save_path {
            let saved = app.save_document_to_while(path, || {
                !cancelled.load(Ordering::Acquire) && started.elapsed() <= deadline
            });
            if saved {
                (true, "saved", None)
            } else if cancelled.load(Ordering::Acquire) {
                (false, "committed_but_unsaved", Some("request_cancelled"))
            } else if started.elapsed() > deadline {
                (false, "committed_but_unsaved", Some("job_timeout"))
            } else {
                (false, "committed_but_unsaved", Some("save_rejected"))
            }
        } else {
            (false, "not_requested", None)
        };
        let finished_at = Instant::now();
        Ok(json!({
            "before": before,
            "after": after,
            "diff": {
                "entry_count": proposal.authoritative_diff().len(),
                "targets": diff_targets,
                "command_digest": committed.command_digest(),
                "result_digest": committed.result_digest(),
            },
            "exact": {
                "complete": exact_report.complete,
                "topology_complete": exact_report.topology_complete,
                "producer_count": exact_report.producers.len(),
                "not_evaluated": exact_report.not_evaluated,
            },
            "validation": {
                "state": validation["state"],
                "complete": validation["complete"],
                "issue_count": validation["issue_count"],
                "requested": validation["requested"],
            },
            "timing_ms": {
                "planning": planned_at.duration_since(*started).as_millis() as u64,
                "exact": exact_at.duration_since(*planned_at).as_millis() as u64,
                "validators": validated_at.duration_since(exact_at).as_millis() as u64,
                "publication": committed_at.duration_since(validated_at).as_millis() as u64,
                "save": finished_at.duration_since(committed_at).as_millis() as u64,
                "total": finished_at.duration_since(*started).as_millis() as u64,
                "deadline": timeout_ms,
            },
            "published": true,
            "saved": saved,
            "save_state": save_state,
            "save_path": save_path.as_ref().map(|path| path.to_string_lossy().into_owned()),
            "save_error": save_error,
            "undo_steps": app.undo_step_count(),
        }))
    }

    /// Synchronous variant for callers already off the bridge queue.
    fn apply_and_verify_now(
        app: &mut KetchupApp,
        request: ApplyAndVerifyRequest,
        ui_busy: bool,
        cancelled: &Arc<AtomicBool>,
        #[cfg(test)] fault: Option<ApplyAndVerifyFault>,
    ) -> Result<Value, &'static str> {
        let (plan, exact_selection, validation_selection) = Self::plan_apply_and_verify(
            app,
            request,
            ui_busy,
            #[cfg(test)]
            fault,
        )
        .map_err(|rejection| match rejection {
            PlanRejection::Code(code) => code,
            PlanRejection::CapabilityGap(_) => "capability_gap",
        })?;
        let prepared = Self::evaluate_apply_and_verify_candidate(
            &plan.candidate,
            &app.container_data,
            &app.exact_results,
            &app.topology_results,
            Self::worker_path(app),
            &exact_selection,
            &validation_selection,
            plan.started,
            plan.timeout_ms,
            Arc::clone(cancelled),
            #[cfg(test)]
            fault,
        )?;
        Self::publish_apply_and_verify(
            app,
            &plan,
            prepared,
            ui_busy,
            cancelled,
            #[cfg(test)]
            fault,
        )
    }

    pub(crate) fn apply_assistant_cad_program(
        app: &mut KetchupApp,
        program: AssistantCadEditProgram,
    ) -> Result<Value, &'static str> {
        Self::apply_and_verify_now(
            app,
            ApplyAndVerifyRequest {
                expected: None,
                selection: None,
                program,
                validators: Vec::new(),
                timeout_ms: DEFAULT_APPLY_VERIFY_TIMEOUT_MS,
                save: None,
            },
            false,
            &Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            None,
        )
    }

    fn poll_apply_and_verify_job(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        ui_busy: bool,
    ) {
        let Some(job) = self.apply_and_verify_job.take() else {
            return;
        };
        let result = if job.cancelled.load(Ordering::Acquire) {
            job.worker_cancelled.store(true, Ordering::Release);
            Some(Err("request_cancelled"))
        } else if job.plan.started.elapsed() > Duration::from_millis(job.plan.timeout_ms) {
            job.worker_cancelled.store(true, Ordering::Release);
            Some(Err("job_timeout"))
        } else if app.document.mutation_epoch() != job.plan.before.mutation_epoch {
            // A human edit wins at once; the candidate can never publish.
            job.worker_cancelled.store(true, Ordering::Release);
            Some(Err("stale_document"))
        } else {
            match job.receiver.try_recv() {
                Ok(Ok(prepared)) => Some(Self::publish_apply_and_verify(
                    app,
                    &job.plan,
                    prepared,
                    ui_busy,
                    &job.cancelled,
                    #[cfg(test)]
                    self.apply_and_verify_fault,
                )),
                Ok(Err(code)) => Some(Err(code)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("apply_and_verify_worker_disconnected"))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            }
        };
        if let Some(result) = result {
            Self::reply(app, job.id, &job.reply, result);
            context.request_repaint();
        } else {
            self.apply_and_verify_job = Some(job);
            context.request_repaint_after(Duration::from_millis(10));
        }
    }

    #[cfg(test)]
    fn execute(
        &mut self,
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
    ) -> Result<Value, &'static str> {
        self.execute_authorized(app, request, ui_busy, &Arc::new(AtomicBool::new(false)))
    }

    fn execute_authorized(
        &mut self,
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Value, &'static str> {
        Self::require_request_authority(cancelled)?;
        match request {
            Request::Status {} => Ok(
                json!({"connected":true,"protocol":1,"image":"cad_viewport_png_thumbnail",
                "image_protocol":{"version":IMAGE_PROTOCOL_VERSION,"capabilities":["capture_mode","capture_metadata","render_metadata","variable_size","selection_framing","detail_selection_framing"],"capture_modes":["offscreen","visible_viewport"],"default_capture_mode":"offscreen","framing_modes":["viewport","selection","detail_selection"],"default_framing":"viewport","min_side_px":MIN_IMAGE_SIDE_PX,"max_side_px":MAX_IMAGE_SIDE_PX,"default_side_px":512},
                "busy":ui_busy || Self::busy(app),"read_only":app.review_candidate.is_some(),
                "selection":Self::selection(app).ok(),"selection_scope":"root_occurrences_only",
                "undo_steps":app.undo_step_count(),"redo_steps":app.redo_step_count(),
                "pending_proposal_id":self.pending.as_ref().map(|p|p.id),
                "limits":{"frame_bytes":MAX_FRAME_BYTES,"image_frame_bytes":MAX_IMAGE_FRAME_BYTES,"queue":QUEUE_CAPACITY,"selection":MAX_SELECTION,"apply_verify_timeout_ms":MAX_APPLY_VERIFY_TIMEOUT_MS,"batch_jobs":MAX_BATCH_JOBS},
                "methods":["status","summary","edit_context","query","detail","workset_create","workset_status","batch_job_start","batch_job_status","batch_job_step","batch_job_cancel","propose","commit","apply_and_verify","undo","redo","save","save_as","open","selection","view","image","disconnect"]}),
            ),
            Request::Summary {} => Ok(self.query.summary(&app.document.current())),
            Request::EditContext { targets, .. } => self
                .query
                .edit_context(
                    &app.document.current(),
                    &app.topology_results,
                    app.document.mutation_epoch(),
                    &EditContextRequest { targets },
                )
                .map_err(|error| error.code()),
            Request::Query { query, .. } => self
                .query
                .page_with_topology(&app.document.current(), &app.topology_results, &query)
                .map_err(|e| e.code()),
            Request::Detail {
                kind, entity_id, ..
            } => self
                .query
                .detail_with_topology(
                    &app.document.current(),
                    &app.topology_results,
                    kind,
                    entity_id,
                )
                .map_err(|e| e.code()),
            Request::WorksetCreate { expected, query } => {
                Self::guard(app, &expected)?;
                self.query
                    .create_workset(&app.document.current(), &query)
                    .map_err(|e| e.code())
            }
            Request::WorksetStatus { handle, .. } => self
                .query
                .workset_status(&app.document.current(), &handle)
                .map_err(|e| e.code()),
            Request::BatchJobStart {
                expected,
                workset_handle,
                operation,
            } => {
                Self::guard(app, &expected)?;
                let task = self
                    .query
                    .create_occurrence_batch_task(&app.document, &workset_handle, operation)
                    .map_err(|e| e.code())?;
                if self.batch_jobs.len() == MAX_BATCH_JOBS {
                    let terminal = self
                        .batch_jobs
                        .iter()
                        .position(|job| {
                            matches!(
                                job.task.status(&app.document).state,
                                OccurrenceBatchState::Completed
                                    | OccurrenceBatchState::Cancelled
                                    | OccurrenceBatchState::Stale
                            )
                        })
                        .ok_or("batch_job_limit")?;
                    self.batch_jobs.remove(terminal);
                }
                let id = self.next_batch_job;
                self.next_batch_job = id.checked_add(1).ok_or("batch_job_ids_exhausted")?;
                let handle = self.batch_job_handle(id);
                let status = task.status(&app.document);
                self.batch_jobs.push_back(BatchJob {
                    handle: handle.clone(),
                    task,
                });
                Ok(json!({"job_handle":handle,"status":status}))
            }
            Request::BatchJobStatus { handle, .. } => {
                let job = self
                    .batch_jobs
                    .iter()
                    .find(|job| job.handle == handle)
                    .ok_or("batch_job_not_found")?;
                Ok(json!({"job_handle":job.handle,"status":job.task.status(&app.document)}))
            }
            Request::BatchJobCancel { expected, handle } => {
                Self::guard(app, &expected)?;
                let job = self
                    .batch_jobs
                    .iter_mut()
                    .find(|job| job.handle == handle)
                    .ok_or("batch_job_not_found")?;
                Self::require_request_authority(cancelled)?;
                job.task.cancel();
                Ok(json!({"job_handle":job.handle,"status":job.task.status(&app.document)}))
            }
            Request::BatchJobStep { expected, handle } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                let index = self
                    .batch_jobs
                    .iter()
                    .position(|job| job.handle == handle)
                    .ok_or("batch_job_not_found")?;
                Self::require_request_authority(cancelled)?;
                let receipt = self.batch_jobs[index]
                    .task
                    .commit_next(app)
                    .map_err(|e| e.code())?;
                if receipt.is_some() {
                    self.query.invalidate();
                    self.observed = Some(app.live_bridge_stamp());
                    app.invalidate_pending_import_reviews();
                    app.clear_ephemeral_edit_state();
                    app.reconcile_selection();
                    app.status_key = "status-ready";
                }
                let status = self.batch_jobs[index].task.status(&app.document);
                Ok(json!({"job_handle":handle,"status":status,"receipt":receipt}))
            }
            Request::Propose {
                expected,
                selection,
                program,
            } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                let selection = Self::selection_guard(app, selection)?;
                program.validate().map_err(|_| "invalid_program")?;
                Self::program_scope(app, &program)?;
                let proposal = app
                    .derive_assistant_cad_edit_proposal(&program)
                    .map_err(|_| "planning_rejected")?;
                let id = self.next_proposal;
                self.next_proposal = id.checked_add(1).ok_or("proposal_ids_exhausted")?;
                let value = json!({"proposal_id":id,
                    "command_digest":proposal.command_digest(),"result_digest":proposal.intended_result_digest(),
                    "write_count":proposal.authoritative_writes().len()});
                self.pending = Some(Pending {
                    id,
                    epoch: app.document.mutation_epoch(),
                    selection,
                    proposal,
                });
                Ok(value)
            }
            Request::Commit {
                expected,
                proposal_id,
            } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                let pending = self.pending.as_ref().ok_or("proposal_not_found")?;
                if pending.id != proposal_id {
                    return Err("proposal_not_found");
                }
                if pending.epoch != app.document.mutation_epoch() {
                    return Err("stale_document");
                }
                Self::check_selection_guard(app, &pending.selection)?;
                Self::require_request_authority(cancelled)?;
                let committed = app
                    .commit_verified_proposal_with_work_recovery(&pending.proposal)
                    .map_err(|error| match error {
                        WorkRecoveryMutationError::Mutation(_) => "commit_rejected",
                        WorkRecoveryMutationError::Recovery(_) => "recovery_rejected",
                    })?;
                self.pending.take().expect("committed pending proposal");
                let value = json!({"proposal_id":proposal_id,"committed":true,
                    "after":app.live_bridge_stamp(),"command_digest":committed.command_digest(),
                    "result_digest":committed.result_digest(),"write_count":committed.verified_writes().len(),
                    "undo_steps":app.undo_step_count(),"geometry_evaluated":false});
                app.invalidate_pending_import_reviews();
                app.clear_ephemeral_edit_state();
                app.reconcile_selection();
                app.status_key = "status-ready";
                Ok(value)
            }
            Request::ApplyAndVerify {
                expected,
                selection,
                program,
                validators,
                timeout_ms,
                save,
            } => Self::apply_and_verify_now(
                app,
                ApplyAndVerifyRequest {
                    expected,
                    selection,
                    program,
                    validators,
                    timeout_ms,
                    save,
                },
                ui_busy,
                cancelled,
                #[cfg(test)]
                self.apply_and_verify_fault,
            ),
            Request::Undo { expected } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                if !app.command_enabled(AppCommand::Undo) {
                    return Err("undo_unavailable");
                }
                Self::require_request_authority(cancelled)?;
                Ok(json!({"changed":app.undo()}))
            }
            Request::Redo { expected } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                if !app.command_enabled(AppCommand::Redo) {
                    return Err("redo_unavailable");
                }
                Self::require_request_authority(cancelled)?;
                Ok(json!({"changed":app.redo()}))
            }
            Request::Save { expected } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                let path = app.document_path.clone().ok_or("save_path_required")?;
                if !app.save_document_to_while(&path, || !cancelled.load(Ordering::Acquire)) {
                    return Err("save_rejected");
                }
                Ok(json!({"saved":true,"same_gui_document":true,"dirty":app.is_dirty()}))
            }
            Request::SaveAs { expected, path } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                if path.is_empty()
                    || path.len() > 4096
                    || path.contains('\0')
                    || !Path::new(&path).is_absolute()
                {
                    return Err("invalid_path");
                }
                if !app
                    .save_document_to_while(Path::new(&path), || !cancelled.load(Ordering::Acquire))
                {
                    return Err("save_rejected");
                }
                Ok(json!({"saved":true,"same_gui_document":true,"dirty":app.is_dirty()}))
            }
            Request::Open { expected, path } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                if path.is_empty()
                    || path.len() > 4096
                    || path.contains('\0')
                    || !Path::new(&path).is_absolute()
                    || !Path::new(&path).is_file()
                {
                    return Err("invalid_path");
                }
                if !app.confirm_live_open_path(Path::new(&path)) {
                    return Err("open_rejected");
                }
                if !app.confirm_discard_if_dirty() {
                    return Err("open_rejected");
                }
                Self::require_request_authority(cancelled)?;
                if !app.open_document_from(Path::new(&path)) {
                    return Err("open_rejected");
                }
                self.invalidate_document_context();
                self.observed = Some(app.live_bridge_stamp());
                Ok(json!({"opened":true,"same_gui_window":true,"dirty":app.is_dirty()}))
            }
            Request::Selection {
                expected,
                occurrence_ids,
            } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                Self::selection(app)?;
                let ids = Self::validate_ids(&occurrence_ids)?;
                Self::root_ids(app, &ids)?;
                let snapshot = app.document.current();
                if ids
                    .iter()
                    .any(|id| snapshot.occurrence(OccurrenceId(*id)).is_none())
                {
                    return Err("entity_not_found");
                }
                app.selection.clear();
                for id in &ids {
                    app.selection.select_occurrence(OccurrenceId(*id), true);
                }
                // An explicit selection command revokes proposals even if it reselects
                // the same IDs; never silently retarget an immutable proposal.
                self.pending = None;
                Ok(json!({"occurrence_ids":ids,"canonical_mutation":false}))
            }
            Request::View { view, .. } => {
                Self::available(app, ui_busy)?;
                let command = match view {
                    View::Iso => AppCommand::ViewIso,
                    View::Top => AppCommand::ViewTop,
                    View::Front => AppCommand::ViewFront,
                    View::ZoomFit => AppCommand::ZoomFit,
                };
                if !app.command_enabled(command) {
                    return Err("view_unavailable");
                }
                app.dispatch_command(command);
                Ok(json!({"view":view,"canonical_mutation":false,"image":"not_requested"}))
            }
            Request::Image {
                expected,
                image_protocol_version,
                ..
            } => {
                if image_protocol_version != IMAGE_PROTOCOL_VERSION {
                    return Err("unsupported_image_protocol");
                }
                Self::guard(app, &expected)?;
                Err("image_requires_frame_callback")
            }
            Request::Disconnect {} => {
                self.pending = None;
                self.batch_jobs.clear();
                self.batch_job_key = RandomState::new();
                self.next_batch_job = 1;
                self.query.invalidate();
                Ok(json!({"disconnected":true}))
            }
        }
    }
}
