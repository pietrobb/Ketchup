//! A plugin runs in the window through the bridge: the program it sends is
//! published like `apply_and_verify`, one Undo step. A plugin that fails, or
//! whose document changes while it runs, leaves the document as it was.
use super::*;
use crate::live_bridge::measurement::tests::integration_support::file_turn;
use ketchup_model::document::{FeatureParameterTarget, ParameterValueType};

pub(super) const EXTRUSION: u64 = 2;

pub(super) fn height(app: &KetchupApp) -> f64 {
    let target = FeatureParameterTarget::new(
        FeatureId(EXTRUSION),
        "extent.distance",
        ParameterValueType::Length,
    )
    .unwrap();
    app.document
        .current()
        .feature_parameter_value(&target)
        .unwrap()
}

/// A window whose user answers the plugin consent prompts from `dialogs`.
pub(super) fn plugin_wire_answering(dialogs: ScriptedFileDialogs) -> Wire {
    let mut wire = Wire::new();
    wire.app.dialogs = Box::new(dialogs);
    wire.app
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut wire.app);
    wire
}

/// A window whose user allows every plugin.
fn plugin_wire() -> Wire {
    plugin_wire_answering(ScriptedFileDialogs::new().always_confirm_high_risk_as(1))
}

/// The bridge takes only an absolute executable path.
pub(super) fn python() -> String {
    let output = std::process::Command::new(ketchup_test_env::python())
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn plugin_request(arguments: Vec<String>) -> RunPluginRequest {
    RunPluginRequest {
        executable: Some(python()),
        arguments,
        principal_id: Some(7001),
        capabilities: vec!["query.agent-state.v1".into(), "edit.cad-program.v1".into()],
        allowed_operations: vec!["set_dimension".into()],
        timeout_ms: 30_000,
        ..RunPluginRequest::default()
    }
}

fn run_plugin(arguments: Vec<String>) -> Request {
    Request::RunPlugin(plugin_request(arguments))
}

fn example_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdk/python/examples/dimension_plugin.py")
}

/// The Python SDK's example plugin, setting the extrusion height to `height_mm`.
fn example(height_mm: &str) -> Request {
    run_plugin(vec![
        example_script().to_string_lossy().into_owned(),
        EXTRUSION.to_string(),
        height_mm.to_owned(),
    ])
}

#[test]
fn the_example_plugin_changes_the_height_in_one_undo_step() {
    let _turn = file_turn();
    let mut wire = plugin_wire();
    let before = wire.app.live_bridge_stamp();
    let undo = wire.app.undo_step_count();
    assert_ne!(height(&wire.app), 35.0);

    let response = wire.call_within(example("35"), Duration::from_secs(120));
    let result = response.result.unwrap();
    assert_eq!(result["published"], true, "{result}");
    assert_eq!(result["before"], json!(before));
    assert_eq!(result["after"], json!(wire.app.live_bridge_stamp()));
    assert_eq!(height(&wire.app), 35.0);
    assert_eq!(wire.app.undo_step_count(), undo + 1);

    wire.call(Request::Undo { expected: None });
    assert_eq!(
        wire.app.document.current().canonical_digest(),
        before.canonical_digest
    );
    assert_eq!(wire.app.undo_step_count(), undo);
}

#[test]
fn a_crashing_plugin_returns_its_cause_and_changes_nothing() {
    let _turn = file_turn();
    let mut wire = plugin_wire();
    let before = wire.app.live_bridge_stamp();
    let undo = wire.app.undo_step_count();

    let response = wire.answer_within(
        run_plugin(vec!["-c".into(), "import sys; sys.exit(3)".into()]),
        Duration::from_secs(30),
    );
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("plugin_failed"));
    assert_eq!(
        response.result.unwrap()["causes"],
        json!([ketchup_scheduler::plugin::PluginHostError::ExitedBeforeDone.to_string()])
    );
    assert_eq!(wire.app.live_bridge_stamp(), before);
    assert_eq!(wire.app.undo_step_count(), undo);
}

