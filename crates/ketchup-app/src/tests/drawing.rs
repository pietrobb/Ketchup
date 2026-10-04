//! Drawing profiles and Push/Pull.

use super::*;

#[test]
fn new_document_starts_empty_without_an_inactive_tool_preview() {
    let mut app = KetchupApp::new();
    app.begin_helix_tool();
    assert!(!app.helix_preview_points().is_empty());

    app.new_document();

    let snapshot = app.document.current();
    assert_eq!(snapshot.definitions().count(), 0);
    assert_eq!(snapshot.occurrences().count(), 0);
    assert_eq!(snapshot.features().count(), 0);
    assert_eq!(app.document.visible_undo_steps(), 0);
    assert!(app.helix_preview_points().is_empty());
    assert_eq!(app.active_tool, ActiveTool::Select);
}

#[test]
fn creating_a_second_box_has_stable_identity_and_undo_redo_visibility() {
    let mut app = KetchupApp::new();
    assert_eq!(app.active_box_count(), 1);

    assert!(app.create_box());
    assert_eq!(app.active_box_count(), 2);
    let created = app.selected_reference().unwrap();
    assert_eq!(created.definition_id, DefinitionId(2));
    assert_eq!(created.instance_path, InstancePath::root(OccurrenceId(2)));
    assert_eq!(app.box_height_mm(created.definition_id), Some(20.0));

    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let second_top = app.project(Vec3::new(125.0, 85.0, 20.0), rect);
    let picked = app.exact_pick_at_screen(second_top, rect).unwrap();
    assert_eq!(picked.definition_id, DefinitionId(2));
    assert_eq!(picked.instance_path, InstancePath::root(OccurrenceId(2)));

    assert!(app.undo());
    assert_eq!(app.active_box_count(), 1);
    assert_eq!(app.selected_reference(), None);

    assert!(app.redo());
    assert_eq!(app.active_box_count(), 2);
}

#[test]
fn push_pull_drag_is_signed_along_the_face_normal() {
    let drag = PushPullDrag {
        source_document_id: DocumentId(1),
        source_revision: 0,
        source_digest: String::new(),
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        pointer_start: Pos2::new(100.0, 100.0),
        extent_start_mm: 20.0,
        screen_normal: Vec2::new(1.0, 0.0),
        pixels_per_mm: 2.0,
    };

    assert_eq!(
        push_pull_distance_from_pointer(&drag, Pos2::new(120.0, 100.0), true),
        10.0
    );
    assert_eq!(
        push_pull_distance_from_pointer(&drag, Pos2::new(80.0, 100.0), true),
        -10.0
    );
    assert_eq!(
        push_pull_distance_from_pointer(&drag, Pos2::new(115.0, 100.0), true),
        7.5
    );
    assert_eq!(
        push_pull_distance_from_pointer(&drag, Pos2::new(119.2, 100.0), true),
        10.0
    );
    assert_eq!(
        push_pull_distance_from_pointer(&drag, Pos2::new(115.0, 100.0), false),
        7.5
    );

    let profile_drag = PushPullDrag {
        extent_start_mm: 0.0,
        ..drag
    };
    assert_eq!(
        push_pull_distance_from_pointer(&profile_drag, Pos2::new(80.0, 100.0), true),
        -10.0
    );
}

#[test]
fn push_pull_distance_accepts_units_and_moves_inward() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    app.set_push_pull_distance_input("-5 mm");

    assert!(app.start_preview());
    assert_eq!(
        app.tool_preview
            .get::<EphemeralBoxPreview>()
            .unwrap()
            .plan
            .preview_box
            .size_mm
            .z,
        15.0
    );
    assert!(app.confirm_preview());
    assert_eq!(app.document_height_mm(), 15.0);
    assert!(app.undo());
    assert_eq!(app.document_height_mm(), 20.0);
}

