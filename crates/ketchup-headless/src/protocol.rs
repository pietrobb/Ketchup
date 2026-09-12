use ketchup_application::evaluation::{
    EvaluationReport, EvidenceStatus, ExactEvaluationProgress, ExactEvaluationTask, ExactSource,
    ProducerKey,
};
mod model_tools;
use ketchup_application::batch_task::{
    OccurrenceBatchError, OccurrenceBatchOperation, OccurrenceBatchState, OccurrenceBatchTask,
};
use ketchup_application::model_query::{ModelQuery, created_receipt};
use ketchup_application::validation::{ASSISTANT_VALIDATOR_IDS, assistant_validator_catalog};
use ketchup_application::{
    AssistantValidationSelection, DocumentSession, SaveOptions, SessionError, SessionSettings,
};
use ketchup_core::assistant_sidecar::AssistantCadEditProgram;
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, FeatureId, OccurrenceId, Snapshot,
};
use ketchup_core::exact_product::{ExactBodyPackage, ExactResultRegistry};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeSet, VecDeque, hash_map::RandomState},
    hash::BuildHasher,
    io::{self, BufRead, Write},
    time::Instant,
};

pub const PROTOCOL: &str = "ketchup.headless.v1";
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_BATCH_JOBS: usize = 16;
pub const MAX_VERIFY_JOBS: usize = 16;
const METHODS: &[&str] = &[
    "capabilities",
    "new",
    "open",
    "state",
    "summary",
    "query",
    "detail",
    "workset_create",
    "workset_status",
    "batch_job_start",
    "batch_job_status",
    "batch_job_step",
    "batch_job_cancel",
    "apply",
    "evaluate",
    "verify_job_start",
    "verify_job_status",
    "verify_job_cancel",
    "list_validators",
    "run_validators",
    "set_grounded",
    "undo",
    "redo",
    "save",
];

#[derive(Debug)]
struct Error {
    code: String,
    message: String,
    details: Option<Value>,
}
impl Error {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }
    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_params", message)
    }
}
impl From<SessionError> for Error {
    fn from(error: SessionError) -> Self {
        if let SessionError::Planning(diagnostic) = error {
            return Self {
                code: diagnostic.code.clone(),
                message: diagnostic.failed_invariant.clone(),
                details: Some(json!(diagnostic)),
            };
        }
        let code = match &error {
            SessionError::Canonical(e) => e.code(),
            SessionError::Prepare(_) => "proposal_prepare_rejected",
            SessionError::Commit(_) => "proposal_commit_rejected",
            SessionError::Persistence(_) => "persistence_error",
            SessionError::ReviewOnly => "review_only",
            SessionError::NoUndo => "no_undo",
            SessionError::NoRedo => "no_redo",
            SessionError::Evaluation(_) => "evaluation_error",
            SessionError::Planning(_) => unreachable!(),
        };
        Self {
            code: code.into(),
            message: error.to_string(),
            details: Some(json!({"diagnostic":format!("{error:?}")})),
        }
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol: String,
    id: Value,
    method: String,
    params: Value,
}

struct BatchJob {
    handle: String,
    task: OccurrenceBatchTask,
}

enum VerifyJobState {
    Running(ExactEvaluationTask),
    Completed(Value),
    Failed(String),
    TimedOut(Value),
    Cancelled,
}

struct VerifyJob {
    handle: String,
    source: ExactSource,
    scope: Option<Vec<ProducerKey>>,
    progress: ExactEvaluationProgress,
    started_at: Instant,
    elapsed_ms: u64,
    timeout_ms: u64,
    deadline_stop: Option<std::sync::mpsc::Sender<()>>,
    state: VerifyJobState,
}

impl VerifyJob {
    fn terminal(&self) -> bool {
        !matches!(self.state, VerifyJobState::Running(_))
    }
}

pub struct Server {
    session: DocumentSession,
    settings: SessionSettings,
    // The process starts with an internal placeholder session so its first New/Open needs no discard.
    initial_placeholder: bool,
    model_queries: ModelQuery,
    batch_jobs: VecDeque<BatchJob>,
    batch_job_key: RandomState,
    next_batch_job: u64,
    verify_jobs: VecDeque<VerifyJob>,
    verify_job_key: RandomState,
    next_verify_job: u64,
    compact_result: bool,
}
impl Server {
    pub fn new(settings: SessionSettings) -> Self {
        Self {
            session: DocumentSession::new(settings.clone()),
            settings,
            initial_placeholder: true,
            model_queries: ModelQuery::default(),
            batch_jobs: VecDeque::new(),
            batch_job_key: RandomState::new(),
            next_batch_job: 1,
            verify_jobs: VecDeque::new(),
            verify_job_key: RandomState::new(),
            next_verify_job: 1,
            compact_result: false,
        }
    }
    fn batch_job_handle(&self, id: u64) -> String {
        format!("batch-{id:016x}-{:016x}", self.batch_job_key.hash_one(id))
    }

    fn verify_job_handle(&self, id: u64) -> String {
        format!("verify-{id:016x}-{:016x}", self.verify_job_key.hash_one(id))
    }

    fn revoke_jobs(&mut self) {
        self.batch_jobs.clear();
        self.batch_job_key = RandomState::new();
        self.next_batch_job = 1;
        self.verify_jobs.clear();
        self.verify_job_key = RandomState::new();
        self.next_verify_job = 1;
    }

