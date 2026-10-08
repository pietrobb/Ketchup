//! Move, rotate, copy, arrays, patterns, alignment and distribution.

use super::*;

/// The Rotate tool is useless if the user cannot see what will turn and
/// about which axis, so the protractor is asserted on the painted frame
/// rather than trusted because the state exists.
#[test]
fn the_armed_rotate_tool_paints_a_protractor_on_the_axis_it_will_turn_about() {
    let mut app = KetchupApp::new();
    let context = egui::Context::default();
    select_initial_top_face(&mut app);

    let blue = guide_tick_colour(Axis::Z);
    let red = guide_tick_colour(Axis::X);
    assert_eq!(
        painted_segments(&context, &mut app, blue),
        0,
        "no other tool may paint a rotation protractor"
    );

    app.dispatch_command(AppCommand::Rotate);
    let ticks = (360.0 / ROTATION_SNAP_DEGREES).round() as usize;
    assert_eq!(
        painted_segments(&context, &mut app, blue),
        ticks,
        "arming Rotate on a selection must show the blue Z protractor before any gesture"
    );

    // Pinning an axis has to move the protractor with it, otherwise the
    // arrow keys change the outcome invisibly.
    app.set_rotate_axis_lock(Some(Axis::X));
    assert_eq!(painted_segments(&context, &mut app, blue), 0);
    assert_eq!(painted_segments(&context, &mut app, red), ticks);

    // A gesture in flight must additionally show where it started and how
    // far it has come, which is the part that answers "which way".
    app.set_rotate_axis_lock(Some(Axis::Z));
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1_000.0, 800.0));
    app.camera.viewport_rect = Some(rect);
    let grab = app.project(Vec3::new(90.0, 30.0, 20.0), rect);
    app.update_viewport_inference(Some(grab), rect);
    assert!(app.begin_rotate_drag_at(grab, rect, false));
    let mut drag = app
        .take_rotate_session(Some(ToolSessionPhase::Gesture))
        .expect("the gesture has started");
    drag.reference_mm = Some(Vec3::new(40.0, 0.0, 0.0));
    app.advance_rotation(
        &mut drag,
        app.project(Vec3::new(50.0, 90.0, 20.0), rect),
        rect,
        false,
    );
    app.set_rotate_session(ToolSessionPhase::Gesture, drag);
    let guide = app.rotation_guide().expect("a live gesture has a guide");
    assert!(
        guide.start_degrees.is_some(),
        "the starting arm must be drawn so the turn has a visible origin"
    );
    assert!(
        rotation_is_meaningful(guide.angle_degrees),
        "the swept angle must reach the guide: {:?}",
        guide.angle_degrees
    );

    app.dispatch_command(AppCommand::Select);
    app.clear_selection();
    app.dispatch_command(AppCommand::Rotate);
    assert_eq!(
        painted_segments(&context, &mut app, blue),
        0,
        "with nothing selected there is no body to turn and nothing to draw"
    );
}

#[test]
fn move_selected_moves_every_selected_occurrence_in_one_undo_step() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    let before = app
        .active_boxes()
        .into_iter()
        .map(|item| (item.instance_path, item.origin_mm))
        .collect::<BTreeMap<_, _>>();
    let undo_steps = app.document.visible_undo_steps();
    let delta = Vec3::new(25.0, -15.0, 8.0);

    assert!(app.move_selected(delta));
    assert_eq!(app.document.visible_undo_steps(), undo_steps + 1);
    assert_eq!(app.selected_occurrence_count(), 2);
    for item in app.active_boxes() {
        assert_eq!(item.origin_mm, before[&item.instance_path] + delta);
        assert!(app.occurrence_is_selected(item.instance_path.root_occurrence()));
    }

    assert!(app.undo());
    for item in app.active_boxes() {
        assert_eq!(item.origin_mm, before[&item.instance_path]);
    }
}

#[test]
fn move_drag_keeps_the_existing_multi_selection_for_preview_and_commit() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let first = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let second = SelectionId {
        definition_id: DefinitionId(2),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.selection.select_exact(first.clone(), false);
    app.selection.select_exact(second, true);
    app.hover.target = Some(first);
    app.hover.snap = None;
    app.hover.pick = None;
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
    let pointer = app.project(Vec3::new(50.0, 30.0, 20.0), rect);

    assert!(app.begin_move_drag_at(pointer, rect, false));
    let mut drag = app
        .take_move_session(Some(ToolSessionPhase::Gesture))
        .unwrap();
    assert_eq!(drag.occurrence_paths.len(), 2);
    drag.delta_mm = Vec3::new(20.0, 10.0, 5.0);
    app.set_move_session(ToolSessionPhase::Gesture, drag.clone());
    assert_eq!(app.move_preview_transform_overrides().len(), 2);
    app.take_move_session(Some(ToolSessionPhase::Gesture));

    assert!(app.commit_move_drag(&drag));
    assert_eq!(app.selected_occurrence_count(), 2);
    assert_eq!(app.active_boxes()[0].origin_mm, Vec3::new(20.0, 10.0, 5.0));
    assert_eq!(app.active_boxes()[1].origin_mm, Vec3::new(55.0, 45.0, 5.0));
}

