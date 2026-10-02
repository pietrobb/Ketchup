//! Camera, picking, selection windows and how the viewport paints the model.

use super::*;

// Palette contrast is proved once for all four appearances in
// `theme::tests::every_palette_keeps_text_and_accent_legible`, so this file
// no longer keeps a second copy of the thresholds for one hardcoded set.
#[test]
fn switching_theme_repaints_the_shell_without_touching_the_document() {
    let mut app = KetchupApp::new();
    let before_revision = app.document_revision();
    let before_digest = app.canonical_digest();
    let graphite = app.palette();

    for kind in ThemeKind::ALL {
        app.set_theme(kind);
        assert_eq!(app.theme(), kind);
        assert_eq!(app.palette(), Palette::of(kind));
        assert_eq!(
            app.document_revision(),
            before_revision,
            "changing appearance must not commit a canonical batch"
        );
        assert_eq!(
            app.canonical_digest(),
            before_digest,
            "changing appearance must not change the model"
        );
    }

    app.set_theme(ThemeKind::Graphite);
    assert_eq!(app.palette(), graphite);
    assert!(
        !app.can_undo(),
        "appearance must never enter the undo stack"
    );
}

#[test]
fn hovering_and_selecting_a_canonical_mesh_body_paints_its_outline() {
    let source = jagged_sphere_binary_stl(6, 8);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("small-sphere.stl");
    std::fs::write(&path, &source).unwrap();
    let mut app = KetchupApp::new();
    app.document = DocumentStore::new();
    let snapshot = app.document.current();
    let pending = PendingStlImport {
        plan: app
            .prepare_stl_import_preview_plan(ImportSourcePlan::seal(
                ImportFormat::Stl,
                path,
                source.clone(),
                ImportLengthUnit::Millimetre,
                &snapshot,
            ))
            .unwrap(),
        review_error: None,
        invalidated: false,
    };
    assert!(app.import_stl_from(&pending), "{}", app.action_digest());
    let snapshot = app.document.current();
    let occurrence = snapshot.occurrences().next().unwrap().id();
    assert!(
        definition_mesh_body(&snapshot, snapshot.definitions().next().unwrap().id()).is_some(),
        "the imported sphere is stored as a canonical mesh body"
    );
    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    app.zoom_fit();

    let unselected = selection_stroke_segments(&context, &mut app);
    app.selection.select_occurrence(occurrence, false);
    let selected = selection_stroke_segments(&context, &mut app);

    assert_eq!(unselected, 0);
    assert!(
        selected > 0,
        "a selected canonical mesh body must paint its selection outline"
    );
}

#[test]
fn orbit_passes_both_poles_without_a_pitch_limit() {
    let mut app = KetchupApp::new();

    app.orbit(Vec2::new(0.0, 400.0));
    assert!(app.camera.pitch > 1.2);

    app.orbit(Vec2::new(0.0, -800.0));
    assert!(app.camera.pitch < -1.2);
}

#[test]
fn zoom_selection_preserves_camera_basis_projection_and_document() {
    let mut app = KetchupApp::new();
    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    assert!(!app.command_enabled(AppCommand::ZoomSelection));
    app.selection.select_occurrence(OccurrenceId(1), false);
    assert!(app.command_enabled(AppCommand::ZoomSelection));

    let basis = app.camera_basis();
    let projection = app.projection_mode();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    app.dispatch_command(AppCommand::ZoomSelection);

    assert_eq!(app.camera_basis(), basis);
    assert_eq!(app.projection_mode(), projection);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(
        app.action_digest(),
        app.catalog.format(
            "digest-zoom-selection",
            &BTreeMap::from([("count", "1".to_owned())]),
        )
    );
}

#[test]
fn zoom_steps_clamp_without_changing_camera_basis_projection_or_document() {
    let mut app = KetchupApp::new();
    let basis = app.camera_basis();
    let projection = app.projection_mode();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    app.camera.zoom = MAX_CAMERA_ZOOM / 1.1;
    app.dispatch_command(AppCommand::ZoomIn);
    assert_eq!(app.camera_zoom(), MAX_CAMERA_ZOOM);
    assert!(!app.command_enabled(AppCommand::ZoomIn));

    app.camera.zoom = MIN_CAMERA_ZOOM * 1.1;
    app.dispatch_command(AppCommand::ZoomOut);
    assert_eq!(app.camera_zoom(), MIN_CAMERA_ZOOM);
    assert!(!app.command_enabled(AppCommand::ZoomOut));

    assert_eq!(app.camera_basis(), basis);
    assert_eq!(app.projection_mode(), projection);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
}

#[test]
fn standard_orthographic_views_match_the_drawing_axes_without_document_mutation() {
    fn assert_basis(actual: (Vec3, Vec3, Vec3), expected: (Vec3, Vec3, Vec3)) {
        for (actual, expected) in [
            (actual.0, expected.0),
            (actual.1, expected.1),
            (actual.2, expected.2),
        ] {
            assert!((actual.x - expected.x).abs() < 1.0e-6);
            assert!((actual.y - expected.y).abs() < 1.0e-6);
            assert!((actual.z - expected.z).abs() < 1.0e-6);
        }
    }

    let mut app = KetchupApp::new();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    for (command, key, basis) in [
        (
            AppCommand::ViewTop,
            "view-top",
            (
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, -1.0),
            ),
        ),
        (
            AppCommand::ViewBottom,
            "view-bottom",
            (
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
        ),
        (
            AppCommand::ViewFront,
            "view-front",
            (
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, 1.0, 0.0),
            ),
        ),
        (
            AppCommand::ViewBack,
            "view-back",
            (
                Vec3::new(-1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, -1.0, 0.0),
            ),
        ),
        (
            AppCommand::ViewRight,
            "view-right",
            (
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(-1.0, 0.0, 0.0),
            ),
        ),
        (
            AppCommand::ViewLeft,
            "view-left",
            (
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 0.0),
            ),
        ),
    ] {
        app.dispatch_command(command);
        assert_basis(app.camera_basis(), basis);
        assert_eq!(
            app.action_digest(),
            app.catalog.format(
                "digest-view-changed",
                &BTreeMap::from([("view", app.catalog.text(key))]),
            )
        );
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    }
}

