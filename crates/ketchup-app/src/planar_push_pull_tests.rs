use super::*;

pub(crate) const TEST_PRISM_DEFINITION: DefinitionId = DefinitionId(17);

pub(crate) fn prism(points: &[[f64; 2]], transform: Transform) -> KetchupApp {
    let mut app = KetchupApp::new();
    app.new_document();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: TEST_PRISM_DEFINITION,
                name: "Polygon".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: TEST_PRISM_DEFINITION,
                name: "Line loop".into(),
                kind: FeatureKind::SegmentProfile {
                    closed: true,
                    segments: (0..points.len())
                        .map(|i| ProfileSegment::Line {
                            start_mm: points[i],
                            end_mm: points[(i + 1) % points.len()],
                        })
                        .collect(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: TEST_PRISM_DEFINITION,
                name: "Extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("12", 12.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: TEST_PRISM_DEFINITION,
                name: "Prism".into(),
                transform,
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let worker = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker alongside the app tests");
    app.headless_force_exact_worker_path(&worker);
    let snapshot = app.document.current();
    let task = ketchup_application::evaluation::start_exact_evaluation(
        snapshot.clone(),
        &app.container_data,
        &app.exact_results,
        &app.topology_results,
        Some(worker),
        || {},
    );
    let products = task.wait(Duration::from_secs(30)).unwrap();
    let report = publish_exact_products(
        &mut app.document,
        &mut app.exact_results,
        &mut app.topology_results,
        &task,
        products,
    )
    .unwrap();
    assert!(report.complete && report.topology_complete, "{report:?}");
    app.exact_source = Some(ketchup_application::evaluation::exact_source(&snapshot));
    app
}

fn package(app: &KetchupApp) -> Arc<ExactBodyPackage> {
    app.topology_results
        .get_render(&app.document.current(), TEST_PRISM_DEFINITION)
        .unwrap()
        .clone()
}

fn volume(app: &KetchupApp) -> f64 {
    let package = package(app);
    let ExactBodyPackage::Graph(graph) = package.as_ref() else {
        panic!("Graph required")
    };
    assert_eq!(graph.topology_counts[3..], [1, 1]);
    assert!(graph.volume_mm3 > 0.0);
    graph.volume_mm3
}

pub(crate) fn wait_preview(app: &mut KetchupApp) {
    if app.face_offset_evaluation.is_none() {
        app.begin_face_offset_evaluation();
    }
    let context = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(30);
    while app
        .face_offset_evaluation
        .as_ref()
        .is_some_and(|evaluation| evaluation.task.is_some())
    {
        assert!(Instant::now() < deadline, "preview timeout");
        app.poll_face_offset_evaluation(&context);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.face_offset_evaluation
            .as_ref()
            .is_some_and(|evaluation| evaluation.ready),
        "{}",
        app.digest
    );
}

fn select(app: &mut KetchupApp, ordinal: u32) {
    let producer_feature_id = package(app).producer_feature_id();
    assert!(app.select_topological_locator(TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id,
        kind: TopologicalElementKind::Face,
        ordinal
    }));
    assert_eq!(
        app.selection.primary.as_ref().unwrap().element,
        ElementId::TopologicalFace(ordinal)
    );
}

#[test]
fn continuous_motion_paints_surface_preview_without_waiting_for_exact_worker() {
    let mut app = prism(
        &[[0.0, 0.0], [40.0, 0.0], [8.0, 30.0]],
        Transform::identity(),
    );
    select(&mut app, 1);
    let digest = app.canonical_digest();
    let steps = app.undo_step_count();
    let context = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
    for distance in [1, 2, 5, -1, -3, 4] {
        app.set_push_pull_distance_input(distance.to_string());
        assert!(app.start_preview());
        let output = context.run(egui::RawInput::default(), |context| {
            let painter = context.layer_painter(egui::LayerId::background());
            app.paint_face_offset_guide(&painter, rect);
        });
        assert!(
            output.shapes.iter().any(|shape| matches!(
                &shape.shape,
                egui::Shape::Path(path) if path.closed && path.fill != Color32::TRANSPARENT
            )),
            "every pointer update must paint a filled surface, not only an arrow"
        );
        assert!(app.face_offset_evaluation.is_none());
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), steps);
    }
    app.cancel_preview();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_face_offset_guide(&context.layer_painter(egui::LayerId::background()), rect);
    });
    assert!(output.shapes.is_empty());
}

