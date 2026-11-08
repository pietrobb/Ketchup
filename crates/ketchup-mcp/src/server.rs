//! MCP over stdio: one JSON-RPC 2.0 message per line in each direction.
use crate::{
    bridge::Cancel,
    discovery, schema,
    tools::{ClientName, ToolOutput, Tools},
};
use serde_json::{Map, Value, json};
use std::{
    borrow::{Borrow, BorrowMut},
    collections::{BTreeMap, VecDeque},
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
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
    let gone = Arc::new(Gone::cancelling(session.cancel.clone()));
    serve(io::stdin().lock(), io::stdout(), &gone, move |message| {
        route(&session, message)
    })
}

/// The tool calls read and waiting for their turn at most, so a client sending
/// faster than the window works does not fill the memory with messages of up
/// to [`MAX_LINE_BYTES`] (review 2026-11-04 r19 MCP-161). One more is refused
/// as [`BUSY`] at once; reading never waits, so a cancellation, a ping or the
/// end of the input is always seen (review 2026-11-06 MCP-180).
const QUEUED_CALLS: usize = 16;

/// JSON-RPC error code (implementation-defined server error) of a tool call
/// refused because [`QUEUED_CALLS`] wait already.
const BUSY: i64 = -32000;

/// The reply refusing the tool call `id` as [`BUSY`].
fn refusal(id: Value) -> Value {
    failure(
        id,
        BUSY,
        &format!(
            "Kečup is busy: {QUEUED_CALLS} tool calls already wait for their turn; send this one again once one of them is answered"
        ),
    )
}

/// Set once the client has gone: its input has ended or a reply could not be
/// written to it. The tool call running then is cancelled, as no one is left
/// to be told what it did (review 2026-11-04 r19 MCP-161).
#[derive(Default)]
struct Gone(Mutex<bool>, Condvar, Cancel);

impl Gone {
    fn cancelling(cancel: Cancel) -> Self {
        Self(Mutex::default(), Condvar::new(), cancel)
    }

    fn set(&self) -> io::Result<()> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.1.notify_all();
        self.2.close()
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

/// The reply to a tool call refused when [`QUEUED_CALLS`] wait already.
type Refusal = Box<dyn FnOnce() -> Value + Send>;

/// Whether a queued tool call still waits for its turn: false once it was
/// cancelled, or has started.
type Waits = Box<dyn Fn() -> bool + Send>;

/// What [`serve`] does with a message.
enum Routed {
    /// Answered at once on the thread that reads the input.
    Now(Option<Value>),
    /// Run by the one worker, after the calls that came before it, or
    /// refused when too many wait.
    InTurn(Turn, Refusal, Waits),
}

/// The tool calls waiting for their turn, in the order they came. One that
/// no longer waits leaves when the next call comes, so a call cancelled while
/// waiting neither takes a place nor keeps its message (review 2026-11-06 r23
/// P3: a client sending again after its timeout was refused as busy until the
/// long call ahead of them ended).
#[derive(Default)]
struct Queue {
    state: Mutex<QueueState>,
    ready: Condvar,
}

#[derive(Default)]
struct QueueState {
    waiting: VecDeque<(Turn, Waits)>,
    /// No more calls come: the input has ended.
    closed: bool,
    /// The worker has ended.
    stopped: bool,
}

/// What became of a call given to the [`Queue`].
enum Queued {
    Waits,
    Full,
    WorkerStopped,
}

impl Queue {
    fn state(&self) -> MutexGuard<'_, QueueState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn push(&self, turn: Turn, waits: Waits) -> Queued {
        let mut state = self.state();
        if state.stopped {
            return Queued::WorkerStopped;
        }
        state.waiting.retain(|(_, waits)| waits());
        if state.waiting.len() >= QUEUED_CALLS {
            return Queued::Full;
        }
        state.waiting.push_back((turn, waits));
        self.ready.notify_one();
        Queued::Waits
    }

