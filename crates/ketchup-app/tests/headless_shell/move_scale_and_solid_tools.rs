use super::*;

#[test]
fn ctrl_copy_drag_snaps_source_endpoint_exactly_to_target_endpoint() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let target_origin = Vec3::new(137.3, -42.7, 0.0);
    assert!(shell.app_mut().copy_selected(target_origin));
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);

    shell.click_command(AppCommand::Move);
    let (source_point, source_anchor, target_point, target_anchor) = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(100.0, 0.0, 0.0),
        Vec3::new(0.0, 60.0, 0.0),
        Vec3::new(100.0, 60.0, 0.0),
        Vec3::new(0.0, 0.0, 20.0),
        Vec3::new(100.0, 0.0, 20.0),
        Vec3::new(0.0, 60.0, 20.0),
        Vec3::new(100.0, 60.0, 20.0),
    ]
    .into_iter()
    .find_map(|source_point| {
        let source_anchor = shell.app().viewport_position(source_point)?;
        shell.move_pointer(source_anchor);
        if shell.app().hovered_snap_position() != Some(source_point) {
            return None;
        }
        let target_point = target_origin + source_point;
        let target_anchor = shell.app().viewport_position(target_point)?;
        shell.move_pointer(target_anchor);
        (shell.app().hovered_snap_position() == Some(target_point)).then_some((
            source_point,
            source_anchor,
            target_point,
            target_anchor,
        ))
    })
    .expect("one corresponding source/target endpoint pair must be visible");
    shell.move_pointer(source_anchor);
    assert_eq!(shell.app().hovered_snap_position(), Some(source_point));
    shell.move_pointer(target_anchor);
    assert_eq!(shell.app().hovered_snap_position(), Some(target_point));
    shell.move_pointer(source_anchor);

    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let source_geometry = shell.app().occurrence_box_geometry(1).unwrap();
    let target_geometry = shell.app().occurrence_box_geometry(2).unwrap();
    shell.click_at_with(source_anchor, ctrl());
    shell.move_pointer(target_anchor);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().document_revision(), before_revision);
    shell.click_command(AppCommand::Move);
    shell.drag_with(source_anchor, target_anchor, ctrl());

    let snapped_digest = shell.app().canonical_digest();
    assert_eq!(
        shell.app().document_revision(),
        before_revision + 1,
        "Ctrl-copy from {source_point:?} to {target_point:?}: {}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().active_box_count(), 3);
    assert_eq!(shell.app().definition_count(), 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(1),
        Some(source_geometry)
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(2),
        Some(target_geometry)
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(3),
        Some(target_geometry)
    );

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), snapped_digest);
    assert_eq!(
        shell.app().occurrence_box_geometry(3),
        Some(target_geometry)
    );
}

#[test]
fn exact_align_previews_then_commits_one_transform_batch_with_undo_redo() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 30.0)));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    for axis in [Axis::X, Axis::Y, Axis::Z] {
        for mode in [AlignMode::Minimum, AlignMode::Center, AlignMode::Maximum] {
            assert!(shell.app_mut().preview_align_occurrences(
                OccurrenceId(2),
                OccurrenceId(1),
                axis,
                mode,
            ));
            assert_eq!(shell.app().document_revision(), before_revision);
            assert_eq!(shell.app().canonical_digest(), before_digest);
        }
    }

    assert!(shell.app_mut().preview_align_occurrences(
        OccurrenceId(2),
        OccurrenceId(1),
        Axis::X,
        AlignMode::Maximum,
    ));
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2)),
        Some((Vec3::new(0.0, 25.0, 30.0), Vec3::new(100.0, 60.0, 20.0)))
    );

    assert!(shell.app_mut().confirm_occurrence_operation_preview());
    let aligned_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(2),
        Some((Vec3::new(0.0, 25.0, 30.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert_eq!(shell.app().definition_count(), 1);
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app_mut().redo());
    assert_eq!(shell.app().canonical_digest(), aligned_digest);
}

#[test]
fn exact_align_preview_fails_closed_after_the_document_changes() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    assert!(shell.app_mut().preview_align_occurrences(
        OccurrenceId(2),
        OccurrenceId(1),
        Axis::Y,
        AlignMode::Center,
    ));
    assert!(shell.app_mut().move_selected(Vec3::new(0.0, 10.0, 0.0)));
    let changed_digest = shell.app().canonical_digest();

    assert!(!shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), changed_digest);
}

#[test]
fn exact_linear_pattern_previews_then_commits_one_shared_definition_batch() {
    let mut shell = Shell::new();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    assert!(
        shell
            .app_mut()
            .preview_linear_pattern(OccurrenceId(1), Axis::X, 200.0, 5,)
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 1);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2)),
        Some((Vec3::new(200.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(5)),
        Some((Vec3::new(800.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );

    assert!(shell.app_mut().confirm_occurrence_operation_preview());
    shell.wait_for_digest("digest-linear-pattern-committed");
    let patterned_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 5);
    assert_eq!(shell.app().definition_count(), 1);
    for occurrence_id in 1..=5 {
        assert_eq!(
            shell
                .app()
                .occurrence_definition_id(OccurrenceId(occurrence_id)),
            shell.app().occurrence_definition_id(OccurrenceId(1)),
        );
    }
    assert_eq!(
        shell.app().occurrence_box_geometry(5),
        Some((Vec3::new(800.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 1);
    assert!(shell.app_mut().redo());
    assert_eq!(shell.app().canonical_digest(), patterned_digest);
    assert_eq!(shell.app().active_box_count(), 5);
}

#[test]
fn exact_linear_pattern_rejects_invalid_input_and_stale_confirmation() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let before_digest = shell.app().canonical_digest();
    assert!(
        !shell
            .app_mut()
            .preview_linear_pattern(OccurrenceId(1), Axis::Y, f64::NAN, 3,)
    );
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), before_digest);

    assert!(
        shell
            .app_mut()
            .preview_linear_pattern(OccurrenceId(1), Axis::Y, -80.0, 3,)
    );
    assert!(shell.app_mut().move_selected(Vec3::new(0.0, 10.0, 0.0)));
    let changed_digest = shell.app().canonical_digest();
    assert!(!shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    assert_eq!(shell.app().active_box_count(), 1);
}

#[test]
fn typed_move_handles_small_and_large_finite_distances_with_repeatable_history() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let initial_digest = shell.app().canonical_digest();
    let initial_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.type_text("0.001,0,0");
    shell.press_key(Key::Enter);
    let small_digest = shell.app().canonical_digest();
    let small_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    assert!((small_origin.x - initial_origin.x - 0.001).abs() < 1.0e-9);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);

    shell.click_command(AppCommand::Select);
    shell.click_command(AppCommand::Move);
    shell.type_text("1000000,0,0");
    shell.press_key(Key::Enter);
    let large_digest = shell.app().canonical_digest();
    let large_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    assert!((large_origin.x - small_origin.x - 1_000_000.0).abs() < 1.0e-9);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 2);

    for expected in [&small_digest, &initial_digest] {
        shell.key(Key::Z, ctrl());
        assert_eq!(&shell.app().canonical_digest(), expected);
    }
    for expected in [&small_digest, &large_digest] {
        shell.key(Key::Y, ctrl());
        assert_eq!(&shell.app().canonical_digest(), expected);
    }
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        large_origin
    );
}

