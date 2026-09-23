use super::*;
use egui_kittest::kittest::Queryable;
use ketchup_application::evaluation::exact_worker_candidates;
use ketchup_core::document::{DocumentStore, MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec};

fn mesh_wire() -> Wire {
    let mut wire = Wire::new();
    wire.app.document = DocumentStore::new();
    wire.app
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Mesh definition".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Mesh".into(),
                kind: FeatureKind::MeshBody(MeshBodySpec {
                    schema: MESH_BODY_SCHEMA_V1.into(),
                    vertices_mm: vec![
                        [0.0, 0.0, 0.0],
                        [12.0, 0.0, 0.0],
                        [12.0, 8.0, 0.0],
                        [0.0, 8.0, 0.0],
                        [0.0, 0.0, 5.0],
                        [12.0, 0.0, 5.0],
                        [12.0, 8.0, 5.0],
                        [0.0, 8.0, 5.0],
                    ],
                    triangles: vec![
                        [0, 2, 1],
                        [0, 3, 2],
                        [4, 5, 6],
                        [4, 6, 7],
                        [0, 1, 5],
                        [0, 5, 4],
                        [1, 2, 6],
                        [1, 6, 5],
                        [2, 3, 7],
                        [2, 7, 6],
                        [3, 0, 4],
                        [3, 4, 7],
                    ],
                    authority: MeshAuthority::Authored {
                        provenance: "live conversion regression".into(),
                    },
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Mesh occurrence".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    wire.app.selection.select_occurrence(OccurrenceId(1), true);
    wire.app.exact_worker_path = Some(
        exact_worker_candidates()
            .into_iter()
            .find(|path| path.is_file())
            .expect("build ketchup-exact-worker before this test"),
    );
    wire.app.exact_worker_attempted = true;
    wire
}

fn call(harness: &mut egui_kittest::Harness<'_, Wire>, request: Request) -> Response {
    harness.state_mut().send(request, false);
    let expected_id = harness.state().id;
    let mut reader = harness.state().stream.try_clone().unwrap();
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
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
        assert!(Instant::now() < deadline, "live response deadline");
        harness.step();
        std::thread::sleep(Duration::from_millis(2));
    };
    thread.join().unwrap();
    assert_eq!(response.id, expected_id);
    response
}

fn protects_mesh_conversion(pending_review: bool, confirm: bool) {
    let mut wire = mesh_wire();
    let stamp = wire.app.live_bridge_stamp();
    let proposed = wire.call(Request::Propose {
        expected: Some(stamp.clone()),
        selection: Some(vec![1]),
        program: program(),
    });
    let commit = Request::Commit {
        expected: Some(stamp.clone()),
        proposal_id: proposed.result.unwrap()["proposal_id"].as_u64().unwrap(),
    };
    let history = (
        wire.app.undo_step_count(),
        wire.app.redo_step_count(),
        wire.app.is_dirty(),
    );
    let confirm_label = wire.app.catalog.text("dialog-mesh-conversion-confirm");
    let cancel_label = wire.app.catalog.text("dialog-mesh-conversion-cancel");
    wire.app.begin_mesh_conversion_review();
    assert!(wire.app.mesh_conversion_active(), "{}", wire.app.digest);
    let mut harness = egui_kittest::Harness::new_state(
        |context, wire: &mut Wire| {
            wire.app.poll_live_bridge(context);
            // Hold task events until explicitly polled, so both raw task and pending
            // review guards are tested deterministically, independently of worker speed.
            wire.app.show_mesh_conversion_window(context);
        },
        wire,
    );
    if pending_review {
        let deadline = Instant::now() + Duration::from_secs(30);
        while harness.query_by_label(&confirm_label).is_none() {
            assert!(
                Instant::now() < deadline,
                "conversion verification deadline"
            );
            let wire = harness.state_mut();
            wire.app.poll_mesh_conversion(&wire.context);
            assert!(wire.app.mesh_conversion_active(), "{}", wire.app.digest);
            harness.step();
            std::thread::sleep(Duration::from_millis(5));
        }
    } else {
        assert!(harness.query_by_label(&confirm_label).is_none());
    }
    let status = call(&mut harness, Request::Status {});
    assert!(status.ok);
    assert_eq!(status.result.unwrap()["busy"], true);
    let mut requests = protected_requests(&stamp, &commit);
    requests.extend([
        Request::SaveAs {
            expected: Some(stamp.clone()),
            path: "not-saved.ketchup".into(),
        },
        Request::Open {
            expected: Some(stamp.clone()),
            path: "not-opened.ketchup".into(),
        },
        Request::Save {
            expected: Some(stamp.clone()),
        },
    ]);
    for request in requests {
        let response = call(&mut harness, request);
        assert!(!response.ok, "mutation unexpectedly accepted: {response:?}");
        assert_eq!(response.error.as_deref(), Some("busy"));
        let app = &harness.state().app;
        assert_eq!(app.live_bridge_stamp(), stamp);
        assert_eq!(
            (app.undo_step_count(), app.redo_step_count(), app.is_dirty()),
            history
        );
        assert!(app.mesh_conversion_active());
        assert_eq!(
            app.selected_occurrence_ids(),
            BTreeSet::from([OccurrenceId(1)])
        );
        assert_eq!(
            harness.query_by_label(&confirm_label).is_some(),
            pending_review
        );
        assert!(harness.query_by_label(&cancel_label).is_some());
    }
    harness
        .get_by_label(if confirm {
            &confirm_label
        } else {
            &cancel_label
        })
        .click();
    harness.run();
    assert!(!harness.state().app.mesh_conversion_active());
    let status = call(&mut harness, Request::Status {});
    assert!(status.ok);
    assert_eq!(status.result.unwrap()["busy"], false);
    if confirm {
        let app = &mut harness.state_mut().app;
        assert_ne!(app.live_bridge_stamp(), stamp);
        assert_eq!(app.undo_step_count(), history.0 + 1);
        assert!(!matches!(
            app.document.current().feature(FeatureId(2)).unwrap().kind(),
            FeatureKind::MeshBody(_)
        ));
        assert!(app.undo());
        assert_eq!(
            app.document.current().canonical_digest(),
            stamp.canonical_digest
        );
        assert!(app.redo());
        assert!(!matches!(
            app.document.current().feature(FeatureId(2)).unwrap().kind(),
            FeatureKind::MeshBody(_)
        ));
    } else {
        assert_eq!(harness.state().app.live_bridge_stamp(), stamp);
        assert_eq!(harness.state().app.undo_step_count(), history.0);
        let response = call(&mut harness, commit);
        assert!(response.ok, "{:?}", response.error);
        assert_eq!(harness.state().app.undo_step_count(), history.0 + 1);
    }
}

#[test]
fn mesh_conversion_task_blocks_authenticated_mutations_until_cancel() {
    protects_mesh_conversion(false, false);
}

#[test]
fn mesh_conversion_pending_review_blocks_authenticated_mutations_until_cancel() {
    protects_mesh_conversion(true, false);
}

#[test]
fn mesh_conversion_pending_review_blocks_authenticated_mutations_until_confirm() {
    protects_mesh_conversion(true, true);
}
