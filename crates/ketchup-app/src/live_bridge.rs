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
use ketchup_application::rejections::rejected;
use ketchup_application::{
    batch_task::{
        OccurrenceBatchDocument, OccurrenceBatchError, OccurrenceBatchOperation,
        OccurrenceBatchState, OccurrenceBatchTask,
    },
    evaluation::{
        EvaluationReport, ExactEvaluationError, ExactEvaluationSelection, ExactEvaluationTask,
        exact_source, exact_worker_candidates, materialize_exact_products,
        plan_incremental_exact_evaluation, start_exact_evaluation_scoped_with_cancellation,
    },
    model_query::{EditContextRequest, EntityKind, ModelQuery, PageRequest},
    validation::{
        AssistantValidationSelection, assistant_validation_context_with_worker_cancellation,
    },
};
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantInstancePath, AssistantRejectionDiagnostic,
};
use ketchup_model::tolerance::limits;
use ketchup_model::{
    document::{
        CommandBatch, DocumentStore, OccurrenceId, Proposal, Snapshot, VerifiedProposalCommit,
    },
    exact_product::ExactResultRegistry,
    tolerance::ROUNDING,
};
use ketchup_rejection::Rejection;
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
mod busy;
pub mod consent;
mod image;
mod program_access;
mod program_check;
pub use program_access::{ReportSection, SourceEdit};
mod measurement;
pub(crate) mod program_pick;
mod program_validate;
mod program_validation_summary;
#[cfg(test)]
mod tests;
mod transport;
pub const MAX_FRAME_BYTES: usize = 32 * 1024;
pub const MAX_IMAGE_FRAME_BYTES: usize = 12 * 1024 * 1024;
pub const MIN_IMAGE_SIDE_PX: u32 = 512;
pub const MAX_IMAGE_SIDE_PX: u32 = 1600;
pub const QUEUE_CAPACITY: usize = 8;
pub const MAX_SELECTION: usize = 100;
const MAX_INVALID_PARAMS_REASON_CHARS: usize = 512;
pub const MAX_APPLY_VERIFY_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_APPLY_VERIFY_TIMEOUT_MS: u64 = 60_000;
const fn default_apply_verify_timeout_ms() -> u64 {
    DEFAULT_APPLY_VERIFY_TIMEOUT_MS
}
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
    /// CAD program operation catalog; one operation with its types when named.
    Operations {
        #[serde(default)]
        name: Option<String>,
    },
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
        /// Reject the edit when a validator fails. By default the edit is
        /// published and the issues are reported.
        #[serde(default)]
        strict: bool,
    },
    /// The Starlark program that owns the document, with each part's occurrence and lines.
    Program {
        #[serde(default)]
        expected: Option<Stamp>,
    },
    ProgramContext {
        #[serde(default)]
        expected: Option<Stamp>,
        #[serde(default)]
        selection_context: bool,
    },
    PatchProgram {
        expected: Stamp,
        edits: Vec<SourceEdit>,
    },
    ProgramReport {
        expected: Stamp,
        section: ReportSection,
        #[serde(default)]
        offset: usize,
        limit: usize,
    },
    /// Evaluates a whole Starlark program and publishes only what changed as one Undo step.
    ApplyProgram {
        #[serde(default)]
        expected: Option<Stamp>,
        source: String,
        #[serde(default)]
        overrides: std::collections::BTreeMap<String, f64>,
        #[serde(default)]
        file_name: Option<String>,
        /// Replace a saved document that no program owns.
        #[serde(default)]
        replace_document: bool,
    },
    MeasureFaces {
        expected: Stamp,
        faces: [ketchup_application::measurement::FaceTarget; 2],
        mode: measurement::MeasurementMode,
        #[serde(default)]
        direction: Option<[f64; 3]>,
    },
    ValidateProgram {
        #[serde(default)]
        expected: Option<Stamp>,
        #[serde(default)]
        validators: Option<Vec<String>>,
        #[serde(default)]
        motion: Option<ketchup_application::ProgramMotionCheck>,
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
    Image(ImageRequest),
    Disconnect {},
}

