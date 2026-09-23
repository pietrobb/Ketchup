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

    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../ketchup-application/tests/fixtures/fast_assembly/nightstand_v9_retention.ketchup",
    );
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
    // The fixture was saved without grounding; gravity support needs to know
    // which parts stand on the floor (both sides, bottom shelf, drawer front).
    // The drawer box (8-11) rides on runners the fixture does not model, so it
    // is declared carried as well. Everything else must be carried by contact
    // or by the verified dowel joints.
    app.document
        .apply_batch(&CommandBatch::new(
            [2, 3, 4, 7, 8, 9, 10, 11]
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