#[test]
fn camera_clearance_keeps_every_mesh_bound_in_front_during_orbit() {
    let mut app = KetchupApp::new();
    let definition_id = DefinitionId(100);
    let feature_id = FeatureId(100);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: "Large mesh".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: feature_id,
                definition_id,
                name: "Large mesh body".to_owned(),
                kind: FeatureKind::MeshBody(MeshBodySpec {
                    schema: MESH_BODY_SCHEMA_V1.to_owned(),
                    vertices_mm: vec![
                        [-1_000.0, -800.0, -600.0],
                        [1_200.0, -800.0, -600.0],
                        [-1_000.0, 1_400.0, -600.0],
                        [-1_000.0, -800.0, 1_600.0],
                    ],
                    triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
                    authority: MeshAuthority::Authored {
                        provenance: "camera-clearance-regression".to_owned(),
                    },
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(100),
                definition_id,
                name: "Large mesh occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    app.camera.projection_mode = ProjectionMode::Perspective;
    app.camera.zoom = MAX_CAMERA_ZOOM;

    for yaw in [-2.8_f32, -1.4, 0.0, 1.4, 2.8] {
        app.camera.yaw = yaw;
        app.camera.pitch = 0.45;
        app.refresh_camera_distance();
        let (_, _, forward) = app.camera_basis();
        let target = app.camera_target();
        let minimum = Vec3::new(-1_000.0, -800.0, -600.0);
        let size = Vec3::new(2_200.0, 2_200.0, 2_200.0);
        for corner in box_corners(size.x, size.y, size.z) {
            let depth = app.camera_distance() + dot(minimum + corner - target, forward);
            assert!(depth > PERSPECTIVE_NEAR_MM, "yaw {yaw}: depth {depth}");
        }
    }
}

#[test]
fn gpu_projection_matches_cpu_projection_inside_callback_viewport() {
    let mut app = KetchupApp::new();
    let rect = Rect::from_min_size(Pos2::new(87.0, 163.0), Vec2::new(1_927.0, 1_184.0));

    for mode in [ProjectionMode::Perspective, ProjectionMode::Parallel] {
        app.camera.projection_mode = mode;
        let matrix = app.world_to_clip(rect);
        for point in box_corners(BOX_WIDTH_MM, BOX_DEPTH_MM, app.document_height_mm()) {
            let clip_x = matrix[0] * point.x as f32
                + matrix[4] * point.y as f32
                + matrix[8] * point.z as f32
                + matrix[12];
            let clip_y = matrix[1] * point.x as f32
                + matrix[5] * point.y as f32
                + matrix[9] * point.z as f32
                + matrix[13];
            // The rasterizer divides by clip w before it maps to the
            // viewport, so the check has to divide as well or it would
            // only ever be valid for the parallel projection.
            let clip_w = matrix[3] * point.x as f32
                + matrix[7] * point.y as f32
                + matrix[11] * point.z as f32
                + matrix[15];
            let gpu_screen = Pos2::new(
                rect.center().x + (clip_x / clip_w) * rect.width() * 0.5,
                rect.center().y - (clip_y / clip_w) * rect.height() * 0.5,
            );
            let cpu_screen = app.project(point, rect);
            assert!((gpu_screen - cpu_screen).length() < 0.01, "{mode:?}");
        }
    }
}

#[test]
fn viewport_omits_edge_on_faces_that_collapse_to_a_line() {
    let mut app = KetchupApp::new();
    // Only a parallel projection collapses an edge-on face to a line; a
    // converging one always leaves a sliver of area.
    app.camera.projection_mode = ProjectionMode::Parallel;
    app.camera.yaw = std::f32::consts::FRAC_PI_2;
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let projected = box_corners(BOX_WIDTH_MM, BOX_DEPTH_MM, app.document_height_mm())
        .map(|point| app.project(point, rect));
    let forward = Vec3::new(
        -f64::from(app.camera.yaw.sin() * app.camera.pitch.sin()),
        -f64::from(app.camera.yaw.cos() * app.camera.pitch.sin()),
        -f64::from(app.camera.pitch.cos()),
    );

    assert_eq!(
        box_faces()
            .into_iter()
            .filter(|face| {
                face_is_visible(&face.element, forward)
                    && projected_face_has_area(face.corners, &projected)
            })
            .count(),
        2
    );
}

#[test]
fn viewport_draws_only_the_three_camera_facing_box_faces() {
    let app = KetchupApp::new();
    let forward = Vec3::new(
        -f64::from(app.camera.yaw.sin() * app.camera.pitch.sin()),
        -f64::from(app.camera.yaw.cos() * app.camera.pitch.sin()),
        -f64::from(app.camera.pitch.cos()),
    );

    assert_eq!(
        box_faces()
            .into_iter()
            .filter(|face| face_is_visible(&face.element, forward))
            .count(),
        3
    );
}

#[test]
fn viewport_click_routes_through_exact_spatial_query() {
    let app = KetchupApp::new();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let selected = app.exact_pick_at_screen(rect.center(), rect).unwrap();
    assert_eq!(selected.definition_id, INITIAL_BOX_DEFINITION);
    assert_eq!(selected.instance_path, InstancePath::root(OccurrenceId(1)));
    assert_eq!(
        selected.element,
        ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        }
    );
}

#[test]
fn picking_chooses_the_frontmost_body_across_mesh_and_box_geometry() {
    let mut app = KetchupApp::new();
    assert!(apply_reviewed_model_intent(
        &mut app,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Grooved behind".to_owned(),
                    size_mm: [100.0, 60.0, 20.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: vec![ketchup_assistant::sidecar::AssistantSubtractionIntent {
                        size_mm: [10.0, 60.0, 5.0],
                        origin_mm: [45.0, 0.0, 15.0],
                    }],
                },
                AssistantBoxIntent {
                    name: "Plain in front".to_owned(),
                    size_mm: [100.0, 60.0, 20.0],
                    origin_mm: [0.0, 0.0, 40.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
        }
    ));
    app.camera.projection_mode = ProjectionMode::Parallel;
    app.camera.yaw = 0.0;
    app.camera.pitch = 0.0;
    app.camera.target_z = 30.0;
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let pointer = app.project(Vec3::new(50.0, 30.0, 60.0), rect);

    let selected = app.exact_pick_at_screen(pointer, rect).unwrap();
    assert_eq!(selected.instance_path, InstancePath::root(OccurrenceId(3)));
}

#[test]
fn repeated_large_scene_picks_reuse_revision_bound_spatial_indices() {
    let mut app = KetchupApp::new();
    let source = app
        .document
        .current()
        .occurrence(OccurrenceId(1))
        .unwrap()
        .clone();
    let commands = (2_u32..=480)
        .map(|id| CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(u64::from(id)),
            definition_id: source.definition_id(),
            name: format!("Stacked {id}"),
            transform: Transform::from_translation(
                f64::from((id - 1) % 24) * 120.0,
                0.0,
                f64::from((id - 1) / 24) * 280.0,
            )
            .unwrap(),
            parent: None,
            tag: None,
            visible: true,
        })
        .collect();
    app.document
        .apply_batch(&CommandBatch::new(commands))
        .unwrap();
    app.zoom_fit();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
    let pointer = app.project(Vec3::new(50.0, 30.0, 20.0), rect);

    assert!(app.exact_pick_at_screen(pointer, rect).is_some());
    let cache_ptrs = app.interaction_projection_cache_ptrs().unwrap();
    for _ in 0..480 {
        assert!(app.exact_pick_at_screen(pointer, rect).is_some());
        assert_eq!(app.interaction_projection_cache_ptrs(), Some(cache_ptrs));
    }
}

#[test]
fn parallel_view_picks_a_hundred_metre_body_after_zoom_fit() {
    let mut app = KetchupApp::new();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
    app.camera.viewport_rect = Some(rect);
    assert!(app.create_box_at(Vec3::new(0.0, 200.0, 0.0), Vec3::new(100_000.0, 60.0, 20.0),));
    app.zoom_fit();
    app.refresh_camera_distance();
    let visible_top = app.project(Vec3::new(75_000.0, 230.0, 20.0), rect);

    let selected = app
        .exact_pick_at_screen(visible_top, rect)
        .expect("a visible point on a 100 m body must remain pickable");

    assert_eq!(selected.definition_id, DefinitionId(2));
    assert_eq!(selected.instance_path, InstancePath::root(OccurrenceId(2)));
}

#[test]
fn viewport_picks_the_geometry_currently_shown_in_preview() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    app.set_push_pull_distance_input("40");
    assert!(app.start_preview());
    let top_center = app.project(Vec3::new(50.0, 30.0, 60.0), rect);

    let selected = app.exact_pick_at_screen(top_center, rect).unwrap();

    assert_eq!(
        selected.element,
        ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        }
    );
}

