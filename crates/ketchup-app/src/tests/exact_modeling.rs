//! Exact solid tools, finishes and their tamper/stale-safe plans.

use super::*;

#[test]
fn exact_history_releases_evicted_and_abandoned_packages_but_keeps_redo() {
    let mut app = KetchupApp::new();
    let mut packages = Vec::new();
    for step in 0..(ketchup_model::tolerance::limits::UNDO_REVISIONS + 3) {
        let snapshot = app.document.current();
        let package = Arc::new(current_box_package(&app));
        packages.push(Arc::downgrade(&package));
        app.exact.results.clear();
        app.exact.topology_results.clear();
        app.exact
            .results
            .insert_current(&snapshot, Arc::clone(&package))
            .unwrap();
        app.exact
            .topology_results
            .insert_current(&snapshot, package)
            .unwrap();
        app.rebind_exact_results(&snapshot);
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: FeatureId(2),
                    dimension: Dimension::from_decimal((40 + step).to_string()).unwrap(),
                },
            ]))
            .unwrap();
        app.rebind_exact_results(&app.document.current());
    }
    assert!(
        packages[0].upgrade().is_none(),
        "evicted geometry must be freed, not just hidden"
    );
    let retained = packages.last().unwrap();
    assert!(retained.upgrade().is_some());
    app.document.undo().unwrap();
    app.rebind_exact_results(&app.document.current());
    assert!(
        !app.exact_render_bounds().is_empty(),
        "Undo must recover exact geometry"
    );
    let undo_bounds = app.exact_render_bounds();
    app.document.undo().unwrap();
    app.rebind_exact_results(&app.document.current());
    assert!(
        retained.upgrade().is_some(),
        "redo geometry must stay alive"
    );
    app.document.redo().unwrap();
    app.rebind_exact_results(&app.document.current());
    assert_eq!(app.exact_render_bounds(), undo_bounds);
    app.document.undo().unwrap();
    app.rebind_exact_results(&app.document.current());
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(2),
                dimension: Dimension::from_decimal("999").unwrap(),
            },
        ]))
        .unwrap();
    app.rebind_exact_results(&app.document.current());
    assert!(
        retained.upgrade().is_none(),
        "abandoned redo geometry must be released"
    );
    app.document.discard_history_before_current();
    app.rebind_exact_results(&app.document.current());
    assert!(packages.iter().all(|package| package.upgrade().is_none()));
}

#[test]
fn active_boxes_cache_invalidates_on_transform_visibility_and_undo_redo() {
    let mut app = KetchupApp::new();
    let initial = app.active_boxes(); // Warm the render-box cache before canonical mutation.
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].origin_mm, Vec3::ZERO);
    assert_eq!(initial[0].size_mm, Vec3::new(100.0, 60.0, 20.0));
    let mut moved = initial.clone();
    moved[0].origin_mm = Vec3::new(10.0, -5.0, 3.0);

    // Go through the canonical store, without presentation helpers clearing caches.
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: Transform::from_translation(10.0, -5.0, 3.0).unwrap(),
            },
        ]))
        .unwrap();
    assert_eq!(app.active_boxes(), moved);
    app.document.undo().unwrap();
    assert_eq!(app.active_boxes(), initial);
    app.document.redo().unwrap();
    assert_eq!(app.active_boxes(), moved);

    for visible in [false, true] {
        let before = app.active_boxes();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceVisibility {
                    id: OccurrenceId(1),
                    visible,
                },
            ]))
            .unwrap();
        let expected = if visible { moved.clone() } else { Vec::new() };
        assert_eq!(app.active_boxes(), expected);
        app.document.undo().unwrap();
        assert_eq!(app.active_boxes(), before);
        app.document.redo().unwrap();
        assert_eq!(app.active_boxes(), expected);
    }
}

#[test]
fn active_boxes_cache_invalidates_on_same_revision_exact_results_and_registry_replacement() {
    let mut app = KetchupApp::new();
    let snapshot = app.document.current();
    let canonical = app.active_boxes();
    assert_eq!(canonical.len(), 1);
    assert_eq!(canonical[0].origin_mm, Vec3::ZERO);
    assert_eq!(canonical[0].size_mm, Vec3::new(100.0, 60.0, 20.0));

    for (replace_registry, minimum, maximum) in [
        (false, [-2.0, -3.0, -4.0], [110.0, 70.0, 30.0]),
        (true, [-5.0, -6.0, -7.0], [120.0, 80.0, 40.0]),
    ] {
        // Synthetic exact bounds distinguish evaluated results from the canonical proxy.
        let mut package = current_box_package(&app);
        let ExactBodyPackage::Graph(graph_package) = &mut package else {
            unreachable!("the test double returns a graph package");
        };
        graph_package.bounds_mm = [minimum, maximum];
        let stamp = app.exact.results.contents_stamp();
        if replace_registry {
            let mut replacement = ExactResultRegistry::default();
            replacement
                .insert_current(&snapshot, Arc::new(package))
                .unwrap();
            assert_eq!(replacement.len(), app.exact.results.len());
            app.exact.results = replacement;
        } else {
            app.exact
                .results
                .insert_current(&snapshot, Arc::new(package))
                .unwrap();
        }
        assert_ne!(app.exact.results.contents_stamp(), stamp);
        assert_eq!(app.document.current().revision_id(), snapshot.revision_id());
        let mut expected = canonical.clone();
        expected[0].origin_mm = Vec3::new(minimum[0], minimum[1], minimum[2]);
        expected[0].size_mm = Vec3::new(maximum[0], maximum[1], maximum[2]) - expected[0].origin_mm;
        assert_eq!(app.active_boxes(), expected); // Also warm before the next replacement.
        assert_eq!(
            app.render_boxes_from_projection(
                &snapshot,
                &CanonicalInteractionProjection::from_snapshot(&snapshot),
                false,
            ),
            canonical,
            "disabled exact bounds must preserve the canonical box proxy"
        );
    }

    app.exact.results = ExactResultRegistry::default();
    assert_eq!(app.document.current().revision_id(), snapshot.revision_id());
    assert_eq!(app.active_boxes(), canonical);
}

