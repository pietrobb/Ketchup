//! Real Rust GUI store -> TCP -> Python LiveSession -> registered beta tool.call.
//! Requires KETCHUP_LIVE_PYTHON pointing at Python 3.11+ with anthropic installed.
//! These explicit integration tests run through scripts/run_production_tests.py;
//! an absent or broken configured runtime is a test failure, never a false pass.
//! This is trusted host attachment integration, NOT production launcher proof.
//! Shell is offscreen AccessKit/egui_kittest: no desktop, renderer or image proof.
mod harness;

use harness::Shell;
use ketchup_app::{AppCommand, live_bridge::Stamp};
use ketchup_core::{assistant_sidecar::*, document::FeatureKind};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

// Never derive Debug: the child owns the private credential pipe.
struct Python(Child);
impl Drop for Python {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn program() -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::CreateHelix {
                name: "Python live helix".into(),
                parameters: AssistantHelixParameters {
                    axis: AssistantAxisSpec::TwoPoints {
                        start_mm: [4.0, -3.0, 2.0],
                        end_mm: [5.0, -1.0, 5.0],
                    },
                    radius_mm: 7.0,
                    pitch_mm: 4.5,
                    turns: 2.25,
                    start_angle_degrees: 37.0,
                    handedness: AssistantHelixHandedness::Left,
                },
            },
            AssistantCadEditOperation::CreateThread {
                name: "Python live thread".into(),
                parameters: AssistantThreadParameters {
                    helix: AssistantHelixParameters {
                        axis: AssistantAxisSpec::OriginDirection {
                            origin_mm: [28.0, 0.0, 0.0],
                            direction: [0.35, 0.2, 1.0],
                        },
                        radius_mm: 8.0,
                        pitch_mm: 6.0,
                        turns: 2.0,
                        start_angle_degrees: 15.0,
                        handedness: AssistantHelixHandedness::Right,
                    },
                    profile_radius_mm: 0.65,
                    profile: AssistantThreadProfile::V,
                },
            },
        ],
    }
}

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

fn wait_for_exact_body(shell: &mut Shell) {
    for _ in 0..2000 {
        shell.step();
        shell.settle();
        if shell.app().exact_render_body_count() == 1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(shell.app().exact_render_body_count(), 1);
}

#[test]
#[ignore = "run via scripts/run_production_tests.py with a required real Python runtime"]
fn registered_disconnect_releases_consent_and_allows_reattach() {
    let python = std::env::var_os("KETCHUP_LIVE_PYTHON")
        .expect("KETCHUP_LIVE_PYTHON must identify the provisioned Python 3.11+ runtime");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = tempfile::tempdir().unwrap();
    let mut shell = Shell::new();
    let broker = shell
        .app_mut()
        .enable_live_consent_broker_in(&eframe::egui::Context::default(), directory.path())
        .unwrap();
    shell.step();
    let instance_id = shell.app().live_consent_instance_id().unwrap().to_owned();
    let stamp = shell.app().live_bridge_stamp();
    let history = shell.app().undo_step_count();
    let mut child = Python(
        Command::new(python)
            .args(["-B", "-u"])
            .arg(root.join("tests/live_bridge_skill_client.py"))
            .arg("consent-disconnect")
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start explicitly configured Python + anthropic"),
    );
    let mut input = child.0.stdin.take().unwrap();
    writeln!(
        input,
        "{}",
        serde_json::json!({
            "discovery_root": directory.path(), "instance_id": instance_id
        })
    )
    .unwrap();
    let output = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    for expected in [
        "attached_0",
        "disconnected_0",
        "attached_1",
        "disconnected_1",
        "finished",
    ] {
        let message = loop {
            assert!(Instant::now() < deadline, "timeout at {expected}");
            shell.step();
            match rx.try_recv() {
                Ok(line) => break line.expect("read checkpoint"),
                Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
                Err(mpsc::TryRecvError::Disconnected) => panic!("child exited at {expected}"),
            }
        };
        let message: serde_json::Value = serde_json::from_str(&message).unwrap();
        assert_eq!(
            message["checkpoint"], expected,
            "helper lines: {}",
            message["lines"]
        );
        assert_eq!(message["stamp"], serde_json::to_value(&stamp).unwrap());
        assert_eq!(shell.app().live_bridge_stamp(), stamp);
        assert_eq!(shell.app().undo_step_count(), history);
        assert_eq!(shell.app().live_consent_address(), Some(broker));
        assert_eq!(
            shell.app().live_consent_instance_id(),
            Some(instance_id.as_str())
        );
        let attached = expected.starts_with("attached_");
        assert_eq!(shell.app().live_consent_attached(), attached);
        assert_eq!(shell.app().live_bridge_credentials().is_some(), attached);
        // Publish discovery availability before allowing the next SDK list/attach.
        shell.step();
        input.write_all(b"continue\n").unwrap();
    }
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "child exit timeout");
        std::thread::sleep(Duration::from_millis(5));
    }
    reader.join().unwrap();
}