#[test]
fn shift_deselecting_a_group_member_moves_only_the_parts_still_selected() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.selection.selected_group.is_some());
    let first = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.selection.select_exact(first, true);
    assert_eq!(app.selection.selected_group, None);
    let before = app
        .active_boxes()
        .iter()
        .map(|item| (item.instance_path.clone(), item.origin_mm))
        .collect::<Vec<_>>();

    assert!(app.move_selected(Vec3::new(10.0, 0.0, 0.0)));
    for (path, origin) in before {
        let moved = app
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == path)
            .expect("both parts remain");
        let expected = if path == InstancePath::root(OccurrenceId(1)) {
            origin
        } else {
            origin + Vec3::new(10.0, 0.0, 0.0)
        };
        assert_eq!(moved.origin_mm, expected, "{path:?}");
    }
}

#[test]
fn move_rotate_delete_are_independent_undoable_scene_operations() {
    let mut app = KetchupApp::new();
    let selected = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.selection.primary = Some(selected);

    assert!(!app.move_selected(Vec3::ZERO));
    assert!(app.move_selected(Vec3::new(10.0, -5.0, 3.0)));
    assert_eq!(app.active_boxes()[0].origin_mm, Vec3::new(10.0, -5.0, 3.0));
    assert!(app.undo());
    assert_eq!(app.active_boxes()[0].origin_mm, Vec3::ZERO);
    assert!(app.redo());
    assert_eq!(app.active_boxes()[0].origin_mm, Vec3::new(10.0, -5.0, 3.0));

    assert!(app.rotate_selected_90());
    assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(60.0, 100.0, 20.0));
    assert_eq!(app.active_boxes()[0].origin_mm, Vec3::new(30.0, -25.0, 3.0));
    assert!(app.undo());
    assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(100.0, 60.0, 20.0));
    assert!(app.redo());
    assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(60.0, 100.0, 20.0));

    assert!(app.delete_selected());
    assert_eq!(app.active_box_count(), 0);
    assert_eq!(app.selected_reference(), None);
    assert!(app.undo());
    assert_eq!(app.active_box_count(), 1);
    assert_eq!(app.selected_reference(), None);
    assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(60.0, 100.0, 20.0));
    assert!(app.redo());
    assert_eq!(app.active_box_count(), 0);
    assert_eq!(app.selected_reference(), None);
}

#[test]
fn linear_pattern_preview_adds_virtual_viewport_occurrences_only() {
    let mut app = KetchupApp::new();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    assert!(app.preview_linear_pattern(OccurrenceId(1), Axis::Z, 50.0, 4));

    let snapshot = app.document.current();
    let exact_projection = app.exact_projection(&snapshot);
    let boxes = app.viewport_boxes(&snapshot, &exact_projection);
    assert_eq!(boxes.len(), 4);
    assert_eq!(boxes[3].origin_mm, Vec3::new(0.0, 0.0, 150.0));
    assert_eq!(app.active_box_count(), 1);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
}

#[test]
fn occurrence_alignment_plan_rejects_tamper_and_replay_atomically() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    assert!(app.copy_selected(Vec3::new(300.0, 300.0, 100.0)));
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    let source = app.occurrence_alignment_source_plan().unwrap();
    let plan = app
        .occurrence_alignment_plan(&source, Axis::Z, AlignMode::Center)
        .unwrap();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    let mut tampered_command = plan.clone();
    tampered_command.command = CanonicalCommand::SetOccurrenceTransform {
        id: source.reference_id,
        transform: source.reference_transform,
    };
    assert!(!app.preview_occurrence_alignment_plan(tampered_command));
    let mut tampered_source = plan.clone();
    tampered_source.source.moving_visible = !tampered_source.source.moving_visible;
    assert!(!app.preview_occurrence_alignment_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    app.begin_occurrence_align();
    app.modal.get_mut::<PendingOccurrenceAlign>().unwrap().axis = Axis::Z;
    assert!(app.preview_pending_occurrence_align());
    let preview = app
        .tool_preview
        .get_mut::<OccurrenceOperationPreview>()
        .unwrap();
    preview.batch = CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
        id: source.reference_id,
    }]);
    preview.command_digest = preview.batch.digest();
    assert!(!app.confirm_occurrence_operation_preview());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.preview_pending_occurrence_align());
    app.tool_preview
        .get_mut::<OccurrenceOperationPreview>()
        .unwrap()
        .boxes
        .get_mut(&source.moving_id)
        .unwrap()
        .origin_mm
        .x += 1.0;
    assert!(!app.confirm_occurrence_operation_preview());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.preview_pending_occurrence_align());
    let Some(OccurrenceCanonicalPreviewPlan::Alignment(sealed_plan)) = app
        .tool_preview
        .get_mut::<OccurrenceOperationPreview>()
        .unwrap()
        .canonical_plan
        .as_mut()
    else {
        panic!("alignment preview seals its typed plan");
    };
    sealed_plan.mode = AlignMode::Maximum;
    assert!(!app.confirm_occurrence_operation_preview());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.preview_pending_occurrence_align());
    app.modal
        .get_mut::<PendingOccurrenceAlign>()
        .unwrap()
        .preview_plan
        .as_mut()
        .unwrap()
        .command = CanonicalCommand::SetOccurrenceTransform {
        id: source.reference_id,
        transform: source.reference_transform,
    };
    assert!(!app.confirm_occurrence_align());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    app.modal
        .get_mut::<PendingOccurrenceAlign>()
        .unwrap()
        .preview_plan = Some(plan.clone());
    assert!(app.confirm_occurrence_align());
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let aligned_digest = app.canonical_digest();
    assert!(!app.preview_occurrence_alignment_plan(plan));
    assert_eq!(app.canonical_digest(), aligned_digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), aligned_digest);
}

