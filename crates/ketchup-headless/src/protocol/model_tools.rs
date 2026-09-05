use super::*;
use ketchup_application::model_query::{self, EntityKind, PageRequest};
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
        if matches!(method, "summary" | "query" | "detail") {
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