#[test]
fn continuous_motion_defers_exact_worker_until_idle_or_confirmation() {
    let mut app = prism(
        &[[0.0, 0.0], [40.0, 0.0], [8.0, 30.0]],
        Transform::identity(),
    );
    select(&mut app, 0);
    let undo_steps = app.undo_step_count();

    for step in 1..=120 {
        app.set_push_pull_distance_input((f64::from(step) / 10.0).to_string());
        assert!(app.start_preview());
        assert!(
            app.face_offset_evaluation.is_none(),
            "live pointer motion must not spawn overlapping exact workers"
        );
    }

    assert!(!app.confirm_preview());
    assert!(
        app.face_offset_evaluation
            .as_ref()
            .is_some_and(|evaluation| {
                evaluation.task.is_some() && evaluation.confirm_requested
            })
    );
    let revision = app.document_revision();
    let context = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.document_revision() == revision {
        assert!(Instant::now() < deadline, "confirmation timed out");
        app.poll_face_offset_evaluation(&context);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.undo_step_count(), undo_steps + 1);
}

#[test]
fn every_polygon_face_supports_signed_offset_and_repeated_edit_with_exact_undo() {
    let polygons = [
        vec![[0.0, 0.0], [40.0, 0.0], [8.0, 30.0]],
        vec![
            [0.0, 0.0],
            [40.0, 0.0],
            [35.0, 20.0],
            [20.0, 30.0],
            [0.0, 20.0],
        ],
        vec![
            [0.0, 0.0],
            [40.0, 0.0],
            [40.0, 15.0],
            [15.0, 15.0],
            [15.0, 35.0],
            [0.0, 35.0],
        ],
    ];
    let transform = Transform::from_matrix([
        0.8, -0.6, 0.0, 100.0, 0.6, 0.8, 0.0, -30.0, 0.0, 0.0, 1.0, 5.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    for points in polygons {
        for ordinal in 0..points.len() as u32 + 2 {
            for distance in [2.0, -2.0] {
                let mut app = prism(&points, transform);
                let original_digest = app.canonical_digest();
                let original_volume = volume(&app);
                let original_steps = app.undo_step_count();
                select(&mut app, ordinal);
                let selected = app.selection.primary.clone().unwrap();
                let face = app.selected_planar_face(&selected).unwrap();
                assert!((vector_length(face.normal) - 1.0).abs() < 1.0e-9);
                let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
                let source_package = package(&app);
                let reference = source_package
                    .topological_reference(TopologicalElementKind::Face, ordinal)
                    .unwrap();
                let triangle = source_package
                    .triangles()
                    .iter()
                    .enumerate()
                    .find(|(index, _)| {
                        source_package.topological_reference_for_triangle(*index) == Some(reference)
                    })
                    .unwrap()
                    .1;
                let center = triangle.vertex_indices.into_iter().fold(
                    Vec3::new(0.0, 0.0, 0.0),
                    |sum, index| {
                        let [x, y, z] = source_package.vertices()[index as usize].position_mm;
                        sum + transform_model_point(transform, Vec3::new(x, y, z))
                    },
                ) * (1.0 / 3.0);
                let pointer = app.project(center, rect);
                let (direction, scale) = app
                    .push_pull_screen_projection(&selected, pointer, rect)
                    .unwrap();
                let expected = app.project(center + face.normal, rect) - app.project(center, rect);
                if expected.length() > 1.0e-4 {
                    assert!(
                        direction.dot(expected.normalized()) > 0.999_999,
                        "face {ordinal}: physical={:?}, resolved={:?}, normal={:?}",
                        direction * scale,
                        expected,
                        face.normal
                    );
                }
                app.set_push_pull_distance_input(distance.to_string());
                assert!(app.start_preview(), "{ordinal}, {distance}: {}", app.digest);
                assert_eq!(app.canonical_digest(), original_digest);
                let drag_mesh = app
                    .face_offset_drag_mesh()
                    .expect("immediate surface preview");
                let source_triangles = source_package
                    .triangles()
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        source_package.topological_reference_for_triangle(*index) == Some(reference)
                    })
                    .map(|(_, triangle)| triangle)
                    .collect::<Vec<_>>();
                assert_eq!(drag_mesh.caps.len(), source_triangles.len());
                assert_eq!(
                    drag_mesh.walls.len(),
                    if face.normal.z.abs() > 0.99 {
                        points.len()
                    } else {
                        4
                    },
                    "no walls on internal triangulation edges"
                );
                for (cap, triangle) in drag_mesh.caps.iter().zip(&source_triangles) {
                    for (moved, index) in cap.iter().zip(triangle.vertex_indices) {
                        let [x, y, z] = source_package.vertices()[index as usize].position_mm;
                        let before = transform_model_point(transform, Vec3::new(x, y, z));
                        let after = transform_model_point(transform, *moved);
                        assert!(
                            vector_length(after - before - face.normal * distance) < 1.0e-8,
                            "live cap must follow the selected face normal and signed world distance"
                        );
                    }
                }
                wait_preview(&mut app);
                assert!(
                    app.face_offset_drag_mesh().is_none(),
                    "exact preview replaces the live sweep"
                );
                let exact_preview = app
                    .face_offset_preview_package(TEST_PRISM_DEFINITION)
                    .unwrap();
                let ExactBodyPackage::Graph(preview_graph) = exact_preview.as_ref() else {
                    panic!()
                };
                assert_eq!(preview_graph.topology_counts[3..], [1, 1]);
                assert!(app.confirm_preview());
                let committed = app
                    .exact_results
                    .get_render(&app.document.current(), TEST_PRISM_DEFINITION)
                    .unwrap()
                    .clone();
                assert_eq!(exact_preview.vertices(), committed.vertices());
                assert_eq!(exact_preview.triangles(), committed.triangles());
                let result_volume = volume(&app);
                assert!(
                    (result_volume - original_volume) * distance > 0.0,
                    "{ordinal}, {distance}: {original_volume} -> {result_volume}"
                );
                assert!((result_volume - preview_graph.volume_mm3).abs() < 1.0e-8);
                assert_eq!(app.undo_step_count(), original_steps + 1);
                let committed = app.canonical_digest();
                assert!(app.undo());
                assert_eq!(app.canonical_digest(), original_digest);
                assert!(app.redo());
                assert_eq!(app.canonical_digest(), committed);
                app.rebind_exact_results(&app.document.current());
                let next = package(&app)
                    .topological_references()
                    .iter()
                    .filter(|reference| reference.kind == TopologicalElementKind::Face)
                    .enumerate()
                    .find_map(|(index, reference)| {
                        planar_face(&package(&app), reference, transform)
                            .filter(|next| dot(next.normal, face.normal) > 0.999)
                            .map(|_| index as u32)
                    })
                    .unwrap();
                select(&mut app, next);
                app.set_push_pull_distance_input("1");
                assert!(app.start_preview());
                wait_preview(&mut app);
                assert!(app.confirm_preview());
                assert!(volume(&app) > result_volume);
                let saved = ketchup_core::persistence::save_document_store(
                    &app.document,
                    &app.container_data,
                )
                .unwrap();
                let reopened = ketchup_core::persistence::load(&saved)
                    .unwrap()
                    .into_editable()
                    .ok()
                    .unwrap();
                assert_eq!(
                    reopened.current().canonical_digest(),
                    app.canonical_digest()
                );
            }
        }
    }
}