/// What a client asks of one CAD image; it answers only after a painted frame.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRequest {
    #[serde(default)]
    pub expected: Option<Stamp>,
    pub image_protocol_version: u32,
    pub capture_mode: CaptureMode,
    pub max_side_px: u32,
    #[serde(default)]
    pub framing: ImageFraming,
    #[serde(default)]
    pub detail_target: Option<ImageDetailTarget>,
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
thread_local! {
    /// Why the next error response failed. Set right before an error code is
    /// returned on the UI thread and attached to that response.
    static ERROR_DETAILS: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

/// Records `rejection` with the request-specific `details` as the result of
/// the next error response.
fn record_rejection(rejection: &Rejection, details: Value) {
    let mut value = serde_json::to_value(rejection).unwrap_or(Value::Null);
    if let (Some(object), Some(details)) = (value.as_object_mut(), details.as_object())
        && !details.is_empty()
    {
        object.insert("details".to_owned(), Value::Object(details.clone()));
    }
    ERROR_DETAILS.with(|slot| *slot.borrow_mut() = Some(value));
}

/// Records that `code` failed because of `cause` (kept as the rejection's
/// cause chain under the catalog reason) and returns the code.
fn failed_because(
    code: &'static str,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> &'static str {
    record_rejection(&rejected(code).caused_by(cause), json!({}));
    code
}

/// Records why `code` failed for the client and returns the code.
fn failure(code: &'static str, reason: impl Into<String>, details: Value) -> &'static str {
    record_rejection(&rejected(code).reason(reason), details);
    code
}

/// Records a planner diagnostic as the rejection of `code`: its target and
/// repair hint replace the generic ones of the code.
fn planning_failure(code: &'static str, diagnostic: &AssistantRejectionDiagnostic) -> &'static str {
    record_rejection(
        &rejected(code)
            .target(diagnostic.target.clone())
            .reason(diagnostic.failed_invariant.clone())
            .fix_hint(diagnostic.repair_hint.clone()),
        json!({
            "diagnostic_code": diagnostic.code,
            "operation": diagnostic.operation,
            "retryable": diagnostic.retryable,
            "published": false,
        }),
    );
    code
}

/// The result of an error response for `code`: the recorded rejection when it
/// belongs to this code, otherwise the code's catalog rejection.
fn rejection_result(code: &str, recorded: Option<Value>) -> Value {
    match recorded {
        Some(value) if value["code"] == code => value,
        _ => serde_json::to_value(rejected(code)).unwrap_or(Value::Null),
    }
}

/// Why a typed edit of a program-owned document matters: it ends program ownership.
const PROGRAM_DETACHED_WARNING: &str = "This document was owned by a Starlark program; the typed \
edit detached it, so parameters no longer drive the model and program read returns no source. \
Undo this step (edit action=undo) to get the program back, and change program parts through \
program action=apply.";

/// The user is holding the mouse in the window or typing into a focused field.
/// A text field left focused in a window the user switched away from does not count.
fn ui_busy(context: &egui::Context) -> bool {
    busy::sample_input(context).is_busy()
}

fn program_owned(app: &KetchupApp) -> bool {
    app.document.current_rule_program().is_some()
}

/// Rejects a typed edit of a program-owned document when the caller asked for strict.
fn reject_program_detach(app: &KetchupApp, strict: bool) -> Result<(), &'static str> {
    if strict && program_owned(app) {
        return Err("program_owned_document");
    }
    Ok(())
}

/// The result fields that tell the client a typed edit detached the program.
fn add_detach_warning(value: &mut Value, owned_before: bool, app: &KetchupApp) {
    let detached = owned_before && !program_owned(app);
    value["program_detached"] = detached.into();
    if detached {
        value["warning"] = PROGRAM_DETACHED_WARNING.into();
    }
}

const MAX_PROGRAM_MESSAGE_CHARS: usize = 4000;
const MAX_PROGRAM_LOG_LINES: usize = 20;

/// A rejected program with its reason and what to do next; nothing was published.
fn program_failure(error: ketchup_application::RuleProgramApplyError) -> &'static str {
    use ketchup_application::RuleProgramApplyError as Error;
    let (reason, hint) = match &error {
        Error::Program { code, .. } => (
            code.clone(),
            "Fix the line named in the message and send the whole program again.",
        ),
        Error::IncrementalUnsupported => (
            "incremental_unsupported".to_owned(),
            "A part changed in a way that cannot keep its identity (e.g. a manually added or moved part, or a renamed profile segment). Keep part names and profile segment names stable.",
        ),
        Error::ReplacementConfirmationRequired => (
            "not_program_document".to_owned(),
            "No program owns this document. If a typed edit detached the program, undo (edit action=undo) until program read returns the source again; otherwise pass replace_document=true to replace the document.",
        ),
        Error::UnsavedChanges => (
            "unsaved_changes".to_owned(),
            "Replacing would lose unsaved changes. Undo them (edit action=undo) until program read returns the source again, or save the document (file action=save or save_as) and apply with replace_document=true.",
        ),
        Error::Session(_) => (
            "not_published".to_owned(),
            "The program is valid but its geometry could not be published; simplify the changed part.",
        ),
    };
    let message = error
        .to_string()
        .chars()
        .take(MAX_PROGRAM_MESSAGE_CHARS)
        .collect::<String>();
    record_rejection(
        &rejected("program_rejected").reason(message).fix_hint(hint),
        json!({"program_code": reason, "published": false}),
    );
    "program_rejected"
}

/// What an applied program changed, bounded to fit one response frame.
/// `exact` is the exact collision check of the applied solids, when it ran.
fn program_edit_result(
    edit: crate::program_edit::ProgramEdit,
    (report, model): (&ketchup_program::Report, &ketchup_program::ProgramModel),
    before: &ketchup_model::document::Snapshot,
    after: &ketchup_model::document::Snapshot,
    stamp: &Stamp,
    undo_steps: usize,
    exact: Option<Value>,
) -> Value {
    let added_total = after
        .occurrences()
        .filter(|occurrence| before.occurrence(occurrence.id()).is_none())
        .count();
    let added = after
        .occurrences()
        .filter(|occurrence| before.occurrence(occurrence.id()).is_none())
        .map(|occurrence| json!({"name": occurrence.name(), "occurrence_id": occurrence.id().0}))
        .take(MAX_REPORTED_PARTS)
        .collect::<Vec<_>>();
    let removed_total = before
        .occurrences()
        .filter(|occurrence| after.occurrence(occurrence.id()).is_none())
        .count();
    let removed = before
        .occurrences()
        .filter(|occurrence| after.occurrence(occurrence.id()).is_none())
        .map(|occurrence| occurrence.name().to_owned())
        .take(MAX_REPORTED_PARTS)
        .collect::<Vec<_>>();
    let truncated = added.len() < added_total
        || removed.len() < removed_total
        || report.issues.len() > MAX_REPORTED_ISSUES
        || report.relations.len() > MAX_REPORTED_RELATIONS;
    let issues = report
        .issues
        .iter()
        .take(MAX_REPORTED_ISSUES)
        .map(|issue| compact_issue(&serde_json::to_value(issue).unwrap_or(Value::Null)))
        .collect::<Vec<_>>();
    let log = &report.log[report.log.len().saturating_sub(MAX_PROGRAM_LOG_LINES)..];
    json!({
        "change": edit.as_str(),
        "after": stamp,
        "undo_steps": undo_steps,
        "parts": after.occurrences().count(),
        "added": added,
        "added_total": added_total,
        "removed": removed,
        "removed_total": removed_total,
        "truncated": truncated,
        "report": {
            "ok": report.ok,
            "errors": report.errors,
            "warnings": report.warnings,
            "issues": issues,
            "issues_total": report.issues.len(),
            "relations": &report.relations[..report.relations.len().min(MAX_REPORTED_RELATIONS)],
            "relations_total": report.relations.len(),
            "bom": {"total_parts": report.bom.total_parts, "cut_list_groups": report.bom.cut_list.len(),
                "hardware_items": report.bom.hardware.len(),
                "machining_operations": report.bom.machining.iter().map(|part| part.operations.len()).sum::<usize>()},
            "details": "program action=report with expected=after, section=cut_list/hardware/machining/relations/issues; follow next_offset until null",
            "params": report.params,
            "unused_overrides": report.unused_overrides,
            "log": log,
        },
        "geometry_evaluated": exact.as_ref().is_some_and(|exact| exact["state"] == "verified"),
        "validation": program_validation_summary::summary(report, model, exact.as_ref()), "exact_collisions": exact,
    })
}

/// Takes the cause recorded by the last `failure`, if any. Callers that turn
/// an error code into something other than a bridge response use this.
pub(crate) fn take_error_details() -> Option<Value> {
    ERROR_DETAILS.with(|slot| slot.borrow_mut().take())
}

const MAX_REPORTED_ISSUES: usize = 12;
const MAX_REPORTED_RELATIONS: usize = 8;
const MAX_REPORTED_PARTS: usize = 100;
const MAX_ISSUE_BYTES: usize = 1024;

/// One issue reduced to its scalar fields when it is too large for a frame.
fn compact_issue(issue: &Value) -> Value {
    if serde_json::to_vec(issue).map_or(0, |bytes| bytes.len()) <= MAX_ISSUE_BYTES {
        return issue.clone();
    }
    let mut compact = serde_json::Map::new();
    let mut size = 0;
    for (key, value) in issue.as_object().into_iter().flatten() {
        let scalar = value.is_string() || value.is_number() || value.is_boolean();
        let length = serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len());
        if scalar && size + key.len() + length < MAX_ISSUE_BYTES {
            size += key.len() + length;
            compact.insert(key.clone(), value.clone());
        }
    }
    compact.insert("truncated".to_owned(), json!(true));
    Value::Object(compact)
}

/// Issues of every validator that reported any, flattened and bounded so the
/// response stays inside one bridge frame. `validation.issue_count` keeps the total.
fn validation_issues(validation: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    if let Some(validators) = validation.as_object() {
        for (validator, report) in validators {
            for issue in report["issues"].as_array().into_iter().flatten() {
                if issues.len() < MAX_REPORTED_ISSUES {
                    let mut issue = compact_issue(issue);
                    if let Some(object) = issue.as_object_mut() {
                        object.insert("validator".to_owned(), json!(validator));
                    }
                    issues.push(issue);
                }
            }
        }
    }
    issues
}

impl Response {
    fn error(id: u64, code: &str) -> Self {
        Self {
            version: 1,
            id,
            ok: false,
            stamp: None,
            result: Some(rejection_result(
                code,
                ERROR_DETAILS.with(|slot| slot.borrow_mut().take()),
            )),
            error: Some(code.into()),
        }
    }

    /// Non-fatal answer to a request body that does not match the protocol;
    /// `reason` is the parser's schema message so the caller can fix it.
    fn invalid_params(id: u64, reason: &str) -> Self {
        let reason: String = reason
            .chars()
            .take(MAX_INVALID_PARAMS_REASON_CHARS)
            .collect();
        Self {
            version: 1,
            id,
            ok: false,
            stamp: None,
            result: serde_json::to_value(rejected("invalid_params").reason(reason)).ok(),
            error: Some("invalid_params".into()),
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
#[derive(Debug)]
pub(crate) enum PlanRejection {
    Code(&'static str),
    /// The program could not be planned; the diagnostic says why.
    Planning(Box<AssistantRejectionDiagnostic>),
    CapabilityGap(Box<AssistantRejectionDiagnostic>),
}
impl From<&'static str> for PlanRejection {
    fn from(code: &'static str) -> Self {
        Self::Code(code)
    }
}
impl PlanRejection {
    /// The bridge error code, with the diagnostic recorded as its details.
    fn into_code(self) -> &'static str {
        match self {
            Self::Code(code) => code,
            Self::Planning(diagnostic) => planning_failure("planning_rejected", &diagnostic),
            Self::CapabilityGap(diagnostic) => planning_failure("capability_gap", &diagnostic),
        }
    }
}
pub(crate) struct ApplyAndVerifyRequest {
    expected: Option<Stamp>,
    selection: Option<Vec<u64>>,
    program: AssistantCadEditProgram,
    validators: Vec<String>,
    timeout_ms: u64,
    save: Option<ApplyAndVerifySave>,
    strict: bool,
}
/// Everything decided on the UI thread before the off-thread exact/validation work.
struct ApplyAndVerifyPlan {
    before: Stamp,
    selection: SelectionGuard,
    proposal: Proposal,
    candidate: Snapshot,
    save_path: Option<PathBuf>,
    timeout_ms: u64,
    strict: bool,
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
            .map_err(|error| match error {
                crate::WorkRecoveryMutationError::Mutation(error) => {
                    OccurrenceBatchError::host_transaction(error)
                }
                crate::WorkRecoveryMutationError::Recovery(error) => {
                    OccurrenceBatchError::host_transaction(error)
                }
            })
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
    program_check_job: Option<program_check::ProgramCheckJob>,
    program_report: Option<(Stamp, ketchup_program::Report, &'static str)>,
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
        if self.live.bridge.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "bridge already enabled",
            ));
        }
        let bridge = transport::start(context.clone())?;
        let address = bridge.address;
        self.live.bridge = Some(bridge);
        Ok(address)
    }

    /// Secret-bearing out-of-band accessor for trusted host code only.
    pub fn live_bridge_credentials(&self) -> Option<Credentials> {
        self.live.bridge.as_ref().map(|b| Credentials {
            address: b.address,
            token: b.token.clone(),
        })
    }

    pub fn disable_live_bridge(&mut self) {
        self.live.bridge = None;
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
        let Some(mut bridge) = self.live.bridge.take() else {
            return;
        };
        let mut revoke_consent = false;
        let ui_busy = ui_busy(context);
        bridge.poll_apply_and_verify_job(self, context, ui_busy);
        bridge.poll_program_check_job(self, context);
        for _ in 0..4 {
            let Ok(queued) = bridge.queue.try_recv() else {
                break;
            };
            if queued.connection_closed {
                bridge.remove_client(queued.session);
                if self.live.consent_attached {
                    revoke_consent = true;
                    break;
                }
                continue;
            }
            if queued.cancelled.load(Ordering::Acquire) {
                continue;
            }
            if bridge.session != queued.session
                && (bridge.image.is_pending()
                    || bridge.apply_and_verify_job.is_some()
                    || bridge.program_check_job.is_some())
            {
                let _ = busy::reject_if_busy(&bridge.busy_jobs());
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
            if let Request::Image(image) = &queued.request {
                let image = image.clone();
                bridge.request_image(self, context, image, queued);
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
                    strict,
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
                            strict,
                        },
                        ui_busy,
                    );
                    continue;
                }
                Request::MeasureFaces {
                    expected,
                    faces,
                    mode,
                    direction,
                } => {
                    bridge.start_measurement(
                        self, context, id, reply, cancelled, expected, faces, mode, direction,
                    );
                    continue;
                }
                Request::ValidateProgram {
                    expected,
                    validators,
                    motion,
                } => {
                    bridge.start_queued_validate_program(
                        self, context, id, reply, cancelled, expected, validators, motion,
                    );
                    continue;
                }
                request @ (Request::ApplyProgram { .. } | Request::PatchProgram { .. }) => {
                    bridge.start_queued_apply_program(
                        self, context, id, reply, cancelled, request, ui_busy,
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
            if disconnect && self.live.consent_attached {
                revoke_consent = true;
                break;
            }
        }
        if revoke_consent {
            self.live.consent_attached = false;
            self.poll_live_consent(context);
        } else {
            self.live.bridge = Some(bridge);
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

    fn available(app: &KetchupApp, ui_busy: bool) -> Result<(), &'static str> {
        if app.file.review_candidate.is_some() {
            return Err("read_only_document");
        }
        busy::reject_if_busy(&busy::blockers(app, ui_busy))
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

    fn selected_context(app: &KetchupApp) -> Value {
        let snapshot = app.document.current();
        let paths = app.selected_instance_paths();
        if paths.len() > MAX_SELECTION {
            return json!({"state":"selection_limit"});
        }
        let selected_paths = paths
            .iter()
            .map(|path| {
                json!({"root_occurrence_id":path.root_occurrence().0,
                "steps":path.steps().iter().map(|step| match step {
                    ketchup_model::document::InstancePathStep::Group(id) =>
                        json!({"kind":"group","local_id":id.0}),
                    ketchup_model::document::InstancePathStep::Occurrence(id) =>
                        json!({"kind":"occurrence","local_id":id.0}),
                }).collect::<Vec<_>>()})
            })
            .collect::<Vec<_>>();
        if app.selection.topological.is_empty() {
            return json!({"state":if app.selection.selected_group.is_some() {"group_only"}
                else if paths.is_empty() {"empty"} else {"part_only"},
                "instance_paths":selected_paths,"selected_group_id":app.selection.selected_group.map(|id|id.0),
                "pin_pair":Value::Null});
        }
        if app.selection.topological.len() != 1 || paths.len() != 1 {
            return json!({"state":"multiple_topological_elements",
                "instance_paths":selected_paths,"pin_pair":Value::Null});
        }
        let target = app.selection.topological[0]
            .1
            .resolve_current(&snapshot, &app.exact.topology_results);
        let Ok(target) = target else {
            return json!({"state":"stale_topology","instance_paths":selected_paths,
                "pin_pair":Value::Null});
        };
        let reference = &target.reference;
        let edges = snapshot
            .resolve_instance_path(&target.instance_path)
            .ok()
            .and_then(|instance| {
                app.exact
                    .topology_results
                    .get_render(&snapshot, instance.definition_id)
            })
            .map(|package| {
                let ordinal = package
                    .topological_references()
                    .iter()
                    .filter(|candidate| candidate.kind == reference.kind)
                    .position(|candidate| candidate == reference);
                package
                    .edge_evidence()
                    .iter()
                    .filter(|edge| match (reference.kind, ordinal) {
                        (ketchup_model::topology::TopologicalElementKind::Edge, Some(index)) => {
                            edge.edge_ordinal == index as u32
                        }
                        (ketchup_model::topology::TopologicalElementKind::Face, Some(index)) => {
                            edge.adjacent_face_ordinals.contains(&(index as u32))
                        }
                        _ => false,
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let matches = snapshot
            .pin_joints()
            .filter_map(|joint| {
                let bindings = joint.physical_hole_pairs.as_ref()?;
                let projection =
                    ketchup_model::pin_joint::project_pin_joint_contract(&snapshot, joint).ok()?;
                Some(
                    bindings
                        .iter()
                        .zip(projection.pairs.iter())
                        .enumerate()
                        .filter_map(|(index, (binding, pair))| {
                            let (hole, side) = if target.instance_path == pair.first.instance_path {
                                (&pair.first, "first")
                            } else if target.instance_path == pair.second.instance_path {
                                (&pair.second, "second")
                            } else {
                                return None;
                            };
                            let linear_mm = snapshot.tolerance().linear_mm();
                            let matches_hole = edges.iter().any(|edge| {
                                let (Some(radius), Some(center), Some(axis)) = (
                                    edge.circle_radius_mm,
                                    edge.axis_origin_mm,
                                    edge.unit_axis_direction,
                                ) else {
                                    return false;
                                };
                                let delta = std::array::from_fn::<_, 3, _>(|i| {
                                    center[i] - hole.entry_local_mm[i]
                                });
                                let depth = (0..3)
                                    .map(|i| delta[i] * hole.inward_unit_local[i])
                                    .sum::<f64>();
                                (radius - hole.diameter_mm / 2.0).abs() <= linear_mm
                                    && (0..3)
                                        .map(|i| axis[i] * hole.inward_unit_local[i])
                                        .sum::<f64>()
                                        .abs()
                                        >= 1.0 - ROUNDING
                                    && depth >= -linear_mm
                                    && depth <= hole.depth_mm + linear_mm
                                    && (0..3)
                                        .map(|i| {
                                            (delta[i] - depth * hole.inward_unit_local[i]).powi(2)
                                        })
                                        .sum::<f64>()
                                        <= linear_mm * linear_mm
                            });
                            matches_hole.then(|| {
                                json!({"joint_id":joint.id.0,
                    "joint_name":joint.name,"pair_index":index,
                    "first_pocket_feature_id":binding.first_pocket_feature_id.0,
                    "second_pocket_feature_id":binding.second_pocket_feature_id.0,
                    "selected_side":side})
                            })
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .take(2)
            .collect::<Vec<_>>();
        json!({"state":if matches.len() == 1 {"pin_pair"}
            else if matches.is_empty() {"topological_element"} else {"ambiguous_pin_pair"},
            "instance_paths":selected_paths,
            "topology":{"definition_id":reference.definition_id.0,
                "source_feature_id":reference.source_feature_id.0,
                "producer_feature_id":reference.producer_feature_id.0,
                "kind":match reference.kind {
                    ketchup_model::topology::TopologicalElementKind::Face => "face",
                    ketchup_model::topology::TopologicalElementKind::Edge => "edge",
                    ketchup_model::topology::TopologicalElementKind::Vertex => "vertex",
                }},
            "program":program_pick::describe(app, &snapshot, &target.instance_path, reference),
            "pin_pair":if matches.len() == 1 {matches.into_iter().next()} else {None}})
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
        Self::visible_root_ids(app, ids)
    }

    fn visible_root_ids(app: &KetchupApp, ids: &[u64]) -> Result<(), &'static str> {
        let snapshot = app.document.current();
        for id in ids {
            snapshot
                .occurrence(OccurrenceId(*id))
                .ok_or("entity_not_found")?;
        }
        let selectable: BTreeSet<_> = app
            .active_scene_query()
            .into_iter()
            .filter(|o| o.visible && o.instance_path.is_root())
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
        Self::reply(
            app,
            id,
            reply,
            Err(planning_failure("capability_gap", diagnostic)),
        );
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
        container_data: &ketchup_model::persistence::ContainerData,
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
        let products = exact_task.wait(remaining).map_err(|error| match error {
            // A wait timeout also raises the shared cancel flag; report it as a timeout.
            ExactEvaluationError::TimedOut => "job_timeout",
            _ if cancelled.load(Ordering::Acquire) => "request_cancelled",
            ExactEvaluationError::Cancelled => "request_cancelled",
            ExactEvaluationError::WorkerDisconnected => "exact_worker_disconnected",
            _ => "exact_evaluation_rejected",
        })?;
        Self::require_request_authority(&cancelled)?;
        let (candidate_exact, candidate_topology, exact_report) =
            materialize_exact_products(candidate, render, topology, &exact_task, products)
                .map_err(|error| failed_because("exact_evaluation_rejected", error))?;
        // A candidate without exact geometry (e.g. a cleared document) has
        // nothing to verify; every producer it does have must be evaluated.
        if exact_report
            .producers
            .iter()
            .any(|producer| !producer.render.is_evaluated() || !producer.topology.is_evaluated())
        {
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
        app.exact.worker_path.clone().or_else(|| {
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
            strict,
        } = request;
        let started = Instant::now();
        if timeout_ms == 0 || timeout_ms > MAX_APPLY_VERIFY_TIMEOUT_MS {
            return Err(PlanRejection::Code("invalid_job_timeout"));
        }
        let save_path = match save {
            None => None,
            Some(ApplyAndVerifySave::Current {}) => {
                Some(app.file.path.clone().ok_or("save_path_required")?)
            }
            Some(ApplyAndVerifySave::Path { path }) => {
                if path.is_empty()
                    || path.len() > 4096
                    || path.contains('\0')
                    || !Path::new(&path).is_absolute()
                {
                    return Err(PlanRejection::Code("invalid_path"));
                }
                Some(PathBuf::from(path))
            }
        };
        let validation_selection = Self::mandatory_validation_selection(&validators)?;
        Self::guard(app, &expected)?;
        Self::available(app, ui_busy)?;
        let selection = Self::selection_guard(app, selection)?;
        program
            .validate()
            .map_err(|error| failure("invalid_program", error.to_string(), json!({})))?;
        Self::program_scope(app, &program)?;
        reject_program_detach(app, strict)?;
        #[cfg(test)]
        if fault == Some(ApplyAndVerifyFault::Planning) {
            return Err(PlanRejection::Code("planning_rejected"));
        }
        let proposal = app
            .derive_assistant_cad_edit_proposal(&program)
            .map_err(|diagnostic| {
                if Self::is_capability_gap(&diagnostic) {
                    PlanRejection::CapabilityGap(diagnostic)
                } else {
                    PlanRejection::Planning(diagnostic)
                }
            })?;
        #[cfg(test)]
        if fault == Some(ApplyAndVerifyFault::Candidate) {
            return Err(PlanRejection::Code("candidate_rejected"));
        }
        let candidate = app
            .document
            .preview_verified_proposal(&proposal)
            .map_err(|error| failure("candidate_rejected", format!("{error:?}"), json!({})))?;
        let exact_selection = plan_incremental_exact_evaluation(
            &app.document.current(),
            &candidate,
            app.exact.source.as_ref(),
        )
        .selection;
        let plan = ApplyAndVerifyPlan {
            before: app.live_bridge_stamp(),
            selection,
            proposal,
            candidate,
            save_path,
            timeout_ms,
            strict,
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
        take_error_details();
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
            Err(rejection) => {
                Self::reply(app, id, &reply, Err(rejection.into_code()));
                return;
            }
        };
        let container_data = app.file.container_data.clone();
        let render = app.exact.results.clone();
        let topology = app.exact.topology_results.clone();
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
            strict,
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
        let issues = validation_issues(&validation);
        if *strict && validation["state"] != "passed" {
            let code = if validation["state"] == "failed" {
                "validation_failed"
            } else {
                "validation_incomplete"
            };
            return Err(failure(
                code,
                format!(
                    "validation {} with {} issue(s); the edit was not published (strict)",
                    validation["state"].as_str().unwrap_or("did not pass"),
                    validation["issue_count"]
                ),
                json!({"issues": issues, "not_evaluated": validation["not_evaluated"]}),
            ));
        }
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
        let owned_before = program_owned(app);
        let committed = app
            .complete_mutation_and_exact_results_with_work_recovery(
                |document, exact_results, topology_results| {
                    let committed =
                        document
                            .commit_verified_proposal(proposal)
                            .map_err(|error| {
                                failure("commit_rejected", format!("{error:?}"), json!({}))
                            })?;
                    for reference in exact_references {
                        document
                            .register_exact_reference_evidence(reference)
                            .map_err(|error| failed_because("exact_reference_rejected", error))?;
                    }
                    document
                        .register_exact_reference_evidence(&candidate_exact)
                        .map_err(|error| failed_because("exact_reference_rejected", error))?;
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
        app.exact.source = Some(exact_source(candidate));
        app.exact.retry_at = None;
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
        let mut value = json!({
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
                "issues": issues,
                "not_evaluated": validation["not_evaluated"],
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
        });
        add_detach_warning(&mut value, owned_before, app);
        Ok(value)
    }

    /// Synchronous variant for callers already off the bridge queue.
    fn apply_and_verify_now(
        app: &mut KetchupApp,
        request: ApplyAndVerifyRequest,
        ui_busy: bool,
        cancelled: &Arc<AtomicBool>,
        #[cfg(test)] fault: Option<ApplyAndVerifyFault>,
    ) -> Result<Value, PlanRejection> {
        let (plan, exact_selection, validation_selection) = Self::plan_apply_and_verify(
            app,
            request,
            ui_busy,
            #[cfg(test)]
            fault,
        )?;
        let prepared = Self::evaluate_apply_and_verify_candidate(
            &plan.candidate,
            &app.file.container_data,
            &app.exact.results,
            &app.exact.topology_results,
            Self::worker_path(app),
            &exact_selection,
            &validation_selection,
            plan.started,
            plan.timeout_ms,
            Arc::clone(cancelled),
            #[cfg(test)]
            fault,
        )?;
        Ok(Self::publish_apply_and_verify(
            app,
            &plan,
            prepared,
            ui_busy,
            cancelled,
            #[cfg(test)]
            fault,
        )?)
    }

    pub(crate) fn apply_assistant_cad_program(
        app: &mut KetchupApp,
        program: AssistantCadEditProgram,
    ) -> Result<Value, PlanRejection> {
        take_error_details();
        Self::apply_and_verify_now(
            app,
            ApplyAndVerifyRequest {
                expected: None,
                selection: None,
                program,
                validators: Vec::new(),
                timeout_ms: DEFAULT_APPLY_VERIFY_TIMEOUT_MS,
                save: None,
                strict: false,
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
        take_error_details();
        Self::require_request_authority(cancelled)?;
        match &request {
            Request::EditContext { expected, .. }
            | Request::Query { expected, .. }
            | Request::Detail { expected, .. }
            | Request::WorksetStatus { expected, .. }
            | Request::BatchJobStatus { expected, .. }
            | Request::View { expected, .. } => Self::guard(app, expected)?,
            _ => {}
        }
        match request {
            Request::Status {} => Ok(
                json!({"connected":true,"protocol":1,"image":"cad_viewport_png_thumbnail",
                "image_protocol":{"version":IMAGE_PROTOCOL_VERSION,"capabilities":["capture_mode","capture_metadata","render_metadata","variable_size","selection_framing","detail_selection_framing"],"capture_modes":["offscreen","visible_viewport"],"default_capture_mode":"offscreen","framing_modes":["viewport","selection","detail_selection"],"default_framing":"viewport","min_side_px":MIN_IMAGE_SIDE_PX,"max_side_px":MAX_IMAGE_SIDE_PX,"default_side_px":512},
                "busy":!self.busy_diagnostics(app, ui_busy).is_empty(),"busy_reasons":self.busy_diagnostics(app, ui_busy),"read_only":app.file.review_candidate.is_some(),
                "selection":Self::selection(app).ok(),"selection_scope":"root_occurrences_only",
                "selected_context":Self::selected_context(app),
                "undo_steps":app.undo_step_count(),"redo_steps":app.redo_step_count(),
                "pending_proposal_id":self.pending.as_ref().map(|p|p.id),
                "limits":{"frame_bytes":MAX_FRAME_BYTES,"image_frame_bytes":MAX_IMAGE_FRAME_BYTES,"queue":QUEUE_CAPACITY,"selection":MAX_SELECTION,"apply_verify_timeout_ms":MAX_APPLY_VERIFY_TIMEOUT_MS,"batch_jobs":limits::BATCH_JOBS},
                "methods":["status","summary","operations","edit_context","query","detail","workset_create","workset_status","batch_job_start","batch_job_status","batch_job_step","batch_job_cancel","propose","commit","apply_and_verify","program","program_context","patch_program","program_report","validate_program","measure_faces","apply_program","undo","redo","save","save_as","open","selection","view","image","disconnect"]}),
            ),
            Request::Summary {} => Ok(self.query.summary(&app.document.current())),
            Request::Operations { name } => {
                ketchup_assistant::catalog::cad_operation_catalog(name.as_deref())
            }
            Request::EditContext { targets, .. } => self
                .query
                .edit_context(
                    &app.document.current(),
                    &app.exact.topology_results,
                    app.document.mutation_epoch(),
                    &EditContextRequest { targets },
                )
                .map_err(|error| error.code()),
            Request::Query { query, .. } => self
                .query
                .page_with_topology(&app.document.current(), &app.exact.topology_results, &query)
                .map_err(|e| e.code()),
            Request::Detail {
                kind, entity_id, ..
            } => self
                .query
                .detail_with_topology(
                    &app.document.current(),
                    &app.exact.topology_results,
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
                if self.batch_jobs.len() == limits::BATCH_JOBS {
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
                let owned_before = program_owned(app);
                let receipt = self.batch_jobs[index]
                    .task
                    .commit_next(app)
                    .map_err(|error| failed_because(error.code(), error))?;
                if receipt.is_some() {
                    self.query.invalidate();
                    self.observed = Some(app.live_bridge_stamp());
                    app.invalidate_pending_import_reviews();
                    app.clear_ephemeral_edit_state();
                    app.reconcile_selection();
                    app.status_key = "status-ready";
                }
                let status = self.batch_jobs[index].task.status(&app.document);
                let mut value = json!({"job_handle":handle,"status":status,"receipt":receipt});
                add_detach_warning(&mut value, owned_before, app);
                Ok(value)
            }
            Request::Propose {
                expected,
                selection,
                program,
            } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                let selection = Self::selection_guard(app, selection)?;
                program
                    .validate()
                    .map_err(|error| failure("invalid_program", error.to_string(), json!({})))?;
                Self::program_scope(app, &program)?;
                let proposal = app
                    .derive_assistant_cad_edit_proposal(&program)
                    .map_err(|diagnostic| planning_failure("planning_rejected", &diagnostic))?;
                let id = self.next_proposal;
                self.next_proposal = id.checked_add(1).ok_or("proposal_ids_exhausted")?;
                let value = json!({"proposal_id":id,
                    "command_digest":proposal.command_digest(),"result_digest":proposal.intended_result_digest(),
                    "write_count":proposal.authoritative_writes().len(),
                    "detaches_program":program_owned(app)});
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
                let owned_before = program_owned(app);
                let committed = app
                    .commit_verified_proposal_with_work_recovery(&pending.proposal)
                    .map_err(|error| match error {
                        WorkRecoveryMutationError::Mutation(_) => "commit_rejected",
                        WorkRecoveryMutationError::Recovery(_) => "recovery_rejected",
                    })?;
                self.pending.take().expect("committed pending proposal");
                let mut value = json!({"proposal_id":proposal_id,"committed":true,
                    "after":app.live_bridge_stamp(),"command_digest":committed.command_digest(),
                    "result_digest":committed.result_digest(),"write_count":committed.verified_writes().len(),
                    "undo_steps":app.undo_step_count(),"geometry_evaluated":false});
                add_detach_warning(&mut value, owned_before, app);
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
                strict,
            } => Self::apply_and_verify_now(
                app,
                ApplyAndVerifyRequest {
                    expected,
                    selection,
                    program,
                    validators,
                    timeout_ms,
                    save,
                    strict,
                },
                ui_busy,
                cancelled,
                #[cfg(test)]
                self.apply_and_verify_fault,
            )
            .map_err(PlanRejection::into_code),
            request @ (Request::Program { .. }
            | Request::MeasureFaces { .. }
            | Request::ValidateProgram { .. }
            | Request::ProgramContext { .. }
            | Request::ProgramReport { .. }) => self.read_program_request(app, request, cancelled),
            request @ (Request::ApplyProgram { .. } | Request::PatchProgram { .. }) => self
                .apply_program(app, request, ui_busy, cancelled)
                .map(|applied| applied.result(None)),
            Request::Undo { expected } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                if !app.command_enabled(AppCommand::Undo) {
                    return Err("undo_unavailable");
                }
                Self::require_request_authority(cancelled)?;
                let changed = app.undo();
                Ok(json!({"changed":changed,"program_owned":program_owned(app)}))
            }
            Request::Redo { expected } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                if !app.command_enabled(AppCommand::Redo) {
                    return Err("redo_unavailable");
                }
                Self::require_request_authority(cancelled)?;
                let changed = app.redo();
                Ok(json!({"changed":changed,"program_owned":program_owned(app)}))
            }
            Request::Save { expected } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                let path = app.file.path.clone().ok_or("save_path_required")?;
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
                if !app.open_document_with_discard(Path::new(&path), true) {
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
                if !app.selection.edit_context.is_empty() {
                    return Err("unsupported_selection_scope");
                }
                let ids = Self::validate_ids(&occurrence_ids)?;
                Self::visible_root_ids(app, &ids)?;
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
                if !matches!(view, View::ZoomFit) {
                    app.camera.projection_mode = crate::ProjectionMode::Parallel;
                }
                app.dispatch_command(command);
                // A client cannot see the viewport: a standard view always frames the model.
                if command != AppCommand::ZoomFit && app.command_enabled(AppCommand::ZoomFit) {
                    app.dispatch_command(AppCommand::ZoomFit);
                }
                Ok(json!({"view":view,"canonical_mutation":false,"image":"not_requested"}))
            }
            Request::Image(ImageRequest {
                expected,
                image_protocol_version,
                ..
            }) => {
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
