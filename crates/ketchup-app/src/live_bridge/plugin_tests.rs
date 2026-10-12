//! A plugin runs in the window through the bridge: the program it sends is
//! published like `apply_and_verify`, one Undo step. A plugin that fails, or
//! whose document changes while it runs, leaves the document as it was.
use super::*;
use crate::live_bridge::measurement::tests::integration_support::file_turn;
use ketchup_model::document::{FeatureParameterTarget, ParameterValueType};

const EXTRUSION: u64 = 2;

fn height(app: &KetchupApp) -> f64 {
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

fn plugin_wire() -> Wire {
    let mut wire = Wire::new();
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

/// The bridge takes only an absolute executable path.
fn python() -> String {
    let output = std::process::Command::new(ketchup_test_env::python())
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn run_plugin(arguments: Vec<String>) -> Request {
    Request::RunPlugin(RunPluginRequest {
        expected: None,
        executable: python(),
        arguments,
        principal_id: 7001,
        capabilities: vec!["query.agent-state.v1".into(), "edit.cad-program.v1".into()],
        allowed_operations: vec!["set_dimension".into()],
        timeout_ms: 30_000,
        validators: Vec::new(),
        strict: false,
    })
}

/// The Python SDK's example plugin, setting the extrusion height to `height_mm`.
fn example(height_mm: &str) -> Request {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../sdk/python/examples/dimension_plugin.py");
    run_plugin(vec![
        script.to_string_lossy().into_owned(),
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
        std::thread::sleep(Duration::from_millis(5));
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
        if let Ok(response) = rx.try_recv() {
            break response;
        }
        assert!(Instant::now() < deadline, "no answer to the stale plugin");
        wire.app.poll_live_bridge(&wire.context);
        std::thread::sleep(Duration::from_millis(5));
    };
    reader_thread.join().unwrap();
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("stale_document"));
    assert_eq!(wire.app.live_bridge_stamp(), edited);
    assert_eq!(height(&wire.app), original);
    assert_eq!(wire.app.undo_step_count(), undo + 1);
}