#[test]
fn occurrence_distribution_plan_rejects_tamper_and_replay_atomically() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    assert!(app.copy_selected(Vec3::new(100.0, 0.0, 0.0)));
    assert!(app.copy_selected(Vec3::new(400.0, 0.0, 0.0)));
    assert!(app.copy_selected(Vec3::new(400.0, 0.0, 0.0)));
    assert!(app.select_all());
    let source = app.occurrence_distribution_source_plan().unwrap();
    let plan = app
        .occurrence_distribution_plan(&source, Axis::X, DistributionMode::Centers)
        .unwrap();
    assert_eq!(
        plan.ordered_occurrence_ids,
        vec![
            OccurrenceId(1),
            OccurrenceId(2),
            OccurrenceId(3),
            OccurrenceId(4)
        ]
    );
    assert_eq!(plan.source_coordinates_mm, vec![50.0, 150.0, 550.0, 950.0]);
    assert_eq!(plan.target_coordinates_mm, vec![50.0, 350.0, 650.0, 950.0]);
    assert_eq!(plan.spacing_mm, 300.0);
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    let mut tampered_commands = plan.clone();
    tampered_commands.commands.reverse();
    assert!(!app.preview_occurrence_distribution_plan(tampered_commands));
    let mut tampered_source = plan.clone();
    let item = tampered_source
        .source
        .occurrences
        .get_mut(&OccurrenceId(2))
        .unwrap();
    item.visible = !item.visible;
    assert!(!app.preview_occurrence_distribution_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    app.begin_occurrence_distribution();
    assert!(app.preview_pending_occurrence_distribution());
    app.modal
        .get_mut::<PendingOccurrenceDistribution>()
        .unwrap()
        .preview_plan
        .as_mut()
        .unwrap()
        .target_coordinates_mm[1] += 1.0;
    assert!(!app.confirm_occurrence_distribution());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    app.modal
        .get_mut::<PendingOccurrenceDistribution>()
        .unwrap()
        .preview_plan = Some(plan.clone());
    assert!(app.confirm_occurrence_distribution());
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let distributed_digest = app.canonical_digest();
    assert!(!app.preview_occurrence_distribution_plan(plan));
    assert_eq!(app.canonical_digest(), distributed_digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), distributed_digest);
}

#[test]
fn linear_pattern_plan_rejects_tamper_and_replay_atomically() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    let source = app.linear_pattern_source_plan().unwrap();
    let plan = app.linear_pattern_plan(&source, Axis::X, 25.0, 3).unwrap();
    assert!(app.preview_linear_pattern_plan(plan.clone()));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    let mut tampered_commands = plan.clone();
    tampered_commands.commands.reverse();
    assert!(!app.apply_linear_pattern_plan(tampered_commands));
    let mut tampered_source = plan.clone();
    tampered_source.source.source_primary = None;
    assert!(!app.apply_linear_pattern_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.apply_linear_pattern_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.active_box_count(), 3);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let patterned_digest = app.canonical_digest();
    assert!(!app.apply_linear_pattern_plan(plan));
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.active_box_count(), 1);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.active_box_count(), 3);
}

#[test]
fn rectangular_pattern_plan_rejects_tamper_and_replay_atomically() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    let source = app.rectangular_pattern_source_plan().unwrap();
    let spec = RectangularPatternSpec {
        primary_axis: Axis::X,
        primary_spacing_mm: 25.0,
        primary_count: 2,
        secondary_axis: Axis::Y,
        secondary_spacing_mm: 30.0,
        secondary_count: 2,
    };
    let plan = app.rectangular_pattern_plan(&source, spec).unwrap();
    assert!(app.preview_rectangular_pattern_plan(plan.clone()));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    let mut tampered_commands = plan.clone();
    tampered_commands.commands.reverse();
    assert!(!app.apply_rectangular_pattern_plan(tampered_commands));
    let mut tampered_source = plan.clone();
    tampered_source.source.source_primary = None;
    assert!(!app.apply_rectangular_pattern_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.apply_rectangular_pattern_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.active_box_count(), 4);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let patterned_digest = app.canonical_digest();
    assert!(!app.apply_rectangular_pattern_plan(plan));
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.active_box_count(), 1);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.active_box_count(), 4);
}