#[test]
fn physical_pick_keeps_distinct_slanted_faces_and_binds_push_pull() {
    let mut app = prism(
        &[
            [0.0, 0.0],
            [50.0, 0.0],
            [45.0, 10.0],
            [35.0, 20.0],
            [0.0, 20.0],
        ],
        Transform::identity(),
    );
    let package = package(&app);
    let ExactBodyPackage::Graph(graph) = package.as_ref() else {
        panic!()
    };
    let faces = package
        .topological_references()
        .iter()
        .filter(|reference| reference.kind == TopologicalElementKind::Face)
        .enumerate()
        .map(|(ordinal, reference)| {
            let mut face = planar_face(&package, reference, Transform::identity()).unwrap();
            let vertices = graph
                .triangles
                .iter()
                .zip(&graph.triangle_face_ordinals)
                .filter(|(_, index)| **index == ordinal as u32)
                .flat_map(|(triangle, _)| triangle.vertex_indices)
                .map(|index| {
                    let p = graph.vertices[index as usize].position_mm;
                    Vec3::new(p[0], p[1], p[2])
                })
                .collect::<Vec<_>>();
            face.origin = vertices
                .iter()
                .copied()
                .fold(Vec3::new(0.0, 0.0, 0.0), |sum, p| sum + p)
                * (1.0 / vertices.len() as f64);
            (ordinal as u32, face)
        })
        .collect::<Vec<_>>();
    let mut by_axis = BTreeMap::<ElementId, Vec<u32>>::new();
    for (ordinal, face) in &faces {
        by_axis
            .entry(face_element_from_normal(face.normal))
            .or_default()
            .push(*ordinal);
    }
    assert!(by_axis.values().any(|faces| faces.len() >= 2));
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
    for (ordinal, face) in faces.iter().filter(|(_, face)| face.normal.z.abs() < 0.1) {
        let pointer = app.project(face.origin, rect);
        let result = app.pick_result_at_screen(pointer, rect, 0.0).unwrap();
        let hit = result
            .overlapping
            .iter()
            .find(|hit| hit.reference.element == ElementId::TopologicalFace(*ordinal))
            .expect("physical overlap retains selected face");
        app.select_push_pull_target(hit.reference.clone(), pointer, rect);
        assert_eq!(app.selection.topological.len(), 1);
        assert_eq!(app.selection.primary.as_ref(), Some(&hit.reference));
        assert!(app.selected_planar_face(&hit.reference).is_some());
    }
}