#[test]
fn a_plugin_whose_document_changes_while_it_runs_is_refused() {
    let _turn = file_turn();
    let mut wire = plugin_wire();
    let original = height(&wire.app);
    let undo = wire.app.undo_step_count();
    wire.send(example("35"), false);
    let mut reader = wire.stream.try_clone().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader_thread = std::thread::spawn(move || {
        let mut header = [0; 4];
        reader.read_exact(&mut header).unwrap();
        let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
        reader.read_exact(&mut bytes).unwrap();
        tx.send(serde_json::from_slice::<Response>(&bytes).unwrap())
            .unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(4);
    while !wire.app.live.bridge.as_ref().is_some_and(|bridge| {
        matches!(
            bridge.program_check_job,
            Some(program_check::ProgramCheckJob::Plugin(_))
        )
    }) {
        assert!(Instant::now() < deadline, "the plugin did not start");
        wire.app.poll_live_bridge(&wire.context);
    }

    // A human edit while the plugin runs.
    wire.app
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: false,
            },
        ]))
        .unwrap();
    let edited = wire.app.live_bridge_stamp();

    let deadline = Instant::now() + Duration::from_secs(30);
    let response = loop {
        if let Ok(response) = rx.recv_timeout(Duration::from_millis(5)) {
            break response;
        }
        assert!(Instant::now() < deadline, "no answer to the stale plugin");
        wire.app.poll_live_bridge(&wire.context);
    };
    reader_thread.join().unwrap();
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("stale_document"));
    assert_eq!(wire.app.live_bridge_stamp(), edited);
    assert_eq!(height(&wire.app), original);
    assert_eq!(wire.app.undo_step_count(), undo + 1);
}

/// Refused before it starts: the document, its stamp and Undo stay as they were.
fn assert_consent_required(wire: &mut Wire, request: RunPluginRequest) -> Value {
    let before = wire.app.live_bridge_stamp();
    let undo = wire.app.undo_step_count();
    let response = wire.answer_within(Request::RunPlugin(request), Duration::from_secs(30));
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("plugin_consent_required"));
    assert_eq!(wire.app.live_bridge_stamp(), before);
    assert_eq!(wire.app.undo_step_count(), undo);
    response.result.unwrap()["details"]["plugin"].clone()
}

#[test]
fn a_plugin_runs_only_with_consent_bound_to_its_file_hash_and_permissions() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let wrapper = directory.path().join("height_plugin.py");
    let example = example_script();
    let source = format!(
        "import runpy, sys\nsys.argv = [r\"{0}\"] + sys.argv[1:]\nrunpy.run_path(r\"{0}\", run_name=\"__main__\")\n",
        example.display()
    );
    std::fs::write(&wrapper, &source).unwrap();
    let hash = ketchup_model::graph::sha256_hex(source.as_bytes());
    let request = || {
        plugin_request(vec![
            wrapper.to_string_lossy().into_owned(),
            EXTRUSION.to_string(),
            "35".to_owned(),
        ])
    };
    // Refuse, allow, then refuse every further prompt.
    let dialogs = ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval(1);
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    let original = height(&wire.app);
    let undo = wire.app.undo_step_count();

    // A network path is refused before it is read or shown.
    if cfg!(windows) {
        let response = wire.answer_within(
            run_plugin(vec![r"\\server\share\plugin.py".into()]),
            Duration::from_secs(30),
        );
        assert_eq!(response.error.as_deref(), Some("invalid_path"));
        assert!(probe.high_risk_prompts().is_empty());
    }

    // Without consent nothing runs; the window showed path, hash and permissions.
    let evidence = assert_consent_required(&mut wire, request());
    let prompts = probe.high_risk_prompts();
    assert_eq!(prompts.len(), 1);
    for shown in [
        wrapper.display().to_string(),
        format!("SHA-256 {hash}"),
        "edit.cad-program.v1".to_owned(),
        "set_dimension".to_owned(),
    ] {
        assert!(prompts[0].contains(&shown), "{shown} not in {}", prompts[0]);
    }
    assert!(
        evidence["files"]
            .as_array()
            .unwrap()
            .contains(&json!({"path": wrapper.display().to_string(), "sha256": hash})),
        "{evidence}"
    );

    // Allowed: the plugin runs and publishes one Undo step.
    let response = wire.call_within(Request::RunPlugin(request()), Duration::from_secs(120));
    assert_eq!(response.result.unwrap()["published"], true);
    assert_eq!(height(&wire.app), 35.0);
    assert_eq!(wire.app.undo_step_count(), undo + 1);
    assert_eq!(probe.high_risk_prompts().len(), 2);
    wire.call(Request::Undo { expected: None });
    assert_eq!(height(&wire.app), original);

    // The same file with the same permissions runs without asking again.
    wire.call_within(Request::RunPlugin(request()), Duration::from_secs(120));
    assert_eq!(height(&wire.app), 35.0);
    assert_eq!(probe.high_risk_prompts().len(), 2);
    wire.call(Request::Undo { expected: None });

    // One more permission than allowed asks again.
    let mut wider = request();
    wider.allowed_operations.push("set_color".into());
    assert_consent_required(&mut wire, wider);
    assert_eq!(probe.high_risk_prompts().len(), 3);

    // A changed plugin file asks again, even with the same permissions.
    std::fs::write(&wrapper, format!("{source}# changed\n")).unwrap();
    let evidence = assert_consent_required(&mut wire, request());
    assert_eq!(probe.high_risk_prompts().len(), 4);
    assert_ne!(evidence["files"][1]["sha256"], json!(hash));
    assert_eq!(height(&wire.app), original);
    assert_eq!(wire.app.undo_step_count(), undo);
}