#[test]
#[ignore = "run via scripts/run_production_tests.py with a required real Python runtime"]
fn registered_python_skill_uses_same_gui_store_and_human_history() {
    let python = std::env::var_os("KETCHUP_LIVE_PYTHON")
        .expect("KETCHUP_LIVE_PYTHON must identify the provisioned Python 3.11+ runtime");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let persistence_path = directory.path().join("live-bridge-roundtrip.ketchup");
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the real exact worker is required");
    wait_for_exact_body(&mut shell);
    assert!(shell.app().live_bridge_credentials().is_none());
    let address = shell
        .app_mut()
        .enable_live_bridge(&eframe::egui::Context::default())
        .unwrap();
    assert!(address.ip().is_loopback());
    let credentials = shell.app().live_bridge_credentials().unwrap();
    let initial = shell.app().live_bridge_stamp();
    let count = shell.app().document_snapshot().occurrences().count();
    let history = shell.app().undo_step_count();

    let mut child = Python(
        Command::new(python)
            .arg("-B")
            .arg("-u")
            .arg(root.join("tests/live_bridge_skill_client.py"))
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start explicitly configured Python + anthropic"),
    );
    let mut input = child.0.stdin.take().unwrap();
    // Address/token never appear in argv, environment, files, logs or assertions.
    let mut attachment = serde_json::to_vec(&serde_json::json!({
        "address": credentials.address.to_string(),
        "token": credentials.token,
        "program": program(),
        "persistence_path": persistence_path,
    }))
    .expect("encode private host attachment");
    attachment.push(b'\n');
    assert!(
        input.write_all(&attachment).is_ok(),
        "write private host attachment"
    );
    attachment.fill(0);
    drop(attachment);

    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let out_thread = std::thread::spawn(move || {
        // Bound all child output, including accidental logging; never relay it.
        let reader = BufReader::new(stdout.take(1024 * 1024));
        for line in reader.lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr.take(1024 * 1024).read_to_end(&mut bytes);
        (result.is_ok(), bytes)
    });
    let mut committed: Option<Stamp> = None;
    let mut after_human_history: Option<Stamp> = None;
    let mut undone: Option<Stamp> = None;
    for checkpoint in [
        "initial",
        "plan_guarded",
        "proposed",
        "committed",
        "aba_ready",
        "stale_rejected",
        "undone",
        "redone",
        "saved",
        "reopened",
        "image_renderer_unavailable",
        "disconnected",
    ] {
        let deadline = Instant::now() + Duration::from_secs(45);
        let line = loop {
            match rx.try_recv() {
                Ok(Ok(line)) => break line,
                Ok(Err(_)) => panic!("Python output read failed at {checkpoint}"),
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("Python exited before {checkpoint}")
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "Python checkpoint deadline: {checkpoint}"
            );
            shell.step();
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(
            !line.contains(&credentials.token),
            "credential leaked in Python stdout"
        );
        assert!(
            !line.contains(&credentials.address.to_string()),
            "endpoint leaked in Python stdout"
        );
        let event: serde_json::Value = serde_json::from_str(&line)
            .unwrap_or_else(|_| panic!("invalid sanitized checkpoint: {checkpoint}"));
        assert!(
            event["checkpoint"].as_str() == Some(checkpoint),
            "Python helper failed or checkpoint out of order at {checkpoint}; helper lines: {}",
            event["lines"]
        );
        let observed: Stamp = serde_json::from_value(event["stamp"].clone())
            .unwrap_or_else(|_| panic!("missing checkpoint stamp at {checkpoint}"));
        let actual = shell.app().live_bridge_stamp();
        assert_eq!(
            observed, actual,
            "Python and GUI stamps differ at {checkpoint}"
        );
        let actual_count = shell.app().document_snapshot().occurrences().count();
        match checkpoint {
            "initial" | "plan_guarded" | "proposed" => {
                assert_eq!(actual, initial, "read/plan/propose mutated GUI store");
                assert_eq!(actual_count, count);
                assert_eq!(shell.app().undo_step_count(), history);
            }
            "committed" => {
                assert_eq!(actual_count, count + 2);
                assert!(actual.revision > initial.revision);
                assert!(actual.mutation_epoch > initial.mutation_epoch);
                assert_eq!(actual.document_id, initial.document_id);
                assert_eq!(shell.app().undo_step_count(), history + 1);
                assert_eq!(
                    shell
                        .app()
                        .document_snapshot()
                        .features()
                        .filter(|feature| matches!(
                            feature.kind(),
                            FeatureKind::TopologyEdgeFinish { edges, .. } if edges.len() == 1
                        ))
                        .count(),
                    1,
                    "the host-issued live edge reference must reach canonical history"
                );
                committed = Some(actual);
            }
            "aba_ready" => {
                assert_eq!(Some(&actual), committed.as_ref());
                assert_eq!(actual_count, count + 2);
                shell.click_command(AppCommand::Undo);
                assert_eq!(shell.app().document_snapshot().occurrences().count(), count);
                let human_undo = shell.app().live_bridge_stamp();
                assert_eq!(human_undo.revision, initial.revision);
                assert!(human_undo.mutation_epoch > actual.mutation_epoch);
                shell.click_command(AppCommand::Redo);
                assert_eq!(
                    shell.app().document_snapshot().occurrences().count(),
                    count + 2
                );
                let restored = shell.app().live_bridge_stamp();
                assert_eq!(restored.revision, actual.revision);
                assert_eq!(restored.canonical_digest, actual.canonical_digest);
                assert!(restored.mutation_epoch > human_undo.mutation_epoch);
                after_human_history = Some(restored);
            }
            "stale_rejected" => {
                assert_eq!(Some(&actual), after_human_history.as_ref());
                assert_eq!(actual_count, count + 2);
                assert_eq!(shell.app().undo_step_count(), history + 1);
            }
            "undone" => {
                assert_eq!(actual_count, count);
                assert_eq!(actual.revision, initial.revision);
                assert_eq!(actual.canonical_digest, initial.canonical_digest);
                assert!(
                    actual.mutation_epoch > after_human_history.as_ref().unwrap().mutation_epoch
                );
                undone = Some(actual);
            }
            "redone" => {
                assert_eq!(actual_count, count + 2);
                assert_eq!(actual.revision, committed.as_ref().unwrap().revision);
                assert_eq!(
                    actual.canonical_digest,
                    committed.as_ref().unwrap().canonical_digest
                );
                assert!(actual.mutation_epoch > undone.as_ref().unwrap().mutation_epoch);
                committed = Some(actual);
            }
            "saved" => {
                assert_eq!(Some(&actual), committed.as_ref());
                assert_eq!(actual_count, count + 2);
                assert!(persistence_path.is_file());
            }
            "reopened" => {
                let before_reopen = committed.as_ref().unwrap();
                assert_eq!(actual.document_id, before_reopen.document_id);
                assert_eq!(actual.revision, before_reopen.revision);
                assert_eq!(actual.canonical_digest, before_reopen.canonical_digest);
                assert_eq!(actual_count, count + 2);
                committed = Some(actual);
            }
            "image_renderer_unavailable" | "disconnected" => {
                assert_eq!(Some(&actual), committed.as_ref());
                assert_eq!(actual_count, count + 2);
            }
            _ => unreachable!(),
        }
        assert!(
            input.write_all(b"continue\n").is_ok(),
            "checkpoint acknowledgement failed"
        );
    }
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().expect("wait for Python") {
            break status;
        }
        assert!(Instant::now() < deadline, "Python exit deadline");
        shell.step();
        std::thread::sleep(Duration::from_millis(5));
    };
    out_thread.join().expect("stdout collector panicked");
    let (read_ok, stderr) = err_thread.join().expect("stderr collector panicked");
    assert!(read_ok, "Python stderr read failed");
    assert!(
        !stderr
            .windows(credentials.token.len())
            .any(|w| w == credentials.token.as_bytes()),
        "credential leaked in Python stderr"
    );
    assert!(
        stderr.is_empty(),
        "Python emitted unexpected stderr (suppressed)"
    );
    assert!(
        rx.try_recv().is_err(),
        "Python emitted unexpected trailing stdout (suppressed)"
    );
    assert!(status.success(), "Python helper failed (output suppressed)");

    // After skill disconnect AND Python exit, the actual GUI still works.
    shell.step();
    assert_eq!(Some(shell.app().live_bridge_stamp()), committed);
    shell.click_command(AppCommand::Undo);
    assert_eq!(shell.app().document_snapshot().occurrences().count(), count);
    shell.click_command(AppCommand::Redo);
    assert_eq!(
        shell.app().document_snapshot().occurrences().count(),
        count + 2
    );
}

