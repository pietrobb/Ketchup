//! Real TCP requests processed by the existing offscreen GUI shell, never OS input.
mod harness;
use harness::Shell;
use ketchup_app::{AppCommand, live_bridge::*};
use ketchup_application::model_query::{EntityKind, PageRequest};
use ketchup_core::{
    assistant_sidecar::*,
    document::{CommandBatch, DocumentStore},
};
use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::mpsc,
    time::{Duration, Instant},
};

struct Client {
    stream: TcpStream,
    token: String,
    sequence: u64,
}
impl Client {
    fn connect(shell: &mut Shell) -> Self {
        assert!(shell.app().live_bridge_credentials().is_none());
        let context = eframe::egui::Context::default();
        let address = shell.app_mut().enable_live_bridge(&context).unwrap();
        assert!(address.ip().is_loopback());
        let credentials = shell.app().live_bridge_credentials().unwrap();
        assert_eq!(credentials.token.len(), 64);
        Self {
            stream: TcpStream::connect(address).unwrap(),
            token: credentials.token,
            sequence: 0,
        }
    }
    fn call(&mut self, shell: &mut Shell, request: Request) -> Response {
        self.sequence += 1;
        let bytes = serde_json::to_vec(&Envelope {
            version: 1,
            id: self.sequence,
            token: self.token.clone(),
            request,
        })
        .unwrap();
        assert!(bytes.len() <= MAX_FRAME_BYTES);
        self.stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .unwrap();
        self.stream.write_all(&bytes).unwrap();
        let mut reader = self.stream.try_clone().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let mut header = [0; 4];
            reader.read_exact(&mut header).unwrap();
            let length = u32::from_be_bytes(header) as usize;
            assert!(length <= MAX_FRAME_BYTES);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            tx.send(serde_json::from_slice::<Response>(&body).unwrap())
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let response = loop {
            if let Ok(response) = rx.try_recv() {
                break response;
            }
            assert!(Instant::now() < deadline, "bridge response deadline");
            shell.step();
            std::thread::sleep(Duration::from_millis(5));
        };
        handle.join().unwrap();
        assert_eq!(response.id, self.sequence);
        response
    }
}
fn program() -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreatePart {
            name: "S4 live cylinder".into(),
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
fn propose(client: &mut Client, shell: &mut Shell) -> (Stamp, u64) {
    let expected = shell.app().live_bridge_stamp();
    let response = client.call(
        shell,
        Request::Propose {
            expected: expected.clone(),
            selection: vec![],
            program: program(),
        },
    );
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(
        expected,
        shell.app().live_bridge_stamp(),
        "propose mutated canonical store"
    );
    let id = response.result.unwrap()["proposal_id"].as_u64().unwrap();
    (expected, id)
}
fn commit(client: &mut Client, shell: &mut Shell) -> Response {
    let (expected, proposal_id) = propose(client, shell);
    let response = client.call(
        shell,
        Request::Commit {
            expected,
            proposal_id,
        },
    );
    assert!(response.ok, "{:?}", response.error);
    response
}

#[test]
fn same_gui_store_observational_reads_verified_once_and_gui_history() {
    let mut shell = Shell::new();
    let mut client = Client::connect(&mut shell);
    let before = shell.app().live_bridge_stamp();
    let count = shell.app().document_snapshot().occurrences().count();
    let summary = client.call(&mut shell, Request::Summary {});
    assert!(summary.ok);
    assert_eq!(summary.stamp.as_ref(), Some(&before));
    assert!(serde_json::to_vec(&summary).unwrap().len() < 8192);
    let (expected, proposal_id) = propose(&mut client, &mut shell);
    let request = Request::Commit {
        expected,
        proposal_id,
    };
    let response = client.call(&mut shell, request.clone());
    assert!(response.ok, "{:?}", response.error);
    let after = shell.app().live_bridge_stamp();
    assert_eq!(
        shell.app().document_snapshot().occurrences().count(),
        count + 1
    );
    assert!(after.mutation_epoch > before.mutation_epoch);
    let steps = shell.app().undo_step_count();
    let replay = client.call(&mut shell, request.clone());
    assert_eq!(response.result, replay.result);
    assert_eq!(shell.app().undo_step_count(), steps);
    assert_eq!(shell.app().live_bridge_stamp(), after);
    // Actual GUI command dispatch through AccessKit, not a separate session.
    shell.click_command(AppCommand::Undo);
    assert_eq!(shell.app().document_snapshot().occurrences().count(), count);
    shell.click_command(AppCommand::Redo);
    assert_eq!(
        shell.app().document_snapshot().occurrences().count(),
        count + 1
    );
    let expected = shell.app().live_bridge_stamp();
    let undo = client.call(&mut shell, Request::Undo { expected });
    assert!(undo.ok, "{:?}", undo.error);
    let expected = shell.app().live_bridge_stamp();
    assert!(client.call(&mut shell, Request::Redo { expected }).ok);
    let stamp = shell.app().live_bridge_stamp();
    assert!(client.call(&mut shell, request).ok);
    assert_eq!(shell.app().live_bridge_stamp(), stamp);
    let occurrence_id = shell
        .app()
        .document_snapshot()
        .occurrences()
        .next()
        .unwrap()
        .id()
        .0;
    assert!(
        client
            .call(
                &mut shell,
                Request::Detail {
                    expected: stamp.clone(),
                    kind: EntityKind::Occurrences,
                    entity_id: occurrence_id
                }
            )
            .ok
    );
    assert!(
        client
            .call(
                &mut shell,
                Request::View {
                    expected: stamp.clone(),
                    view: View::Top
                }
            )
            .ok
    );
    assert_eq!(shell.app().live_bridge_stamp(), stamp);
    let image = client.call(
        &mut shell,
        Request::Image {
            expected: stamp,
            capture_mode: CaptureMode::Offscreen,
        },
    );
    assert!(
        matches!(
            image.error.as_deref(),
            Some("image_timeout" | "stale_image")
        ),
        "{:?}",
        image.error
    );
}

#[test]
fn human_history_aba_and_selection_refuse_stale_proposals_and_cursors() {
    let mut shell = Shell::new();
    let mut client = Client::connect(&mut shell);
    commit(&mut client, &mut shell);
    commit(&mut client, &mut shell);
    let before = shell.app().live_bridge_stamp();
    let query = PageRequest {
        kind: EntityKind::Occurrences,
        limit: 1,
        search: String::new(),
        definition_id: None,
        tag_id: None,
        classification_dimension_id: None,
        classification_category_id: None,
        world_bounds_mm: None,
        cursor: None,
    };
    let page = client.call(
        &mut shell,
        Request::Query {
            expected: before.clone(),
            query: query.clone(),
        },
    );
    assert!(page.ok);
    let cursor = page.result.unwrap()["next_cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let workset = client.call(
        &mut shell,
        Request::WorksetCreate {
            expected: before.clone(),
            query: PageRequest {
                cursor: None,
                ..query.clone()
            },
        },
    );
    assert!(workset.ok, "{:?}", workset.error);
    let workset_handle = workset.result.unwrap()["workset_handle"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        client
            .call(
                &mut shell,
                Request::WorksetStatus {
                    expected: before.clone(),
                    handle: workset_handle.clone(),
                },
            )
            .ok
    );
    let (expected, proposal_id) = propose(&mut client, &mut shell);
    shell.click_command(AppCommand::Undo);
    let stale = client.call(
        &mut shell,
        Request::Commit {
            expected: expected.clone(),
            proposal_id,
        },
    );
    assert_eq!(stale.error.as_deref(), Some("stale_document"));
    shell.click_command(AppCommand::Redo);
    let after = shell.app().live_bridge_stamp();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.canonical_digest, before.canonical_digest);
    assert!(after.mutation_epoch > before.mutation_epoch);
    let stale_workset = client.call(
        &mut shell,
        Request::WorksetStatus {
            expected: after.clone(),
            handle: workset_handle,
        },
    );
    assert_eq!(stale_workset.error.as_deref(), Some("stale_workset"));
    let stale = client.call(
        &mut shell,
        Request::Commit {
            expected,
            proposal_id,
        },
    );
    assert_eq!(stale.error.as_deref(), Some("stale_document"));
    let stale_cursor = client.call(
        &mut shell,
        Request::Query {
            expected: after.clone(),
            query: PageRequest {
                cursor: Some(cursor),
                ..query
            },
        },
    );
    assert_eq!(stale_cursor.error.as_deref(), Some("stale_cursor"));
    let id = shell
        .app()
        .document_snapshot()
        .occurrences()
        .next()
        .unwrap()
        .id()
        .0;
    let mismatch = client.call(
        &mut shell,
        Request::Propose {
            expected: after.clone(),
            selection: vec![id],
            program: program(),
        },
    );
    assert_eq!(mismatch.error.as_deref(), Some("selection_changed"));
    let (expected, proposal_id) = propose(&mut client, &mut shell);
    assert!(
        client
            .call(
                &mut shell,
                Request::Selection {
                    expected: after.clone(),
                    occurrence_ids: vec![id]
                }
            )
            .ok
    );
    assert_eq!(shell.app().live_bridge_stamp(), after);
    let stale = client.call(
        &mut shell,
        Request::Commit {
            expected,
            proposal_id,
        },
    );
    assert_eq!(stale.error.as_deref(), Some("proposal_not_found"));
}

