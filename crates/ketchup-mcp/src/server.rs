//! MCP over stdio: one JSON-RPC 2.0 message per line in each direction.
use crate::{
    bridge::Cancel,
    discovery, schema,
    tools::{ClientName, ToolOutput, Tools},
};
use serde_json::{Map, Value, json};
use std::{
    borrow::{Borrow, BorrowMut},
    collections::HashMap,
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc},
    thread,
};

/// Protocol versions this server speaks, newest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// The longest message read: a whole program of about 8 MB, JSON-escaped.
const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;

/// Serves MCP until stdin closes. `app_executable` starts new windows for `open_window`.
pub fn serve_stdio(app_executable: Option<PathBuf>) -> io::Result<()> {
    let session = Arc::new(Session::new(Tools::new(
        app_executable,
        discovery::default_root(),
    )));
    let gone = Arc::default();
    serve(io::stdin().lock(), io::stdout(), &gone, move |message| {
        route(&session, message)
    })
}

/// Set once the client has gone, that is its input has ended.
#[derive(Default)]
struct Gone(Mutex<bool>, Condvar);

impl Gone {
    fn set(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.1.notify_all();
    }

    fn is_set(&self) -> bool {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the client goes within `timeout`.
    #[cfg(test)]
    fn wait(&self, timeout: std::time::Duration) -> bool {
        let gone = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let (gone, _) = self
            .1
            .wait_timeout_while(gone, timeout, |gone| !*gone)
            .unwrap_or_else(PoisonError::into_inner);
        *gone
    }
}

/// A tool call waiting for its turn; its reply, or none when it was cancelled.
type Turn = Box<dyn FnOnce() -> Option<Value> + Send>;

/// What [`serve`] does with a message.
enum Routed {
    /// Answered at once on the thread that reads the input.
    Now(Option<Value>),
    /// Run by the one worker, after the calls that came before it.
    InTurn(Turn),
}

/// Tool calls take their turns in the order they came, on one worker, so an
/// edit sent behind a long export runs after it (a `Mutex` serves waiting
/// threads in no set order); every other message (initialize, ping,
/// tools/list, a cancellation) is answered at once, also while a call waits
/// for the window (review 2026-11-04 MCP-142).
fn route(session: &Arc<Session<Tools>>, message: Value) -> Routed {
    let Some(id) = message
        .get("id")
        .filter(|_| message["method"] == "tools/call")
    else {
        return Routed::Now(handle(session, message));
    };
    // Known before its turn, so a cancellation meanwhile keeps it from running.
    session.queued(id);
    let session = Arc::clone(session);
    Routed::InTurn(Box::new(move || handle(&session, message)))
}

/// Answers each line of `input` on `output` until `input` ends: at once, or
/// a tool call in its turn on one worker (MECH-1, MCP-142). A line that is no
/// JSON, or longer than [`MAX_LINE_BYTES`], gets an error reply and the
/// server goes on with the next (review 2026-10-31 P3). When `input` ends the
/// client has gone: the call running then is answered, those waiting for
/// their turn are not run (review 2026-11-04 MCP-142); `gone` is set then.
fn serve(
    mut input: impl BufRead,
    output: impl Write + Send + 'static,
    gone: &Arc<Gone>,
    route: impl Fn(Value) -> Routed,
) -> io::Result<()> {
    let output = Arc::new(Mutex::new(output));
    let (queue, turns) = mpsc::channel::<Turn>();
    let worker = {
        let (output, gone) = (Arc::clone(&output), Arc::clone(gone));
        thread::spawn(move || {
            let mut first_failure = Ok(());
            for turn in turns {
                if gone.is_set() {
                    continue;
                }
                if let Some(reply) = turn() {
                    let written = write_reply(&output, &reply);
                    if first_failure.is_ok() {
                        first_failure = written;
                    }
                }
            }
            first_failure
        })
    };
    let read = read_messages(&mut input, &output, &queue, route);
    gone.set();
    drop(queue);
    let worked = worker.join().unwrap_or_else(|panic| {
        Err(io::Error::other(format!(
            "a tool call failed unexpectedly: {}",
            panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("no message")
        )))
    });
    read.and(worked)
}

/// Reads `input` to its end, answering at once or queueing each message.
fn read_messages(
    input: &mut impl BufRead,
    output: &Mutex<impl Write>,
    queue: &mpsc::Sender<Turn>,
    route: impl Fn(Value) -> Routed,
) -> io::Result<()> {
    while let Some(line) = read_line(input)? {
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
        match message.map(&route) {
            Ok(Routed::Now(Some(reply))) | Err(reply) => write_reply(output, &reply)?,
            Ok(Routed::Now(None)) => {}
            Ok(Routed::InTurn(turn)) => {
                if queue.send(turn).is_err() {
                    return Err(io::Error::other("the worker that runs tool calls stopped"));
                }
            }
        }
    }
    Ok(())
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
    client: ClientName,
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
            client: Borrow::<Tools>::borrow(&tools).client_name(),
            tools: Mutex::new(tools),
            calls: Mutex::new(Calls::default()),
        }
    }

    /// Keeps the name the client gives in `initialize` for the windows it
    /// attaches to; never waits for a tool call.
    fn remember_client(&self, params: &Value) {
        *self.client.lock().unwrap_or_else(PoisonError::into_inner) =
            client_name(&params["clientInfo"]);
    }

    fn calls(&self) -> MutexGuard<'_, Calls> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes a tool call waiting for its turn, so a cancellation finds it.
    fn queued(&self, id: &Value) {
        self.calls().cancelled.insert(id.to_string(), false);
    }

    /// Runs one tool call when its turn comes; `None` when the client
    /// cancelled it meanwhile, which then gets no reply.
    fn call(&self, id: &Value, name: &str, arguments: Map<String, Value>) -> Option<ToolOutput> {
        let key = id.to_string();
        self.calls().cancelled.entry(key.clone()).or_insert(false);
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
        "initialize" => {
            session.remember_client(&params);
            initialize(&params)
        }
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

/// How the window names the client: its title, or the name a known client
/// gives itself in the words its users know.
fn client_name(info: &Value) -> Option<String> {
    let title = info["title"].as_str().map(str::trim);
    if let Some(title) = title.filter(|title| !title.is_empty()) {
        return Some(title.to_owned());
    }
    let name = info["name"].as_str()?.trim();
    let known = match name {
        "" => return None,
        "claude-code" => "Claude Code",
        "claude-ai" => "Claude Desktop",
        "codex" | "codex-mcp-client" => "Codex",
        other => other,
    };
    Some(known.to_owned())
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn replies(input: &[u8]) -> Vec<Value> {
        let (send_reply, replies) = std::sync::mpsc::channel();
        serve(
            input,
            Replies(send_reply, Vec::new()),
            &Arc::default(),
            |message| Routed::Now(answer(&message)),
        )
        .expect("served to the end");
        replies.try_iter().collect()
    }

    fn answer(message: &Value) -> Option<Value> {
        let id = message.get("id").cloned()?;
        Some(json!({"jsonrpc": "2.0", "id": id, "result": {}}))
    }

    fn tool_call(id: u64) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "inspect", "arguments": {"action": "status"}}})
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
        let window_answers = Arc::new(Mutex::new(window_answers));
        std::thread::spawn(move || {
            let input = io::BufReader::new(Lines(lines, Vec::new()));
            serve(
                input,
                Replies(send_reply, Vec::new()),
                &Arc::default(),
                move |message| {
                    if message["method"] != "tools/call" {
                        return Routed::Now(answer(&message));
                    }
                    let window_answers = Arc::clone(&window_answers);
                    Routed::InTurn(Box::new(move || {
                        // A window that does not answer this request until the test ends.
                        let window = window_answers.lock().expect("one call at a time");
                        let answered = window.recv_timeout(std::time::Duration::from_secs(60));
                        assert!(answered.is_err(), "the test never answers the call");
                        answer(&message)
                    }))
                },
            )
        });
        let ping = json!({"jsonrpc": "2.0", "id": 2, "method": "ping"});
        for message in [tool_call(1), ping] {
            send_line
                .send(format!("{message}\n").into_bytes())
                .expect("the server reads");
        }
        let first = replies.recv_timeout(std::time::Duration::from_secs(5));
        drop(answer_the_call);
        let first = first.expect("no reply within 5 s while a tool call waits for the window");
        assert_eq!(first["id"], 2, "{first}");
    }

    /// Review 2026-11-04 MCP-142: each call had its own thread waiting on a
    /// `Mutex`, which hands it to waiting threads in no set order, so an edit
    /// sent after another could run first. Calls now run in the order they came.
    #[test]
    fn tool_calls_run_in_the_order_they_came() {
        let (send_line, lines) = std::sync::mpsc::channel();
        let (send_reply, replies) = std::sync::mpsc::channel();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let order = Arc::clone(&ran);
        let served = std::thread::spawn(move || {
            let input = io::BufReader::new(Lines(lines, Vec::new()));
            serve(
                input,
                Replies(send_reply, Vec::new()),
                &Arc::default(),
                move |message| {
                    let order = Arc::clone(&order);
                    Routed::InTurn(Box::new(move || {
                        order
                            .lock()
                            .expect("one call at a time")
                            .push(message["id"].clone());
                        answer(&message)
                    }))
                },
            )
        });
        for id in 1..=40 {
            let line = format!("{}\n", tool_call(id)).into_bytes();
            send_line.send(line).expect("the server reads");
        }
        let replied: Vec<Value> = (1..=40)
            .map(|_| {
                let reply = replies.recv_timeout(std::time::Duration::from_secs(5));
                reply.expect("every call is answered")["id"].clone()
            })
            .collect();
        drop(send_line);
        served
            .join()
            .expect("the server ends")
            .expect("served to the end");
        let expected: Vec<Value> = (1..=40).map(Value::from).collect();
        assert_eq!(*ran.lock().expect("served"), expected);
        assert_eq!(replied, expected);
    }

    /// Review 2026-11-04 MCP-142: when stdin ended, the server waited for every
    /// call, so the edits a client queued behind a 15-minute export were made
    /// minutes after it had gone. The call running then still ends and is
    /// answered; the ones waiting for their turn are not run.
    #[test]
    fn calls_waiting_their_turn_are_not_run_once_the_client_has_gone() {
        let (send_line, lines) = std::sync::mpsc::channel();
        let (send_started, started) = std::sync::mpsc::channel();
        let (send_reply, replies) = std::sync::mpsc::channel();
        let gone: Arc<Gone> = Arc::default();
        let second_ran = Arc::new(AtomicBool::new(false));
        let (seen_gone, ran) = (Arc::clone(&gone), Arc::clone(&second_ran));
        let served = std::thread::spawn(move || {
            let input = io::BufReader::new(Lines(lines, Vec::new()));
            serve(
                input,
                Replies(send_reply, Vec::new()),
                &gone,
                move |message| {
                    let (started, gone, ran) = (
                        send_started.clone(),
                        Arc::clone(&seen_gone),
                        Arc::clone(&ran),
                    );
                    Routed::InTurn(Box::new(move || {
                        if message["id"] != 1 {
                            ran.store(true, Ordering::Release);
                            return answer(&message);
                        }
                        // A long export: it is still running when the client goes.
                        started.send(()).expect("the test waits");
                        assert!(
                            gone.wait(std::time::Duration::from_secs(5)),
                            "the client never went"
                        );
                        answer(&message)
                    }))
                },
            )
        });
        for id in [1, 2] {
            let line = format!("{}\n", tool_call(id)).into_bytes();
            send_line.send(line).expect("the server reads");
        }
        started
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first call starts");
        drop(send_line);
        served
            .join()
            .expect("the server ends")
            .expect("served to the end");
        let replied: Vec<Value> = replies
            .try_iter()
            .map(|reply| reply["id"].clone())
            .collect();
        assert_eq!(replied, vec![json!(1)], "only the running call is answered");
        assert!(
            !second_ran.load(Ordering::Acquire),
            "a call still waiting for its turn ran after the client had gone"
        );
    }

    /// Review 2026-11-04 MCP-142: a call registered itself only when its thread
    /// started, so a cancellation sent while it still waited was dropped and
    /// the call then ran. It is now known from the moment it is read.
    #[test]
    fn a_call_cancelled_while_waiting_for_its_turn_never_runs() {
        let root = tempfile::tempdir().expect("a temporary folder");
        let session = Arc::new(Session::new(Tools::new(None, Some(root.path().to_owned()))));
        let Routed::InTurn(turn) = route(&session, tool_call(5)) else {
            panic!("a tool call waits for its turn");
        };
        let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": 5, "reason": "the user stopped it"}});
        assert!(matches!(route(&session, cancel), Routed::Now(None)));
        assert_eq!(turn(), None, "the cancelled call ran and was answered");
        assert!(
            session.calls().cancelled.is_empty(),
            "nothing is left registered"
        );
    }
}
