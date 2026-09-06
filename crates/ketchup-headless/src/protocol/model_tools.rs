use super::*;
use ketchup_application::model_query::{self, EntityKind, PageRequest};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchJobStart {
    #[serde(rename = "expected_revision")]
    _expected_revision: u64,
    #[serde(rename = "expected_digest")]
    _expected_digest: String,
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
    #[serde(rename = "expected_revision")]
    _expected_revision: u64,
    #[serde(rename = "expected_digest")]
    _expected_digest: String,
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

pub(super) fn bounded_error(mut error: Error) -> Error {
    let message = model_query::bounded_text(&error.message);
    let code = model_query::bounded_text(&error.code);
    let oversized = error
        .details
        .as_ref()
        .is_some_and(|d| serde_json::to_vec(d).map_or(true, |v| v.len() > 16 * 1024));
    if message["truncated"] == true || code["truncated"] == true || oversized {
        error.message = message["text"]
            .as_str()
            .unwrap_or("diagnostic omitted")
            .to_owned();
        error.code = code["text"].as_str().unwrap_or("error").to_owned();
        error.details = Some(
            json!({"diagnostic_truncated":true,"original_message_bytes":message["original_bytes"]}),
        );
    }
    error
}

impl Server {
    pub(super) fn compact_state_result(&self) -> Value {
        let snapshot = self.session.snapshot();
        let mut state = model_query::identity(&snapshot);
        state["undo_steps"] = json!(self.session.visible_undo_steps());
        state["redo_steps"] = json!(self.redo_steps);
        json!({"state":state,"summary":self.model_queries.summary(&snapshot),
            "path":self.session.path().map(|p| model_query::bounded_text(&p.to_string_lossy())),
            "modified":!self.pristine && self.session.is_modified(),"response":"compact"})
    }

    pub(super) fn dispatch(&mut self, method: &str, mut params: Value) -> Result<Value> {
        let p = params
            .as_object_mut()
            .ok_or_else(|| Error::invalid("params must be an object"))?;
        if matches!(
            method,
            "batch_job_start" | "batch_job_status" | "batch_job_step" | "batch_job_cancel"
        ) {
            let result = match method {
                "batch_job_start" => {
                    let request: BatchJobStart = serde_json::from_value(Value::Object(p.clone()))
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    self.guard(p)?;
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
                    self.guard(p)?;
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
                        self.redo_steps = 0;
                        self.pristine = false;
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
            "summary" | "query" | "detail" | "workset_create" | "workset_status"
        ) {
            let snapshot = self.session.snapshot();
            let result = match method {
                "summary" => {
                    if !p.is_empty() {
                        return Err(Error::invalid("summary takes no parameters"));
                    }
                    return Ok(self.compact_state_result());
                }
                "query" => {
                    let request: PageRequest = serde_json::from_value(params)
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    self.model_queries.page(&snapshot, &request)
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
                    self.model_queries
                        .detail(&snapshot, request.kind, request.id)
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
            "new" | "open" | "apply" | "set_grounded" | "undo" | "redo" | "save"
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

    fn call(server: &mut Server, method: &str, params: Value) -> Value {
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
            "expected_digest":result["result"]["state"]["canonical_digest"],"response":"compact"})
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
