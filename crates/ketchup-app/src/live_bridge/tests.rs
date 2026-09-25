use super::*;
use crate::dialogs::{
    DiscardRequest, ExportRequest, FileDialogs, HighRiskConfirmationRequest,
    HistoryTruncationRequest, ImportDialogRequest, SaveRequest, ScriptedFileDialogs,
};
#[path = "idle_retry_tests.rs"]
mod idle_retry;
#[path = "mesh_conversion_tests.rs"]
mod mesh_conversion;
#[path = "topology_recovery_tests.rs"]
mod topology_recovery;
use ketchup_core::{
    assembly_recipe::{
        AssemblyRecipe, RecipeEditScope, RecipeKey, RecipePartAdoption, RecipePartMobility,
        RecognizedRecipeFeatureKind,
    },
    document::NodeId,
    document::Transform,
    document::{
        CanonicalCommand, CommandBatch, DefinitionId, EdgeFinishKind, FeatureId, FeatureKind,
        GroupId, InstancePath, TagId,
    },
};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
#[path = "product_integration_tests.rs"]
mod product_integration;
fn program() -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::SetColor {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: vec![1],
            },
            color: Some([17, 29, 41]),
        }],
    }
}
fn mandatory_validators() -> Vec<String> {
    ["collision", "gravity_support"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[test]
fn edit_context_is_public_guarded_and_read_only() {
    let (mut app, mut bridge) = setup();
    let expected = app.live_bridge_stamp();
    let history = app.undo_step_count();
    let result = bridge
        .execute(
            &mut app,
            Request::EditContext {
                expected: Some(expected.clone()),
                targets: vec![AssistantInstancePath {
                    root_occurrence_id: 1,
                    steps: Vec::new(),
                }],
            },
            false,
        )
        .unwrap();

    assert_eq!(result["identity"]["document_id"], expected.document_id);
    assert_eq!(result["identity"]["revision"], expected.revision);
    assert_eq!(
        result["targets"][0]["instance"]["instance_path"],
        json!({
            "root_occurrence_id": 1,
            "steps": []
        })
    );
    assert_eq!(app.live_bridge_stamp(), expected);
    assert_eq!(app.undo_step_count(), history);
}

fn geometry_program() -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::Transform {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: vec![2],
            },
            translation_mm: [50.0, 0.0, 0.0],
            rotation: None,
        }],
    }
}
fn worker_required_program() -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::SetDimension {
            feature_id: 2,
            constraint_id: None,
            value_mm: 45.0,
        }],
    }
}
fn exact_fingerprints(app: &KetchupApp) -> Vec<String> {
    let mut fingerprints = app
        .exact_results
        .values()
        .map(|package| package.result_key().result_fingerprint)
        .collect::<Vec<_>>();
    fingerprints.sort();
    fingerprints
}
fn setup() -> (KetchupApp, LiveBridge) {
    let mut app = KetchupApp::new();
    app.selection.clear();
    let bridge = transport::start(egui::Context::default()).unwrap();
    (app, bridge)
}

struct DisconnectOnHighRisk {
    stream: Arc<Mutex<Option<TcpStream>>>,
    prompted: Arc<AtomicBool>,
}

impl FileDialogs for DisconnectOnHighRisk {
    fn pick_open_path(&mut self, _filter_label: &str) -> Option<PathBuf> {
        None
    }

    fn pick_save_path(&mut self, _request: SaveRequest<'_>) -> Option<PathBuf> {
        None
    }

    fn pick_export_path(&mut self, _request: ExportRequest<'_>) -> Option<PathBuf> {
        None
    }

    fn pick_import_path(&mut self, _request: ImportDialogRequest<'_>) -> Option<PathBuf> {
        None
    }

    fn confirm_discard(&mut self, _request: DiscardRequest<'_>) -> bool {
        false
    }

    fn confirm_history_truncation(&mut self, _request: HistoryTruncationRequest<'_>) -> bool {
        false
    }

    fn confirm_high_risk(&mut self, _request: HighRiskConfirmationRequest<'_>) -> Option<u64> {
        self.prompted.store(true, Ordering::Release);
        let mut stream = self.stream.lock().unwrap().take().unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).unwrap(), 0);
        Some(7)
    }
}

#[test]
fn disconnect_during_live_overwrite_consent_revokes_publication_authority() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cancelled-live-save-as.ketchup");
    let original_bytes = b"existing destination";
    std::fs::write(&path, original_bytes).unwrap();
    let stream_slot = Arc::new(Mutex::new(None));
    let prompted = Arc::new(AtomicBool::new(false));
    let dialogs = DisconnectOnHighRisk {
        stream: Arc::clone(&stream_slot),
        prompted: Arc::clone(&prompted),
    };
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.selection.clear();
    assert!(app.create_box());
    let expected = app.live_bridge_stamp();
    let context = egui::Context::default();
    let address = app.enable_live_bridge(&context).unwrap();
    let credentials = app.live_bridge_credentials().unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    *stream_slot.lock().unwrap() = Some(stream.try_clone().unwrap());
    let bytes = serde_json::to_vec(&Envelope {
        version: 1,
        id: 1,
        token: credentials.token,
        request: Request::SaveAs {
            expected: Some(expected),
            path: path.to_string_lossy().into_owned(),
        },
    })
    .unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    while !prompted.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        app.poll_live_bridge(&context);
        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
    assert!(app.document_path.is_none());
    assert!(app.is_dirty());
}

#[test]
fn save_is_revision_bound_and_uses_the_live_gui_overwrite_consent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("live-save.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval(7);
    let probe = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.selection.clear();
    assert!(app.save_document_to(&path));
    let original_bytes = std::fs::read(&path).unwrap();
    let before_edit = app.live_bridge_stamp();
    let mut bridge = transport::start(egui::Context::default()).unwrap();
    let commit = proposal(&mut app, &mut bridge);
    bridge.execute(&mut app, commit, false).unwrap();
    assert!(app.is_dirty());

    assert_eq!(
        bridge.execute(
            &mut app,
            Request::Save {
                expected: Some(before_edit),
            },
            false,
        ),
        Err("stale_document")
    );
    assert!(probe.high_risk_prompts().is_empty());

    let expected = app.live_bridge_stamp();
    assert_eq!(
        bridge.execute(
            &mut app,
            Request::Save {
                expected: Some(expected.clone()),
            },
            false,
        ),
        Err("save_rejected")
    );
    assert!(app.is_dirty());
    assert_eq!(probe.high_risk_prompts().len(), 1);
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);

    let saved = bridge
        .execute(
            &mut app,
            Request::Save {
                expected: Some(expected),
            },
            false,
        )
        .unwrap();
    assert_eq!(
        saved,
        json!({"saved":true,"same_gui_document":true,"dirty":false})
    );
    assert!(!app.is_dirty());
    assert_eq!(probe.high_risk_prompts().len(), 2);
    assert_ne!(std::fs::read(&path).unwrap(), original_bytes);
    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_path(&path));
    assert_eq!(
        reopened.document_snapshot().canonical_digest(),
        app.document_snapshot().canonical_digest()
    );

    let (mut untitled, mut untitled_bridge) = setup();
    let expected = untitled.live_bridge_stamp();
    assert_eq!(
        untitled_bridge.execute(
            &mut untitled,
            Request::Save {
                expected: Some(expected)
            },
            false
        ),
        Err("save_path_required")
    );
}

