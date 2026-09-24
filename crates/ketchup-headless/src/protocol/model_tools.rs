use super::*;
use ketchup_application::model_query::{self, EditContextRequest, EntityKind, PageRequest};
use ketchup_core::assistant_sidecar::AssistantInstancePath;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardedEditContext {
    #[serde(default)]
    expected_document_id: Option<u64>,
    #[serde(default)]
    expected_revision: Option<u64>,
    #[serde(default)]
    expected_digest: Option<String>,
    #[serde(default)]
    expected_mutation_epoch: Option<u64>,
    targets: Vec<AssistantInstancePath>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchJobStart {
    #[serde(default, rename = "expected_revision")]
    _expected_revision: Option<u64>,
    #[serde(default, rename = "expected_digest")]
    _expected_digest: Option<String>,
    #[serde(default, rename = "expected_mutation_epoch")]
    _expected_mutation_epoch: Option<u64>,
    workset_handle: String,
    operation: OccurrenceBatchOperation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchJobHandle {
    handle: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchJobStep {
    #[serde(default, rename = "expected_revision")]
    _expected_revision: Option<u64>,
    #[serde(default, rename = "expected_digest")]
    _expected_digest: Option<String>,
    #[serde(default, rename = "expected_mutation_epoch")]
    _expected_mutation_epoch: Option<u64>,
    handle: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyJobStart {
    #[serde(default, rename = "expected_revision")]
    _expected_revision: Option<u64>,
    #[serde(default, rename = "expected_digest")]
    _expected_digest: Option<String>,
    #[serde(default, rename = "expected_mutation_epoch")]
    _expected_mutation_epoch: Option<u64>,
    scope: Option<Vec<VerifyProducerScope>>,
    #[serde(default = "default_verify_timeout_ms")]
    timeout_ms: u64,
}

#[derive(Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
struct VerifyProducerScope {
    definition_id: u64,
    feature_id: u64,
}

const fn default_verify_timeout_ms() -> u64 {
    30_000
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyJobHandle {
    handle: String,
}

fn batch_error(error: OccurrenceBatchError) -> Error {
    match error {
        OccurrenceBatchError::Query(error) => {
            Error::new(error.code(), format!("batch query rejected: {error:?}"))
        }
        OccurrenceBatchError::Session(error) => error.into(),
        OccurrenceBatchError::HostTransaction => Error::new(
            "batch_transaction_failed",
            "batch host rejected the transaction",
        ),
        OccurrenceBatchError::Cancelled => Error::new("batch_cancelled", "batch job cancelled"),
        OccurrenceBatchError::StaleTask { expected, actual } => {
            let mut error = Error::new(
                "stale_batch_task",
                "batch job no longer matches the document mutation epoch",
            );
            error.details = Some(json!({"expected":expected,"actual":actual}));
            error
        }
    }
}

/// Largest error message sent to a client. Messages carry the cause and the
/// fix (for rule programs: file, line and source excerpt), so they are not cut
/// to the short limit used for names in query results.
const MAX_ERROR_MESSAGE_BYTES: usize = 4 * 1024;
const MAX_ERROR_DETAILS_BYTES: usize = 16 * 1024;

fn truncate(text: &mut String, limit: usize) -> bool {
    if text.len() <= limit {
        return false;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

pub(super) fn bounded_error(mut error: Error) -> Error {
    let original_bytes = error.message.len();
    let message_truncated = truncate(&mut error.message, MAX_ERROR_MESSAGE_BYTES);
    truncate(&mut error.code, model_query::MAX_TEXT_BYTES);
    let oversized = error.details.as_ref().is_some_and(|details| {
        serde_json::to_vec(details).map_or(true, |bytes| bytes.len() > MAX_ERROR_DETAILS_BYTES)
    });
    if oversized {
        error.details = Some(json!({"details_omitted":true}));
    }
    if message_truncated {
        let details = error.details.get_or_insert_with(|| json!({}));
        if let Some(object) = details.as_object_mut() {
            object.insert("message_truncated".to_owned(), json!(true));
            object.insert("original_message_bytes".to_owned(), json!(original_bytes));
        }
    }
    error
}

impl Server {
    pub(super) fn compact_state_result(&self) -> Value {
        let snapshot = self.session.snapshot();
        let mut state = model_query::identity(&snapshot);
        state["mutation_epoch"] = json!(self.session.mutation_epoch());
        state["undo_steps"] = json!(self.session.visible_undo_steps());
        state["redo_steps"] = json!(self.session.visible_redo_steps());
        json!({"state":state,"summary":self.model_queries.summary(&snapshot),
            "path":self.session.path().map(|p| model_query::bounded_text(&p.to_string_lossy())),
            "recovery":self.session.recovery_state().map(|recovery| json!({
                "requested_path":model_query::bounded_text(&recovery.requested_path().to_string_lossy()),
                "source_path":model_query::bounded_text(&recovery.source_path().to_string_lossy()),
                "save_as_required":true
            })),
            "modified":self.session.is_modified(),"response":"compact"})
    }

    pub(super) fn dispatch(&mut self, method: &str, mut params: Value) -> Result<Value> {
        let p = params
            .as_object_mut()
            .ok_or_else(|| Error::invalid("params must be an object"))?;
        if method_requires_guard(method) {
            self.guard(p)?;
        }
        if matches!(
            method,
            "verify_job_start" | "verify_job_status" | "verify_job_cancel"
        ) {
            let result = match method {
                "verify_job_start" => {
                    let request: VerifyJobStart = serde_json::from_value(Value::Object(p.clone()))
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    if !(1..=300_000).contains(&request.timeout_ms) {
                        return Err(Error::invalid("timeout_ms must be in [1, 300000]"));
                    }
                    let scope = request
                        .scope
                        .map(|scope| {
                            if scope.is_empty() || scope.len() > 100 {
                                return Err(Error::invalid(
                                    "scope must contain 1..100 producer keys",
                                ));
                            }
                            if scope
                                .iter()
                                .any(|key| key.definition_id == 0 || key.feature_id == 0)
                            {
                                return Err(Error::invalid("scope IDs must be positive integers"));
                            }
                            let unique = scope.iter().copied().collect::<BTreeSet<_>>();
                            if unique.len() != scope.len() {
                                return Err(Error::invalid(
                                    "scope contains duplicate producer keys",
                                ));
                            }
                            Ok(unique
                                .into_iter()
                                .map(|key| ProducerKey {
                                    definition_id: DefinitionId(key.definition_id),
                                    feature_id: FeatureId(key.feature_id),
                                })
                                .collect::<BTreeSet<_>>())
                        })
                        .transpose()?;
                    self.start_verify_job(scope, request.timeout_ms)?
                }
                "verify_job_status" => {
                    let request: VerifyJobHandle = serde_json::from_value(Value::Object(p.clone()))
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    let index = self
                        .verify_jobs
                        .iter()
                        .position(|job| job.handle == request.handle)
                        .ok_or_else(|| {
                            Error::new("verify_job_not_found", "Verify job not found")
                        })?;
                    self.refresh_verify_job(index);
                    self.verify_job_value(index)
                }
                "verify_job_cancel" => {
                    let request: VerifyJobHandle = serde_json::from_value(Value::Object(p.clone()))
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    let index = self
                        .verify_jobs
                        .iter()
                        .position(|job| job.handle == request.handle)
                        .ok_or_else(|| {
                            Error::new("verify_job_not_found", "Verify job not found")
                        })?;
                    let progress = match &self.verify_jobs[index].state {
                        VerifyJobState::Running(task) => {
                            task.cancel();
                            Some(task.progress())
                        }
                        _ => None,
                    };
                    if let Some(progress) = progress {
                        self.verify_jobs[index].progress = progress;
                        self.verify_jobs[index].elapsed_ms = self.verify_jobs[index]
                            .started_at
                            .elapsed()
                            .as_millis()
                            .min(u128::from(u64::MAX))
                            as u64;
                        self.verify_jobs[index].state = VerifyJobState::Cancelled;
                        if let Some(stop) = self.verify_jobs[index].deadline_stop.take() {
                            let _ = stop.send(());
                        }
                    }
                    self.verify_job_value(index)
                }
                _ => unreachable!(),
            };
            return Ok(result);
        }
        if matches!(
            method,
            "batch_job_start" | "batch_job_status" | "batch_job_step" | "batch_job_cancel"
        ) {
            let result = match method {
                "batch_job_start" => {
                    let request: BatchJobStart = serde_json::from_value(Value::Object(p.clone()))
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    let task = self
                        .model_queries
                        .create_occurrence_batch_task(
                            &self.session,
                            &request.workset_handle,
                            request.operation,
                        )
                        .map_err(|e| {
                            Error::new(e.code(), format!("batch workset rejected: {e:?}"))
                        })?;
                    if self.batch_jobs.len() == MAX_BATCH_JOBS {
                        let terminal = self
                            .batch_jobs
                            .iter()
                            .position(|job| {
                                matches!(
                                    job.task.status(&self.session).state,
                                    OccurrenceBatchState::Completed
                                        | OccurrenceBatchState::Cancelled
                                        | OccurrenceBatchState::Stale
                                )
                            })
                            .ok_or_else(|| {
                                Error::new(
                                    "batch_job_limit",
                                    "all bounded batch job slots are active",
                                )
                            })?;
                        self.batch_jobs.remove(terminal);
                    }
                    let id = self.next_batch_job;
                    self.next_batch_job = id.checked_add(1).ok_or_else(|| {
                        Error::new("batch_job_ids_exhausted", "batch job IDs exhausted")
                    })?;
                    let handle = self.batch_job_handle(id);
                    let status = task.status(&self.session);
                    self.batch_jobs.push_back(BatchJob {
                        handle: handle.clone(),
                        task,
                    });
                    json!({"job_handle":handle,"status":status})
                }
                "batch_job_status" => {
                    let request: BatchJobHandle = serde_json::from_value(Value::Object(p.clone()))
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    let job = self
                        .batch_jobs
                        .iter()
                        .find(|job| job.handle == request.handle)
                        .ok_or_else(|| Error::new("batch_job_not_found", "batch job not found"))?;
                    json!({"job_handle":job.handle,"status":job.task.status(&self.session)})
                }
                "batch_job_cancel" => {
                    let request: BatchJobHandle = serde_json::from_value(Value::Object(p.clone()))
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    let job = self
                        .batch_jobs
                        .iter_mut()
                        .find(|job| job.handle == request.handle)
                        .ok_or_else(|| Error::new("batch_job_not_found", "batch job not found"))?;
                    job.task.cancel();
                    json!({"job_handle":job.handle,"status":job.task.status(&self.session)})
                }
                "batch_job_step" => {
                    let request: BatchJobStep = serde_json::from_value(Value::Object(p.clone()))
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    let index = self
                        .batch_jobs
                        .iter()
                        .position(|job| job.handle == request.handle)
                        .ok_or_else(|| Error::new("batch_job_not_found", "batch job not found"))?;
                    let receipt = self.batch_jobs[index]
                        .task
                        .commit_next(&mut self.session)
                        .map_err(batch_error)?;
                    if receipt.is_some() {
                        self.initial_placeholder = false;
                        self.model_queries.invalidate();
                    }
                    let status = self.batch_jobs[index].task.status(&self.session);
                    json!({"job_handle":request.handle,"status":status,"receipt":receipt})
                }
                _ => unreachable!(),
            };
            return Ok(result);
        }
        if matches!(
            method,
            "summary" | "edit_context" | "query" | "detail" | "workset_create" | "workset_status"
        ) {
            let snapshot = self.session.snapshot();
            let result = match method {
                "summary" => {
                    if !p.is_empty() {
                        return Err(Error::invalid("summary takes no parameters"));
                    }
                    return Ok(self.compact_state_result());
                }
                "edit_context" => {
                    let request: GuardedEditContext = serde_json::from_value(params)
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    let actual_epoch = self.session.mutation_epoch();
                    if request
                        .expected_document_id
                        .is_some_and(|id| id != snapshot.document_id().0)
                        || request
                            .expected_revision
                            .is_some_and(|revision| revision != snapshot.revision_id())
                        || request
                            .expected_digest
                            .is_some_and(|digest| digest != snapshot.canonical_digest())
                        || request
                            .expected_mutation_epoch
                            .is_some_and(|epoch| epoch != actual_epoch)
                    {
                        return Err(Error {
                            code: "stale_state".into(),
                            message: "expected document stamp does not match observed document"
                                .into(),
                            details: Some(json!({"document_id": snapshot.document_id().0,
                                "revision": snapshot.revision_id(),
                                "canonical_digest": snapshot.canonical_digest(),
                                "mutation_epoch": actual_epoch})),
                        });
                    }
                    let query = EditContextRequest {
                        targets: request.targets,
                    };
                    let context = self
                        .model_queries
                        .edit_context(
                            &snapshot,
                            self.session.topology_results(),
                            actual_epoch,
                            &query,
                        )
                        .map_err(|error| {
                            Error::new(error.code(), format!("model query rejected: {error:?}"))
                        })?;
                    let needs_exact = context["targets"].as_array().is_some_and(|targets| {
                        targets.iter().any(|target| {
                            target["stable_faces"]["status"] == "unsupported"
                                && target["features"]
                                    .as_array()
                                    .is_some_and(|features| !features.is_empty())
                        })
                    });
                    if !needs_exact {
                        Ok(context)
                    } else {
                        self.session.evaluate()?;
                        self.model_queries.edit_context(
                            &self.session.snapshot(),
                            self.session.topology_results(),
                            actual_epoch,
                            &query,
                        )
                    }
                }
                "query" => {
                    let request: PageRequest = serde_json::from_value(params)
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    self.model_queries.page_with_topology(
                        &snapshot,
                        self.session.topology_results(),
                        &request,
                    )
                }
                "detail" => {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Detail {
                        kind: EntityKind,
                        id: u64,
                    }
                    let request: Detail = serde_json::from_value(params)
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    self.model_queries.detail_with_topology(
                        &snapshot,
                        self.session.topology_results(),
                        request.kind,
                        request.id,
                    )
                }
                "workset_create" => {
                    let request: PageRequest = serde_json::from_value(params)
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    self.model_queries.create_workset(&snapshot, &request)
                }
                "workset_status" => {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct WorksetStatus {
                        handle: String,
                    }
                    let request: WorksetStatus = serde_json::from_value(params)
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    self.model_queries
                        .workset_status(&snapshot, &request.handle)
                }
                _ => unreachable!(),
            };
            return result
                .map_err(|e| Error::new(e.code(), format!("model query rejected: {e:?}")));
        }
        let mutation = matches!(
            method,
            "new" | "open" | "apply" | "program_apply" | "set_grounded" | "undo" | "redo" | "save"
        );
        self.compact_result = false;
        if mutation && let Some(response) = p.remove("response") {
            if response != "compact" {
                return Err(Error::invalid("response must be compact when supplied"));
            }
            self.compact_result = true;
        }
        let result = self.dispatch_inner(method, params);
        if result.is_ok() && mutation && method != "save" {
            self.model_queries.invalidate();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(server: &mut Server, method: &str, mut params: Value) -> Value {
        if params.get("expected_revision").is_some()
            && params.get("expected_mutation_epoch").is_none()
        {
            params["expected_mutation_epoch"] = json!(server.session.mutation_epoch());
        }
        server.handle(
            serde_json::to_vec(
                &json!({"protocol":PROTOCOL,"id":1,"method":method,"params":params}),
            )
            .unwrap()
            .as_slice(),
        )
    }
    fn guard(result: &Value) -> Value {
        json!({"expected_revision":result["result"]["state"]["revision"],
            "expected_digest":result["result"]["state"]["canonical_digest"],
            "expected_mutation_epoch":result["result"]["state"]["mutation_epoch"],"response":"compact"})
    }

    #[test]
    fn edit_context_is_guarded_and_read_only_for_duplicate_names() {
        use ketchup_core::document::{DefinitionId, Transform};

        let mut server = Server::new(SessionSettings::default());
        let seed = server
            .session
            .plan_commands(CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Shared panel".into(),
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(1),
                    definition_id: DefinitionId(1),
                    name: "Panel".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(1),
                    name: "Panel".into(),
                    transform: Transform::from_translation(50.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        server.session.apply_proposal(&seed).unwrap();
        let snapshot = server.session.snapshot();
        let stamp = json!({"expected_document_id": snapshot.document_id().0,
            "expected_revision": snapshot.revision_id(),
            "expected_digest": snapshot.canonical_digest(),
            "expected_mutation_epoch": server.session.mutation_epoch()});
        let modified = server.session.is_modified();
        let undo_steps = server.session.visible_undo_steps();

        let mut params = stamp.clone();
        params["targets"] = json!([
            {"root_occurrence_id": 1, "steps": []},
            {"root_occurrence_id": 2, "steps": []}
        ]);
        let result = call(&mut server, "edit_context", params);
        assert_eq!(result["result"]["targets"].as_array().unwrap().len(), 2);
        assert_ne!(
            result["result"]["targets"][0]["world_transform"],
            result["result"]["targets"][1]["world_transform"]
        );
        assert_eq!(
            server.session.snapshot().canonical_digest(),
            snapshot.canonical_digest()
        );
        assert_eq!(server.session.is_modified(), modified);
        assert_eq!(server.session.visible_undo_steps(), undo_steps);

        let mut stale = stamp;
        stale["expected_revision"] = json!(snapshot.revision_id() + 1);
        stale["targets"] = json!([{"root_occurrence_id": 1, "steps": []}]);
        assert_eq!(
            call(&mut server, "edit_context", stale)["error"]["code"],
            "stale_state"
        );
    }

    #[test]
    fn one_edit_context_call_evaluates_v9_and_returns_both_target_faces_read_only() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../ketchup-application/tests/fixtures/fast_assembly/nightstand_v9_retention.ketchup",
        );
        let mut server = Server::new(SessionSettings::default());
        server.session = DocumentSession::open(path, server.settings.clone()).unwrap();
        assert!(server.session.topology_results().is_empty());
        let snapshot = server.session.snapshot();
        let before = (
            snapshot.revision_id(),
            snapshot.canonical_digest(),
            server.session.mutation_epoch(),
            server.session.visible_undo_steps(),
            server.session.is_modified(),
        );
        let result = call(
            &mut server,
            "edit_context",
            json!({
                "expected_document_id": snapshot.document_id().0,
                "expected_revision": snapshot.revision_id(),
                "expected_digest": snapshot.canonical_digest(),
                "expected_mutation_epoch": before.2,
                "targets": [
                    {"root_occurrence_id": 6, "steps": []},
                    {"root_occurrence_id": 1, "steps": []}
                ]
            }),
        );

        let targets = result["result"]["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0]["instance"]["id"], "root:6");
        assert_eq!(targets[1]["instance"]["id"], "root:1");
        assert!(targets.iter().all(|target| {
            target["stable_faces"]["status"] == "supported"
                && target["stable_faces"]["complete"] == true
                && target["stable_faces"]["items"]
                    .as_array()
                    .is_some_and(|faces| !faces.is_empty())
        }));
        assert!(
            targets[1]["stable_faces"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|face| face["semantic_role"] == "extrusion.bottom"
                    && face["reference_id"].is_string())
        );
        assert_eq!(
            (
                server.session.snapshot().revision_id(),
                server.session.snapshot().canonical_digest(),
                server.session.mutation_epoch(),
                server.session.visible_undo_steps(),
                server.session.is_modified(),
            ),
            before
        );
    }

    #[test]
    fn batch_jobs_are_cancellable_bounded_and_publish_compact_verified_receipts() {
        use ketchup_core::document::{DefinitionId, Transform};

        let mut server = Server::new(SessionSettings::default());
        let seed = server
            .session
            .plan_commands(CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Batch part".into(),
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(1),
                    definition_id: DefinitionId(1),
                    name: "one".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(1),
                    name: "two".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        server.session.apply_proposal(&seed).unwrap();
        let workset = call(
            &mut server,
            "workset_create",
            json!({"kind":"occurrences","limit":10}),
        );
        let workset_handle = workset["result"]["workset_handle"]
            .as_str()
            .unwrap()
            .to_owned();
        let snapshot = server.session.snapshot();
        let guarded = |handle: &str| {
            json!({"expected_revision":snapshot.revision_id(),
                "expected_digest":snapshot.canonical_digest(),"handle":handle})
        };
        let start = |server: &mut Server| {
            call(
                server,
                "batch_job_start",
                json!({"expected_revision":snapshot.revision_id(),
                    "expected_digest":snapshot.canonical_digest(),
                    "workset_handle":workset_handle,
                    "operation":{"type":"set_color","color":[10,20,30]}}),
            )
        };

        let cancelled = start(&mut server);
        let cancelled_handle = cancelled["result"]["job_handle"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(cancelled["result"]["status"]["state"], "pending");
        let cancelled = call(
            &mut server,
            "batch_job_cancel",
            json!({"handle":cancelled_handle}),
        );
        assert_eq!(cancelled["result"]["status"]["state"], "cancelled");
        for _ in 1..MAX_BATCH_JOBS {
            assert_eq!(start(&mut server)["result"]["status"]["state"], "pending");
        }
        let rejected = call(
            &mut server,
            "batch_job_start",
            json!({"expected_revision":snapshot.revision_id(),
                "expected_digest":snapshot.canonical_digest(),
                "workset_handle":"forged",
                "operation":{"type":"set_color","color":[10,20,30]}}),
        );
        assert_eq!(rejected["error"]["code"], "workset_not_found");
        assert_eq!(server.batch_jobs.len(), MAX_BATCH_JOBS);
        assert_eq!(
            call(
                &mut server,
                "batch_job_status",
                json!({"handle":cancelled_handle})
            )["result"]["status"]["state"],
            "cancelled"
        );
        let undo_before = server.session.visible_undo_steps();
        assert_eq!(
            call(&mut server, "batch_job_step", guarded(&cancelled_handle))["error"]["code"],
            "batch_cancelled"
        );
        assert_eq!(server.session.visible_undo_steps(), undo_before);
        assert_eq!(
            server
                .session
                .snapshot()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .color(),
            None
        );

        let running = start(&mut server);
        let running_handle = running["result"]["job_handle"].as_str().unwrap().to_owned();
        assert_ne!(running_handle, cancelled_handle);
        let stepped = call(&mut server, "batch_job_step", guarded(&running_handle));
        assert_eq!(stepped["result"]["status"]["state"], "completed");
        assert_eq!(stepped["result"]["receipt"]["applied_count"], 2);
        assert_eq!(stepped["result"]["receipt"]["verified_write_count"], 2);
        assert_eq!(server.session.visible_undo_steps(), undo_before + 1);
        let encoded = serde_json::to_vec(&stepped).unwrap();
        assert!(encoded.len() < 4096);
        assert!(
            !String::from_utf8(encoded)
                .unwrap()
                .contains("occurrence_ids")
        );
        assert_eq!(
            call(
                &mut server,
                "batch_job_status",
                json!({"handle":running_handle})
            )["result"]["status"]["state"],
            "completed"
        );
        assert_eq!(
            call(&mut server, "batch_job_status", json!({"handle":"forged"}))["error"]["code"],
            "batch_job_not_found"
        );
    }

    #[test]
    fn compact_transport_summary_mutations_and_query_errors() {
        let mut server = Server::new(SessionSettings::default());
        let initial = call(&mut server, "summary", json!({}));
        let fresh = call(&mut server, "new", guard(&initial));
        assert_eq!(fresh["result"]["response"], "compact");
        assert!(fresh["result"]["state"].get("occurrences").is_none());
        assert!(fresh["result"]["summary"]["counts"].is_object());
        assert!(serde_json::to_vec(&fresh).unwrap().len() < 8192);
        let page = call(
            &mut server,
            "query",
            json!({"kind":"occurrences","limit":10}),
        );
        assert_eq!(page["result"]["total_matches"], 0);
        assert_eq!(page["result"]["complete"], true);
        let relations = call(
            &mut server,
            "query",
            json!({"kind":"relations","limit":10,"definition_id":1}),
        );
        assert_eq!(relations["result"]["total_matches"], 0);
        assert_eq!(relations["result"]["total_matches_complete"], true);
        let workset = call(
            &mut server,
            "workset_create",
            json!({"kind":"occurrences","limit":10}),
        );
        assert_eq!(workset["result"]["item_count"], 0);
        assert_eq!(workset["result"]["completeness"]["usable_for_batch"], true);
        let handle = workset["result"]["workset_handle"].as_str().unwrap();
        assert_eq!(
            call(&mut server, "workset_status", json!({"handle":handle}))["result"],
            workset["result"]
        );
        assert_eq!(
            call(&mut server, "workset_status", json!({"handle":"forged"}))["error"]["code"],
            "workset_not_found"
        );
        assert_eq!(
            call(
                &mut server,
                "query",
                json!({"kind":"occurrences","limit":101})
            )["error"]["code"],
            "invalid_params"
        );
        assert_eq!(
            call(&mut server, "detail", json!({"kind":"occurrences","id":1}))["error"]["code"],
            "entity_not_found"
        );
        assert_eq!(
            call(&mut server, "summary", json!({"extra":1}))["error"]["code"],
            "invalid_params"
        );
        assert!(call(&mut server, "state", json!({}))["result"]["state"]["occurrences"].is_array());
        assert_eq!(
            call(&mut server, "new", guard(&initial))["error"]["code"],
            "stale_state"
        );
    }
}
