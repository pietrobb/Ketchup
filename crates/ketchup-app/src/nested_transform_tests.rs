use super::*;
#[test]
fn nested_gesture_preview_transforms_equal_committed_transforms() {
    for group_id in [None, Some(GroupId(30))] {
        for rotate in [false, true] {
            let mut app = nested_app();
            let selection = SelectionId {
                definition_id: DefinitionId(1),
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            };
            if let Some(group_id) = group_id {
                assert!(app.select_group(group_id));
            } else {
                assert!(app.enter_group_context(GroupId(30)));
                app.select_from_outliner(selection.instance_path.clone(), false);
            }
            let before = app.document.current();
            let preview = if rotate {
                let drag = RotateDrag {
                    source_document_id: before.document_id(),
                    source_revision: before.revision_id(),
                    selection: selection.clone(),
                    occurrence_paths: BTreeSet::from([selection.instance_path.clone()]),
                    group_id,
                    centre_mm: Vec3::new(26.0, 44.0, -17.0),
                    axis: Axis::Y,
                    reference_mm: Some(Vec3::new(1.0, 0.0, 0.0)),
                    angle_degrees: 35.0,
                    copy: false,
                };
                app.set_rotate_session(ToolSessionPhase::Gesture, drag.clone());
                let preview = app.preview_transform_overrides();
                assert_eq!(app.canonical_digest(), before.canonical_digest());
                app.take_rotate_session(Some(ToolSessionPhase::Gesture));
                assert!(app.commit_rotate_drag(&drag));
                preview
            } else {
                let drag = MoveDrag {
                    source_document_id: before.document_id(),
                    source_revision: before.revision_id(),
                    occurrence_paths: BTreeSet::from([selection.instance_path.clone()]),
                    selection,
                    group_id,
                    profile_target: None,
                    pointer_start_world: Vec3::ZERO,
                    plane_z: 0.0,
                    axis: None,
                    axis_reference: None,
                    delta_mm: Vec3::new(37.0, -19.0, 53.0),
                    copy: false,
                };
                app.set_move_session(ToolSessionPhase::Gesture, drag.clone());
                let preview = app.preview_transform_overrides();
                assert_eq!(app.canonical_digest(), before.canonical_digest());
                app.take_move_session(Some(ToolSessionPhase::Gesture));
                assert!(app.commit_move_drag(&drag));
                preview
            };
            assert_eq!(preview.len(), if group_id.is_some() { 2 } else { 1 });
            assert!(!preview.contains_key(&InstancePath::root(OccurrenceId(2))));
            for (path, transform) in preview {
                assert_transform(world(&app, path.root_occurrence().0), transform);
            }
        }
    }
}

fn nested_app() -> KetchupApp {
    let mut app = KetchupApp::new();
    assert!(app.create_box_at(Vec3::new(250.0, 100.0, 40.0), Vec3::new(30.0, 20.0, 10.0)));
    assert!(app.create_box_at(Vec3::new(160.0, 70.0, 25.0), Vec3::new(40.0, 30.0, 20.0)));
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(10),
                name: "Outer".into(),
                parent: None,
                transform: Transform::from_translation(30.0, -40.0, 20.0)
                    .unwrap()
                    .compose(world_rotation_transform(Vec3::ZERO, Axis::Z, 90.0).unwrap()),
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(20),
                name: "Middle".into(),
                parent: Some(GroupId(10)),
                transform: Transform::from_translation(10.0, 20.0, 30.0)
                    .unwrap()
                    .compose(world_rotation_transform(Vec3::ZERO, Axis::X, 90.0).unwrap()),
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(30),
                name: "Target".into(),
                parent: Some(GroupId(20)),
                transform: Transform::from_translation(12.0, 8.0, 4.0).unwrap(),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: Some(GroupId(30)),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(2),
                parent: Some(GroupId(20)),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(3),
                parent: Some(GroupId(30)),
            },
        ]))
        .unwrap();
    app.selection.clear();
    app
}

fn assert_transform(actual: Transform, expected: Transform) {
    for (index, (a, e)) in actual.matrix().iter().zip(expected.matrix()).enumerate() {
        assert!(
            (a - e).abs() < 1.0e-8,
            "matrix[{index}]: {a} != {e}\n{actual:?}\n{expected:?}"
        );
    }
}

fn world(app: &KetchupApp, id: u64) -> Transform {
    app.document
        .current()
        .world_transform_for_occurrence(OccurrenceId(id))
        .unwrap()
}

fn assert_history(app: &mut KetchupApp, before: &Snapshot, steps: usize) {
    let after = app.canonical_digest();
    assert_eq!(app.undo_step_count(), steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before.canonical_digest());
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), after);
}