#[test]
fn save_as_requires_an_absolute_path_and_live_gui_overwrite_consent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("live-save-as.ketchup");
    let original_bytes = b"existing destination";
    std::fs::write(&path, original_bytes).unwrap();
    let dialogs = ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval(9);
    let probe = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.selection.clear();
    let stale = app.live_bridge_stamp();
    assert!(app.create_box());
    let expected = app.live_bridge_stamp();
    let mut bridge = transport::start(egui::Context::default()).unwrap();

    assert_eq!(
        bridge.execute(
            &mut app,
            Request::SaveAs {
                expected: Some(expected.clone()),
                path: "relative.ketchup".to_owned(),
            },
            false,
        ),
        Err("invalid_path")
    );
    assert_eq!(
        bridge.execute(
            &mut app,
            Request::SaveAs {
                expected: Some(stale),
                path: path.to_string_lossy().into_owned(),
            },
            false,
        ),
        Err("stale_document")
    );
    assert!(probe.high_risk_prompts().is_empty());

    assert_eq!(
        bridge.execute(
            &mut app,
            Request::SaveAs {
                expected: Some(expected.clone()),
                path: path.to_string_lossy().into_owned(),
            },
            false,
        ),
        Err("save_rejected")
    );
    assert!(app.is_dirty());
    assert!(app.document_path.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);

    let saved = bridge
        .execute(
            &mut app,
            Request::SaveAs {
                expected: Some(expected),
                path: path.to_string_lossy().into_owned(),
            },
            false,
        )
        .unwrap();
    assert_eq!(
        saved,
        json!({"saved":true,"same_gui_document":true,"dirty":false})
    );
    assert_eq!(probe.high_risk_prompts().len(), 2);
    assert_eq!(app.document_path.as_deref(), Some(path.as_path()));
    assert_ne!(std::fs::read(&path).unwrap(), original_bytes);
    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_path(&path));
    assert_eq!(
        reopened.document_snapshot().canonical_digest(),
        app.document_snapshot().canonical_digest()
    );
}

#[test]
fn open_is_revision_bound_and_uses_the_live_gui_discard_consent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("live-open.ketchup");
    let mut source = KetchupApp::new();
    assert!(source.create_box());
    assert!(source.save_document_to(&path));
    let source_digest = source.document_snapshot().canonical_digest();

    let dialogs = ScriptedFileDialogs::new();
    let probe = dialogs.clone();
    let mut refused = KetchupApp::new().with_dialogs(Box::new(dialogs));
    let stale = refused.live_bridge_stamp();
    assert!(refused.create_box());
    let expected = refused.live_bridge_stamp();
    let refused_digest = refused.document_snapshot().canonical_digest();
    let mut bridge = transport::start(egui::Context::default()).unwrap();

    assert_eq!(
        bridge.execute(
            &mut refused,
            Request::Open {
                expected: Some(expected.clone()),
                path: "relative.ketchup".to_owned(),
            },
            false,
        ),
        Err("invalid_path")
    );
    assert_eq!(
        bridge.execute(
            &mut refused,
            Request::Open {
                expected: Some(stale),
                path: path.to_string_lossy().into_owned(),
            },
            false,
        ),
        Err("stale_document")
    );
    assert_eq!(probe.discard_prompts(), 0);
    assert_eq!(
        bridge.execute(
            &mut refused,
            Request::Open {
                expected: Some(expected),
                path: path.to_string_lossy().into_owned(),
            },
            false,
        ),
        Err("open_rejected")
    );
    assert_eq!(probe.discard_prompts(), 1);
    assert!(refused.is_dirty());
    assert_eq!(
        refused.document_snapshot().canonical_digest(),
        refused_digest
    );
    assert!(refused.document_path().is_none());

    let dialogs = ScriptedFileDialogs::new().always_discard();
    let probe = dialogs.clone();
    let mut approved = KetchupApp::new().with_dialogs(Box::new(dialogs));
    assert!(approved.create_box());
    let mut bridge = transport::start(egui::Context::default()).unwrap();
    let pending = proposal(&mut approved, &mut bridge);
    assert!(matches!(pending, Request::Commit { .. }));
    assert!(bridge.pending.is_some());
    let expected = approved.live_bridge_stamp();
    let opened = bridge
        .execute(
            &mut approved,
            Request::Open {
                expected: Some(expected),
                path: path.to_string_lossy().into_owned(),
            },
            false,
        )
        .unwrap();

    assert_eq!(
        opened,
        json!({"opened":true,"same_gui_window":true,"dirty":false})
    );
    assert_eq!(probe.discard_prompts(), 1);
    assert_eq!(approved.document_path(), Some(path.as_path()));
    assert_eq!(
        approved.document_snapshot().canonical_digest(),
        source_digest
    );
    assert!(!approved.is_dirty());
    assert!(bridge.pending.is_none());
    assert_eq!(bridge.observed, Some(approved.live_bridge_stamp()));
}

fn proposal(app: &mut KetchupApp, bridge: &mut LiveBridge) -> Request {
    let expected = app.live_bridge_stamp();
    let selection = LiveBridge::selection(app).unwrap();
    let result = bridge
        .execute(
            app,
            Request::Propose {
                expected: Some(expected.clone()),
                selection: Some(selection),
                program: program(),
            },
            false,
        )
        .unwrap();
    Request::Commit {
        expected: Some(expected),
        proposal_id: result["proposal_id"].as_u64().unwrap(),
    }
}
fn protected_requests(stamp: &Stamp, commit: &Request) -> Vec<Request> {
    vec![
        commit.clone(),
        Request::Undo {
            expected: Some(stamp.clone()),
        },
        Request::Redo {
            expected: Some(stamp.clone()),
        },
        Request::Selection {
            expected: Some(stamp.clone()),
            occurrence_ids: vec![],
        },
        Request::View {
            expected: Some(stamp.clone()),
            view: View::Top,
        },
        Request::Propose {
            expected: Some(stamp.clone()),
            selection: Some(vec![1]),
            program: program(),
        },
    ]
}

#[test]
fn sdk_and_builtin_assistant_share_apply_and_verify_error_codes() {
    let invalid_program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::SetColor {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: vec![999],
            },
            color: Some([17, 29, 41]),
        }],
    };
    let (mut assistant_app, _) = setup();
    let assistant_before = assistant_app.live_bridge_stamp();
    let assistant_undo = assistant_app.undo_step_count();
    let assistant_error =
        LiveBridge::apply_assistant_cad_program(&mut assistant_app, invalid_program.clone())
            .unwrap_err();

    let (mut sdk_app, mut bridge) = setup();
    let sdk_before = sdk_app.live_bridge_stamp();
    let sdk_undo = sdk_app.undo_step_count();
    let sdk_error = bridge
        .execute(
            &mut sdk_app,
            Request::ApplyAndVerify {
                expected: Some(sdk_before.clone()),
                selection: Some(vec![]),
                program: invalid_program,
                validators: mandatory_validators(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: None,
            },
            false,
        )
        .unwrap_err();

    assert_eq!(assistant_error.into_code(), sdk_error);
    assert_eq!(assistant_app.live_bridge_stamp(), assistant_before);
    assert_eq!(sdk_app.live_bridge_stamp(), sdk_before);
    assert_eq!(assistant_app.undo_step_count(), assistant_undo);
    assert_eq!(sdk_app.undo_step_count(), sdk_undo);
}

#[test]
fn sdk_and_builtin_assistant_share_apply_and_verify_success_contract() {
    let prepare = |app: &mut KetchupApp| {
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceGrounded {
                    id: OccurrenceId(1),
                    grounded: true,
                },
            ]))
            .unwrap();
        crate::tests::install_initial_graph_result(app);
    };
    let contract = |value: &Value| {
        json!({
            "published": value["published"],
            "saved": value["saved"],
            "save_state": value["save_state"],
            "diff_entry_count": value["diff"]["entry_count"],
            "exact_complete": value["exact"]["complete"],
            "topology_complete": value["exact"]["topology_complete"],
            "validation_state": value["validation"]["state"],
            "validation_complete": value["validation"]["complete"],
            "validators": value["validation"]["requested"],
        })
    };

    let (mut assistant_app, _) = setup();
    prepare(&mut assistant_app);
    let assistant_before = assistant_app.live_bridge_stamp();
    let assistant_undo = assistant_app.undo_step_count();
    let assistant_result =
        LiveBridge::apply_assistant_cad_program(&mut assistant_app, program()).unwrap();

    let (mut sdk_app, mut bridge) = setup();
    prepare(&mut sdk_app);
    let sdk_before = sdk_app.live_bridge_stamp();
    let sdk_undo = sdk_app.undo_step_count();
    let sdk_result = bridge
        .execute(
            &mut sdk_app,
            Request::ApplyAndVerify {
                expected: Some(sdk_before.clone()),
                selection: Some(vec![]),
                program: program(),
                validators: Vec::new(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: None,
            },
            false,
        )
        .unwrap();

    assert_eq!(contract(&assistant_result), contract(&sdk_result));
    // Collision is the only check every edit pays for; gravity is on demand.
    assert_eq!(sdk_result["validation"]["requested"], json!(["collision"]));
    assert_eq!(
        assistant_app.live_bridge_stamp().revision,
        assistant_before.revision + 1
    );
    assert_eq!(
        sdk_app.live_bridge_stamp().revision,
        sdk_before.revision + 1
    );
    assert_eq!(assistant_app.undo_step_count(), assistant_undo + 1);
    assert_eq!(sdk_app.undo_step_count(), sdk_undo + 1);
}