#[test]
fn circular_pattern_plan_rejects_tamper_and_replay_atomically() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    let source = app.circular_pattern_source_plan().unwrap();
    let plan = app
        .circular_pattern_plan(&source, Axis::Z, Vec3::new(100.0, 0.0, 0.0), 90.0, 4)
        .unwrap();
    assert!(app.preview_circular_pattern_plan(plan.clone()));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    let mut tampered_commands = plan.clone();
    tampered_commands.commands.reverse();
    assert!(!app.apply_circular_pattern_plan(tampered_commands));
    let mut tampered_source = plan.clone();
    tampered_source.source.source_primary = None;
    assert!(!app.apply_circular_pattern_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.apply_circular_pattern_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.active_box_count(), 4);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let patterned_digest = app.canonical_digest();
    assert!(!app.apply_circular_pattern_plan(plan));
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.active_box_count(), 1);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.active_box_count(), 4);
}

#[test]
fn exact_topological_selection_ends_numeric_move_correction() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    let initial_origin = app.occurrence_box_geometry(1).unwrap().0;
    let initial_steps = app.undo_step_count();

    app.dispatch_command(AppCommand::Move);
    app.value_box.input = "25,0,0".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.undo_step_count(), initial_steps + 1);

    install_initial_graph_result(&mut app);
    select_initial_topological(&mut app, TopologicalElementKind::Face, 3);
    app.value_box.input = "40,0,0".to_owned();
    assert!(app.apply_value_input());

    assert_eq!(
        app.occurrence_box_geometry(1).unwrap().0,
        initial_origin + Vec3::new(65.0, 0.0, 0.0),
        "a value entered after an exact topological selection must start a fresh Move"
    );
    assert_eq!(app.undo_step_count(), initial_steps + 2);
}

#[test]
fn rotate_previews_commits_and_corrects_the_entire_multi_selection_atomically() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    let occurrence_paths = app.selected_instance_paths();
    let selection = app.selected_move_reference().unwrap();
    let centre_mm = app
        .rotation_centre_for(&|path| occurrence_paths.contains(path))
        .unwrap();
    let snapshot = app.document.current();
    let originals = occurrence_paths
        .iter()
        .map(|path| {
            let id = path.root_occurrence();
            (id, snapshot.occurrence(id).unwrap().transform())
        })
        .collect::<BTreeMap<_, _>>();
    let base_digest = snapshot.canonical_digest();
    let base_revision = snapshot.revision_id();
    let base_steps = app.undo_step_count();

    app.set_rotate_session(
        ToolSessionPhase::Gesture,
        RotateDrag {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            selection: selection.clone(),
            occurrence_paths: occurrence_paths.clone(),
            group_id: None,
            centre_mm,
            axis: Axis::Z,
            reference_mm: Some(Vec3::new(1.0, 0.0, 0.0)),
            angle_degrees: 25.0,
            copy: false,
        },
    );
    assert_eq!(app.rotation_preview_transforms(false).len(), 2);
    assert_eq!(app.canonical_digest(), base_digest);
    app.cancel_preview();
    assert!(app.active_rotate_gesture().is_none());
    assert_eq!(app.canonical_digest(), base_digest);
    assert_eq!(app.undo_step_count(), base_steps);

    app.gesture.transform.rotate_axis_lock = Some(Axis::Z);
    assert!(app.rotate_selected(90.0));
    assert_eq!(app.document_revision(), base_revision + 1);
    assert_eq!(app.undo_step_count(), base_steps + 1);
    assert_eq!(app.selected_occurrence_count(), 2);
    let assert_angle = |app: &KetchupApp, angle_degrees| {
        let rotation = world_rotation_transform(centre_mm, Axis::Z, angle_degrees).unwrap();
        let current = app.document.current();
        for (id, original) in &originals {
            let actual = current.occurrence(*id).unwrap().transform();
            let expected = rotation.compose(*original);
            for (actual, expected) in actual.matrix().iter().zip(expected.matrix()) {
                assert!((actual - expected).abs() < 1.0e-9);
            }
        }
    };
    assert_angle(&app, 90.0);

    assert!(app.correct_last_rotation(40.0));
    assert_angle(&app, 40.0);
    assert_eq!(app.undo_step_count(), base_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), base_digest);
    assert_eq!(app.undo_step_count(), base_steps);
    assert!(app.redo());
    assert_angle(&app, 40.0);
}

