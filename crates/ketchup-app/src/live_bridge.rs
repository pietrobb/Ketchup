//! Opt-in live GUI bridge v1. No independent document/session or scripting engine.
//! Wire: u32 big-endian byte length, then UTF-8 JSON `Envelope`; responses use
//! the same framing. Both directions are capped at 32 KiB. At most four active clients,
//! one in-flight request per client, queue capacity 8, at most 4 requests per UI frame.
//! Authentication is required on EVERY request. Never publish credentials in
//! logs, command lines, environment variables, documents, or ordinary UI output.
//! Only a trusted embedding host should use `live_bridge_credentials` and carry
//! its result over an independently trusted out-of-band channel.
//! Proposals/commit receipts are connection-local; disconnect drops pending authority.
//! Guarded apply-and-verify receipts survive reconnect and require an identical payload digest.
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
    graph::sha256_hex,
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
pub const MAX_RECEIPTS: usize = 32;
pub const MAX_APPLY_VERIFY_TIMEOUT_MS: u64 = 10_000;
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
        expected: Stamp,
        targets: Vec<AssistantInstancePath>,
    },
    Query {
        expected: Stamp,
        query: PageRequest,
    },
    Detail {
        expected: Stamp,
        kind: EntityKind,
        entity_id: u64,
    },
    WorksetCreate {
        expected: Stamp,
        query: PageRequest,
    },
    WorksetStatus {
        expected: Stamp,
        handle: String,
    },
    BatchJobStart {
        expected: Stamp,
        workset_handle: String,
        operation: OccurrenceBatchOperation,
    },
    BatchJobStatus {
        expected: Stamp,
        handle: String,
    },
    BatchJobStep {
        expected: Stamp,
        handle: String,
    },
    BatchJobCancel {
        expected: Stamp,
        handle: String,
    },
    Propose {
        expected: Stamp,
        selection: Vec<u64>,
        program: AssistantCadEditProgram,
    },
    Commit {
        expected: Stamp,
        proposal_id: u64,
    },
    ApplyAndVerify {
        request_id: String,
        expected: Stamp,
        selection: Vec<u64>,
        program: AssistantCadEditProgram,
        validators: Vec<String>,
        timeout_ms: u64,
        #[serde(default)]
        save: Option<ApplyAndVerifySave>,
    },
    Undo {
        expected: Stamp,
    },
    Redo {
        expected: Stamp,
    },
    Save {
        expected: Stamp,
    },
    SaveAs {
        expected: Stamp,
        path: String,
    },
    Open {
        expected: Stamp,
        path: String,
    },
    Selection {
        expected: Stamp,
        occurrence_ids: Vec<u64>,
    },
    View {
        expected: Stamp,
        view: View,
    },
    Image {
        expected: Stamp,
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
struct Pending {
    id: u64,
    stamp: Stamp,
    selection: Vec<u64>,
    primary: Option<SelectionId>,
    proposal: Proposal,
}
struct Receipt {
    id: u64,
    expected: Stamp,
    value: Value,
}
struct ApplyAndVerifyReceipt {
    request_id: String,
    payload_digest: String,
    value: Value,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplyAndVerifyFault {
    Planning,
    Candidate,
    Exact,
    Validation,
    Report,
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
    request_id: String,
    payload_digest: String,
    expected: Stamp,
    selection: Vec<u64>,
    primary: Option<SelectionId>,
    proposal: Proposal,
    candidate: Snapshot,
    save_path: Option<PathBuf>,
    timeout_ms: u64,
    started: Instant,
    planned_at: Instant,
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
    receipts: VecDeque<Receipt>,
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
            receipts: VecDeque::new(),
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
    receipts: VecDeque<Receipt>,
    apply_and_verify_receipts: VecDeque<ApplyAndVerifyReceipt>,
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
                    request_id,
                    expected,
                    selection,
                    program,
                    validators,
                    timeout_ms,
                    save,
                } => {
                    bridge.start_queued_apply_and_verify(
                        self, context, id, reply, cancelled, request_id, expected, selection,
                        program, validators, timeout_ms, save, ui_busy,
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
                receipts: std::mem::take(&mut self.receipts),
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
        self.receipts = state.receipts;
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
        self.receipts.clear();
        self.batch_jobs.clear();
        self.batch_job_key = RandomState::new();
        self.next_batch_job = 1;
        self.image.revoke();
    }

    pub(super) fn invalidate_document_context(&mut self) {
        self.observed = None;
        self.client_states.clear();
        self.pending = None;
        self.receipts.clear();
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
    #[cfg(feature = "named-product-fixtures")]
    fn fixture_busy(app: &KetchupApp) -> bool {
        app.bottle_direct_drag.is_some()
            || app.bottle_editor.is_some()
            || app.part_authoring_preview_pending()
    }
    #[cfg(not(feature = "named-product-fixtures"))]
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
    fn guard(app: &KetchupApp, expected: &Stamp) -> Result<(), &'static str> {
        if &app.live_bridge_stamp() != expected {
            return Err("stale_document");
        }
        Ok(())
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
        let validator_names = validators.iter().map(String::as_str).collect::<Vec<_>>();
        let selection = AssistantValidationSelection::only(&validator_names);
        if !selection.is_valid()
            || ["collision", "gravity_support"]
                .into_iter()
                .any(|required| !selection.requested.contains(required))
        {
            return Err("mandatory_validators_required");
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
            if cancelled.load(Ordering::Acquire) {
                "request_cancelled"
            } else if error.contains("timed out") {
                "job_timeout"
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

    #[allow(clippy::too_many_arguments)]
    fn start_queued_apply_and_verify(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        id: u64,
        reply: mpsc::SyncSender<Response>,
        cancelled: Arc<AtomicBool>,
        request_id: String,
        expected: Stamp,
        selection: Vec<u64>,
        program: AssistantCadEditProgram,
        validators: Vec<String>,
        timeout_ms: u64,
        save: Option<ApplyAndVerifySave>,
        ui_busy: bool,
    ) {
        macro_rules! reject {
            ($code:expr) => {{
                Self::reply(app, id, &reply, Err($code));
                return;
            }};
        }
        if self.apply_and_verify_job.is_some() {
            reject!("apply_and_verify_busy");
        }
        if request_id.is_empty()
            || request_id.len() > 128
            || request_id.chars().any(char::is_control)
        {
            reject!("invalid_request_id");
        }
        if timeout_ms == 0 || timeout_ms > MAX_APPLY_VERIFY_TIMEOUT_MS {
            reject!("invalid_job_timeout");
        }
        let payload = match serde_json::to_vec(&(
            &expected,
            &selection,
            &program,
            &validators,
            timeout_ms,
            &save,
        )) {
            Ok(payload) => payload,
            Err(_) => reject!("payload_encoding_rejected"),
        };
        let payload_digest = sha256_hex(&payload);
        if let Some(receipt) = self
            .apply_and_verify_receipts
            .iter()
            .find(|receipt| receipt.request_id == request_id)
        {
            if receipt.payload_digest != payload_digest {
                reject!("request_id_payload_mismatch");
            }
            Self::reply(app, id, &reply, Ok(receipt.value.clone()));
            return;
        }
        let save_path = match &save {
            None => None,
            Some(ApplyAndVerifySave::Current {}) => match app.document_path.clone() {
                Some(path) => Some(path),
                None => reject!("save_path_required"),
            },
            Some(ApplyAndVerifySave::Path { path }) => {
                if path.is_empty()
                    || path.len() > 4096
                    || path.contains('\0')
                    || !Path::new(path).is_absolute()
                {
                    reject!("invalid_path");
                }
                Some(Path::new(path).to_owned())
            }
        };
        let validation_selection = match Self::mandatory_validation_selection(&validators) {
            Ok(selection) => selection,
            Err(code) => reject!(code),
        };
        let started = Instant::now();
        if let Err(code) = Self::guard(app, &expected) {
            reject!(code);
        }
        if let Err(code) = Self::available(app, ui_busy) {
            reject!(code);
        }
        let selection = match Self::validate_ids(&selection) {
            Ok(selection) => selection,
            Err(code) => reject!(code),
        };
        match Self::selection(app) {
            Ok(current) if current == selection => {}
            Ok(_) => reject!("selection_changed"),
            Err(code) => reject!(code),
        }
        let primary = app.selection.primary.clone();
        if program.validate().is_err() {
            reject!("invalid_program");
        }
        if let Err(code) = Self::program_scope(app, &program) {
            reject!(code);
        }
        #[cfg(test)]
        if self.apply_and_verify_fault == Some(ApplyAndVerifyFault::Planning) {
            reject!("planning_rejected");
        }
        let proposal = match app.derive_assistant_cad_edit_proposal(&program) {
            Ok(proposal) => proposal,
            Err(diagnostic) if Self::is_capability_gap(&diagnostic) => {
                Self::reply_capability_gap(app, id, &reply, &diagnostic);
                return;
            }
            Err(_) => reject!("planning_rejected"),
        };
        #[cfg(test)]
        if self.apply_and_verify_fault == Some(ApplyAndVerifyFault::Candidate) {
            reject!("candidate_rejected");
        }
        let candidate = match app.document.preview_verified_proposal(&proposal) {
            Ok(candidate) => candidate,
            Err(_) => reject!("candidate_rejected"),
        };
        if let Err(code) = Self::require_request_authority(&cancelled) {
            reject!(code);
        }
        let exact_selection = match plan_incremental_exact_evaluation(
            &app.document.current(),
            &candidate,
            app.exact_source.as_ref(),
        ) {
            Ok(plan) => plan.selection,
            Err(_) => reject!("planning_rejected"),
        };
        let planned_at = Instant::now();
        let worker_path = app.exact_worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        });
        let container_data = app.container_data.clone();
        let render = app.exact_results.clone();
        let topology = app.topology_results.clone();
        let worker_candidate = candidate.clone();
        let worker_cancelled = Arc::new(AtomicBool::new(false));
        let job_worker_cancelled = Arc::clone(&worker_cancelled);
        #[cfg(test)]
        let worker_fault = self.apply_and_verify_fault;
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-live-apply-verify".into())
            .spawn(move || {
                let result = Self::evaluate_apply_and_verify_candidate(
                    &worker_candidate,
                    &container_data,
                    &render,
                    &topology,
                    worker_path,
                    &exact_selection,
                    &validation_selection,
                    started,
                    timeout_ms,
                    Arc::clone(&worker_cancelled),
                    #[cfg(test)]
                    worker_fault,
                );
                let _ = sender.try_send(result);
                repaint.request_repaint();
            });
        if spawn.is_err() {
            reject!("job_worker_unavailable");
        }
        self.apply_and_verify_job = Some(ApplyAndVerifyJob {
            id,
            reply,
            cancelled,
            worker_cancelled: job_worker_cancelled,
            request_id,
            payload_digest,
            expected,
            selection,
            primary,
            proposal,
            candidate,
            save_path,
            timeout_ms,
            started,
            planned_at,
            receiver,
        });
        context.request_repaint_after(Duration::from_millis(10));
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_apply_and_verify(
        app: &mut KetchupApp,
        request_id: &str,
        payload_digest: &str,
        expected: &Stamp,
        selection: &[u64],
        primary: &Option<SelectionId>,
        proposal: &Proposal,
        candidate: &Snapshot,
        save_path: Option<&Path>,
        timeout_ms: u64,
        started: Instant,
        planned_at: Instant,
        prepared: PreparedApplyAndVerify,
        ui_busy: bool,
        cancelled: &AtomicBool,
        receipts: &mut VecDeque<ApplyAndVerifyReceipt>,
        #[cfg(test)] fault: Option<ApplyAndVerifyFault>,
    ) -> Result<Value, &'static str> {
        let deadline = Duration::from_millis(timeout_ms);
        if started.elapsed() > deadline {
            cancelled.store(true, Ordering::Release);
            return Err("job_timeout");
        }
        Self::require_request_authority(cancelled)?;
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        if selection != Self::selection(app)? || *primary != app.selection.primary {
            return Err("selection_changed");
        }
        Self::require_request_authority(cancelled)?;

        let PreparedApplyAndVerify {
            candidate_exact,
            candidate_topology,
            exact_report,
            validation,
            exact_at,
            validated_at,
            _exact_task,
        } = prepared;
        #[cfg(test)]
        if fault == Some(ApplyAndVerifyFault::Report) {
            return Err("validation_report_encoding_rejected");
        }
        let validation_bytes =
            serde_json::to_vec(&validation).map_err(|_| "validation_report_encoding_rejected")?;
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
        debug_assert_eq!(
            exact_source(&app.document.current()),
            exact_source(candidate)
        );
        app.exact_source = Some(exact_source(candidate));
        app.exact_retry_at = None;
        app.invalidate_pending_import_reviews();
        app.clear_ephemeral_edit_state();
        app.reconcile_selection();
        app.status_key = "status-ready";
        let committed_at = Instant::now();

        let save_path_report = save_path.map(|path| path.to_string_lossy().into_owned());
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
        let candidate_document_id = candidate.document_id().0;
        let value = json!({
            "request_id": request_id,
            "payload_digest": payload_digest,
            "before": expected,
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
                "report_digest": sha256_hex(&validation_bytes),
            },
            "timing_ms": {
                "planning": planned_at.duration_since(started).as_millis() as u64,
                "exact": exact_at.duration_since(planned_at).as_millis() as u64,
                "validators": validated_at.duration_since(exact_at).as_millis() as u64,
                "publication": committed_at.duration_since(validated_at).as_millis() as u64,
                "save": finished_at.duration_since(committed_at).as_millis() as u64,
                "total": finished_at.duration_since(started).as_millis() as u64,
                "deadline": timeout_ms,
            },
            "published": true,
            "saved": saved,
            "save_state": save_state,
            "save_path": save_path_report,
            "save_error": save_error,
            "same_gui_document": true,
            "execution": {
                "candidate_state": "isolated_snapshot",
                "candidate_document_id": candidate_document_id,
                "gui_document_id": after.document_id,
                "helper_headless_documents": 0,
            },
            "undo_steps": app.undo_step_count(),
        });
        if receipts.len() == MAX_RECEIPTS {
            receipts.pop_front();
        }
        receipts.push_back(ApplyAndVerifyReceipt {
            request_id: request_id.to_owned(),
            payload_digest: payload_digest.to_owned(),
            value: value.clone(),
        });
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_and_verify(
        &mut self,
        app: &mut KetchupApp,
        request_id: String,
        expected: Stamp,
        selection: Vec<u64>,
        program: AssistantCadEditProgram,
        validators: Vec<String>,
        timeout_ms: u64,
        save: Option<ApplyAndVerifySave>,
        ui_busy: bool,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Value, &'static str> {
        Self::apply_and_verify_with_state(
            app,
            request_id,
            expected,
            selection,
            program,
            validators,
            timeout_ms,
            save,
            ui_busy,
            cancelled,
            &mut self.apply_and_verify_receipts,
            #[cfg(test)]
            self.apply_and_verify_fault,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_and_verify_with_state(
        app: &mut KetchupApp,
        request_id: String,
        expected: Stamp,
        selection: Vec<u64>,
        program: AssistantCadEditProgram,
        validators: Vec<String>,
        timeout_ms: u64,
        save: Option<ApplyAndVerifySave>,
        ui_busy: bool,
        cancelled: &Arc<AtomicBool>,
        receipts: &mut VecDeque<ApplyAndVerifyReceipt>,
        #[cfg(test)] fault: Option<ApplyAndVerifyFault>,
    ) -> Result<Value, &'static str> {
        if request_id.is_empty()
            || request_id.len() > 128
            || request_id.chars().any(char::is_control)
        {
            return Err("invalid_request_id");
        }
        if timeout_ms == 0 || timeout_ms > MAX_APPLY_VERIFY_TIMEOUT_MS {
            return Err("invalid_job_timeout");
        }
        let payload = serde_json::to_vec(&(
            &expected,
            &selection,
            &program,
            &validators,
            timeout_ms,
            &save,
        ))
        .map_err(|_| "payload_encoding_rejected")?;
        let payload_digest = sha256_hex(&payload);
        if let Some(receipt) = receipts
            .iter()
            .find(|receipt| receipt.request_id == request_id)
        {
            if receipt.payload_digest != payload_digest {
                return Err("request_id_payload_mismatch");
            }
            return Ok(receipt.value.clone());
        }
        let save_path = match &save {
            None => None,
            Some(ApplyAndVerifySave::Current {}) => {
                Some(app.document_path.clone().ok_or("save_path_required")?)
            }
            Some(ApplyAndVerifySave::Path { path }) => {
                if path.is_empty()
                    || path.len() > 4096
                    || path.contains('\0')
                    || !Path::new(path).is_absolute()
                {
                    return Err("invalid_path");
                }
                Some(Path::new(path).to_owned())
            }
        };
        let validation_selection = Self::mandatory_validation_selection(&validators)?;

        let started = Instant::now();
        Self::guard(app, &expected)?;
        Self::available(app, ui_busy)?;
        let selection = Self::validate_ids(&selection)?;
        if selection != Self::selection(app)? {
            return Err("selection_changed");
        }
        let primary = app.selection.primary.clone();
        program.validate().map_err(|_| "invalid_program")?;
        Self::program_scope(app, &program)?;
        #[cfg(test)]
        if fault == Some(ApplyAndVerifyFault::Planning) {
            return Err("planning_rejected");
        }
        let proposal = app
            .derive_assistant_cad_edit_proposal(&program)
            .map_err(|diagnostic| {
                if Self::is_capability_gap(&diagnostic) {
                    "capability_gap"
                } else {
                    "planning_rejected"
                }
            })?;
        #[cfg(test)]
        if fault == Some(ApplyAndVerifyFault::Candidate) {
            return Err("candidate_rejected");
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
        let planned_at = Instant::now();
        Self::require_request_authority(cancelled)?;

        let worker_path = app.exact_worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        });
        let prepared = Self::evaluate_apply_and_verify_candidate(
            &candidate,
            &app.container_data,
            &app.exact_results,
            &app.topology_results,
            worker_path,
            &exact_selection,
            &validation_selection,
            started,
            timeout_ms,
            Arc::clone(cancelled),
            #[cfg(test)]
            fault,
        )?;
        Self::publish_apply_and_verify(
            app,
            &request_id,
            &payload_digest,
            &expected,
            &selection,
            &primary,
            &proposal,
            &candidate,
            save_path.as_deref(),
            timeout_ms,
            started,
            planned_at,
            prepared,
            ui_busy,
            cancelled,
            receipts,
            #[cfg(test)]
            fault,
        )
    }

    pub(crate) fn apply_assistant_cad_program(
        app: &mut KetchupApp,
        request_id: String,
        program: AssistantCadEditProgram,
    ) -> Result<Value, &'static str> {
        let expected = app.live_bridge_stamp();
        let selection = Self::selection(app)?;
        let mut receipts = VecDeque::new();
        Self::apply_and_verify_with_state(
            app,
            request_id,
            expected,
            selection,
            program,
            vec!["collision".to_owned(), "gravity_support".to_owned()],
            MAX_APPLY_VERIFY_TIMEOUT_MS,
            None,
            false,
            &Arc::new(AtomicBool::new(false)),
            &mut receipts,
            #[cfg(test)]
            None,
        )
    }

    fn finish_queued_apply_and_verify(
        &mut self,
        app: &mut KetchupApp,
        job: &ApplyAndVerifyJob,
        prepared: PreparedApplyAndVerify,
        ui_busy: bool,
    ) -> Result<Value, &'static str> {
        Self::publish_apply_and_verify(
            app,
            &job.request_id,
            &job.payload_digest,
            &job.expected,
            &job.selection,
            &job.primary,
            &job.proposal,
            &job.candidate,
            job.save_path.as_deref(),
            job.timeout_ms,
            job.started,
            job.planned_at,
            prepared,
            ui_busy,
            &job.cancelled,
            &mut self.apply_and_verify_receipts,
            #[cfg(test)]
            self.apply_and_verify_fault,
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
        } else if job.started.elapsed() > Duration::from_millis(job.timeout_ms) {
            job.worker_cancelled.store(true, Ordering::Release);
            Some(Err("job_timeout"))
        } else {
            match job.receiver.try_recv() {
                Ok(Ok(prepared)) => {
                    Some(self.finish_queued_apply_and_verify(app, &job, prepared, ui_busy))
                }
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
                "limits":{"frame_bytes":MAX_FRAME_BYTES,"image_frame_bytes":MAX_IMAGE_FRAME_BYTES,"queue":QUEUE_CAPACITY,"receipts":MAX_RECEIPTS,"selection":MAX_SELECTION,"apply_verify_timeout_ms":MAX_APPLY_VERIFY_TIMEOUT_MS,"batch_jobs":MAX_BATCH_JOBS},
                "methods":["status","summary","edit_context","query","detail","workset_create","workset_status","batch_job_start","batch_job_status","batch_job_step","batch_job_cancel","propose","commit","apply_and_verify","undo","redo","save","save_as","open","selection","view","image","disconnect"]}),
            ),
            Request::Summary {} => Ok(self.query.summary(&app.document.current())),
            Request::EditContext { expected, targets } => {
                Self::guard(app, &expected)?;
                self.query
                    .edit_context(
                        &app.document.current(),
                        &app.topology_results,
                        app.document.mutation_epoch(),
                        &EditContextRequest { targets },
                    )
                    .map_err(|error| error.code())
            }
            Request::Query { expected, query } => {
                Self::guard(app, &expected)?;
                self.query
                    .page_with_topology(&app.document.current(), &app.topology_results, &query)
                    .map_err(|e| e.code())
            }
            Request::Detail {
                expected,
                kind,
                entity_id,
            } => {
                Self::guard(app, &expected)?;
                self.query
                    .detail_with_topology(
                        &app.document.current(),
                        &app.topology_results,
                        kind,
                        entity_id,
                    )
                    .map_err(|e| e.code())
            }
            Request::WorksetCreate { expected, query } => {
                Self::guard(app, &expected)?;
                self.query
                    .create_workset(&app.document.current(), &query)
                    .map_err(|e| e.code())
            }
            Request::WorksetStatus { expected, handle } => {
                Self::guard(app, &expected)?;
                self.query
                    .workset_status(&app.document.current(), &handle)
                    .map_err(|e| e.code())
            }
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
            Request::BatchJobStatus { expected, handle } => {
                Self::guard(app, &expected)?;
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
                // No GUI-owned edit state is cleared while unavailable.
                // Receipt replay above remains observational.
                let selection = Self::validate_ids(&selection)?;
                if selection != Self::selection(app)? {
                    return Err("selection_changed");
                }
                // Existing typed CAD validation bounds operations, selectors and geometry.
                program.validate().map_err(|_| "invalid_program")?;
                Self::program_scope(app, &program)?;
                let proposal = app
                    .derive_assistant_cad_edit_proposal(&program)
                    .map_err(|_| "planning_rejected")?;
                let id = self.next_proposal;
                self.next_proposal = id.checked_add(1).ok_or("proposal_ids_exhausted")?;
                let value = json!({"proposal_id":id,"observational":true,"selection":selection,
                    "command_digest":proposal.command_digest(),"result_digest":proposal.intended_result_digest(),
                    "write_count":proposal.authoritative_writes().len(),"image":"not_requested"});
                self.pending = Some(Pending {
                    id,
                    stamp: expected,
                    selection,
                    primary: app.selection.primary.clone(),
                    proposal,
                });
                Ok(value)
            }
            Request::Commit {
                expected,
                proposal_id,
            } => {
                // A retained receipt is returned even after later human edits, with its
                // original commit stamp. Never execute the proposal for a second time.
                if let Some(receipt) = self.receipts.iter().find(|r| r.id == proposal_id) {
                    if receipt.expected != expected {
                        return Err("receipt_guard_mismatch");
                    }
                    return Ok(receipt.value.clone());
                }
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                // No GUI-owned edit state is cleared while unavailable.
                // Receipt replay above remains observational.
                let pending = self.pending.as_ref().ok_or("proposal_not_found")?;
                if pending.id != proposal_id {
                    return Err("proposal_not_found");
                }
                if pending.stamp != expected {
                    return Err("stale_document");
                }
                if pending.selection != Self::selection(app)?
                    || pending.primary != app.selection.primary
                {
                    return Err("selection_changed");
                }
                Self::require_request_authority(cancelled)?;
                let committed = app
                    .commit_verified_proposal_with_work_recovery(&pending.proposal)
                    .map_err(|error| match error {
                        WorkRecoveryMutationError::Mutation(_) => "commit_rejected",
                        WorkRecoveryMutationError::Recovery(_) => "recovery_rejected",
                    })?;
                self.pending.take().expect("committed pending proposal");
                let value = json!({"proposal_id":proposal_id,"committed":true,"verified":true,
                    "before":expected,"after":app.live_bridge_stamp(),"command_digest":committed.command_digest(),
                    "result_digest":committed.result_digest(),"write_count":committed.verified_writes().len(),
                    "undo_steps":app.undo_step_count(),"geometry_evaluated":false,"image":"not_requested"});
                app.invalidate_pending_import_reviews();
                app.clear_ephemeral_edit_state();
                app.reconcile_selection();
                app.status_key = "status-ready";
                if self.receipts.len() == MAX_RECEIPTS {
                    self.receipts.pop_front();
                }
                self.receipts.push_back(Receipt {
                    id: proposal_id,
                    expected,
                    value: value.clone(),
                });
                Ok(value)
            }
            Request::ApplyAndVerify {
                request_id,
                expected,
                selection,
                program,
                validators,
                timeout_ms,
                save,
            } => self.apply_and_verify(
                app, request_id, expected, selection, program, validators, timeout_ms, save,
                ui_busy, cancelled,
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
            Request::View { expected, view } => {
                Self::guard(app, &expected)?;
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
                self.receipts.clear();
                self.batch_jobs.clear();
                self.batch_job_key = RandomState::new();
                self.next_batch_job = 1;
                self.query.invalidate();
                Ok(json!({"disconnected":true}))
            }
        }
    }
}