#[test]
fn historical_exact_geometry_is_bound_to_its_own_snapshot() {
    let mut app = KetchupApp::new();
    assert!(app.headless_install_exact_package(current_box_package(&app)));
    let parent = app.document.current();

    assert!(
        app.apply_batch_with_work_recovery(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(2),
                dimension: Dimension::from_decimal("40").unwrap(),
            },
        ]))
        .is_ok()
    );
    let tip = app.document.current();
    assert!(app.headless_install_exact_package(current_box_package(&app)));

    assert_eq!(
        app.active_boxes_for_snapshot(&parent)[0].size_mm,
        Vec3::new(100.0, 60.0, 20.0)
    );
    assert_eq!(
        app.active_boxes_for_snapshot(&tip)[0].size_mm,
        Vec3::new(100.0, 60.0, 40.0)
    );
    let ray = Ray::new(Vec3::new(50.0, 30.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    let parent_hit = app
        .exact_projection(&parent)
        .exact_surface_pick(ray)
        .expect("the parent exact body remains pickable");
    let tip_hit = app
        .exact_projection(&tip)
        .exact_surface_pick(ray)
        .expect("the tip exact body remains pickable");
    assert_eq!(parent_hit.position_mm.z, 20.0);
    assert_eq!(tip_hit.position_mm.z, 40.0);

    for (snapshot, expected_height) in [(&parent, 20.0), (&tip, 40.0)] {
        app.refresh_interaction_projection_cache(snapshot);
        let cache = app.hover.projection_cache.borrow();
        let cache = cache.as_ref().expect("the requested snapshot is cached");
        assert_eq!(cache.document_id, snapshot.document_id());
        assert_eq!(cache.revision_id, snapshot.revision_id());
        assert_eq!(cache.canonical_digest, snapshot.canonical_digest());
        assert_eq!(
            cache
                .exact
                .exact_surface_pick(ray)
                .expect("the cached exact body remains pickable")
                .position_mm
                .z,
            expected_height
        );
        assert_eq!(
            app.render_boxes_from_projection(snapshot, &cache.canonical, true)[0]
                .size_mm
                .z,
            expected_height
        );

        app.selection.select_exact(
            SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        assert_eq!(
            app.selected_definition_id_for_snapshot(snapshot),
            Some(INITIAL_BOX_DEFINITION)
        );
        assert_eq!(
            app.box_height_mm_for_snapshot(snapshot, INITIAL_BOX_DEFINITION),
            Some(expected_height)
        );
    }

    assert!(app.undo());
    assert_eq!(app.active_boxes()[0].size_mm.z, 20.0);
    assert!(app.redo());
    assert_eq!(app.active_boxes()[0].size_mm.z, 40.0);

    assert!(
        app.render_boxes_from_projection(
            &tip,
            &CanonicalInteractionProjection::from_snapshot(&parent),
            true,
        )
        .is_empty(),
        "a projection and snapshot from different revisions must fail closed"
    );
}

#[test]
fn interaction_projection_refresh_defers_while_the_current_frame_reads_the_cache() {
    let app = KetchupApp::new();
    let snapshot = app.document.current();
    app.refresh_interaction_projection_cache(&snapshot);
    let expected_revision = snapshot.revision_id();
    let expected_exact_stamp = (app.exact.results.contents_stamp(), 0);
    {
        let mut cache = app.hover.projection_cache.borrow_mut();
        let cache = cache.as_mut().expect("the initial projection is cached");
        cache.revision_id = expected_revision.wrapping_add(1);
        cache.exact_results_stamp = (expected_exact_stamp.0.wrapping_add(1), 0);
    }

    let current_frame = app.hover.projection_cache.borrow();
    app.refresh_interaction_projection_cache(&snapshot);
    let deferred = current_frame
        .as_ref()
        .expect("an active reader keeps the last valid projection");
    assert_ne!(deferred.revision_id, expected_revision);
    assert_ne!(deferred.exact_results_stamp, expected_exact_stamp);
    drop(current_frame);

    app.refresh_interaction_projection_cache(&snapshot);
    let refreshed = app.hover.projection_cache.borrow();
    let refreshed = refreshed
        .as_ref()
        .expect("the deferred projection rebuild completes next frame");
    assert_eq!(refreshed.revision_id, expected_revision);
    assert_eq!(refreshed.exact_results_stamp, expected_exact_stamp);
}

#[test]
fn current_exact_occurrence_suppresses_only_the_non_preview_proxy() {
    let mut app = KetchupApp::new();
    let package = current_box_package(&app);
    let snapshot = app.document.current();
    app.exact
        .results
        .insert_current(&snapshot, Arc::new(package))
        .unwrap();
    let exact_projection = app.exact_projection(&snapshot);

    assert!(exact_projection.contains_occurrence(&InstancePath::root(OccurrenceId(1))));
    assert!(app.viewport_boxes(&snapshot, &exact_projection).is_empty());

    app.selection.primary = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    app.set_push_pull_distance_input("5");
    assert!(app.start_preview());
    assert_eq!(app.viewport_boxes(&snapshot, &exact_projection).len(), 1);
}

#[test]
fn production_exact_refresh_uses_graph_for_a_general_boolean_chain() {
    let executable = exact_worker_executable();
    assert!(
        executable.is_file(),
        "build workspace all-targets so the exact worker exists at {}",
        executable.display()
    );
    let definition_id = DefinitionId(77);
    let producer_feature_id = FeatureId(30);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: "General boolean".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(10),
                definition_id,
                name: "Base pentagon".into(),
                kind: FeatureKind::polygon(&[
                    [-12.0, -8.0],
                    [18.0, -6.0],
                    [24.0, 9.0],
                    [3.0, 20.0],
                    [-17.0, 7.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id,
                name: "Unequal base".into(),
                kind: FeatureKind::extrusion(FeatureId(10), Dimension::from_decimal("13").unwrap()),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(20),
                definition_id,
                name: "Slanted tool".into(),
                kind: FeatureKind::polygon(&[[-3.0, -15.0], [27.0, 4.0], [5.0, 24.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(21),
                definition_id,
                name: "Unequal tool".into(),
                kind: FeatureKind::extrusion(FeatureId(20), Dimension::from_decimal("19").unwrap()),
            },
            CanonicalCommand::CreateFeature {
                id: producer_feature_id,
                definition_id,
                name: "Generic intersection".into(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Intersect,
                    target: FeatureId(11),
                    tool: FeatureId(21),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(77),
                definition_id,
                name: "General boolean occurrence".into(),
                transform: Transform::default(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let mut app = KetchupApp::new();
    app.document = document;
    app.reset_document_presentation();
    app.connect_exact_worker(&executable).unwrap();
    let context = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.exact.results.len() != 1 && Instant::now() < deadline {
        app.refresh_exact_products(&context);
        std::thread::sleep(Duration::from_millis(10));
    }

    let snapshot = app.document.current();
    let package = app
        .exact
        .results
        .get_render(&snapshot, definition_id)
        .expect("the general boolean chain must produce one exact body");
    assert!(matches!(
        package.as_ref(),
        ExactBodyPackage::Graph(package)
            if package.identity.producer_feature_id == producer_feature_id
    ));
    assert!(!package.topological_references().is_empty());
    assert!(
        package
            .mesh_export(Transform::identity())
            .mesh_obj
            .contains("g topological.face.")
    );
}

#[test]
fn gui_exact_publication_rolls_back_when_work_recovery_finalization_fails() {
    let executable = exact_worker_executable();
    assert!(executable.is_file());
    let directory = tempfile::tempdir().unwrap();
    let primary = directory
        .path()
        .join("transactional-exact-publication.ketchup");
    let recovery = ketchup_model::persistence::work_recovery_path(&primary);
    let mut app = KetchupApp::new();
    app.document = through_cut_document();
    app.reset_document_presentation();
    assert!(app.save_document_to(&primary));
    std::fs::create_dir(&recovery).unwrap();
    app.connect_exact_worker(&executable).unwrap();
    let before = app.document.current();
    let context = egui::Context::default();

    app.refresh_exact_products(&context);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while app.exact.task.is_some() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        app.refresh_exact_products(&context);
    }

    assert!(app.exact.task.is_none(), "exact worker did not complete");
    assert_eq!(
        app.document.current().canonical_digest(),
        before.canonical_digest()
    );
    assert_eq!(app.document.current().revision_id(), before.revision_id());
    assert_eq!(app.document.current().exact_reference_evidence().count(), 0);
    assert!(app.exact.results.is_empty());
    assert!(app.exact.topology_results.is_empty());

    std::fs::remove_dir(&recovery).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while app.exact.results.is_empty() && std::time::Instant::now() < deadline {
        app.refresh_exact_products(&context);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!app.exact.results.is_empty());
    assert!(app.document.current().exact_reference_evidence().count() > 0);
}

#[test]
fn running_app_uses_one_exact_cut_body_for_render_pick_and_export() {
    let executable = exact_worker_executable();
    assert!(
        executable.is_file(),
        "build workspace all-targets so the exact worker exists at {}",
        executable.display()
    );
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(63),
    ));
    app.document = through_cut_document();
    app.document
        .configure_human_confirmation_policy(app.confirmation_surface.verifying_key(), 1)
        .unwrap();
    app.reset_document_presentation();
    app.connect_exact_worker(&executable).unwrap();
    let before = app.document.current();
    let context = egui::Context::default();

    for _ in 0..200 {
        app.refresh_exact_products(&context);
        if app.exact_render_body_count() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(app.exact_render_body_count(), 1);
    assert_eq!(app.document.current().revision_id(), before.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        before.canonical_digest()
    );
    let projection = app.exact_projection(&app.document.current());
    assert!(
        projection
            .exact_surface_pick(
                Ray::new(Vec3::new(5.0, 5.0, 20.0), Vec3::new(0.0, 0.0, -1.0)).unwrap()
            )
            .is_none(),
        "the exact through-hole must not be filled by an axis-aligned proxy"
    );
    let wall = projection
        .exact_surface_pick(Ray::new(Vec3::new(5.0, 5.0, 5.0), Vec3::new(1.0, 0.0, 0.0)).unwrap())
        .expect("the cut wall must remain pickable");
    assert!((wall.position_mm.x - 6.0).abs() < 1.0e-6);
    assert!(
        wall.topological_target.is_some(),
        "the cut wall must be addressable through its topology"
    );

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("through-cut.obj");
    assert!(app.export_exact_occurrence_mesh_to(&InstancePath::root(OccurrenceId(10)), &path));
    let mesh = std::fs::read_to_string(&path).unwrap();
    assert!(mesh.contains("g topological.face."));
    assert!(mesh.lines().any(|line| line.starts_with("f ")));
    let loss = std::fs::read_to_string(path.with_extension("obj.loss.txt")).unwrap();
    assert!(loss.contains("authority=accepted exact OCCT B-Rep"));

    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(12),
                dimension: Dimension::from_decimal("11").unwrap(),
            },
        ]))
        .unwrap();
    assert_eq!(app.exact_render_body_count(), 0);
    let stale_snapshot = app.document.current();
    let stale_projection = app.exact_projection(&stale_snapshot);
    assert!(!stale_projection.contains_occurrence(&InstancePath::root(OccurrenceId(10))));
    assert!(
        app.viewport_boxes(&stale_snapshot, &stale_projection)
            .is_empty()
    );
    let stale_path = directory.path().join("stale-through-cut.obj");
    assert!(
        !app.export_exact_occurrence_mesh_to(&InstancePath::root(OccurrenceId(10)), &stale_path)
    );
    assert!(!stale_path.exists());
}

#[test]
fn contained_slanted_polygon_solid_tools_round_trip_atomically() {
    fn add_polygon_tool(app: &mut KetchupApp, points: &[[f64; 2]]) {
        let mut segments = points
            .windows(2)
            .map(|pair| ProfileSegment::Line {
                start_mm: pair[0],
                end_mm: pair[1],
            })
            .collect::<Vec<_>>();
        segments.push(ProfileSegment::Line {
            start_mm: *points.last().unwrap(),
            end_mm: points[0],
        });
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "Polygon tool".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: DefinitionId(2),
                    name: "Slanted containing profile".to_owned(),
                    kind: FeatureKind::Profile {
                        segments,
                        closed: true,
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(4),
                    definition_id: DefinitionId(2),
                    name: "Polygon tool extrusion".to_owned(),
                    kind: FeatureKind::extrusion(FeatureId(3), Dimension::new("20", 20.0).unwrap()),
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(2),
                    name: "Polygon tool occurrence".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                    tags: Default::default(),
                    visible: true,
                },
            ]))
            .unwrap();
        app.document.discard_history_before_current();
    }

    let target = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let tool = SelectionId {
        definition_id: DefinitionId(2),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };

    let mut app = KetchupApp::new();
    add_polygon_tool(
        &mut app,
        &[[-20.0, -10.0], [110.0, -15.0], [125.0, 70.0], [-15.0, 80.0]],
    );
    app.active_tool = ActiveTool::SolidUnion;
    let before_digest = app.canonical_digest();
    let before_revision = app.document_revision();
    let before_undo = app.document.visible_undo_steps();
    app.solid_tools.target = Some(target.clone());
    assert!(app.prepare_solid_tool_preview(tool.clone(), false));
    assert!(app.has_occurrence_operation_preview());
    assert_eq!(app.canonical_digest(), before_digest);
    assert_eq!(app.document_revision(), before_revision);
    assert_eq!(app.document.visible_undo_steps(), before_undo);
    assert_eq!(
        app.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(-20.0, -15.0, 0.0), Vec3::new(145.0, 95.0, 20.0)))
    );
    app.clear_ephemeral_edit_state();
    assert!(!app.has_occurrence_operation_preview());
    assert_eq!(app.canonical_digest(), before_digest);

    app.active_tool = ActiveTool::SolidUnion;
    app.solid_tools.target = Some(target);
    assert!(app.prepare_solid_tool_preview(tool, false));
    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.document.visible_undo_steps(), before_undo + 1);
    let committed = app.document.current();
    let result_definition = committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(committed.occurrence(OccurrenceId(2)).is_none());
    let polygon_result_feature_id = *committed
        .definition(result_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let graph =
        ExactBRepGraph::from_snapshot(&committed, result_definition, polygon_result_feature_id)
            .unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Union,
            ..
        }
    )));
    let committed_digest = committed.canonical_digest();
    let reopened = ketchup_model::persistence::load(&ketchup_model::persistence::save(&committed))
        .unwrap()
        .snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert!(
        ExactBRepGraph::from_snapshot(&reopened, result_definition, polygon_result_feature_id)
            .is_ok()
    );
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), committed_digest);

    let mut partial = KetchupApp::new();
    add_polygon_tool(
        &mut partial,
        &[[20.0, 15.0], [115.0, 12.0], [110.0, 45.0], [18.0, 42.0]],
    );
    partial.active_tool = ActiveTool::SolidUnion;
    let partial_digest = partial.canonical_digest();
    partial.solid_tools.target = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(partial.prepare_solid_tool_preview(
        SelectionId {
            definition_id: DefinitionId(2),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    ));
    assert_eq!(partial.canonical_digest(), partial_digest);
    assert!(partial.has_occurrence_operation_preview());

    let intersect_target = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let intersect_tool = SelectionId {
        definition_id: DefinitionId(2),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let mut intersection = KetchupApp::new();
    add_polygon_tool(
        &mut intersection,
        &[
            [12.0, 10.0],
            [70.0, 8.0],
            [88.0, 40.0],
            [55.0, 52.0],
            [15.0, 45.0],
        ],
    );
    intersection.active_tool = ActiveTool::SolidIntersect;
    let intersect_before_digest = intersection.canonical_digest();
    let intersect_before_revision = intersection.document_revision();
    let intersect_before_undo = intersection.document.visible_undo_steps();
    intersection.solid_tools.target = Some(intersect_target.clone());
    assert!(intersection.prepare_solid_tool_preview(intersect_tool.clone(), false));
    assert!(intersection.has_occurrence_operation_preview());
    assert_eq!(intersection.canonical_digest(), intersect_before_digest);
    assert_eq!(intersection.document_revision(), intersect_before_revision);
    assert_eq!(
        intersection.document.visible_undo_steps(),
        intersect_before_undo
    );
    assert_eq!(
        intersection.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(12.0, 8.0, 0.0), Vec3::new(76.0, 44.0, 20.0)))
    );
    intersection.clear_ephemeral_edit_state();
    assert!(!intersection.has_occurrence_operation_preview());
    assert_eq!(intersection.canonical_digest(), intersect_before_digest);

    intersection.active_tool = ActiveTool::SolidIntersect;
    intersection.solid_tools.target = Some(intersect_target);
    assert!(intersection.prepare_solid_tool_preview(intersect_tool, false));
    assert!(intersection.confirm_occurrence_operation_preview());
    assert_eq!(
        intersection.document.visible_undo_steps(),
        intersect_before_undo + 1
    );
    let intersected = intersection.document.current();
    let intersect_definition = intersected
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(intersected.occurrence(OccurrenceId(2)).is_none());
    let intersect_feature_id = *intersected
        .definition(intersect_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let intersect_graph =
        ExactBRepGraph::from_snapshot(&intersected, intersect_definition, intersect_feature_id)
            .unwrap();
    assert!(intersect_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Intersect,
            ..
        }
    )));
    let intersect_digest = intersected.canonical_digest();
    let reopened =
        ketchup_model::persistence::load(&ketchup_model::persistence::save(&intersected))
            .unwrap()
            .snapshot();
    assert_eq!(reopened.canonical_digest(), intersect_digest);
    assert!(
        ExactBRepGraph::from_snapshot(&reopened, intersect_definition, intersect_feature_id,)
            .is_ok()
    );
    assert!(intersection.undo());
    assert_eq!(intersection.canonical_digest(), intersect_before_digest);
    assert!(intersection.redo());
    assert_eq!(intersection.canonical_digest(), intersect_digest);

    let mut crossing = KetchupApp::new();
    add_polygon_tool(
        &mut crossing,
        &[[20.0, 10.0], [105.0, 8.0], [80.0, 50.0], [15.0, 45.0]],
    );
    crossing.active_tool = ActiveTool::SolidIntersect;
    let crossing_digest = crossing.canonical_digest();
    crossing.solid_tools.target = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(crossing.prepare_solid_tool_preview(
        SelectionId {
            definition_id: DefinitionId(2),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    ));
    assert_eq!(crossing.canonical_digest(), crossing_digest);
    assert!(crossing.has_occurrence_operation_preview());

    let mut split = KetchupApp::new();
    add_polygon_tool(
        &mut split,
        &[
            [12.0, 10.0],
            [70.0, 8.0],
            [88.0, 40.0],
            [55.0, 52.0],
            [15.0, 45.0],
        ],
    );
    split.active_tool = ActiveTool::SolidSplit;
    let split_before_digest = split.canonical_digest();
    let split_before_revision = split.document_revision();
    let split_before_undo = split.document.visible_undo_steps();
    split.solid_tools.target = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    let split_tool = SelectionId {
        definition_id: DefinitionId(2),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    assert!(split.prepare_solid_tool_preview(split_tool.clone(), false));
    assert!(split.has_occurrence_operation_preview());
    assert_eq!(split.canonical_digest(), split_before_digest);
    assert_eq!(split.document_revision(), split_before_revision);
    assert_eq!(split.document.visible_undo_steps(), split_before_undo);
    assert_eq!(
        split.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::ZERO, Vec3::new(100.0, 60.0, 20.0)))
    );
    split.clear_ephemeral_edit_state();
    assert!(!split.has_occurrence_operation_preview());
    assert_eq!(split.canonical_digest(), split_before_digest);

    split.active_tool = ActiveTool::SolidSplit;
    split.solid_tools.target = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(split.prepare_solid_tool_preview(split_tool, false));
    assert!(split.confirm_occurrence_operation_preview());
    assert_eq!(split.document.visible_undo_steps(), split_before_undo + 1);
    let split_snapshot = split.document.current();
    let split_definition = split_snapshot
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(split_snapshot.occurrence(OccurrenceId(2)).is_some());
    let split_feature_id = *split_snapshot
        .definition(split_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let split_graph =
        ExactBRepGraph::from_snapshot(&split_snapshot, split_definition, split_feature_id).unwrap();
    assert!(split_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Split,
            ..
        }
    )));
    let split_digest = split_snapshot.canonical_digest();
    let reopened =
        ketchup_model::persistence::load(&ketchup_model::persistence::save(&split_snapshot))
            .unwrap()
            .snapshot();
    assert_eq!(reopened.canonical_digest(), split_digest);
    assert!(ExactBRepGraph::from_snapshot(&reopened, split_definition, split_feature_id).is_ok());
    assert!(split.undo());
    assert_eq!(split.canonical_digest(), split_before_digest);
    assert!(split.redo());
    assert_eq!(split.canonical_digest(), split_digest);

    let mut boundary_touching = KetchupApp::new();
    add_polygon_tool(
        &mut boundary_touching,
        &[[20.0, 10.0], [100.0, 8.0], [80.0, 50.0], [15.0, 45.0]],
    );
    boundary_touching.active_tool = ActiveTool::SolidSplit;
    let boundary_digest = boundary_touching.canonical_digest();
    boundary_touching.solid_tools.target = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(boundary_touching.prepare_solid_tool_preview(
        SelectionId {
            definition_id: DefinitionId(2),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    ));
    assert_eq!(boundary_touching.canonical_digest(), boundary_digest);
    assert!(boundary_touching.has_occurrence_operation_preview());
}