    /// The next call in turn; `None` once the input has ended and none is left.
    fn next(&self) -> Option<Turn> {
        let mut state = self.state();
        loop {
            if let Some((turn, _)) = state.waiting.pop_front() {
                return Some(turn);
            }
            if state.closed {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn close(&self) {
        self.state().closed = true;
        self.ready.notify_all();
    }
}

/// Tells the [`Queue`] its worker has ended, panicked or not.
struct WorkerEnds(Arc<Queue>);

impl Drop for WorkerEnds {
    fn drop(&mut self) {
        self.0.state().stopped = true;
    }
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
    let turn = session.queued(id);
    let (refused, waits, id) = (Arc::clone(session), Arc::clone(session), id.clone());
    let session = Arc::clone(session);
    Routed::InTurn(
        Box::new(move || {
            let reply = answer(&session, message, Some(turn));
            // A call refused as malformed did not run, so it is forgotten here.
            session.forget(turn);
            reply
        }),
        Box::new(move || {
            refused.forget(turn);
            refusal(id)
        }),
        Box::new(move || waits.waits(turn)),
    )
}

/// Answers each line of `input` on `output` until `input` ends: at once, or
/// a tool call in its turn on one worker (MECH-1, MCP-142). A line that is no
/// JSON, or longer than [`MAX_LINE_BYTES`], gets an error reply and the
/// server goes on with the next (review 2026-10-31 P3). When `input` ends, or
/// a reply cannot be written, the client has gone: `gone` is set, cancelling
/// the call running then, and those waiting for their turn are not run
/// (review 2026-11-04 MCP-142, r19 MCP-161).
fn serve(
    mut input: impl BufRead,
    output: impl Write + Send + 'static,
    gone: &Arc<Gone>,
    route: impl Fn(Value) -> Routed,
) -> io::Result<()> {
    let output = Arc::new(Mutex::new(output));
    let queue = Arc::new(Queue::default());
    let worker = {
        let (output, gone) = (Arc::clone(&output), Arc::clone(gone));
        let ends = WorkerEnds(Arc::clone(&queue));
        thread::spawn(move || {
            let (mut written, mut stopped) = (Ok(()), Ok(()));
            // Once gone, the calls left are taken off the queue, not run.
            while let Some(turn) = ends.0.next() {
                if gone.is_set() {
                    continue;
                }
                if let Some(reply) = turn()
                    && let Err(error) = write_reply(&output, &reply)
                {
                    written = Err(error);
                    stopped = gone.set();
                }
            }
            written.and(stopped)
        })
    };
    let read = read_messages(&mut input, &output, &queue, route);
    let stopped = gone.set();
    queue.close();
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
    read.and(worked).and(stopped)
}

/// Reads `input` to its end, answering at once or queueing each message.
fn read_messages(
    input: &mut impl BufRead,
    output: &Mutex<impl Write>,
    queue: &Queue,
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
            Ok(Routed::InTurn(turn, refused, waits)) => match queue.push(turn, waits) {
                Queued::Waits => {}
                Queued::Full => write_reply(output, &refused())?,
                Queued::WorkerStopped => {
                    return Err(io::Error::other("the worker that runs tool calls stopped"));
                }
            },
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

/// Tool calls waiting for the tools or running on them, each by the number of
/// its turn, not its request id: a client may send a call again under the id
/// of one still waiting or running (review 2026-11-07 P3: refusing or ending
/// the first forgot the other, which the queue then dropped unanswered).
#[derive(Default)]
struct Calls {
    /// The number the next call gets.
    next: u64,
    /// By turn: the call's request id and whether the client cancelled it.
    known: BTreeMap<u64, (String, bool)>,
    running: Option<u64>,
    /// Calls cancelled after they reached the window, told with the next reply.
    cancelled_after_sent: Vec<Value>,
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

    /// Notes a tool call waiting for its turn, so a cancellation finds it;
    /// the number of its turn.
    fn queued(&self, id: &Value) -> u64 {
        let mut calls = self.calls();
        let turn = calls.next;
        calls.next += 1;
        calls.known.insert(turn, (id.to_string(), false));
        turn
    }

    /// Forgets a tool call refused before its turn.
    fn forget(&self, turn: u64) {
        self.calls().known.remove(&turn);
    }

    /// Whether the queued call of `turn` still waits for it. One cancelled
    /// meanwhile is forgotten: only the [`Queue`] asks, and drops it then.
    fn waits(&self, turn: u64) -> bool {
        let mut calls = self.calls();
        if calls.running == Some(turn) {
            return false;
        }
        match calls.known.get(&turn) {
            Some((_, false)) => true,
            Some((_, true)) => {
                calls.known.remove(&turn);
                false
            }
            None => false,
        }
    }

    /// Runs one tool call when its turn comes (`None`: a call not queued,
    /// which takes the next); `None` when the client cancelled it meanwhile,
    /// which then gets no reply.
    fn call(
        &self,
        turn: Option<u64>,
        id: &Value,
        name: &str,
        arguments: Map<String, Value>,
    ) -> Option<ToolOutput> {
        let turn = turn.unwrap_or_else(|| self.queued(id));
        let mut tools = self.tools.lock().unwrap_or_else(PoisonError::into_inner);
        {
            let mut calls = self.calls();
            if calls
                .known
                .get(&turn)
                .is_none_or(|(_, cancelled)| *cancelled)
            {
                calls.known.remove(&turn);
                return None;
            }
            self.cancel.clear();
            calls.running = Some(turn);
        }
        let output = BorrowMut::<Tools>::borrow_mut(&mut *tools).call(name, arguments);
        let mut calls = self.calls();
        calls.running = None;
        if calls
            .known
            .remove(&turn)
            .is_none_or(|(_, cancelled)| !cancelled)
        {
            return Some(output);
        }
        if let Some(outcome) = cancelled_outcome(id, name, &output) {
            calls.cancelled_after_sent.push(outcome);
        }
        None
    }

    /// What became of the calls cancelled after they reached the window,
    /// once: they got no reply, though their change can stay.
    fn take_cancelled_after_sent(&self) -> Option<Value> {
        let outcomes = std::mem::take(&mut self.calls().cancelled_after_sent);
        (!outcomes.is_empty()).then(|| {
            json!({
                "cancelled_calls": outcomes,
                "note": "The client cancelled these calls after they had been sent to the window, \
                         so they got no reply. A completed one did what it was asked (stamp is the \
                         document after it); for the others read inspect action=status before \
                         repeating them.",
            })
        })
    }

    /// `notifications/cancelled`: the call stops waiting for the window, or
    /// is not started when still waiting for its turn; every call known under
    /// the id then, as the client cannot tell them apart.
    fn cancelled(&self, params: &Value) {
        let key = params["requestId"].to_string();
        let mut calls = self.calls();
        let mut running = false;
        let Calls {
            known,
            running: now,
            ..
        } = &mut *calls;
        for (turn, (id, cancelled)) in known.iter_mut() {
            if *id == key {
                *cancelled = true;
                running |= *now == Some(*turn);
            }
        }
        if running && let Err(error) = self.cancel.cancel() {
            ketchup_rejection::log!(
                "ketchup-mcp: the cancelled call {key} still waits for the window: {error}"
            );
        }
    }
}

/// What a cancelled call's agent needs of its `output`: whether it completed,
/// and the document after it or why not; `None` when it was never sent.
fn cancelled_outcome(id: &Value, name: &str, output: &ToolOutput) -> Option<Value> {
    let answer = output
        .content
        .iter()
        .find_map(|item| item["text"].as_str())
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .unwrap_or(Value::Null);
    if !output.is_error {
        return Some(json!({"id": id, "tool": name, "completed": true, "stamp": answer["stamp"]}));
    }
    (answer["error"] != "cancelled").then(|| {
        json!({"id": id, "tool": name, "completed": false,
               "error": answer["error"], "message": answer["message"]})
    })
}

/// The reply to one message; notifications get none. Tool calls take turns
/// on the tools; every other request is answered without waiting for them.
pub(crate) fn handle(session: &Session<impl BorrowMut<Tools>>, message: Value) -> Option<Value> {
    answer(session, message, None)
}

/// [`handle`] of a message that is the tool call of `turn`, if queued.
fn answer(
    session: &Session<impl BorrowMut<Tools>>,
    message: Value,
    turn: Option<u64>,
) -> Option<Value> {
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
            let output = session.call(turn, &id, name, arguments)?;
            let mut content = output.content;
            if let Some(earlier) = session.take_cancelled_after_sent() {
                content.push(json!({"type": "text", "text": earlier.to_string()}));
            }
            json!({"content": content, "isError": output.is_error})
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
                    let refused = busy(&message);
                    Routed::InTurn(
                        Box::new(move || {
                            // A window that does not answer this request until the test ends.
                            let window = window_answers.lock().expect("one call at a time");
                            let answered = window.recv_timeout(std::time::Duration::from_secs(60));
                            assert!(answered.is_err(), "the test never answers the call");
                            answer(&message)
                        }),
                        refused,
                        always_waits(),
                    )
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
                    let refused = busy(&message);
                    Routed::InTurn(
                        Box::new(move || {
                            order
                                .lock()
                                .expect("one call at a time")
                                .push(message["id"].clone());
                            answer(&message)
                        }),
                        refused,
                        always_waits(),
                    )
                },
            )
        });
        // As many as may wait, so none is refused as busy.
        let calls = u64::try_from(QUEUED_CALLS).expect("a few");
        for id in 1..=calls {
            let line = format!("{}\n", tool_call(id)).into_bytes();
            send_line.send(line).expect("the server reads");
        }
        let replied: Vec<Value> = (1..=calls)
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
        let expected: Vec<Value> = (1..=calls).map(Value::from).collect();
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
                    let refused = busy(&message);
                    Routed::InTurn(
                        Box::new(move || {
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
                        }),
                        refused,
                        always_waits(),
                    )
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

