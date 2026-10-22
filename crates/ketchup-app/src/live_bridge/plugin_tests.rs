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
    let mut app = grounded_app();
    app.dialogs = Box::new(dialogs);
    Wire::connect(app)
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

    let directory = tempfile::tempdir().unwrap();
    let crash = directory.path().join("crash.py");
    std::fs::write(&crash, "import sys\nsys.exit(3)\n").unwrap();
    let response = wire.answer_within(
        run_plugin(vec![crash.display().to_string()]),
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
    let dialogs = ScriptedFileDialogs::new();
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    // An argument naming no file is plain words (SEC-7), so a forged line
    // there is refused before any prompt; a file name may hold a space.
    manifest["arguments"] = json!(["plugin.py", "x\r\nCommand line: C:\\Safe\\safe.exe\u{2066}"]);
    std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
    let request = folder_request(&folder, json!({"height_mm": 35}));
    assert_refused_without_prompt(&mut wire, request, "plugin_manifest_invalid", &probe);
    std::fs::write(folder.join("a b"), "").unwrap();
    manifest["arguments"] = json!(["plugin.py", "a b"]);
    std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
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
        " plugin.py \"a b\"",
    ] {
        assert!(prompt.contains(shown), "{shown} not in {prompt}");
    }
}

/// Review 2026-10-17 P3: inputs larger than the `INPUTS` line can carry are
/// refused, naming the input, before the user is asked; they used to fail
/// only after consent, once the plugin had started and asked for them.
#[test]
fn inputs_too_large_for_the_protocol_line_are_refused_by_name_before_the_prompt() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    manifest["inputs"]["label"] = json!({"type": "string"});
    std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
    let dialogs = ScriptedFileDialogs::new();
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    // A connection's first frame, before its token is checked, must be small.
    wire.call(Request::Status {});
    // What the host sends: the given inputs plus the defaults.
    let empty = json!({"feature_id": EXTRUSION, "height_mm": 35, "label": ""});
    let fitting = ketchup_scheduler::plugin::MAX_PLUGIN_INPUTS_BYTES - empty.to_string().len();
    let request = |length: usize| {
        folder_request(
            &folder,
            json!({"height_mm": 35, "label": "x".repeat(length)}),
        )
    };

    assert_refused_without_prompt(
        &mut wire,
        request(fitting + 1),
        "plugin_manifest_invalid",
        &probe,
    );
    let response = wire.answer_within(
        Request::RunPlugin(request(fitting + 1)),
        Duration::from_secs(30),
    );
    let reason = response.result.unwrap()["reason"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(reason.starts_with("input \"label\""), "{reason}");
    assert!(probe.high_risk_prompts().is_empty());

    // Inputs exactly at the limit still reach the prompt.
    assert_consent_required(&mut wire, request(fitting));
    assert_eq!(probe.high_risk_prompts().len(), 1);
}

/// docs/plugins.md names every interpreter and every option it may get before
/// its script, as the window lets them pass (review 2026-10-18 SEC-8).
#[test]
fn plugins_doc_lists_every_option_allowed_before_a_script() {
    use crate::live_bridge::plugin_manifest::SCRIPT_OPTIONS;
    let doc = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/plugins.md"),
    )
    .unwrap();
    for (names, options, _) in SCRIPT_OPTIONS {
        for word in names.iter().chain(options) {
            let word = format!("`{word}`");
            assert!(doc.contains(&word), "docs/plugins.md does not list {word}");
        }
    }
}