#[test]
fn contained_circle_subtract_intersect_split_and_containing_union_round_trip_atomically() {
    fn app_with_circle_tool(center: [f64; 2], radius: f64) -> KetchupApp {
        let mut app = KetchupApp::new();
        let left = [center[0] - radius, center[1]];
        let right = [center[0] + radius, center[1]];
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "Circle tool".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: DefinitionId(2),
                    name: "Circle profile".to_owned(),
                    kind: FeatureKind::Profile {
                        segments: vec![
                            ProfileSegment::CircularArc {
                                start_mm: left,
                                end_mm: right,
                                center_mm: center,
                                clockwise: false,
                            },
                            ProfileSegment::CircularArc {
                                start_mm: right,
                                end_mm: left,
                                center_mm: center,
                                clockwise: false,
                            },
                        ],
                        closed: true,
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(4),
                    definition_id: DefinitionId(2),
                    name: "Circle extrusion".to_owned(),
                    kind: FeatureKind::extrusion(FeatureId(3), Dimension::new("20", 20.0).unwrap()),
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(2),
                    name: "Circle tool occurrence".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                    tags: Default::default(),
                    visible: true,
                },
            ]))
            .unwrap();
        app.document.discard_history_before_current();
        app
    }

    fn solid_tool_graph(snapshot: &Snapshot, definition_id: DefinitionId) -> ExactBRepGraph {
        let producer_feature_id = *snapshot
            .definition(definition_id)
            .unwrap()
            .feature_ids()
            .last()
            .unwrap();
        ExactBRepGraph::from_snapshot(snapshot, definition_id, producer_feature_id).unwrap()
    }

    let target = || SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let tool = || SelectionId {
        definition_id: DefinitionId(2),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };

    let mut subtract = app_with_circle_tool([40.0, 30.0], 10.0);
    subtract.active_tool = ActiveTool::SolidSubtract;
    let subtract_before_digest = subtract.canonical_digest();
    let subtract_before_revision = subtract.document_revision();
    let subtract_before_undo = subtract.document.visible_undo_steps();
    subtract.solid_tools.target = Some(target());
    assert!(subtract.prepare_solid_tool_preview(tool(), false));
    assert_eq!(subtract.canonical_digest(), subtract_before_digest);
    assert_eq!(subtract.document_revision(), subtract_before_revision);
    assert_eq!(subtract.document.visible_undo_steps(), subtract_before_undo);
    assert_eq!(
        subtract.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::ZERO, Vec3::new(100.0, 60.0, 20.0)))
    );
    assert_eq!(
        subtract.push_pull_preview_exact_evaluator(),
        Some(ketchup_model::exact_product::EXACT_BREP_GRAPH_EVALUATOR_V1)
    );
    subtract.clear_ephemeral_edit_state();
    assert!(!subtract.has_occurrence_operation_preview());
    assert_eq!(subtract.canonical_digest(), subtract_before_digest);

    subtract.active_tool = ActiveTool::SolidSubtract;
    subtract.solid_tools.target = Some(target());
    assert!(subtract.prepare_solid_tool_preview(tool(), false));
    assert!(subtract.confirm_occurrence_operation_preview());
    assert_eq!(
        subtract.document.visible_undo_steps(),
        subtract_before_undo + 1
    );
    let subtract_committed = subtract.document.current();
    let subtract_definition = subtract_committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(subtract_committed.occurrence(OccurrenceId(2)).is_none());
    let subtract_graph = solid_tool_graph(&subtract_committed, subtract_definition);
    assert!(subtract_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Cut,
            ..
        }
    )));
    let subtract_digest = subtract_committed.canonical_digest();
    let subtract_reopened =
        ketchup_model::persistence::load(&ketchup_model::persistence::save(&subtract_committed))
            .unwrap()
            .snapshot();
    assert_eq!(subtract_reopened.canonical_digest(), subtract_digest);
    assert_eq!(
        solid_tool_graph(&subtract_reopened, subtract_definition),
        subtract_graph
    );
    assert!(subtract.undo());
    assert_eq!(subtract.canonical_digest(), subtract_before_digest);
    assert!(subtract.redo());
    assert_eq!(subtract.canonical_digest(), subtract_digest);

    for (center, radius) in [([10.0, 30.0], 10.0), ([120.0, 30.0], 5.0)] {
        let mut rejected = app_with_circle_tool(center, radius);
        rejected.active_tool = ActiveTool::SolidSubtract;
        let digest = rejected.canonical_digest();
        rejected.solid_tools.target = Some(target());
        assert!(rejected.prepare_solid_tool_preview(tool(), false));
        assert_eq!(rejected.canonical_digest(), digest);
        assert!(rejected.has_occurrence_operation_preview());
    }

    let mut union = app_with_circle_tool([50.0, 30.0], 70.0);
    union.active_tool = ActiveTool::SolidUnion;
    let union_before_digest = union.canonical_digest();
    let union_before_revision = union.document_revision();
    let union_before_undo = union.document.visible_undo_steps();
    union.solid_tools.target = Some(target());
    assert!(union.prepare_solid_tool_preview(tool(), false));
    assert_eq!(union.canonical_digest(), union_before_digest);
    assert_eq!(union.document_revision(), union_before_revision);
    assert_eq!(union.document.visible_undo_steps(), union_before_undo);
    assert_eq!(
        union.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(-20.0, -40.0, 0.0), Vec3::new(140.0, 140.0, 20.0)))
    );
    union.clear_ephemeral_edit_state();
    assert!(!union.has_occurrence_operation_preview());
    assert_eq!(union.canonical_digest(), union_before_digest);

    union.active_tool = ActiveTool::SolidUnion;
    union.solid_tools.target = Some(target());
    assert!(union.prepare_solid_tool_preview(tool(), false));
    assert!(union.confirm_occurrence_operation_preview());
    assert_eq!(union.document.visible_undo_steps(), union_before_undo + 1);
    let union_committed = union.document.current();
    let union_definition = union_committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(union_committed.occurrence(OccurrenceId(2)).is_none());
    let union_graph = solid_tool_graph(&union_committed, union_definition);
    assert!(union_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Union,
            ..
        }
    )));
    let union_digest = union_committed.canonical_digest();
    let union_reopened =
        ketchup_model::persistence::load(&ketchup_model::persistence::save(&union_committed))
            .unwrap()
            .snapshot();
    assert_eq!(union_reopened.canonical_digest(), union_digest);
    assert_eq!(
        solid_tool_graph(&union_reopened, union_definition),
        union_graph
    );
    assert!(union.undo());
    assert_eq!(union.canonical_digest(), union_before_digest);
    assert!(union.redo());
    assert_eq!(union.canonical_digest(), union_digest);

    for (center, radius) in [
        ([40.0, 30.0], 10.0),
        ([50.0, 30.0], 55.0),
        ([150.0, 30.0], 10.0),
        ([50.0, 30.0], 58.309_518_948_453_004),
    ] {
        let mut rejected = app_with_circle_tool(center, radius);
        rejected.active_tool = ActiveTool::SolidUnion;
        let digest = rejected.canonical_digest();
        rejected.solid_tools.target = Some(target());
        assert!(rejected.prepare_solid_tool_preview(tool(), false));
        assert_eq!(rejected.canonical_digest(), digest);
        assert!(rejected.has_occurrence_operation_preview());
    }

    let mut app = app_with_circle_tool([40.0, 30.0], 10.0);
    app.active_tool = ActiveTool::SolidIntersect;
    let before_digest = app.canonical_digest();
    let before_revision = app.document_revision();
    let before_undo = app.document.visible_undo_steps();
    app.solid_tools.target = Some(target());
    assert!(app.prepare_solid_tool_preview(tool(), false));
    assert_eq!(app.canonical_digest(), before_digest);
    assert_eq!(app.document_revision(), before_revision);
    assert_eq!(app.document.visible_undo_steps(), before_undo);
    assert_eq!(
        app.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(30.0, 20.0, 0.0), Vec3::new(20.0, 20.0, 20.0)))
    );
    app.clear_ephemeral_edit_state();
    assert!(!app.has_occurrence_operation_preview());
    assert_eq!(app.canonical_digest(), before_digest);

    app.active_tool = ActiveTool::SolidIntersect;
    app.solid_tools.target = Some(target());
    assert!(app.prepare_solid_tool_preview(tool(), false));
    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.document.visible_undo_steps(), before_undo + 1);
    let committed = app.document.current();
    let result_definition = committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(committed.occurrence(OccurrenceId(2)).is_none());
    let result_graph = solid_tool_graph(&committed, result_definition);
    assert!(result_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Intersect,
            ..
        }
    )));
    let committed_digest = committed.canonical_digest();
    let reopened = ketchup_model::persistence::load(&ketchup_model::persistence::save(&committed))
        .unwrap()
        .snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert_eq!(solid_tool_graph(&reopened, result_definition), result_graph);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), committed_digest);

    let mut split = app_with_circle_tool([40.0, 30.0], 10.0);
    split.active_tool = ActiveTool::SolidSplit;
    let split_before_digest = split.canonical_digest();
    let split_before_revision = split.document_revision();
    let split_before_undo = split.document.visible_undo_steps();
    split.solid_tools.target = Some(target());
    assert!(split.prepare_solid_tool_preview(tool(), false));
    assert_eq!(split.canonical_digest(), split_before_digest);
    assert_eq!(split.document_revision(), split_before_revision);
    assert_eq!(split.document.visible_undo_steps(), split_before_undo);
    assert_eq!(
        split.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    split.clear_ephemeral_edit_state();
    assert!(!split.has_occurrence_operation_preview());
    assert_eq!(split.canonical_digest(), split_before_digest);

    split.active_tool = ActiveTool::SolidSplit;
    split.solid_tools.target = Some(target());
    assert!(split.prepare_solid_tool_preview(tool(), false));
    assert!(split.confirm_occurrence_operation_preview());
    assert_eq!(split.document.visible_undo_steps(), split_before_undo + 1);
    let split_committed = split.document.current();
    let split_definition = split_committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(split_committed.occurrence(OccurrenceId(2)).is_some());
    let split_graph = solid_tool_graph(&split_committed, split_definition);
    assert!(split_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Split,
            ..
        }
    )));
    let split_digest = split_committed.canonical_digest();
    let split_reopened =
        ketchup_model::persistence::load(&ketchup_model::persistence::save(&split_committed))
            .unwrap()
            .snapshot();
    assert_eq!(split_reopened.canonical_digest(), split_digest);
    assert_eq!(
        solid_tool_graph(&split_reopened, split_definition),
        split_graph
    );
    assert!(split.undo());
    assert_eq!(split.canonical_digest(), split_before_digest);
    assert!(split.redo());
    assert_eq!(split.canonical_digest(), split_digest);

    for active_tool in [ActiveTool::SolidIntersect, ActiveTool::SolidSplit] {
        let mut boundary_tangent = app_with_circle_tool([10.0, 30.0], 10.0);
        boundary_tangent.active_tool = active_tool;
        let digest = boundary_tangent.canonical_digest();
        boundary_tangent.solid_tools.target = Some(target());
        assert!(boundary_tangent.prepare_solid_tool_preview(tool(), false));
        assert_eq!(boundary_tangent.canonical_digest(), digest);
        assert!(boundary_tangent.has_occurrence_operation_preview());

        let mut disjoint = app_with_circle_tool([120.0, 30.0], 5.0);
        disjoint.active_tool = active_tool;
        let digest = disjoint.canonical_digest();
        disjoint.solid_tools.target = Some(target());
        assert!(!disjoint.prepare_solid_tool_preview(tool(), false));
        assert_eq!(disjoint.canonical_digest(), digest);
        assert!(!disjoint.has_occurrence_operation_preview());
    }
}

