use super::*;
use ketchup_core::{
    document::NodeId,
    document::Transform,
    document::{
        CanonicalCommand, CommandBatch, DefinitionId, FeatureId, GroupId, InstancePath, TagId,
    },
};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    time::{Duration, Instant},
};

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
fn setup() -> (KetchupApp, LiveBridge) {
    let mut app = KetchupApp::new();
    app.selection.clear();
    let bridge = transport::start(egui::Context::default()).unwrap();
    (app, bridge)
}
fn proposal(app: &mut KetchupApp, bridge: &mut LiveBridge) -> Request {
    let expected = app.live_bridge_stamp();
    let selection = LiveBridge::selection(app).unwrap();
    let result = bridge
        .execute(
            app,
            Request::Propose {
                expected: expected.clone(),
                selection,
                program: program(),
            },
            false,
        )
        .unwrap();
    Request::Commit {
        expected,
        proposal_id: result["proposal_id"].as_u64().unwrap(),
    }
}
fn protected_requests(stamp: &Stamp, commit: &Request) -> Vec<Request> {
    vec![
        commit.clone(),
        Request::Undo {
            expected: stamp.clone(),
        },
        Request::Redo {
            expected: stamp.clone(),
        },
        Request::Selection {
            expected: stamp.clone(),
            occurrence_ids: vec![],
        },
        Request::View {
            expected: stamp.clone(),
            view: View::Top,
        },
        Request::Propose {
            expected: stamp.clone(),
            selection: vec![1],
            program: program(),
        },
    ]
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
                app.move_anchor = Some(crate::MoveDrag {
                    source_document_id: app.document.current().document_id(),
                    source_revision: stamp.revision,
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
                });
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
                app.move_anchor.as_ref().unwrap().delta_mm,
                crate::Vec3::new(1.0, 2.0, 3.0)
            ),
            6 => assert_eq!(app.preview_definition_id, Some(DefinitionId(999))),
            7 => assert_eq!(app.zoom_window_start, Some(egui::pos2(12.0, 34.0))),
            _ => unreachable!(),
        }
    }
}

#[test]
fn review_only_history_and_focused_editor_reject_but_receipt_replay_is_observational() {
    let (mut app, mut bridge) = setup();
    let commit = proposal(&mut app, &mut bridge);
    let receipt = bridge.execute(&mut app, commit.clone(), false).unwrap();
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
            expected: stamp.clone(),
        },
        Request::Redo {
            expected: stamp.clone(),
        },
    ] {
        assert_eq!(
            bridge.execute(&mut app, request, false),
            Err("read_only_document")
        );
    }
    app.preview = Some(CommandBatch::new(vec![]));
    assert_eq!(bridge.execute(&mut app, commit, true).unwrap(), receipt);
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
                    expected: stamp.clone(),
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
                        expected: stamp.clone(),
                        selection: vec![1],
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
            expected: expected.clone(),
            selection: vec![],
            program: program(),
        });
        Request::Commit {
            expected,
            proposal_id: response.result.unwrap()["proposal_id"].as_u64().unwrap(),
        }
    }
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