    fn start_verify_job(
        &mut self,
        scope: Option<BTreeSet<ProducerKey>>,
        timeout_ms: u64,
    ) -> Result<Value> {
        if self.verify_jobs.len() == MAX_VERIFY_JOBS {
            let terminal = self
                .verify_jobs
                .iter()
                .position(VerifyJob::terminal)
                .ok_or_else(|| {
                    Error::new(
                        "verify_job_limit",
                        "all bounded Verify job slots are active",
                    )
                })?;
            self.verify_jobs.remove(terminal);
        }
        let id = self.next_verify_job;
        self.next_verify_job = id
            .checked_add(1)
            .ok_or_else(|| Error::new("verify_job_ids_exhausted", "Verify job IDs exhausted"))?;
        let handle = self.verify_job_handle(id);
        let started_at = Instant::now();
        let task = self
            .session
            .start_scoped_exact_evaluation_task(scope.as_ref());
        let source = task.source.clone();
        let progress = task.progress();
        let deadline_cancelled = std::sync::Arc::clone(&task.cancelled);
        let deadline_finished = std::sync::Arc::clone(&task.finished);
        let (deadline_stop, deadline_wait) = std::sync::mpsc::channel();
        let remaining =
            std::time::Duration::from_millis(timeout_ms).saturating_sub(started_at.elapsed());
        std::thread::spawn(move || {
            if matches!(
                deadline_wait.recv_timeout(remaining),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) && !deadline_finished.load(std::sync::atomic::Ordering::Acquire)
            {
                deadline_cancelled.store(true, std::sync::atomic::Ordering::Release);
            }
        });
        self.verify_jobs.push_back(VerifyJob {
            handle,
            source,
            scope: scope.map(|scope| scope.into_iter().collect()),
            progress,
            started_at,
            elapsed_ms: 0,
            timeout_ms,
            deadline_stop: Some(deadline_stop),
            state: VerifyJobState::Running(task),
        });
        Ok(self.verify_job_value(self.verify_jobs.len() - 1))
    }

    fn time_out_verify_job(&mut self, index: usize, progress: ExactEvaluationProgress) {
        if let VerifyJobState::Running(task) = &self.verify_jobs[index].state {
            task.cancel();
        }
        let active = progress.active_producer.map(|key| {
            json!({
                "definition_id":key.definition_id.0,
                "feature_id":key.feature_id.0,
                "elapsed_ms":progress.active_elapsed_ms,
            })
        });
        let remaining = progress
            .total_producers
            .saturating_sub(progress.completed_producers);
        self.verify_jobs[index].state = VerifyJobState::TimedOut(json!({
            "code":"verify_timeout",
            "message":"exact Verify exceeded its native deadline",
            "elapsed_ms":self.verify_jobs[index].elapsed_ms,
            "timeout_ms":self.verify_jobs[index].timeout_ms,
            "active_body":active,
            "completed_bodies":progress.completed_producers,
            "remaining_bodies":remaining,
        }));
    }

    fn refresh_verify_job(&mut self, index: usize) {
        let (progress, result) = match &self.verify_jobs[index].state {
            VerifyJobState::Running(task) => (task.progress(), Some(task.poll())),
            _ => return,
        };
        self.verify_jobs[index].progress = progress;
        self.verify_jobs[index].elapsed_ms = self.verify_jobs[index]
            .started_at
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        match result.expect("running Verify job has a poll result") {
            Ok(Ok(products)) => {
                let published = {
                    let VerifyJobState::Running(task) = &self.verify_jobs[index].state else {
                        unreachable!("Verify job state changed while polling")
                    };
                    self.session.publish_exact_evaluation(task, products)
                };
                self.verify_jobs[index].state = match published {
                    Ok(report) => {
                        VerifyJobState::Completed(evaluation_report(&self.session, &report))
                    }
                    Err(error) => VerifyJobState::Failed(error.to_string()),
                };
            }
            Ok(Err(reason)) => self.verify_jobs[index].state = VerifyJobState::Failed(reason),
            Err(std::sync::mpsc::TryRecvError::Empty)
                if self.verify_jobs[index].elapsed_ms >= self.verify_jobs[index].timeout_ms =>
            {
                self.time_out_verify_job(index, progress);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected)
                if self.verify_jobs[index].elapsed_ms >= self.verify_jobs[index].timeout_ms =>
            {
                self.time_out_verify_job(index, progress);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.verify_jobs[index].state =
                    VerifyJobState::Failed("exact evaluation worker disconnected".into());
            }
        }
        if self.verify_jobs[index].terminal()
            && let Some(stop) = self.verify_jobs[index].deadline_stop.take()
        {
            let _ = stop.send(());
        }
        if matches!(
            &self.verify_jobs[index].state,
            VerifyJobState::Completed(_) | VerifyJobState::Failed(_)
        ) {
            self.verify_jobs[index].progress.active_producer = None;
            self.verify_jobs[index].progress.active_elapsed_ms = None;
        }
    }

    fn verify_job_value(&self, index: usize) -> Value {
        let job = &self.verify_jobs[index];
        let mut value = json!({
            "job_handle":job.handle,
            "source":{"document_id":job.source.0.0,"revision":job.source.1,"canonical_digest":job.source.2},
            "scope":job.scope.as_ref().map(|scope| scope.iter().map(|key| json!({"definition_id":key.definition_id.0,"feature_id":key.feature_id.0})).collect::<Vec<_>>()),
            "progress":{"total_bodies":job.progress.total_producers,"completed_bodies":job.progress.completed_producers,"reused_bodies":job.progress.reused_producers,
                "active_body":job.progress.active_producer.map(|key| json!({"definition_id":key.definition_id.0,"feature_id":key.feature_id.0,"elapsed_ms":job.progress.active_elapsed_ms}))},
            "timing":{"elapsed_ms":job.elapsed_ms,"timeout_ms":job.timeout_ms},
            "state":match job.state {
                VerifyJobState::Running(_) => "running",
                VerifyJobState::Completed(_) => "completed",
                VerifyJobState::Failed(_) => "failed",
                VerifyJobState::TimedOut(_) => "timed_out",
                VerifyJobState::Cancelled => "cancelled",
            }
        });
        match &job.state {
            VerifyJobState::Completed(report) => value["report"] = report.clone(),
            VerifyJobState::Failed(reason) => value["diagnostic"] = json!(reason),
            VerifyJobState::TimedOut(diagnostic) => value["diagnostic"] = diagnostic.clone(),
            VerifyJobState::Running(_) | VerifyJobState::Cancelled => {}
        }
        value
    }