#[test]
fn typed_move_correction_replaces_the_last_distance_in_one_undo_step() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let initial_digest = shell.app().canonical_digest();
    let initial_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.type_text("30,40,0");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);

    shell.type_text("100");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        initial_origin + Vec3::new(60.0, 80.0, 0.0),
        "the replacement distance must keep the original Move direction"
    );
    assert_eq!(
        shell.app().undo_step_count(),
        initial_undo_steps + 1,
        "correcting the last Move must replace its history tip"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn typed_move_distance_correction_accepts_zero_in_one_undo_step() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let initial_digest = shell.app().canonical_digest();
    let initial_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.type_text("30,40,0");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);

    shell.type_text("0");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        initial_origin,
        "a zero distance must return the Move to its original position"
    );
    assert_eq!(
        shell.app().undo_step_count(),
        initial_undo_steps + 1,
        "zero must correct the last Move distance instead of starting a new step"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn typed_move_vector_correction_replaces_the_last_vector_in_one_undo_step() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let initial_digest = shell.app().canonical_digest();
    let initial_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.type_text("30,40,0");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);

    shell.type_text("60,80,0");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        initial_origin + Vec3::new(60.0, 80.0, 0.0),
        "the replacement vector must be measured from the original position"
    );
    assert_eq!(
        shell.app().undo_step_count(),
        initial_undo_steps + 1,
        "correcting the last Move vector must replace its history tip"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn typed_move_vector_correction_accepts_zero_in_one_undo_step() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let initial_digest = shell.app().canonical_digest();
    let initial_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.type_text("30,40,0");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);

    shell.type_text("0,0,0");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        initial_origin
    );
    assert_eq!(
        shell.app().undo_step_count(),
        initial_undo_steps + 1,
        "zero must correct the last Move vector instead of being rejected"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn typed_move_correction_replaces_group_distance_in_one_undo_step() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-model", AppCommand::Group);
    assert_eq!(shell.app().group_count(), 1);
    let grouped_digest = shell.app().canonical_digest();
    let initial_origins = [1, 2].map(|id| shell.app().occurrence_box_geometry(id).unwrap().0);
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.type_text("25");
    shell.press_key(Key::Enter);
    let first_undo_steps = shell.app().undo_step_count();
    assert_eq!(first_undo_steps, initial_undo_steps + 1);

    shell.type_text("40");
    shell.press_key(Key::Enter);

    for (index, id) in [1, 2].into_iter().enumerate() {
        assert_eq!(
            shell.app().occurrence_box_geometry(id).unwrap().0,
            initial_origins[index] + Vec3::new(40.0, 0.0, 0.0),
            "the replacement distance must move the whole group from its original position"
        );
    }
    assert_eq!(
        shell.app().undo_step_count(),
        first_undo_steps,
        "correcting a grouped Move must replace its history tip"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), grouped_digest);
}

#[test]
fn typed_rotate_correction_replaces_group_angle_in_one_undo_step() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-model", AppCommand::Group);
    assert_eq!(shell.app().group_count(), 1);
    let grouped_digest = shell.app().canonical_digest();
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Rotate);
    shell.type_text("45");
    shell.press_key(Key::Enter);
    let first_undo_steps = shell.app().undo_step_count();
    assert_eq!(first_undo_steps, initial_undo_steps + 1);

    shell.type_text("90");
    shell.press_key(Key::Enter);

    for id in [1, 2] {
        assert_eq!(
            rounded_extents(shell.app().occurrence_box_geometry(id).unwrap().1),
            (60.0, 100.0, 20.0),
            "the replacement angle must rotate the whole group from its original orientation"
        );
    }
    assert_eq!(
        shell.app().undo_step_count(),
        first_undo_steps,
        "correcting a grouped Rotate must replace its history tip"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), grouped_digest);
}

#[test]
fn stale_numeric_move_correction_targets_the_current_selection() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    shell.click_command(AppCommand::Move);
    shell.type_text("25,0,0");
    shell.press_key(Key::Enter);
    let first_after_move = shell.app().occurrence_box_geometry(1).unwrap().0;

    assert!(shell.app_mut().create_box());
    let second_before_move = shell.app().occurrence_box_geometry(2).unwrap().0;
    let undo_steps = shell.app().undo_step_count();
    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        first_after_move,
        "a stale correction must not move the previously transformed occurrence"
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(2).unwrap().0,
        second_before_move + Vec3::new(40.0, 0.0, 0.0),
        "a plain distance after selection drift must start a fresh Move on the current selection"
    );
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
}

#[test]
fn stale_numeric_rotate_correction_targets_the_current_selection() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    shell.click_command(AppCommand::Rotate);
    shell.type_text("45");
    shell.press_key(Key::Enter);
    let first_after_rotate = shell.app().occurrence_box_geometry(1).unwrap();

    assert!(shell.app_mut().create_box());
    let second_before_rotate = shell.app().occurrence_box_geometry(2).unwrap();
    let undo_steps = shell.app().undo_step_count();
    shell.type_text("90");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1),
        Some(first_after_rotate),
        "a stale correction must not rotate the previously transformed occurrence"
    );
    assert_eq!(
        rounded_extents(shell.app().occurrence_box_geometry(2).unwrap().1),
        (60.0, 100.0, 20.0),
        "an angle after selection drift must start a fresh Rotate on the current selection"
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(2).unwrap().0,
        second_before_rotate.0 + Vec3::new(20.0, -20.0, 0.0)
    );
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
}

#[test]
fn reselecting_after_numeric_move_starts_a_new_history_step() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    shell.settle();
    let row_label = |shell: &Shell, occurrence_id| {
        let name = shell
            .app()
            .occurrence_name(occurrence_id)
            .unwrap()
            .trim_end_matches(" #1")
            .to_owned();
        shell.catalog().format(
            "outliner-object",
            &BTreeMap::from([
                ("name", name),
                ("dimensions", "100 × 60 × 20".to_owned()),
                ("visibility", "◉".to_owned()),
            ]),
        )
    };
    let first_row = row_label(&shell, OccurrenceId(1));
    let second_row = row_label(&shell, OccurrenceId(2));
    assert!(shell.app_mut().headless_select_occurrence(OccurrenceId(1)));
    let initial_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.focus_text_input(&shell.catalog().text("value-label-distance"));
    shell.type_text("25");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);

    shell.click_row(&second_row);
    shell.click_row(&first_row);
    shell.click_command(AppCommand::Move);
    shell.focus_text_input(&shell.catalog().text("value-label-distance"));
    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        initial_origin + Vec3::new(65.0, 0.0, 0.0),
        "leaving and restoring the selection must end correction of the old Move"
    );
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 2);
    shell.key(Key::Z, ctrl());
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        initial_origin + Vec3::new(25.0, 0.0, 0.0)
    );
}

#[test]
fn reselecting_after_numeric_rotate_starts_a_new_history_step() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    shell.settle();
    let row_label = |shell: &Shell, occurrence_id| {
        let name = shell
            .app()
            .occurrence_name(occurrence_id)
            .unwrap()
            .trim_end_matches(" #1")
            .to_owned();
        shell.catalog().format(
            "outliner-object",
            &BTreeMap::from([
                ("name", name),
                ("dimensions", "100 × 60 × 20".to_owned()),
                ("visibility", "◉".to_owned()),
            ]),
        )
    };
    let first_row = row_label(&shell, OccurrenceId(1));
    let second_row = row_label(&shell, OccurrenceId(2));
    assert!(shell.app_mut().headless_select_occurrence(OccurrenceId(1)));
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Rotate);
    shell.focus_text_input(&shell.catalog().text("value-label-angle"));
    shell.type_text("45");
    shell.press_key(Key::Enter);
    let first_rotation = shell.app().canonical_digest();
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);

    shell.click_row(&second_row);
    shell.click_row(&first_row);
    shell.click_command(AppCommand::Rotate);
    shell.focus_text_input(&shell.catalog().text("value-label-angle"));
    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().undo_step_count(),
        initial_undo_steps + 2,
        "leaving and restoring the selection must end correction of the old Rotate"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_rotation);
}

#[test]
fn undo_and_redo_return_the_document_to_identical_canonical_states() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let composed = shell.app().canonical_digest();
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();
    let copied = shell.app().canonical_digest();

    shell.key(Key::Z, ctrl());
    let undone_once = shell.app().canonical_digest();
    shell.key(Key::Y, ctrl());
    let redone_once = shell.app().canonical_digest();

    shell.key(Key::Z, ctrl());
    let undone_twice = shell.app().canonical_digest();
    shell.key(Key::Y, ctrl());
    let redone_twice = shell.app().canonical_digest();

    assert_eq!(
        undone_once, composed,
        "Undo must restore the previous state"
    );
    assert_eq!(redone_once, copied, "Redo must restore the copied state");
    assert_eq!(undone_once, undone_twice, "Undo must be reproducible");
    assert_eq!(redone_once, redone_twice, "Redo must be reproducible");
    assert_ne!(undone_once, redone_once);
}

#[test]
fn escape_ends_numeric_move_correction_before_reselecting_the_same_occurrence() {
    let mut shell = Shell::new();
    let initial_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let initial_undo_steps = shell.app().undo_step_count();
    shell.click_at(shell.viewport_rect().center());
    shell.click_command(AppCommand::Move);
    shell.type_text("25,0,0");
    shell.press_key(Key::Enter);
    let first_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    assert_eq!(first_origin, initial_origin + Vec3::new(25.0, 0.0, 0.0));
    let first_undo_steps = shell.app().undo_step_count();
    assert_eq!(first_undo_steps, initial_undo_steps + 1);

    shell.press_key(Key::Escape);
    assert_eq!(shell.app().selected_occurrence_count(), 0);
    assert!(shell.app_mut().headless_select_occurrence(OccurrenceId(1)));
    assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
    shell.click_command(AppCommand::Move);
    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        first_origin + Vec3::new(40.0, 0.0, 0.0),
        "a value entered after Escape must start a fresh Move"
    );
    assert_eq!(shell.app().undo_step_count(), first_undo_steps + 1);
}