/// Review 2026-10-17 documentation: docs/plugins.md did not give the shapes a
/// plugin argument may take. Each shape it names is the one the window passes,
/// and each refusal it names is refused.
#[test]
fn plugins_doc_gives_the_argument_shapes_the_window_passes() {
    use crate::live_bridge::plugin_manifest::{plain_value, refused_shape};
    let doc = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/plugins.md"),
    )
    .unwrap();
    let shapes = [
        ("`-x`", "-B", true),
        ("`-xVALUE`", "-D1", true),
        ("`--name`", "--verbose", true),
        ("`--name=VALUE`", "--level=2", true),
        ("`name=VALUE`", "mode=fast", true),
        ("`@`", "@args.txt", false),
        ("a root", "--out=/etc", false),
        ("`..`", "--out=x/..", false),
    ];
    for (named, example, passes) in shapes {
        assert!(doc.contains(named), "docs/plugins.md does not name {named}");
        assert_eq!(refused_shape(example).is_none(), passes, "{example}");
    }
    assert!(doc.contains("`-_.,+/\\`"));
    assert!(plain_value("a-b_c.d,e+f/g\\h") && !plain_value("exec(x)"));
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

/// SEC-2 (review 2026-10-15): Windows starts `tool.exe` for `tool`, so that is
/// the file hashed and shown; an executable that is no `.exe` file, or no file
/// at all, is refused before anyone is asked.
#[cfg(windows)]
#[test]
fn a_windows_plugin_executable_is_the_exe_file_that_starts() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let decoy = directory.path().join("tool");
    let started = directory.path().join("tool.exe");
    std::fs::write(&decoy, "the file a reviewer would read").unwrap();
    std::fs::write(&started, "the program Windows starts").unwrap();
    let started_hash = ketchup_model::graph::sha256_hex(b"the program Windows starts");
    let script = directory.path().join("tool.cmd");
    std::fs::write(&script, "@exit 0\n").unwrap();
    let missing = directory.path().join("missing.exe");
    // Every prompt is refused: nothing here may start.
    let dialogs = ScriptedFileDialogs::new();
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    // A command line names a hashed script besides its executable (SEC-7).
    let command = |executable: &Path| RunPluginRequest {
        executable: Some(executable.display().to_string()),
        ..plugin_request(vec![script.display().to_string()])
    };
    let (folder, _) = height_folder(directory.path());
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    let mut from_folder = |executable: &Path| {
        manifest["executable"] = json!(executable.display().to_string());
        std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
        folder_request(&folder, json!({"height_mm": 35}))
    };

    for request in [command(&decoy), from_folder(&decoy)] {
        let evidence = assert_consent_required(&mut wire, request);
        let shown = json!({"path": started.display().to_string(), "sha256": started_hash});
        assert!(
            evidence["files"].as_array().unwrap().contains(&shown),
            "{evidence}"
        );
        assert_eq!(evidence["command"][0], json!(started.display().to_string()));
        let prompt = probe.high_risk_prompts().pop().unwrap();
        assert!(prompt.contains(&started.display().to_string()), "{prompt}");
    }
    let asked = probe.high_risk_prompts().len();
    for executable in [&script, &missing, &directory.path().join("absent")] {
        assert_refused_without_prompt(&mut wire, command(executable), "invalid_path", &probe);
        let request = from_folder(executable);
        assert_refused_without_prompt(&mut wire, request, "invalid_path", &probe);
    }
    assert_eq!(probe.high_risk_prompts().len(), asked);
}

/// SEC-3 (review 2026-10-15): a folder without a manifest is refused before
/// its files are walked, copied or hashed.
#[test]
fn a_folder_without_a_manifest_is_refused_before_its_files_are_read() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("not-a-plugin");
    std::fs::create_dir(&folder).unwrap();
    for index in 0..1_001 {
        std::fs::write(folder.join(format!("{index}.txt")), "").unwrap();
    }
    let probe = ScriptedFileDialogs::new();
    let mut wire = plugin_wire_answering(probe.clone());
    let request = folder_request(&folder, json!({}));
    let response = wire.answer_within(Request::RunPlugin(request), Duration::from_secs(30));
    assert_eq!(response.error.as_deref(), Some("plugin_manifest_invalid"));
    let result = response.result.unwrap();
    assert_eq!(
        result["reason"], "the folder has no plugin.json; it is no plugin folder",
        "{result}"
    );
    assert!(probe.high_risk_prompts().is_empty());
}