#[test]
fn push_pull_minimum_side_keeps_the_opposite_face_fixed() {
    let mut app = KetchupApp::new();
    app.selection.primary = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::X,
            side: Side::Minimum,
        },
    });
    let old_maximum = app.active_boxes()[0].origin_mm.x + app.active_boxes()[0].size_mm.x;

    app.set_push_pull_distance_input("30");
    assert!(app.start_preview());
    let preview = app
        .tool_preview
        .get::<EphemeralBoxPreview>()
        .unwrap()
        .plan
        .preview_box
        .clone();
    assert_eq!(preview.origin_mm.x, -30.0);
    assert_eq!(preview.size_mm.x, 130.0);
    assert_eq!(preview.origin_mm.x + preview.size_mm.x, old_maximum);

    assert!(app.confirm_preview());
    assert_eq!(app.active_boxes()[0], preview);
    assert!(app.undo());
    assert_eq!(
        app.active_boxes()[0].origin_mm.x + app.active_boxes()[0].size_mm.x,
        old_maximum
    );
    assert_eq!(app.active_boxes()[0].size_mm.x, 100.0);
}

#[test]
fn push_pull_uses_the_projected_feature_pair_and_preserves_local_profile_origin() {
    let mut app = KetchupApp::new();
    let offset_points = vec![[10.0, 20.0], [110.0, 20.0], [110.0, 80.0], [10.0, 80.0]];
    let unrelated_points = vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0]];
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: FeatureId(1),
                points_mm: offset_points,
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Unrelated profile".to_owned(),
                kind: FeatureKind::polygon(&unrelated_points),
            },
        ]))
        .unwrap();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::X,
            side: Side::Maximum,
        },
    };
    app.selection.primary = Some(selection.clone());
    let item = app.active_boxes()[0].clone();
    assert_eq!(item.profile_feature_id, FeatureId(1));
    assert_eq!(item.extrusion_feature_id, Some(FeatureId(2)));
    assert_eq!(item.origin_mm, Vec3::new(10.0, 20.0, 0.0));
    assert!(
        push_pull_batch(
            &app.document.current(),
            &SelectionId {
                definition_id: DefinitionId(999),
                ..selection
            },
            &item,
            None,
            50.0,
            150.0,
            "150".to_owned(),
        )
        .is_none()
    );

    app.set_push_pull_distance_input("50");
    assert!(app.start_preview());
    assert!(app.confirm_preview());
    let snapshot = app.document.current();
    let Some(points_mm) = snapshot
        .feature(FeatureId(1))
        .unwrap()
        .kind()
        .polygon_points()
    else {
        panic!("linked profile must remain a profile");
    };
    assert_eq!(
        points_mm,
        vec![[10.0, 20.0], [160.0, 20.0], [160.0, 80.0], [10.0, 80.0]]
    );
    let Some(points_mm) = snapshot
        .feature(FeatureId(3))
        .unwrap()
        .kind()
        .polygon_points()
    else {
        panic!("unrelated feature must remain a profile");
    };
    assert_eq!(points_mm, unrelated_points);
}

#[test]
fn push_pull_preview_fails_closed_after_the_source_revision_changes() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    app.set_push_pull_distance_input("5");
    assert!(app.start_preview());
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let current = app.active_boxes()[0].clone();
    assert!(!app.has_preview());
    assert_eq!(app.render_box(current.clone()), current);
    assert!(!app.confirm_preview());
}

