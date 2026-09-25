use super::*;
use crate::{AppCommand, Ray, Vec3};
use egui_kittest::{Harness, kittest::Queryable as _};

#[test]
fn original_v9_nightstand_guarded_physical_repair_has_one_undo_and_verified_geometry() {
    use ketchup_core::assistant_sidecar::{
        AssistantCadEditOperation as Op, AssistantCadParameterValueType, AssistantDowelJointFace,
        AssistantInstancePath, AssistantStandardDowel,
    };
    use ketchup_core::document::FeatureParameterTarget;
    use ketchup_core::joinery::project_dowel_joint_contract;

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../ketchup-application/tests/fixtures/fast_assembly/nightstand_v9_retention.ketchup",
    );
    // Open a private copy: editing the fixture in place leaves recovery
    // sidecars beside it that the next run would silently load.
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("nightstand_v9_retention.ketchup");
    std::fs::copy(&fixture, &source).unwrap();
    let (mut app, mut bridge) = setup();
    if let Some(path) = std::env::var_os("KETCHUP_TEST_EXACT_WORKER") {
        let path = std::path::PathBuf::from(path);
        assert!(
            path.is_file(),
            "selected exact worker does not exist: {path:?}"
        );
        app.exact_worker_path = Some(path);
        app.exact_worker_attempted = true;
    }
    assert!(app.open_document_path(&source));
    // The fixture was saved without grounding. Parts resting on the XY plane
    // (both sides, bottom shelf, drawer front) stand on the floor by default.
    // The drawer box (8-11) rides on runners the fixture does not model, so it
    // is declared carried explicitly. Everything else must be carried by
    // contact or by the verified dowel joints.
    app.document
        .apply_batch(&CommandBatch::new(
            [8, 9, 10, 11]
                .map(|id| CanonicalCommand::SetOccurrenceGrounded {
                    id: OccurrenceId(id),
                    grounded: true,
                })
                .to_vec(),
        ))
        .unwrap();
    let context = egui::Context::default();
    let baseline_deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while app.exact_source.as_ref()
        != Some(&ketchup_application::evaluation::exact_source(
            &app.document.current(),
        ))
    {
        assert!(
            std::time::Instant::now() < baseline_deadline,
            "GUI exact baseline timed out"
        );
        app.refresh_exact_products(&context);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.exact_results.is_empty());
    let before = app.live_bridge_stamp();
    let undo = app.undo_step_count();
    let face = |id, origin, inward, maximum| AssistantDowelJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: id,
            steps: vec![],
        },
        face_origin_local_mm: origin,
        inward_unit_local: inward,
        bounds_min_local_mm: [0.0; 3],
        bounds_max_local_mm: maximum,
    };
    let rear = |origin, inward| face(6, origin, inward, [464.0, 218.0, 8.0]);
    let side = |id| face(id, [0.0; 3], [0.0, 0.0, 1.0], [350.0, 432.0, 18.0]);
    let row =
        |name: &str, first, second, center, direction, spacing| Op::CreatePhysicalDowelJoint {
            joint_id: None,
            name: name.into(),
            first,
            second,
            first_center_local_mm: center,
            row_unit_first_local: direction,
            count: 3,
            spacing_mm: spacing,
            dowel: AssistantStandardDowel::D8x30,
            first_insertion_mm: None,
        };
    let program = AssistantCadEditProgram {
        operations: vec![
            Op::SetFeatureParameter {
                feature_id: 95,
                parameter_path: "bounds.height".into(),
                value_type: AssistantCadParameterValueType::Length,
                value: 218.0,
            },
            Op::MakeOccurrenceUnique { occurrence_id: 2 },
            Op::MakeOccurrenceUnique { occurrence_id: 3 },
            row(
                "Rear to top",
                rear([0.0, 218.0, 0.0], [0.0, -1.0, 0.0]),
                face(1, [0.0; 3], [0.0, 0.0, 1.0], [500.0, 350.0, 18.0]),
                [80.0, 218.0, 4.0],
                [1.0, 0.0, 0.0],
                152.0,
            ),
            row(
                "Rear to left",
                rear([0.0; 3], [1.0, 0.0, 0.0]),
                side(2),
                [0.0, 54.5, 4.0],
                [0.0, 1.0, 0.0],
                54.5,
            ),
            row(
                "Rear to right",
                rear([464.0, 0.0, 0.0], [-1.0, 0.0, 0.0]),
                side(3),
                [464.0, 54.5, 4.0],
                [0.0, 1.0, 0.0],
                54.5,
            ),
        ],
    };
    let proposal = app.derive_assistant_cad_edit_proposal(&program).unwrap();
    let candidate = app.document.preview_verified_proposal(&proposal).unwrap();
    let incremental = ketchup_application::evaluation::plan_incremental_exact_evaluation(
        &app.document.current(),
        &candidate,
        app.exact_source.as_ref(),
    )
    .unwrap();
    assert!(incremental.baseline_reused, "{incremental:?}");
    assert!(
        matches!(
            incremental.selection,
            ketchup_application::evaluation::ExactEvaluationSelection::Scoped(_)
        ),
        "{incremental:?}"
    );
    let report = bridge
        .execute(
            &mut app,
            Request::ApplyAndVerify {
                expected: Some(before.clone()),
                selection: Some(vec![]),
                program,
                validators: mandatory_validators(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: None,
            },
            false,
        )
        .unwrap_or_else(|error| {
            assert_eq!(
                app.live_bridge_stamp(),
                before,
                "failed guarded job mutated the GUI"
            );
            assert_eq!(app.undo_step_count(), undo, "failed guarded job added Undo");
            panic!("v9 repair did not publish within the guarded deadline: {error}");
        });
    assert_eq!(report["published"], true);
    assert_eq!(report["exact"]["complete"], true);
    assert_eq!(report["validation"]["state"], "passed");
    assert_eq!(report["validation"]["complete"], true);
    assert_eq!(app.undo_step_count(), undo + 1);
    let repaired = app.document_snapshot();
    let target = FeatureParameterTarget::new(
        FeatureId(95),
        "bounds.height",
        ketchup_core::document::ParameterValueType::Length,
    )
    .unwrap();
    assert_eq!(repaired.feature_parameter_value(&target), Some(218.0));
    assert_eq!(repaired.dowel_joints().count(), 10);
    for joint in repaired.dowel_joints() {
        let projected = project_dowel_joint_contract(&repaired, joint).unwrap();
        for pair in projected.pairs {
            assert_eq!(pair.first.diameter_mm, 8.0);
            assert_eq!(pair.first.depth_mm, 16.0);
            assert_eq!(pair.second.depth_mm, 16.0);
            assert!(
                pair.physical_probe_coincidence
                    .unwrap()
                    .maximum_endpoint_error_mm
                    < 1e-6
            );
        }
    }
    let empty = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    assert_eq!(empty["selected_context"]["state"], "empty");
    let joint = repaired
        .dowel_joints()
        .find(|joint| {
            joint.first.instance_path.root_occurrence() == OccurrenceId(6)
                && joint.second.instance_path.root_occurrence() == OccurrenceId(1)
        })
        .unwrap();
    let binding = joint.physical_hole_pairs.as_ref().unwrap()[1];
    let path = joint.first.instance_path.clone();
    let definition_id = repaired.resolve_instance_path(&path).unwrap().definition_id;
    let package = app
        .topology_results
        .get_render(&repaired, definition_id)
        .unwrap()
        .clone();
    let hole = &project_dowel_joint_contract(&repaired, joint)
        .unwrap()
        .pairs[1]
        .first;
    let edges = package.edge_evidence();
    let matching_edges = edges
        .iter()
        .filter(|edge| {
            edge.circle_radius_mm
                .is_some_and(|radius| (radius - hole.diameter_mm / 2.0).abs() < 1e-5)
                && edge.axis_origin_mm.is_some_and(|origin| {
                    origin
                        .into_iter()
                        .zip(hole.entry_local_mm)
                        .all(|(a, b)| (a - b).abs() < 1e-5)
                })
        })
        .collect::<Vec<_>>();
    assert!(
        !matching_edges.is_empty(),
        "no circular edge at the selected hole entry"
    );
    let ordinal = matching_edges[0].edge_ordinal;
    assert!(app.select_topological_locator(
        ketchup_interaction::exact_projection::TopologicalPickLocator {
            instance_path: path,
            producer_feature_id: package.producer_feature_id(),
            kind: ketchup_core::topology::TopologicalElementKind::Edge,
            ordinal,
        }
    ));
    let selected = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    assert_eq!(selected["selected_context"]["state"], "dowel_pair");
    assert_eq!(
        selected["selected_context"]["dowel_pair"]["joint_id"],
        joint.id.0
    );
    assert_eq!(selected["selected_context"]["dowel_pair"]["pair_index"], 1);
    assert_eq!(
        selected["selected_context"]["dowel_pair"]["first_pocket_feature_id"],
        binding.first_pocket_feature_id.0
    );
    assert_eq!(
        selected["selected_context"]["dowel_pair"]["second_pocket_feature_id"],
        binding.second_pocket_feature_id.0
    );
    for index in [0, 2] {
        let pair = &project_dowel_joint_contract(&repaired, joint)
            .unwrap()
            .pairs[index];
        let ordinal = edges
            .iter()
            .find(|edge| {
                edge.circle_radius_mm
                    .is_some_and(|radius| (radius - pair.first.diameter_mm / 2.0).abs() < 1e-5)
                    && edge.axis_origin_mm.is_some_and(|origin| {
                        origin
                            .into_iter()
                            .zip(pair.first.entry_local_mm)
                            .all(|(a, b)| (a - b).abs() < 1e-5)
                    })
            })
            .unwrap()
            .edge_ordinal;
        assert!(app.select_topological_locator(
            ketchup_interaction::exact_projection::TopologicalPickLocator {
                instance_path: joint.first.instance_path.clone(),
                producer_feature_id: package.producer_feature_id(),
                kind: ketchup_core::topology::TopologicalElementKind::Edge,
                ordinal,
            }
        ));
        let status = bridge.execute(&mut app, Request::Status {}, false).unwrap();
        assert_eq!(status["selected_context"]["state"], "dowel_pair");
        assert_eq!(
            status["selected_context"]["dowel_pair"]["joint_id"],
            joint.id.0
        );
        assert_eq!(
            status["selected_context"]["dowel_pair"]["pair_index"],
            index
        );
    }
    let second = &project_dowel_joint_contract(&repaired, joint)
        .unwrap()
        .pairs[1]
        .second;
    let second_definition = repaired
        .resolve_instance_path(&second.instance_path)
        .unwrap()
        .definition_id;
    let second_package = app
        .topology_results
        .get_render(&repaired, second_definition)
        .unwrap()
        .clone();
    let second_edge = second_package
        .edge_evidence()
        .iter()
        .find(|edge| {
            edge.circle_radius_mm
                .is_some_and(|radius| (radius - second.diameter_mm / 2.0).abs() < 1e-5)
                && edge.axis_origin_mm.is_some_and(|origin| {
                    origin
                        .into_iter()
                        .zip(second.entry_local_mm)
                        .all(|(a, b)| (a - b).abs() < 1e-5)
                })
        })
        .unwrap();
    assert!(app.select_topological_locator(
        ketchup_interaction::exact_projection::TopologicalPickLocator {
            instance_path: second.instance_path.clone(),
            producer_feature_id: second_package.producer_feature_id(),
            kind: ketchup_core::topology::TopologicalElementKind::Edge,
            ordinal: second_edge.edge_ordinal,
        }
    ));
    let opposite = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    assert_eq!(opposite["selected_context"]["state"], "dowel_pair");
    assert_eq!(
        opposite["selected_context"]["dowel_pair"]["joint_id"],
        joint.id.0
    );
    assert_eq!(opposite["selected_context"]["dowel_pair"]["pair_index"], 1);
    assert_eq!(
        opposite["selected_context"]["dowel_pair"]["selected_side"],
        "second"
    );
    assert!(app.select_topological_locator(
        ketchup_interaction::exact_projection::TopologicalPickLocator {
            instance_path: joint.first.instance_path.clone(),
            producer_feature_id: package.producer_feature_id(),
            kind: ketchup_core::topology::TopologicalElementKind::Edge,
            ordinal,
        }
    ));
    assert!(
        app.select_topological_locator_additive(
            ketchup_interaction::exact_projection::TopologicalPickLocator {
                instance_path: joint.first.instance_path.clone(),
                producer_feature_id: package.producer_feature_id(),
                kind: ketchup_core::topology::TopologicalElementKind::Edge,
                ordinal: edges
                    .iter()
                    .find(|edge| {
                        edge.axis_origin_mm.is_some_and(|origin| {
                            origin
                                .into_iter()
                                .zip(
                                    project_dowel_joint_contract(&repaired, joint)
                                        .unwrap()
                                        .pairs[0]
                                        .first
                                        .entry_local_mm,
                                )
                                .all(|(a, b)| (a - b).abs() < 1e-5)
                        }) && edge.circle_radius_mm == Some(4.0)
                    })
                    .unwrap()
                    .edge_ordinal,
            },
            true
        )
    );
    let multiple = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    assert_eq!(
        multiple["selected_context"]["state"],
        "multiple_topological_elements"
    );
    assert!(multiple["selected_context"]["dowel_pair"].is_null());
    let other = edges
        .iter()
        .find(|edge| edge.circle_radius_mm.is_none() && edge.edge_ordinal != ordinal)
        .unwrap();
    assert!(app.select_topological_locator(
        ketchup_interaction::exact_projection::TopologicalPickLocator {
            instance_path: joint.first.instance_path.clone(),
            producer_feature_id: package.producer_feature_id(),
            kind: ketchup_core::topology::TopologicalElementKind::Edge,
            ordinal: other.edge_ordinal,
        }
    ));
    let non_hole = bridge.execute(&mut app, Request::Status {}, false).unwrap();
    assert_eq!(non_hole["selected_context"]["state"], "topological_element");
    assert!(non_hole["selected_context"]["dowel_pair"].is_null());
    let after = app.live_bridge_stamp();
    assert!(app.undo());
    assert_eq!(
        app.live_bridge_stamp().canonical_digest,
        before.canonical_digest
    );
    assert!(app.redo());
    assert_eq!(
        app.live_bridge_stamp().canonical_digest,
        after.canonical_digest
    );
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.step();
    harness.state_mut().dispatch_command(AppCommand::ViewBack);
    harness.state_mut().dispatch_command(AppCommand::ZoomFit);
    harness.step();
    let center = hole.shared_center_world_mm;
    let pointer = harness
        .state()
        .viewport_position(Vec3::new(center[0], center[1], center[2]))
        .unwrap();
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(pointer));
    harness.step();
    assert_eq!(
        harness.state().hover_snap.as_ref().map(|snap| snap.kind),
        Some(ketchup_interaction::SnapKind::Center),
        "pointer={pointer:?} rect={:?} hovered={:?} snap={:?}",
        harness.state().viewport_rect(),
        harness.state().hovered,
        harness.state().hover_snap
    );
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: pointer,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let clicked = bridge
        .execute(harness.state_mut(), Request::Status {}, false)
        .unwrap();
    assert_eq!(clicked["selected_context"]["state"], "dowel_pair");
    assert_eq!(
        clicked["selected_context"]["dowel_pair"]["joint_id"],
        joint.id.0
    );
    assert_eq!(clicked["selected_context"]["dowel_pair"]["pair_index"], 1);
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: pointer,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let away = pointer + egui::Vec2::new(250.0, 180.0);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(away));
    harness.step();
    let moved = bridge
        .execute(harness.state_mut(), Request::Status {}, false)
        .unwrap();
    assert_eq!(moved["selected_context"]["dowel_pair"]["pair_index"], 1);
    let highlight = harness.state().selected_topological_edge_paths();
    assert!(
        !highlight.is_empty(),
        "selected hole mouth must stay highlighted"
    );
    let hole_center = Vec3::new(center[0], center[1], center[2]);
    let radius = hole.diameter_mm / 2.0;
    let ring_points: Vec<_> = highlight.iter().flatten().collect();
    assert!(ring_points.len() >= 8);
    for point in &ring_points {
        let distance = (**point - hole_center).length();
        assert!(
            (distance - radius).abs() <= 0.25,
            "highlight point {point:?} is {distance} mm from the hole centre"
        );
    }
    let span = |axis: fn(&Vec3) -> f64| {
        let values = ring_points.iter().map(|point| axis(point));
        values.clone().fold(f64::NEG_INFINITY, f64::max) - values.fold(f64::INFINITY, f64::min)
    };
    assert!(
        [span(|p| p.x), span(|p| p.y), span(|p| p.z)]
            .iter()
            .filter(|extent| **extent >= hole.diameter_mm - 0.5)
            .count()
            == 2,
        "the highlight must cover the whole hole mouth, not one arc"
    );
}

