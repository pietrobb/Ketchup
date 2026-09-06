use ketchup_application::evaluation::EvidenceStatus;
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
use ketchup_core::document::{CanonicalCommand, CommandBatch, OccurrenceId, Snapshot};
use ketchup_core::exact_product::{ExactBodyPackage, ExactResultRegistry};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeSet, VecDeque, hash_map::RandomState},
    hash::{BuildHasher, Hash, Hasher},
    io::{self, BufRead, Write},
};

pub const PROTOCOL: &str = "ketchup.headless.v1";
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_BATCH_JOBS: usize = 16;
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

pub struct Server {
    session: DocumentSession,
    settings: SessionSettings,
    // Open creates a fresh history cursor in the application persistence API.
    redo_steps: usize,
    pristine: bool,
    model_queries: ModelQuery,
    batch_jobs: VecDeque<BatchJob>,
    batch_job_key: RandomState,
    next_batch_job: u64,
    compact_result: bool,
}
impl Server {
    pub fn new(settings: SessionSettings) -> Self {
        Self {
            session: DocumentSession::new(settings.clone()),
            settings,
            redo_steps: 0,
            pristine: true,
            model_queries: ModelQuery::default(),
            batch_jobs: VecDeque::new(),
            batch_job_key: RandomState::new(),
            next_batch_job: 1,
            compact_result: false,
        }
    }
    fn batch_job_handle(&self, id: u64) -> String {
        let mut hasher = self.batch_job_key.build_hasher();
        id.hash(&mut hasher);
        format!("batch-{id:016x}-{:016x}", hasher.finish())
    }

    fn revoke_batch_jobs(&mut self) {
        self.batch_jobs.clear();
        self.batch_job_key = RandomState::new();
        self.next_batch_job = 1;
    }

    fn state(&self) -> Value {
        let s = self.session.snapshot();
        json!({"document_id":s.document_id().0,"revision":s.revision_id(),"canonical_digest":s.canonical_digest(),
            "undo_steps":self.session.visible_undo_steps(),"redo_steps":self.redo_steps,
            "definitions":s.definitions().map(|d| json!({"id":d.id().0,"name":d.name(),"feature_ids":d.feature_ids().iter().map(|id|id.0).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "occurrences":s.occurrences().map(|o| json!({"id":o.id().0,"definition_id":o.definition_id().0,"name":o.name(),"transform":o.transform().matrix(),"color":o.color()})).collect::<Vec<_>>(),
            "features":s.features().map(|f| json!({"id":f.id().0,"definition_id":f.definition_id().0,"name":f.name(),"kind":format!("{:?}",f.kind()).split([' ', '{', '(']).next().unwrap_or("unknown")})).collect::<Vec<_>>(),
            "grounded_occurrence_ids":s.grounded_occurrences().map(|id|id.0).collect::<Vec<_>>()})
    }
    fn state_result(&self) -> Value {
        if self.compact_result {
            return self.compact_state_result();
        }
        json!({"state":self.state(),"path":self.session.path().map(|p|p.to_string_lossy()),"modified":!self.pristine && self.session.is_modified()})
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
        if !boolean(p, "discard_unsaved", false)? && !self.pristine && self.session.is_modified() {
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
                "bounds":{"max_line_bytes":MAX_LINE_BYTES,"max_output_bytes":MAX_LINE_BYTES,"max_selection":100,"max_operations":64,"max_batch_jobs":MAX_BATCH_JOBS,"evaluation_timeout_ms":{"default":30000,"min":1,"max":300000}},
                "mutation_preconditions":["expected_revision","expected_digest"],"units":"mm","transform":"row-major 4x4 local occurrence transform","transactions":"one apply = one atomic CAD program; no within-program references to newly allocated IDs","protocol":PROTOCOL}),
            ),
            "state" => Ok(self.state_result()),
            "new" => {
                self.discard_guard(p)?;
                self.session = DocumentSession::new(self.settings.clone());
                self.revoke_batch_jobs();
                self.redo_steps = 0;
                self.pristine = true;
                Ok(self.state_result())
            }
            "open" => {
                self.discard_guard(p)?;
                let next = DocumentSession::open(string(p, "path")?, self.settings.clone())?;
                self.session = next;
                self.revoke_batch_jobs();
                self.redo_steps = 0;
                self.pristine = false;
                Ok(self.state_result())
            }
            "save" => {
                self.session.save(
                    string(p, "path")?,
                    SaveOptions {
                        overwrite: boolean(p, "overwrite", false)?,
                    },
                )?;
                self.pristine = false;
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
                self.redo_steps = 0;
                self.pristine = false;
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
                self.redo_steps = 0;
                self.pristine = false;
                Ok(self.state_result())
            }
            "undo" => {
                self.session.undo()?;
                self.redo_steps += 1;
                self.pristine = false;
                Ok(self.state_result())
            }
            "redo" => {
                self.session.redo()?;
                self.redo_steps = self.redo_steps.saturating_sub(1);
                self.pristine = false;
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
                let snapshot = self.session.snapshot();
                Ok(
                    json!({"document_id":report.source.0.0,"revision":report.source.1,"canonical_digest":report.source.2,
                    "complete":report.complete,"topology_complete":report.topology_complete,"not_evaluated":report.not_evaluated,
                    "producers":report.producers.iter().map(|p|json!({"definition_id":p.key.definition_id.0,"feature_id":p.key.feature_id.0,"render":status(&p.render),"topology":status(&p.topology)})).collect::<Vec<_>>(),
                    "geometry":geometry(self.session.exact_results(),&snapshot),"topology_geometry":geometry(self.session.topology_results(),&snapshot)}),
                )
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
fn geometry(registry: &ExactResultRegistry, snapshot: &Snapshot) -> Vec<Value> {
    registry.values().filter(|p|p.is_current(snapshot)).map(|p| {
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
    fn schema_covers_current_all_operation_variants() {
        let mut s = Server::new(SessionSettings::default());
        let caps = request(&mut s, "capabilities", json!({}));
        let variants =
            caps["result"]["cad_program_schema"]["$defs"]["AssistantCadEditOperation"]["oneOf"]
                .as_array()
                .unwrap();
        assert_eq!(variants.len(), 10);
        assert!(
            variants
                .iter()
                .any(|v| v["properties"]["operation"]["const"] == "append_feature")
        );
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