#[test]
fn rectangle_drag_preview_preserves_signed_bounds_until_release() {
    let mut app = KetchupApp::new();
    app.new_document();
    assert!(
        app.complete_rectangle_sketch(Vec3::new(-20.0, -15.0, 7.0), Vec3::new(20.0, 15.0, 7.0),)
    );
    let profile = app.active_boxes()[0].clone();
    let digest = app.canonical_digest();
    let steps = app.undo_step_count();
    app.dispatch_command(AppCommand::PushPull);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.step();
    let pointer = harness
        .state()
        .viewport_position(Vec3::new(0.0, 0.0, 7.0))
        .unwrap();
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(pointer));
    harness.step();
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: pointer,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let drag = harness.state().gesture.drag.get::<PushPullDrag>().unwrap();
    let screen_delta = drag.screen_normal * drag.pixels_per_mm;
    let mut release = pointer;
    for distance in [15.0_f64, -20.0, 10.0, -30.0] {
        release = pointer + screen_delta * distance as f32;
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(release));
        harness.step();
        let state = harness.state();
        assert!(state.has_preview());
        assert_eq!(state.canonical_digest(), digest);
        let rendered = state.render_box(profile.clone());
        let mut expected = profile.clone();
        expected.origin_mm.z += distance.min(0.0);
        expected.size_mm.z = distance.abs();
        assert_eq!(rendered.origin_mm, expected.origin_mm);
        assert_eq!(rendered.size_mm, expected.size_mm);
        let rect = state.viewport_rect().unwrap();
        let projected = box_corners(expected.size_mm.x, expected.size_mm.y, expected.size_mm.z)
            .map(|point| state.project(point + expected.origin_mm, rect));
        assert!(
            box_faces()
                .into_iter()
                .filter(|face| matches!(
                    face.element,
                    ElementId::Face {
                        axis: Axis::X | Axis::Y,
                        ..
                    }
                ))
                .any(|face| {
                    let points = face.corners.map(|index| projected[index]).to_vec();
                    crate::viewport_feedback::output_has_fill(
                        &harness.output().shapes,
                        &points,
                        |fill| fill != Color32::TRANSPARENT,
                    )
                }),
            "painted side walls must span the signed interval, not its positive mirror"
        );
    }
    let preview = harness.state().render_box(profile);
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: release,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let committed = harness.state().active_boxes()[0].clone();
    assert_eq!(committed.origin_mm, preview.origin_mm);
    assert_eq!(committed.size_mm, preview.size_mm);
    assert_eq!(harness.state().undo_step_count(), steps + 1);
    assert!(harness.state_mut().undo());
    assert_eq!(harness.state().canonical_digest(), digest);
}

#[test]
fn rectangle_sketch_creates_a_profile_then_push_pull_adds_the_extrusion() {
    let mut app = KetchupApp::new();

    assert!(
        app.complete_rectangle_sketch(Vec3::new(40.0, 25.0, 0.0), Vec3::new(-10.0, -5.0, 0.0),)
    );
    assert_eq!(app.active_box_count(), 2);
    let profile = app.active_boxes()[1].clone();
    assert_eq!(profile.origin_mm, Vec3::new(-10.0, -5.0, 0.0));
    assert_eq!(profile.size_mm, Vec3::new(50.0, 30.0, 0.0));
    assert_eq!(profile.extrusion_feature_id, None);
    let profile_digest = app.canonical_digest();

    app.set_push_pull_distance_input("30");
    assert!(app.start_preview());
    assert!(app.confirm_preview());
    let solid = app.active_boxes()[1].clone();
    assert_eq!(solid.size_mm, Vec3::new(50.0, 30.0, 30.0));
    assert!(solid.extrusion_feature_id.is_some());

    assert!(app.undo());
    assert_eq!(app.canonical_digest(), profile_digest);
    assert_eq!(app.active_boxes()[1].size_mm.z, 0.0);

    app.set_push_pull_distance_input("-30");
    assert!(app.start_preview());
    assert_eq!(
        app.tool_preview
            .get::<EphemeralBoxPreview>()
            .unwrap()
            .plan
            .preview_box
            .origin_mm
            .z,
        -30.0
    );
    let rendered = app.render_box(app.active_boxes()[1].clone());
    assert_eq!(
        rendered.origin_mm.z, -30.0,
        "viewport preview must extend below the profile"
    );
    assert_eq!(rendered.size_mm.z, 30.0);
    assert!(app.confirm_preview());
    assert_eq!(app.active_boxes()[1].origin_mm, rendered.origin_mm);
    assert_eq!(app.active_boxes()[1].size_mm, rendered.size_mm);
    assert_eq!(app.active_boxes()[1].size_mm.z, 30.0);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), profile_digest);
    assert!(app.redo());
    assert_eq!(app.active_boxes()[1].origin_mm.z, -30.0);
}

