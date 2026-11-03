//! MCP over stdio: one JSON-RPC 2.0 message per line in each direction.
use crate::{
    bridge::Cancel,
    discovery, schema,
    tools::{ToolOutput, Tools},
};
use serde_json::{Map, Value, json};
use std::{
    borrow::{Borrow, BorrowMut},
    collections::HashMap,
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    thread,
};

/// Protocol versions this server speaks, newest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// The longest message read: a whole program of about 8 MB, JSON-escaped.
const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;

/// Serves MCP until stdin closes. `app_executable` starts new windows for `open_window`.
pub fn serve_stdio(app_executable: Option<PathBuf>) -> io::Result<()> {
    let session = Session::new(Tools::new(app_executable, discovery::default_root()));
    serve(io::stdin().lock(), io::stdout(), move |message| {
        handle(&session, message)
    })
}

/// Answers each line of `input` on `output` until `input` ends, each message
/// on its own thread, so a ping or tools/list is answered while a tool call
/// waits for the window (MECH-1). A line that is no JSON, or longer than
/// [`MAX_LINE_BYTES`], gets an error reply and the server goes on with the
/// next (review 2026-10-31 P3). Requests still running when `input` ends are
/// answered before this returns.
fn serve(
    mut input: impl BufRead,
    output: impl Write + Send + 'static,
    handle: impl Fn(Value) -> Option<Value> + Send + Sync + 'static,
) -> io::Result<()> {
    let output = Arc::new(Mutex::new(output));
    let handle = Arc::new(handle);
    let mut running = Vec::new();
    while let Some(line) = read_line(&mut input)? {
        let message = match line {
            None => Err(failure(
                Value::Null,
                -32600,
                &format!("a message is at most {} MiB", MAX_LINE_BYTES >> 20),
            )),
            Some(bytes) if bytes.trim_ascii().is_empty() => continue,
            Some(bytes) => serde_json::from_slice::<Value>(&bytes)
                .map_err(|error| failure(Value::Null, -32700, &error.to_string())),
        };
        match message {
            Ok(message) => {
                let (output, handle) = (Arc::clone(&output), Arc::clone(&handle));
                running.push(thread::spawn(move || match handle(message) {
                    Some(reply) => write_reply(&output, &reply),
                    None => Ok(()),
                }));
            }
            Err(reply) => write_reply(&output, &reply)?,
        }
        finish(&mut running, false)?;
    }
    finish(&mut running, true)
}

/// Joins the request threads that have ended, or all of them with `wait`,
/// and returns the first failure to write a reply.
fn finish(running: &mut Vec<thread::JoinHandle<io::Result<()>>>, wait: bool) -> io::Result<()> {
    let mut first_failure = Ok(());
    for request in std::mem::take(running) {
        if !wait && !request.is_finished() {
            running.push(request);
            continue;
        }
        let ended = match request.join() {
            Ok(ended) => ended,
            Err(panic) => Err(io::Error::other(format!(
                "a request failed unexpectedly: {}",
                panic
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("no message")
            ))),
        };
        if first_failure.is_ok() {
            first_failure = ended;
        }
    }
    first_failure
}