#[test]
fn apply_and_verify_needs_only_a_program() {
    // Nothing is grounded: gravity is not requested, so it cannot block the edit.
    let (mut app, mut bridge) = setup();
    crate::tests::install_initial_graph_result(&mut app);
    let before_steps = app.undo_step_count();
    let request: Request = serde_json::from_value(json!({
        "method": "apply_and_verify",
        "program": program(),
    }))
    .unwrap();

    let report = bridge.execute(&mut app, request, false).unwrap();

    assert_eq!(report["published"], true);
    assert_eq!(report["validation"]["state"], "passed");
    assert_eq!(app.undo_step_count(), before_steps + 1);
}

#[test]
fn apply_and_verify_is_one_undo_step_and_refuses_a_stale_resend() {
    let (mut app, mut bridge) = setup();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    let expected = app.live_bridge_stamp();
    let before_digest = expected.canonical_digest.clone();
    let before_reference_count = app.document_snapshot().exact_reference_evidence().count();
    let before_steps = app.undo_step_count();
    let request = Request::ApplyAndVerify {
        expected: Some(expected.clone()),
        selection: Some(vec![]),
        program: program(),
        validators: mandatory_validators(),
        timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
        strict: false,
        save: None,
    };

    let first = bridge.execute(&mut app, request.clone(), false).unwrap();
    let committed_digest = app.live_bridge_stamp().canonical_digest;
    assert_ne!(committed_digest, before_digest);
    let committed_reference_count = app.document_snapshot().exact_reference_evidence().count();
    assert_eq!(committed_reference_count, before_reference_count);
    assert_eq!(app.undo_step_count(), before_steps + 1);
    assert_eq!(first["published"], true);
    assert_eq!(first["saved"], false);
    assert_eq!(first["save_state"], "not_requested");
    assert_eq!(first["exact"]["complete"], true);
    assert_eq!(first["exact"]["topology_complete"], true);
    assert_eq!(first["validation"]["state"], "passed");
    assert_eq!(first["validation"]["complete"], true);

    assert_eq!(
        bridge.execute(&mut app, request, false),
        Err("stale_document")
    );
    assert_eq!(app.live_bridge_stamp().canonical_digest, committed_digest);
    assert_eq!(app.undo_step_count(), before_steps + 1);
    assert!(app.undo());
    assert_eq!(app.live_bridge_stamp().canonical_digest, before_digest);
    assert_eq!(
        app.document_snapshot().exact_reference_evidence().count(),
        before_reference_count
    );
    assert!(app.redo());
    assert_eq!(app.live_bridge_stamp().canonical_digest, committed_digest);
    assert_eq!(
        app.document_snapshot().exact_reference_evidence().count(),
        committed_reference_count
    );
}

#[test]
fn queued_apply_and_verify_yields_to_manual_edit_and_refuses_stale_publication() {
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
    let expected = wire.app.live_bridge_stamp();
    let before_steps = wire.app.undo_step_count();
    wire.send(
        Request::ApplyAndVerify {
            expected: Some(expected),
            selection: Some(vec![]),
            program: program(),
            validators: mandatory_validators(),
            timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
            strict: false,
            save: None,
        },
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

    let start_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        wire.app.poll_live_bridge(&wire.context);
        if wire
            .app
            .live_bridge
            .as_ref()
            .is_some_and(|bridge| bridge.apply_and_verify_job.is_some())
        {
            break;
        }
        assert!(Instant::now() < start_deadline, "job did not start");
        std::thread::sleep(Duration::from_millis(5));
    }

    wire.app
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: false,
            },
        ]))
        .unwrap();
    let manual_stamp = wire.app.live_bridge_stamp();
    assert_eq!(wire.app.undo_step_count(), before_steps + 1);

    let response_deadline = Instant::now() + Duration::from_secs(4);
    let response = loop {
        if let Ok(response) = rx.try_recv() {
            break response;
        }
        assert!(
            Instant::now() < response_deadline,
            "stale result was not returned"
        );
        wire.app.poll_live_bridge(&wire.context);
        std::thread::sleep(Duration::from_millis(5));
    };
    reader_thread.join().unwrap();
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("stale_document"));
    assert_eq!(response.stamp.as_ref(), Some(&manual_stamp));
    assert_eq!(wire.app.live_bridge_stamp(), manual_stamp);
    assert_eq!(wire.app.undo_step_count(), before_steps + 1);
}

#[test]
fn queued_apply_and_verify_refuses_post_validation_selection_change() {
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
    let expected = wire.app.live_bridge_stamp();
    let before_steps = wire.app.undo_step_count();
    wire.send(
        Request::ApplyAndVerify {
            expected: Some(expected.clone()),
            selection: Some(vec![]),
            program: program(),
            validators: mandatory_validators(),
            timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
            strict: false,
            save: None,
        },
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

    let start_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        wire.app.poll_live_bridge(&wire.context);
        if wire
            .app
            .live_bridge
            .as_ref()
            .is_some_and(|bridge| bridge.apply_and_verify_job.is_some())
        {
            break;
        }
        assert!(Instant::now() < start_deadline, "job did not start");
        std::thread::sleep(Duration::from_millis(5));
    }

    wire.app.selection.select_occurrence(OccurrenceId(1), false);
    assert_eq!(wire.app.live_bridge_stamp(), expected);
    assert_eq!(wire.app.undo_step_count(), before_steps);

    let response_deadline = Instant::now() + Duration::from_secs(4);
    let response = loop {
        if let Ok(response) = rx.try_recv() {
            break response;
        }
        assert!(
            Instant::now() < response_deadline,
            "selection-stale result was not returned"
        );
        wire.app.poll_live_bridge(&wire.context);
        std::thread::sleep(Duration::from_millis(5));
    };
    reader_thread.join().unwrap();
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("selection_changed"));
    assert_eq!(response.stamp.as_ref(), Some(&expected));
    assert_eq!(wire.app.live_bridge_stamp(), expected);
    assert_eq!(wire.app.undo_step_count(), before_steps);
}

#[test]
fn apply_and_verify_worker_disconnect_is_zero_mutation() {
    let (mut app, mut bridge) = setup();
    let expected = app.live_bridge_stamp();
    let before = (
        expected.clone(),
        app.undo_step_count(),
        app.redo_step_count(),
        app.is_dirty(),
    );
    let proposal = app.derive_assistant_cad_edit_proposal(&program()).unwrap();
    let candidate = app.document.preview_verified_proposal(&proposal).unwrap();
    let (reply, response) = mpsc::sync_channel(1);
    let (sender, receiver) = mpsc::sync_channel::<Result<PreparedApplyAndVerify, &'static str>>(1);
    drop(sender);
    bridge.apply_and_verify_job = Some(ApplyAndVerifyJob {
        id: 91,
        reply,
        cancelled: Arc::new(AtomicBool::new(false)),
        worker_cancelled: Arc::new(AtomicBool::new(false)),
        plan: ApplyAndVerifyPlan {
            before: expected,
            selection: None,
            proposal,
            candidate,
            save_path: None,
            timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
            strict: false,
            started: Instant::now(),
            planned_at: Instant::now(),
        },
        receiver,
    });

    bridge.poll_apply_and_verify_job(&mut app, &egui::Context::default(), false);

    let response = response.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(!response.ok);
    assert_eq!(
        response.error.as_deref(),
        Some("apply_and_verify_worker_disconnected")
    );
    assert_eq!(
        (
            app.live_bridge_stamp(),
            app.undo_step_count(),
            app.redo_step_count(),
            app.is_dirty(),
        ),
        before
    );
}