#[test]
fn push_pull_prioritizes_a_standalone_rectangle_over_an_occluding_solid() {
    let mut app = KetchupApp::new();
    assert!(app.complete_rectangle_sketch(Vec3::new(40.0, 25.0, 0.0), Vec3::new(10.0, 5.0, 0.0),));
    let profile = app
        .active_boxes()
        .into_iter()
        .find(|item| item.extrusion_feature_id.is_none())
        .unwrap();
    select_initial_top_face(&mut app);
    app.dispatch_command(AppCommand::PushPull);

    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1_000.0, 800.0));
    let pointer = app.project(Vec3::new(25.0, 15.0, 0.0), rect);
    app.update_viewport_inference(Some(pointer), rect);

    assert!(
        app.hovered_overlap_choice()
            .is_some_and(|(index, count)| index == 0 && count >= 2)
    );
    assert_eq!(
        app.hovered_selection()
            .map(|selection| &selection.instance_path),
        Some(&profile.instance_path)
    );
    assert_eq!(
        app.push_pull_pointer_target()
            .map(|selection| selection.instance_path),
        Some(profile.instance_path)
    );
}

#[test]
fn a_pocket_in_the_document_keeps_an_editable_depth_as_canonical_undo_steps() {
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
    let original_digest = app.canonical_digest();
    assert!(
        !app.set_selected_pocket_depth(12.0),
        "the box has no pocket yet"
    );
    assert_eq!(app.canonical_digest(), original_digest);

    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Pocket profile".to_owned(),
                kind: FeatureKind::polygon(&[
                    [20.0, 15.0],
                    [50.0, 15.0],
                    [50.0, 35.0],
                    [20.0, 35.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Pocket".to_owned(),
                kind: FeatureKind::pocket(
                    FeatureId(2),
                    FeatureId(3),
                    Dimension::new("8", 8.0).unwrap(),
                ),
            },
        ]))
        .unwrap();
    let pocketed_digest = app.canonical_digest();
    let steps = app.document.visible_undo_steps();

    assert!(app.set_selected_pocket_depth(12.0));
    assert_eq!(app.document.visible_undo_steps(), steps + 1);
    assert!(matches!(
        app.document.current().feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(depth), operation: PadOperation::Cut { .. }, .. }) if depth.millimetres() == 12.0
    ));
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), pocketed_digest);
    assert!(app.redo());
    assert!(matches!(
        app.document.current().feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(depth), operation: PadOperation::Cut { .. }, .. }) if depth.millimetres() == 12.0
    ));
}

#[test]
fn closed_polyline_path_preserves_points_and_rejects_invalid_input_atomically() {
    let mut app = KetchupApp::new();
    let points_mm = vec![
        [-12.5, 4.25],
        [80.0, 4.25],
        [95.5, 40.0],
        [35.0, 72.75],
        [-12.5, 40.0],
    ];
    assert!(app.create_closed_polyline(points_mm.clone()));
    let created = app.active_boxes()[1].clone();
    assert_eq!(created.extrusion_feature_id, None);
    assert_eq!(created.size_mm.z, 0.0);
    assert!(matches!(
        app.document
            .current()
            .feature(created.profile_feature_id)
            .unwrap()
            .kind().polygon_points(),
        Some(ref stored) if stored == &points_mm
    ));

    let digest = app.canonical_digest();
    let revision = app.document_revision();
    // A repeated corner leaves a zero-length side, so the loop is refused unchanged.
    assert!(!app.create_closed_polyline(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 0.0], [0.0, 10.0],]));
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.document_revision(), revision);
    // Drawing direction does not matter: a clockwise loop is as valid as the reverse.
    assert!(app.create_closed_polyline(vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0],]));
}