#[test]
fn imported_exact_occurrences_route_through_solid_tool_preview_and_commit() {
    let mut app = KetchupApp::new();
    app.document = DocumentStore::new();
    let sources = [
        b"target imported exact body".as_slice(),
        b"tool imported exact body".as_slice(),
    ];
    let evidences = [
        StepImportEvidence {
            source_sha256: ketchup_model::graph::sha256_bytes(sources[0]),
            source_byte_len: sources[0].len() as u64,
            source_unit: ImportLengthUnit::Millimetre,
            result_fingerprint: "target-exact-result".into(),
            body_kind: ketchup_model::document::BodyKind::Solid,
            solid_count: 1,
            topology_counts: [8, 12, 6, 1, 1],
            area_mm2: 1.0,
            volume_mm3: 1_000.0,
            bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
            backend: "headless-imported-solid-tool.v1".into(),
            tolerance: "1e-7-mm".into(),
        },
        StepImportEvidence {
            source_sha256: ketchup_model::graph::sha256_bytes(sources[1]),
            source_byte_len: sources[1].len() as u64,
            source_unit: ImportLengthUnit::Millimetre,
            result_fingerprint: "tool-exact-result".into(),
            body_kind: ketchup_model::document::BodyKind::Solid,
            solid_count: 1,
            topology_counts: [8, 12, 6, 1, 1],
            area_mm2: 1.0,
            volume_mm3: 432.0,
            bounds_mm: [[0.0, 0.0, 0.0], [6.0, 6.0, 12.0]],
            backend: "headless-imported-solid-tool.v1".into(),
            tolerance: "1e-7-mm".into(),
        },
    ];
    for (index, (source, evidence)) in sources.iter().zip(&evidences).enumerate() {
        app.document
            .apply_batch(
                &plan_step_import(
                    &app.document.current(),
                    source,
                    &format!("imported-{index}.step"),
                    evidence,
                )
                .unwrap(),
            )
            .unwrap();
    }
    app.document.discard_history_before_current();
    let snapshot = app.document.current();
    let mut occurrences = snapshot.occurrences().map(|occurrence| occurrence.id());
    let target_occurrence_id = occurrences.next().unwrap();
    let tool_occurrence_id = occurrences.next().unwrap();
    let target_definition_id = snapshot
        .occurrence(target_occurrence_id)
        .unwrap()
        .definition_id();
    let tool_definition_id = snapshot
        .occurrence(tool_occurrence_id)
        .unwrap()
        .definition_id();
    let target_feature_id = exact_solid_tool_feature_id(&snapshot, target_definition_id).unwrap();
    let tool_feature_id = exact_solid_tool_feature_id(&snapshot, tool_definition_id).unwrap();
    let angle = 30.0_f64.to_radians();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: target_occurrence_id,
                transform: Transform::from_translation(2.0, -1.0, 0.5).unwrap(),
            },
            CanonicalCommand::SetOccurrenceTransform {
                id: tool_occurrence_id,
                transform: Transform::from_matrix([
                    angle.cos(),
                    -angle.sin(),
                    0.0,
                    4.0,
                    angle.sin(),
                    angle.cos(),
                    0.0,
                    1.0,
                    0.0,
                    0.0,
                    1.0,
                    2.0,
                    0.0,
                    0.0,
                    0.0,
                    1.0,
                ])
                .unwrap(),
            },
        ]))
        .unwrap();
    app.document.discard_history_before_current();

    let selection = |definition_id, occurrence_id| SelectionId {
        definition_id,
        instance_path: InstancePath::root(occurrence_id),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.active_tool = ActiveTool::SolidIntersect;
    let imported_boxes = app.active_boxes();
    assert_eq!(imported_boxes.len(), 2, "{imported_boxes:?}");
    let snapshot = app.document.current();
    let exact_projection = app.exact_projection(&snapshot);
    assert!(app.viewport_boxes(&snapshot, &exact_projection).is_empty());
    app.refresh_interaction_projection_cache(&snapshot);
    assert_eq!(
        app.hover
            .projection_cache
            .borrow()
            .as_ref()
            .unwrap()
            .boxes
            .occurrence_count(),
        0
    );
    assert!(app.command_enabled(AppCommand::SolidIntersect));
    let before_digest = app.canonical_digest();
    let before_revision = app.document_revision();
    let before_undo_steps = app.undo_step_count();
    app.solid_tools.target = Some(selection(target_definition_id, target_occurrence_id));
    assert!(
        app.prepare_solid_tool_preview(selection(tool_definition_id, tool_occurrence_id), true,)
    );
    assert!(app.has_occurrence_operation_preview());
    let source = &app
        .tool_preview
        .get::<OccurrenceOperationPreview>()
        .unwrap()
        .solid_tool_plan
        .as_ref()
        .unwrap()
        .source;
    assert_eq!(source.target_feature_id, target_feature_id);
    assert_eq!(source.tool_feature_id, tool_feature_id);
    assert_eq!(app.canonical_digest(), before_digest);
    assert_eq!(app.document_revision(), before_revision);
    assert_eq!(app.undo_step_count(), before_undo_steps);

    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.undo_step_count(), before_undo_steps + 1);
    let committed = app.document.current();
    let result_definition_id = committed
        .occurrence(target_occurrence_id)
        .unwrap()
        .definition_id();
    let result_feature_id = *committed
        .definition(result_definition_id)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let graph =
        ExactBRepGraph::from_snapshot(&committed, result_definition_id, result_feature_id).unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Intersect,
            ..
        }
    )));
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| matches!(node.operation, ExactBRepOperation::ImportedExact { .. }))
            .count(),
        2
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| matches!(node.operation, ExactBRepOperation::RigidTransform { .. }))
            .count(),
        2
    );
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), committed.canonical_digest());
}