/// SEC-3 (review 2026-10-15): the Plugins menu lists a folder from its
/// manifest and file names on the UI thread; it reads none of the other files,
/// so one held open exclusively by another program does not stop the listing.
#[cfg(windows)]
#[test]
fn the_plugins_menu_lists_a_folder_without_reading_its_files() {
    use std::os::windows::fs::OpenOptionsExt;
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let data = folder.join("data.bin");
    std::fs::write(&data, "data").unwrap();
    let exclusive = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&data)
        .unwrap();
    take_error_details();
    let listed = crate::live_bridge::plugin_manifest::describe(&folder);
    let details = take_error_details();
    drop(exclusive);
    assert_eq!(
        listed.map(|description| description.name).ok(),
        Some("Height".to_owned()),
        "{details:?}"
    );
}

/// SEC-3 (review 2026-10-15): the plugin folder is read, staged and hashed on
/// a worker thread; the window keeps drawing frames meanwhile.
#[test]
fn a_plugin_folder_is_read_staged_and_hashed_off_the_ui_thread() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let probe = ScriptedFileDialogs::new();
    let mut wire = plugin_wire_answering(probe.clone());
    let (release, gate) = mpsc::channel::<()>();
    *crate::live_bridge::plugin::LAUNCH_GATE.lock().unwrap() =
        Some((folder.to_string_lossy().into_owned(), gate));
    let original = height(&wire.app);
    wire.send(
        Request::RunPlugin(folder_request(&folder, json!({"height_mm": 35}))),
        false,
    );
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

    // While the folder cannot be read every frame returns: one that waited for
    // the gate, opened below, would never end.
    for _ in 0..50 {
        wire.app.poll_live_bridge(&wire.context);
        assert!(rx.recv_timeout(Duration::from_millis(5)).is_err());
    }
    assert!(wire.app.live.bridge.as_ref().is_some_and(|bridge| matches!(
        bridge.program_check_job,
        Some(program_check::ProgramCheckJob::PluginPreparing(_))
    )));

    // Once read, the user is asked; refused, nothing ran.
    drop(release);
    let deadline = Instant::now() + Duration::from_secs(30);
    let response = loop {
        if let Ok(response) = rx.recv_timeout(Duration::from_millis(5)) {
            break response;
        }
        assert!(
            Instant::now() < deadline,
            "no answer after the folder was read"
        );
        wire.app.poll_live_bridge(&wire.context);
    };
    reader_thread.join().unwrap();
    assert_eq!(response.error.as_deref(), Some("plugin_consent_required"));
    assert_eq!(probe.high_risk_prompts().len(), 1);
    assert_eq!(height(&wire.app), original);
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
            &Default::default(),
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

/// Review 2026-10-15 N-8: `..` joined to an option (`--script=../x`) is no
/// path component of the argument, so it is not resolved to a hashed file;
/// the plugin would open whatever it names outside its stage. It is refused.
#[test]
fn a_dot_dot_joined_to_a_plugin_option_is_refused() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    for argument in [
        "--script=../height/plugin.py",
        "--script=..\\height\\plugin.py",
        "-s../height/plugin.py",
        "--dir=..",
        // Review 2026-10-16: resolving `..` dropped the option with its name,
        // and the plugin got only the folder's plugin.py.
        "--s=a/../plugin.py",
        "-sa/../plugin.py",
        "script=a\\..\\plugin.py",
    ] {
        manifest["arguments"] = json!(["plugin.py", argument]);
        std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
        let inputs = json!({"height_mm": 35});
        let launch = crate::live_bridge::plugin_manifest::load(
            &folder.to_string_lossy(),
            inputs.as_object(),
            &Default::default(),
        );
        assert_eq!(
            launch.err(),
            Some("plugin_manifest_invalid"),
            "{argument:?}"
        );
    }
    for argument in ["--range=1..5", "--script=plugin.py"] {
        manifest["arguments"] = json!(["plugin.py", argument]);
        std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
        let inputs = json!({"height_mm": 35});
        let launch = crate::live_bridge::plugin_manifest::load(
            &folder.to_string_lossy(),
            inputs.as_object(),
            &Default::default(),
        );
        assert!(launch.is_ok(), "{argument:?}");
    }
}