#[test]
fn drawing_snap_acquires_box_corners_and_edge_midpoints_in_screen_space() {
    let mut app = KetchupApp::new();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let face_center = app.project(Vec3::new(50.0, 30.0, 20.0), rect);

    let corner = Vec3::new(0.0, 0.0, 20.0);
    let corner_screen = app.project(corner, rect);
    let corner_pointer = corner_screen + (corner_screen - face_center).normalized() * 6.0;
    let corner_snap = app
        .scene_snap_at_screen(corner_pointer, rect, 8.0, None)
        .expect("a nearby pointer must acquire the corner");
    assert_eq!(corner_snap.kind, SnapKind::Endpoint);
    assert_eq!(corner_snap.position_mm, corner);
    assert_eq!(
        app.viewport_point_at_screen(corner_pointer, rect, corner.z),
        Some(corner)
    );

    let midpoint = Vec3::new(50.0, 0.0, 20.0);
    let midpoint_screen = app.project(midpoint, rect);
    let midpoint_pointer = midpoint_screen + (midpoint_screen - face_center).normalized() * 6.0;
    let midpoint_snap = app
        .scene_snap_at_screen(midpoint_pointer, rect, 8.0, None)
        .expect("a nearby pointer must acquire the edge midpoint");
    assert_eq!(midpoint_snap.kind, SnapKind::Midpoint);
    assert_eq!(midpoint_snap.position_mm, midpoint);

    let edge_point = Vec3::new(25.0, 0.0, 20.0);
    let edge_pointer = app.project(edge_point, rect);
    let edge_snap = app
        .scene_snap_at_screen(edge_pointer, rect, 8.0, None)
        .expect("a pointer anywhere along the edge must acquire the edge");
    assert_eq!(edge_snap.kind, SnapKind::Edge);
    assert_eq!(edge_snap.position_mm, edge_point);
    assert!(matches!(
        edge_snap.reference.element,
        ElementId::Snap { .. }
    ));

    app.face_workflow.set_snaps_enabled(false);
    assert_ne!(
        app.viewport_point_at_screen(corner_pointer, rect, corner.z),
        Some(corner),
        "turning snaps off must also disable snapping during drawing"
    );

    app.face_workflow.set_snaps_enabled(true);
    select_initial_top_face(&mut app);
    app.set_rotate_axis_lock(Some(Axis::Z));
    assert!(app.rotate_selected(30.0));
    let transform = app.document.current().scene_query()[0].transform;
    let rotated_corner = transform_model_point(transform, corner);
    let rotated_center = transform_model_point(transform, Vec3::new(50.0, 30.0, 20.0));
    let rotated_corner_screen = app.project(rotated_corner, rect);
    let rotated_center_screen = app.project(rotated_center, rect);
    let rotated_pointer =
        rotated_corner_screen + (rotated_corner_screen - rotated_center_screen).normalized() * 6.0;
    let rotated_snap = app
        .scene_snap_at_screen(rotated_pointer, rect, 8.0, None)
        .expect("a rotated canonical corner must remain snappable");
    assert_eq!(rotated_snap.kind, SnapKind::Endpoint);
    assert_eq!(rotated_snap.position_mm, rotated_corner);
}

#[test]
fn push_pull_keeps_the_opposite_face_fixed_on_screen() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let bottom = Vec3::new(0.0, 0.0, 0.0);
    let top = Vec3::new(0.0, 0.0, app.document_height_mm());
    let bottom_before = app.project(bottom, rect);
    let top_before = app.project(top, rect);

    app.set_push_pull_distance_input("20");
    assert!(app.start_preview());

    assert_eq!(app.project(bottom, rect), bottom_before);
    assert_ne!(app.project(Vec3::new(0.0, 0.0, 40.0), rect), top_before);
}

#[test]
fn confirmed_push_pull_can_be_undone_and_redone() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    assert!(!app.can_undo());

    app.set_push_pull_distance_input("22");
    assert!(app.start_preview());
    assert!(app.confirm_preview());
    assert_eq!(app.document_height_mm(), 42.0);

    assert!(app.undo());
    assert_eq!(app.document_height_mm(), 20.0);
    assert!(app.can_redo());

    assert!(app.redo());
    assert_eq!(app.document_height_mm(), 42.0);
}

#[test]
fn typed_push_pull_values_correct_the_last_one_instead_of_stacking() {
    let mut app = KetchupApp::new();
    let base_height = app.document_height_mm();
    app.active_tool = ActiveTool::PushPull;
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

    let base_digest = app.canonical_digest();
    app.value_box.input = "20".to_owned();
    assert!(app.apply_value_input());
    let original_revision = app.document_revision();
    assert_eq!(app.document_height_mm(), base_height + 20.0);
    assert_eq!(app.document.visible_undo_steps(), 1);

    app.value_box.input = "25".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.document_revision(), original_revision + 1);
    assert_eq!(app.document_height_mm(), base_height + 25.0);
    assert_eq!(app.document.visible_undo_steps(), 1);

    app.value_box.input = "0".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.document_revision(), original_revision + 2);
    assert_eq!(app.document_height_mm(), base_height);
    assert_eq!(app.document.visible_undo_steps(), 1);

    assert!(app.undo());
    assert_eq!(app.document_height_mm(), base_height);
    assert_eq!(app.canonical_digest(), base_digest);
    assert!(!app.can_undo(), "corrections must stay one undo step");
}