#[test]
fn viewport_direct_drag_commits_slanted_face_without_initial_box() {
    let mut app = prism(
        &[[0.0, 0.0], [40.0, 0.0], [8.0, 30.0]],
        Transform::identity(),
    );
    app.yaw = 0.8;
    app.pitch = 0.9;
    app.dispatch_command(AppCommand::PushPull);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.step();
    let point = Vec3::new(24.0, 15.0, 6.0);
    let pointer = harness.state().viewport_position(point).unwrap();
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(pointer));
    harness.step();
    assert!(matches!(
        harness.state().hovered.as_ref().unwrap().element,
        ElementId::TopologicalFace(_)
    ));
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: pointer,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let selected = harness.state().selection.primary.clone().unwrap();
    let face = harness.state().selected_planar_face(&selected).unwrap();
    assert!(
        face.normal.x.abs() > 0.1 && face.normal.y.abs() > 0.1,
        "{face:?}"
    );
    let rect = harness.state().viewport_rect().unwrap();
    let ray = harness.state().view_ray(pointer, rect).unwrap();
    let snapshot = harness.state().document.current();
    let topology = harness
        .state()
        .topology_results_for_snapshot(&snapshot)
        .unwrap();
    let hit = ExactInteractionProjection::from_snapshot(&snapshot, topology)
        .exact_surface_picks(ray)
        .into_iter()
        .find(|hit| {
            harness.state().exact_hit_element(hit).as_ref() == Some(&selected.element)
                && hit.instance_path == selected.instance_path
        })
        .unwrap();
    let expected_screen_normal = harness
        .state()
        .project(hit.position_mm + hit.outward_normal, rect)
        - harness.state().project(hit.position_mm, rect);
    let expected_screen_normal = expected_screen_normal.normalized();
    let drag = harness.state().push_pull_drag.as_ref().unwrap();
    assert!(drag.screen_normal.dot(expected_screen_normal) > 0.999_999);
    let before_package = package(harness.state());
    let before_support = before_package
        .vertices()
        .iter()
        .map(|vertex| {
            let [x, y, z] = vertex.position_mm;
            dot(Vec3::new(x, y, z), hit.outward_normal)
        })
        .fold(f64::NEG_INFINITY, f64::max);
    let drag_target = harness
        .state()
        .viewport_position(point + face.normal * 8.0)
        .unwrap();
    let before = harness.state().document_revision();
    let steps = harness.state().undo_step_count();
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(drag_target));
    harness.step();
    assert!(harness.state().preview_action_digest().is_some());
    for distance in [4.0, -2.0, 8.0] {
        let position = harness
            .state()
            .viewport_position(point + face.normal * distance)
            .unwrap();
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(position));
        harness.step();
        let mesh = harness
            .state()
            .face_offset_drag_mesh()
            .expect("visible mesh while dragging");
        assert!((mesh.distance_mm - distance).abs() < 1.0e-6);
        for vertex in mesh.caps.iter().flatten() {
            assert!((dot(*vertex - face.origin, face.normal) - distance).abs() < 1.0e-6);
        }
        let projected = mesh.caps[0]
            .map(|p| harness.state().project(p, rect))
            .to_vec();
        assert!(harness.output().shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Path(path) if path.points == projected && path.fill != Color32::TRANSPARENT
        )), "viewport output must contain the current displaced face, not just the helper data");
    }
    let state = harness.state();
    let snapshot = state.document.current();
    let projection = ExactInteractionProjection::from_snapshot(&snapshot, &state.topology_results);
    assert!(
        state.viewport_boxes(&snapshot, &projection).is_empty(),
        "side-face offset must not draw the original profile extruded along Z"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let visible_preview = loop {
        if let Some(preview) = harness
            .state()
            .face_offset_preview_package(TEST_PRISM_DEFINITION)
        {
            break preview;
        }
        assert!(
            Instant::now() < deadline,
            "idle drag did not produce exact preview"
        );
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    };
    assert_eq!(harness.state().document_revision(), before);
    assert_eq!(harness.state().undo_step_count(), steps);
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: drag_target,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let deadline = Instant::now() + Duration::from_secs(30);
    while harness.state().document_revision() == before {
        assert!(Instant::now() < deadline, "{}", harness.state().digest);
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    assert_eq!(harness.state().document_revision(), before + 1);
    assert_eq!(harness.state().undo_step_count(), steps + 1);
    assert!(volume(harness.state()) > 7200.0);
    let committed = harness
        .state()
        .exact_results
        .get_render(&harness.state().document.current(), TEST_PRISM_DEFINITION)
        .unwrap()
        .clone();
    assert_eq!(
        visible_preview.result_fingerprint(),
        committed.result_fingerprint()
    );
    assert_eq!(visible_preview.vertices(), committed.vertices());
    assert_eq!(visible_preview.triangles(), committed.triangles());
    let after_support = package(harness.state())
        .vertices()
        .iter()
        .map(|vertex| {
            let [x, y, z] = vertex.position_mm;
            dot(Vec3::new(x, y, z), hit.outward_normal)
        })
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (after_support - before_support - 8.0).abs() < 1.0e-6,
        "selected face moved by {} mm along {:?}",
        after_support - before_support,
        hit.outward_normal
    );
    assert!(harness.state().face_offset_evaluation.is_none());
    assert!(harness.state().push_pull_anchor.is_none());
}