/// Review 2026-10-15 N-9: a path outside the folder where no file is yet was
/// checked only at the start, and another process could create it while the
/// plugin ran, which then opened it unhashed. Such a path is refused before
/// anyone is asked, from a plugin folder and from a command line.
#[test]
fn a_plugin_path_outside_where_no_file_is_yet_is_refused_without_a_prompt() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let later = directory.path().join("later.py");
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    manifest["arguments"] = json!(["plugin.py", later.display().to_string()]);
    std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
    let dialogs = ScriptedFileDialogs::new().always_confirm_high_risk_as(1);
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    let request = folder_request(&folder, json!({"height_mm": 35}));
    assert_refused_without_prompt(&mut wire, request, "invalid_path", &probe);

    for named in [later.clone(), directory.path().join("sub/../later.py")] {
        let command = plugin_request(vec![
            example_script().to_string_lossy().into_owned(),
            named.display().to_string(),
        ]);
        assert_refused_without_prompt(&mut wire, command, "invalid_path", &probe);
    }
    assert!(probe.high_risk_prompts().is_empty());
}

/// Review 2026-10-15 N-9: a relative argument of a command line that names
/// no file resolved against the window's working directory, where a file
/// could appear later. The process runs in an empty private directory, which
/// must still be empty when it starts; a relative argument naming a file is
/// given as the absolute path of the file that was hashed.
#[test]
fn a_command_lines_relative_arguments_resolve_in_an_empty_private_directory() {
    let _turn = file_turn();
    let here = std::env::current_dir().unwrap();
    let script = example_script();
    let relative = pathdiff_relative(&script, &here);
    let request = plugin_request(vec![relative.display().to_string(), "later.py".to_owned()]);
    let launch = crate::live_bridge::plugin::launch(&request, &Default::default()).unwrap();
    let directory = launch
        .command
        .working_directory
        .clone()
        .expect("the process runs in its own directory");
    assert_ne!(directory, here);
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
    assert_eq!(launch.command.arguments[1], "later.py");
    let given = Path::new(&launch.command.arguments[0]);
    assert!(given.is_absolute());
    assert!(
        given
            .components()
            .all(|component| component != std::path::Component::ParentDir),
        "{given:?}"
    );
    assert_eq!(
        std::fs::canonicalize(given).unwrap(),
        std::fs::canonicalize(&script).unwrap()
    );
    launch.held.unchanged().unwrap();
    std::fs::write(directory.join("later.py"), "raise SystemExit\n").unwrap();
    assert!(launch.held.unchanged().is_err());
}

/// Review 2026-10-15 structural point 7: a command line's words are resolved
/// by the rule a plugin folder's are. `..` joined to an option is no path
/// component, so it reached the process as given and the plugin opened a file
/// beside its empty private directory that nobody hashed.
#[test]
fn a_dot_dot_joined_to_an_option_of_a_command_line_is_refused() {
    let _turn = file_turn();
    let script = example_script().to_string_lossy().into_owned();
    for argument in [
        "--script=../plugin.py",
        "--script=..\\plugin.py",
        "-s../plugin.py",
        "--dir=..",
        "--s=a/../plugin.py",
        "-sa/../plugin.py",
    ] {
        let request = plugin_request(vec![script.clone(), argument.to_owned()]);
        assert_eq!(
            crate::live_bridge::plugin::launch(&request, &Default::default()).err(),
            Some("invalid_path"),
            "{argument:?}"
        );
    }
    for argument in ["--range=1..5", "--script=plugin.py"] {
        let request = plugin_request(vec![script.clone(), argument.to_owned()]);
        let launch = crate::live_bridge::plugin::launch(&request, &Default::default());
        assert!(launch.is_ok(), "{argument:?}");
    }
}