#[test]
fn mixed_extrusion_and_imported_exact_occurrences_route_through_solid_tools() {
    let mut app = KetchupApp::new();
    let source = b"mixed imported exact body";
    let evidence = StepImportEvidence {
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "mixed-imported-exact-result".into(),
        body_kind: ketchup_model::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 1.0,
        volume_mm3: 12_000.0,
        bounds_mm: [[0.0, 0.0, 0.0], [20.0, 30.0, 20.0]],
        backend: "headless-mixed-solid-tool.v1".into(),
        tolerance: "1e-7-mm".into(),
    };
    app.document
        .apply_batch(
            &plan_step_import(
                &app.document.current(),
                source,
                "mixed-imported.step",
                &evidence,
            )
            .unwrap(),
        )
        .unwrap();
    let snapshot = app.document.current();
    let imported_occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.id() != OccurrenceId(1))
        .unwrap();
    let imported_occurrence_id = imported_occurrence.id();
    let imported_definition_id = imported_occurrence.definition_id();
    app.file
        .container_data
        .insert_import_blob(source.to_vec())
        .unwrap();
    let target_angle = 15.0_f64.to_radians();
    let target_transform = Transform::from_matrix([
        target_angle.cos(),
        -target_angle.sin(),
        0.0,
        5.0,
        target_angle.sin(),
        target_angle.cos(),
        0.0,
        -3.0,
        0.0,
        0.0,
        1.0,
        2.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ])
    .unwrap();
    let tool_angle = 30.0_f64.to_radians();
    let tool_transform = Transform::from_matrix([
        tool_angle.cos(),
        -tool_angle.sin(),
        0.0,
        40.0,
        tool_angle.sin(),
        tool_angle.cos(),
        0.0,
        15.0,
        0.0,
        0.0,
        1.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ])
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: target_transform,
            },
            CanonicalCommand::SetOccurrenceTransform {
                id: imported_occurrence_id,
                transform: tool_transform,
            },
        ]))
        .unwrap();
    app.document.discard_history_before_current();

    let target = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let tool = SelectionId {
        definition_id: imported_definition_id,
        instance_path: InstancePath::root(imported_occurrence_id),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    assert!(app.command_enabled(AppCommand::SolidSubtract));
    assert!(app.command_enabled(AppCommand::SolidUnion));
    assert!(app.command_enabled(AppCommand::SolidIntersect));
    app.active_tool = ActiveTool::SolidUnion;
    let before_digest = app.canonical_digest();
    let before_undo = app.undo_step_count();
    app.solid_tools.target = Some(target);
    assert!(app.prepare_solid_tool_preview(tool.clone(), true));
    assert_eq!(app.canonical_digest(), before_digest);
    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.undo_step_count(), before_undo + 1);

    let committed = app.document.current();
    let result_definition_id = committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    let result_feature_id = *committed
        .definition(result_definition_id)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let graph =
        ExactBRepGraph::from_snapshot(&committed, result_definition_id, result_feature_id).unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::Extrude { .. }))
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::ImportedExact { .. }))
    );
    let expected_relative_matrix = target_transform
        .rigid_inverse()
        .unwrap()
        .compose(tool_transform)
        .matrix()
        .map(f64::to_bits);
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::RigidTransform { matrix_bits, .. }
            if matrix_bits == expected_relative_matrix
    )));
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Union,
            ..
        }
    )));
    let reopened = ketchup_model::persistence::load(
        &ketchup_model::persistence::save_container(&committed, &app.file.container_data).unwrap(),
    )
    .unwrap()
    .snapshot();
    assert_eq!(reopened.canonical_digest(), committed.canonical_digest());

    assert_eq!(
        exact_solid_tool_feature_id(&committed, result_definition_id),
        Some(result_feature_id)
    );
    assert!(
        !app.active_boxes()
            .iter()
            .any(|item| item.definition_id == result_definition_id),
        "graph-derived bodies without a current exact package must fail closed"
    );
    app.active_tool = ActiveTool::SolidSubtract;
    app.solid_tools.target = Some(SelectionId {
        definition_id: result_definition_id,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(!app.prepare_solid_tool_preview(tool.clone(), true));
    assert_eq!(app.canonical_digest(), committed.canonical_digest());

    install_graph_result(
        &mut app,
        result_definition_id,
        result_feature_id,
        Some([[-100.0, -100.0, -100.0], [200.0, 200.0, 200.0]]),
    );
    assert!(
        app.active_boxes()
            .iter()
            .any(|item| item.definition_id == result_definition_id),
        "a graph-derived body with a current exact package must be selectable"
    );
    assert!(app.prepare_solid_tool_preview(tool.clone(), true));
    let graph_preview = app
        .tool_preview
        .get::<OccurrenceOperationPreview>()
        .unwrap()
        .solid_tool_plan
        .as_ref()
        .unwrap();
    assert_eq!(graph_preview.source.result_feature_ids.len(), 9);
    assert_eq!(app.canonical_digest(), committed.canonical_digest());
    let graph_before_undo = app.undo_step_count();
    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.undo_step_count(), graph_before_undo + 1);
    let nested = app.document.current();
    let nested_definition_id = nested.occurrence(OccurrenceId(1)).unwrap().definition_id();
    let nested_definition = nested.definition(nested_definition_id).unwrap();
    assert_eq!(nested_definition.feature_ids().len(), 9);
    let nested_result = *nested_definition.feature_ids().last().unwrap();
    let nested_graph =
        ExactBRepGraph::from_snapshot(&nested, nested_definition_id, nested_result).unwrap();
    assert_eq!(
        nested_graph
            .nodes
            .iter()
            .filter(|node| matches!(node.operation, ExactBRepOperation::Boolean { .. }))
            .count(),
        2
    );
    assert!(matches!(
        nested_graph.nodes.last().unwrap().operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Cut,
            ..
        }
    ));
    let nested_digest = nested.canonical_digest();
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), committed.canonical_digest());
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), nested_digest);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), committed.canonical_digest());

    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), committed.canonical_digest());
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_digest);

    app.active_tool = ActiveTool::SolidIntersect;
    app.solid_tools.target = Some(SelectionId {
        definition_id: imported_definition_id,
        instance_path: InstancePath::root(imported_occurrence_id),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(app.prepare_solid_tool_preview(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    ));
    let reverse_preview = app
        .tool_preview
        .get::<OccurrenceOperationPreview>()
        .unwrap()
        .solid_tool_plan
        .as_ref()
        .unwrap();
    assert_eq!(
        reverse_preview.preview_box.profile_feature_id,
        reverse_preview.source.result_feature_ids[4]
    );
    assert!(reverse_preview.preview_box.extrusion_feature_id.is_none());
    assert!(app.confirm_occurrence_operation_preview());
    let reverse = app.document.current();
    assert!(reverse.occurrence(OccurrenceId(1)).is_none());
    let reverse_definition_id = reverse
        .occurrence(imported_occurrence_id)
        .unwrap()
        .definition_id();
    let reverse_feature_id = *reverse
        .definition(reverse_definition_id)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let reverse_graph =
        ExactBRepGraph::from_snapshot(&reverse, reverse_definition_id, reverse_feature_id).unwrap();
    let reverse_relative_matrix = tool_transform
        .rigid_inverse()
        .unwrap()
        .compose(target_transform)
        .matrix()
        .map(f64::to_bits);
    assert!(reverse_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::RigidTransform { matrix_bits, .. }
            if matrix_bits == reverse_relative_matrix
    )));
    assert!(reverse_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_model::exact_brep_graph::ExactBRepBooleanOperation::Intersect,
            ..
        }
    )));
}