#[test]
fn directional_selection_window_contains_left_to_right_and_crosses_right_to_left() {
    let mut app = KetchupApp::new();
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
    assert!(app.copy_selected(Vec3::new(240.0, 0.0, 0.0)));
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
    app.camera.viewport_rect = Some(viewport);
    app.home_view();
    let projected_box = |app: &KetchupApp, occurrence_id| {
        let (origin, size) = app.occurrence_box_geometry(occurrence_id).unwrap();
        box_corners(size.x, size.y, size.z)
            .into_iter()
            .map(|corner| {
                let point = app.project(origin + corner, viewport);
                Rect::from_min_max(point, point)
            })
            .reduce(|left, right| left.union(right))
            .unwrap()
    };
    let first = projected_box(&app, 1);
    let second = projected_box(&app, 2);
    assert!(!first.intersects(second));

    let contained = app.selection_window_paths(
        first.left_top() - Vec2::splat(2.0),
        first.right_bottom() + Vec2::splat(2.0),
        viewport,
    );
    assert_eq!(
        contained,
        BTreeSet::from([InstancePath::root(OccurrenceId(1))])
    );

    let partial_left_to_right = app.selection_window_paths(
        Pos2::new(first.center().x, first.top() - 2.0),
        first.right_bottom() + Vec2::splat(2.0),
        viewport,
    );
    assert!(partial_left_to_right.is_empty());

    let crossing_start = second.right_top() + Vec2::splat(2.0);
    let crossing_end = Pos2::new(second.center().x, second.bottom() + 2.0);
    let crossing_right_to_left = app.selection_window_paths(crossing_start, crossing_end, viewport);
    assert_eq!(
        crossing_right_to_left,
        BTreeSet::from([InstancePath::root(OccurrenceId(2))]),
        "second={second:?}, start={crossing_start:?}, end={crossing_end:?}, all={:?}",
        app.selection_window_paths(viewport.right_top(), viewport.left_bottom(), viewport)
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    app.selection.clear();
    app.complete_selection_window(
        SelectionWindowDrag {
            start: first.left_top() - Vec2::splat(2.0),
            cursor: first.right_bottom() + Vec2::splat(2.0),
            additive: false,
        },
        viewport,
    );
    app.complete_selection_window(
        SelectionWindowDrag {
            start: second.left_top() - Vec2::splat(2.0),
            cursor: second.right_bottom() + Vec2::splat(2.0),
            additive: true,
        },
        viewport,
    );
    assert_eq!(
        app.selection.occurrences,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );

    app.gesture.drag.open(SelectionWindowDrag {
        start: viewport.left_top(),
        cursor: viewport.center(),
        additive: false,
    });
    app.cancel_preview();
    assert!(app.gesture.drag.get::<SelectionWindowDrag>().is_none());
    assert_eq!(app.selection.occurrences.len(), 2);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
}

#[test]
fn crossing_selection_uses_projected_geometry_instead_of_its_empty_bounds() {
    let mut app = KetchupApp::new();
    let definition_id = DefinitionId(100);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: "Triangular mesh".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(100),
                definition_id,
                name: "Triangular mesh body".to_owned(),
                kind: FeatureKind::MeshBody(MeshBodySpec {
                    schema: MESH_BODY_SCHEMA_V1.to_owned(),
                    vertices_mm: vec![
                        [0.0, 0.0, 0.0],
                        [100.0, 0.0, 0.0],
                        [0.0, 100.0, 0.0],
                        [0.0, 0.0, 100.0],
                    ],
                    triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
                    authority: MeshAuthority::Authored {
                        provenance: "crossing-selection-regression".to_owned(),
                    },
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(100),
                definition_id,
                name: "Triangular mesh occurrence".to_owned(),
                transform: Transform::from_translation(300.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
    app.camera.viewport_rect = Some(viewport);
    app.camera.projection_mode = ProjectionMode::Parallel;
    app.dispatch_command(AppCommand::ViewTop);
    app.home_view();

    let empty_corner = Rect::from_two_pos(
        app.project(Vec3::new(375.0, 75.0, 0.0), viewport),
        app.project(Vec3::new(395.0, 95.0, 0.0), viewport),
    );
    let empty_crossing = app.selection_window_paths(
        empty_corner.right_top(),
        empty_corner.left_bottom(),
        viewport,
    );
    assert!(
        !empty_crossing.contains(&InstancePath::root(OccurrenceId(100))),
        "the window lies inside the mesh bounds but does not touch its triangle"
    );

    let touched = Rect::from_two_pos(
        app.project(Vec3::new(345.0, 45.0, 0.0), viewport),
        app.project(Vec3::new(355.0, 55.0, 0.0), viewport),
    );
    let touched_crossing =
        app.selection_window_paths(touched.right_top(), touched.left_bottom(), viewport);
    assert!(touched_crossing.contains(&InstancePath::root(OccurrenceId(100))));
}

#[test]
fn adaptive_grid_keeps_metric_lines_readable_across_camera_scales() {
    assert_eq!(adaptive_grid_step(8.0), 10.0);
    assert_eq!(adaptive_grid_step(0.01), 5_000.0);
    assert_eq!(adaptive_grid_step(0.000_01), 5_000_000.0);
    for scale in [8.0, 1.0, 0.01, 0.000_01] {
        let screen_spacing = adaptive_grid_step(scale) * scale;
        assert!(screen_spacing >= 32.0);
        assert!(screen_spacing <= 80.0);
    }
}

#[test]
fn perspective_ground_axes_share_the_projected_world_origin() {
    let mut app = KetchupApp::new();
    app.camera.projection_mode = ProjectionMode::Perspective;
    app.camera.yaw = -0.65;
    app.camera.pitch = -0.5;
    app.camera.zoom = 2.8;
    app.camera.pan = Vec2::ZERO;
    app.camera.distance_mm = 150.0;
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let origin = app.project(Vec3::ZERO, rect);
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_ground_plane(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("perspective-ground-axes"),
            )),
            rect,
        );
    });

    for axis in [Axis::X, Axis::Y, Axis::Z] {
        let points = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, stroke } if stroke.color == axis_color(axis) => {
                    Some(*points)
                }
                _ => None,
            })
            .expect("each world axis must be painted");
        let direction = points[1] - points[0];
        let amount = ((origin - points[0]).dot(direction) / direction.length_sq()).clamp(0.0, 1.0);
        let distance = (origin - points[0].lerp(points[1], amount)).length();
        assert!(
            distance <= 0.25,
            "{axis:?} axis misses the projected world origin by {distance} px"
        );
    }
}

