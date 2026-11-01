//! MCP over stdio: one JSON-RPC 2.0 message per line in each direction.
use crate::{discovery, schema, tools::Tools};
use serde_json::{Map, Value, json};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
};

/// Protocol versions this server speaks, newest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// The longest message read: a whole program of about 8 MB, JSON-escaped.
const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;

/// Serves MCP until stdin closes. `app_executable` starts new windows for `open_window`.
pub fn serve_stdio(app_executable: Option<PathBuf>) -> io::Result<()> {
    let mut tools = Tools::new(app_executable, discovery::default_root());
    serve(io::stdin().lock(), io::stdout().lock(), |message| {
        handle(&mut tools, message)
    })
}

/// Answers each line of `input` on `output` until `input` ends. A line that is
/// no JSON, or longer than [`MAX_LINE_BYTES`], gets an error reply and the
/// server goes on with the next (review 2026-10-31 P3).
fn serve(
    mut input: impl BufRead,
    mut output: impl Write,
    mut handle: impl FnMut(Value) -> Option<Value>,
) -> io::Result<()> {
    while let Some(line) = read_line(&mut input)? {
        let reply = match line {
            None => Some(failure(
                Value::Null,
                -32600,
                &format!("a message is at most {} MiB", MAX_LINE_BYTES >> 20),
            )),
            Some(bytes) if bytes.trim_ascii().is_empty() => continue,
            Some(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(message) => handle(message),
                Err(error) => Some(failure(Value::Null, -32700, &error.to_string())),
            },
        };
        if let Some(reply) = reply {
            serde_json::to_writer(&mut output, &reply)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// The next line without its end: `None` when input has ended, `Some(None)`
/// for a line longer than [`MAX_LINE_BYTES`], which is read past and dropped.
fn read_line(input: &mut impl BufRead) -> io::Result<Option<Option<Vec<u8>>>> {
    let mut line = Vec::new();
    let mut oversized = false;
    let mut read = false;
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            break;
        }
        read = true;
        let end = buffer.iter().position(|byte| *byte == b'\n');
        let take = end.map_or(buffer.len(), |index| index + 1);
        let content = end.unwrap_or(buffer.len());
        if !oversized && line.len() + content <= MAX_LINE_BYTES {
            line.extend_from_slice(&buffer[..content]);
        } else {
            oversized = true;
            line = Vec::new();
        }
        input.consume(take);
        if end.is_some() {
            break;
        }
    }
    Ok(read.then_some((!oversized).then_some(line)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn replies(input: &[u8]) -> Vec<Value> {
        let mut output = Vec::new();
        serve(input, &mut output, |message| {
            let id = message.get("id").cloned()?;
            Some(json!({"jsonrpc": "2.0", "id": id, "result": {}}))
        })
        .expect("served to the end");
        output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).expect("a JSON reply"))
            .collect()
    }

    /// Review 2026-10-31 P3: one byte that is not UTF-8 ended the server, and a
    /// line was read whole however long it grew. Both now get an error reply
    /// and the next message is served.
    #[test]
    fn a_bad_or_endless_line_is_answered_and_the_next_one_served() {
        let mut input =
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\",\"p\":\"\xff\"}\n".to_vec();
        input.extend(vec![b' '; MAX_LINE_BYTES + 1]);
        input.extend(b"\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n");
        let replies = replies(&input);
        assert_eq!(replies.len(), 3, "{replies:?}");
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(replies[1]["error"]["code"], -32600);
        assert_eq!(replies[2]["id"], 2);
    }
}
