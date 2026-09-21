use super::*;
use egui_kittest::Harness;

fn drag(app: &KetchupApp, selection: SelectionId) -> PushPullDrag {
    let snapshot = app.document.current();
    PushPullDrag {
        source_document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        selection,
        pointer_start: Pos2::ZERO,
        extent_start_mm: app.selected_face_extent_mm().unwrap(),
        screen_normal: Vec2::new(0.0, -1.0),
        pixels_per_mm: 1.0,
    }
}

#[test]
fn push_pull_extrusion_replaces_cut_preview_without_resurrecting_it() {
    for cut_distance in ["-10", "-20"] {
        let mut app = KetchupApp::new();
        assert!(app.complete_circle(Vec3::new(35.0, 25.0, 20.0), 5.0, Vec3::new(1.0, 0.0, 0.0)));
        let before = app.canonical_digest();
        let steps = app.undo_step_count();
        app.set_push_pull_distance_input(cut_distance);
        assert!(app.start_preview());
        app.smart_push_pull_chooser.as_mut().unwrap().selected =
            SmartPushPullChoice::ProfileCut(OccurrenceId(1));
        assert!(app.confirm_smart_push_pull_choice());
        assert!(app.has_occurrence_operation_preview());
        app.set_push_pull_distance_input("5");
        assert!(app.start_preview());
        assert!(app.has_preview());
        assert!(!app.has_occurrence_operation_preview());
        assert_eq!(app.canonical_digest(), before);
        assert_eq!(app.undo_step_count(), steps);
        app.set_push_pull_distance_input(cut_distance);
        assert!(
            !app.has_occurrence_operation_preview(),
            "typing the former distance must not resurrect a superseded cut preview"
        );
        assert!(app.occurrence_operation_preview.is_none());
        assert!(!app.confirm_push_pull_preview());
        assert_eq!(app.canonical_digest(), before);
        assert_eq!(app.undo_step_count(), steps);
        app.set_push_pull_distance_input("5");
        assert!(app.start_preview());
        assert!(app.confirm_push_pull_preview());
        assert_eq!(app.undo_step_count(), steps + 1);
        assert_eq!(app.occurrence_box_geometry(1).unwrap().1.z, 20.0);
        assert_eq!(app.occurrence_box_geometry(2).unwrap().1.z, 5.0);
        assert!(app.occurrence_operation_preview.is_none());
        let after = app.canonical_digest();
        assert!(app.undo());
        assert_eq!(app.canonical_digest(), before);
        assert!(app.redo());
        assert_eq!(app.canonical_digest(), after);
    }
}

#[test]
fn push_pull_zero_gesture_clears_cut_choice_without_losing_anchor() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Second target".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    assert!(app.complete_circle(Vec3::new(35.0, 25.0, 20.0), 5.0, Vec3::new(1.0, 0.0, 0.0)));
    let drag = drag(&app, app.selection.primary.clone().unwrap());
    app.push_pull_anchor = Some(drag.clone());
    let before = app.canonical_digest();
    let steps = app.undo_step_count();
    assert!(!app.update_push_pull_gesture(&drag, Pos2::new(0.0, 10.0)));
    assert!(app.has_smart_push_pull_chooser());
    assert!(!app.update_push_pull_gesture(&drag, Pos2::ZERO));
    assert!(
        !app.has_smart_push_pull_chooser(),
        "zero distance must dismiss the old cut choice"
    );
    assert!(app.smart_push_pull_proposal.is_none());
    assert!(app.smart_push_pull_planning.is_none());
    assert_eq!(app.status_key, "status-ready");
    assert!(app.push_pull_click_anchor_active());
    assert_eq!(app.canonical_digest(), before);
    assert_eq!(app.undo_step_count(), steps);
    assert!(app.update_push_pull_gesture(&drag, Pos2::new(0.0, -5.0)));
    assert!(app.has_preview());
    assert!(app.confirm_push_pull_preview());
    assert_eq!(app.undo_step_count(), steps + 1);
    let after = app.canonical_digest();
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), after);
}