#[test]
fn gpu_scene_is_painted_after_the_ground_grid() {
    let mut app = KetchupApp::new();
    let snapshot = app.document.current();
    let plan = Arc::new(InstancedRenderPlan::from_snapshot(
        &snapshot,
        &app.exact.results,
        &mut app.render.cache,
    ));
    let context = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("scene-base-layer-order"),
        ));
        app.paint_scene_base_layers(&painter, rect, Some(Arc::clone(&plan)));
    });

    assert!(output.shapes.len() > 1);
    assert!(matches!(
        output.shapes.last().map(|shape| &shape.shape),
        Some(egui::Shape::Callback(_))
    ));
}

#[test]
fn shadows_are_painted_under_the_gpu_scene_without_grid_dependency() {
    let mut app = KetchupApp::new();
    app.view.set(ViewFlag::GridAxes, false);
    app.toggle_view(ViewFlag::Shadows);
    let boxes = app.active_boxes();
    let snapshot = app.document.current();
    let plan = Arc::new(InstancedRenderPlan::from_snapshot(
        &snapshot,
        &app.exact.results,
        &mut app.render.cache,
    ));
    let context = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("shadow-base-layer-order"),
        ));
        app.paint_projected_shadows(&painter, rect, &boxes);
        app.paint_scene_base_layers(&painter, rect, Some(Arc::clone(&plan)));
    });

    assert_eq!(output.shapes.len(), boxes.len() + 1);
    assert!(matches!(
        output.shapes.last().map(|shape| &shape.shape),
        Some(egui::Shape::Callback(_))
    ));
}

#[test]
fn fog_is_painted_over_the_gpu_scene_as_a_depth_gradient() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Fog);
    let snapshot = app.document.current();
    let plan = Arc::new(InstancedRenderPlan::from_snapshot(
        &snapshot,
        &app.exact.results,
        &mut app.render.cache,
    ));
    let context = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("fog-overlay-order"),
        ));
        app.paint_scene_base_layers(&painter, rect, Some(Arc::clone(&plan)));
        app.paint_viewport_fog(&painter, rect);
    });

    let callback_index = output
        .shapes
        .iter()
        .position(|shape| matches!(shape.shape, egui::Shape::Callback(_)))
        .expect("the GPU scene callback must be present");
    assert_eq!(callback_index + 1, output.shapes.len() - 1);
    let Some(egui::Shape::Mesh(haze)) = output.shapes.last().map(|shape| &shape.shape) else {
        panic!("fog must finish with one gradient mesh");
    };
    assert_eq!(haze.vertices.len(), 4);
    assert_eq!(haze.vertices[0].color.a(), 112);
    assert_eq!(haze.vertices[3].color.a(), 8);
}