    /// Review 2026-11-04 r19 MCP-161: when stdin ended, the call running then
    /// still waited for the window, so an apply_and_verify or run_plugin
    /// changed the document for a client that had gone. Its wait is cancelled.
    #[test]
    fn the_call_running_when_the_client_goes_is_cancelled() {
        let (send_line, lines) = std::sync::mpsc::channel();
        let (send_started, started) = std::sync::mpsc::channel();
        let cancel = Cancel::default();
        let gone = Arc::new(Gone::cancelling(cancel.clone()));
        let (send_reply, _replies) = std::sync::mpsc::channel();
        let served = std::thread::spawn(move || {
            let input = io::BufReader::new(Lines(lines, Vec::new()));
            serve(
                input,
                Replies(send_reply, Vec::new()),
                &gone,
                move |message| {
                    let (started, cancel) = (send_started.clone(), cancel.clone());
                    let refused = busy(&message);
                    Routed::InTurn(
                        Box::new(move || {
                            // An apply waiting for the window's reply, as
                            // Tools::send waits; the window sees the connection end.
                            let window =
                                std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
                            let address = window.local_addr().expect("its address");
                            let connection =
                                crate::bridge::Connection::open(address, String::new())
                                    .expect("the window accepts");
                            let (mut accepted, _) = window.accept().expect("connected");
                            cancel.clear();
                            assert!(cancel.arm(&connection));
                            started.send(()).expect("the test waits");
                            accepted
                                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                                .expect("a read timeout");
                            let mut byte = [0; 1];
                            let read = io::Read::read(&mut accepted, &mut byte);
                            assert_eq!(
                                read.expect(
                                    "the call still waits for the window after the client went"
                                ),
                                0,
                                "the connection ended"
                            );
                            answer(&message)
                        }),
                        refused,
                        always_waits(),
                    )
                },
            )
        });
        send_line
            .send(format!("{}\n", tool_call(1)).into_bytes())
            .expect("the server reads");
        started
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the call starts");
        drop(send_line);
        served
            .join()
            .expect("the call is cancelled, not left waiting")
            .expect("served to the end");
    }