fn write_reply(output: &Mutex<impl Write>, reply: &Value) -> io::Result<()> {
    let mut output = output.lock().unwrap_or_else(PoisonError::into_inner);
    serde_json::to_writer(&mut *output, reply)?;
    output.write_all(b"\n")?;
    output.flush()
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

/// The tools and the tool calls in progress, shared by the threads that
/// answer messages.
pub(crate) struct Session<T> {
    tools: Mutex<T>,
    cancel: Cancel,
    calls: Mutex<Calls>,
}

/// Tool calls waiting for the tools or running on them.
#[derive(Default)]
struct Calls {
    /// By request id: whether the client cancelled the call.
    cancelled: HashMap<String, bool>,
    running: Option<String>,
}

impl<T: BorrowMut<Tools>> Session<T> {
    pub(crate) fn new(tools: T) -> Self {
        Self {
            cancel: Borrow::<Tools>::borrow(&tools).cancel(),
            tools: Mutex::new(tools),
            calls: Mutex::new(Calls::default()),
        }
    }

    fn calls(&self) -> MutexGuard<'_, Calls> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs one tool call when its turn comes; `None` when the client
    /// cancelled it meanwhile, which then gets no reply.
    fn call(&self, id: &Value, name: &str, arguments: Map<String, Value>) -> Option<ToolOutput> {
        let key = id.to_string();
        self.calls().cancelled.insert(key.clone(), false);
        let mut tools = self.tools.lock().unwrap_or_else(PoisonError::into_inner);
        {
            let mut calls = self.calls();
            if calls.cancelled.get(&key) == Some(&true) {
                calls.cancelled.remove(&key);
                return None;
            }
            self.cancel.clear();
            calls.running = Some(key.clone());
        }
        let output = BorrowMut::<Tools>::borrow_mut(&mut *tools).call(name, arguments);
        let mut calls = self.calls();
        calls.running = None;
        (calls.cancelled.remove(&key) != Some(true)).then_some(output)
    }

    /// `notifications/cancelled`: the call stops waiting for the window, or
    /// is not started when still waiting for its turn.
    fn cancelled(&self, params: &Value) {
        let key = params["requestId"].to_string();
        let mut calls = self.calls();
        let Some(cancelled) = calls.cancelled.get_mut(&key) else {
            return;
        };
        *cancelled = true;
        if calls.running.as_ref() == Some(&key)
            && let Err(error) = self.cancel.cancel()
        {
            eprintln!("ketchup-mcp: the cancelled call {key} still waits for the window: {error}");
        }
    }
}

/// The reply to one message; notifications get none. Tool calls take turns
/// on the tools; every other request is answered without waiting for them.
pub(crate) fn handle(session: &Session<impl BorrowMut<Tools>>, message: Value) -> Option<Value> {
    if message["method"] == "notifications/cancelled" {
        session.cancelled(&message["params"]);
        return None;
    }
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
            let output = session.call(&id, name, arguments)?;
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
        let (send_reply, replies) = std::sync::mpsc::channel();
        serve(input, Replies(send_reply, Vec::new()), |message| {
            let id = message.get("id").cloned()?;
            Some(json!({"jsonrpc": "2.0", "id": id, "result": {}}))
        })
        .expect("served to the end");
        replies.try_iter().collect()
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

    /// Lines the test sends, read as stdin.
    struct Lines(std::sync::mpsc::Receiver<Vec<u8>>, Vec<u8>);

    impl io::Read for Lines {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.1.is_empty() {
                match self.0.recv() {
                    Ok(line) => self.1 = line,
                    Err(std::sync::mpsc::RecvError) => return Ok(0),
                }
            }
            let taken = buffer.len().min(self.1.len());
            buffer[..taken].copy_from_slice(&self.1[..taken]);
            self.1.drain(..taken);
            Ok(taken)
        }
    }

    /// Replies the server writes on stdout, one per line.
    struct Replies(std::sync::mpsc::Sender<Value>, Vec<u8>);

    impl Write for Replies {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.1.extend_from_slice(bytes);
            while let Some(end) = self.1.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.1.drain(..=end).collect();
                let reply = serde_json::from_slice(&line).map_err(io::Error::other)?;
                self.0.send(reply).map_err(io::Error::other)?;
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// MECH-1 (Měchenice 2026-11-02): a tools/call the window did not answer
    /// left the server silent, with a ping sent meanwhile unanswered until the
    /// call ended (up to 15 minutes for drawings), so the client saw a dead
    /// server and the window stayed used_by_another_client. A request sent
    /// while a call waits is answered within seconds.
    #[test]
    fn a_ping_is_answered_while_a_tool_call_waits_for_the_window() {
        let (send_line, lines) = std::sync::mpsc::channel();
        let (send_reply, replies) = std::sync::mpsc::channel();
        let (answer_the_call, window_answers) = std::sync::mpsc::channel::<()>();
        let window_answers = Mutex::new(window_answers);
        std::thread::spawn(move || {
            let input = io::BufReader::new(Lines(lines, Vec::new()));
            serve(input, Replies(send_reply, Vec::new()), move |message| {
                let id = message.get("id").cloned()?;
                if message["method"] == "tools/call" {
                    // A window that does not answer this request until the test ends.
                    let window = window_answers.lock().expect("one call at a time");
                    let answered = window.recv_timeout(std::time::Duration::from_secs(60));
                    assert!(answered.is_err(), "the test never answers the call");
                }
                Some(json!({"jsonrpc": "2.0", "id": id, "result": {}}))
            })
        });
        let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "inspect", "arguments": {"action": "status"}}});
        let ping = json!({"jsonrpc": "2.0", "id": 2, "method": "ping"});
        for message in [call, ping] {
            send_line
                .send(format!("{message}\n").into_bytes())
                .expect("the server reads");
        }
        let first = replies.recv_timeout(std::time::Duration::from_secs(5));
        drop(answer_the_call);
        let first = first.expect("no reply within 5 s while a tool call waits for the window");
        assert_eq!(first["id"], 2, "{first}");
    }
}