    fn state(&self) -> Value {
        let s = self.session.snapshot();
        json!({"document_id":s.document_id().0,"revision":s.revision_id(),"canonical_digest":s.canonical_digest(),
            "undo_steps":self.session.visible_undo_steps(),"redo_steps":self.session.visible_redo_steps(),
            "definitions":s.definitions().map(|d| json!({"id":d.id().0,"name":d.name(),"feature_ids":d.feature_ids().iter().map(|id|id.0).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "occurrences":s.occurrences().map(|o| json!({"id":o.id().0,"definition_id":o.definition_id().0,"name":o.name(),"transform":o.transform().matrix(),"color":o.color()})).collect::<Vec<_>>(),
            "features":s.features().map(|f| json!({"id":f.id().0,"definition_id":f.definition_id().0,"name":f.name(),"kind":format!("{:?}",f.kind()).split([' ', '{', '(']).next().unwrap_or("unknown")})).collect::<Vec<_>>(),
            "grounded_occurrence_ids":s.grounded_occurrences().map(|id|id.0).collect::<Vec<_>>()})
    }
    fn state_result(&self) -> Value {
        if self.compact_result {
            return self.compact_state_result();
        }
        json!({"state":self.state(),"path":self.session.path().map(|p|p.to_string_lossy()),
            "recovery":self.session.recovery_state().map(|recovery| json!({
                "requested_path":recovery.requested_path().to_string_lossy(),
                "source_path":recovery.source_path().to_string_lossy(),
                "save_as_required":true
            })),
            "modified":self.session.is_modified()})
    }
    fn guard(&self, p: &Map<String, Value>) -> Result<()> {
        let revision = uint(p, "expected_revision")?;
        let digest = string(p, "expected_digest")?;
        let s = self.session.snapshot();
        if revision != s.revision_id() || digest != s.canonical_digest() {
            return Err(Error {
                code: "stale_state".into(),
                message: "expected revision/digest does not match observed document".into(),
                details: Some(
                    json!({"revision":s.revision_id(),"canonical_digest":s.canonical_digest(),"repair_hint":"Read state and explicitly re-plan; do not blindly retry a mutation."}),
                ),
            });
        }
        Ok(())
    }
    fn discard_guard(&self, p: &Map<String, Value>) -> Result<()> {
        if !boolean(p, "discard_unsaved", false)?
            && !self.initial_placeholder
            && self.session.is_modified()
        {
            return Err(Error::new(
                "unsaved_changes",
                "new/open requires discard_unsaved=true for unsaved changes",
            ));
        }
        Ok(())
    }
    fn dispatch_inner(&mut self, method: &str, params: Value) -> Result<Value> {
        let p = params
            .as_object()
            .ok_or_else(|| Error::invalid("params must be an object"))?;
        let (fields, mutation): (&[&str], bool) = match method {
            "capabilities" | "state" | "list_validators" => (&[], false),
            "new" => (&["discard_unsaved"], true),
            "open" => (&["path", "discard_unsaved"], true),
            "apply" => (&["program", "selection"], true),
            "evaluate" => (&["timeout_ms"], false),
            "run_validators" => (&["ids"], false),
            "set_grounded" => (&["occurrence_ids", "grounded"], true),
            "undo" | "redo" => (&[], true),
            "save" => (&["path", "overwrite"], true),
            _ => {
                return Err(Error::new(
                    "unknown_method",
                    format!("unknown method {method}"),
                ));
            }
        };
        for key in p.keys() {
            if !fields.contains(&key.as_str())
                && !(mutation && ["expected_revision", "expected_digest"].contains(&key.as_str()))
            {
                return Err(Error::invalid(format!("unknown field {key}")));
            }
        }
        if mutation {
            self.guard(p)?;
        }
        match method {
            "capabilities" => Ok(
                json!({"methods":METHODS.iter().map(|name| json!({"name":name,"mutates":matches!(*name,"new"|"open"|"apply"|"batch_job_step"|"set_grounded"|"undo"|"redo"|"save")})).collect::<Vec<_>>(),
                "cad_program_schema":serde_json::from_str::<Value>(include_str!(concat!(env!("OUT_DIR"),"/cad-program-schema.json"))).expect("build-generated schema"),
                "bounds":{"max_line_bytes":MAX_LINE_BYTES,"max_output_bytes":MAX_LINE_BYTES,"max_selection":100,"max_operations":64,"max_batch_jobs":MAX_BATCH_JOBS,"max_verify_jobs":MAX_VERIFY_JOBS,"evaluation_timeout_ms":{"default":30000,"min":1,"max":300000}},
                "mutation_preconditions":["expected_revision","expected_digest"],"units":"mm","transform":"row-major 4x4 local occurrence transform","transactions":"one apply = one atomic CAD program; newly allocated Definition, Sketch and body references use zero-based earlier operation_index plus a typed output, never guessed IDs","protocol":PROTOCOL}),
            ),
            "state" => Ok(self.state_result()),
            "new" => {
                self.discard_guard(p)?;
                self.session = DocumentSession::new(self.settings.clone());
                self.revoke_jobs();
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "open" => {
                self.discard_guard(p)?;
                let next = DocumentSession::open(string(p, "path")?, self.settings.clone())?;
                self.session = next;
                self.revoke_jobs();
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "save" => {
                self.session.save(
                    string(p, "path")?,
                    SaveOptions {
                        overwrite: boolean(p, "overwrite", false)?,
                    },
                )?;
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "apply" => {
                let program: AssistantCadEditProgram = serde_json::from_value(
                    p.get("program")
                        .cloned()
                        .ok_or_else(|| Error::invalid("missing program"))?,
                )
                .map_err(|e| Error::invalid(e.to_string()))?;
                let selection = ids(p, "selection", true)?
                    .into_iter()
                    .map(OccurrenceId)
                    .collect();
                let before = self.session.snapshot();
                self.session.apply_cad_program(&program, &selection)?;
                self.initial_placeholder = false;
                let mut result = self.state_result();
                result["created"] = if self.compact_result {
                    created_receipt(&before, &self.session.snapshot())
                } else {
                    created(&before, &self.session.snapshot())
                };
                Ok(result)
            }
            "set_grounded" => {
                let selected = ids(p, "occurrence_ids", false)?;
                let grounded = p
                    .get("grounded")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| Error::invalid("grounded must be boolean"))?;
                let batch = CommandBatch::new(
                    selected
                        .into_iter()
                        .map(|id| CanonicalCommand::SetOccurrenceGrounded {
                            id: OccurrenceId(id),
                            grounded,
                        })
                        .collect(),
                );
                // This is the only canonical-command adapter, not a raw command endpoint.
                let proposal = self.session.plan_commands(batch)?;
                self.session.apply_proposal(&proposal)?;
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "undo" => {
                self.session.undo()?;
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "redo" => {
                self.session.redo()?;
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "list_validators" => Ok(json!({"validators":assistant_validator_catalog()})),
            "run_validators" => {
                let names = p
                    .get("ids")
                    .and_then(Value::as_array)
                    .ok_or_else(|| Error::invalid("ids must be an array"))?;
                let names = names
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| Error::invalid("validator id must be string"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                if names.is_empty()
                    || names.len() > ASSISTANT_VALIDATOR_IDS.len()
                    || names.iter().collect::<BTreeSet<_>>().len() != names.len()
                {
                    return Err(Error::invalid("validator ids must be nonempty and unique"));
                }
                let selection = AssistantValidationSelection::only(&names);
                if !selection.is_valid() {
                    return Err(Error::invalid("unknown validator id"));
                }
                Ok(self.session.validators(&selection))
            }
            "evaluate" => {
                let timeout = uint(p, "timeout_ms")?;
                // The session accounts for preparation and waiting in this budget,
                // and rejects expired results before publication.
                if !(1..=300_000).contains(&timeout) {
                    return Err(Error::new(
                        "invalid_params",
                        "timeout_ms must be between 1 and 300000",
                    ));
                }
                let report = self
                    .session
                    .evaluate_with_timeout(std::time::Duration::from_millis(timeout))?;
                Ok(evaluation_report(&self.session, &report))
            }
            _ => unreachable!(),
        }
    }
    pub fn handle(&mut self, line: &[u8]) -> Value {
        let value: Value = match crate::json_input::parse(line) {
            Ok(v) => v,
            Err(e) => return failure(Value::Null, Error::new("invalid_json", e.to_string())),
        };
        let id = value
            .get("id")
            .filter(|id| valid_id(id))
            .cloned()
            .unwrap_or(Value::Null);
        let request: Request = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => return failure(id, Error::new("invalid_request", e.to_string())),
        };
        if !valid_id(&request.id) {
            return failure(
                Value::Null,
                Error::new(
                    "invalid_request",
                    "id must be null, u64, or a string of at most 256 bytes",
                ),
            );
        }
        if request.protocol != PROTOCOL {
            return failure(
                id,
                Error::new("unsupported_protocol", "expected ketchup.headless.v1"),
            );
        }
        match self.dispatch(&request.method, request.params) {
            Ok(result) => json!({"protocol":PROTOCOL,"id":id,"result":result}),
            Err(error) => failure(id, error),
        }
    }
}
fn valid_id(id: &Value) -> bool {
    id.is_null() || id.as_u64().is_some() || id.as_str().is_some_and(|s| s.len() <= 256)
}
fn failure(id: Value, error: Error) -> Value {
    let error = model_tools::bounded_error(error);
    let mut detail = json!({"code":error.code,"message":error.message});
    if let Some(details) = error.details {
        detail["details"] = details;
    }
    json!({"protocol":PROTOCOL,"id":id,"error":detail})
}
fn uint(p: &Map<String, Value>, key: &str) -> Result<u64> {
    p.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::invalid(format!("{key} must be u64")))
}
fn string<'a>(p: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    p.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::invalid(format!("{key} must be nonempty string")))
}
fn boolean(p: &Map<String, Value>, key: &str, default: bool) -> Result<bool> {
    p.get(key).map_or(Ok(default), |v| {
        v.as_bool()
            .ok_or_else(|| Error::invalid(format!("{key} must be boolean")))
    })
}
fn ids(p: &Map<String, Value>, key: &str, empty: bool) -> Result<Vec<u64>> {
    let values = match p.get(key) {
        None if empty => return Ok(Vec::new()),
        Some(Value::Array(v)) => v,
        _ => return Err(Error::invalid(format!("{key} must be an array"))),
    };
    if values.len() > 100 || (!empty && values.is_empty()) {
        return Err(Error::invalid("ID count out of bounds"));
    }
    let ids = values
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|id| *id != 0)
                .ok_or_else(|| Error::invalid("IDs must be positive u64"))
        })
        .collect::<Result<Vec<_>>>()?;
    if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(Error::invalid("duplicate IDs"));
    }
    Ok(ids)
}
fn created(before: &Snapshot, after: &Snapshot) -> Value {
    json!({"definition_ids":after.definitions().filter(|d|before.definition(d.id()).is_none()).map(|d|d.id().0).collect::<Vec<_>>(),
        "occurrence_ids":after.occurrences().filter(|o|before.occurrence(o.id()).is_none()).map(|o|o.id().0).collect::<Vec<_>>(),
        "feature_ids":after.features().filter(|f|before.feature(f.id()).is_none()).map(|f|f.id().0).collect::<Vec<_>>()})
}
fn evaluation_report(session: &DocumentSession, report: &EvaluationReport) -> Value {
    let snapshot = session.snapshot();
    let producers = report
        .producers
        .iter()
        .map(|producer| (producer.key.definition_id.0, producer.key.feature_id.0))
        .collect::<BTreeSet<_>>();
    json!({"document_id":report.source.0.0,"revision":report.source.1,"canonical_digest":report.source.2,
        "complete":report.complete,"topology_complete":report.topology_complete,"not_evaluated":report.not_evaluated,
        "producers":report.producers.iter().map(|p|json!({"definition_id":p.key.definition_id.0,"feature_id":p.key.feature_id.0,"render":status(&p.render),"topology":status(&p.topology)})).collect::<Vec<_>>(),
        "geometry":geometry(session.exact_results(),&snapshot,&producers),"topology_geometry":geometry(session.topology_results(),&snapshot,&producers)})
}

fn status(s: &EvidenceStatus) -> Value {
    match s {
        EvidenceStatus::Current => json!({"status":"current"}),
        EvidenceStatus::Evaluated => json!({"status":"evaluated"}),
        EvidenceStatus::Failed { reason } => json!({"status":"failed","reason":reason}),
        EvidenceStatus::NotEvaluated { reason } => {
            json!({"status":"not_evaluated","reason":reason})
        }
    }
}
fn geometry(
    registry: &ExactResultRegistry,
    snapshot: &Snapshot,
    producers: &BTreeSet<(u64, u64)>,
) -> Vec<Value> {
    registry.values().filter(|p| p.is_current(snapshot)
        && producers.contains(&(p.definition_id().0, p.producer_feature_id().0))).map(|p| {
        let key=p.result_key();
        // This volume is explicitly mesh evidence, not a fabricated native BRep volume.
        let vertices=p.vertices();
        let volume:f64=p.triangles().iter().map(|t| {
            let [a,b,c]=t.vertex_indices.map(|i|vertices[i as usize].position_mm);
            (a[0]*(b[1]*c[2]-b[2]*c[1])+a[1]*(b[2]*c[0]-b[0]*c[2])+a[2]*(b[0]*c[1]-b[1]*c[0]))/6.0
        }).sum();
        let kind=match p.as_ref() { ExactBodyPackage::Rectangle(_)=>"rectangle",ExactBodyPackage::Revolve(_)=>"revolve",ExactBodyPackage::Graph(_)=>"graph",ExactBodyPackage::Imported(_)=>"imported" };
        json!({"definition_id":p.definition_id().0,"feature_id":p.producer_feature_id().0,"kind":kind,"bounds_mm":p.bounds_mm(),
            "mesh_signed_volume_mm3":volume,"native_evidence":match p.as_ref() { ExactBodyPackage::Graph(g)=>json!({"volume_mm3":g.volume_mm3,"area_mm2":g.area_mm2,"topology_counts":g.topology_counts}), ExactBodyPackage::Imported(g)=>json!({"volume_mm3":g.volume_mm3,"topology_counts":g.topology_counts}), _=>Value::Null },"vertex_count":vertices.len(),"triangle_count":p.triangles().len(),
            "result_fingerprint":key.result_fingerprint,"canonical_input_digest":key.canonical_input_digest,"exact_input_digest":key.exact_input_digest,"backend":key.backend,"evaluator":key.evaluator,"tolerance":key.tolerance})
    }).collect()
}

// Both input and serialization enforce the wire bound without unbounded read_line.
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len() + bytes.len() > MAX_LINE_BYTES - 1 {
            return Err(io::Error::other("output limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub fn serve(
    mut input: impl BufRead,
    mut output: impl Write,
    mut server: Server,
) -> io::Result<()> {
    loop {
        let mut line = Vec::new();
        let mut oversized = false;
        let mut terminated = false;
        loop {
            let buffer = input.fill_buf()?;
            if buffer.is_empty() {
                break;
            }
            let take = buffer
                .iter()
                .position(|b| *b == b'\n')
                .map_or(buffer.len(), |i| i + 1);
            terminated = buffer[take - 1] == b'\n';
            if !oversized && line.len() + take <= MAX_LINE_BYTES {
                line.extend_from_slice(&buffer[..take]);
            } else {
                oversized = true;
                line.clear();
            }
            input.consume(take);
            if terminated {
                break;
            }
        }
        if line.is_empty() && !oversized && !terminated {
            return Ok(());
        }
        let response = if oversized {
            failure(
                Value::Null,
                Error::new("line_too_large", "maximum input line is 4 MiB"),
            )
        } else if !terminated {
            failure(
                Value::Null,
                Error::new("invalid_json", "unterminated JSON line"),
            )
        } else {
            server.handle(&line)
        };
        let mut bytes = Bounded(Vec::new());
        if serde_json::to_writer(&mut bytes, &response).is_err() {
            bytes.0.clear();
            let mut error = Error::new(
                "output_too_large",
                "response exceeds 4 MiB; query state before further mutation",
            );
            error.details = Some(json!({"mutation_outcome":"possibly_applied"}));
            serde_json::to_writer(&mut bytes, &failure(response["id"].clone(), error))?;
        }
        output.write_all(&bytes.0)?;
        output.write_all(b"\n")?;
        output.flush()?;
        if !terminated {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(server: &mut Server, method: &str, params: Value) -> Value {
        server.handle(
            serde_json::to_string(
                &json!({"protocol":PROTOCOL,"id":7,"method":method,"params":params}),
            )
            .unwrap()
            .as_bytes(),
        )
    }
    #[test]
    fn rejects_unknown_version_fields_and_stale_without_mutation() {
        let mut s = Server::new(SessionSettings::default());
        let before = s.state();
        for (method, params) in [
            ("wat", json!({})),
            ("state", json!({"extra":1})),
            (
                "new",
                json!({"expected_revision":999,"expected_digest":"stale","discard_unsaved":true}),
            ),
        ] {
            assert!(request(&mut s, method, params).get("error").is_some());
            assert_eq!(s.state(), before);
        }
        assert_eq!(
            s.handle(br#"{"protocol":"bad","id":"abc","method":"state","params":{}}"#)["error"]["code"],
            "unsupported_protocol"
        );
        assert_eq!(s.handle(br#"{"protocol":"ketchup.headless.v1","id":1,"method":"state","params":{"x":1e999}}"#)["error"]["code"],"invalid_json");
    }
    #[test]
    fn verify_jobs_start_publish_status_and_cancel_through_native_task() {
        let mut server = Server::new(SessionSettings::default());
        let state = request(&mut server, "state", json!({}))["result"]["state"].clone();
        let stale = request(
            &mut server,
            "verify_job_start",
            json!({"expected_revision":state["revision"],"expected_digest":"stale"}),
        );
        assert_eq!(stale["error"]["code"], "stale_state");

        let start = || {
            json!({"expected_revision":state["revision"],
                "expected_digest":state["canonical_digest"]})
        };
        let invalid_timeout = request(
            &mut server,
            "verify_job_start",
            json!({"expected_revision":state["revision"],
                "expected_digest":state["canonical_digest"],"timeout_ms":0}),
        );
        assert_eq!(invalid_timeout["error"]["code"], "invalid_params");

        let completed_before_late_poll = request(
            &mut server,
            "verify_job_start",
            json!({"expected_revision":state["revision"],
                "expected_digest":state["canonical_digest"],"timeout_ms":100}),
        );
        let completed_handle = completed_before_late_poll["result"]["job_handle"]
            .as_str()
            .unwrap()
            .to_owned();
        std::thread::sleep(std::time::Duration::from_millis(150));
        let completed = request(
            &mut server,
            "verify_job_status",
            json!({"handle":completed_handle}),
        );
        assert_eq!(completed["result"]["state"], "completed", "{completed}");
        assert!(
            completed["result"]["timing"]["elapsed_ms"]
                .as_u64()
                .unwrap()
                >= 100
        );

        let cancelled = request(&mut server, "verify_job_start", start());
        assert_eq!(cancelled["result"]["timing"]["timeout_ms"], 30_000);
        let cancelled_handle = cancelled["result"]["job_handle"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(cancelled["result"]["state"], "running");
        assert_eq!(cancelled["result"]["progress"]["total_bodies"], 0);
        let cancelled = request(
            &mut server,
            "verify_job_cancel",
            json!({"handle":cancelled_handle}),
        );
        assert_eq!(cancelled["result"]["state"], "cancelled");

        let started = request(&mut server, "verify_job_start", start());
        let handle = started["result"]["job_handle"].as_str().unwrap().to_owned();
        let mut status = Value::Null;
        for _ in 0..100 {
            status = request(&mut server, "verify_job_status", json!({"handle":handle}));
            if status["result"]["state"] == "completed" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(status["result"]["state"], "completed", "{status}");
        assert_eq!(status["result"]["report"]["complete"], false);
        assert_eq!(
            status["result"]["report"]["not_evaluated"],
            "no exact producers selected"
        );
        let unknown = request(&mut server, "verify_job_status", json!({"handle":"forged"}));
        assert_eq!(unknown["error"]["code"], "verify_job_not_found");
    }

    #[test]
    fn schema_covers_current_all_operation_variants() {
        let mut s = Server::new(SessionSettings::default());
        let caps = request(&mut s, "capabilities", json!({}));
        let variants =
            caps["result"]["cad_program_schema"]["$defs"]["AssistantCadEditOperation"]["oneOf"]
                .as_array()
                .unwrap();
        assert_eq!(variants.len(), 25);
        for operation in [
            "append_feature",
            "create_program_sketch",
            "append_program_pocket",
            "create_spatial_path",
            "create_helix_path",
            "create_construction_point",
            "create_construction_axis",
            "create_construction_plane",
            "create_helix",
            "create_thread",
            "fillet_edges",
            "chamfer_edges",
        ] {
            assert!(
                variants
                    .iter()
                    .any(|v| v["properties"]["operation"]["const"] == operation)
            );
        }
    }
    #[test]
    fn schema_covers_program_body_feature_references() {
        let mut s = Server::new(SessionSettings::default());
        let caps = request(&mut s, "capabilities", json!({}));
        let references =
            caps["result"]["cad_program_schema"]["$defs"]["AssistantCadFeatureReference"]["oneOf"]
                .as_array()
                .unwrap();
        assert!(references.iter().any(|value| value["type"] == "integer"));
        assert!(
            references
                .iter()
                .any(|value| { value["$ref"] == "#/$defs/AssistantCadProgramFeatureReference" })
        );
        let local_reference =
            &caps["result"]["cad_program_schema"]["$defs"]["AssistantCadProgramFeatureReference"];
        assert_eq!(local_reference["additionalProperties"], false);
        assert_eq!(
            local_reference["required"],
            json!(["operation_index", "output"])
        );
        let outputs = caps["result"]["cad_program_schema"]["$defs"]
            ["AssistantCadProgramFeatureOutput"]["oneOf"]
            .as_array()
            .unwrap();
        assert_eq!(
            outputs
                .iter()
                .map(|value| value["const"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "definition",
                "sketch_feature",
                "construction_feature",
                "body_feature"
            ]
        );
    }
    #[test]
    fn typed_program_outputs_apply_one_atomic_public_pocket_edit() {
        let mut server = Server::new(SessionSettings::default());
        let before = request(&mut server, "state", json!({}))["result"]["state"].clone();
        let program = json!({"operations":[
            {
                "operation":"create_part",
                "name":"Wall",
                "workplane":{"type":"principal","plane":"xy"},
                "entities":[{"type":"circle","id":1,"center_mm":[0,0],"radius_mm":10}],
                "constraints":[],
                "feature":{"type":"extrusion","distance_mm":20},
                "translation_mm":[0,0,0]
            },
            {
                "operation":"create_program_sketch",
                "definition":{"operation_index":0,"output":"definition"},
                "name":"Opening profile",
                "workplane":{"type":"principal","plane":"xy"},
                "entities":[{"type":"circle","id":1,"center_mm":[0,0],"radius_mm":4}],
                "constraints":[]
            },
            {
                "operation":"append_program_pocket",
                "definition":{"operation_index":0,"output":"definition"},
                "name":"Opening",
                "target_feature":{"operation_index":0,"output":"body_feature"},
                "profile_feature":{"operation_index":1,"output":"sketch_feature"},
                "depth_mm":5
            }
        ]});
        let applied = request(
            &mut server,
            "apply",
            json!({
                "expected_revision": before["revision"],
                "expected_digest": before["canonical_digest"],
                "program": program,
                "selection": []
            }),
        );
        assert!(applied.get("error").is_none(), "{applied}");
        let state = &applied["result"]["state"];
        assert_eq!(state["revision"], before["revision"].as_u64().unwrap() + 1);
        assert_eq!(state["undo_steps"], 1);
        assert_eq!(applied["result"]["created"]["definition_ids"], json!([1]));
        assert_eq!(
            state["features"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|feature| feature["kind"] == "Pocket")
                .count(),
            1
        );

        let undone = request(
            &mut server,
            "undo",
            json!({
                "expected_revision": state["revision"],
                "expected_digest": state["canonical_digest"]
            }),
        );
        assert_eq!(
            undone["result"]["state"]["canonical_digest"],
            before["canonical_digest"]
        );
    }

    #[test]
    fn persistent_history_drives_compact_redo_modified_and_discard_guard() {
        let mut author = Server::new(SessionSettings::default());
        let initial = request(&mut author, "state", json!({}))["result"]["state"].clone();
        let created = request(
            &mut author,
            "apply",
            json!({
                "expected_revision":initial["revision"],
                "expected_digest":initial["canonical_digest"],
                "selection":[],
                "program":{"operations":[{
                    "operation":"create_part",
                    "name":"History authority",
                    "workplane":{"type":"principal","plane":"xy"},
                    "entities":[
                        {"type":"line","id":1,"start_mm":[0,0],"end_mm":[10,0]},
                        {"type":"line","id":2,"start_mm":[10,0],"end_mm":[10,10]},
                        {"type":"line","id":3,"start_mm":[10,10],"end_mm":[0,10]},
                        {"type":"line","id":4,"start_mm":[0,10],"end_mm":[0,0]}
                    ],
                    "constraints":[],
                    "feature":{"type":"extrusion","distance_mm":5},
                    "translation_mm":[0,0,0]
                }]}
            }),
        );
        assert!(created.get("error").is_none(), "{created}");
        let base = created["result"]["state"].clone();
        let grounded = request(
            &mut author,
            "set_grounded",
            json!({
                "expected_revision":base["revision"],
                "expected_digest":base["canonical_digest"],
                "occurrence_ids":[1],
                "grounded":true
            }),
        );
        let grounded_state = grounded["result"]["state"].clone();
        let undone = request(
            &mut author,
            "undo",
            json!({
                "expected_revision":grounded_state["revision"],
                "expected_digest":grounded_state["canonical_digest"]
            }),
        );
        assert_eq!(
            undone["result"]["state"]["canonical_digest"],
            base["canonical_digest"]
        );
        assert_eq!(undone["result"]["state"]["redo_steps"], 1);

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.ketchup");
        let undone_state = undone["result"]["state"].clone();
        let saved = request(
            &mut author,
            "save",
            json!({
                "expected_revision":undone_state["revision"],
                "expected_digest":undone_state["canonical_digest"],
                "path":path,
                "overwrite":false
            }),
        );
        assert_eq!(saved["result"]["modified"], false);
        assert_eq!(saved["result"]["state"]["redo_steps"], 1);

        let mut reopened = Server::new(SessionSettings::default());
        let placeholder = request(&mut reopened, "state", json!({}))["result"]["state"].clone();
        let opened = request(
            &mut reopened,
            "open",
            json!({
                "expected_revision":placeholder["revision"],
                "expected_digest":placeholder["canonical_digest"],
                "path":path,
                "discard_unsaved":false,
                "response":"compact"
            }),
        );
        assert_eq!(opened["result"]["response"], "compact");
        assert_eq!(opened["result"]["state"]["redo_steps"], 1);
        assert_eq!(opened["result"]["modified"], false);
        let opened_state = opened["result"]["state"].clone();

        let redone = request(
            &mut reopened,
            "redo",
            json!({
                "expected_revision":opened_state["revision"],
                "expected_digest":opened_state["canonical_digest"],
                "response":"compact"
            }),
        );
        let redone_state = redone["result"]["state"].clone();
        assert_eq!(redone["result"]["modified"], true);
        assert_eq!(redone_state["redo_steps"], 0);
        let reverted = request(
            &mut reopened,
            "set_grounded",
            json!({
                "expected_revision":redone_state["revision"],
                "expected_digest":redone_state["canonical_digest"],
                "occurrence_ids":[1],
                "grounded":false,
                "response":"compact"
            }),
        );
        let reverted_state = reverted["result"]["state"].clone();
        assert_eq!(
            reverted_state["canonical_digest"],
            opened_state["canonical_digest"]
        );
        assert_eq!(reverted["result"]["modified"], true);

        let refused = request(
            &mut reopened,
            "new",
            json!({
                "expected_revision":reverted_state["revision"],
                "expected_digest":reverted_state["canonical_digest"],
                "discard_unsaved":false,
                "response":"compact"
            }),
        );
        assert_eq!(refused["error"]["code"], "unsaved_changes");
        let after_refusal = request(&mut reopened, "state", json!({}));
        assert_eq!(
            after_refusal["result"]["state"]["revision"],
            reverted_state["revision"]
        );
        assert_eq!(
            after_refusal["result"]["state"]["canonical_digest"],
            reverted_state["canonical_digest"]
        );
    }

    #[test]
    fn oversized_open_response_reports_possible_replacement_and_recovers_compactly() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large-state.ketchup");
        let mut document = ketchup_core::document::DocumentStore::new();
        let mut commands = vec![CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Part".to_owned(),
        }];
        commands.extend((1..=35_000).map(|id| {
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(id),
                definition_id: DefinitionId(1),
                name: "Instance".to_owned(),
                transform: ketchup_core::document::Transform::from_translation(
                    id as f64 + 0.123456789,
                    0.123456789,
                    0.123456789,
                )
                .unwrap(),
                parent: None,
                tag: None,
                visible: true,
            }
        }));
        document.apply_batch(&CommandBatch::new(commands)).unwrap();
        std::fs::write(&path, ketchup_core::persistence::save(&document.current())).unwrap();
        let author = DocumentSession::open(&path, SessionSettings::default()).unwrap();
        let mut expected = Server::new(SessionSettings::default());
        expected.session = author;
        let loaded = request(&mut expected, "summary", json!({}))["result"]["state"].clone();
        let mut server = Server::new(SessionSettings::default());
        let before = request(&mut server, "summary", json!({}))["result"]["state"].clone();
        assert_ne!(before["canonical_digest"], loaded["canonical_digest"]);
        let mut input = Vec::new();
        for (id, method, params) in [
            (
                1,
                "open",
                json!({"path":path,"discard_unsaved":false,
                "expected_revision":before["revision"],"expected_digest":before["canonical_digest"]}),
            ),
            (2, "summary", json!({})),
            (
                3,
                "new",
                json!({"discard_unsaved":false,"response":"compact",
                "expected_revision":before["revision"],"expected_digest":before["canonical_digest"]}),
            ),
            (
                4,
                "open",
                json!({"path":path,"discard_unsaved":false,"response":"compact",
                "expected_revision":loaded["revision"],"expected_digest":loaded["canonical_digest"]}),
            ),
            (5, "summary", json!({})),
        ] {
            serde_json::to_writer(
                &mut input,
                &json!({"protocol":PROTOCOL,"id":id,"method":method,"params":params}),
            )
            .unwrap();
            input.push(b'\n');
        }
        let mut output = Vec::new();
        serve(io::Cursor::new(input), &mut output, server).unwrap();
        let output = String::from_utf8(output).unwrap();
        let responses: Vec<Value> = output
            .lines()
            .map(|line| {
                assert!(line.len() < MAX_LINE_BYTES);
                serde_json::from_str(line).unwrap()
            })
            .collect();
        assert_eq!(responses.len(), 5);
        assert_eq!(responses[0]["error"]["code"], "output_too_large");
        assert_eq!(
            responses[0]["error"]["details"]["mutation_outcome"],
            "possibly_applied"
        );
        assert_eq!(responses[1]["result"]["state"], loaded);
        assert_eq!(responses[2]["error"]["code"], "stale_state");
        assert_eq!(responses[3]["result"]["response"], "compact");
        assert_eq!(responses[3]["result"]["state"], loaded);
        assert_eq!(responses[4]["result"]["state"], loaded);
    }

    #[test]
    fn bounded_lines_resynchronize() {
        let mut input = vec![b'x'; MAX_LINE_BYTES + 1];
        input.extend_from_slice(b"\n{\"protocol\":\"ketchup.headless.v1\",\"id\":9,\"method\":\"state\",\"params\":{}}\n");
        let mut output = Vec::new();
        serve(
            io::Cursor::new(input),
            &mut output,
            Server::new(SessionSettings::default()),
        )
        .unwrap();
        let lines = String::from_utf8(output).unwrap();
        let lines: Vec<_> = lines.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(lines[0]).unwrap()["error"]["code"],
            "line_too_large"
        );
        assert_eq!(serde_json::from_str::<Value>(lines[1]).unwrap()["id"], 9);
    }
}