/// Review 2026-10-18 MCP-5: reading a command line had no limit on its
/// arguments and did not stop when the request was cancelled.
#[test]
fn a_command_line_is_read_within_limits_and_not_after_a_cancel() {
    let _turn = file_turn();
    let script = example_script().to_string_lossy().into_owned();
    let mut arguments = vec![script.clone()];
    arguments.extend((0..plugin_manifest::MAX_ARGUMENTS).map(|index| index.to_string()));
    let request = plugin_request(arguments);
    assert_eq!(
        crate::live_bridge::plugin::launch(&request, &Default::default()).err(),
        Some("invalid_params")
    );
    let request = plugin_request(vec![script, "2".into(), "35".into()]);
    let cancelled = Arc::new(AtomicBool::new(true));
    assert_eq!(
        crate::live_bridge::plugin::launch(&request, &cancelled).err(),
        Some("request_cancelled")
    );
    assert!(crate::live_bridge::plugin::launch(&request, &Default::default()).is_ok());
}

/// Review 2026-10-16 R-2: an absolute path joined to an option
/// (`--require=/x.js`, `-r/x.js`, `--x=C:\x.js`) has no root component of its
/// own, so it stayed "inside" the folder, or all-normal on a command line, and
/// reached the process as given; an interpreter then opened a file outside the
/// stage that nobody hashed or showed. It is refused from a plugin folder and
/// from a command line, before anyone is asked.
#[test]
fn an_absolute_path_joined_to_a_plugin_option_is_refused() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let outside = directory.path().join("outside.js");
    std::fs::write(&outside, "// outside the stage\n").unwrap();
    let outside = outside.display().to_string();
    let mut joined = vec![
        format!("--require={outside}"),
        format!("-r{outside}"),
        format!("--x=a={outside}"),
        "--require=/tmp/x.js".to_owned(),
        "-r/tmp/x.js".to_owned(),
    ];
    if cfg!(windows) {
        joined.extend([
            "--x=C:\\evil.js".to_owned(),
            "--x=C:evil.js".to_owned(),
            "-rC:\\evil.js".to_owned(),
            "--x=\\\\server\\share\\evil.js".to_owned(),
        ]);
    }
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    let script = example_script().to_string_lossy().into_owned();
    for argument in &joined {
        manifest["arguments"] = json!(["plugin.py", argument]);
        std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
        let inputs = json!({"height_mm": 35});
        let launch = crate::live_bridge::plugin_manifest::load(
            &folder.to_string_lossy(),
            inputs.as_object(),
            &Default::default(),
        );
        assert_eq!(
            launch.err(),
            Some("plugin_manifest_invalid"),
            "{argument:?}"
        );
        let request = plugin_request(vec![script.clone(), argument.clone()]);
        assert_eq!(
            crate::live_bridge::plugin::launch(&request, &Default::default()).err(),
            Some("invalid_path"),
            "{argument:?}"
        );
    }
    for argument in [
        "--script=plugin.py",
        "--script=sub/plugin.py",
        "--size=1x2",
        "-v",
    ] {
        manifest["arguments"] = json!(["plugin.py", argument]);
        std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
        let inputs = json!({"height_mm": 35});
        let launch = crate::live_bridge::plugin_manifest::load(
            &folder.to_string_lossy(),
            inputs.as_object(),
            &Default::default(),
        );
        assert!(launch.is_ok(), "{argument:?}");
        let request = plugin_request(vec![script.clone(), argument.to_owned()]);
        assert!(
            crate::live_bridge::plugin::launch(&request, &Default::default()).is_ok(),
            "{argument:?}"
        );
    }
}