    /// Replies to a client that has gone: every write fails, and is told.
    struct Broken(std::sync::mpsc::Sender<()>);

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            self.0.send(()).map_err(io::Error::other)?;
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Review 2026-11-04 r19 MCP-161: after a reply could not be written (the
    /// client's pipe broken), the worker went on making the edits queued.
    #[test]
    fn no_call_runs_after_a_reply_could_not_be_written() {
        let (send_line, lines) = std::sync::mpsc::channel();
        let (send_routed, routed) = std::sync::mpsc::channel();
        let routed = Arc::new(Mutex::new(routed));
        let later_ran = Arc::new(AtomicBool::new(false));
        let ran = Arc::clone(&later_ran);
        let (send_failed, failed) = std::sync::mpsc::channel();
        let served = std::thread::spawn(move || {
            let input = io::BufReader::new(Lines(lines, Vec::new()));
            serve(
                input,
                Broken(send_failed),
                &Arc::default(),
                move |message| {
                    send_routed
                        .send(message["id"].clone())
                        .expect("the test reads");
                    let (routed, ran) = (Arc::clone(&routed), Arc::clone(&ran));
                    let refused = busy(&message);
                    Routed::InTurn(
                        Box::new(move || {
                            if message["id"] == 1 {
                                // Answered once the next calls are read.
                                let routed = routed.lock().expect("one call at a time");
                                for _ in 0..3 {
                                    routed
                                        .recv_timeout(std::time::Duration::from_secs(5))
                                        .expect("read");
                                }
                            } else {
                                ran.store(true, Ordering::Release);
                            }
                            answer(&message)
                        }),
                        refused,
                        always_waits(),
                    )
                },
            )
        });
        for id in 1..=3 {
            let line = format!("{}\n", tool_call(id)).into_bytes();
            send_line.send(line).expect("the server reads");
        }
        // The client goes only after the reply failed, so its input is open.
        failed
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first reply is written");
        drop(send_line);
        let served = served.join().expect("the server ends");
        assert_eq!(
            served.expect_err("the reply was not written").kind(),
            io::ErrorKind::BrokenPipe
        );
        assert!(
            !later_ran.load(Ordering::Acquire),
            "an edit ran after its client's pipe broke"
        );
    }

    /// A server running call 1 until its client goes, with `over` more calls
    /// than [`QUEUED_CALLS`] sent behind it.
    struct Behind {
        send_line: std::sync::mpsc::Sender<Vec<u8>>,
        replies: std::sync::mpsc::Receiver<Value>,
        /// The messages other than tool calls, as read.
        seen: std::sync::mpsc::Receiver<Value>,
        /// Whether a call other than 1 ran.
        later_ran: Arc<AtomicBool>,
        served: thread::JoinHandle<io::Result<()>>,
    }

    fn calls_behind_a_long_one(over: u64) -> Behind {
        let (send_line, lines) = std::sync::mpsc::channel();
        let (send_reply, replies) = std::sync::mpsc::channel();
        let (send_started, started) = std::sync::mpsc::channel();
        let (send_seen, seen) = std::sync::mpsc::channel();
        let gone: Arc<Gone> = Arc::default();
        let later_ran = Arc::new(AtomicBool::new(false));
        let (seen_gone, ran) = (Arc::clone(&gone), Arc::clone(&later_ran));
        let served = thread::spawn(move || {
            let input = io::BufReader::new(Lines(lines, Vec::new()));
            serve(
                input,
                Replies(send_reply, Vec::new()),
                &gone,
                move |message| {
                    if message["method"] != "tools/call" {
                        send_seen.send(message.clone()).expect("the test reads");
                        return Routed::Now(answer(&message));
                    }
                    let (started, gone, ran) = (
                        send_started.clone(),
                        Arc::clone(&seen_gone),
                        Arc::clone(&ran),
                    );
                    let refused = busy(&message);
                    Routed::InTurn(
                        Box::new(move || {
                            if message["id"] != 1 {
                                ran.store(true, Ordering::Release);
                                return answer(&message);
                            }
                            started.send(()).expect("the test waits");
                            assert!(
                                gone.wait(std::time::Duration::from_secs(10)),
                                "the client never went"
                            );
                            answer(&message)
                        }),
                        refused,
                        always_waits(),
                    )
                },
            )
        });
        send_line
            .send(format!("{}\n", tool_call(1)).into_bytes())
            .expect("the server reads");
        started
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first call starts");
        let last = u64::try_from(QUEUED_CALLS).expect("a few") + 1 + over;
        for id in 2..=last {
            let line = format!("{}\n", tool_call(id)).into_bytes();
            send_line.send(line).expect("the server reads");
        }
        Behind {
            send_line,
            replies,
            seen,
            later_ran,
            served,
        }
    }

    fn busy(message: &Value) -> Refusal {
        let id = message["id"].clone();
        Box::new(move || refusal(id))
    }

    fn always_waits() -> Waits {
        Box::new(|| true)
    }

    /// Review 2026-11-06 r23 P3: calls cancelled while waiting still took
    /// their places, so a client sending them again after its timeout was
    /// refused as busy until the long call ahead ended. They leave the queue,
    /// with their messages, once the next call comes.
    #[test]
    fn calls_cancelled_while_waiting_leave_their_places() {
        let queue = Queue::default();
        let dropped = Arc::new(());
        let flags: Vec<Arc<AtomicBool>> = (0..QUEUED_CALLS)
            .map(|_| Arc::new(AtomicBool::new(true)))
            .collect();
        for flag in &flags {
            let (flag, message) = (Arc::clone(flag), Arc::clone(&dropped));
            let pushed = queue.push(
                Box::new(move || Some(json!(Arc::strong_count(&message)))),
                Box::new(move || flag.load(Ordering::Acquire)),
            );
            assert!(matches!(pushed, Queued::Waits));
        }
        assert!(matches!(
            queue.push(Box::new(|| None), always_waits()),
            Queued::Full
        ));
        for flag in &flags {
            flag.store(false, Ordering::Release);
        }
        assert!(matches!(
            queue.push(Box::new(|| Some(json!("again"))), always_waits()),
            Queued::Waits
        ));
        assert_eq!(Arc::strong_count(&dropped), 1, "their messages are dropped");
        queue.close();
        assert_eq!(queue.next().and_then(|turn| turn()), Some(json!("again")));
        assert!(queue.next().is_none());
    }

    /// A queued call waits until it is cancelled; the cancelled one is then
    /// forgotten as the queue drops it.
    #[test]
    fn a_queued_call_waits_until_it_is_cancelled() {
        let root = tempfile::tempdir().expect("a temporary folder");
        let session = Arc::new(Session::new(Tools::new(None, Some(root.path().to_owned()))));
        let Routed::InTurn(_, _, waits) = route(&session, tool_call(9)) else {
            panic!("a tool call waits for its turn");
        };
        assert!(waits());
        let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": 9}});
        assert!(matches!(route(&session, cancel), Routed::Now(None)));
        assert!(!waits());
        assert!(session.calls().known.is_empty());
    }

    /// Review 2026-11-06 MCP-180: with [`QUEUED_CALLS`] waiting the reader
    /// stopped in the queue, so a cancellation of the running call or a ping
    /// was not read until a call ended. Reading goes on: a call beyond the
    /// queue is refused at once as busy, and every other message is answered.
    #[test]
    fn a_full_queue_refuses_more_calls_and_reads_on() {
        let Behind {
            send_line,
            replies,
            seen,
            served,
            ..
        } = calls_behind_a_long_one(2);
        let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": 1}});
        let ping = json!({"jsonrpc": "2.0", "id": "ping", "method": "ping"});
        for message in [&cancel, &ping] {
            send_line
                .send(format!("{message}\n").into_bytes())
                .expect("the server reads");
        }
        let cancelled = seen.recv_timeout(std::time::Duration::from_secs(5));
        assert_eq!(cancelled.expect("the cancellation is read"), cancel);
        let replied: Vec<Value> = (0..3)
            .map(|_| {
                let reply = replies.recv_timeout(std::time::Duration::from_secs(5));
                reply.expect("answered while the queue is full")
            })
            .collect();
        let busy_ids: Vec<&Value> = replied
            .iter()
            .filter(|reply| reply["error"]["code"] == BUSY)
            .map(|reply| &reply["id"])
            .collect();
        let last = u64::try_from(QUEUED_CALLS).expect("a few") + 3;
        assert_eq!(busy_ids, [&json!(last - 1), &json!(last)], "{replied:?}");
        assert!(replied.iter().any(|reply| reply["id"] == "ping"));
        drop(send_line);
        served
            .join()
            .expect("the server ends")
            .expect("served to the end");
    }

    /// Review 2026-11-06 MCP-180: when the input ended behind a full queue the
    /// reader did not see it until the calls had made room, and 16 of them ran
    /// for a client that had gone.
    #[test]
    fn calls_behind_a_full_queue_are_not_run_once_the_client_has_gone() {
        // The client still reads the replies; only its input has ended.
        let Behind {
            send_line,
            replies: _replies,
            later_ran,
            served,
            ..
        } = calls_behind_a_long_one(3);
        drop(send_line);
        served
            .join()
            .expect("the server ends")
            .expect("served to the end");
        assert!(
            !later_ran.load(Ordering::Acquire),
            "a call waiting behind a full queue ran after the client had gone"
        );
    }

    /// Review 2026-11-04 MCP-142: a call registered itself only when its thread
    /// started, so a cancellation sent while it still waited was dropped and
    /// the call then ran. It is now known from the moment it is read.
    #[test]
    fn a_call_cancelled_while_waiting_for_its_turn_never_runs() {
        let root = tempfile::tempdir().expect("a temporary folder");
        let session = Arc::new(Session::new(Tools::new(None, Some(root.path().to_owned()))));
        let Routed::InTurn(turn, _, _) = route(&session, tool_call(5)) else {
            panic!("a tool call waits for its turn");
        };
        let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": 5, "reason": "the user stopped it"}});
        assert!(matches!(route(&session, cancel), Routed::Now(None)));
        assert_eq!(turn(), None, "the cancelled call ran and was answered");
        assert!(
            session.calls().known.is_empty(),
            "nothing is left registered"
        );
    }

    /// A call refused as busy (MCP-180) is answered with its id and leaves
    /// nothing registered for a cancellation to find.
    #[test]
    fn a_call_refused_as_busy_is_forgotten() {
        let root = tempfile::tempdir().expect("a temporary folder");
        let session = Arc::new(Session::new(Tools::new(None, Some(root.path().to_owned()))));
        let Routed::InTurn(_, refused, _) = route(&session, tool_call(7)) else {
            panic!("a tool call waits for its turn");
        };
        let reply = refused();
        assert_eq!(
            (reply["id"].clone(), reply["error"]["code"].clone()),
            (json!(7), json!(BUSY))
        );
        assert!(session.calls().known.is_empty());
    }

    /// Review 2026-11-07 P3: the calls were known by their id, so a client
    /// that sent a call again under the id of one refused or run meanwhile
    /// lost the one waiting: refusing or ending the first forgot it, and the
    /// queue dropped it without a reply.
    #[test]
    fn a_call_sent_again_under_the_id_of_another_still_waits_when_that_one_ends() {
        let root = tempfile::tempdir().expect("a temporary folder");
        let session = Arc::new(Session::new(Tools::new(None, Some(root.path().to_owned()))));
        let Routed::InTurn(_, refused, _) = route(&session, tool_call(7)) else {
            panic!("a tool call waits for its turn");
        };
        let Routed::InTurn(ran, _, _) = route(&session, tool_call(7)) else {
            panic!("a tool call waits for its turn");
        };
        let Routed::InTurn(_, _, waits) = route(&session, tool_call(7)) else {
            panic!("a tool call waits for its turn");
        };
        assert_eq!(refused()["error"]["code"], json!(BUSY));
        assert!(waits(), "refusing the first forgot the last");
        assert!(ran().is_some(), "the second is answered");
        assert!(waits(), "running the second forgot the last");
        // A cancellation of the id stops the call still waiting.
        let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": 7}});
        assert!(matches!(route(&session, cancel), Routed::Now(None)));
        assert!(!waits());
        // It reaches no call sent under the id after it.
        let Routed::InTurn(again, _, waits_again) = route(&session, tool_call(7)) else {
            panic!("a tool call waits for its turn");
        };
        assert!(waits_again(), "a call sent after the cancellation waits");
        assert!(again().is_some(), "and is answered");
    }
}