#[test]
fn auth_bounds_disconnect_and_default_disabled() {
    let mut shell = Shell::new();
    let before = shell.app().live_bridge_stamp();
    let mut client = Client::connect(&mut shell);
    let credentials = shell.app().live_bridge_credentials().unwrap();
    client.token = "0".repeat(64);
    assert_eq!(
        client
            .call(&mut shell, Request::Summary {})
            .error
            .as_deref(),
        Some("unauthorized")
    );
    drop(client);
    let mut client = Client {
        stream: TcpStream::connect(credentials.address).unwrap(),
        token: credentials.token,
        sequence: 0,
    };
    let status = client.call(&mut shell, Request::Status {});
    assert!(status.ok);
    let invalid = client.call(
        &mut shell,
        Request::Query {
            expected: before.clone(),
            query: PageRequest {
                kind: EntityKind::Occurrences,
                limit: 101,
                search: String::new(),
                definition_id: None,
                tag_id: None,
                classification_dimension_id: None,
                classification_category_id: None,
                world_bounds_mm: None,
                cursor: None,
            },
        },
    );
    assert_eq!(invalid.error.as_deref(), Some("invalid_params"));
    let (expected, proposal_id) = propose(&mut client, &mut shell);
    assert!(client.call(&mut shell, Request::Disconnect {}).ok);
    let mut byte = [0];
    assert_eq!(client.stream.read(&mut byte).unwrap(), 0);
    client.stream = TcpStream::connect(credentials.address).unwrap();
    let replay = client.call(
        &mut shell,
        Request::Commit {
            expected,
            proposal_id,
        },
    );
    assert_eq!(replay.error.as_deref(), Some("proposal_not_found"));
    assert_eq!(shell.app().live_bridge_stamp(), before);
    assert!(client.call(&mut shell, Request::Disconnect {}).ok);
    let mut oversized = TcpStream::connect(credentials.address).unwrap();
    oversized
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    oversized
        .write_all(&((MAX_FRAME_BYTES + 1) as u32).to_be_bytes())
        .unwrap();
    assert!(matches!(oversized.read(&mut byte), Ok(0) | Err(_)));
    shell.app_mut().disable_live_bridge();
    assert!(shell.app().live_bridge_credentials().is_none());
    assert_eq!(shell.app().live_bridge_stamp(), before);
    assert!(serde_json::from_str::<Request>(r#"{"method":"shutdown"}"#).is_err());
    assert!(
        serde_json::from_str::<Request>(r#"{"method":"summary","script":"anything"}"#).is_err()
    );
}

#[test]
fn central_epoch_is_not_revision_state_and_failed_operations_are_observational() {
    let mut store = DocumentStore::new();
    let initial = store.mutation_epoch();
    assert!(store.undo().is_none());
    assert!(store.redo().is_none());
    assert_eq!(store.mutation_epoch(), initial);
    let batch = CommandBatch::new(vec![
        ketchup_core::document::CanonicalCommand::CreateDefinition {
            id: ketchup_core::document::DefinitionId(1),
            name: "epoch".into(),
        },
    ]);
    let proposal = store.prepare_proposal(batch.clone()).unwrap();
    assert_eq!(store.mutation_epoch(), initial);
    store.commit_verified_proposal(&proposal).unwrap();
    let committed = store.mutation_epoch();
    assert!(committed > initial);
    assert!(store.apply_batch(&batch).is_err());
    assert_eq!(store.mutation_epoch(), committed);
    let digest = store.current().canonical_digest();
    let revision = store.current().revision_id();
    store.undo().unwrap();
    let undone = store.mutation_epoch();
    store.redo().unwrap();
    assert!(undone > committed);
    assert!(store.mutation_epoch() > undone);
    assert_eq!(store.current().canonical_digest(), digest);
    assert_eq!(store.current().revision_id(), revision);
    assert!(DocumentStore::new().mutation_epoch() > store.mutation_epoch());
}