/// Review 2026-10-17 D-6: a list of refused shapes was never complete. A URL
/// joined to an option (`--import=file:///…`) or given alone, and a file of
/// further arguments (`@/tmp/args`), named code nobody staged or hashed. Only
/// the shapes of a white list pass: an option name, with a value of plain
/// words and relative paths, or a word that is a local path or plain text.
#[test]
fn only_white_listed_argument_shapes_reach_a_plugin() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    let script = example_script().to_string_lossy().into_owned();
    let load = |manifest: &Value| {
        std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
        let inputs = json!({"height_mm": 35});
        crate::live_bridge::plugin_manifest::load(
            &folder.to_string_lossy(),
            inputs.as_object(),
            &Default::default(),
        )
    };
    for argument in [
        "--import=file:///tmp/x.mjs",
        "--loader=file:///tmp/x.mjs",
        "--import=data:text/javascript,1",
        "--require=https://example.invalid/x.js",
        "file:///tmp/x.mjs",
        "@/tmp/args",
        "@args",
        "-Dx=$(evil)",
        "--x=~/x.js",
    ] {
        manifest["arguments"] = json!(["plugin.py", argument]);
        assert_eq!(
            load(&manifest).err(),
            Some("plugin_manifest_invalid"),
            "{argument:?}"
        );
        let request = plugin_request(vec![script.clone(), argument.to_owned()]);
        assert_eq!(
            crate::live_bridge::plugin::launch(&request, &Default::default()).err(),
            Some("invalid_path"),
            "{argument:?}"
        );
    }
    for argument in ["-Dkey=value", "--", "-5", "--mode=fast", "a+b,c"] {
        manifest["arguments"] = json!(["plugin.py", argument]);
        assert!(load(&manifest).is_ok(), "{argument:?}");
        let request = plugin_request(vec![script.clone(), argument.to_owned()]);
        assert!(
            crate::live_bridge::plugin::launch(&request, &Default::default()).is_ok(),
            "{argument:?}"
        );
    }
}

/// `path` relative to `base`, through `..` where it must.
fn pathdiff_relative(path: &Path, base: &Path) -> PathBuf {
    let path = std::path::absolute(path).unwrap();
    let path: Vec<_> = path.components().collect();
    let base: Vec<_> = base.components().collect();
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut relative = PathBuf::new();
    for _ in common..base.len() {
        relative.push("..");
    }
    relative.extend(&path[common..]);
    relative
}

/// SEC-6: a folder outside the plugin folder named on the command line could
/// gain files nobody hashed; it is refused before anyone is asked.
#[test]
fn a_plugin_argument_naming_a_folder_outside_is_refused_without_a_prompt() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let modules = directory.path().join("modules");
    std::fs::create_dir(&modules).unwrap();
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    manifest["arguments"] = json!(["plugin.py", modules.display().to_string()]);
    std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
    let dialogs = ScriptedFileDialogs::new().always_confirm_high_risk_as(1);
    let probe = dialogs.clone();
    let mut wire = plugin_wire_answering(dialogs);
    let request = folder_request(&folder, json!({"height_mm": 35}));
    assert_refused_without_prompt(&mut wire, request, "invalid_path", &probe);

    let command = plugin_request(vec![
        example_script().to_string_lossy().into_owned(),
        modules.display().to_string(),
    ]);
    assert_refused_without_prompt(&mut wire, command, "invalid_path", &probe);
    assert!(probe.high_risk_prompts().is_empty());
}

/// SEC-6: files are held, but a file can still be added to the stage's
/// folders; a stage that no longer holds exactly the hashed files does not run.
#[test]
fn a_file_added_to_a_plugin_stage_after_hashing_stops_the_run() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let inputs = json!({"height_mm": 35});
    for planted in ["json.py", "ketchup_sdk/json.py"] {
        let launch = crate::live_bridge::plugin_manifest::load(
            &folder.to_string_lossy(),
            inputs.as_object(),
            &Default::default(),
        )
        .unwrap();
        launch.held.unchanged().unwrap();
        let stage = launch.command.working_directory.clone().unwrap();
        std::fs::write(stage.join(planted), "raise SystemExit\n").unwrap();
        let error = launch.held.unchanged().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("was added to the plugin's files"),
            "{planted}: {error}"
        );
    }
}