#[test]
fn rotate_copy_copies_the_entire_multi_selection_in_one_undo_step() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    let occurrence_paths = app.selected_instance_paths();
    let selection = app.selected_move_reference().unwrap();
    let centre_mm = app
        .rotation_centre_for(&|path| occurrence_paths.contains(path))
        .unwrap();
    let snapshot = app.document.current();
    let originals = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
        (
            snapshot.occurrence(id).unwrap().transform(),
            snapshot.occurrence(id).unwrap().definition_id(),
        )
    });
    let base_digest = snapshot.canonical_digest();
    let base_steps = app.undo_step_count();

    assert!(app.rotate_copy_occurrences(&selection, &occurrence_paths, centre_mm, Axis::Z, 90.0,));
    assert_eq!(app.document.current().occurrences().count(), 4);
    assert_eq!(
        app.selected_occurrence_ids(),
        BTreeSet::from([OccurrenceId(3), OccurrenceId(4)])
    );
    assert_eq!(app.undo_step_count(), base_steps + 1);
    let rotation = world_rotation_transform(centre_mm, Axis::Z, 90.0).unwrap();
    let current = app.document.current();
    for (index, target_id) in [OccurrenceId(3), OccurrenceId(4)].into_iter().enumerate() {
        let target = current.occurrence(target_id).unwrap();
        assert_eq!(target.definition_id(), originals[index].1);
        let actual = target.transform();
        let expected = rotation.compose(originals[index].0);
        for (actual, expected) in actual.matrix().iter().zip(expected.matrix()) {
            assert!((actual - expected).abs() < 1.0e-9);
        }
    }

    assert!(app.undo());
    assert_eq!(app.canonical_digest(), base_digest);
    assert_eq!(app.document.current().occurrences().count(), 2);
}

#[test]
fn move_and_ctrl_copy_commit_occurrence_only_batches_visible_in_outliner() {
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
    let definition_count = app.document.current().definitions().count();

    assert!(app.move_selected(Vec3::new(30.0, 20.0, 0.0)));
    assert_eq!(app.document.visible_undo_steps(), 1);
    assert_eq!(app.outliner_query()[0].occurrences[0].position, "30,20");
    assert_eq!(
        app.document.current().definitions().count(),
        definition_count
    );

    assert!(app.copy_selected(Vec3::new(50.0, 0.0, 0.0)));
    let snapshot = app.document.current();
    assert_eq!(snapshot.definitions().count(), definition_count);
    assert_eq!(snapshot.occurrences().count(), 2);
    assert_eq!(
        snapshot
            .occurrence(OccurrenceId(1))
            .unwrap()
            .definition_id(),
        snapshot
            .occurrence(OccurrenceId(2))
            .unwrap()
            .definition_id()
    );
    assert_eq!(snapshot.scene_query()[0].shared_occurrence_count, 2);
    assert_eq!(
        app.selected_move_reference().unwrap().instance_path,
        InstancePath::root(OccurrenceId(2))
    );
    assert_eq!(app.outliner_query()[0].occurrences[1].position, "80,20");
    assert_eq!(app.document.visible_undo_steps(), 2);

    assert!(app.undo());
    assert_eq!(app.active_box_count(), 1);
    assert!(app.redo());
    assert_eq!(app.active_box_count(), 2);
    assert_eq!(app.outliner_query()[0].occurrences[1].position, "80,20");
}

#[test]
fn move_copy_array_multiplies_and_divides_the_last_vector_in_one_undo_step() {
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
    app.dispatch_command(AppCommand::Move);
    let delta = Vec3::new(50.0, -25.0, 10.0);
    assert!(app.copy_selected(delta));
    assert_eq!(app.document.visible_undo_steps(), 1);

    app.value_box.input = "×5".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.document.visible_undo_steps(), 1);
    assert_eq!(app.document.current().occurrences().count(), 6);
    {
        let snapshot = app.document.current();
        for index in 1..=5 {
            let occurrence = snapshot.occurrence(OccurrenceId(index + 1)).unwrap();
            let transform = occurrence.transform();
            let matrix = transform.matrix();
            assert_eq!(
                [matrix[3], matrix[7], matrix[11]],
                [
                    50.0 * index as f64,
                    -25.0 * index as f64,
                    10.0 * index as f64
                ]
            );
            assert_eq!(occurrence.definition_id(), INITIAL_BOX_DEFINITION);
        }
    }
    assert_eq!(
        app.selected_move_reference().unwrap().instance_path,
        InstancePath::root(OccurrenceId(6))
    );

    app.value_box.input = "/5".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.document.visible_undo_steps(), 1);
    assert_eq!(app.document.current().occurrences().count(), 6);
    {
        let snapshot = app.document.current();
        for index in 1..=5 {
            let occurrence = snapshot.occurrence(OccurrenceId(index + 1)).unwrap();
            let transform = occurrence.transform();
            let matrix = transform.matrix();
            assert_eq!(
                [matrix[3], matrix[7], matrix[11]],
                [10.0 * index as f64, -5.0 * index as f64, 2.0 * index as f64]
            );
        }
    }

    assert!(app.undo());
    assert_eq!(app.document.current().occurrences().count(), 1);
    assert!(app.redo());
    assert_eq!(app.document.current().occurrences().count(), 6);
}