#[test]
fn apply_and_verify_timeout_is_zero_mutation_and_cancels_worker() {
    let (mut app, mut bridge) = setup();
    let expected = app.live_bridge_stamp();
    let before = (
        expected.clone(),
        app.undo_step_count(),
        app.redo_step_count(),
        app.is_dirty(),
    );
    let proposal = app.derive_assistant_cad_edit_proposal(&program()).unwrap();
    let candidate = app.document.preview_verified_proposal(&proposal).unwrap();
    let (reply, response) = mpsc::sync_channel(1);
    let (_sender, receiver) = mpsc::sync_channel::<Result<PreparedApplyAndVerify, &'static str>>(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::new(AtomicBool::new(false));
    bridge.apply_and_verify_job = Some(ApplyAndVerifyJob {
        id: 92,
        reply,
        cancelled: Arc::clone(&cancelled),
        worker_cancelled: Arc::clone(&worker_cancelled),
        plan: ApplyAndVerifyPlan {
            before: expected,
            selection: None,
            proposal,
            candidate,
            save_path: None,
            timeout_ms: 1,
            strict: false,
            started: Instant::now() - Duration::from_millis(2),
            planned_at: Instant::now() - Duration::from_millis(2),
        },
        receiver,
    });

    bridge.poll_apply_and_verify_job(&mut app, &egui::Context::default(), false);

    let response = response.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("job_timeout"));
    assert!(!cancelled.load(Ordering::Acquire));
    assert!(worker_cancelled.load(Ordering::Acquire));
    assert_eq!(
        (
            app.live_bridge_stamp(),
            app.undo_step_count(),
            app.redo_step_count(),
            app.is_dirty(),
        ),
        before
    );
}

#[cfg(windows)]
#[test]
fn apply_and_verify_real_exact_worker_crash_is_zero_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let worker = directory.path().join("crash-worker.cmd");
    std::fs::write(&worker, "@exit /b 23\r\n").unwrap();
    let (mut app, mut bridge) = setup();
    app.exact_worker_path = Some(worker);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    let expected = app.live_bridge_stamp();
    let before = (
        expected.clone(),
        app.undo_step_count(),
        app.redo_step_count(),
        app.is_dirty(),
        exact_fingerprints(&app),
    );

    assert_eq!(
        bridge.execute(
            &mut app,
            Request::ApplyAndVerify {
                expected: Some(expected),
                selection: Some(vec![]),
                program: worker_required_program(),
                validators: mandatory_validators(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: None,
            },
            false,
        ),
        Err("exact_evaluation_incomplete")
    );
    assert_eq!(
        (
            app.live_bridge_stamp(),
            app.undo_step_count(),
            app.redo_step_count(),
            app.is_dirty(),
            exact_fingerprints(&app),
        ),
        before
    );
}

#[cfg(windows)]
#[test]
fn apply_and_verify_cancels_a_running_exact_worker_without_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let worker = directory.path().join("slow-worker.cmd");
    let started = directory.path().join("started.txt");
    std::fs::write(
        &worker,
        format!(
            "@echo started>\"{}\"\r\n:wait\r\n@goto wait\r\n",
            started.display()
        ),
    )
    .unwrap();
    let (mut app, mut bridge) = setup();
    app.exact_worker_path = Some(worker);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    let expected = app.live_bridge_stamp();
    let before = (
        expected.clone(),
        app.undo_step_count(),
        app.redo_step_count(),
        app.is_dirty(),
        exact_fingerprints(&app),
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel_flag = Arc::clone(&cancelled);
    let canceller = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(4);
        while !started.is_file() {
            assert!(Instant::now() < deadline, "exact worker did not start");
            std::thread::sleep(Duration::from_millis(5));
        }
        cancel_flag.store(true, Ordering::Release);
    });

    assert_eq!(
        bridge.execute_authorized(
            &mut app,
            Request::ApplyAndVerify {
                expected: Some(expected),
                selection: Some(vec![]),
                program: worker_required_program(),
                validators: mandatory_validators(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: None,
            },
            false,
            &cancelled,
        ),
        Err("request_cancelled")
    );
    canceller.join().unwrap();
    assert_eq!(
        (
            app.live_bridge_stamp(),
            app.undo_step_count(),
            app.redo_step_count(),
            app.is_dirty(),
            exact_fingerprints(&app),
        ),
        before
    );
}

#[test]
fn apply_and_verify_one_undo_redo_restores_geometry_recipe_and_exact_binding_without_helper_document()
 {
    let (mut app, mut bridge) = setup();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "Independent recipe-safe instance".into(),
                transform: Transform::from_translation(500.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(2),
                grounded: true,
            },
        ]))
        .unwrap();
    let recipe = AssemblyRecipe::adopt(
        &app.document.current(),
        RecipeKey::new("live-transaction").unwrap(),
        vec![RecipePartAdoption {
            key: RecipeKey::new("owned-box").unwrap(),
            instance_path: InstancePath::root(OccurrenceId(1)),
            mobility: RecipePartMobility::Fixed,
            edit_scope: RecipeEditScope::SharedDefinition(DefinitionId(1)),
            parameters: BTreeMap::new(),
            features: vec![
                (
                    RecipeKey::new("owned-box/profile").unwrap(),
                    FeatureId(1),
                    RecognizedRecipeFeatureKind::Profile,
                ),
                (
                    RecipeKey::new("owned-box/extrusion").unwrap(),
                    FeatureId(2),
                    RecognizedRecipeFeatureKind::Extrusion,
                ),
            ],
        }],
        vec![],
        vec![],
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe.clone()),
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    let before = app.document.current();
    let before_stamp = app.live_bridge_stamp();
    let before_transform = before.occurrence(OccurrenceId(2)).unwrap().transform();
    let before_exact = exact_fingerprints(&app);
    let before_steps = app.undo_step_count();

    bridge
        .execute(
            &mut app,
            Request::ApplyAndVerify {
                expected: Some(before_stamp.clone()),
                selection: Some(vec![]),
                program: geometry_program(),
                validators: mandatory_validators(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: None,
            },
            false,
        )
        .unwrap();
    let after = app.document.current();
    let after_stamp = app.live_bridge_stamp();
    let after_transform = after.occurrence(OccurrenceId(2)).unwrap().transform();
    let after_exact = exact_fingerprints(&app);
    assert_ne!(after_transform, before_transform);
    assert_eq!(after.assembly_recipe(), Some(&recipe));
    recipe.audit(&after).unwrap();
    assert!(app.exact_results.is_bound_to(&after));
    assert!(app.topology_results.is_bound_to(&after));
    assert_eq!(before_exact, after_exact);
    assert_eq!(app.undo_step_count(), before_steps + 1);

    assert!(app.undo());
    let undone = app.document.current();
    assert_eq!(
        app.live_bridge_stamp().canonical_digest,
        before_stamp.canonical_digest
    );
    assert_eq!(
        undone.occurrence(OccurrenceId(2)).unwrap().transform(),
        before_transform
    );
    assert_eq!(undone.assembly_recipe(), Some(&recipe));
    recipe.audit(&undone).unwrap();
    let undone_exact = exact_fingerprints(&app);
    assert!(
        before_exact
            .iter()
            .all(|fingerprint| undone_exact.contains(fingerprint))
    );
    assert!(app.exact_results.is_bound_to(&undone));
    assert_eq!(app.redo_step_count(), 1);

    assert!(app.redo());
    let redone = app.document.current();
    let redone_stamp = app.live_bridge_stamp();
    assert_eq!(redone_stamp.document_id, after_stamp.document_id);
    assert_eq!(redone_stamp.revision, after_stamp.revision);
    assert_eq!(redone_stamp.canonical_digest, after_stamp.canonical_digest);
    assert!(redone_stamp.mutation_epoch > after_stamp.mutation_epoch);
    assert_eq!(
        redone.occurrence(OccurrenceId(2)).unwrap().transform(),
        after_transform
    );
    assert_eq!(redone.assembly_recipe(), Some(&recipe));
    recipe.audit(&redone).unwrap();
    assert_eq!(exact_fingerprints(&app), after_exact);
    assert!(app.exact_results.is_bound_to(&redone));
    assert_eq!(app.undo_step_count(), before_steps + 1);
}