const FOLDER_PLUGIN: &str = r#"import sys
from ketchup_sdk import Client, Manifest, EDIT_CAD_PROGRAM, QUERY_AGENT_STATE

if sys.argv[1:]:
    raise SystemExit("inputs arrive by protocol only")
client = Client(Manifest(package="org.ketchup.height", version="1.0.0", principal_id=7001,
                         capabilities=(QUERY_AGENT_STATE, EDIT_CAD_PROGRAM)))
client.start()
inputs = client.inputs()
client.query_agent_state()
client.apply_operations([{"operation": "set_dimension", "feature_id": inputs["feature_id"],
                          "value_mm": inputs["height_mm"]}])
client.finish()
"#;

fn folder_manifest(operations: &[&str]) -> String {
    json!({
        "name": "Height",
        "executable": python(),
        "arguments": ["plugin.py"],
        "principal_id": 7001,
        "capabilities": ["query.agent-state.v1", "edit.cad-program.v1"],
        "allowed_operations": operations,
        "inputs": {
            "height_mm": {"type": "number"},
            "feature_id": {"type": "integer", "default": EXTRUSION},
        },
    })
    .to_string()
}

pub(super) fn folder_request(folder: &Path, inputs: Value) -> RunPluginRequest {
    RunPluginRequest {
        plugin: Some(folder.to_string_lossy().into_owned()),
        inputs: inputs.as_object().cloned(),
        timeout_ms: 30_000,
        ..RunPluginRequest::default()
    }
}

/// Refused before anything was read into a prompt or ran.
fn assert_refused_without_prompt(
    wire: &mut Wire,
    request: RunPluginRequest,
    code: &str,
    prompts: &ScriptedFileDialogs,
) {
    let before = wire.app.live_bridge_stamp();
    let asked = prompts.high_risk_prompts().len();
    let response = wire.answer_within(Request::RunPlugin(request), Duration::from_secs(30));
    assert_eq!(
        response.error.as_deref(),
        Some(code),
        "{:?}",
        response.result
    );
    assert_eq!(prompts.high_risk_prompts().len(), asked);
    assert_eq!(wire.app.live_bridge_stamp(), before);
}

/// `root/height`: a plugin folder setting the extrusion height to its input
/// `height_mm`, with its own copy of the SDK. Returns the folder and the SDK source.
pub(super) fn height_folder(root: &Path) -> (PathBuf, String) {
    let folder = root.join("height");
    let sdk = folder.join("ketchup_sdk/__init__.py");
    std::fs::create_dir_all(sdk.parent().unwrap()).unwrap();
    let sdk_source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdk/python/ketchup_sdk/__init__.py"),
    )
    .unwrap();
    std::fs::write(&sdk, &sdk_source).unwrap();
    std::fs::write(folder.join("plugin.py"), FOLDER_PLUGIN).unwrap();
    std::fs::write(
        folder.join("plugin.json"),
        folder_manifest(&["set_dimension"]),
    )
    .unwrap();
    (folder, sdk_source)
}