#[test]
fn escape_ends_numeric_rotate_correction_before_reselecting_the_same_occurrence() {
    let mut shell = Shell::new();
    let initial_undo_steps = shell.app().undo_step_count();
    shell.click_at(shell.top_face_centre(1));
    shell.click_command(AppCommand::Rotate);
    shell.type_text("45");
    shell.press_key(Key::Enter);
    let first_rotation = shell.app().canonical_digest();
    let first_undo_steps = shell.app().undo_step_count();
    assert_eq!(first_undo_steps, initial_undo_steps + 1);

    shell.press_key(Key::Escape);
    assert_eq!(shell.app().selected_occurrence_count(), 0);
    assert!(shell.app_mut().headless_select_occurrence(OccurrenceId(1)));
    assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
    shell.click_command(AppCommand::Rotate);
    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().undo_step_count(),
        first_undo_steps + 1,
        "a value entered after Escape must start a fresh Rotate"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_rotation);
}

#[test]
fn a_viewport_drag_in_move_commits_exactly_one_canonical_batch() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(rect.center());
    shell.click_command(AppCommand::Move);

    let before = shell.app().document_revision();
    let from = rect.center();
    shell.drag(from, from + eframe::egui::Vec2::new(120.0, 0.0));

    assert_eq!(
        shell.app().document_revision(),
        before + 1,
        "one completed drag must produce exactly one canonical revision"
    );
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "Move must not create geometry"
    );
}

#[test]
fn move_drag_preserves_submillimetre_pointer_motion_between_inference_targets() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let grabbed = origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z);
    let delta = Vec3::new(7.5, 3.25, 0.0);

    shell.click_command(AppCommand::Move);
    shell.drag(
        shell.app().project_to_screen(grabbed, rect),
        shell.app().project_to_screen(grabbed + delta, rect),
    );

    let moved = shell.app().occurrence_box_geometry(1).unwrap().0;
    assert!(
        (moved.x - origin.x - delta.x).abs() < 1.0e-4,
        "continuous X delta was {:?}; expected {:?}",
        moved.x - origin.x,
        delta.x
    );
    assert!(
        (moved.y - origin.y - delta.y).abs() < 1.0e-4,
        "continuous Y delta was {:?}; expected {:?}",
        moved.y - origin.y,
        delta.y
    );
    assert!((moved.z - origin.z).abs() < 1.0e-4);
}

#[test]
fn move_drag_snaps_the_grabbed_corner_to_the_world_origin() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let (grabbed, from) = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(size.x, 0.0, 0.0),
        Vec3::new(0.0, size.y, 0.0),
        Vec3::new(size.x, size.y, 0.0),
        Vec3::new(0.0, 0.0, size.z),
        Vec3::new(size.x, 0.0, size.z),
        Vec3::new(0.0, size.y, size.z),
        Vec3::new(size.x, size.y, size.z),
    ]
    .into_iter()
    .find_map(|corner| {
        let world = origin + corner;
        let screen = shell.app().project_to_screen(world, rect);
        shell.move_pointer(screen);
        (shell.app().hovered_snap_position() == Some(world)).then_some((world, screen))
    })
    .expect("one box corner must be available as the Move anchor");

    shell.click_command(AppCommand::Move);
    shell.drag(from, shell.app().project_to_screen(Vec3::ZERO, rect));

    let moved_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    assert!((moved_origin - (origin - grabbed)).length() < 1.0e-4);
}

#[test]
fn a_viewport_drag_in_rotate_turns_the_occurrence_and_the_arrow_keys_pick_the_axis() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(shell.top_face_centre(1));
    let (origin, size) = shell
        .app()
        .occurrence_box_geometry(1)
        .expect("the default scene has one box");
    // The default box is 100 x 60, so a quarter turn about the blue axis has to
    // swap those two extents. Reading the extents proves the body really turned
    // instead of merely reporting that it did.
    assert_eq!((size.x, size.y), (100.0, 60.0));
    let selected_outline_segments: usize = shell
        .output_shapes()
        .iter()
        .map(|clipped| count_selection_outline_segments(&clipped.shape))
        .sum();
    assert!(
        selected_outline_segments > 0,
        "the test must first observe the selected body's outline"
    );
    let centre = Vec3::new(
        origin.x + size.x * 0.5,
        origin.y + size.y * 0.5,
        origin.z + size.z,
    );

    shell.click_command(AppCommand::Rotate);
    let rotation_rings: usize = shell
        .output_shapes()
        .iter()
        .map(|clipped| count_rotation_rings(&clipped.shape))
        .sum();
    let selection_outline_segments: usize = shell
        .output_shapes()
        .iter()
        .map(|clipped| count_selection_outline_segments(&clipped.shape))
        .sum();
    assert_eq!(
        rotation_rings, 1,
        "Rotate must retain exactly one circular degree guide"
    );
    assert_eq!(
        selection_outline_segments, 0,
        "Rotate must not paint the selected body's bounding outline"
    );
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    // Choose the pivot, then grab an arm and swing it a quarter turn.
    shell.click_at(shell.app().project_to_screen(centre, rect));
    let from = shell.app().project_to_screen(
        Vec3::new(centre.x + size.x * 0.25, centre.y, centre.z),
        rect,
    );
    let to = shell.app().project_to_screen(
        Vec3::new(centre.x, centre.y + size.x * 0.25, centre.z),
        rect,
    );
    shell.drag(from, to);

    assert_eq!(
        shell.app().document_revision(),
        before_revision + 1,
        "one completed Rotate drag must produce exactly one canonical revision: {:?}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "Rotate must not create geometry"
    );
    let turned = shell.app().occurrence_box_geometry(1).unwrap().1;
    assert_eq!(
        (turned.x, turned.y),
        (60.0, 100.0),
        "a quarter turn about the blue axis must swap the width and the depth"
    );

    shell.key(Key::Z, ctrl());
    assert_eq!(
        shell.app().canonical_digest(),
        before_digest,
        "one Rotate drag must be exactly one undo step"
    );

    // The arrow keys pin the protractor to a coloured axis, the way SketchUp
    // does, and pressing the same arrow again releases the pin.
    shell.click_command(AppCommand::Rotate);
    shell.press_key(Key::ArrowRight);
    let before_steps = shell.app().undo_step_count();
    shell.type_text("90");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "a typed angle must commit exactly one undo step: {:?}",
        shell.app().action_digest()
    );
    let about_x = shell.app().occurrence_box_geometry(1).unwrap().1;
    assert_eq!(
        rounded_extents(about_x),
        (100.0, 20.0, 60.0),
        "a locked red axis must turn the depth into the height"
    );

    // Pressing the same arrow again releases the pin, so the protractor falls
    // back to blue Z and a quarter turn swaps the width and the depth instead.
    shell.press_key(Key::ArrowRight);
    let before_steps = shell.app().undo_step_count();
    shell.type_text("90");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);
    let released = shell.app().occurrence_box_geometry(1).unwrap().1;
    assert_eq!(
        rounded_extents(released),
        (20.0, 100.0, 60.0),
        "pressing the locked arrow again must release the lock back to blue Z"
    );
}

#[test]
fn rotate_ctrl_after_the_pivot_switches_the_live_gesture_to_copy() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(shell.top_face_centre(1));
    let source = shell.app().occurrence_box_geometry(1).unwrap();
    let centre = source.0 + source.1 * 0.5;

    shell.click_command(AppCommand::Rotate);
    shell.click_at(shell.app().project_to_screen(centre, rect));
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let from = shell
        .app()
        .project_to_screen(centre + Vec3::new(source.1.x * 0.25, 0.0, 0.0), rect);
    let to = shell
        .app()
        .project_to_screen(centre + Vec3::new(0.0, source.1.x * 0.25, 0.0), rect);

    shell.drag_with(from, to, ctrl());

    let copied_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().definition_count(), 1);
    assert_eq!(shell.app().occurrence_box_geometry(1), Some(source));
    assert_eq!(
        rounded_extents(shell.app().occurrence_box_geometry(2).unwrap().1),
        (60.0, 100.0, 20.0)
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 1);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), copied_digest);
    assert_eq!(shell.app().active_box_count(), 2);
}