#[test]
fn xray_projected_faces_use_translucent_fill() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Xray);
    let faces = [ProjectedFace {
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        polygon: ProjectedPolygon::Triangle([
            Pos2::new(10.0, 10.0),
            Pos2::new(110.0, 10.0),
            Pos2::new(10.0, 110.0),
        ]),
        color: Color32::GRAY,
        depth: 0.0,
        previewed: false,
        out_of_context: false,
    }];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("xray-projected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });

    assert_eq!(output.shapes.len(), 1, "Xray must not double-blend a face");
    let egui::Shape::Mesh(fill) = &output.shapes[0].shape else {
        panic!("x-ray projected faces must have one fill mesh");
    };
    assert!(fill.vertices.iter().all(|vertex| vertex.color.a() == 72));
}

#[test]
fn wireframe_projected_faces_emit_no_fill_shapes() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Wireframe);
    let faces = [ProjectedFace {
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        polygon: ProjectedPolygon::Triangle([
            Pos2::new(10.0, 10.0),
            Pos2::new(110.0, 10.0),
            Pos2::new(10.0, 110.0),
        ]),
        color: Color32::GRAY,
        depth: 0.0,
        previewed: false,
        out_of_context: false,
    }];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("wireframe-projected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });

    assert!(output.shapes.is_empty());
}

#[test]
fn hidden_edges_emit_no_edge_shapes_but_keep_shaded_face_fills() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Edges);
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let faces = [ProjectedFace {
        selection: selection.clone(),
        polygon: ProjectedPolygon::Triangle([
            Pos2::new(10.0, 10.0),
            Pos2::new(110.0, 10.0),
            Pos2::new(10.0, 110.0),
        ]),
        color: Color32::from_rgb(80, 140, 200),
        depth: 0.0,
        previewed: false,
        out_of_context: false,
    }];
    let edges = [ProjectedEdge {
        selection,
        points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
        depth: 0.0,
        dominant_axis: None,
    }];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("hidden-edge-shaded-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
        app.paint_projected_edges(&painter, &edges);
    });

    assert_eq!(output.shapes.len(), 1);
    assert!(matches!(output.shapes[0].shape, egui::Shape::Mesh(_)));
}

#[test]
fn profiles_use_a_wider_edge_stroke_without_changing_edge_count() {
    let mut app = KetchupApp::new();
    let edges = [ProjectedEdge {
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Edge(0),
        },
        points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
        depth: 0.0,
        dominant_axis: None,
    }];
    let context = egui::Context::default();
    let normal = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("normal-profile-stroke"),
            )),
            &edges,
        );
    });
    app.toggle_view(ViewFlag::Profiles);
    let emphasized = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("emphasized-profile-stroke"),
            )),
            &edges,
        );
    });

    assert_eq!(normal.shapes.len(), 1);
    assert_eq!(emphasized.shapes.len(), 1);
    let egui::Shape::LineSegment { stroke: normal, .. } = normal.shapes[0].shape else {
        panic!("ordinary profile must be one line segment");
    };
    let egui::Shape::LineSegment {
        stroke: emphasized, ..
    } = emphasized.shapes[0].shape
    else {
        panic!("emphasized profile must be one line segment");
    };
    assert_eq!(normal.width, 1.25);
    assert_eq!(emphasized.width, 2.75);
}

#[test]
fn halos_underpaint_each_edge_with_fixed_screen_space_width_and_compose_with_dashes() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Halos);
    let edge = ProjectedEdge {
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Edge(0),
        },
        points: [Pos2::new(10.0, 10.0), Pos2::new(30.0, 10.0)],
        depth: 0.0,
        dominant_axis: Some(Axis::X),
    };
    let context = egui::Context::default();
    let solid = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("solid-edge-halo"),
            )),
            std::slice::from_ref(&edge),
        );
    });

    assert_eq!(solid.shapes.len(), 2);
    assert!(matches!(
        solid.shapes[0].shape,
        egui::Shape::LineSegment { points, stroke }
            if points == edge.points
                && stroke.width == 4.25
                && stroke.color == Color32::from_rgb(24, 28, 35)
    ));
    assert!(matches!(
        solid.shapes[1].shape,
        egui::Shape::LineSegment { points, stroke }
            if points == edge.points
                && stroke.width == 1.25
                && stroke.color == Color32::from_rgb(182, 192, 207)
    ));

    app.toggle_view(ViewFlag::Dashes);
    app.toggle_view(ViewFlag::ColorByAxis);
    let dashed = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("dashed-edge-halo"),
            )),
            std::slice::from_ref(&edge),
        );
    });
    assert_eq!(dashed.shapes.len(), 6);
    assert!(dashed.shapes[..3].iter().all(|shape| matches!(
        shape.shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 4.25
                && stroke.color == Color32::from_rgb(24, 28, 35)
    )));
    assert!(dashed.shapes[3..].iter().all(|shape| matches!(
        shape.shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 1.25 && stroke.color == axis_color(Axis::X)
    )));
}

#[test]
fn depth_cue_weights_near_edges_more_than_far_edges_without_reordering() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::DepthCue);
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let near_points = [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)];
    let far_points = [Pos2::new(10.0, 30.0), Pos2::new(110.0, 30.0)];
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: near_points,
            depth: 0.0,
            dominant_axis: None,
        },
        ProjectedEdge {
            selection,
            points: far_points,
            depth: 10.0,
            dominant_axis: None,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("depth-cue-strokes"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 2);
    let egui::Shape::LineSegment {
        points: painted_near,
        stroke: near_stroke,
    } = output.shapes[0].shape
    else {
        panic!("near edge must remain the first line segment");
    };
    let egui::Shape::LineSegment {
        points: painted_far,
        stroke: far_stroke,
    } = output.shapes[1].shape
    else {
        panic!("far edge must remain the second line segment");
    };
    assert_eq!(painted_near, near_points);
    assert_eq!(painted_far, far_points);
    assert!(near_stroke.width > far_stroke.width);
    assert!((near_stroke.width - 1.6).abs() < f32::EPSILON * 2.0);
    assert!((far_stroke.width - 0.9).abs() < f32::EPSILON * 2.0);
}