#[test]
fn idle_preview_coalesces_in_flight_changes_and_confirms_latest_distance() {
    let mut app = prism(
        &[[0.0, 0.0], [40.0, 0.0], [8.0, 30.0]],
        Transform::identity(),
    );
    select(&mut app, 0);
    let context = egui::Context::default();
    let revision = app.document_revision();
    let steps = app.undo_step_count();
    let target = app.selection.primary.clone().unwrap();
    let face = app.selected_planar_face(&target).unwrap();
    app.set_push_pull_distance_input("2");
    assert!(app.start_preview());
    app.face_offset_preview_due = Some(Instant::now());
    app.poll_face_offset_evaluation(&context);
    let first_source = app.face_offset_evaluation.as_ref().unwrap().source.clone();
    assert!(app.face_offset_evaluation.as_ref().unwrap().task.is_some());
    for distance in [3, 5, -1, 4] {
        app.set_push_pull_distance_input(distance.to_string());
        assert!(app.start_preview());
        app.begin_face_offset_evaluation();
        assert_eq!(
            app.face_offset_evaluation.as_ref().unwrap().source,
            first_source,
            "pointer motion must not replace an in-flight worker"
        );
        assert!(
            app.face_offset_preview_package(TEST_PRISM_DEFINITION)
                .is_none()
        );
    }
    assert!(!app.confirm_preview());
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.document_revision() == revision {
        assert!(Instant::now() < deadline, "{}", app.digest);
        app.poll_face_offset_evaluation(&context);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.undo_step_count(), steps + 1);
    let support = package(&app)
        .vertices()
        .iter()
        .map(|v| {
            let [x, y, z] = v.position_mm;
            dot(Vec3::new(x, y, z) - face.origin, face.normal)
        })
        .fold(f64::NEG_INFINITY, f64::max);
    assert!((support - 4.0).abs() < 1.0e-6, "latest offset: {support}");
}