#[test]
fn move_copy_array_repeats_the_entire_multi_selection_atomically() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    let originals = app
        .active_boxes()
        .into_iter()
        .map(|item| item.origin_mm)
        .collect::<Vec<_>>();
    let delta = Vec3::new(30.0, -10.0, 5.0);

    app.dispatch_command(AppCommand::Move);
    assert!(app.copy_selected(delta));
    assert_eq!(app.selected_occurrence_count(), 2);
    app.value_box.input = "×3".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.document.current().occurrences().count(), 8);
    assert_eq!(app.selected_occurrence_count(), 2);
    for (source_index, origin) in originals.iter().copied().enumerate() {
        let copied = app
            .active_boxes()
            .into_iter()
            .find(|item| {
                item.instance_path == InstancePath::root(OccurrenceId(7 + source_index as u64))
            })
            .unwrap();
        assert_eq!(copied.origin_mm, origin + delta * 3.0);
    }

    app.value_box.input = "/3".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.document.current().occurrences().count(), 8);
    for copy_index in 1..=3 {
        for (source_index, origin) in originals.iter().copied().enumerate() {
            let id = 3 + (copy_index - 1) * 2 + source_index;
            let copied = app
                .active_boxes()
                .into_iter()
                .find(|item| item.instance_path == InstancePath::root(OccurrenceId(id as u64)))
                .unwrap();
            assert_eq!(copied.origin_mm, origin + delta * (copy_index as f64 / 3.0));
        }
    }

    assert!(app.undo());
    assert_eq!(app.document.current().occurrences().count(), 2);
    assert!(app.redo());
    assert_eq!(app.document.current().occurrences().count(), 8);
}

#[test]
fn move_copy_array_rejects_stale_or_invalid_modifiers_without_mutation() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.dispatch_command(AppCommand::Move);
    assert!(app.copy_selected(Vec3::new(40.0, 0.0, 0.0)));
    assert!(app.create_box());
    let digest = app.canonical_digest();
    let undo_steps = app.document.visible_undo_steps();

    app.dispatch_command(AppCommand::Move);
    app.value_box.input = "x5".to_owned();
    assert!(!app.apply_value_input());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.document.visible_undo_steps(), undo_steps);

    app.value_box.input = "/0".to_owned();
    assert!(!app.apply_value_input());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.document.visible_undo_steps(), undo_steps);
}

#[test]
fn move_ctrl_is_a_persistent_copy_toggle_during_an_active_gesture() {
    let mut app = KetchupApp::new();
    let snapshot = app.document.current();
    app.set_move_session(
        ToolSessionPhase::Anchor,
        MoveDrag {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            occurrence_paths: BTreeSet::from([InstancePath::root(OccurrenceId(1))]),
            selection: SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            group_id: None,
            profile_target: None,
            pointer_start_world: Vec3::ZERO,
            plane_z: 0.0,
            axis: None,
            axis_reference: None,
            delta_mm: Vec3::new(25.0, 0.0, 0.0),
            copy: false,
        },
    );

    app.update_move_copy_modifier(true);
    assert!(!app.gesture.transform.move_copy);
    app.update_move_copy_modifier(false);
    assert!(app.gesture.transform.move_copy);
    assert!(app.move_session().unwrap().0.copy);

    app.update_move_copy_modifier(true);
    app.update_move_copy_modifier(false);
    assert!(!app.gesture.transform.move_copy);
    assert!(!app.move_session().unwrap().0.copy);
}

#[test]
fn rotate_ctrl_is_a_persistent_copy_toggle_during_an_active_gesture() {
    let mut app = KetchupApp::new();
    let snapshot = app.document.current();
    app.set_rotate_session(
        ToolSessionPhase::Anchor,
        RotateDrag {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            selection: SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            occurrence_paths: BTreeSet::from([InstancePath::root(OccurrenceId(1))]),
            group_id: None,
            centre_mm: Vec3::new(50.0, 30.0, 20.0),
            axis: Axis::Z,
            reference_mm: Some(Vec3::new(25.0, 0.0, 0.0)),
            angle_degrees: 45.0,
            copy: false,
        },
    );

    app.update_rotate_copy_modifier(true);
    assert!(!app.gesture.transform.rotate_copy);
    app.update_rotate_copy_modifier(false);
    assert!(app.gesture.transform.rotate_copy);
    assert!(app.rotate_session().unwrap().0.copy);

    app.update_rotate_copy_modifier(true);
    app.update_rotate_copy_modifier(false);
    assert!(!app.gesture.transform.rotate_copy);
    assert!(!app.rotate_session().unwrap().0.copy);
}