#[test]
fn a_plugin_folder_is_allowed_with_all_its_files_and_takes_inputs_only_by_protocol() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, sdk_source) = height_folder(directory.path());
    let sdk = folder.join("ketchup_sdk/__init__.py");
    let python = python();
    let python_sha256 = std::fs::File::open(&python)
        .and_then(ketchup_model::graph::sha256_reader_hex)
        .unwrap();
    // Refuse, allow, then refuse every further prompt.
    let dialogs = ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval(1);
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    let original = height(&wire.app);
    let undo = wire.app.undo_step_count();

    // The manifest alone decides the command line and the permissions.
    let mut extended = folder_request(&folder, json!({"height_mm": 35}));
    extended.arguments = vec!["-c".into(), "print()".into()];
    assert_refused_without_prompt(&mut wire, extended, "invalid_params", &probe);
    // Only declared inputs of the declared type.
    for inputs in [
        json!({"height_mm": 35, "width_mm": 10}),
        json!({"height_mm": "35"}),
        json!({}),
    ] {
        let request = folder_request(&folder, inputs);
        assert_refused_without_prompt(&mut wire, request, "plugin_manifest_invalid", &probe);
    }

    // The prompt shows the folder with one hash over all its files, and the
    // interpreter outside it with its own hash.
    let evidence =
        assert_consent_required(&mut wire, folder_request(&folder, json!({"height_mm": 35})));
    let prompt = &probe.high_risk_prompts()[0];
    let folder_sha256 = evidence["folder"]["sha256"].as_str().unwrap().to_owned();
    for shown in [
        folder.display().to_string(),
        format!("SHA-256 {folder_sha256} (3)"),
        python.clone(),
        format!("SHA-256 {python_sha256}"),
        "set_dimension".to_owned(),
    ] {
        assert!(prompt.contains(&shown), "{shown} not in {prompt}");
    }
    assert_eq!(
        evidence["folder"]["files"],
        json!([
            {"path": "ketchup_sdk/__init__.py", "sha256": ketchup_model::graph::sha256_hex(sdk_source.as_bytes())},
            {"path": "plugin.json", "sha256": ketchup_model::graph::sha256_hex(folder_manifest(&["set_dimension"]).as_bytes())},
            {"path": "plugin.py", "sha256": ketchup_model::graph::sha256_hex(FOLDER_PLUGIN.as_bytes())},
        ])
    );
    assert_eq!(
        evidence["files"],
        json!([{"path": python, "sha256": python_sha256}])
    );

    // Allowed: the input reaches the plugin by protocol, one Undo step.
    let response = wire.call_within(
        Request::RunPlugin(folder_request(&folder, json!({"height_mm": 35}))),
        Duration::from_secs(120),
    );
    assert_eq!(response.result.unwrap()["published"], true);
    assert_eq!(height(&wire.app), 35.0);
    assert_eq!(wire.app.undo_step_count(), undo + 1);
    wire.call(Request::Undo { expected: None });
    assert_eq!(height(&wire.app), original);

    // Another input value is no new consent; the run wrote nothing into the folder.
    wire.call_within(
        Request::RunPlugin(folder_request(&folder, json!({"height_mm": 40.5}))),
        Duration::from_secs(120),
    );
    assert_eq!(height(&wire.app), 40.5);
    assert_eq!(probe.high_risk_prompts().len(), 2);
    wire.call(Request::Undo { expected: None });

    // A changed module inside the folder asks again.
    std::fs::write(&sdk, format!("{sdk_source}# changed\n")).unwrap();
    let changed =
        assert_consent_required(&mut wire, folder_request(&folder, json!({"height_mm": 35})));
    assert_eq!(probe.high_risk_prompts().len(), 3);
    assert_ne!(changed["folder"]["sha256"], json!(folder_sha256));
    std::fs::write(&sdk, &sdk_source).unwrap();

    // A manifest asking for one more operation asks again.
    std::fs::write(
        folder.join("plugin.json"),
        folder_manifest(&["set_dimension", "set_color"]),
    )
    .unwrap();
    assert_consent_required(&mut wire, folder_request(&folder, json!({"height_mm": 35})));
    assert_eq!(probe.high_risk_prompts().len(), 4);
    assert_eq!(height(&wire.app), original);
    assert_eq!(wire.app.undo_step_count(), undo);
}