#[test]
fn push_pull_failed_replan_clears_previous_choice_and_can_recover() {
    for invalid in ["invalid", "-30"] {
        let mut app = KetchupApp::new();
        assert!(app.complete_circle(Vec3::new(35.0, 25.0, 20.0), 5.0, Vec3::new(1.0, 0.0, 0.0)));
        let before = app.canonical_digest();
        let steps = app.undo_step_count();
        app.set_push_pull_distance_input("-10");
        assert!(app.start_preview());
        assert!(app.has_smart_push_pull_chooser());
        app.set_push_pull_distance_input(invalid);
        assert!(!app.start_preview());
        assert!(
            !app.has_smart_push_pull_chooser(),
            "failed replan {invalid} kept the old cut choice"
        );
        assert!(app.smart_push_pull_proposal.is_none());
        assert!(app.smart_push_pull_planning.is_none());
        assert!(!app.has_preview());
        assert!(!app.has_occurrence_operation_preview());
        assert_eq!(app.status_key, "error-preview-stale");
        assert_eq!(app.canonical_digest(), before);
        assert_eq!(app.undo_step_count(), steps);
        app.set_push_pull_distance_input("5");
        assert!(app.start_preview());
        assert!(app.confirm_push_pull_preview());
        assert_eq!(app.undo_step_count(), steps + 1);
    }
}

#[test]
fn push_pull_clears_exact_preview_on_zero_or_failed_replan() {
    for zero_gesture in [true, false] {
        let mut app = planar_push_pull::tests::prism(
            &[[0.0, 0.0], [40.0, 0.0], [8.0, 30.0]],
            Transform::identity(),
        );
        assert!(app.select_topological_locator(TopologicalPickLocator {
            instance_path: InstancePath::root(OccurrenceId(1)),
            producer_feature_id: FeatureId(2),
            kind: TopologicalElementKind::Face,
            ordinal: 1,
        }));
        let drag = drag(&app, app.selection.primary.clone().unwrap());
        app.push_pull_anchor = Some(drag.clone());
        let before = app.canonical_digest();
        let steps = app.undo_step_count();
        assert!(app.update_push_pull_gesture(&drag, Pos2::new(0.0, -2.0)));
        planar_push_pull::tests::wait_preview(&mut app);
        assert!(app.face_offset_evaluation.is_some());
        if zero_gesture {
            assert!(!app.update_push_pull_gesture(&drag, Pos2::ZERO));
        } else {
            app.set_push_pull_distance_input("invalid");
            assert!(!app.start_preview());
        }
        assert!(app.face_offset_evaluation.is_none());
        assert!(app.face_offset_preview_due.is_none());
        assert!(app.smart_push_pull_proposal.is_none());
        assert!(app.preview_box.is_none());
        assert!(app.push_pull_click_anchor_active());
        assert!(!app.confirm_push_pull_preview());
        assert_eq!(app.canonical_digest(), before);
        assert_eq!(app.undo_step_count(), steps);
        assert!(app.update_push_pull_gesture(&drag, Pos2::new(0.0, -3.0)));
        planar_push_pull::tests::wait_preview(&mut app);
        assert!(app.confirm_push_pull_preview());
        assert_eq!(app.undo_step_count(), steps + 1);
    }
}

#[test]
fn push_pull_snaps_points_and_edges_on_all_signed_axes() {
    for axis in [Axis::X, Axis::Y, Axis::Z] {
        for side in [Side::Minimum, Side::Maximum] {
            for kind in [
                SnapKind::Endpoint,
                SnapKind::Midpoint,
                SnapKind::Edge,
                SnapKind::Center,
            ] {
                let mut app = KetchupApp::new();
                let source = app.active_boxes()[0].clone();
                let selection = SelectionId {
                    definition_id: source.definition_id,
                    instance_path: source.instance_path.clone(),
                    element: ElementId::Face { axis, side },
                };
                app.selection.primary = Some(selection.clone());
                let drag = drag(&app, selection.clone());
                let normal = axis_direction(axis) * if side == Side::Maximum { 1.0 } else { -1.0 };
                let origin = source.origin_mm
                    + if side == Side::Maximum {
                        source.size_mm
                    } else {
                        Vec3::ZERO
                    };
                for distance in [17.123456789, -3.125] {
                    app.hover_snap = Some(SnapResult {
                        kind,
                        reference: selection.clone(),
                        position_mm: origin + normal * distance,
                        distance_mm: 0.0,
                    });
                    let actual = app
                        .push_pull_snap_distance(&drag)
                        .expect("point/edge target, even on the same occurrence");
                    assert!(
                        (actual - distance).abs() < 1e-10,
                        "{axis:?} {side:?} {kind:?}: {actual}"
                    );
                }
                app.face_workflow.set_snaps_enabled(false);
                assert!(app.push_pull_snap_distance(&drag).is_none());
            }
        }
    }
}