#[test]
fn typed_angle_corrects_the_last_rotate_copy_in_one_undo_step() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(shell.top_face_centre(1));
    let source = shell.app().occurrence_box_geometry(1).unwrap();
    let centre = source.0 + source.1 * 0.5;
    let turned_extents = |degrees: f64| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let round = |value: f64| (value * 1_000.0).round() / 1_000.0;
        (
            round(source.1.x.mul_add(cos.abs(), source.1.y * sin.abs())),
            round(source.1.x.mul_add(sin.abs(), source.1.y * cos.abs())),
        )
    };
    let before_digest = shell.app().canonical_digest();
    let before_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Rotate);
    shell.click_at(shell.app().project_to_screen(centre, rect));
    let from = shell
        .app()
        .project_to_screen(centre + Vec3::new(source.1.x * 0.25, 0.0, 0.0), rect);
    let to = shell
        .app()
        .project_to_screen(centre + Vec3::new(0.0, source.1.x * 0.25, 0.0), rect);
    shell.drag_with(from, to, ctrl());
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);

    shell.type_text("40");
    shell.press_key(Key::Enter);

    let corrected = shell.app().occurrence_box_geometry(2).unwrap().1;
    assert_eq!(
        (
            (corrected.x * 1_000.0).round() / 1_000.0,
            (corrected.y * 1_000.0).round() / 1_000.0,
        ),
        turned_extents(40.0),
        "the typed angle must replace the copy's original turn: {:?}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().occurrence_box_geometry(1), Some(source));
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "Rotate-Copy correction must remain one undo step"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 1);
}

#[test]
fn undo_redo_ends_numeric_move_correction() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let base_digest = shell.app().canonical_digest();
    let initial_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let before_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.type_text("25,0,0");
    shell.press_key(Key::Enter);
    let first_move = shell.app().canonical_digest();
    let first_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    assert_eq!(first_origin, initial_origin + Vec3::new(25.0, 0.0, 0.0));
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), base_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_move);

    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        first_origin + Vec3::new(40.0, 0.0, 0.0),
        "a value entered after Undo/Redo must start a fresh Move"
    );
    assert_eq!(shell.app().undo_step_count(), before_steps + 2);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_move);
}

/// SketchUp treats a value typed straight after a rotation as a correction of
/// that rotation, not as a second one. Choosing 45 and then thinking better of
/// it and typing 40 has to leave the body at 40, not at 85.
#[test]
fn undo_redo_ends_numeric_rotate_correction() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let base_digest = shell.app().canonical_digest();
    let before_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Rotate);
    shell.type_text("45");
    shell.press_key(Key::Enter);
    let first_rotation = shell.app().canonical_digest();
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), base_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_rotation);

    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 2,
        "a value entered after Undo/Redo must start a fresh rotation"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_rotation);
}

#[test]
fn changing_and_releasing_the_move_axis_ends_numeric_correction() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let before_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.type_text("25,0,0");
    shell.press_key(Key::Enter);
    let first_move = shell.app().canonical_digest();
    let first_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);

    shell.press_key(Key::ArrowRight);
    shell.press_key(Key::ArrowRight);
    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        first_origin + Vec3::new(40.0, 0.0, 0.0),
        "changing the Move axis and releasing it must start a fresh move"
    );
    assert_eq!(shell.app().undo_step_count(), before_steps + 2);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_move);
}

#[test]
fn changing_and_releasing_the_rotate_axis_ends_numeric_correction() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let before_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Rotate);
    shell.type_text("45");
    shell.press_key(Key::Enter);
    let first_rotation = shell.app().canonical_digest();
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);

    shell.press_key(Key::ArrowRight);
    shell.press_key(Key::ArrowRight);
    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 2,
        "changing the Rotate axis and releasing it must end correction of the old turn"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_rotation);
}

#[test]
fn switching_away_from_move_ends_numeric_correction() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let base_digest = shell.app().canonical_digest();
    let initial_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let before_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.type_text("25,0,0");
    shell.press_key(Key::Enter);
    let first_move = shell.app().canonical_digest();
    let first_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    assert_eq!(first_origin, initial_origin + Vec3::new(25.0, 0.0, 0.0));
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);

    shell.click_command(AppCommand::Select);
    shell.click_command(AppCommand::Move);
    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        first_origin + Vec3::new(40.0, 0.0, 0.0),
        "returning to Move after another tool must start a fresh move"
    );
    assert_eq!(shell.app().undo_step_count(), before_steps + 2);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_move);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), base_digest);
}

#[test]
fn switching_away_from_rotate_ends_numeric_correction() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let base_digest = shell.app().canonical_digest();
    let before_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Rotate);
    shell.type_text("45");
    shell.press_key(Key::Enter);
    let first_rotation = shell.app().canonical_digest();
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);

    shell.click_command(AppCommand::Select);
    shell.click_command(AppCommand::Rotate);
    shell.type_text("40");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 2,
        "returning to Rotate after another tool must start a fresh turn"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_rotation);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), base_digest);
}

#[test]
fn a_typed_angle_corrects_the_last_rotate_instead_of_stacking_onto_it() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let size = shell
        .app()
        .occurrence_box_geometry(1)
        .expect("the default scene has one box")
        .1;
    assert_eq!((size.x, size.y), (100.0, 60.0));
    // The axis-aligned extents of a box turned about the blue axis, which is
    // what reading the geometry back can prove.
    let turned_extents = |degrees: f64| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let round = |value: f64| (value * 1_000.0).round() / 1_000.0;
        (
            round(size.x.mul_add(cos.abs(), size.y * sin.abs())),
            round(size.x.mul_add(sin.abs(), size.y * cos.abs())),
        )
    };
    let base_digest = shell.app().canonical_digest();

    shell.click_command(AppCommand::Rotate);
    let before_steps = shell.app().undo_step_count();
    shell.type_text("45");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);
    let first = shell.app().occurrence_box_geometry(1).unwrap().1;
    assert_eq!(
        (
            (first.x * 1_000.0).round() / 1_000.0,
            (first.y * 1_000.0).round() / 1_000.0
        ),
        turned_extents(45.0)
    );

    // Thinking better of it: 40 replaces the 45, so the extents are those of a
    // single 40° turn and the history still holds exactly one step.
    shell.type_text("40");
    shell.press_key(Key::Enter);
    let corrected = shell.app().occurrence_box_geometry(1).unwrap().1;
    assert_eq!(
        (
            (corrected.x * 1_000.0).round() / 1_000.0,
            (corrected.y * 1_000.0).round() / 1_000.0
        ),
        turned_extents(40.0),
        "a typed angle must replace the last rotation, not add to it: {:?}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "corrections must stay one undo step"
    );

    // Zero is a legitimate correction back to where the body started.
    shell.type_text("0");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().canonical_digest(),
        base_digest,
        "typing zero must undo the turn rather than refuse the value"
    );
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), base_digest);
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps,
        "the whole correction chain must undo in a single step"
    );
}

#[test]
fn a_pinned_move_travels_along_the_blue_axis_and_the_arrow_releases_the_pin() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(shell.top_face_centre(1));
    let (origin, size) = shell
        .app()
        .occurrence_box_geometry(1)
        .expect("the default scene has one box");
    assert_eq!(origin.z, 0.0);
    let top_centre = Vec3::new(
        origin.x + size.x * 0.5,
        origin.y + size.y * 0.5,
        origin.z + size.z,
    );

    // Without a pin the pointer stays on the plane it grabbed, which is exactly
    // why a part could never be set down on top of another one.
    shell.click_command(AppCommand::Move);
    assert!(!shell.app().move_copy_mode_active());
    let before_digest = shell.app().canonical_digest();
    let from = shell.app().project_to_screen(top_centre, rect);
    let lifted = Vec3::new(top_centre.x, top_centre.y, top_centre.z + 40.0);
    shell.drag(from, shell.app().project_to_screen(lifted, rect));
    assert!(!shell.app().move_copy_mode_active());
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0.z,
        0.0,
        "an unpinned Move must keep its own plane: {:?}",
        shell.app().action_digest()
    );
    if shell.app().canonical_digest() != before_digest {
        shell.key(Key::Z, ctrl());
    }
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(!shell.app().move_copy_mode_active());

    // The up arrow pins the travel to blue Z, and then the very same drag lifts
    // the body by the full distance it was dragged.
    shell.press_key(Key::ArrowUp);
    assert!(!shell.app().move_copy_mode_active());
    let before_steps = shell.app().undo_step_count();
    let from = shell.app().project_to_screen(top_centre, rect);
    shell.drag(from, shell.app().project_to_screen(lifted, rect));
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "one pinned drag must be exactly one undo step: {:?}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "Move must not create geometry: {}",
        shell.app().action_digest()
    );
    let raised = shell.app().occurrence_box_geometry(1).unwrap().0;
    assert_eq!(
        (raised.x, raised.y),
        (origin.x, origin.y),
        "a Move pinned to blue Z must travel only along it"
    );
    assert!(
        (raised.z - 40.0).abs() < 1.0e-4,
        "continuous blue-axis travel was {} mm",
        raised.z
    );

    // The pin outlives the gesture, so a typed distance is read along the same
    // axis instead of needing the whole vector spelled out.
    shell.click_command(AppCommand::Move);
    let anchor = Vec3::new(top_centre.x, top_centre.y, raised.z + size.z);
    shell.click_at(shell.app().project_to_screen(anchor, rect));
    let before_steps = shell.app().undo_step_count();
    shell.type_text("25");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "a typed distance along the pinned axis must commit: {:?}",
        shell.app().action_digest()
    );
    let typed_origin_z = shell.app().occurrence_box_geometry(1).unwrap().0.z;
    assert!(
        (typed_origin_z - 65.0).abs() < 1.0e-4,
        "a typed distance must be read along the pinned axis; origin Z was {typed_origin_z}"
    );

    // Pressing the same arrow again releases the pin, and an exact vector can
    // still reach the blue axis on its own.
    shell.press_key(Key::ArrowUp);
    let before_steps = shell.app().undo_step_count();
    shell.type_text("0,0,-15");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);
    assert!(
        (shell.app().occurrence_box_geometry(1).unwrap().0.z - 50.0).abs() < 1.0e-4,
        "an exact x,y,z vector must move in Z with no pin at all"
    );
}