#[test]
fn solid_tool_preview_survives_accepted_exact_bounds_refresh_and_commits_once() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let target = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let tool = SelectionId {
        definition_id: app
            .document
            .current()
            .occurrence(OccurrenceId(2))
            .unwrap()
            .definition_id(),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.active_tool = ActiveTool::SolidIntersect;
    app.solid_tools.target = Some(target);
    assert!(app.prepare_solid_tool_preview(tool, false));
    let preview_geometry = app
        .occurrence_operation_preview_geometry(OccurrenceId(1))
        .unwrap();
    let snapshot = app.document.current();
    let source_document_id = snapshot.document_id();
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo_steps = app.undo_step_count();

    let graph =
        ExactBRepGraph::from_snapshot(&snapshot, INITIAL_BOX_DEFINITION, FeatureId(2)).unwrap();
    let minimum = Vec3::new(-10.0, -5.0, 0.0);
    let maximum = Vec3::new(110.0, 75.0, 20.0);
    let size = maximum - minimum;
    let vertices_mm = box_corners(size.x, size.y, size.z)
        .map(|point| {
            let point = point + minimum;
            [point.x, point.y, point.z]
        })
        .to_vec();
    let triangles = [
        ([0, 2, 1], 0),
        ([1, 2, 3], 0),
        ([4, 5, 6], 1),
        ([5, 7, 6], 1),
        ([0, 1, 4], 2),
        ([1, 5, 4], 2),
        ([2, 6, 3], 3),
        ([3, 6, 7], 3),
        ([0, 4, 2], 4),
        ([2, 4, 6], 4),
        ([1, 3, 5], 5),
        ([3, 7, 5], 5),
    ]
    .into_iter()
    .map(|(vertex_indices, face_ordinal)| StepMeshTriangle {
        vertex_indices,
        face_ordinal,
    })
    .collect();
    let package = ExactBRepGraphPackage::from_worker_evidence(
        &graph,
        ExactBRepGraphWorkerEvidence {
            exact_input_digest: "solid-tool-refreshed-input".into(),
            result_fingerprint: "solid-tool-refreshed-result".into(),
            volume_mm3: size.x * size.y * size.z,
            area_mm2: 0.0,
            topology_counts: [8, 12, 6, 1, 1],
            wire_count: None,
            bounds_mm: [
                [minimum.x, minimum.y, minimum.z],
                [maximum.x, maximum.y, maximum.z],
            ],
            backend: "solid-tool-headless-backend.v1".into(),
            tolerance: "1e-7-mm".into(),
            faces: Vec::new(),
            edges: Vec::new(),
        },
        &StepImportMesh {
            vertices_mm,
            triangles,
        },
    )
    .unwrap();
    assert!(app.headless_install_exact_package(ExactBodyPackage::Graph(package)));

    assert_eq!(app.document.current().document_id(), source_document_id);
    assert_eq!(app.document_revision(), before_revision);
    assert_eq!(app.canonical_digest(), before_digest);
    assert_eq!(app.undo_step_count(), before_undo_steps);
    assert_eq!(
        app.occurrence_box_geometry(1),
        Some((minimum, size)),
        "the accepted exact package must change only the target's derived bounds"
    );
    assert_eq!(
        app.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some(preview_geometry)
    );
    assert!(app.has_occurrence_operation_preview());

    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.document_revision(), before_revision + 1);
    assert_eq!(app.undo_step_count(), before_undo_steps + 1);
}