#[test]
fn distant_edge_fade_blends_far_axis_color_into_current_background() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::FadeDistantEdges);
    app.toggle_view(ViewFlag::ColorByAxis);
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
            depth: 0.0,
            dominant_axis: Some(Axis::X),
        },
        ProjectedEdge {
            selection,
            points: [Pos2::new(10.0, 30.0), Pos2::new(110.0, 30.0)],
            depth: 10.0,
            dominant_axis: Some(Axis::X),
        },
    ];
    let context = egui::Context::default();
    let paint = |app: &KetchupApp, id: &'static str| {
        context.run(egui::RawInput::default(), |context| {
            app.paint_projected_edges(
                &context.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new(id))),
                &edges,
            );
        })
    };

    let dark = paint(&app, "dark-distant-edge-fade");
    assert_eq!(dark.shapes.len(), 2);
    assert!(matches!(
        dark.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == axis_color(Axis::X)
    ));
    assert!(matches!(
        dark.shapes[1].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.color == Color32::from_rgb(94, 48, 44)
    ));

    app.toggle_view(ViewFlag::WhiteBackground);
    let white = paint(&app, "white-distant-edge-fade");
    assert_eq!(white.shapes.len(), 2);
    assert!(matches!(
        white.shapes[1].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.color == Color32::from_rgb(237, 190, 184)
    ));
}

#[test]
fn high_contrast_edges_override_axis_color_and_fade_into_each_background() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::HighContrastEdges);
    app.toggle_view(ViewFlag::ColorByAxis);
    app.toggle_view(ViewFlag::FadeDistantEdges);
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
            depth: 0.0,
            dominant_axis: Some(Axis::X),
        },
        ProjectedEdge {
            selection,
            points: [Pos2::new(10.0, 30.0), Pos2::new(110.0, 30.0)],
            depth: 10.0,
            dominant_axis: Some(Axis::X),
        },
    ];
    let context = egui::Context::default();
    let paint = |app: &KetchupApp, id: &'static str| {
        context.run(egui::RawInput::default(), |context| {
            app.paint_projected_edges(
                &context.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new(id))),
                &edges,
            );
        })
    };

    let dark = paint(&app, "dark-high-contrast-edges");
    assert_eq!(dark.shapes.len(), 2);
    assert!(matches!(
        dark.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == Color32::WHITE
    ));
    assert!(matches!(
        dark.shapes[1].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.color == Color32::from_rgb(104, 107, 112)
    ));

    app.toggle_view(ViewFlag::WhiteBackground);
    let white = paint(&app, "white-high-contrast-edges");
    assert_eq!(white.shapes.len(), 2);
    assert!(matches!(
        white.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == Color32::BLACK
    ));
    assert!(matches!(
        white.shapes[1].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.color == Color32::from_rgb(158, 160, 162)
    ));
}

#[test]
fn selection_halo_underpaints_selected_edges_with_background_contrast() {
    let mut app = KetchupApp::new();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    app.selection.select_exact(selection.clone(), false);
    let edge = ProjectedEdge {
        selection,
        points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
        depth: 0.0,
        dominant_axis: None,
    };
    let context = egui::Context::default();
    let paint = |app: &KetchupApp, id: &'static str| {
        context.run(egui::RawInput::default(), |context| {
            app.paint_projected_selection(
                &context.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new(id))),
                std::slice::from_ref(&edge),
            );
        })
    };

    let ordinary = paint(&app, "ordinary-selection");
    assert_eq!(ordinary.shapes.len(), 1);
    assert!(matches!(
        ordinary.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 1.8 && stroke.color == Color32::from_rgb(240, 78, 35)
    ));

    app.toggle_view(ViewFlag::SelectionHalo);
    let dark = paint(&app, "dark-selection-halo");
    assert_eq!(dark.shapes.len(), 2);
    assert!(matches!(
        dark.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 4.8 && stroke.color == Color32::WHITE
    ));
    assert!(matches!(
        dark.shapes[1].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 1.8 && stroke.color == Color32::from_rgb(240, 78, 35)
    ));

    app.toggle_view(ViewFlag::WhiteBackground);
    let white = paint(&app, "white-selection-halo");
    assert_eq!(white.shapes.len(), 2);
    assert!(matches!(
        white.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 4.8 && stroke.color == Color32::BLACK
    ));
}

#[test]
fn endpoints_follow_all_edge_strokes_with_exactly_two_markers_per_edge() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Endpoints);
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let first_points = [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)];
    let second_points = [Pos2::new(10.0, 30.0), Pos2::new(110.0, 30.0)];
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: first_points,
            depth: 0.0,
            dominant_axis: None,
        },
        ProjectedEdge {
            selection,
            points: second_points,
            depth: 10.0,
            dominant_axis: None,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("endpoint-markers"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 6);
    assert!(matches!(
        output.shapes[0].shape,
        egui::Shape::LineSegment { points, .. } if points == first_points
    ));
    assert!(matches!(
        output.shapes[1].shape,
        egui::Shape::LineSegment { points, .. } if points == second_points
    ));
    for (shape, expected) in output.shapes[2..].iter().zip([
        first_points[0],
        first_points[1],
        second_points[0],
        second_points[1],
    ]) {
        let egui::Shape::Circle(marker) = &shape.shape else {
            panic!("endpoint marker must follow every edge stroke");
        };
        assert_eq!(marker.center, expected);
        assert_eq!(marker.radius, 3.25);
        assert_eq!(marker.fill, Color32::from_rgb(232, 158, 72));
    }
}