#[test]
fn push_pull_snaps_oblique_scaled_face_using_world_normal() {
    let transform = Transform::from_matrix([
        1.6, -1.2, 0.0, 100.0, 1.2, 1.6, 0.0, -30.0, 0.0, 0.0, 2.0, 5.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    for distance in [3.123456789, -1.123456789] {
        let mut app =
            planar_push_pull::tests::prism(&[[0.0, 0.0], [40.0, 0.0], [8.0, 30.0]], transform);
        assert!(app.select_topological_locator(TopologicalPickLocator {
            instance_path: InstancePath::root(OccurrenceId(1)),
            producer_feature_id: FeatureId(2),
            kind: TopologicalElementKind::Face,
            ordinal: 1,
        }));
        let selection = app.selection.primary.clone().unwrap();
        let face = app.selected_planar_face(&selection).unwrap();
        assert!(face.normal.x.abs() > 0.01 && face.normal.y.abs() > 0.01);
        let drag = drag(&app, selection.clone());
        let tangent = cross(face.normal, Vec3::new(0.0, 0.0, 1.0));
        app.hover_snap = Some(SnapResult {
            kind: SnapKind::Edge,
            reference: selection,
            position_mm: face.origin + face.normal * distance + tangent * 25.0,
            distance_mm: 0.0,
        });
        let before = app.canonical_digest();
        let steps = app.undo_step_count();
        assert!(app.update_push_pull_gesture(&drag, Pos2::ZERO));
        assert!(
            (parse_distance_mm(&app.push_pull_distance_input).unwrap() - distance).abs() < 1e-10
        );
        planar_push_pull::tests::wait_preview(&mut app);
        assert!(app.confirm_push_pull_preview());
        let snapshot = app.document.current();
        let FeatureKind::TopologyFaceOffset {
            distance: stored, ..
        } = snapshot.features().last().unwrap().kind()
        else {
            panic!("face offset required")
        };
        let local_distance = distance / face.local_to_world_scale;
        assert!((stored.millimetres() - local_distance).abs() < 1e-10);
        assert!((parse_distance_mm(stored.source_token()).unwrap() - local_distance).abs() < 1e-10);
        assert_eq!(app.undo_step_count(), steps + 1);
        let after = app.canonical_digest();
        assert!(app.undo());
        assert_eq!(app.canonical_digest(), before);
        assert!(app.redo());
        assert_eq!(app.canonical_digest(), after);
    }
}

#[test]
fn push_pull_negative_profile_target_and_invalid_solid_extent() {
    let mut app = KetchupApp::new();
    app.new_document();
    assert!(app.create_profile_at(
        Vec3::new(0.0, 0.0, 40.0),
        vec![[0.0, 0.0], [60.0, 0.0], [10.0, 30.0]]
    ));
    let selection = SelectionId {
        definition_id: app.active_boxes()[0].definition_id,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.selection.primary = Some(selection.clone());
    app.active_tool = ActiveTool::PushPull;
    let drag = drag(&app, selection.clone());
    assert_eq!(drag.extent_start_mm, 0.0);
    app.hover_snap = Some(SnapResult {
        kind: SnapKind::Endpoint,
        reference: selection,
        position_mm: Vec3::new(100.0, 0.0, 13.123456789),
        distance_mm: 0.0,
    });
    assert!(app.update_push_pull_gesture(&drag, Pos2::ZERO));
    assert!(app.confirm_push_pull_preview());
    let (origin, size) = app.occurrence_box_geometry(1).unwrap();
    assert!((origin.z - 13.123456789).abs() < 1e-10);
    assert!((origin.z + size.z - 40.0).abs() < 1e-10);
    let stale_digest = app.canonical_digest();
    assert!(!app.update_push_pull_gesture(&drag, Pos2::ZERO));
    assert_eq!(app.canonical_digest(), stale_digest);

    let mut solid = KetchupApp::new();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    solid.selection.primary = Some(selection.clone());
    let drag = self::drag(&solid, selection.clone());
    for z in [0.0, -10.0, f64::NAN, f64::INFINITY] {
        solid.hover_snap = Some(SnapResult {
            kind: SnapKind::Endpoint,
            reference: selection.clone(),
            position_mm: Vec3::new(0.0, 0.0, z),
            distance_mm: 0.0,
        });
        assert!(solid.push_pull_snap_distance(&drag).is_none());
    }
}

fn pointer(harness: &mut Harness<'_, KetchupApp>, world: Vec3) -> Pos2 {
    let pos = harness.state().viewport_position(world).unwrap();
    assert!(harness.state().viewport_rect.unwrap().contains(pos));
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(pos));
    harness.step();
    pos
}

fn click(harness: &mut Harness<'_, KetchupApp>, world: Vec3) {
    let pos = pointer(harness, world);
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
}

#[test]
fn push_pull_snaps_profile_corner_midpoint_and_edge_with_exact_preview_commit() {
    for (target, kind) in [
        (Vec3::new(160.0, 0.0, 57.123456789), SnapKind::Endpoint),
        (Vec3::new(190.0, 0.0, 57.123456789), SnapKind::Midpoint),
        (Vec3::new(175.0, 0.0, 57.123456789), SnapKind::Edge),
    ] {
        let mut app = KetchupApp::new();
        assert!(app.create_profile_at(
            Vec3::new(160.0, 0.0, target.z),
            vec![[0.0, 0.0], [60.0, 0.0], [15.0, 40.0]]
        ));
        app.dispatch_command(AppCommand::PushPull);
        let mut harness = Harness::builder()
            .with_size(Vec2::new(1600.0, 1000.0))
            .with_step_dt(1.0 / 60.0)
            .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
        harness.step();
        click(&mut harness, Vec3::new(50.0, 30.0, 20.0));
        assert!(harness.state().push_pull_click_anchor_active());
        let before = harness.state().canonical_digest();
        let steps = harness.state().undo_step_count();
        pointer(&mut harness, target);
        assert_eq!(harness.state().hovered_snap_kind(), Some(kind));
        assert_eq!(harness.state().canonical_digest(), before);
        let expected = harness.state().hovered_snap_position().unwrap().z;
        assert!((expected - target.z).abs() < 1e-8);
        let preview = &harness
            .state()
            .preview_box
            .as_ref()
            .expect("snapped preview")
            .plan
            .preview_box;
        assert!(
            (preview.origin_mm.z + preview.size_mm.z - expected).abs() < 1e-8,
            "preview must meet target without rounding"
        );
        click(&mut harness, target);
        assert!(!harness.state().push_pull_click_anchor_active());
        let (origin, size) = harness.state().occurrence_box_geometry(1).unwrap();
        assert!((origin.z + size.z - expected).abs() < 1e-8);
        assert_eq!(harness.state().undo_step_count(), steps + 1);
        let after = harness.state().canonical_digest();
        assert!(harness.state_mut().undo());
        assert_eq!(harness.state().canonical_digest(), before);
        assert!(harness.state_mut().redo());
        assert_eq!(harness.state().canonical_digest(), after);
    }
}

#[test]
fn push_pull_snaps_to_nonparallel_face_at_pointer() {
    let mut app = KetchupApp::new();
    app.selection.select_occurrence(OccurrenceId(1), false);
    assert!(app.copy_selected(Vec3::new(150.0, 0.0, 40.0)));
    app.dispatch_command(AppCommand::PushPull);
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 1000.0));
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.selection.primary = Some(selection.clone());
    let drag = drag(&app, selection);
    let point = Vec3::new(200.0, 0.0, 47.123456789);
    app.update_viewport_inference(Some(app.project(point, rect)), rect);
    assert_eq!(app.hovered_snap_kind(), Some(SnapKind::Face));
    let hit = app.hover_pick.as_ref().unwrap().overlap_choice(0).unwrap();
    assert!(matches!(
        hit.reference.element,
        ElementId::Face { axis: Axis::Y, .. }
    ));
    let expected = hit.position_mm.z - 20.0;
    assert!(expected > 20.0);
    let actual = app
        .push_pull_snap_distance(&drag)
        .expect("nonparallel face target");
    assert!((actual - expected).abs() < 1e-8);
}