/// Review 2026-10-17 SEC-7: a word with no `=` and no leading `-` passed as
/// plain text, so `-c "exec(open('…/x.py').read())"`, `-e require(…)`,
/// `-m evil` or `-` (standard input) ran code nobody hashed, and a consent
/// stayed valid however `x.py` changed. An interpreter option that runs code
/// given as text, from standard input or found by name is refused; a word that
/// names no file must be plain words; the command line names at least one
/// hashed file; and the stage is not in the shared temporary directory.
#[test]
fn a_plugin_command_line_runs_only_hashed_code() {
    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let unhashed = directory.path().join("x.py");
    std::fs::write(&unhashed, "raise SystemExit\n").unwrap();
    let open_unhashed = format!(
        "exec(open({:?}).read())",
        unhashed.display().to_string().replace('\\', "/")
    );
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    let script = example_script().to_string_lossy().into_owned();
    let load = |manifest: &Value| {
        std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
        let inputs = json!({"height_mm": 35});
        crate::live_bridge::plugin_manifest::load(
            &folder.to_string_lossy(),
            inputs.as_object(),
            &Default::default(),
        )
    };
    let refused: Vec<Vec<String>> = vec![
        vec!["-c".into(), open_unhashed.clone()],
        vec!["-c".into(), "pass".into(), "plugin.py".into()],
        vec!["-e".into(), "require('x')".into()],
        vec!["-m".into(), "evil".into()],
        vec!["-mevil".into(), "plugin.py".into()],
        vec!["-Bm".into(), "evil".into(), "plugin.py".into()],
        // Review 2026-10-19 SEC-9: PowerShell reads -C as -Command.
        vec!["-C".into(), "Remove-Item".into(), "plugin.py".into()],
        vec!["-BC".into(), "evil".into(), "plugin.py".into()],
        vec!["--require=evil".into(), "plugin.py".into()],
        vec!["--import=evil".into(), "plugin.py".into()],
        vec!["-".into()],
        vec!["plugin.py".into(), open_unhashed.clone()],
        vec!["evil".into()],
        vec![],
    ];
    for arguments in &refused {
        manifest["arguments"] = json!(arguments);
        assert_eq!(
            load(&manifest).err(),
            Some("plugin_manifest_invalid"),
            "{arguments:?}"
        );
        // The same words after a script on a command line (or alone).
        let command = if arguments.first().is_some_and(|word| word == "plugin.py") {
            std::iter::once(script.clone())
                .chain(arguments[1..].iter().cloned())
                .collect()
        } else {
            arguments.clone()
        };
        let request = plugin_request(command.clone());
        assert_eq!(
            crate::live_bridge::plugin::launch(&request, &Default::default()).err(),
            Some("invalid_path"),
            "{command:?}"
        );
    }

    manifest["arguments"] = json!(["-B", "plugin.py", "-v", "-Dkey=value", "later"]);
    let launch = load(&manifest).unwrap();
    let stage = launch.command.working_directory.clone().unwrap();
    assert!(
        !stage.starts_with(std::env::temp_dir()),
        "{} is in the shared temporary directory",
        stage.display()
    );
    let request = plugin_request(vec![script, "-v".into(), "later".into()]);
    let launch = crate::live_bridge::plugin::launch(&request, &Default::default()).unwrap();
    let directory = launch.command.working_directory.clone().unwrap();
    assert!(!directory.starts_with(std::env::temp_dir()));
}