#[test]
fn solid_tool_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_intersection() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.create_box());
        let target = SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        let tool = SelectionId {
            definition_id: app
                .document
                .current()
                .occurrence(OccurrenceId(2))
                .unwrap()
                .definition_id(),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        app.active_tool = ActiveTool::SolidIntersect;
        app.solid_tools.target = Some(target);
        assert!(app.prepare_solid_tool_preview(tool, false));
        assert!(app.has_occurrence_operation_preview());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut command_tamper = prepared_intersection();
    let revision = command_tamper.document_revision();
    let digest = command_tamper.canonical_digest();
    let undo_steps = command_tamper.undo_step_count();
    command_tamper
        .tool_preview
        .get_mut::<OccurrenceOperationPreview>()
        .unwrap()
        .solid_tool_plan
        .as_mut()
        .unwrap()
        .command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!command_tamper.has_occurrence_operation_preview());
    assert!(!command_tamper.confirm_occurrence_operation_preview());
    assert_unchanged(&command_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_intersection();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .tool_preview
        .get_mut::<OccurrenceOperationPreview>()
        .unwrap()
        .solid_tool_plan
        .as_mut()
        .unwrap()
        .source
        .tool_transform = Transform::identity();
    assert!(!source_tamper.confirm_occurrence_operation_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_intersection();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_occurrence_operation_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_intersection();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_occurrence_operation_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_intersection();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(2),
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_occurrence_operation_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_intersection();
    let before_digest = valid.canonical_digest();
    let before_revision = valid.document_revision();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_occurrence_operation_preview());
    let committed_digest = valid.canonical_digest();
    assert_eq!(valid.document_revision(), before_revision + 1);
    assert_eq!(valid.undo_step_count(), before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    let committed_revision = valid.document_revision();
    let committed_undo_steps = valid.undo_step_count();
    assert!(!valid.confirm_occurrence_operation_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn revolve_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_revolve() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.create_closed_polyline(vec![
            [120.0, 0.0],
            [140.0, 0.0],
            [140.0, 30.0],
            [120.0, 30.0],
        ]));
        assert!(app.begin_revolve_tool());
        assert!(!app.add_revolve_axis_point(Vec3::new(110.0, 0.0, 0.0)));
        assert!(app.add_revolve_axis_point(Vec3::new(110.0, 30.0, 0.0)));
        assert!(app.has_revolve_preview());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_revolve();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper
        .tool_preview
        .get_mut::<RevolvePreview>()
        .unwrap()
        .batch = CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    }]);
    assert!(!batch_tamper.has_revolve_preview());
    assert!(!batch_tamper.confirm_revolve_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut plan_tamper = prepared_revolve();
    let revision = plan_tamper.document_revision();
    let digest = plan_tamper.canonical_digest();
    let undo_steps = plan_tamper.undo_step_count();
    plan_tamper
        .tool_preview
        .get_mut::<RevolvePreview>()
        .unwrap()
        .plan
        .command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!plan_tamper.confirm_revolve_preview());
    assert_unchanged(&plan_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_revolve();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .tool_preview
        .get_mut::<RevolvePreview>()
        .unwrap()
        .plan
        .source
        .profile_kind = FeatureKind::polygon(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
    assert!(!source_tamper.confirm_revolve_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_revolve();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_revolve_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_revolve();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_revolve_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_revolve();
    let occurrence_id = stale
        .tool_preview
        .get::<RevolvePreview>()
        .unwrap()
        .plan
        .source
        .source_primary
        .as_ref()
        .unwrap()
        .instance_path
        .root_occurrence();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: occurrence_id,
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_revolve_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_revolve();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_revolve_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_revolve_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn sweep_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_sweep() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.create_sweep_inputs(
            vec![[-5.0, -10.0], [5.0, -10.0], [5.0, 10.0], [-5.0, 10.0]],
            [0.0, 0.0],
            [0.0, 125.0],
        ));
        app.dispatch_command(AppCommand::Sweep);
        assert!(app.sweep_preview_is_current());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_sweep();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper
        .tool_preview
        .get_mut::<SweepPreview>()
        .unwrap()
        .batch = CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    }]);
    assert!(!batch_tamper.sweep_preview_is_current());
    assert!(!batch_tamper.confirm_sweep_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut plan_tamper = prepared_sweep();
    let revision = plan_tamper.document_revision();
    let digest = plan_tamper.canonical_digest();
    let undo_steps = plan_tamper.undo_step_count();
    plan_tamper
        .tool_preview
        .get_mut::<SweepPreview>()
        .unwrap()
        .plan
        .command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!plan_tamper.confirm_sweep_preview());
    assert_unchanged(&plan_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_sweep();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .tool_preview
        .get_mut::<SweepPreview>()
        .unwrap()
        .plan
        .exact_graph
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_sweep_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_sweep();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .tool_preview
        .get_mut::<SweepPreview>()
        .unwrap()
        .plan
        .source
        .profile_kind = FeatureKind::polygon(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
    assert!(!source_tamper.confirm_sweep_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_sweep();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_sweep_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_sweep();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_sweep_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_sweep();
    let occurrence_id = stale
        .tool_preview
        .get::<SweepPreview>()
        .unwrap()
        .plan
        .source
        .source_primary
        .as_ref()
        .unwrap()
        .instance_path
        .root_occurrence();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: occurrence_id,
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_sweep_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_sweep();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_sweep_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_sweep_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn loft_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_loft() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.create_loft_inputs(vec![
            (
                vec![[-20.0, -10.0], [20.0, -10.0], [20.0, 10.0], [-20.0, 10.0]],
                0.0,
            ),
            (
                vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]],
                80.0,
            ),
        ]));
        app.dispatch_command(AppCommand::Loft);
        assert!(app.loft_preview_is_current());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_loft();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper
        .tool_preview
        .get_mut::<LoftPreview>()
        .unwrap()
        .batch = CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    }]);
    assert!(!batch_tamper.loft_preview_is_current());
    assert!(!batch_tamper.confirm_loft_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut plan_tamper = prepared_loft();
    let revision = plan_tamper.document_revision();
    let digest = plan_tamper.canonical_digest();
    let undo_steps = plan_tamper.undo_step_count();
    plan_tamper
        .tool_preview
        .get_mut::<LoftPreview>()
        .unwrap()
        .plan
        .command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!plan_tamper.confirm_loft_preview());
    assert_unchanged(&plan_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_loft();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .tool_preview
        .get_mut::<LoftPreview>()
        .unwrap()
        .plan
        .exact_graph
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_loft_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_loft();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .tool_preview
        .get_mut::<LoftPreview>()
        .unwrap()
        .plan
        .source
        .profile_kinds[0]
        .1 = FeatureKind::closed_spline(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    assert!(!source_tamper.confirm_loft_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_loft();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_loft_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_loft();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_loft_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_loft();
    let occurrence_id = stale
        .tool_preview
        .get::<LoftPreview>()
        .unwrap()
        .plan
        .source
        .source_primary
        .as_ref()
        .unwrap()
        .instance_path
        .root_occurrence();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: occurrence_id,
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_loft_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_loft();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_loft_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_loft_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn manual_and_assistant_multi_edge_finish_share_the_canonical_plan() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    let locator = |ordinal| TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id: FeatureId(2),
        kind: TopologicalElementKind::Edge,
        ordinal,
    };
    assert!(app.select_topological_locator(locator(9)));
    assert!(app.select_topological_locator_additive(locator(2), true));
    app.dispatch_command(AppCommand::Fillet);
    let manual_preview = app.tool_preview.get::<GeneralFinishPreview>().unwrap();
    let CanonicalCommand::CreateFeature {
        kind: manual_kind, ..
    } = &manual_preview.plan.command
    else {
        panic!("manual finish preview must create one feature");
    };
    let mut reference_ids = app
        .general_finish_preview_selection_parameters()
        .unwrap()
        .1
        .into_iter()
        .map(|reference| reference.lineage_digest)
        .collect::<Vec<_>>();
    reference_ids.reverse();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::FilletEdges {
            definition_id: INITIAL_BOX_DEFINITION.0,
            name: "Assistant multi-edge fillet".to_owned(),
            target_feature_id: 2,
            edge_reference_ids: reference_ids,
            radius_mm: 1.0,
        }],
    };
    let assistant_batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    let [
        CanonicalCommand::CreateFeature {
            kind: assistant_kind,
            ..
        },
    ] = assistant_batch.commands()
    else {
        panic!("assistant finish plan must create one feature");
    };
    assert_eq!(manual_kind, assistant_kind);
}

