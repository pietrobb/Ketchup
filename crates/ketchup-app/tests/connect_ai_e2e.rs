//! #2687 (d): Connect AI end to end, from a profile no AI client has used
//! with Kečup: Connect writes the client's settings, the client starts the
//! server they name, and its tool call reaches this window. A second client
//! then takes the window over from the first, which stopped answering.
use crate::harness::Shell;
use ketchup_app::live_bridge::mcp_setup::{Environment, Harness, Registration};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
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

    /// Sends one request and runs the window until its answer arrives.
    fn ask(&mut self, shell: &mut Shell, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.input, "{request}").unwrap();
        let mut answer = None;
        let answered = shell.wait_until(|shell| {
            shell.step();
            while let Ok(message) = self.lines.try_recv() {
                if message["id"] == id {
                    answer = Some(message);
                }
            }
            answer.is_some()
        });
        assert!(answered, "no answer to {method}");
        answer.unwrap()
    }

    /// A tool call's result, as the agent reads it.
    fn call(&mut self, shell: &mut Shell, tool: &str, arguments: Value) -> (bool, Value) {
        let answer = self.ask(
            shell,
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
        );
        let result = &answer["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        (
            result["isError"] == true,
            serde_json::from_str(text).unwrap(),
        )
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn connected_label(shell: &Shell, client: &str) -> String {
    shell.catalog().format(
        "ai-connected",
        &BTreeMap::from([("client", client.to_owned())]),
    )
}

#[test]
fn connect_writes_the_settings_whose_server_drives_this_window_and_a_new_client_takes_over() {
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
    let context = eframe::egui::Context::default();
    shell
        .app_mut()
        .enable_live_consent_broker_in(&context, &runtime.join("Ketchup").join("live-instances"))
        .unwrap();

    let mut first = Client::start(&mut shell, &settings, &runtime, "claude-code");
    let (failed, connected) = first.call(&mut shell, "connect", json!({}));
    assert!(!failed, "{connected}");
    let (failed, status) = first.call(&mut shell, "inspect", json!({"action": "status"}));
    assert!(!failed, "{status}");
    assert_eq!(status["stamp"], json!(shell.app().live_bridge_stamp()));
    let named = connected_label(&shell, "Claude Code");
    assert!(
        shell.has_visible_label(&named),
        "the status line names the AI"
    );

    // The first client stops answering its user; a second one is started.
    let mut second = Client::start(&mut shell, &settings, &runtime, "codex");
    let (failed, windows) = second.call(&mut shell, "windows", json!({}));
    assert!(!failed, "{windows}");
    assert_eq!(
        windows["windows"][0]["used_by_another_client"], true,
        "{windows}"
    );
    assert!(windows["used_by_another_client"].is_string(), "{windows}");
    let (failed, taken) = second.call(&mut shell, "connect", json!({}));
    assert!(!failed, "the window is taken over, not refused: {taken}");
    assert!(shell.has_visible_label(&connected_label(&shell, "Codex")));
    assert!(!shell.has_visible_label(&named));

    let (failed, told) = first.call(&mut shell, "inspect", json!({"action": "status"}));
    assert!(failed);
    assert_eq!(told["error"], "connection_taken_over", "{told}");
    let (failed, status) = second.call(&mut shell, "inspect", json!({"action": "status"}));
    assert!(!failed, "{status}");
}