#[test]
fn transform_copy_toggle_ends_with_the_committed_gesture() {
    let mut move_app = KetchupApp::new();
    move_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    let snapshot = move_app.document.current();
    let selection = move_app.selected_move_reference().unwrap();
    let occurrence_paths = move_app.selected_instance_paths();
    move_app.update_move_copy_modifier(true);
    move_app.update_move_copy_modifier(false);
    assert!(move_app.commit_move_drag(&MoveDrag {
        source_document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        selection,
        occurrence_paths,
        group_id: None,
        profile_target: None,
        pointer_start_world: Vec3::ZERO,
        plane_z: 0.0,
        axis: None,
        axis_reference: None,
        delta_mm: Vec3::new(25.0, 0.0, 0.0),
        copy: true,
    }));
    assert!(!move_app.gesture.transform.move_copy);
    move_app.update_move_copy_modifier(true);
    move_app.update_move_copy_modifier(false);
    assert!(
        move_app.gesture.transform.move_copy,
        "the next standalone Ctrl gesture must enable Copy"
    );

    let mut rotate_app = KetchupApp::new();
    rotate_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    let snapshot = rotate_app.document.current();
    let selection = rotate_app.selected_move_reference().unwrap();
    let occurrence_paths = rotate_app.selected_instance_paths();
    let centre_mm = rotate_app
        .rotation_centre_for(&|path| occurrence_paths.contains(path))
        .unwrap();
    rotate_app.update_rotate_copy_modifier(true);
    rotate_app.update_rotate_copy_modifier(false);
    assert!(rotate_app.commit_rotate_drag(&RotateDrag {
        source_document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        selection,
        occurrence_paths,
        group_id: None,
        centre_mm,
        axis: Axis::Z,
        reference_mm: Some(Vec3::new(25.0, 0.0, 0.0)),
        angle_degrees: 45.0,
        copy: true,
    }));
    assert!(!rotate_app.gesture.transform.rotate_copy);
    rotate_app.update_rotate_copy_modifier(true);
    rotate_app.update_rotate_copy_modifier(false);
    assert!(
        rotate_app.gesture.transform.rotate_copy,
        "the next standalone Ctrl gesture must enable Rotate-Copy"
    );
}

#[test]
fn escape_and_tool_change_clear_transform_constraints_without_mutation() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let selected = app.selected_occurrence_ids();
    let snapshot = app.document.current();
    let selection = app.selected_move_reference().unwrap();
    let paths = app.selected_instance_paths();

    app.dispatch_command(AppCommand::Move);
    app.set_move_axis_lock(Some(Axis::X));
    app.set_move_session(
        ToolSessionPhase::Anchor,
        MoveDrag {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            occurrence_paths: paths.clone(),
            selection: selection.clone(),
            group_id: None,
            profile_target: None,
            pointer_start_world: Vec3::ZERO,
            plane_z: 0.0,
            axis: Some(Axis::X),
            axis_reference: Some(0.0),
            delta_mm: Vec3::new(25.0, 0.0, 0.0),
            copy: false,
        },
    );
    app.dispatch_command(AppCommand::Rotate);
    assert!(app.move_session().is_none());
    assert_eq!(app.gesture.transform.move_axis_lock, None);

    app.set_rotate_axis_lock(Some(Axis::Y));
    app.set_rotate_session(
        ToolSessionPhase::Anchor,
        RotateDrag {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            selection,
            occurrence_paths: paths,
            group_id: None,
            centre_mm: Vec3::new(50.0, 30.0, 20.0),
            axis: Axis::Y,
            reference_mm: Some(Vec3::new(25.0, 0.0, 0.0)),
            angle_degrees: 45.0,
            copy: false,
        },
    );
    app.cancel_preview();
    assert!(app.rotate_session().is_none());
    assert_eq!(app.gesture.transform.rotate_axis_lock, None);

    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.selected_occurrence_ids(), selected);
}

#[test]
fn stale_numeric_transform_confirmation_ends_the_copy_toggle_without_mutation() {
    let mut move_app = KetchupApp::new();
    assert!(move_app.create_box());
    move_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    move_app.dispatch_command(AppCommand::Move);
    let snapshot = move_app.document.current();
    move_app.set_move_session(
        ToolSessionPhase::Anchor,
        MoveDrag {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            occurrence_paths: move_app.selected_instance_paths(),
            selection: move_app.selected_move_reference().unwrap(),
            group_id: None,
            profile_target: None,
            pointer_start_world: Vec3::ZERO,
            plane_z: 0.0,
            axis: None,
            axis_reference: None,
            delta_mm: Vec3::new(25.0, 0.0, 0.0),
            copy: true,
        },
    );
    move_app.update_move_copy_modifier(true);
    move_app.update_move_copy_modifier(false);
    move_app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let digest = move_app.canonical_digest();
    let undo_steps = move_app.undo_step_count();
    move_app.value_box.input = "25, 0, 0".to_owned();

    assert!(!move_app.apply_value_input());
    assert!(!move_app.gesture.transform.move_copy);
    assert_eq!(move_app.canonical_digest(), digest);
    assert_eq!(move_app.undo_step_count(), undo_steps);

    let mut rotate_app = KetchupApp::new();
    assert!(rotate_app.create_box());
    rotate_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    rotate_app.dispatch_command(AppCommand::Rotate);
    let snapshot = rotate_app.document.current();
    let occurrence_paths = rotate_app.selected_instance_paths();
    rotate_app.set_rotate_session(
        ToolSessionPhase::Anchor,
        RotateDrag {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            selection: rotate_app.selected_move_reference().unwrap(),
            occurrence_paths: occurrence_paths.clone(),
            group_id: None,
            centre_mm: rotate_app
                .rotation_centre_for(&|path| occurrence_paths.contains(path))
                .unwrap(),
            axis: Axis::Z,
            reference_mm: Some(Vec3::new(25.0, 0.0, 0.0)),
            angle_degrees: 45.0,
            copy: true,
        },
    );
    rotate_app.update_rotate_copy_modifier(true);
    rotate_app.update_rotate_copy_modifier(false);
    rotate_app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let digest = rotate_app.canonical_digest();
    let undo_steps = rotate_app.undo_step_count();
    rotate_app.value_box.input = "45".to_owned();

    assert!(!rotate_app.apply_value_input());
    assert!(!rotate_app.gesture.transform.rotate_copy);
    assert_eq!(rotate_app.canonical_digest(), digest);
    assert_eq!(rotate_app.undo_step_count(), undo_steps);
}