#[test]
fn apply_and_verify_reports_committed_but_unsaved() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("apply-verify-save-refused.ketchup");
    let dialogs = ScriptedFileDialogs::new().queue_refused_high_risk();
    let probe = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.selection.clear();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    assert!(app.save_document_to(&path));
    let original_bytes = std::fs::read(&path).unwrap();
    let expected = app.live_bridge_stamp();
    let before_steps = app.undo_step_count();
    let mut bridge = transport::start(egui::Context::default()).unwrap();
    let request = Request::ApplyAndVerify {
        expected: Some(expected),
        selection: Some(vec![]),
        program: program(),
        validators: mandatory_validators(),
        timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
        strict: false,
        save: Some(ApplyAndVerifySave::Current {}),
    };

    let first = bridge.execute(&mut app, request, false).unwrap();
    assert_eq!(first["published"], true);
    assert_eq!(first["saved"], false);
    assert_eq!(first["save_state"], "committed_but_unsaved");
    assert_eq!(first["save_error"], "save_rejected");
    assert_eq!(first["save_path"], path.to_string_lossy().as_ref());
    assert!(app.is_dirty());
    assert_eq!(app.undo_step_count(), before_steps + 1);
    assert_eq!(probe.high_risk_prompts().len(), 1);
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
}

#[test]
fn apply_and_verify_save_io_failure_preserves_last_good_file_and_dirty_gui_state() {
    use egui_kittest::{Harness, kittest::Queryable as _};

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("save-io-failure.ketchup");
    let dialogs = ScriptedFileDialogs::new().queue_high_risk_approval(42);
    let probe = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.selection.clear();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    assert!(app.save_document_to(&path));
    let original_bytes = std::fs::read(&path).unwrap();
    let original_stamp = app.live_bridge_stamp();
    let saved_digest = app.saved_digest.clone();
    let file_identity = app.file_identity;
    let undo_before = app.undo_step_count();
    // Fail the real atomic-save backup write, without touching any user's save-lock.
    std::fs::create_dir(directory.path().join("save-io-failure.ketchup.recovery")).unwrap();
    let mut bridge = transport::start(egui::Context::default()).unwrap();
    let report = bridge
        .execute(
            &mut app,
            Request::ApplyAndVerify {
                expected: Some(original_stamp.clone()),
                selection: Some(vec![]),
                program: worker_required_program(),
                validators: mandatory_validators(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: Some(ApplyAndVerifySave::Current {}),
            },
            false,
        )
        .unwrap();
    assert_eq!(report["published"], true);
    assert_eq!(report["saved"], false);
    assert_eq!(report["save_state"], "committed_but_unsaved");
    assert_eq!(report["save_error"], "save_rejected");
    assert_eq!(probe.high_risk_prompts().len(), 1);
    assert_eq!(
        app.live_bridge_stamp().document_id,
        original_stamp.document_id
    );
    assert_ne!(
        app.live_bridge_stamp().canonical_digest,
        original_stamp.canonical_digest
    );
    assert_eq!(app.undo_step_count(), undo_before + 1);
    assert!(app.is_dirty());
    assert_eq!(app.saved_digest, saved_digest);
    assert_eq!(app.file_identity, file_identity);
    assert_eq!(app.document_path.as_deref(), Some(path.as_path()));
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
    let disk = ketchup_core::persistence::load(&original_bytes)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(
        disk.current().canonical_digest(),
        original_stamp.canonical_digest
    );
    assert!(
        app.digest.contains("active model remains unsaved"),
        "{}",
        app.digest
    );
    let error = format!(
        "{}  \u{b7}  {}",
        app.catalog.format(
            "status-selected",
            &BTreeMap::from([("count", "0".to_owned())])
        ),
        app.digest
    );
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_max_steps(64)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    assert!(
        harness.query_by_label(&error).is_some(),
        "save error must be visible in the GUI"
    );
    assert!(harness.state().is_dirty());
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
}

#[test]
fn apply_and_verify_saves_only_the_explicit_absolute_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("apply-verify-saved.ketchup");
    let (mut app, mut bridge) = setup();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    let expected = app.live_bridge_stamp();

    let report = bridge
        .execute(
            &mut app,
            Request::ApplyAndVerify {
                expected: Some(expected),
                selection: Some(vec![]),
                program: program(),
                validators: mandatory_validators(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: Some(ApplyAndVerifySave::Path {
                    path: path.to_string_lossy().into_owned(),
                }),
            },
            false,
        )
        .unwrap();

    assert_eq!(report["published"], true);
    assert_eq!(report["saved"], true);
    assert_eq!(report["save_state"], "saved");
    assert_eq!(report["save_error"], Value::Null);
    assert_eq!(report["save_path"], path.to_string_lossy().as_ref());
    assert!(!app.is_dirty());
    assert_eq!(app.document_path.as_deref(), Some(path.as_path()));
    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_path(&path));
    assert_eq!(
        reopened.document_snapshot().canonical_digest(),
        app.document_snapshot().canonical_digest()
    );
}

#[test]
fn apply_and_verify_preflight_failures_are_zero_mutation() {
    let (mut app, mut bridge) = setup();
    let expected = app.live_bridge_stamp();
    let before = (
        expected.clone(),
        app.undo_step_count(),
        app.redo_step_count(),
        app.is_dirty(),
    );
    assert_eq!(
        bridge.execute(
            &mut app,
            Request::ApplyAndVerify {
                expected: Some(expected.clone()),
                selection: Some(vec![]),
                program: program(),
                validators: vec!["no_such_validator".into()],
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: None,
            },
            false,
        ),
        Err("unknown_validator")
    );
    assert_eq!(
        (
            app.live_bridge_stamp(),
            app.undo_step_count(),
            app.redo_step_count(),
            app.is_dirty(),
        ),
        before
    );

    assert_eq!(
        bridge.execute(
            &mut app,
            Request::ApplyAndVerify {
                expected: Some(expected.clone()),
                selection: Some(vec![]),
                program: program(),
                validators: mandatory_validators(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: Some(ApplyAndVerifySave::Path {
                    path: "relative.ketchup".into(),
                }),
            },
            false,
        ),
        Err("invalid_path")
    );
    assert_eq!(
        (
            app.live_bridge_stamp(),
            app.undo_step_count(),
            app.redo_step_count(),
            app.is_dirty(),
        ),
        before
    );

    let cancelled = Arc::new(AtomicBool::new(true));
    assert_eq!(
        bridge.execute_authorized(
            &mut app,
            Request::ApplyAndVerify {
                expected: Some(expected),
                selection: Some(vec![]),
                program: program(),
                validators: mandatory_validators(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: None,
            },
            false,
            &cancelled,
        ),
        Err("request_cancelled")
    );
    assert_eq!(
        (
            app.live_bridge_stamp(),
            app.undo_step_count(),
            app.redo_step_count(),
            app.is_dirty(),
        ),
        before
    );
}

#[test]
fn apply_and_verify_phase_failures_are_zero_mutation() {
    for (fault, error) in [
        (ApplyAndVerifyFault::Planning, "planning_rejected"),
        (ApplyAndVerifyFault::Candidate, "candidate_rejected"),
        (ApplyAndVerifyFault::Exact, "exact_evaluation_rejected"),
        (ApplyAndVerifyFault::Validation, "validation_failed"),
        (ApplyAndVerifyFault::Publication, "commit_rejected"),
    ] {
        let (mut app, mut bridge) = setup();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceGrounded {
                    id: OccurrenceId(1),
                    grounded: true,
                },
            ]))
            .unwrap();
        crate::tests::install_initial_graph_result(&mut app);
        let expected = app.live_bridge_stamp();
        let before = (
            expected.clone(),
            app.undo_step_count(),
            app.redo_step_count(),
            app.is_dirty(),
        );
        bridge.apply_and_verify_fault = Some(fault);

        assert_eq!(
            bridge.execute(
                &mut app,
                Request::ApplyAndVerify {
                    expected: Some(expected),
                    selection: Some(vec![]),
                    program: program(),
                    validators: mandatory_validators(),
                    timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                    strict: false,
                    save: None,
                },
                false,
            ),
            Err(error),
            "fault phase {fault:?}"
        );
        assert_eq!(
            (
                app.live_bridge_stamp(),
                app.undo_step_count(),
                app.redo_step_count(),
                app.is_dirty(),
            ),
            before,
            "fault phase {fault:?} mutated the live document"
        );
    }
}