#[test]
fn midpoints_follow_jittered_edges_once_and_compose_with_endpoints_and_dashes() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Midpoints);
    app.toggle_view(ViewFlag::Jitter);
    app.toggle_view(ViewFlag::Endpoints);
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
            depth: 0.0,
            dominant_axis: Some(Axis::X),
        },
        ProjectedEdge {
            selection,
            points: [Pos2::new(10.0, 30.0), Pos2::new(110.0, 30.0)],
            depth: 10.0,
            dominant_axis: Some(Axis::X),
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("edge-midpoints"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 8);
    for (shape, expected) in output.shapes[2..4]
        .iter()
        .zip([Pos2::new(58.5, 9.25), Pos2::new(61.5, 29.25)])
    {
        let egui::Shape::Circle(marker) = &shape.shape else {
            panic!("each edge must receive exactly one midpoint marker");
        };
        assert_eq!(marker.center, expected);
        assert_eq!(marker.radius, 3.25);
        assert_eq!(marker.fill, Color32::from_rgb(96, 201, 138));
    }
    assert!(
        output.shapes[4..]
            .iter()
            .all(|shape| matches!(shape.shape, egui::Shape::Circle(_)))
    );

    app.toggle_view(ViewFlag::Jitter);
    app.toggle_view(ViewFlag::Endpoints);
    app.toggle_view(ViewFlag::Dashes);
    app.toggle_view(ViewFlag::ColorByAxis);
    let dashed = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("dashed-edge-midpoint"),
            )),
            &[ProjectedEdge {
                selection: edges[0].selection.clone(),
                points: [Pos2::new(10.0, 10.0), Pos2::new(30.0, 10.0)],
                depth: 0.0,
                dominant_axis: Some(Axis::X),
            }],
        );
    });
    assert_eq!(dashed.shapes.len(), 4);
    assert!(dashed.shapes[..3].iter().all(|shape| matches!(
        shape.shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == axis_color(Axis::X)
    )));
    assert!(matches!(
        dashed.shapes[3].shape,
        egui::Shape::Circle(ref marker)
            if marker.center == Pos2::new(20.0, 10.0)
                && marker.fill == Color32::from_rgb(96, 201, 138)
    ));
}

#[test]
fn extensions_follow_all_edge_strokes_with_fixed_screen_space_overhangs() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Extensions);
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let horizontal = [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)];
    let vertical = [Pos2::new(30.0, 20.0), Pos2::new(30.0, 120.0)];
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: horizontal,
            depth: 0.0,
            dominant_axis: None,
        },
        ProjectedEdge {
            selection,
            points: vertical,
            depth: 10.0,
            dominant_axis: None,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("edge-extensions"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 6);
    assert!(matches!(
        output.shapes[0].shape,
        egui::Shape::LineSegment { points, .. } if points == horizontal
    ));
    assert!(matches!(
        output.shapes[1].shape,
        egui::Shape::LineSegment { points, .. } if points == vertical
    ));
    for (shape, expected) in output.shapes[2..].iter().zip([
        [Pos2::new(3.0, 10.0), Pos2::new(10.0, 10.0)],
        [Pos2::new(110.0, 10.0), Pos2::new(117.0, 10.0)],
        [Pos2::new(30.0, 13.0), Pos2::new(30.0, 20.0)],
        [Pos2::new(30.0, 120.0), Pos2::new(30.0, 127.0)],
    ]) {
        assert!(matches!(
            shape.shape,
            egui::Shape::LineSegment { points, .. } if points == expected
        ));
    }
}

#[test]
fn jitter_offsets_edge_strokes_deterministically_without_reordering() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Jitter);
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
            depth: 0.0,
            dominant_axis: None,
        },
        ProjectedEdge {
            selection,
            points: [Pos2::new(30.0, 20.0), Pos2::new(30.0, 120.0)],
            depth: 10.0,
            dominant_axis: None,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("edge-jitter"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 2);
    assert!(matches!(
        output.shapes[0].shape,
        egui::Shape::LineSegment { points, .. }
            if points == [Pos2::new(8.5, 9.25), Pos2::new(108.5, 9.25)]
    ));
    assert!(matches!(
        output.shapes[1].shape,
        egui::Shape::LineSegment { points, .. }
            if points == [Pos2::new(31.5, 19.25), Pos2::new(31.5, 119.25)]
    ));
}

#[test]
fn dashes_split_each_edge_with_fixed_screen_space_rhythm_without_reordering() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Dashes);
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: [Pos2::new(10.0, 10.0), Pos2::new(30.0, 10.0)],
            depth: 0.0,
            dominant_axis: None,
        },
        ProjectedEdge {
            selection,
            points: [Pos2::new(40.0, 20.0), Pos2::new(40.0, 40.0)],
            depth: 10.0,
            dominant_axis: None,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("edge-dashes"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 6);
    for (shape, expected) in output.shapes.iter().zip([
        [Pos2::new(10.0, 10.0), Pos2::new(15.0, 10.0)],
        [Pos2::new(19.0, 10.0), Pos2::new(24.0, 10.0)],
        [Pos2::new(28.0, 10.0), Pos2::new(30.0, 10.0)],
        [Pos2::new(40.0, 20.0), Pos2::new(40.0, 25.0)],
        [Pos2::new(40.0, 29.0), Pos2::new(40.0, 34.0)],
        [Pos2::new(40.0, 38.0), Pos2::new(40.0, 40.0)],
    ]) {
        assert!(matches!(
            shape.shape,
            egui::Shape::LineSegment { points, .. } if points == expected
        ));
    }
}