#[test]
fn uniform_scale_drag_grows_the_part_about_its_centre_in_one_undo_step() {
    let mut shell = scale_shell();
    let rect = shell.viewport_rect();
    shell.click_at(shell.top_face_centre(1));
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let centre_screen = shell.app().project_to_screen(origin + size * 0.5, rect);
    // The top face centre is a grip; off every grip a drag scales evenly
    // about the centre.
    let quarter = Vec3::new(size.x * 0.75, size.y * 0.75, size.z);
    let from = shell.app().project_to_screen(origin + quarter, rect);
    let to = centre_screen + (from - centre_screen) * 1.5;
    let (_, digest, undo, _) = scale_untouched_state(&shell);

    shell.click_menu_command("menu-tools", AppCommand::Scale);
    shell.set_primary_button(from, Modifiers::NONE, true);
    shell.move_pointer(to);
    let factor = shell
        .app()
        .scale_preview_factor()
        .expect("a drag away from the centre previews its factor");
    assert!((factor - 1.5).abs() < 0.05, "{factor}");
    shell.set_primary_button(to, Modifiers::NONE, false);
    shell.settle();

    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "{}",
        shell.app().action_digest()
    );
    assert_ne!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().scale_preview_factor(), None);
    assert_exact_bounds(&mut shell, &[stretched(origin, size, [factor; 3])]);
}

#[test]
fn switching_tools_during_a_scale_drag_leaves_the_document_unchanged() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(shell.top_face_centre(1));
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let centre_screen = shell.app().project_to_screen(origin + size * 0.5, rect);
    let from = shell.top_face_centre(1);
    let to = centre_screen + (from - centre_screen) * 1.5;
    let before = scale_untouched_state(&shell);

    shell.click_menu_command("menu-tools", AppCommand::Scale);
    shell.set_primary_button(from, Modifiers::NONE, true);
    shell.move_pointer(to);
    assert!(shell.app().scale_preview_factor().is_some());
    assert_eq!(
        scale_untouched_state(&shell),
        before,
        "a preview is no edit"
    );

    shell.click_command(AppCommand::Move);
    shell.set_primary_button(to, Modifiers::NONE, false);
    shell.settle();

    assert_eq!(shell.app().scale_preview_factor(), None);
    assert!(!shell.app().transform_gesture_active());
    assert_eq!(scale_untouched_state(&shell), before);
}

#[test]
fn scale_arrow_locks_stretch_the_part_along_that_axis_only() {
    let mut shell = scale_shell();
    shell.click_at(shell.top_face_centre(1));
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let (_, digest, undo, _) = scale_untouched_state(&shell);

    for (key, axis) in [(Key::ArrowRight, 0), (Key::ArrowLeft, 1), (Key::ArrowUp, 2)] {
        shell.click_menu_command("menu-tools", AppCommand::Scale);
        shell.press_key(key);
        enter_value(&mut shell, "2");
        assert_eq!(
            shell.app().undo_step_count(),
            undo + 1,
            "{key:?}: {}",
            shell.app().action_digest()
        );
        let mut factors = [1.0; 3];
        factors[axis] = 2.0;
        assert_exact_bounds(&mut shell, &[stretched(origin, size, factors)]);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest, "{key:?}");
        shell.press_key(key);
    }
}

#[test]
fn a_typed_scale_factor_grows_the_part_and_a_second_one_replaces_it() {
    let mut shell = scale_shell();
    shell.click_at(shell.top_face_centre(1));
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let (_, digest, undo, _) = scale_untouched_state(&shell);

    shell.click_menu_command("menu-tools", AppCommand::Scale);
    // A Slovak decimal comma (GX-15).
    enter_value(&mut shell, "1,5");
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "{}",
        shell.app().action_digest()
    );
    assert_exact_bounds(&mut shell, &[stretched(origin, size, [1.5; 3])]);

    enter_value(&mut shell, "2");
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "a second factor replaces the first: {}",
        shell.app().action_digest()
    );
    assert_exact_bounds(&mut shell, &[stretched(origin, size, [2.0; 3])]);
    let scaled = shell.app().canonical_digest();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_exact_bounds(&mut shell, &[stretched(origin, size, [1.0; 3])]);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), scaled);
    assert_exact_bounds(&mut shell, &[stretched(origin, size, [2.0; 3])]);
}

#[test]
fn a_refused_scale_correction_keeps_the_correction_and_one_takes_the_scale_back() {
    let mut shell = scale_shell();
    shell.click_at(shell.top_face_centre(1));
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let (_, _, undo, _) = scale_untouched_state(&shell);

    shell.click_menu_command("menu-tools", AppCommand::Scale);
    enter_value(&mut shell, "2");
    assert_exact_bounds(&mut shell, &[stretched(origin, size, [2.0; 3])]);
    enter_value(&mut shell, "0");
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("digest-scale-invalid-factor")
    );
    // The refused value leaves ×2 open to correction: 3 replaces it.
    enter_value(&mut shell, "3");
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "{}",
        shell.app().action_digest()
    );
    assert_exact_bounds(&mut shell, &[stretched(origin, size, [3.0; 3])]);
    // 1 corrects the step back to the part's own size and stays open.
    enter_value(&mut shell, "1");
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "{}",
        shell.app().action_digest()
    );
    assert_exact_bounds(&mut shell, &[stretched(origin, size, [1.0; 3])]);
    enter_value(&mut shell, "1,5");
    assert_eq!(shell.app().undo_step_count(), undo + 1);
    assert_exact_bounds(&mut shell, &[stretched(origin, size, [1.5; 3])]);
}

#[test]
fn a_typed_scale_factor_of_zero_or_less_or_not_finite_is_refused() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let before = scale_untouched_state(&shell);

    shell.click_menu_command("menu-tools", AppCommand::Scale);
    // Letters would be tool shortcuts, so no infinity can be typed here.
    for text in ["0", "-2", "0.0", "1001"] {
        enter_value(&mut shell, text);
        assert_scale_refused(&shell, "digest-scale-invalid-factor", &before);
    }
}

#[test]
fn copies_sharing_a_definition_refuse_scale_until_made_unique() {
    let mut shell = scale_shell();
    shell.click_at(shell.top_face_centre(1));
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    shell.click_at_with(shell.top_face_centre(1), shift());
    assert_eq!(shell.app().selected_occurrence_count(), 2);
    let first = shell.app().occurrence_box_geometry(1).unwrap();
    let second = shell.app().occurrence_box_geometry(2).unwrap();
    let before = scale_untouched_state(&shell);

    shell.click_menu_command("menu-tools", AppCommand::Scale);
    enter_value(&mut shell, "1.5");
    assert_scale_refused(&shell, "digest-scale-refused-shared", &before);
    assert_eq!(shell.app().occurrence_box_geometry(1).unwrap(), first);
    assert_eq!(shell.app().occurrence_box_geometry(2).unwrap(), second);
    assert_eq!(shell.app().selected_occurrence_count(), 2);

    assert!(shell.app_mut().headless_select_occurrence(OccurrenceId(2)));
    assert!(
        shell.app_mut().make_unique(),
        "{}",
        shell.app().action_digest()
    );
    shell.settle();
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    shell.click_command(AppCommand::Select);
    shell.click_at(shell.top_face_centre(1));
    shell.click_at_with(shell.top_face_centre(2), shift());
    assert_eq!(shell.app().selected_occurrence_count(), 2);
    shell.click_menu_command("menu-tools", AppCommand::Scale);
    let undo = shell.app().undo_step_count();
    enter_value(&mut shell, "1.5");
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "{}",
        shell.app().action_digest()
    );
    let corner = |pick: fn(f64, f64) -> f64, left: Vec3, right: Vec3| {
        Vec3::new(
            pick(left.x, right.x),
            pick(left.y, right.y),
            pick(left.z, right.z),
        )
    };
    let low = corner(f64::min, first.0, second.0);
    let high = corner(f64::max, first.0 + first.1, second.0 + second.1);
    let centre = (low + high) * 0.5;
    // Exact bounds are in each part's own frame, and the copy's placement
    // moves it by `offset` from the original's.
    let offset = second.0 - first.0;
    assert_exact_bounds(
        &mut shell,
        &[
            stretched_about(first.0, first.1, centre, [1.5; 3]),
            stretched_about(first.0, second.1, centre - offset, [1.5; 3]),
        ],
    );
}

