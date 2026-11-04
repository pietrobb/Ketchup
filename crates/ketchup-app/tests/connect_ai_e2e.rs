//! #2687 (d): Connect AI end to end, from a profile no AI client has used
//! with Kečup: Connect writes the client's settings, the client starts the
//! server they name, and its tool call reaches this window. A second client
//! then takes the window over while the first one's request is stuck in it:
//! still queued, or begun and published without an answer (MECH-1).
use crate::harness::{Shell, WAIT_DEADLINE, poll_until};
use eframe::egui;
use ketchup_app::live_bridge::mcp_setup::{Environment, Harness, Registration};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

/// The variable both the window and the server find their windows under.
const RUNTIME_DIRECTORY: &str = if cfg!(windows) {
    "LOCALAPPDATA"
} else if cfg!(target_os = "macos") {
    "TMPDIR"
} else {
    "XDG_RUNTIME_DIR"
};

/// An MCP client: the server its settings name, spoken to line by line.
struct Client {
    child: Child,
    input: ChildStdin,
    lines: mpsc::Receiver<Value>,
    next_id: u64,
}

impl Client {
    /// Starts what `settings` (a client's JSON settings file) names for
    /// `ketchup`, finding windows under `runtime`, and introduces itself as `name`.
    fn start(shell: &mut Shell, settings: &Path, runtime: &Path, name: &str) -> Self {
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        let server = &settings["mcpServers"]["ketchup"];
        let args: Vec<&str> = server["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        let mut child = Command::new(server["command"].as_str().unwrap())
            .args(args)
            .env(RUNTIME_DIRECTORY, runtime)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in output.lines().map_while(Result::ok) {
                let Ok(message) = serde_json::from_str(&line) else {
                    continue;
                };
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input,
            lines,
            next_id: 1,
        };
        let initialized = client.ask(
            shell,
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": name, "version": "1"}}),
        );
        assert!(
            initialized["result"]["serverInfo"].is_object(),
            "{initialized}"
        );
        writeln!(
            client.input,
            "{}",
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .unwrap();
        client
    }

    /// Sends one request without running the window; returns its id.
    fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.input, "{request}").unwrap();
        id
    }

    /// Sends one request and runs the window until its answer arrives.
    fn ask(&mut self, shell: &mut Shell, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        self.answer(shell, id, method)
    }

    /// Runs the window until the answer to request `id` arrives.
    fn answer(&mut self, shell: &mut Shell, id: u64, method: &str) -> Value {
        let mut answer = None;
        let answered = poll_until(WAIT_DEADLINE, || {
            shell.step();
            while let Ok(message) = self.lines.try_recv() {
                if message["id"] == id {
                    answer = Some(message);
                }
            }
            answer.is_some()
        });
        assert!(answered, "no answer to {method} within {WAIT_DEADLINE:?}");
        answer.unwrap()
    }

    /// A tool call's result, as the agent reads it.
    fn call(&mut self, shell: &mut Shell, tool: &str, arguments: Value) -> (bool, Value) {
        let answer = self.ask(
            shell,
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
        );
        tool_result(&answer)
    }
}

/// A tool call's answer as the agent reads it: whether it failed, and its text.
fn tool_result(answer: &Value) -> (bool, Value) {
    let result = &answer["result"];
    let text = result["content"][0]["text"].as_str().unwrap();
    (
        result["isError"] == true,
        serde_json::from_str(text).unwrap(),
    )
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.child.kill().is_err() {
            // The client has already exited; it is reaped all the same.
        }
        if let Err(error) = self.child.wait() {
            eprintln!("the test client was not reaped: {error}");
        }
    }
}

/// How long a request sent to the server may take to reach the window.
const REQUEST_ARRIVAL: Duration = Duration::from_secs(20);

/// One message per frame asked of `context` at once, as a connection does
/// right after it queued a request for the window. egui tells only of a
/// request sooner than the one it already has, so `context` must have run
/// until it asks for no frame.
#[track_caller]
fn frame_requests(context: &egui::Context) -> mpsc::Receiver<()> {
    assert!(
        !context.has_requested_repaint(),
        "a frame is still asked for"
    );
    let (sender, requests) = mpsc::channel();
    context.set_request_repaint_callback(move |info| {
        if info.delay.is_zero() && sender.send(()).is_err() {
            // The test is over and no longer counts repaints.
        }
    });
    requests
}