#[test]
fn ready_preview_is_hidden_after_direction_change_and_cancel_is_non_mutating() {
    let mut app = prism(
        &[[0.0, 0.0], [40.0, 0.0], [8.0, 30.0]],
        Transform::identity(),
    );
    select(&mut app, 0);
    let digest = app.canonical_digest();
    let steps = app.undo_step_count();
    app.set_push_pull_distance_input("2");
    assert!(app.start_preview());
    wait_preview(&mut app);
    let first = app
        .face_offset_preview_package(TEST_PRISM_DEFINITION)
        .unwrap();
    app.set_push_pull_distance_input("-2");
    assert!(app.start_preview());
    assert!(
        app.face_offset_preview_package(TEST_PRISM_DEFINITION)
            .is_none()
    );
    app.face_offset_preview_due = Some(Instant::now());
    app.poll_face_offset_evaluation(&egui::Context::default());
    wait_preview(&mut app);
    let second = app
        .face_offset_preview_package(TEST_PRISM_DEFINITION)
        .unwrap();
    assert_ne!(first.result_fingerprint(), second.result_fingerprint());
    app.cancel_preview();
    assert!(app.face_offset_preview_due.is_none());
    assert!(app.face_offset_evaluation.is_none());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), steps);
}

#[test]
fn failed_cancelled_or_stale_face_offset_never_changes_document() {
    let mut app = prism(
        &[[0.0, 0.0], [40.0, 0.0], [8.0, 30.0]],
        Transform::identity(),
    );
    select(&mut app, 0);
    let digest = app.canonical_digest();
    let steps = app.undo_step_count();
    app.set_push_pull_distance_input("2");
    assert!(app.start_preview());
    wait_preview(&mut app);
    app.preview_box
        .as_mut()
        .unwrap()
        .plan
        .source
        .topological_reference
        .as_mut()
        .unwrap()
        .producer_element_id
        .push_str("-tampered");
    assert!(!app.confirm_preview());
    assert_eq!(app.canonical_digest(), digest);
    app.cancel_preview();
    select(&mut app, 0);
    app.set_push_pull_distance_input("2");
    assert!(app.start_preview());
    app.cancel_preview();
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), steps);
    select(&mut app, 0);
    app.headless_force_exact_worker_path("missing-exact-worker.exe");
    app.set_push_pull_distance_input("2");
    assert!(app.start_preview());
    assert!(!app.confirm_preview());
    let context = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.face_offset_evaluation.as_ref().unwrap().task.is_some() {
        assert!(Instant::now() < deadline);
        app.poll_face_offset_evaluation(&context);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.face_offset_evaluation.as_ref().unwrap().failed);
    assert!(!app.confirm_preview());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), steps);
}