#[test]
fn solid_union_previews_and_ctrl_keeps_the_tool_in_one_undo_step() {
    let mut shell = Shell::new();
    hold_exact_bodies_back(&mut shell);
    assert!(shell.app_mut().create_box());
    assert!(shell.app_mut().move_selected(Vec3::new(0.0, -35.0, 0.0)));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-model", AppCommand::SolidUnion);
    let target = shell
        .app()
        .viewport_position(Vec3::new(10.0, 30.0, 20.0))
        .unwrap();
    let tool = shell
        .app()
        .viewport_position(Vec3::new(125.0, 30.0, 20.0))
        .unwrap();
    shell.click_at(target);
    shell.click_at_with(tool, ctrl());
    assert!(shell.app().has_occurrence_operation_preview());
    // The exact bodies arrive while the preview is open; it must stay.
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 2);

    assert!(shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1))
            .unwrap()
            .1,
        Vec3::new(135.0, 60.0, 20.0)
    );

    shell.press_key(Key::Enter);
    let union_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 2, "Ctrl must keep the tool");
    assert_ne!(union_digest, before_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), union_digest);
}

#[test]
fn solid_intersect_previews_exact_overlap_and_commits_in_one_undo_step() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-model", AppCommand::SolidIntersect);
    let target = shell
        .app()
        .viewport_position(Vec3::new(10.0, 10.0, 20.0))
        .unwrap();
    let tool = shell
        .app()
        .viewport_position(Vec3::new(120.0, 50.0, 20.0))
        .unwrap();
    shell.click_at(target);
    shell.click_at(tool);

    assert!(
        shell.app().has_occurrence_operation_preview(),
        "Intersect preview failed: {}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(35.0, 35.0, 0.0), Vec3::new(65.0, 25.0, 20.0)))
    );
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_BREP_GRAPH_EVALUATOR_V1)
    );

    shell.press_key(Key::Enter);
    let intersect_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 1);
    assert_ne!(intersect_digest, before_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), intersect_digest);
    assert_eq!(shell.app().active_box_count(), 1);
}

#[test]
fn solid_intersect_rejects_one_same_or_disjoint_bodies_without_mutation() {
    let mut shell = Shell::new();
    assert!(!shell.app().command_is_enabled(AppCommand::SolidIntersect));

    assert!(shell.app_mut().create_box());
    assert!(shell.app_mut().move_selected(Vec3::new(400.0, 0.0, 0.0)));
    shell.settle();
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    assert!(shell.app().command_is_enabled(AppCommand::SolidIntersect));
    let before = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );

    shell.click_menu_command("menu-model", AppCommand::SolidIntersect);
    let target = shell.top_face_centre(1);
    shell.click_at(target);
    shell.click_at(target);
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        before
    );

    shell.press_key(Key::Escape);
    shell.click_menu_command("menu-model", AppCommand::SolidIntersect);
    shell.click_at(shell.top_face_centre(1));
    shell.click_at(shell.top_face_centre(2));
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        before
    );
}

#[test]
fn d_profile_solid_intersect_preserves_review_lifecycle_exact_mesh_and_arc_lineage() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("d-profile-intersect.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    let start = Vec3::new(120.0, 20.0, 0.0);
    let end = Vec3::new(140.0, 20.0, 0.0);
    let bulge = Vec3::new(130.0, 30.0, 0.0);
    shell.click_command(AppCommand::Arc);
    shell.click_at(shell.app().viewport_position(start).unwrap());
    shell.click_at(shell.app().viewport_position(end).unwrap());
    shell.click_at(shell.app().viewport_position(bulge).unwrap());
    shell.click_command(AppCommand::PushPull);
    push_pull_typed_depth(&mut shell, "20");
    assert!(shell.app_mut().move_selected(Vec3::new(-100.0, 0.0, 0.0)));
    shell.settle();
    assert_eq!(shell.app().active_box_count(), 2);
    wait_for_exact_bodies(&mut shell, 2);

    let before_revision = shell.app().document_revision();
    let before_undo = shell.app().undo_step_count();
    let before_digest = shell.app().canonical_digest();

    let preview_intersection = |shell: &mut Shell| {
        shell.click_menu_command("menu-model", AppCommand::SolidIntersect);
        let target = shell
            .app()
            .viewport_position(Vec3::new(10.0, 10.0, 20.0))
            .unwrap();
        let tool = shell
            .app()
            .viewport_position(Vec3::new(30.0, 25.0, 20.0))
            .unwrap();
        shell.click_at(target);
        shell.move_pointer(tool);
        for _ in 0..4 {
            if shell.app().hovered_selection().is_some_and(|selection| {
                selection.instance_path == InstancePath::root(OccurrenceId(2))
            }) {
                break;
            }
            shell.press_key(Key::Tab);
        }
        assert_eq!(
            shell
                .app()
                .hovered_selection()
                .map(|selection| selection.instance_path.clone()),
            Some(InstancePath::root(OccurrenceId(2)))
        );
        shell.click_at(tool);
        assert!(
            shell.app().has_occurrence_operation_preview(),
            "D-profile Intersect preview failed: {}",
            shell.app().action_digest()
        );
        assert_eq!(
            shell.app().push_pull_preview_exact_evaluator(),
            Some(EXACT_BREP_GRAPH_EVALUATOR_V1)
        );
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1))
            .unwrap();
        assert!(
            origin.x > 0.0
                && origin.y > 0.0
                && origin.z.abs() < 2.0e-5
                && size.x > 19.9
                && size.y > 9.9
                && (size.z - 20.0).abs() < 2.0e-5
                && origin.x + size.x < 100.0
                && origin.y + size.y < 60.0,
            "D-profile preview must remain strictly contained: origin={origin:?}, size={size:?}"
        );
    };

    preview_intersection(&mut shell);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo);

    preview_intersection(&mut shell);
    shell.press_key(Key::Enter);
    let intersect_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_undo + 1);
    assert_eq!(shell.app().active_box_count(), 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let snapshot = shell.app().document_snapshot();
    let producer = *snapshot
        .definition(result_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, result_definition, producer).unwrap();
    let (tool_profile, tool_geometry) =
        boolean_tool_profile(&graph, ExactBRepBooleanOperation::Intersect).unwrap();
    assert!(is_line_arc_profile(tool_geometry));
    assert!(
        graph
            .profiles
            .iter()
            .any(|profile| FeatureId(profile.source_feature_id) != tool_profile)
    );
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let direct_package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(direct_package.vertices.len() > 8);
    assert!(
        direct_package
            .topological_references
            .iter()
            .any(|reference| reference.producer_feature_id == producer)
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    let wait_for_intersect = |shell: &mut Shell| {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            shell.settle();
            if shell.app().exact_render_body_count() == 1 {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            shell.app().exact_render_body_count(),
            1,
            "exact Intersect result failed: {}",
            shell.app().action_digest()
        );
    };
    wait_for_intersect(&mut shell);
    assert!(
        shell.app().exact_render_triangle_count() > 12,
        "the accepted Intersect mesh must preserve the curved arc side"
    );
    assert!(shell.app().exact_current_producer_ids().contains(&producer));

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), intersect_digest);
    assert_eq!(shell.app().active_box_count(), 1);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), intersect_digest);
    let reopened_snapshot = shell.app().document_snapshot();
    let reopened_producer = *reopened_snapshot
        .definition(result_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened_snapshot, result_definition, reopened_producer)
            .unwrap();
    assert!(
        boolean_tool_profile(&reopened_graph, ExactBRepBooleanOperation::Intersect)
            .is_some_and(|(_, geometry)| is_line_arc_profile(geometry))
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_intersect(&mut shell);
    assert!(
        shell
            .app()
            .exact_current_producer_ids()
            .contains(&reopened_producer)
    );
}