/// Runs `context`, which nothing paints on, until it asks for no frame.
fn settle(context: &egui::Context) {
    for _ in 0..8 {
        if !context.has_requested_repaint() {
            return;
        }
        let painted = context.run(egui::RawInput::default(), |_| {});
        drop(painted);
    }
}

fn connected_label(shell: &Shell, client: &str) -> String {
    shell.catalog().format(
        "ai-connected",
        &BTreeMap::from([("client", client.to_owned())]),
    )
}

/// A window Claude Code drives through the settings Connect wrote, and Codex
/// started on the same settings, which sees the window held. Clients first:
/// each one's server ends before the window and the profile go.
struct TwoClients {
    first: Client,
    second: Client,
    shell: Shell,
    /// The context the window's attach requests wake.
    consent: egui::Context,
    /// The stamp the first client read after it connected.
    stamp: Value,
    profile: tempfile::TempDir,
}

fn two_clients() -> TwoClients {
    let profile = tempfile::tempdir().unwrap();
    let home = profile.path().join("home");
    let runtime = profile.path().join("runtime");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&runtime).unwrap();
    // Claude Code has run here once, never with Kečup.
    std::fs::write(home.join(".claude.json"), "{}").unwrap();
    let environment = Environment::in_home(home);
    let executable = Path::new(env!("CARGO_BIN_EXE_ketchup-app"));
    assert_eq!(
        environment.registration(Harness::ClaudeCode, executable),
        Registration::NotSetUp
    );
    let row = environment.set_up(Harness::ClaudeCode, executable);
    assert_eq!(row.setup_error, None);
    assert_eq!(row.registration, Registration::ThisKetchup);
    let settings = environment.config_file(Harness::ClaudeCode).unwrap();

    let mut shell = Shell::new();
    let consent = egui::Context::default();
    shell
        .app_mut()
        .enable_live_consent_broker_in(&consent, &runtime.join("Ketchup").join("live-instances"))
        .unwrap();

    let mut first = Client::start(&mut shell, &settings, &runtime, "claude-code");
    let (failed, connected) = first.call(&mut shell, "connect", json!({}));
    assert!(!failed, "{connected}");
    let (failed, status) = first.call(&mut shell, "inspect", json!({"action": "status"}));
    assert!(!failed, "{status}");
    assert_eq!(status["stamp"], json!(shell.app().live_bridge_stamp()));
    assert!(
        shell.has_visible_label(&connected_label(&shell, "Claude Code")),
        "the status line names the AI"
    );

    let mut second = Client::start(&mut shell, &settings, &runtime, "codex");
    let (failed, windows) = second.call(&mut shell, "windows", json!({}));
    assert!(!failed, "{windows}");
    assert_eq!(
        windows["windows"][0]["used_by_another_client"], true,
        "{windows}"
    );
    assert!(windows["used_by_another_client"].is_string(), "{windows}");
    TwoClients {
        first,
        second,
        shell,
        consent,
        stamp: status["stamp"].clone(),
        profile,
    }
}

/// The second client connects while the first one's request is stuck in the
/// window, which runs only for the connect from now on; the connect is
/// answered as a takeover. That it does not wait for the stuck request the
/// caller shows by that request's outcome, not by a time (review 2026-11-04 P3).
fn take_over(clients: &mut TwoClients) {
    let TwoClients {
        second,
        shell,
        consent,
        ..
    } = clients;
    settle(consent);
    let connect_queued = frame_requests(consent);
    let connect = second.send("tools/call", json!({"name": "connect", "arguments": {}}));
    connect_queued
        .recv_timeout(REQUEST_ARRIVAL)
        .expect("the connect reaches the window");
    let taken = second.answer(shell, connect, "connect");
    let (failed, taken) = tool_result(&taken);
    assert!(!failed, "the window is taken over, not refused: {taken}");
    assert!(shell.has_visible_label(&connected_label(shell, "Codex")));
    assert!(!shell.has_visible_label(&connected_label(shell, "Claude Code")));
}