#[test]
fn unsupported_planning_diagnostic_is_a_bounded_capability_gap() {
    let diagnostic = AssistantRejectionDiagnostic {
        phase: ketchup_core::assistant_sidecar::AssistantRejectionPhase::ProposalPlanning,
        code: "planning.cad_feature_result_unsupported".into(),
        operation: "append_feature".into(),
        target: "feature:7".into(),
        failed_invariant: "unsupported exact result".into(),
        repair_hint: "use a supported exact operation".into(),
        retryable: true,
    };
    assert!(LiveBridge::is_capability_gap(&diagnostic));
    let response = Response::capability_gap(19, &diagnostic);
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("capability_gap"));
    assert_eq!(
        response.result,
        Some(json!({
            "kind": "capability_gap",
            "capability": "planning.cad_feature_result_unsupported",
            "operation": "append_feature",
            "retryable": false,
            "published": false,
        }))
    );
}

#[test]
fn image_protocol_is_versioned_declared_and_required() {
    let (mut app, mut bridge) = setup();
    let status = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    assert_eq!(status["protocol"], 1);
    assert_eq!(
        status["image_protocol"],
        json!({
            "version": IMAGE_PROTOCOL_VERSION,
            "capabilities": ["capture_mode", "capture_metadata", "render_metadata", "variable_size", "selection_framing", "detail_selection_framing"],
            "capture_modes": ["offscreen", "visible_viewport"],
            "default_capture_mode": "offscreen",
            "framing_modes": ["viewport", "selection", "detail_selection"],
            "default_framing": "viewport",
            "min_side_px": 512,
            "max_side_px": 1600,
            "default_side_px": 512,
        })
    );
    let expected = serde_json::to_value(app.live_bridge_stamp()).unwrap();
    assert!(
        serde_json::from_value::<Request>(json!({
            "method": "image",
            "expected": expected,
            "capture_mode": "offscreen",
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Request>(json!({
            "method": "image",
            "expected": expected,
            "image_protocol_version": IMAGE_PROTOCOL_VERSION,
        }))
        .is_err()
    );
    let stamp = app.live_bridge_stamp();
    assert_eq!(
        bridge.execute(
            &mut app,
            Request::Image {
                expected: Some(stamp),
                image_protocol_version: IMAGE_PROTOCOL_VERSION - 1,
                capture_mode: CaptureMode::Offscreen,
                max_side_px: MIN_IMAGE_SIDE_PX,
                framing: ImageFraming::Viewport,
                detail_target: None,
            },
            false,
        ),
        Err("unsupported_image_protocol")
    );
    for max_side_px in [MIN_IMAGE_SIDE_PX - 1, MAX_IMAGE_SIDE_PX + 1] {
        let (reply, receiver) = mpsc::sync_channel(1);
        bridge.request_image(
            &app,
            &egui::Context::default(),
            Queued {
                session: bridge.session,
                id: u64::from(max_side_px),
                request: Request::Image {
                    expected: Some(app.live_bridge_stamp()),
                    image_protocol_version: IMAGE_PROTOCOL_VERSION,
                    capture_mode: CaptureMode::Offscreen,
                    max_side_px,
                    framing: ImageFraming::Viewport,
                    detail_target: None,
                },
                connection_closed: false,
                cancelled: Arc::new(AtomicBool::new(false)),
                reply,
            },
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap().error,
            Some("invalid_image_dimensions".into())
        );
    }
}

#[test]
fn topology_query_detail_and_multi_edge_fillet_share_the_live_host_stamp() {
    let (mut app, mut bridge) = setup();
    crate::tests::install_initial_graph_result(&mut app);
    let expected = app.live_bridge_stamp();
    let page = bridge
        .execute(
            &mut app,
            Request::Query {
                expected: Some(expected.clone()),
                query: PageRequest {
                    kind: EntityKind::Edges,
                    limit: 10,
                    search: "line".into(),
                    definition_id: Some(1),
                    tag_id: None,
                    classification_dimension_id: None,
                    classification_category_id: None,
                    world_bounds_mm: None,
                    cursor: None,
                },
            },
            false,
        )
        .unwrap();
    let edges = page["items"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    let edge_id = edges[0]["id"].as_u64().unwrap();
    assert!(edge_id > 0);
    let detail = bridge
        .execute(
            &mut app,
            Request::Detail {
                expected: Some(expected.clone()),
                kind: EntityKind::Edges,
                entity_id: edge_id,
            },
            false,
        )
        .unwrap();
    assert_eq!(detail["item"], edges[0]);
    assert_eq!(detail["identity"], page["identity"]);

    let reference_ids = edges
        .iter()
        .map(|edge| edge["reference_id"].as_str().unwrap().to_owned())
        .collect();
    let proposed = bridge
        .execute(
            &mut app,
            Request::Propose {
                expected: Some(expected.clone()),
                selection: Some(vec![]),
                program: AssistantCadEditProgram {
                    operations: vec![AssistantCadEditOperation::FilletEdges {
                        definition_id: 1,
                        name: "Live multi-edge fillet".into(),
                        target_feature_id: 2,
                        edge_reference_ids: reference_ids,
                        radius_mm: 1.0,
                    }],
                },
            },
            false,
        )
        .unwrap();
    bridge
        .execute(
            &mut app,
            Request::Commit {
                expected: Some(expected.clone()),
                proposal_id: proposed["proposal_id"].as_u64().unwrap(),
            },
            false,
        )
        .unwrap();
    assert!(matches!(
        app.document.current().feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::TopologyEdgeFinish {
            target: FeatureId(2),
            kind: EdgeFinishKind::Fillet,
            edges,
            ..
        } if edges.len() == 2
    ));
}

#[test]
fn raw_preview_sketch_parameter_editor_dialog_and_anchor_are_busy_and_retained() {
    for state in 0..8 {
        let (mut app, mut bridge) = setup();
        app.selection.select_occurrence(OccurrenceId(1), true);
        let commit = proposal(&mut app, &mut bridge);
        let stamp = app.live_bridge_stamp();
        let steps = (app.undo_step_count(), app.redo_step_count());
        let primary = app.selection.primary.clone();
        let paths = app.selection.occurrences.clone();
        let camera = (app.yaw, app.pitch, app.zoom, app.pan);
        match state {
            0 => {
                app.preview = Some(CommandBatch::new(vec![]));
                assert!(!app.has_preview());
            }
            1 => {
                app.sketch_mode = true;
                app.line_chain_points.push(crate::Vec3::new(1.0, 2.0, 3.0));
                app.value_input = "unfinished sketch".into();
            }
            2 => {
                app.parameter_editor_node = Some(NodeId(999));
                app.parameter_expression_input = "human unfinished expression".into();
            }
            3 => {
                app.pocket_editor_feature = Some(FeatureId(999));
                app.pocket_depth_input = "human depth".into();
            }
            4 => {
                app.begin_definition_rename();
                app.pending_definition_rename.as_mut().unwrap().name = "unfinished rename".into();
            }
            5 => {
                app.set_move_session(
                    crate::ToolSessionPhase::Anchor,
                    crate::MoveDrag {
                        source_document_id: app.document.current().document_id(),
                        source_revision: stamp.revision,
                        occurrence_paths: BTreeSet::from([InstancePath::root(OccurrenceId(1))]),
                        selection: SelectionId {
                            definition_id: DefinitionId(1),
                            instance_path: InstancePath::root(OccurrenceId(1)),
                            element: crate::ElementId::Face {
                                axis: crate::Axis::Z,
                                side: crate::Side::Maximum,
                            },
                        },
                        group_id: None,
                        profile_target: None,
                        pointer_start_world: crate::Vec3::new(0.0, 0.0, 0.0),
                        plane_z: 0.0,
                        axis: None,
                        axis_reference: None,
                        delta_mm: crate::Vec3::new(1.0, 2.0, 3.0),
                        copy: false,
                    },
                );
            }
            6 => {
                app.preview_definition_id = Some(DefinitionId(999));
            }
            7 => {
                app.zoom_window_start = Some(egui::pos2(12.0, 34.0));
            }
            _ => unreachable!(),
        }
        let status = bridge.execute(&mut app, Request::Status {}, false).unwrap();
        assert_eq!(status["busy"], true);
        assert_eq!(status["read_only"], false);
        for request in protected_requests(&stamp, &commit) {
            assert_eq!(
                bridge.execute(&mut app, request, false),
                Err("busy"),
                "state {state}"
            );
        }
        assert_eq!(app.live_bridge_stamp(), stamp);
        assert_eq!((app.undo_step_count(), app.redo_step_count()), steps);
        assert_eq!(app.selection.primary, primary);
        assert_eq!(app.selection.occurrences, paths);
        assert_eq!((app.yaw, app.pitch, app.zoom, app.pan), camera);
        assert!(bridge.pending.is_some());
        match state {
            0 => assert!(app.preview.is_some()),
            1 => {
                assert!(app.sketch_mode);
                assert_eq!(app.line_chain_points, vec![crate::Vec3::new(1.0, 2.0, 3.0)]);
                assert_eq!(app.value_input, "unfinished sketch");
            }
            2 => {
                assert_eq!(app.parameter_editor_node, Some(NodeId(999)));
                assert_eq!(
                    app.parameter_expression_input,
                    "human unfinished expression"
                );
            }
            3 => {
                assert_eq!(app.pocket_editor_feature, Some(FeatureId(999)));
                assert_eq!(app.pocket_depth_input, "human depth");
            }
            4 => assert_eq!(
                app.pending_definition_rename.as_ref().unwrap().name,
                "unfinished rename"
            ),
            5 => assert_eq!(
                app.move_session().unwrap().0.delta_mm,
                crate::Vec3::new(1.0, 2.0, 3.0)
            ),
            6 => assert_eq!(app.preview_definition_id, Some(DefinitionId(999))),
            7 => assert_eq!(app.zoom_window_start, Some(egui::pos2(12.0, 34.0))),
            _ => unreachable!(),
        }
    }
}

#[test]
fn review_only_history_and_focused_editor_reject_mutations() {
    let (mut app, mut bridge) = setup();
    let commit = proposal(&mut app, &mut bridge);
    bridge.execute(&mut app, commit.clone(), false).unwrap();
    assert!(app.undo()); // Both Undo and Redo now have history.
    let stamp = app.live_bridge_stamp();
    let steps = (app.undo_step_count(), app.redo_step_count());
    // Presence of a review candidate is the GUI's read-only boundary.
    app.review_candidate = Some(
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&app.document.current()))
            .unwrap(),
    );
    let status = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    assert_eq!(status["read_only"], true);
    for request in [
        Request::Undo {
            expected: Some(stamp.clone()),
        },
        Request::Redo {
            expected: Some(stamp.clone()),
        },
    ] {
        assert_eq!(
            bridge.execute(&mut app, request, false),
            Err("read_only_document")
        );
    }
    app.preview = Some(CommandBatch::new(vec![]));
    // A committed proposal is never executed twice.
    assert_eq!(
        bridge.execute(&mut app, commit, true),
        Err("stale_document")
    );
    assert!(app.preview.is_some());
    assert_eq!(app.live_bridge_stamp(), stamp);
    assert_eq!((app.undo_step_count(), app.redo_step_count()), steps);
    app.review_candidate = None;
    app.preview = None;
    let new_commit = proposal(&mut app, &mut bridge);
    for request in protected_requests(&stamp, &new_commit) {
        assert_eq!(bridge.execute(&mut app, request, true), Err("busy"));
    }
    assert_eq!(
        bridge.execute(&mut app, Request::Status {}, true).unwrap()["busy"],
        true
    );
}

#[test]
fn root_scope_rejects_grouped_hidden_tag_hidden_mixed_and_explicit_selectors_atomically() {
    let (mut app, mut bridge) = setup();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(1),
                name: "group".into(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "child".into(),
                transform: Transform::identity(),
                parent: Some(GroupId(1)),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(3),
                definition_id: DefinitionId(1),
                name: "hidden".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: false,
            },
            CanonicalCommand::CreateTag {
                id: TagId(1),
                name: "hidden tag".into(),
                visible: false,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(4),
                definition_id: DefinitionId(1),
                name: "tag hidden".into(),
                transform: Transform::identity(),
                parent: None,
                tag: Some(TagId(1)),
                visible: true,
            },
        ]))
        .unwrap();
    app.selection.select_occurrence(OccurrenceId(1), true);
    let commit = proposal(&mut app, &mut bridge);
    let stamp = app.live_bridge_stamp();
    let paths = app.selection.occurrences.clone();
    let primary = app.selection.primary.clone();
    let pending_id = bridge.pending.as_ref().unwrap().id;
    for ids in [
        vec![2],
        vec![3],
        vec![4],
        vec![1, 2],
        vec![1, 3],
        vec![1, 4],
    ] {
        assert_eq!(
            bridge.execute(
                &mut app,
                Request::Selection {
                    expected: Some(stamp.clone()),
                    occurrence_ids: ids.clone()
                },
                false
            ),
            Err("unsupported_selection_scope")
        );
        let selector = AssistantCadEntitySelector::Occurrences {
            occurrence_ids: ids,
        };
        let operations = vec![
            AssistantCadEditOperation::Transform {
                selector: selector.clone(),
                translation_mm: [1.0, 0.0, 0.0],
                rotation: None,
            },
            AssistantCadEditOperation::Delete {
                selector: selector.clone(),
                dependency_policy:
                    ketchup_core::assistant_sidecar::AssistantCadDeletePolicy::RemoveReferences,
            },
            AssistantCadEditOperation::SetColor {
                selector: selector.clone(),
                color: Some([1, 2, 3]),
            },
            AssistantCadEditOperation::Copy {
                selector: selector.clone(),
                translation_mm: [1.0, 0.0, 0.0],
            },
            AssistantCadEditOperation::LinearPattern {
                selector: selector.clone(),
                instances: 2,
                step_mm: [1.0, 0.0, 0.0],
            },
            AssistantCadEditOperation::Mirror {
                selector,
                plane_origin_mm: [0.0; 3],
                plane_normal: [1.0, 0.0, 0.0],
            },
        ];
        for operation in operations {
            assert_eq!(
                bridge.execute(
                    &mut app,
                    Request::Propose {
                        expected: Some(stamp.clone()),
                        selection: Some(vec![1]),
                        program: AssistantCadEditProgram {
                            operations: vec![operation]
                        }
                    },
                    false
                ),
                Err("unsupported_selection_scope")
            );
        }
        assert_eq!(app.live_bridge_stamp(), stamp);
        assert_eq!(app.selection.occurrences, paths);
        assert_eq!(app.selection.primary, primary);
        assert_eq!(bridge.pending.as_ref().unwrap().id, pending_id);
    }
    // A raw grouped child masquerading as a root path is rejected too.
    app.selection.clear();
    app.selection.select_occurrence(OccurrenceId(2), true);
    assert_eq!(
        LiveBridge::selection(&app),
        Err("unsupported_selection_scope")
    );
    assert_eq!(
        bridge.execute(&mut app, commit, false),
        Err("unsupported_selection_scope")
    );
}