#[test]
fn d_profile_solid_union_preserves_review_lifecycle_exact_mesh_and_arc_lineage() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("d-profile-union.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    let start = Vec3::new(-10.0, -20.0, 0.0);
    let end = Vec3::new(-10.0, 80.0, 0.0);
    let bulge = Vec3::new(120.0, 30.0, 0.0);
    shell.click_command(AppCommand::Arc);
    shell.click_at(shell.app().viewport_position(start).unwrap());
    shell.click_at(shell.app().viewport_position(end).unwrap());
    shell.click_at(shell.app().viewport_position(bulge).unwrap());
    shell.click_command(AppCommand::PushPull);
    push_pull_typed_depth(&mut shell, "20");
    assert_eq!(shell.app().active_box_count(), 2);

    let before_revision = shell.app().document_revision();
    let before_undo = shell.app().undo_step_count();
    let before_digest = shell.app().canonical_digest();

    let preview_union = |shell: &mut Shell| {
        shell.click_menu_command("menu-model", AppCommand::SolidUnion);
        let target = shell
            .app()
            .viewport_position(Vec3::new(10.0, 30.0, 20.0))
            .unwrap();
        shell.move_pointer(target);
        for _ in 0..4 {
            if shell.app().hovered_selection().is_some_and(|selection| {
                selection.instance_path == InstancePath::root(OccurrenceId(1))
            }) {
                break;
            }
            shell.press_key(Key::Tab);
        }
        assert_eq!(
            shell
                .app()
                .hovered_selection()
                .map(|selection| selection.instance_path.clone()),
            Some(InstancePath::root(OccurrenceId(1)))
        );
        shell.click_at(target);

        let tool = shell
            .app()
            .viewport_position(Vec3::new(115.0, 30.0, 20.0))
            .unwrap();
        shell.move_pointer(tool);
        for _ in 0..4 {
            if shell.app().hovered_selection().is_some_and(|selection| {
                selection.instance_path == InstancePath::root(OccurrenceId(2))
            }) {
                break;
            }
            shell.press_key(Key::Tab);
        }
        assert_eq!(
            shell
                .app()
                .hovered_selection()
                .map(|selection| selection.instance_path.clone()),
            Some(InstancePath::root(OccurrenceId(2)))
        );
        shell.click_at(tool);
        assert!(
            shell.app().has_occurrence_operation_preview(),
            "D-profile Union preview failed: {}",
            shell.app().action_digest()
        );
        assert_eq!(
            shell.app().push_pull_preview_exact_evaluator(),
            Some(EXACT_BREP_GRAPH_EVALUATOR_V1)
        );
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1))
            .unwrap();
        assert!(
            origin.x < -9.9
                && origin.y < -40.0
                && origin.z.abs() < 2.0e-5
                && size.x > 129.0
                && size.y > 145.0
                && (size.z - 20.0).abs() < 2.0e-5,
            "D-profile preview must strictly contain the host: origin={origin:?}, size={size:?}"
        );
    };

    preview_union(&mut shell);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo);

    preview_union(&mut shell);
    shell.press_key(Key::Enter);
    let union_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_undo + 1);
    assert_eq!(shell.app().active_box_count(), 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let snapshot = shell.app().document_snapshot();
    let producer = *snapshot
        .definition(result_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, result_definition, producer).unwrap();
    let (_, tool_geometry) =
        boolean_tool_profile(&graph, ExactBRepBooleanOperation::Union).unwrap();
    assert!(is_line_arc_profile(tool_geometry));
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let direct_package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(direct_package.vertices.len() > 8);
    assert!(
        direct_package
            .topological_references
            .iter()
            .any(|reference| reference.producer_feature_id == producer)
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    let wait_for_union = |shell: &mut Shell| {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            shell.settle();
            if shell.app().exact_render_body_count() == 1 {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            shell.app().exact_render_body_count(),
            1,
            "exact Union result failed: {}",
            shell.app().action_digest()
        );
    };
    wait_for_union(&mut shell);
    assert!(shell.app().exact_render_triangle_count() > 12);
    assert!(shell.app().exact_current_producer_ids().contains(&producer));

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), union_digest);
    assert_eq!(shell.app().active_box_count(), 1);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), union_digest);
    let reopened_snapshot = shell.app().document_snapshot();
    let reopened_producer = *reopened_snapshot
        .definition(result_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened_snapshot, result_definition, reopened_producer)
            .unwrap();
    assert!(
        boolean_tool_profile(&reopened_graph, ExactBRepBooleanOperation::Union)
            .is_some_and(|(_, geometry)| is_line_arc_profile(geometry))
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_union(&mut shell);
    assert!(
        shell
            .app()
            .exact_current_producer_ids()
            .contains(&reopened_producer)
    );
}

#[test]
fn d_profile_solid_split_preserves_review_lifecycle_partition_mesh_and_arc_lineage() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("d-profile-split.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    let start = Vec3::new(120.0, 20.0, 0.0);
    let end = Vec3::new(140.0, 20.0, 0.0);
    let bulge = Vec3::new(130.0, 30.0, 0.0);
    shell.click_command(AppCommand::Arc);
    shell.click_at(shell.app().viewport_position(start).unwrap());
    shell.click_at(shell.app().viewport_position(end).unwrap());
    shell.click_at(shell.app().viewport_position(bulge).unwrap());
    shell.click_command(AppCommand::PushPull);
    push_pull_typed_depth(&mut shell, "20");
    assert!(shell.app_mut().move_selected(Vec3::new(-100.0, 0.0, 0.0)));
    shell.settle();

    let before_revision = shell.app().document_revision();
    let before_undo = shell.app().undo_step_count();
    let before_digest = shell.app().canonical_digest();
    let preview_split = |shell: &mut Shell| {
        shell.click_menu_command("menu-model", AppCommand::SolidSplit);
        let target = shell
            .app()
            .viewport_position(Vec3::new(10.0, 10.0, 20.0))
            .unwrap();
        let tool = shell
            .app()
            .viewport_position(Vec3::new(30.0, 25.0, 20.0))
            .unwrap();
        shell.click_at(target);
        shell.move_pointer(tool);
        for _ in 0..4 {
            if shell.app().hovered_selection().is_some_and(|selection| {
                selection.instance_path == InstancePath::root(OccurrenceId(2))
            }) {
                break;
            }
            shell.press_key(Key::Tab);
        }
        shell.click_at(tool);
        assert!(
            shell.app().has_occurrence_operation_preview(),
            "D-profile Split preview failed: {}",
            shell.app().action_digest()
        );
        assert_eq!(
            shell.app().push_pull_preview_exact_evaluator(),
            Some(EXACT_BREP_GRAPH_EVALUATOR_V1)
        );
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1))
            .unwrap();
        assert!(
            origin.length() < 2.0e-5
                && (size.x - 100.0).abs() < 2.0e-5
                && (size.y - 60.0).abs() < 2.0e-5
                && (size.z - 20.0).abs() < 2.0e-5
        );
    };

    preview_split(&mut shell);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    preview_split(&mut shell);
    shell.press_key(Key::Enter);
    let split_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_undo + 1);
    assert_eq!(shell.app().active_box_count(), 2);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let snapshot = shell.app().document_snapshot();
    let producer = *snapshot
        .definition(result_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, result_definition, producer).unwrap();
    let (_, tool_geometry) =
        boolean_tool_profile(&graph, ExactBRepBooleanOperation::Split).unwrap();
    assert!(is_line_arc_profile(tool_geometry));
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let direct_package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(direct_package.vertices.len() > 16);
    assert!(
        direct_package
            .topological_references
            .iter()
            .any(|reference| reference.producer_feature_id == producer)
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let wait_for_result = |shell: &mut Shell, expected_producer: FeatureId| {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            shell.settle();
            if shell
                .app()
                .exact_current_producer_ids()
                .contains(&expected_producer)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "exact D-profile Split result failed: {}",
            shell.app().action_digest()
        );
    };
    wait_for_result(&mut shell, producer);
    assert!(shell.app().exact_render_triangle_count() > 24);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), split_digest);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), split_digest);
    let reopened_snapshot = shell.app().document_snapshot();
    let reopened_producer = *reopened_snapshot
        .definition(result_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened_snapshot, result_definition, reopened_producer)
            .unwrap();
    assert!(
        boolean_tool_profile(&reopened_graph, ExactBRepBooleanOperation::Split)
            .is_some_and(|(_, geometry)| is_line_arc_profile(geometry))
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_result(&mut shell, reopened_producer);
}

