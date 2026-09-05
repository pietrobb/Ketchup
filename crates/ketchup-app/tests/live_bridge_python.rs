//! Real Rust GUI store -> TCP -> Python LiveSession -> registered beta tool.call.
//! Requires KETCHUP_LIVE_PYTHON pointing at Python 3.11+ with anthropic installed.
//! Only an unset variable skips; a missing/broken dependency is a test failure.
//! This is trusted host attachment integration, NOT production launcher proof.
//! Shell is offscreen AccessKit/egui_kittest: no desktop, renderer or image proof.
mod harness;

use harness::Shell;
use ketchup_app::{AppCommand, live_bridge::Stamp};
use ketchup_core::assistant_sidecar::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
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
        operations: vec![AssistantCadEditOperation::CreatePart {
            name: "Python live cylinder".into(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 12.0,
            }],
            constraints: vec![AssistantSketchConstraint::Radius {
                id: 1,
                entity_id: 1,
                value_mm: 12.0,
            }],
            feature: AssistantCadPartFeature::Extrusion { distance_mm: 30.0 },
            translation_mm: [5.0, 6.0, 7.0],
            rotation: None,
        }],
    }
}

#[test]
fn registered_python_skill_uses_same_gui_store_and_human_history() {
    let Some(python) = std::env::var_os("KETCHUP_LIVE_PYTHON") else {
        eprintln!("SKIP: set KETCHUP_LIVE_PYTHON to Python 3.11+ with anthropic installed");
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let mut shell = Shell::new();
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
            "Python helper failed or checkpoint out of order at {checkpoint}"
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
                assert_eq!(actual_count, count + 1);
                assert!(actual.revision > initial.revision);
                assert!(actual.mutation_epoch > initial.mutation_epoch);
                assert_eq!(actual.document_id, initial.document_id);
                assert_eq!(shell.app().undo_step_count(), history + 1);
                committed = Some(actual);
            }
            "aba_ready" => {
                assert_eq!(Some(&actual), committed.as_ref());
                assert_eq!(actual_count, count + 1);
                shell.click_command(AppCommand::Undo);
                assert_eq!(shell.app().document_snapshot().occurrences().count(), count);
                let human_undo = shell.app().live_bridge_stamp();
                assert_eq!(human_undo.revision, initial.revision);
                assert!(human_undo.mutation_epoch > actual.mutation_epoch);
                shell.click_command(AppCommand::Redo);
                assert_eq!(
                    shell.app().document_snapshot().occurrences().count(),
                    count + 1
                );
                let restored = shell.app().live_bridge_stamp();
                assert_eq!(restored.revision, actual.revision);
                assert_eq!(restored.canonical_digest, actual.canonical_digest);
                assert!(restored.mutation_epoch > human_undo.mutation_epoch);
                after_human_history = Some(restored);
            }
            "stale_rejected" => {
                assert_eq!(Some(&actual), after_human_history.as_ref());
                assert_eq!(actual_count, count + 1);
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
                assert_eq!(actual_count, count + 1);
                assert_eq!(actual.revision, committed.as_ref().unwrap().revision);
                assert_eq!(
                    actual.canonical_digest,
                    committed.as_ref().unwrap().canonical_digest
                );
                assert!(actual.mutation_epoch > undone.as_ref().unwrap().mutation_epoch);
                committed = Some(actual);
            }
            "image_renderer_unavailable" | "disconnected" => {
                assert_eq!(Some(&actual), committed.as_ref());
                assert_eq!(actual_count, count + 1);
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
        count + 1
    );
}