#[test]
fn connect_writes_the_settings_whose_server_drives_this_window_and_a_new_client_takes_over() {
    let mut clients = two_clients();
    // MECH-1, a request still queued: the window is not run while the first
    // client's save waits for it, so the save is queued but never begun. Its
    // user starts over with the second client. The save asks the window for a
    // frame once it is queued; the test waits for that, then for the connect
    // to be queued too, before the window runs again: it grants the attach
    // before it reads the save, as after a long load.
    let saved = clients.profile.path().join("frozen.ketchup");
    let before = clients.shell.app().live_bridge_stamp();
    // A background thread may ask for a frame just after the window settles.
    let quiet = poll_until(REQUEST_ARRIVAL, || {
        clients.shell.settle();
        !clients.shell.context().has_requested_repaint()
    });
    assert!(quiet, "the window keeps asking for frames");
    let save_queued = frame_requests(clients.shell.context());
    let save = clients.first.send(
        "tools/call",
        json!({"name": "file", "arguments": {"action": "save_as",
               "path": saved.to_str().unwrap(), "expected": clients.stamp}}),
    );
    save_queued
        .recv_timeout(REQUEST_ARRIVAL)
        .expect("the save reaches the window");
    take_over(&mut clients);
    let TwoClients {
        mut first,
        mut second,
        mut shell,
        ..
    } = clients;

    // The first agent learns its save never ran and what the document is now.
    let (failed, told) = tool_result(&first.answer(&mut shell, save, "file"));
    assert!(failed, "{told}");
    assert_eq!(told["error"], "connection_taken_over", "{told}");
    let message = told["message"].as_str().unwrap();
    assert!(message.contains("did not change"), "{message}");
    assert_eq!(told["details"]["stamp"], json!(before), "{told}");
    assert_eq!(shell.app().live_bridge_stamp(), before);
    assert!(!saved.exists(), "the save that waited never ran");

    let (failed, told) = first.call(&mut shell, "inspect", json!({"action": "status"}));
    assert!(failed);
    assert_eq!(told["error"], "connection_taken_over", "{told}");
    let (failed, status) = second.call(&mut shell, "inspect", json!({"action": "status"}));
    assert!(!failed, "{status}");
}

/// Two boxes 20 mm apart and the gap the program expects between them, which
/// only the exact check measures: the window publishes the program, then
/// answers once that check ends.
const GAP_PROGRAM: &str =
    "a = box('a', (10, 10, 10))\nb = box('b', (10, 10, 10), at = (30, 0, 0))\nexpect_gap(a, b, 20)";

#[test]
fn a_client_whose_program_the_window_published_but_never_answered_is_told_it_was_probably_applied()
{
    let mut clients = two_clients();
    // MECH-1, a request begun: the window plans and publishes the first
    // client's program, and is not run on while its answer waits for the
    // exact check, as if that check hung. Its user starts over with the
    // second client.
    let before = clients.shell.app().live_bridge_stamp();
    let apply = clients.first.send(
        "tools/call",
        json!({"name": "program", "arguments": {"action": "apply", "source": GAP_PROGRAM,
               "file_name": "gap.star", "replace_document": true}}),
    );
    let lines = &clients.first.lines;
    let mut early = None;
    let published = clients.shell.wait_until(|shell| {
        shell.step();
        while let Ok(message) = lines.try_recv() {
            if message["id"] == apply {
                early = Some(message);
            }
        }
        early.is_some() || shell.app().live_bridge_stamp() != before
    });
    assert!(published, "the window publishes the program");
    assert!(early.is_none(), "answered before the takeover: {early:?}");
    let after = clients.shell.app().live_bridge_stamp();
    take_over(&mut clients);
    let TwoClients {
        mut first,
        mut second,
        mut shell,
        ..
    } = clients;

    // The first agent learns its program probably went in, and the document now.
    let (failed, told) = tool_result(&first.answer(&mut shell, apply, "program"));
    assert!(failed, "the takeover came before the answer: {told}");
    assert_eq!(told["error"], "connection_taken_over", "{told}");
    let message = told["message"].as_str().unwrap();
    assert!(message.contains("probably applied"), "{message}");
    assert_eq!(told["details"]["stamp"], json!(after), "{told}");

    // It did: the second client reads the published program.
    let (failed, status) = second.call(&mut shell, "inspect", json!({"action": "status"}));
    assert!(!failed, "{status}");
    assert_eq!(status["stamp"], json!(after), "{status}");
    let (failed, read) = second.call(&mut shell, "program", json!({"action": "read"}));
    assert!(!failed, "{read}");
    assert!(read.to_string().contains("expect_gap"), "{read}");
}