#[test]
fn transform_gestures_fail_closed_after_selection_drift() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    let snapshot = app.document.current();
    let selection = app.selected_move_reference().unwrap();
    let occurrence_paths = app.selected_instance_paths();
    let centre_mm = app
        .rotation_centre_for(&|path| occurrence_paths.contains(path))
        .unwrap();
    let move_drag = MoveDrag {
        source_document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        selection: selection.clone(),
        occurrence_paths: occurrence_paths.clone(),
        group_id: None,
        profile_target: None,
        pointer_start_world: Vec3::ZERO,
        plane_z: 0.0,
        axis: None,
        axis_reference: None,
        delta_mm: Vec3::new(25.0, 0.0, 0.0),
        copy: false,
    };
    let rotate_drag = RotateDrag {
        source_document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        selection,
        occurrence_paths,
        group_id: None,
        centre_mm,
        axis: Axis::Z,
        reference_mm: Some(Vec3::new(25.0, 0.0, 0.0)),
        angle_degrees: 45.0,
        copy: false,
    };

    app.update_move_copy_modifier(true);
    app.update_move_copy_modifier(false);
    app.update_rotate_copy_modifier(true);
    app.update_rotate_copy_modifier(false);
    assert!(app.gesture.transform.move_copy);
    assert!(app.gesture.transform.rotate_copy);

    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let selected = app.selected_occurrence_ids();

    assert!(!app.move_preview_is_current(&move_drag));
    assert!(!app.rotate_preview_is_current(&rotate_drag));
    assert!(!app.commit_move_drag(&move_drag));
    assert!(!app.gesture.transform.move_copy);
    assert!(!app.commit_rotate_drag(&rotate_drag));
    assert!(!app.gesture.transform.rotate_copy);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.selected_occurrence_ids(), selected);
}

#[test]
fn move_vcb_accepts_last_direction_distance_and_exact_vector() {
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
    app.dispatch_command(AppCommand::Move);
    assert!(app.move_selected(Vec3::new(30.0, 40.0, 0.0)));
    app.value_box.input = "100 mm".to_owned();
    assert!(app.apply_value_input());
    let transform = app
        .document
        .current()
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform();
    assert_eq!(transform.matrix()[3], 60.0);
    assert_eq!(transform.matrix()[7], 80.0);
    assert_eq!(app.document.visible_undo_steps(), 1);

    app.value_box.input = "10,-20,5".to_owned();
    assert!(app.apply_value_input());
    let transform = app
        .document
        .current()
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform();
    assert_eq!(transform.matrix()[3], 70.0);
    assert_eq!(transform.matrix()[7], 60.0);
    assert_eq!(transform.matrix()[11], 5.0);
}

#[test]
fn move_drag_is_continuous_and_shift_constrains_dominant_axis() {
    let start = Vec3::new(3.0, 4.0, 20.0);
    assert_eq!(
        continuous_move_delta(start, Vec3::new(31.25, 28.75, 20.0), false),
        Vec3::new(28.25, 24.75, 0.0)
    );
    assert_eq!(
        continuous_move_delta(start, Vec3::new(31.25, 28.75, 20.0), true),
        Vec3::new(28.25, 0.0, 0.0)
    );
}

#[test]
fn move_inference_targets_origin_and_world_axes_only_with_snapping_enabled() {
    let mut app = KetchupApp::new();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
    let near_y_axis = Vec3::new(0.2, 40.0, 0.0);
    let pointer = app.project(near_y_axis, rect);
    assert_eq!(
        app.move_inference_target_at_screen(pointer, rect, near_y_axis),
        Some(Vec3::new(0.0, 40.0, 0.0))
    );

    let origin_pointer = app.project(Vec3::ZERO, rect);
    assert_eq!(
        app.move_inference_target_at_screen(origin_pointer, rect, Vec3::new(20.0, 20.0, 20.0)),
        Some(Vec3::ZERO)
    );

    app.face_workflow.set_snaps_enabled(false);
    assert_eq!(
        app.move_inference_target_at_screen(pointer, rect, near_y_axis),
        None
    );
}