struct Wire {
    app: KetchupApp,
    context: egui::Context,
    stream: TcpStream,
    token: String,
    id: u64,
}
impl Wire {
    fn new() -> Self {
        let mut app = KetchupApp::new();
        app.selection.clear();
        let context = egui::Context::default();
        let address = app.enable_live_bridge(&context).unwrap();
        let token = app.live_bridge_credentials().unwrap().token;
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        Self {
            app,
            context,
            stream,
            token,
            id: 0,
        }
    }
    fn send(&mut self, request: Request, extra: bool) {
        self.id += 1;
        let bytes = serde_json::to_vec(&Envelope {
            version: 1,
            id: self.id,
            token: self.token.clone(),
            request,
        })
        .unwrap();
        let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
        frame.extend(bytes);
        if extra {
            frame.push(0);
        }
        self.stream.write_all(&frame).unwrap();
    }
    fn call(&mut self, request: Request) -> Response {
        self.send(request, false);
        let mut reader = self.stream.try_clone().unwrap();
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let mut header = [0; 4];
            reader.read_exact(&mut header).unwrap();
            let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
            reader.read_exact(&mut bytes).unwrap();
            tx.send(serde_json::from_slice::<Response>(&bytes).unwrap())
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(4);
        let response = loop {
            if let Ok(response) = rx.try_recv() {
                break response;
            }
            assert!(Instant::now() < deadline);
            self.app.poll_live_bridge(&self.context);
            std::thread::sleep(Duration::from_millis(5));
        };
        handle.join().unwrap();
        assert!(response.ok, "{:?}", response.error);
        response
    }
    fn closed(&mut self, timeout: Duration) {
        self.stream.set_read_timeout(Some(timeout)).unwrap();
        let mut byte = [0];
        match self.stream.read(&mut byte) {
            Ok(0) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::BrokenPipe
                ) => {}
            other => panic!("expected prompt close, got {other:?}"),
        }
    }
    fn propose(&mut self) -> Request {
        let expected = self.app.live_bridge_stamp();
        let response = self.call(Request::Propose {
            expected: Some(expected.clone()),
            selection: Some(vec![]),
            program: program(),
        });
        Request::Commit {
            expected: Some(expected),
            proposal_id: response.result.unwrap()["proposal_id"].as_u64().unwrap(),
        }
    }
}