#[test]
fn solid_split_previews_target_partition_and_commits_from_localized_headless_shell() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    assert!(shell.app_mut().create_box());
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let target_geometry = shell.app().occurrence_box_geometry(1).unwrap();

    shell.click_menu_command("menu-model", AppCommand::SolidSplit);
    let target = shell
        .app()
        .viewport_position(Vec3::new(10.0, 10.0, 20.0))
        .unwrap();
    let splitter = shell
        .app()
        .viewport_position(Vec3::new(120.0, 50.0, 20.0))
        .unwrap();
    shell.click_at(target);
    shell.click_at(splitter);

    assert!(
        shell.app().has_occurrence_operation_preview(),
        "Split preview failed: {}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some(target_geometry)
    );
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_BREP_GRAPH_EVALUATOR_V1)
    );

    shell.press_key(Key::Enter);
    let split_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(
        shell.app().active_box_count(),
        2,
        "Split must preserve the splitter"
    );
    assert_ne!(split_digest, before_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), split_digest);
    assert_eq!(shell.app().active_box_count(), 2);
}

#[test]
fn solid_subtract_previews_consumes_the_tool_and_undo_restores_both_solids() {
    let mut shell = Shell::new();
    let rectangle_start = shell
        .app()
        .viewport_position(Vec3::new(115.0, 20.0, 0.0))
        .unwrap();
    shell.click_command(AppCommand::Rectangle);
    shell.click_at(rectangle_start);
    shell.type_text("30,20");
    shell.press_key(Key::Enter);
    shell.click_command(AppCommand::PushPull);
    push_pull_typed_depth(&mut shell, "20");
    assert!(shell.app_mut().move_selected(Vec3::new(-95.0, -5.0, 0.0)));
    shell.settle();
    assert_eq!(shell.app().occurrence_box_geometry(2).unwrap().1.z, 20.0);
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-model", AppCommand::SolidSubtract);
    let target = shell
        .app()
        .viewport_position(Vec3::new(80.0, 45.0, 20.0))
        .unwrap();
    let tool = shell
        .app()
        .viewport_position(Vec3::new(35.0, 25.0, 20.0))
        .unwrap();
    shell.click_at(target);
    shell.move_pointer(tool);
    if shell
        .app()
        .hovered_selection()
        .is_none_or(|selection| selection.instance_path != InstancePath::root(OccurrenceId(2)))
    {
        shell.press_key(Key::Tab);
    }
    assert_eq!(
        shell
            .app()
            .hovered_selection()
            .map(|selection| selection.instance_path.clone()),
        Some(InstancePath::root(OccurrenceId(2)))
    );
    shell.click_at(tool);

    assert!(
        shell.app().has_occurrence_operation_preview(),
        "Subtract preview failed: {}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.press_key(Key::Enter);

    let subtract_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 1);
    assert_ne!(subtract_digest, before_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), subtract_digest);
    assert_eq!(shell.app().active_box_count(), 1);
}

#[test]
fn solid_trim_subtracts_the_overlap_and_keeps_the_cutter() {
    let mut shell = Shell::new();
    let rectangle_start = shell
        .app()
        .viewport_position(Vec3::new(115.0, 20.0, 0.0))
        .unwrap();
    shell.click_command(AppCommand::Rectangle);
    shell.click_at(rectangle_start);
    shell.type_text("30,20");
    shell.press_key(Key::Enter);
    shell.click_command(AppCommand::PushPull);
    push_pull_typed_depth(&mut shell, "20");
    assert!(shell.app_mut().move_selected(Vec3::new(-95.0, -5.0, 0.0)));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-model", AppCommand::SolidTrim);
    let target = shell
        .app()
        .viewport_position(Vec3::new(80.0, 45.0, 20.0))
        .unwrap();
    let cutter = shell
        .app()
        .viewport_position(Vec3::new(35.0, 25.0, 20.0))
        .unwrap();
    shell.click_at(target);
    shell.move_pointer(cutter);
    if shell
        .app()
        .hovered_selection()
        .is_none_or(|selection| selection.instance_path != InstancePath::root(OccurrenceId(2)))
    {
        shell.press_key(Key::Tab);
    }
    shell.click_at(cutter);

    assert!(
        shell.app().has_occurrence_operation_preview(),
        "Trim preview failed: {}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.press_key(Key::Enter);

    let trim_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(
        shell.app().active_box_count(),
        2,
        "Trim must keep the cutter"
    );
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("digest-solid-trim-committed")
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), trim_digest);
    assert_eq!(shell.app().active_box_count(), 2);
}

#[test]
fn the_model_menu_offers_make_unique() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());

    shell.open_menu("menu-model");

    assert!(
        shell.offers(AppCommand::MakeUnique),
        "the Model menu must expose Make Unique"
    );
    for command in [
        AppCommand::SolidSubtract,
        AppCommand::SolidTrim,
        AppCommand::SolidUnion,
        AppCommand::SolidIntersect,
        AppCommand::SolidSplit,
    ] {
        assert!(
            shell.offers(command),
            "the Model menu must expose {command:?}"
        );
    }
}

#[test]
fn make_unique_is_localized_exact_selection_bound_and_one_undo_step() {
    for (locale_index, catalog) in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ]
    .into_iter()
    .enumerate()
    {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::MakeUnique),
            shell.catalog().text("model-make-unique")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::MakeUnique));

        shell.click_at(shell.top_face_centre(1));
        assert!(!shell.app().command_is_enabled(AppCommand::MakeUnique));
        let single_revision = shell.app().document_revision();
        let single_digest = shell.app().canonical_digest();
        let single_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::MakeUnique);
        assert_eq!(shell.app().document_revision(), single_revision);
        assert_eq!(shell.app().canonical_digest(), single_digest);
        assert_eq!(shell.app().undo_step_count(), single_undo_steps);
        shell.press_key(Key::Escape);
        shell.click_at(shell.top_face_centre(1));

        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        assert_eq!(shell.app().definition_count(), 1);
        assert!(shell.app().command_is_enabled(AppCommand::MakeUnique));

        shell.click_at_with(shell.top_face_centre(1), shift());
        assert_eq!(
            shell.app().selected_occurrence_count(),
            2,
            "selection={:?}, hover={:?}, status={}",
            shell.app().selected_reference(),
            shell.app().hovered_selection(),
            shell.app().action_digest()
        );
        assert!(!shell.app().command_is_enabled(AppCommand::MakeUnique));
        let multiple_revision = shell.app().document_revision();
        let multiple_digest = shell.app().canonical_digest();
        let multiple_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::MakeUnique);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().document_revision(), multiple_revision);
        assert_eq!(shell.app().canonical_digest(), multiple_digest);
        assert_eq!(shell.app().undo_step_count(), multiple_undo_steps);

        shell.click_at(shell.top_face_centre(2));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(shell.app().command_is_enabled(AppCommand::MakeUnique));
        let source_definition_id = shell
            .app()
            .occurrence_definition_id(OccurrenceId(1))
            .expect("the source occurrence exists");
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(source_definition_id)
        );
        let before_revision = shell.app().document_revision();
        let before_digest = shell.app().canonical_digest();
        let before_undo_steps = shell.app().undo_step_count();
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        if locale_index == 0 {
            shell.click_menu_command("menu-model", AppCommand::MakeUnique);
        } else {
            shell.settle();
            let make_unique = shell.catalog().text("model-make-unique");
            shell.click_role_and_label(Role::Button, &make_unique);
        }

        let unique_digest = shell.app().canonical_digest();
        let unique_definition_id = shell
            .app()
            .occurrence_definition_id(OccurrenceId(2))
            .expect("the unique occurrence exists");
        assert_ne!(unique_definition_id, source_definition_id);
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(1)),
            Some(source_definition_id)
        );
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().active_box_count(), 2);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(shell.app().document_revision(), before_revision + 1);
        assert_eq!(shell.app().undo_step_count(), before_undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-made-unique")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::MakeUnique));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), before_digest);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(source_definition_id)
        );
        assert_eq!(shell.app().undo_step_count(), before_undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), unique_digest);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(unique_definition_id)
        );
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        context_shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(context_shell.app().command_is_enabled(AppCommand::Paste));
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        let selected = context_shell.top_face_centre(2);
        context_shell.double_click_at(selected);
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        assert!(
            !context_shell
                .app()
                .command_is_enabled(AppCommand::MakeUnique)
        );
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        let context_action_digest = context_shell.app().action_digest().to_owned();
        context_shell.click_menu_command("menu-model", AppCommand::MakeUnique);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(context_shell.app().action_digest(), context_action_digest);
        context_shell.press_key(Key::Escape);
        assert_eq!(context_shell.app().edit_context_depth(), 0);
        assert!(context_shell.app().command_is_enabled(AppCommand::Paste));
    }
}