/// Review 2026-10-18 SEC-8: the window refused a list of code options, which
/// was never complete (`pwsh -C cmdlet hashed.txt` ran the cmdlet), took any
/// hashed file anywhere on the line for the script, and did not hash the
/// interpreter a `#!` executable starts. An interpreter now gets only the
/// options listed for it, then its hashed script, then words for the script;
/// an executable starting with `#!` is refused.
#[test]
fn an_interpreter_gets_only_listed_options_before_its_hashed_script() {
    use crate::live_bridge::plugin_manifest::script_first;
    let words = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    let pwsh = Path::new("C:/Program Files/PowerShell/7/pwsh.exe");
    let python = Path::new("/usr/bin/python3.12");
    let other = Path::new("/usr/bin/deno");
    for (executable, arguments, names_file, passes) in [
        (
            pwsh,
            words(&["-C", "Get-Date", "x.txt"]),
            vec![false, false, true],
            false,
        ),
        (
            pwsh,
            words(&["-NoProfile", "-file", "x.ps1", "-C"]),
            vec![false, false, true, false],
            true,
        ),
        // Review 2026-10-19 SEC-9: Windows PowerShell reads a bare first word
        // as -Command and joins the words after it into that command.
        (
            Path::new("C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"),
            words(&["x.ps1", ";", "Remove-Item", "C:/data"]),
            vec![true, false, false, false],
            false,
        ),
        (
            pwsh,
            words(&["-File", "-NoProfile", "x.ps1"]),
            vec![false, false, true],
            false,
        ),
        (
            Path::new("C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"),
            words(&["-NoLogo", "-FILE", "x.ps1", ";", "Remove-Item"]),
            vec![false, false, true, false, false],
            true,
        ),
        (
            python,
            words(&["-B", "-u", "x.py", "-c", "pass"]),
            vec![false, false, true, false, false],
            true,
        ),
        (python, words(&["-b", "x.py"]), vec![false, true], false),
        (
            python,
            words(&["-W", "error", "x.py"]),
            vec![false, false, true],
            false,
        ),
        (python, words(&["evil", "x.py"]), vec![false, true], false),
        (python, words(&["-B"]), vec![false], false),
        (other, words(&["run", "x.ts"]), vec![false, true], false),
        (
            other,
            words(&["x.ts", "--allow-all"]),
            vec![true, false],
            true,
        ),
    ] {
        assert_eq!(
            script_first(executable, &arguments, &names_file, |_| "invalid_path").is_ok(),
            passes,
            "{executable:?} {arguments:?}"
        );
    }

    let _turn = file_turn();
    let directory = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(directory.path());
    let script = example_script().to_string_lossy().into_owned();
    let load = |manifest: &Value| {
        std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
        let inputs = json!({"height_mm": 35});
        crate::live_bridge::plugin_manifest::load(
            &folder.to_string_lossy(),
            inputs.as_object(),
            &Default::default(),
        )
    };
    let mut manifest: Value = serde_json::from_str(&folder_manifest(&["set_dimension"])).unwrap();
    // An option off the list before the script, which the old list let pass.
    manifest["arguments"] = json!(["-W", "error", "plugin.py"]);
    assert_eq!(load(&manifest).err(), Some("plugin_manifest_invalid"));
    let request = plugin_request(vec!["-W".into(), "error".into(), script.clone()]);
    assert_eq!(
        crate::live_bridge::plugin::launch(&request, &Default::default()).err(),
        Some("invalid_path")
    );
    // The listed options before the script, and any word after it, pass.
    manifest["arguments"] = json!(["-B", "-u", "plugin.py", "-c", "-"]);
    assert!(load(&manifest).is_ok());
    let request = plugin_request(vec!["-u".into(), script.clone(), "-c".into()]);
    assert!(crate::live_bridge::plugin::launch(&request, &Default::default()).is_ok());

    // An executable starting with #!, from a folder and a command line.
    let shebang = directory.path().join("interpreter.exe");
    std::fs::write(&shebang, "#!/bin/sh\nexec python3 \"$@\"\n").unwrap();
    manifest["executable"] = json!(shebang.display().to_string());
    manifest["arguments"] = json!(["plugin.py"]);
    assert_eq!(load(&manifest).err(), Some("plugin_manifest_invalid"));
    let mut request = plugin_request(vec![script]);
    request.executable = Some(shebang.display().to_string());
    assert_eq!(
        crate::live_bridge::plugin::launch(&request, &Default::default()).err(),
        Some("invalid_path")
    );
}