fn call_stream(
    app: &mut KetchupApp,
    context: &egui::Context,
    stream: &mut TcpStream,
    token: &str,
    id: u64,
    request: Request,
) -> Response {
    let bytes = serde_json::to_vec(&Envelope {
        version: 1,
        id,
        token: token.to_owned(),
        request,
    })
    .unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
    let mut reader = stream.try_clone().unwrap();
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let mut header = [0; 4];
        reader.read_exact(&mut header).unwrap();
        let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
        reader.read_exact(&mut bytes).unwrap();
        tx.send(serde_json::from_slice::<Response>(&bytes).unwrap())
            .unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(4);
    let response = loop {
        if let Ok(response) = rx.try_recv() {
            break response;
        }
        assert!(Instant::now() < deadline);
        app.poll_live_bridge(context);
        std::thread::sleep(Duration::from_millis(5));
    };
    handle.join().unwrap();
    response
}

#[test]
fn hundred_authenticated_disconnect_cycles_leave_host_and_registries_clean() {
    let mut app = KetchupApp::new();
    app.selection.clear();
    let context = egui::Context::default();
    let address = app.enable_live_bridge(&context).unwrap();
    let token = app.live_bridge_credentials().unwrap().token;
    let document_id = app.live_bridge_stamp().document_id;

    for _ in 0..100 {
        let mut stream = TcpStream::connect(address).unwrap();
        assert!(
            call_stream(
                &mut app,
                &context,
                &mut stream,
                &token,
                1,
                Request::Status {},
            )
            .ok
        );
        assert!(
            call_stream(
                &mut app,
                &context,
                &mut stream,
                &token,
                2,
                Request::Disconnect {},
            )
            .ok
        );
        drop(stream);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            app.poll_live_bridge(&context);
            let bridge = app.live_bridge.as_ref().unwrap();
            if bridge.active_connections.load(Ordering::Acquire) == 0
                && bridge.session == 0
                && bridge.client_states.is_empty()
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    assert_eq!(app.live_bridge_stamp().document_id, document_id);
    assert!(app.live_bridge.is_some());
}

#[test]
fn multiple_clients_reconnect_same_host_and_cannot_cross_window_boundary() {
    let mut window_a = Wire::new();
    let mut window_b = Wire::new();
    let address_a = window_a.stream.peer_addr().unwrap();
    let address_b = window_b.stream.peer_addr().unwrap();
    let token_a = window_a.token.clone();
    let before_b = window_b.app.live_bridge_stamp();

    assert!(window_a.call(Request::Status {}).ok);
    let mut peer_a = TcpStream::connect(address_a).unwrap();
    assert!(
        call_stream(
            &mut window_a.app,
            &window_a.context,
            &mut peer_a,
            &token_a,
            1,
            Request::Status {},
        )
        .ok
    );
    let expected = window_a.app.live_bridge_stamp();
    let proposed = call_stream(
        &mut window_a.app,
        &window_a.context,
        &mut peer_a,
        &token_a,
        2,
        Request::Propose {
            expected: Some(expected.clone()),
            selection: Some(vec![]),
            program: program(),
        },
    );
    assert!(proposed.ok);
    assert!(window_a.call(Request::Disconnect {}).ok);
    let committed = call_stream(
        &mut window_a.app,
        &window_a.context,
        &mut peer_a,
        &token_a,
        3,
        Request::Commit {
            expected: Some(expected),
            proposal_id: proposed.result.unwrap()["proposal_id"].as_u64().unwrap(),
        },
    );
    assert!(committed.ok);
    assert_eq!(window_b.app.live_bridge_stamp(), before_b);
    assert!(
        call_stream(
            &mut window_a.app,
            &window_a.context,
            &mut peer_a,
            &token_a,
            4,
            Request::Status {},
        )
        .ok
    );

    drop(peer_a);
    let mut reconnected = TcpStream::connect(address_a).unwrap();
    let reconnected_status = call_stream(
        &mut window_a.app,
        &window_a.context,
        &mut reconnected,
        &token_a,
        1,
        Request::Status {},
    );
    assert_eq!(
        reconnected_status.stamp.as_ref().unwrap().document_id,
        window_a.app.live_bridge_stamp().document_id
    );
    let mut wrong_window = TcpStream::connect(address_b).unwrap();
    let rejected = call_stream(
        &mut window_b.app,
        &window_b.context,
        &mut wrong_window,
        &token_a,
        1,
        Request::Status {},
    );
    assert_eq!(rejected.error.as_deref(), Some("unauthorized"));
    assert_eq!(window_b.app.live_bridge_stamp(), before_b);
    assert!(window_b.call(Request::Status {}).ok);
}

#[test]
fn authenticated_thinking_before_propose_and_commit_survives_and_disable_closes_idle() {
    let mut wire = Wire::new();
    wire.call(Request::Status {});
    std::thread::sleep(Duration::from_millis(2200));
    let commit = wire.propose();
    let before = wire.app.live_bridge_stamp();
    std::thread::sleep(Duration::from_millis(2200));
    wire.call(commit);
    assert_ne!(wire.app.live_bridge_stamp(), before);
    wire.app.disable_live_bridge();
    wire.closed(Duration::from_millis(500));
}

#[test]
fn preauth_idle_and_partial_header_body_deadlines_close_without_ui_mutation() {
    let mut preauth = Wire::new();
    let start = Instant::now();
    preauth.closed(Duration::from_secs(4));
    assert!(start.elapsed() >= Duration::from_millis(1800));
    let mut partial = Wire::new();
    partial.call(Request::Status {});
    let before = partial.app.live_bridge_stamp();
    // Authenticated FIRST byte starts the deadline, not completion of header.
    partial.stream.write_all(&[0]).unwrap();
    std::thread::sleep(Duration::from_millis(1150));
    partial.stream.write_all(&[0, 0, 20, b'{']).unwrap();
    partial.closed(Duration::from_millis(1400));
    partial.app.poll_live_bridge(&partial.context);
    assert_eq!(partial.app.live_bridge_stamp(), before);
    let mut header = Wire::new();
    header.stream.write_all(&[0, 0]).unwrap();
    header.closed(Duration::from_secs(4));
}

#[test]
fn pipelined_byte_and_disconnect_before_ui_poll_revoke_commit() {
    for disconnect in [false, true] {
        let mut wire = Wire::new();
        let commit = wire.propose();
        let before = wire.app.live_bridge_stamp();
        let steps = wire.app.undo_step_count();
        wire.send(commit, true);
        if disconnect {
            wire.stream.shutdown(Shutdown::Write).unwrap();
        }
        // Deliberately DO NOT poll UI until the worker has cancelled and closed.
        wire.closed(Duration::from_secs(2));
        wire.app.poll_live_bridge(&wire.context);
        assert_eq!(wire.app.live_bridge_stamp(), before);
        assert_eq!(wire.app.undo_step_count(), steps);
    }
}