/// SEC-2: a plugin file changed while the consent prompt is open does not run;
/// the folder's copy the prompt hashed does, and the changed folder asks again.
#[test]
fn a_plugin_folder_runs_the_files_the_user_allowed_not_files_changed_while_asking() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let swapped = FOLDER_PLUGIN.replace("inputs[\"height_mm\"]", "99");
    let dialogs = ScriptedFileDialogs::new().queue_high_risk_approval_after_write_attempt(
        1,
        folder.join("plugin.py"),
        swapped.clone(),
    );
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    let original = height(&wire.app);

    let response = wire.call_within(
        Request::RunPlugin(folder_request(&folder, json!({"height_mm": 35}))),
        Duration::from_secs(120),
    );
    assert_eq!(response.result.unwrap()["published"], true);
    // The folder's own file changed while asking, the allowed copy ran.
    assert_eq!(probe.high_risk_write_attempts(), [true]);
    assert_eq!(
        std::fs::read_to_string(folder.join("plugin.py")).unwrap(),
        swapped
    );
    assert_eq!(height(&wire.app), 35.0);
    wire.call(Request::Undo { expected: None });

    // The changed folder is not the allowed one.
    assert_consent_required(&mut wire, folder_request(&folder, json!({"height_mm": 35})));
    assert_eq!(probe.high_risk_prompts().len(), 2);
    assert_eq!(height(&wire.app), original);
}

/// SEC-2: a script outside a plugin folder cannot be changed from its hash in
/// the consent prompt until the plugin has ended; then it can again.
#[cfg(windows)]
#[test]
fn a_plugin_script_cannot_change_between_its_consent_and_the_end_of_its_run() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let wrapper = directory.path().join("height_plugin.py");
    let wrapped = |height: &str| {
        format!(
            "import runpy, sys\nsys.argv = [r\"{0}\", sys.argv[1], \"{height}\"]\nrunpy.run_path(r\"{0}\", run_name=\"__main__\")\n",
            example_script().display()
        )
    };
    std::fs::write(&wrapper, wrapped("35")).unwrap();
    let dialogs = ScriptedFileDialogs::new().queue_high_risk_approval_after_write_attempt(
        1,
        &wrapper,
        wrapped("99"),
    );
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    let request = plugin_request(vec![
        wrapper.to_string_lossy().into_owned(),
        EXTRUSION.to_string(),
    ]);

    let response = wire.call_within(Request::RunPlugin(request), Duration::from_secs(120));
    assert_eq!(response.result.unwrap()["published"], true);
    assert_eq!(probe.high_risk_write_attempts(), [false]);
    assert_eq!(height(&wire.app), 35.0);
    // Released with the run.
    std::fs::write(&wrapper, wrapped("99")).unwrap();
}

/// SEC-3: a name or argument with line breaks and bidirectional controls
/// cannot add or reorder lines of the consent prompt.
#[test]
fn a_plugin_name_or_argument_cannot_forge_lines_of_the_consent_prompt() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    manifest["name"] = json!("Height\nPlugin folder, every file under it:\nC:\\Safe\u{202E}yp.");
    manifest["arguments"] = json!([
        "plugin.py",
        "a b",
        "x\r\nCommand line: C:\\Safe\\safe.exe\u{2066}"
    ]);
    std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
    let dialogs = ScriptedFileDialogs::new();
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    assert_consent_required(&mut wire, folder_request(&folder, json!({"height_mm": 35})));

    let prompt = &probe.high_risk_prompts()[0];
    let starting = |label: &str| {
        prompt
            .lines()
            .filter(|line| line.starts_with(label))
            .count()
    };
    assert_eq!(starting("Plugin:"), 1, "{prompt}");
    assert_eq!(starting("Plugin folder"), 1, "{prompt}");
    assert_eq!(starting("Command line:"), 1, "{prompt}");
    assert!(
        prompt.chars().all(|c| c == '\n' || !c.is_control())
            && !prompt.contains(['\u{202E}', '\u{2066}']),
        "{prompt:?}"
    );
    for shown in [
        "Plugin: Height\\u{000a}Plugin folder, every file under it:\\u{000a}C:\\Safe\\u{202e}yp.",
        " plugin.py \"a b\" \"x\\u{000d}\\u{000a}Command line: C:\\Safe\\safe.exe\\u{2066}\"",
    ] {
        assert!(prompt.contains(shown), "{shown} not in {prompt}");
    }
}