#[test]
#[ignore = "run via scripts/run_production_tests.py with a required real Python runtime"]
fn registered_python_skill_runs_bounded_model_workflow_in_gui_document() {
    let python = std::env::var_os("KETCHUP_LIVE_PYTHON")
        .expect("KETCHUP_LIVE_PYTHON must identify the provisioned Python 3.11+ runtime");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let mut shell = Shell::new();
    let (target_id, target_name) = shell
        .app()
        .document_snapshot()
        .occurrences()
        .next()
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .unwrap();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the real exact worker is required");
    wait_for_exact_body(&mut shell);
    let assembly_title = shell.catalog().text("assembly-title");
    shell.click_button_label(&assembly_title);
    let ground = shell.catalog().format(
        "assembly-preview-ground",
        &std::collections::BTreeMap::from([("name", target_name)]),
    );
    shell.click_button_label(&ground);
    assert!(shell.app().assembly_preview_pending());
    let confirm = shell.catalog().text("assembly-confirm-preview");
    shell.click_button_label(&confirm);
    assert_eq!(shell.app().grounded_occurrence_count(), 1);
    shell
        .app_mut()
        .enable_live_bridge(&eframe::egui::Context::default())
        .unwrap();
    let credentials = shell.app().live_bridge_credentials().unwrap();
    let initial = shell.app().live_bridge_stamp();
    let count = shell.app().document_snapshot().occurrences().count();
    let history = shell.app().undo_step_count();

    let mut child = Python(
        Command::new(python)
            .arg("-B")
            .arg("-u")
            .arg(root.join("tests/live_bridge_skill_client.py"))
            .arg("model-workflow")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start explicitly configured Python + anthropic"),
    );
    let mut input = child.0.stdin.take().unwrap();
    let mut attachment = serde_json::to_vec(&serde_json::json!({
        "address": credentials.address.to_string(),
        "token": credentials.token,
        "program": AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::SetColor {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: vec![target_id.0],
                },
                color: Some([73, 109, 151]),
            }],
        },
        "target": {
            "root_occurrence_id": target_id.0,
            "steps": [],
        },
    }))
    .unwrap();
    attachment.push(b'\n');
    input.write_all(&attachment).unwrap();
    attachment.fill(0);

    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let out_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout.take(1024 * 1024)).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr.take(1024 * 1024).read_to_end(&mut bytes);
        (result.is_ok(), bytes)
    });
    let mut committed = None;
    for checkpoint in ["model_context", "model_committed", "model_disconnected"] {
        let deadline = Instant::now() + Duration::from_secs(45);
        let line = loop {
            match rx.try_recv() {
                Ok(Ok(line)) => break line,
                Ok(Err(_)) => panic!("Python output read failed at {checkpoint}"),
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("Python exited before {checkpoint}")
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "Python checkpoint deadline: {checkpoint}"
            );
            shell.step();
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(!line.contains(&credentials.token));
        assert!(!line.contains(&credentials.address.to_string()));
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(event["checkpoint"].as_str(), Some(checkpoint), "{event}");
        let observed: Stamp = serde_json::from_value(event["stamp"].clone()).unwrap();
        let actual = shell.app().live_bridge_stamp();
        assert_eq!(observed, actual);
        assert_eq!(shell.app().document_snapshot().occurrences().count(), count);
        match checkpoint {
            "model_context" => {
                assert_eq!(actual, initial);
                assert_eq!(shell.app().undo_step_count(), history);
            }
            "model_committed" => {
                assert_eq!(
                    event["evidence"]["modeling_trace"],
                    serde_json::json!(["edit_context", "apply_and_verify"])
                );
                assert_eq!(event["evidence"]["modeling_round_trips"], 2);
                assert_eq!(event["evidence"]["discovery_round_trips"], 0);
                assert_eq!(event["evidence"]["retry_round_trips"], 0);
                assert_eq!(event["evidence"]["compile_or_test_processes"], 0);
                assert!(actual.revision > initial.revision);
                assert!(actual.mutation_epoch > initial.mutation_epoch);
                assert_eq!(shell.app().undo_step_count(), history + 1);
                assert_eq!(
                    shell
                        .app()
                        .document_snapshot()
                        .occurrence(target_id)
                        .unwrap()
                        .color(),
                    Some([73, 109, 151])
                );
                committed = Some(actual);
            }
            "model_disconnected" => assert_eq!(Some(&actual), committed.as_ref()),
            _ => unreachable!(),
        }
        input.write_all(b"continue\n").unwrap();
    }
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "Python exit deadline");
        shell.step();
        std::thread::sleep(Duration::from_millis(5));
    };
    out_thread.join().unwrap();
    let (read_ok, stderr) = err_thread.join().unwrap();
    assert!(read_ok && stderr.is_empty(), "Python stderr was not empty");
    assert!(status.success(), "Python model workflow failed");
    assert_eq!(Some(shell.app().live_bridge_stamp()), committed);
}