#[test]
fn nested_world_move_and_copy_preserve_siblings_and_history() {
    for copy in [false, true] {
        let mut app = nested_app();
        assert!(app.enter_group_context(GroupId(30)));
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        let before = app.document.current();
        let steps = app.undo_step_count();
        let original = world(&app, 1);
        let sibling = world(&app, 2);
        let reference = world(&app, 3);
        let delta = Vec3::new(37.0, -19.0, 53.0);
        assert!(if copy {
            app.copy_selected(delta)
        } else {
            app.move_selected(delta)
        });
        let target = if copy { 4 } else { 1 };
        assert_transform(
            world(&app, target),
            translated_transform(original, delta).unwrap(),
        );
        assert_eq!(world(&app, 2), sibling);
        assert_eq!(world(&app, 3), reference);
        assert_eq!(
            app.document
                .current()
                .occurrence(OccurrenceId(target))
                .unwrap()
                .parent(),
            Some(GroupId(30))
        );
        if copy {
            assert_eq!(world(&app, 1), original);
        }
        assert_history(&mut app, &before, steps);
    }
}

#[test]
fn nested_group_world_move_and_rotate_preserve_siblings_and_history() {
    for rotate in [false, true] {
        let mut app = nested_app();
        assert!(app.select_group(GroupId(30)));
        let before = app.document.current();
        let steps = app.undo_step_count();
        let original = world(&app, 1);
        let reference = world(&app, 3);
        let sibling = world(&app, 2);
        let delta = Vec3::new(37.0, -19.0, 53.0);
        let centre = Vec3::new(26.0, 44.0, -17.0);
        let edit = if rotate {
            assert!(app.rotate_selected_around(centre, Axis::Y, 35.0));
            world_rotation_transform(centre, Axis::Y, 35.0).unwrap()
        } else {
            assert!(app.move_selected(delta));
            Transform::from_translation(delta.x, delta.y, delta.z).unwrap()
        };
        assert_transform(world(&app, 1), edit.compose(original));
        assert_transform(world(&app, 3), edit.compose(reference));
        assert_eq!(world(&app, 2), sibling);
        assert_eq!(
            app.document
                .current()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .transform(),
            before.occurrence(OccurrenceId(1)).unwrap().transform()
        );
        assert_history(&mut app, &before, steps);
    }
}

#[test]
fn nested_world_rotate_copy_and_angle_correction_use_the_same_pivot() {
    for group in [false, true] {
        for copy in [false, true] {
            if group && copy {
                continue;
            }
            let mut app = nested_app();
            let before = app.document.current();
            let steps = app.undo_step_count();
            let original = world(&app, 1);
            let sibling = world(&app, 2);
            let centre = Vec3::new(26.0, 44.0, -17.0);
            app.rotate_axis_lock = Some(Axis::Y);
            if group {
                assert!(app.select_group(GroupId(30)));
                assert!(app.rotate_selected_around(centre, Axis::Y, 35.0));
            } else {
                assert!(app.enter_group_context(GroupId(30)));
                app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
                let selection = app.selected_move_reference().unwrap();
                let occurrence_paths = app.selected_instance_paths();
                assert!(if copy {
                    app.rotate_copy_occurrences(
                        &selection,
                        &occurrence_paths,
                        centre,
                        Axis::Y,
                        35.0,
                    )
                } else {
                    app.rotate_selected_around(centre, Axis::Y, 35.0)
                });
            }
            assert_transform(
                world(&app, if copy { 4 } else { 1 }),
                world_rotation_transform(centre, Axis::Y, 35.0)
                    .unwrap()
                    .compose(original),
            );
            if copy {
                assert_eq!(world(&app, 1), original);
            } else {
                assert!(app.correct_last_rotation(20.0));
                assert_transform(
                    world(&app, 1),
                    world_rotation_transform(centre, Axis::Y, 20.0)
                        .unwrap()
                        .compose(original),
                );
            }
            assert_eq!(world(&app, 2), sibling);
            assert_history(&mut app, &before, steps);
        }
    }
}

#[test]
fn nested_world_alignment_preview_matches_committed_geometry() {
    for axis in [Axis::X, Axis::Y, Axis::Z] {
        let mut app = nested_app();
        assert!(app.enter_group_context(GroupId(30)));
        let before = app.document.current();
        let steps = app.undo_step_count();
        let sibling = world(&app, 2);
        let reference = world(&app, 3);
        let original = world(&app, 1);
        let origin = app.occurrence_box_geometry(1).unwrap().0;
        assert!(app.preview_align_occurrences(
            OccurrenceId(1),
            OccurrenceId(3),
            axis,
            AlignMode::Center
        ));
        let preview = app
            .occurrence_operation_preview_geometry(OccurrenceId(1))
            .unwrap();
        assert_eq!(app.canonical_digest(), before.canonical_digest());
        assert!(app.confirm_occurrence_operation_preview());
        assert_transform(
            world(&app, 1),
            translated_transform(original, preview.0 - origin).unwrap(),
        );
        let actual = app.occurrence_box_geometry(1).unwrap();
        assert!(vector_length(actual.0 - preview.0) < 1.0e-8);
        assert!(vector_length(actual.1 - preview.1) < 1.0e-8);
        assert_eq!(world(&app, 2), sibling);
        assert_eq!(world(&app, 3), reference);
        assert_history(&mut app, &before, steps);
    }
}