#[test]
fn rejected_push_pull_correction_preserves_the_last_valid_operation() {
    let mut app = KetchupApp::new();
    app.active_tool = ActiveTool::PushPull;
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
    app.value_box.input = "20".to_owned();
    assert!(app.apply_value_input());
    let valid_height = app.document_height_mm();
    let valid_revision = app.document_revision();
    let valid_digest = app.canonical_digest();
    let valid_undo_steps = app.document.visible_undo_steps();

    app.value_box.input = "-100".to_owned();
    assert!(!app.apply_value_input());
    assert_eq!(app.document_height_mm(), valid_height);
    assert_eq!(app.document_revision(), valid_revision);
    assert_eq!(app.canonical_digest(), valid_digest);
    assert_eq!(app.document.visible_undo_steps(), valid_undo_steps);
    assert_eq!(
        app.push_pull
            .last
            .as_ref()
            .map(|operation| operation.canonical_digest.as_str()),
        Some(valid_digest.as_str())
    );
}

#[test]
fn shared_definition_push_pull_previews_each_occurrence_and_explains_impact() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "Box-1 #2".to_owned(),
                transform: Transform::from_translation(250.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
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
    app.push_pull.distance_input = "60".to_owned();

    assert!(app.start_preview());
    let rendered = app
        .active_boxes()
        .into_iter()
        .map(|item| app.render_box(item))
        .collect::<Vec<_>>();
    assert_eq!(rendered[0].origin_mm, Vec3::ZERO);
    assert_eq!(rendered[1].origin_mm, Vec3::new(250.0, 0.0, 0.0));
    assert_eq!(rendered[0].size_mm.z, 80.0);
    assert_eq!(rendered[1].size_mm.z, 80.0);
    assert!(app.digest.contains("2 occurrence(s) follow"));

    app.cancel_preview();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::X,
                side: Side::Minimum,
            },
        },
        false,
    );
    app.push_pull.distance_input = "30".to_owned();
    assert!(app.start_preview());
    let rendered = app
        .active_boxes()
        .into_iter()
        .map(|item| app.render_box(item))
        .collect::<Vec<_>>();
    assert_eq!(rendered[0].origin_mm.x, -30.0);
    assert_eq!(rendered[1].origin_mm.x, 250.0);
    assert_eq!(rendered[0].size_mm.x, 130.0);
    assert_eq!(rendered[1].size_mm.x, 130.0);
}

#[test]
fn exact_rectangle_and_push_pull_are_atomic_undo_steps() {
    let mut app = KetchupApp::new();
    app.dispatch_command(AppCommand::Rectangle);
    app.gesture.sketch.start = Some(Vec3::new(40.0, 30.0, 20.0));
    app.gesture.sketch.cursor = Some(Vec3::new(20.0, 10.0, 20.0));
    app.value_box.input = "300,200".to_owned();

    assert!(app.apply_value_input());
    assert_eq!(app.active_box_count(), 2);
    let created = app.active_boxes()[1].clone();
    assert_eq!(created.origin_mm, Vec3::new(-260.0, -170.0, 20.0));
    assert_eq!(created.size_mm, Vec3::new(300.0, 200.0, 0.0));
    assert_eq!(app.document.visible_undo_steps(), 1);

    app.dispatch_command(AppCommand::PushPull);
    app.value_box.input = "55".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.active_boxes()[1].size_mm.z, 55.0);
    assert_eq!(app.document.visible_undo_steps(), 2);

    assert!(app.undo());
    assert_eq!(app.active_boxes()[1].size_mm.z, 0.0);
    assert!(app.undo());
    assert_eq!(app.active_box_count(), 1);
    assert!(app.redo());
    assert!(app.redo());
    assert_eq!(app.active_boxes()[1].size_mm.z, 55.0);
}