#[test]
fn topology_finish_planner_canonicalizes_shell_and_chamfer_permutations() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    let locator = |kind, ordinal| TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id: FeatureId(2),
        kind,
        ordinal,
    };

    assert!(app.select_topological_locator(locator(TopologicalElementKind::Face, 5)));
    assert!(
        app.select_topological_locator_additive(locator(TopologicalElementKind::Face, 1), true)
    );
    app.dispatch_command(AppCommand::Shell);
    let (_, faces, _, _) = app
        .general_finish_preview_selection_parameters()
        .expect("two selected faces must plan a shell");
    let mut reversed_faces = faces.clone();
    reversed_faces.reverse();
    assert_eq!(
        plan_topology_finish_kind(
            GeneralFinishKind::Shell,
            FeatureId(2),
            faces,
            Dimension::from_decimal("1").unwrap(),
        ),
        plan_topology_finish_kind(
            GeneralFinishKind::Shell,
            FeatureId(2),
            reversed_faces,
            Dimension::from_decimal("1").unwrap(),
        )
    );

    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    assert!(app.select_topological_locator(locator(TopologicalElementKind::Edge, 9)));
    assert!(
        app.select_topological_locator_additive(locator(TopologicalElementKind::Edge, 2), true)
    );
    app.dispatch_command(AppCommand::Chamfer);
    let (_, edges, _, _) = app
        .general_finish_preview_selection_parameters()
        .expect("two selected edges must plan a chamfer");
    let mut reversed_edges = edges.clone();
    reversed_edges.reverse();
    assert_eq!(
        plan_topology_finish_kind(
            GeneralFinishKind::Chamfer,
            FeatureId(2),
            edges,
            Dimension::from_decimal("1").unwrap(),
        ),
        plan_topology_finish_kind(
            GeneralFinishKind::Chamfer,
            FeatureId(2),
            reversed_edges,
            Dimension::from_decimal("1").unwrap(),
        )
    );
}

#[test]
fn general_finish_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_shell() -> KetchupApp {
        let mut app = KetchupApp::new();
        install_initial_graph_result(&mut app);
        select_initial_topological(&mut app, TopologicalElementKind::Face, 3);
        app.dispatch_command(AppCommand::Shell);
        assert!(app.general_finish_preview_is_current());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_shell();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper
        .tool_preview
        .get_mut::<GeneralFinishPreview>()
        .unwrap()
        .batch = CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    }]);
    assert!(!batch_tamper.general_finish_preview_is_current());
    assert!(!batch_tamper.confirm_general_finish_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut command_tamper = prepared_shell();
    let revision = command_tamper.document_revision();
    let digest = command_tamper.canonical_digest();
    let undo_steps = command_tamper.undo_step_count();
    command_tamper
        .tool_preview
        .get_mut::<GeneralFinishPreview>()
        .unwrap()
        .plan
        .command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!command_tamper.confirm_general_finish_preview());
    assert_unchanged(&command_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_shell();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .tool_preview
        .get_mut::<GeneralFinishPreview>()
        .unwrap()
        .plan
        .exact_graph
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_general_finish_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_shell();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .tool_preview
        .get_mut::<GeneralFinishPreview>()
        .unwrap()
        .plan
        .source
        .target_feature_kind = FeatureKind::polygon(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]);
    assert!(!source_tamper.confirm_general_finish_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut amount_tamper = prepared_shell();
    let revision = amount_tamper.document_revision();
    let digest = amount_tamper.canonical_digest();
    let undo_steps = amount_tamper.undo_step_count();
    amount_tamper
        .tool_preview
        .get_mut::<GeneralFinishPreview>()
        .unwrap()
        .plan
        .amount_mm_bits = 3.0_f64.to_bits();
    assert!(!amount_tamper.confirm_general_finish_preview());
    assert_unchanged(&amount_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_shell();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_general_finish_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_shell();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_general_finish_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_shell();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_general_finish_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_shell();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_general_finish_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_general_finish_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn topology_bound_push_pull_uses_the_selected_planar_face_and_rejects_tamper() {
    let points = [[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]];
    let mut app = planar_push_pull::tests::prism(&points, Transform::identity());
    select_initial_topological(&mut app, TopologicalElementKind::Face, 2);
    assert_eq!(
        app.selected_reference().unwrap().element,
        ElementId::TopologicalFace(2)
    );
    let source_digest = app.canonical_digest();
    let source_revision = app.document_revision();
    app.set_push_pull_distance_input("5");
    assert!(app.start_preview());
    assert_eq!(app.document_revision(), source_revision);
    assert_eq!(app.canonical_digest(), source_digest);
    let preview = app.tool_preview.get::<EphemeralBoxPreview>().unwrap();
    assert!(matches!(
        preview.batch.commands(),
        [CanonicalCommand::CreateFeature {
            kind: FeatureKind::FaceOffset { .. },
            ..
        }]
    ));
    let topology = preview.plan.source.topological_selection.as_ref().unwrap();
    assert_eq!(
        preview.plan.source.topological_reference.as_ref(),
        Some(&topology.target().reference)
    );
    let manual_batch = preview.batch.clone();
    let source = preview.plan.source.clone();
    let (_, assistant_batch, assistant_proposal) = app
        .derive_push_pull_preview_plan(&source, ProposalPrincipal::LocalAssistant, "5", 5.0)
        .unwrap();
    assert_eq!(assistant_batch, manual_batch);
    assert_eq!(
        assistant_proposal.principal(),
        ProposalPrincipal::LocalAssistant
    );
    planar_push_pull::tests::wait_preview(&mut app);
    let preview_bounds = app
        .face_offset_preview_package(planar_push_pull::tests::TEST_PRISM_DEFINITION)
        .unwrap()
        .bounds_mm();
    assert!((preview_bounds[1][1] - 65.0).abs() < 1.0e-5);
    assert!(app.confirm_preview());
    assert!((app.active_boxes().into_iter().next().unwrap().size_mm.y - 65.0).abs() < 1.0e-5);
    let committed_digest = app.canonical_digest();
    let reopened = ketchup_model::persistence::load(&ketchup_model::persistence::save(
        &app.document.current(),
    ))
    .unwrap()
    .snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), source_digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), committed_digest);

    let mut minimum_face = planar_push_pull::tests::prism(&points, Transform::identity());
    select_initial_topological(&mut minimum_face, TopologicalElementKind::Face, 3);
    assert_eq!(
        minimum_face.selected_reference().unwrap().element,
        ElementId::TopologicalFace(3)
    );
    minimum_face.set_push_pull_distance_input("5");
    assert!(minimum_face.start_preview());
    planar_push_pull::tests::wait_preview(&mut minimum_face);
    let bounds = minimum_face
        .face_offset_preview_package(planar_push_pull::tests::TEST_PRISM_DEFINITION)
        .unwrap()
        .bounds_mm();
    assert!((bounds[0][0] + 5.0).abs() < 1.0e-5);
    assert!((bounds[1][0] - 100.0).abs() < 1.0e-5);

    let revision = minimum_face.document_revision();
    let digest = minimum_face.canonical_digest();
    let undo_steps = minimum_face.undo_step_count();
    minimum_face
        .tool_preview
        .get_mut::<EphemeralBoxPreview>()
        .unwrap()
        .plan
        .source
        .topological_reference
        .as_mut()
        .unwrap()
        .producer_element_id
        .push_str("-tampered");
    assert!(!minimum_face.confirm_preview());
    assert_eq!(minimum_face.document_revision(), revision);
    assert_eq!(minimum_face.canonical_digest(), digest);
    assert_eq!(minimum_face.undo_step_count(), undo_steps);
}

#[test]
fn push_pull_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_push_pull() -> KetchupApp {
        let mut app = KetchupApp::new();
        select_initial_top_face(&mut app);
        app.set_push_pull_distance_input("5");
        assert!(app.start_preview());
        assert!(app.has_preview());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_push_pull();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper
        .tool_preview
        .get_mut::<EphemeralBoxPreview>()
        .unwrap()
        .batch = CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    }]);
    assert!(!batch_tamper.confirm_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut command_tamper = prepared_push_pull();
    let revision = command_tamper.document_revision();
    let digest = command_tamper.canonical_digest();
    let undo_steps = command_tamper.undo_step_count();
    command_tamper
        .tool_preview
        .get_mut::<EphemeralBoxPreview>()
        .unwrap()
        .plan
        .commands
        .clear();
    assert!(!command_tamper.confirm_preview());
    assert_unchanged(&command_tamper, revision, &digest, undo_steps);

    let mut geometry_tamper = prepared_push_pull();
    let revision = geometry_tamper.document_revision();
    let digest = geometry_tamper.canonical_digest();
    let undo_steps = geometry_tamper.undo_step_count();
    geometry_tamper
        .tool_preview
        .get_mut::<EphemeralBoxPreview>()
        .unwrap()
        .plan
        .preview_box
        .size_mm
        .z += 1.0;
    assert!(!geometry_tamper.confirm_preview());
    assert_unchanged(&geometry_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_push_pull();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_push_pull();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut input_drift = prepared_push_pull();
    let revision = input_drift.document_revision();
    let digest = input_drift.canonical_digest();
    let undo_steps = input_drift.undo_step_count();
    input_drift.push_pull.distance_input = "6".to_owned();
    assert!(!input_drift.confirm_preview());
    assert_unchanged(&input_drift, revision, &digest, undo_steps);

    let mut stale = prepared_push_pull();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_push_pull();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}