#[test]
fn verified_geometry_is_render_ready_in_same_gui_across_history_and_preserves_view_layers() {
    let (mut app, mut bridge) = setup();
    #[cfg(feature = "private-oauth")]
    {
        assert_eq!(app.assistant_provider.protocol_name(), "codex-oauth");
        assert_eq!(app.assistant_model, "gpt-5.6-sol");
    }
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: TagId(1),
                name: "Furniture".into(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: TagId(2),
                name: "Control probes".into(),
                visible: false,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: Some(TagId(1)),
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "Hidden control instance".into(),
                transform: Transform::from_translation(500.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: Some(TagId(2)),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(2),
                grounded: true,
            },
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    app.selection.select_occurrence(OccurrenceId(1), false);
    app.yaw = -0.42;
    app.pitch = -0.61;
    app.zoom = 3.1;
    app.pan = egui::vec2(12.0, -18.0);
    app.camera_target_z = 15.0;
    app.grid_axes_visible = false;
    app.profiles_visible = false;
    app.dimensions_visible = false;
    app.tags_visible = true;
    let camera = app.camera_view_state();
    let camera_target = app.camera_target();
    let selection = (
        app.selection.occurrences.clone(),
        app.selection.primary.clone(),
    );
    let before = app.live_bridge_stamp();
    let undo_steps = app.undo_step_count();
    let before_exact = exact_fingerprints(&app);
    let baseline_boxes = app.active_boxes(); // Warm the viewport cache before publication.
    assert_eq!(baseline_boxes.len(), 1);
    assert_eq!(baseline_boxes[0].size_mm.z, 20.0);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_max_steps(64)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();

    let report = bridge
        .execute(
            harness.state_mut(),
            Request::ApplyAndVerify {
                expected: Some(before.clone()),
                selection: Some(vec![1]),
                program: worker_required_program(),
                validators: mandatory_validators(),
                timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
                strict: false,
                save: None,
            },
            false,
        )
        .unwrap();
    let published = harness.state().live_bridge_stamp();
    let published_exact = exact_fingerprints(harness.state());
    assert_ne!(before_exact, published_exact);
    assert_eq!(report["before"], serde_json::to_value(&before).unwrap());
    assert_eq!(report["after"], serde_json::to_value(&published).unwrap());
    assert_eq!(report["exact"]["complete"], true);
    assert_eq!(report["exact"]["topology_complete"], true);
    assert_eq!(report["validation"]["complete"], true);
    assert_eq!(harness.state().undo_step_count(), undo_steps + 1);

    for (command, height, expected_stamp, expected_exact) in [
        (None, 45.0, &published, &published_exact),
        (Some(AppCommand::Undo), 20.0, &before, &before_exact),
        (Some(AppCommand::Redo), 45.0, &published, &published_exact),
    ] {
        if let Some(command) = command {
            harness.state_mut().dispatch_command(command);
        }
        harness.run();
        let app = harness.state();
        let stamp = app.live_bridge_stamp();
        assert_eq!(stamp.document_id, before.document_id);
        assert_eq!(stamp.revision, expected_stamp.revision);
        assert_eq!(stamp.canonical_digest, expected_stamp.canonical_digest);
        if command.is_some() {
            assert!(stamp.mutation_epoch > published.mutation_epoch);
            assert_ne!(
                report["after"],
                serde_json::to_value(&stamp).unwrap(),
                "the original receipt must not masquerade as current after history changes"
            );
        }
        assert_eq!(app.camera_view_state(), camera);
        assert_eq!(app.camera_target(), camera_target);
        assert_eq!(
            (&app.selection.occurrences, &app.selection.primary),
            (&selection.0, &selection.1)
        );
        assert!(!app.dimensions_visible);
        assert!(app.tags_visible);
        let snapshot = app.document.current();
        assert_eq!(snapshot.tag(TagId(1)).unwrap().name(), "Furniture");
        assert!(snapshot.tag(TagId(1)).unwrap().visible());
        assert_eq!(snapshot.tag(TagId(2)).unwrap().name(), "Control probes");
        assert!(!snapshot.tag(TagId(2)).unwrap().visible());
        assert_eq!(
            snapshot.occurrence(OccurrenceId(1)).unwrap().tag(),
            Some(TagId(1))
        );
        assert_eq!(
            snapshot.occurrence(OccurrenceId(2)).unwrap().tag(),
            Some(TagId(2))
        );
        assert!(app.exact_results.is_bound_to(&snapshot));
        assert!(app.topology_results.is_bound_to(&snapshot));
        let fingerprints = exact_fingerprints(app);
        assert!(expected_exact.iter().all(|key| fingerprints.contains(key)));
        let boxes = app.active_boxes();
        assert_eq!(
            boxes.len(),
            1,
            "hidden control layer must stay out of the viewport"
        );
        assert_eq!(boxes[0].size_mm, Vec3::new(100.0, 60.0, height));
        let ray = Ray::new(Vec3::new(50.0, 30.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
        let hit = app
            .exact_projection(&snapshot)
            .exact_surface_pick(ray)
            .expect("published/history geometry must be exact-pickable, not only a proxy box");
        assert!((hit.position_mm.z - height).abs() < 1e-7);
        let status = format!(
            "{}  \u{b7}  {}",
            app.catalog.format(
                "status-selected",
                &BTreeMap::from([("count", "1".to_owned())])
            ),
            app.digest
        );
        assert!(
            harness.query_by_label(&status).is_some(),
            "same GUI must show its current status"
        );
    }
    assert_eq!(harness.state().undo_step_count(), undo_steps + 1);
    assert_eq!(harness.state().redo_step_count(), 0);
}