/// A directory link: a junction on Windows (no privilege needed), a symlink elsewhere.
fn link_directory(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn a_link_in_or_as_a_plugin_folder_is_refused_without_a_prompt() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let elsewhere = directory.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("module.py"), "x = 1\n").unwrap();
    let dialogs = ScriptedFileDialogs::new().always_confirm_high_risk_as(1);
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    let original = height(&wire.app);
    let undo = wire.app.undo_step_count();

    // A link inside the folder would bring in files the consent never hashed.
    link_directory(&folder.join("linked"), &elsewhere);
    let request = folder_request(&folder, json!({"height_mm": 35}));
    assert_refused_without_prompt(
        &mut wire,
        request.clone(),
        "plugin_manifest_invalid",
        &probe,
    );
    let response = wire.answer_within(Request::RunPlugin(request), Duration::from_secs(30));
    let reason = response.result.unwrap()["reason"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(reason.starts_with("linked is a link"), "{reason}");
    std::fs::remove_dir(folder.join("linked")).unwrap();

    // So would a link naming the plugin folder itself.
    let alias = directory.path().join("alias");
    link_directory(&alias, &folder);
    let request = folder_request(&alias, json!({"height_mm": 35}));
    assert_refused_without_prompt(&mut wire, request, "plugin_manifest_invalid", &probe);

    assert!(probe.high_risk_prompts().is_empty());
    assert_eq!(height(&wire.app), original);
    assert_eq!(wire.app.undo_step_count(), undo);
}

/// SEC-1/PR-N1: a manifest argument naming a share, however its separators are
/// mixed, is refused before anything asks the share whether the file exists.
#[cfg(windows)]
#[test]
fn a_plugin_argument_on_a_network_share_is_refused_without_a_prompt() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let dialogs = ScriptedFileDialogs::new().always_confirm_high_risk_as(1);
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    for share in [
        r"\\server.invalid\share\module.py",
        r"\/server.invalid\share\module.py",
        r"/\server.invalid\share\module.py",
    ] {
        let mut manifest: Value =
            serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
        manifest["arguments"] = json!(["plugin.py", share]);
        std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
        let request = folder_request(&folder, json!({"height_mm": 35}));
        assert_refused_without_prompt(&mut wire, request, "invalid_path", &probe);
    }
    assert!(probe.high_risk_prompts().is_empty());
}

/// SEC-1: an argument that leaves the folder and comes back into it names the
/// folder's file the consent hashed, and the process gets that file's staged
/// copy, not a path that leads out of the stage it runs in.
#[test]
fn a_plugin_argument_through_dot_dot_runs_the_staged_file_it_names() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    for argument in [
        "../height/plugin.py".to_owned(),
        folder.join("../height/plugin.py").display().to_string(),
    ] {
        manifest["arguments"] = json!([argument]);
        std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
        let inputs = json!({"height_mm": 35});
        let launch = crate::live_bridge::plugin_manifest::load(
            &folder.to_string_lossy(),
            inputs.as_object(),
        )
        .unwrap();
        let stage = launch.command.working_directory.clone().unwrap();
        let given = Path::new(&launch.command.arguments[0]);
        assert!(
            given
                .components()
                .all(|component| component != std::path::Component::ParentDir),
            "{argument:?} reaches the process as {given:?}"
        );
        assert_eq!(stage.join(given), stage.join("plugin.py"), "{argument:?}");
    }
}
