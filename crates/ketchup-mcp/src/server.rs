//! MCP over stdio: one JSON-RPC 2.0 message per line in each direction.
use crate::{discovery, schema, tools::Tools};
use serde_json::{Map, Value, json};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
};

/// Protocol versions this server speaks, newest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Serves MCP until stdin closes. `app_executable` starts new windows for `open_window`.
pub fn serve_stdio(app_executable: Option<PathBuf>) -> io::Result<()> {
    let mut tools = Tools::new(app_executable, discovery::default_root());
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&mut tools, message),
            Err(error) => Some(failure(Value::Null, -32700, &error.to_string())),
        };
        if let Some(reply) = reply {
            serde_json::to_writer(&mut stdout, &reply)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// The reply to one message; notifications get none.
pub(crate) fn handle(tools: &mut Tools, message: Value) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Some(failure(id, -32600, "a request needs a method"));
    };
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => initialize(&params),
        "ping" => json!({}),
        "tools/list" => json!({"tools": schema::tools()}),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(failure(id, -32602, "tools/call needs a tool name"));
            };
            let arguments = match params.get("arguments") {
                None | Some(Value::Null) => Map::new(),
                Some(Value::Object(arguments)) => arguments.clone(),
                Some(_) => return Some(failure(id, -32602, "arguments must be an object")),
            };
            let output = tools.call(name, arguments);
            json!({"content": output.content, "isError": output.is_error})
        }
        _ => return Some(failure(id, -32601, &format!("method not found: {method}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested
        .filter(|version| PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "ketchup", "title": "Kečup", "version": env!("CARGO_PKG_VERSION")},
        "instructions": schema::INSTRUCTIONS,
    })
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}