#[test]
fn color_by_axis_classifies_world_edges_and_colors_strokes_and_extensions() {
    assert_eq!(
        dominant_edge_axis([Vec3::ZERO, Vec3::new(12.0, 3.0, 1.0)]),
        Some(Axis::X)
    );
    assert_eq!(
        dominant_edge_axis([Vec3::ZERO, Vec3::new(2.0, -8.0, 4.0)]),
        Some(Axis::Y)
    );
    assert_eq!(
        dominant_edge_axis([Vec3::ZERO, Vec3::new(1.0, 2.0, -9.0)]),
        Some(Axis::Z)
    );
    assert_eq!(dominant_edge_axis([Vec3::ZERO, Vec3::ZERO]), None);

    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::ColorByAxis);
    app.toggle_view(ViewFlag::Extensions);
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [Axis::X, Axis::Y, Axis::Z].map(|axis| ProjectedEdge {
        selection: selection.clone(),
        points: [Pos2::new(10.0, 10.0), Pos2::new(30.0, 10.0)],
        depth: 0.0,
        dominant_axis: Some(axis),
    });
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("color-by-axis"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 9);
    let colors = output
        .shapes
        .iter()
        .map(|shape| match shape.shape {
            egui::Shape::LineSegment { stroke, .. } => stroke.color,
            _ => panic!("axis-colored edge must remain a line segment"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        colors[..3],
        [
            axis_color(Axis::X),
            axis_color(Axis::Y),
            axis_color(Axis::Z)
        ]
    );
    assert_eq!(
        colors[3..],
        [
            axis_color(Axis::X),
            axis_color(Axis::X),
            axis_color(Axis::Y),
            axis_color(Axis::Y),
            axis_color(Axis::Z),
            axis_color(Axis::Z),
        ]
    );

    app.toggle_view(ViewFlag::Extensions);
    app.toggle_view(ViewFlag::Dashes);
    let dashed = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("color-by-axis-dashes"),
            )),
            &edges[..1],
        );
    });
    assert_eq!(dashed.shapes.len(), 3);
    assert!(dashed.shapes.iter().all(|shape| matches!(
        shape.shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == axis_color(Axis::X)
    )));
}

#[test]
fn monochrome_projected_faces_are_grayscale_but_selection_feedback_stays_colored() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::Monochrome);
    let faces = [ProjectedFace {
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        polygon: ProjectedPolygon::Triangle([
            Pos2::new(10.0, 10.0),
            Pos2::new(110.0, 10.0),
            Pos2::new(10.0, 110.0),
        ]),
        color: Color32::from_rgb(80, 140, 200),
        depth: 0.0,
        previewed: false,
        out_of_context: false,
    }];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("monochrome-projected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });
    let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
        panic!("monochrome projected faces must start with a mesh underlay");
    };
    assert!(underlay.vertices.iter().all(|vertex| {
        vertex.color.r() == vertex.color.g() && vertex.color.g() == vertex.color.b()
    }));

    app.selection.select_occurrence(OccurrenceId(1), false);
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("monochrome-selected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });
    let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
        panic!("selected monochrome faces must start with a mesh underlay");
    };
    assert!(
        underlay
            .vertices
            .iter()
            .all(|vertex| vertex.color == Color32::from_rgb(154, 91, 67))
    );
}

#[test]
fn hidden_line_projected_faces_are_flat_neutral_but_selection_feedback_stays_colored() {
    let mut app = KetchupApp::new();
    app.toggle_view(ViewFlag::HiddenLine);
    let faces = [
        ProjectedFace {
            selection: SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            polygon: ProjectedPolygon::Triangle([
                Pos2::new(10.0, 10.0),
                Pos2::new(110.0, 10.0),
                Pos2::new(10.0, 110.0),
            ]),
            color: Color32::from_rgb(80, 140, 200),
            depth: 0.0,
            previewed: false,
            out_of_context: false,
        },
        ProjectedFace {
            selection: SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(2)),
                element: ElementId::Face {
                    axis: Axis::X,
                    side: Side::Maximum,
                },
            },
            polygon: ProjectedPolygon::Triangle([
                Pos2::new(120.0, 10.0),
                Pos2::new(220.0, 10.0),
                Pos2::new(120.0, 110.0),
            ]),
            color: Color32::from_rgb(200, 90, 40),
            depth: 0.0,
            previewed: false,
            out_of_context: false,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("hidden-line-projected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });
    let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
        panic!("hidden-line projected faces must start with a mesh underlay");
    };
    assert!(
        underlay
            .vertices
            .iter()
            .all(|vertex| vertex.color == Color32::from_rgb(214, 218, 224))
    );

    app.selection.select_occurrence(OccurrenceId(1), false);
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("hidden-line-selected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });
    let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
        panic!("selected hidden-line faces must start with a mesh underlay");
    };
    assert!(
        underlay.vertices[..3]
            .iter()
            .all(|vertex| vertex.color == Color32::from_rgb(154, 91, 67))
    );
    assert!(
        underlay.vertices[3..]
            .iter()
            .all(|vertex| vertex.color == Color32::from_rgb(214, 218, 224))
    );
}

#[test]
fn adjacent_projected_triangles_share_a_fill_underlay_and_keep_antialiased_outlines() {
    let app = KetchupApp::new();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let faces = vec![
        ProjectedFace {
            selection: selection.clone(),
            polygon: ProjectedPolygon::Triangle([
                Pos2::new(10.0, 10.0),
                Pos2::new(110.0, 10.0),
                Pos2::new(10.0, 110.0),
            ]),
            color: Color32::GRAY,
            depth: 0.0,
            previewed: false,
            out_of_context: false,
        },
        ProjectedFace {
            selection,
            polygon: ProjectedPolygon::Triangle([
                Pos2::new(110.0, 10.0),
                Pos2::new(110.0, 110.0),
                Pos2::new(10.0, 110.0),
            ]),
            color: Color32::GRAY,
            depth: 0.0,
            previewed: false,
            out_of_context: false,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("projected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });

    // One unfeathered mesh: feathering sliver triangles one by one draws
    // anti-aliasing spikes far outside faces with holes.
    assert_eq!(output.shapes.len(), 1);
    let egui::Shape::Mesh(fill) = &output.shapes[0].shape else {
        panic!("projected faces must be one shared fill mesh");
    };
    assert_eq!(fill.vertices.len(), 6);
    assert_eq!(fill.indices.len(), 6);
}
