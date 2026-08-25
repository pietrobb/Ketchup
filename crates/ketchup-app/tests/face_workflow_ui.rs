//! Program 5 face-driven Rectangle to Smart Push/Pull replayed offscreen.

mod harness;

use eframe::egui::{Key, Vec2, accesskit::Role};
use harness::{Shell, alt};
use ketchup_app::{AppCommand, HeadlessFaceWorkflowFailure, dialogs::ScriptedFileDialogs};
use ketchup_core::document::{FeatureId, FeatureKind, InstancePath, OccurrenceId, ProfileSegment};
use ketchup_core::exact_product::{
    EXACT_ARC_PROFILE_EVALUATOR_V1, EXACT_CIRCLE_EVALUATOR_V1, EXACT_LINEAR_PROFILE_EVALUATOR_V1,
    EXACT_POCKET_EVALUATOR_V1, EXACT_THROUGH_CUT_EVALUATOR_V1, ExactFeatureChainRequest,
};
use ketchup_core::sketch::{PrincipalPlane, WorkplaneSupport};
use ketchup_interaction::{SnapKind, Vec3};
use std::collections::BTreeMap;

fn open_face_workflow(shell: &mut Shell) {
    shell.click_command(AppCommand::Rectangle);
    let title = shell.catalog().text("face-workflow-title");
    shell.click_role_and_label(Role::Button, &title);
}

#[test]
fn localized_serial_rectangle_to_hover_bound_push_pull_uses_the_viewport_value_box() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("face-workflow.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell =
        Shell::with_catalog_and_dialogs(ketchup_interaction::LocaleCatalog::slovak(), dialogs);
    open_face_workflow(&mut shell);
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let initial_undo = shell.app().undo_step_count();

    let xz = shell.catalog().text("face-workflow-datum-xz");
    shell.click_role_and_label(Role::RadioButton, &xz);
    assert_eq!(shell.app().face_workflow_datum(), PrincipalPlane::Xz);
    let xy = shell.catalog().text("face-workflow-datum-xy");
    shell.click_role_and_label(Role::RadioButton, &xy);
    assert_eq!(shell.app().face_workflow_datum(), PrincipalPlane::Xy);
    shell.click_role_and_label(Role::RadioButton, &xz);
    assert_eq!(shell.app().face_workflow_datum(), PrincipalPlane::Xz);
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert_eq!(shell.app().undo_step_count(), initial_undo);

    let snaps = shell.catalog().text("face-workflow-snaps");
    shell.click_role_and_label(Role::CheckBox, &snaps);
    assert!(!shell.app().face_workflow_snaps_enabled());
    let endpoint = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();
    shell.move_pointer(endpoint + Vec2::new(3.0, 0.0));
    assert_eq!(shell.app().hovered_snap_kind(), None);
    shell.click_role_and_label(Role::CheckBox, &snaps);
    shell.move_pointer(endpoint + Vec2::new(3.0, 0.0));
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));

    let anchor = shell
        .app()
        .viewport_position(Vec3::new(120.0, 0.0, 10.0))
        .unwrap();
    shell.click_command(AppCommand::Rectangle);
    shell.click_at(anchor);
    shell.move_pointer(anchor + Vec2::new(25.0, -15.0));
    assert_eq!(shell.app().document_revision(), initial_revision);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert_eq!(shell.app().undo_step_count(), initial_undo);

    shell.click_command(AppCommand::Rectangle);
    shell.click_at(anchor);
    shell.type_text("30,20");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().undo_step_count(), initial_undo + 1);

    let snapshot = shell.app().document_snapshot();
    let FeatureKind::Workplane(workplane) = snapshot.feature(FeatureId(3)).unwrap().kind() else {
        panic!("XZ Rectangle must create an explicit workplane");
    };
    assert_eq!(
        workplane.support,
        WorkplaneSupport::Principal(PrincipalPlane::Xz)
    );
    assert!(matches!(
        snapshot.feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Sketch(_)
    ));

    let top = shell.top_face_centre(1);
    shell.click_command(AppCommand::Select);
    shell.click_at(top);
    let selected = shell.app().selected_reference().unwrap();
    assert_eq!(selected.instance_path, InstancePath::root(OccurrenceId(1)));
    let selection_anchor = shell.viewport_rect().left_top() + Vec2::new(24.0, 200.0);
    shell.move_pointer(selection_anchor);
    assert!(shell.app().hovered_selection().is_none());
    shell.press_key(Key::P);
    assert_eq!(shell.app().selected_reference(), Some(selected.clone()));
    assert_ne!(
        shell.app().face_workflow_target_feedback(),
        shell.catalog().text("hover-none")
    );

    shell.click_at(selection_anchor);
    assert!(shell.app().push_pull_click_anchor_active());
    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().undo_step_count(), initial_undo + 1);

    let lifted = selection_anchor
        + (shell
            .app()
            .viewport_position(Vec3::new(50.0, 30.0, 30.0))
            .unwrap()
            - top);
    shell.move_pointer(lifted);
    assert!(shell.app().preview_action_digest().is_some());
    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().undo_step_count(), initial_undo + 1);
    shell.click_at(lifted);
    assert!(!shell.app().push_pull_click_anchor_active());
    assert_eq!(shell.app().document_revision(), initial_revision + 2);
    assert_eq!(shell.app().undo_step_count(), initial_undo + 2);
    assert_eq!(shell.app().document_height_mm(), 30.0);

    let pointer_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().document_height_mm(), 20.0);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), pointer_digest);
    assert_eq!(shell.app().document_height_mm(), 30.0);

    shell.type_text("5");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_height_mm(), 25.0);
    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().document_height_mm(), 25.0);
    assert!(!shell.app().can_undo());
}

#[test]
fn press_drag_release_preserves_live_preview_and_commits_one_reviewed_step() {
    let mut shell = Shell::new();
    let top = shell.top_face_centre(1);
    let projected_up = shell
        .app()
        .viewport_position(Vec3::new(50.0, 30.0, 21.0))
        .unwrap();
    let drag_direction = (projected_up - top).normalized();
    shell.move_pointer(top);
    shell.press_key(Key::P);

    let before = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    );
    let mut saw_preview = false;
    shell.drag_observing(top, top + drag_direction * 120.0, |app| {
        saw_preview |= app.preview_action_digest().is_some();
    });

    assert!(
        saw_preview,
        "the held drag must publish an observational preview"
    );
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_ne!(shell.app().canonical_digest(), before.1);
    assert_eq!(shell.app().undo_step_count(), before.2 + 1);
    assert_eq!(shell.app().redo_step_count(), before.3);
    assert!(shell.app().document_height_mm() > 20.0);
    assert!(!shell.app().push_pull_click_anchor_active());
}

#[test]
fn stale_anchor_hidden_selection_and_invalid_extent_fail_closed() {
    let mut shell = Shell::new();
    let top = shell.top_face_centre(1);
    shell.move_pointer(top);
    shell.press_key(Key::P);
    shell.click_at(top);
    assert!(shell.app().push_pull_click_anchor_active());

    assert!(shell.app_mut().move_selected(Vec3::new(1.0, 0.0, 0.0)));
    let after_intervening_edit = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    );
    shell.click_at(top);
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
        ),
        after_intervening_edit
    );
    shell.press_key(Key::Escape);
    assert!(!shell.app().push_pull_click_anchor_active());

    shell.click_menu_command("menu-view", AppCommand::Hide);
    let hidden = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    );
    shell.press_key(Key::P);
    shell.type_text("5");
    shell.press_key(Key::Enter);
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
        ),
        hidden
    );

    shell.click_menu_command("menu-view", AppCommand::Unhide);
    let before_invalid = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    );
    shell.press_key(Key::P);
    shell.type_text("-20");
    shell.press_key(Key::Enter);
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
        ),
        before_invalid
    );
    assert_eq!(shell.app().document_height_mm(), 20.0);
}

#[test]
fn failed_ambiguous_and_lost_ui_paths_preserve_history_and_last_valid_exact_output() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .seed_headless_face_workflow_last_valid_output()
        .unwrap();
    let top = shell.top_face_centre(1);
    let before = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
        shell.app().headless_face_workflow_exact_output_stamp(),
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
        shell.app().exact_render_bounds(),
        shell.app().exact_render_triangle_count(),
        shell.app().document_height_mm(),
    );
    assert_eq!(shell.app().exact_render_body_count(), 1);

    for failure in [
        HeadlessFaceWorkflowFailure::FailedEvaluation,
        HeadlessFaceWorkflowFailure::Ambiguous,
        HeadlessFaceWorkflowFailure::Lost,
    ] {
        shell.move_pointer(top);
        shell.press_key(Key::P);
        shell.app_mut().arm_headless_face_workflow_failure(failure);
        shell.type_text("5");
        shell.press_key(Key::Enter);

        assert_eq!(
            (
                shell.app().document_revision(),
                shell.app().canonical_digest(),
                shell.app().undo_step_count(),
                shell.app().redo_step_count(),
                shell.app().headless_face_workflow_exact_output_stamp(),
                shell
                    .app()
                    .headless_face_workflow_exact_output_fingerprints(),
                shell.app().exact_render_bounds(),
                shell.app().exact_render_triangle_count(),
                shell.app().document_height_mm(),
            ),
            before,
            "{failure:?} must fail closed without replacing last-valid output"
        );
        assert!(!shell.app().push_pull_click_anchor_active());
        assert!(shell.app().preview_action_digest().is_none());
    }
}

#[test]
fn deliberate_alt_pick_through_has_transient_xray_feedback_without_mutation() {
    let mut shell = Shell::new();
    let centre = shell.viewport_rect().center();
    shell.click_at(centre);
    assert!(shell.app_mut().copy_selected(Vec3::new(100.0, 0.0, 0.0)));
    assert!(shell.app_mut().move_selected(Vec3::new(-100.0, 0.0, 0.0)));
    shell.settle();
    shell.move_pointer(centre);
    assert_eq!(shell.app().hovered_overlap_choice(), Some((0, 2)));

    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo = shell.app().undo_step_count();
    shell.key(Key::Tab, alt());

    assert_eq!(shell.app().hovered_overlap_choice(), Some((1, 2)));
    assert!(shell.app().face_workflow_xray_active());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo);

    shell.press_key(Key::Escape);
    assert!(!shell.app().face_workflow_xray_active());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo);
}

#[test]
fn line_click_preview_exact_length_cancel_undo_and_save_open_are_canonical() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("line-workflow.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let start = shell
        .app()
        .viewport_position(Vec3::new(25.0, 20.0, 20.0))
        .unwrap();
    let direction = shell
        .app()
        .viewport_position(Vec3::new(45.0, 30.0, 20.0))
        .unwrap();
    let before = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );

    shell.click_command(AppCommand::Line);
    shell.click_at(start);
    shell.move_pointer(direction);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), before.1);
    assert_eq!(shell.app().undo_step_count(), before.2);
    assert!(!shell.app().value_input().is_empty());

    shell.press_key(Key::Escape);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), before.1);
    assert_eq!(shell.app().undo_step_count(), before.2);

    shell.click_command(AppCommand::Line);
    shell.click_at(start);
    shell.click_at(start);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), before.1);
    assert_eq!(shell.app().undo_step_count(), before.2);
    shell.move_pointer(direction);
    shell.type_text("15");
    shell.press_key(Key::Enter);

    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.2 + 1);
    let snapshot = shell.app().document_snapshot();
    let (line_definition_id, line_start, line_end) = snapshot
        .features()
        .filter_map(|feature| {
            let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
                return None;
            };
            if *closed || segments.len() != 1 {
                return None;
            }
            let ProfileSegment::Line { start_mm, end_mm } = &segments[0] else {
                return None;
            };
            Some((feature.definition_id(), *start_mm, *end_mm))
        })
        .last()
        .expect("Line must create one open canonical segment profile");
    assert_eq!(line_start, [0.0, 0.0]);
    assert!((line_end[0].hypot(line_end[1]) - 15.0).abs() < 1.0e-9);
    let first_origin = snapshot
        .occurrences()
        .find(|occurrence| occurrence.definition_id() == line_definition_id)
        .unwrap()
        .transform();
    let first_end = Vec3::new(
        first_origin.matrix()[3] + line_end[0],
        first_origin.matrix()[7] + line_end[1],
        first_origin.matrix()[11],
    );
    let second_target = first_end + Vec3::new(12.0, 8.0, 0.0);
    let second_target_screen = shell.app().viewport_position(second_target).unwrap();
    shell.click_at(second_target_screen);
    assert_eq!(shell.app().document_revision(), before.0 + 2);
    assert_eq!(shell.app().undo_step_count(), before.2 + 2);

    let open_chain_digest = shell.app().canonical_digest();
    let snapshot = shell.app().document_snapshot();
    let lines = snapshot
        .features()
        .filter_map(|feature| {
            let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
                return None;
            };
            if *closed || segments.len() != 1 {
                return None;
            }
            let ProfileSegment::Line { start_mm, end_mm } = &segments[0] else {
                return None;
            };
            let origin = snapshot
                .occurrences()
                .find(|occurrence| occurrence.definition_id() == feature.definition_id())?
                .transform();
            Some((
                [
                    origin.matrix()[3] + start_mm[0],
                    origin.matrix()[7] + start_mm[1],
                ],
                [
                    origin.matrix()[3] + end_mm[0],
                    origin.matrix()[7] + end_mm[1],
                ],
            ))
        })
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!((lines[0].1[0] - lines[1].0[0]).abs() < 1.0e-9);
    assert!((lines[0].1[1] - lines[1].0[1]).abs() < 1.0e-9);

    shell.click_at(second_target_screen);
    assert_eq!(shell.app().document_revision(), before.0 + 2);
    assert_eq!(shell.app().canonical_digest(), open_chain_digest);
    assert_eq!(shell.app().undo_step_count(), before.2 + 2);

    shell.click_at(start);
    assert_eq!(shell.app().document_revision(), before.0 + 3);
    assert_eq!(shell.app().undo_step_count(), before.2 + 3);
    let closed_digest = shell.app().canonical_digest();
    let snapshot = shell.app().document_snapshot();
    let (closed_definition_id, closed_segments) = snapshot
        .features()
        .filter_map(|feature| {
            let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
                return None;
            };
            (*closed && segments.len() == 3).then_some((feature.definition_id(), segments))
        })
        .last()
        .expect("returning to the first point must create one closed three-line profile");
    assert!(
        closed_segments
            .iter()
            .all(|segment| matches!(segment, ProfileSegment::Line { .. }))
    );
    assert_eq!(
        snapshot
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::SegmentProfile { closed: false, segments } if segments.len() == 1))
            .count(),
        0
    );
    let closed_origin = snapshot
        .occurrences()
        .find(|occurrence| occurrence.definition_id() == closed_definition_id)
        .unwrap()
        .transform();
    let local_centroid = closed_segments.iter().fold([0.0; 2], |mut sum, segment| {
        let point = segment.start_mm();
        sum[0] += point[0] / closed_segments.len() as f64;
        sum[1] += point[1] / closed_segments.len() as f64;
        sum
    });
    let profile_face = shell
        .app()
        .viewport_position(Vec3::new(
            closed_origin.matrix()[3] + local_centroid[0],
            closed_origin.matrix()[7] + local_centroid[1],
            closed_origin.matrix()[11],
        ))
        .unwrap();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), open_chain_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), closed_digest);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().canonical_digest(), closed_digest);

    shell.click_command(AppCommand::Select);
    shell.move_pointer(profile_face);
    if shell
        .app()
        .hovered_selection()
        .is_some_and(|selection| selection.definition_id != closed_definition_id)
    {
        shell.key(Key::Tab, alt());
    }
    shell.click_at(profile_face);
    assert_eq!(
        shell.app().selected_reference().unwrap().definition_id,
        closed_definition_id
    );
    let before_push = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.app_mut().set_push_pull_distance_input("-20");
    assert!(shell.app_mut().start_preview());
    shell.settle();
    assert!(shell.app().has_smart_push_pull_chooser());
    assert_eq!(shell.app().document_revision(), before_push.0);
    assert_eq!(shell.app().canonical_digest(), before_push.1);
    assert_eq!(shell.app().undo_step_count(), before_push.2);
    shell.app_mut().cancel_preview();

    shell.app_mut().set_push_pull_distance_input("8");
    assert!(shell.app_mut().start_preview());
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_LINEAR_PROFILE_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), before_push.0);
    assert_eq!(shell.app().canonical_digest(), before_push.1);
    shell.app_mut().cancel_preview();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("8");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before_push.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before_push.2 + 1);
    let pushed_snapshot = shell.app().document_snapshot();
    assert!(
        pushed_snapshot
            .definition(closed_definition_id)
            .unwrap()
            .feature_ids()
            .iter()
            .any(|feature_id| matches!(
                pushed_snapshot.feature(*feature_id).unwrap().kind(),
                FeatureKind::Extrusion { .. }
            ))
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&pushed_snapshot, closed_definition_id)
            .unwrap()
            .evaluator(),
        EXACT_LINEAR_PROFILE_EVALUATOR_V1
    );
    let pushed_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), closed_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), pushed_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), pushed_digest);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(
            &shell.app().document_snapshot(),
            closed_definition_id,
        )
        .unwrap()
        .evaluator(),
        EXACT_LINEAR_PROFILE_EVALUATOR_V1
    );
    assert!(!shell.app().can_undo());
}

#[test]
fn rectangular_line_profile_negative_push_pull_creates_exact_through_cut_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("line-rectangle-cut.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let points = [
        Vec3::new(10.0, 10.0, 20.0),
        Vec3::new(25.0, 10.0, 20.0),
        Vec3::new(25.0, 30.0, 20.0),
        Vec3::new(10.0, 30.0, 20.0),
    ]
    .map(|point| shell.app().viewport_position(point).unwrap());

    shell.click_command(AppCommand::Line);
    shell.click_at(points[0]);
    shell.click_at(points[1]);
    shell.click_at(points[2]);
    shell.click_at(points[3]);
    shell.click_at(points[0]);
    let profile_digest = shell.app().canonical_digest();
    let before = (
        shell.app().document_revision(),
        shell.app().undo_step_count(),
    );

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("-20");
    assert!(shell.app_mut().start_preview());
    shell.settle();
    assert!(shell.app().has_smart_push_pull_chooser());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);

    let box_name = shell.catalog().format(
        "model-default-box",
        &BTreeMap::from([("number", "1".to_owned())]),
    );
    let occurrence = shell.catalog().format(
        "model-default-occurrence",
        &BTreeMap::from([("name", box_name)]),
    );
    let target_label = shell.catalog().format(
        "choice-smart-push-pull-cut-target",
        &BTreeMap::from([
            ("feature", shell.catalog().text("model-default-extrusion")),
            ("feature_id", "2".to_owned()),
            ("occurrence", occurrence),
            ("occurrence_id", "1".to_owned()),
        ]),
    );
    assert!(shell.has_role_and_label(Role::RadioButton, &target_label));
    shell.click_role_and_label(Role::RadioButton, &target_label);
    shell.click_role_and_label(
        Role::Button,
        &shell.catalog().text("choice-smart-push-pull-continue"),
    );
    assert!(
        shell.app().has_occurrence_operation_preview(),
        "digest={}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_THROUGH_CUT_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), before.0);
    shell.press_key(Key::Enter);

    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.1 + 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(
            &shell.app().document_snapshot(),
            result_definition
        )
        .unwrap()
        .evaluator(),
        EXACT_THROUGH_CUT_EVALUATOR_V1
    );
    let cut_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), cut_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), cut_digest);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(
            &shell.app().document_snapshot(),
            result_definition
        )
        .unwrap()
        .evaluator(),
        EXACT_THROUGH_CUT_EVALUATOR_V1
    );
}

#[test]
fn triangular_line_profile_negative_push_pull_creates_exact_through_cut_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("line-triangle-cut.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let points = [
        Vec3::new(10.0, 10.0, 20.0),
        Vec3::new(30.0, 12.0, 20.0),
        Vec3::new(18.0, 32.0, 20.0),
    ]
    .map(|point| shell.app().viewport_position(point).unwrap());

    shell.click_command(AppCommand::Line);
    shell.click_at(points[0]);
    shell.click_at(points[1]);
    shell.click_at(points[2]);
    shell.click_at(points[0]);
    let profile_digest = shell.app().canonical_digest();
    let before = (
        shell.app().document_revision(),
        shell.app().undo_step_count(),
    );

    let choose_target = |shell: &mut Shell| {
        assert!(shell.app_mut().start_preview());
        shell.settle();
        assert!(shell.app().has_smart_push_pull_chooser());
        let box_name = shell.catalog().format(
            "model-default-box",
            &BTreeMap::from([("number", "1".to_owned())]),
        );
        let occurrence = shell.catalog().format(
            "model-default-occurrence",
            &BTreeMap::from([("name", box_name)]),
        );
        let target_label = shell.catalog().format(
            "choice-smart-push-pull-cut-target",
            &BTreeMap::from([
                ("feature", shell.catalog().text("model-default-extrusion")),
                ("feature_id", "2".to_owned()),
                ("occurrence", occurrence),
                ("occurrence_id", "1".to_owned()),
            ]),
        );
        shell.click_role_and_label(Role::RadioButton, &target_label);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("choice-smart-push-pull-continue"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(
            shell.app().push_pull_preview_exact_evaluator(),
            Some(EXACT_THROUGH_CUT_EVALUATOR_V1)
        );
    };

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("-20");
    choose_target(&mut shell);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);

    choose_target(&mut shell);
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.1 + 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    assert_eq!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .map(|profile| profile.segments.len()),
        Some(3)
    );
    let cut_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), cut_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), cut_digest);
    let reopened = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(reopened.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    assert_eq!(
        reopened
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .map(|profile| profile.segments.len()),
        Some(3)
    );
}

#[test]
fn semicircular_arc_profile_negative_push_pull_creates_exact_through_cut_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("arc-through-cut.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let start = Vec3::new(20.0, 20.0, 20.0);
    let end = Vec3::new(40.0, 20.0, 20.0);
    let bulge = Vec3::new(30.0, 30.0, 20.0);

    shell.click_command(AppCommand::Arc);
    shell.click_at(shell.app().viewport_position(start).unwrap());
    shell.click_at(shell.app().viewport_position(end).unwrap());
    shell.click_at(shell.app().viewport_position(bulge).unwrap());
    assert_eq!(shell.app().arc_profile_count(), 1);
    let profile_digest = shell.app().canonical_digest();
    let before = (
        shell.app().document_revision(),
        shell.app().undo_step_count(),
    );

    let choose_target = |shell: &mut Shell| {
        assert!(shell.app_mut().start_preview());
        shell.settle();
        assert!(shell.app().has_smart_push_pull_chooser());
        let box_name = shell.catalog().format(
            "model-default-box",
            &BTreeMap::from([("number", "1".to_owned())]),
        );
        let occurrence = shell.catalog().format(
            "model-default-occurrence",
            &BTreeMap::from([("name", box_name)]),
        );
        let target_label = shell.catalog().format(
            "choice-smart-push-pull-cut-target",
            &BTreeMap::from([
                ("feature", shell.catalog().text("model-default-extrusion")),
                ("feature_id", "2".to_owned()),
                ("occurrence", occurrence),
                ("occurrence_id", "1".to_owned()),
            ]),
        );
        shell.click_role_and_label(Role::RadioButton, &target_label);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("choice-smart-push-pull-continue"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(
            shell.app().push_pull_preview_exact_evaluator(),
            Some(EXACT_THROUGH_CUT_EVALUATOR_V1)
        );
    };

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("-20");
    choose_target(&mut shell);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);

    choose_target(&mut shell);
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.1 + 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(profile.is_line_arc_d_profile());
    let cut_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), cut_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), cut_digest);
    let reopened = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert!(
        reopened
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    );
}

#[test]
fn semicircular_arc_profile_negative_push_pull_creates_exact_depth_limited_pocket_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("arc-pocket.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let start = Vec3::new(20.0, 20.0, 20.0);
    let end = Vec3::new(40.0, 20.0, 20.0);
    let bulge = Vec3::new(30.0, 30.0, 20.0);

    shell.click_command(AppCommand::Arc);
    shell.click_at(shell.app().viewport_position(start).unwrap());
    shell.click_at(shell.app().viewport_position(end).unwrap());
    shell.click_at(shell.app().viewport_position(bulge).unwrap());
    assert_eq!(shell.app().arc_profile_count(), 1);
    let profile_digest = shell.app().canonical_digest();
    let before = (
        shell.app().document_revision(),
        shell.app().undo_step_count(),
    );

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("-25");
    assert!(!shell.app_mut().start_preview());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().undo_step_count(), before.1);

    shell.app_mut().set_push_pull_distance_input("-8");
    let choose_target = |shell: &mut Shell| {
        assert!(shell.app_mut().start_preview());
        shell.settle();
        assert!(shell.app().has_smart_push_pull_chooser());
        let box_name = shell.catalog().format(
            "model-default-box",
            &BTreeMap::from([("number", "1".to_owned())]),
        );
        let occurrence = shell.catalog().format(
            "model-default-occurrence",
            &BTreeMap::from([("name", box_name)]),
        );
        let target_label = shell.catalog().format(
            "choice-smart-push-pull-cut-target",
            &BTreeMap::from([
                ("feature", shell.catalog().text("model-default-extrusion")),
                ("feature_id", "2".to_owned()),
                ("occurrence", occurrence),
                ("occurrence_id", "1".to_owned()),
            ]),
        );
        shell.click_role_and_label(Role::RadioButton, &target_label);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("choice-smart-push-pull-continue"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(
            shell.app().push_pull_preview_exact_evaluator(),
            Some(EXACT_POCKET_EVALUATOR_V1)
        );
    };

    choose_target(&mut shell);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);

    choose_target(&mut shell);
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.1 + 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    );
    let pocket_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), pocket_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), pocket_digest);
    let reopened = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(reopened.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(reopened.pocket_depth_bits, Some(8.0_f64.to_bits()));
    assert!(
        reopened
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    );
}

#[test]
fn capsule_profile_negative_push_pull_creates_exact_depth_limited_pocket_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("capsule-pocket.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let segments = vec![
        ProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [20.0, 0.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [20.0, 0.0],
            end_mm: [20.0, 20.0],
            center_mm: [20.0, 10.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [20.0, 20.0],
            end_mm: [0.0, 20.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [0.0, 20.0],
            end_mm: [0.0, 0.0],
            center_mm: [0.0, 10.0],
            clockwise: false,
        },
    ];
    assert!(
        shell
            .app_mut()
            .create_capsule_profile(Vec3::new(30.0, 20.0, 20.0), segments)
    );
    let profile_digest = shell.app().canonical_digest();
    let before = (
        shell.app().document_revision(),
        shell.app().undo_step_count(),
    );

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("-25");
    assert!(!shell.app_mut().start_preview());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().undo_step_count(), before.1);

    shell.app_mut().set_push_pull_distance_input("-8");
    let choose_target = |shell: &mut Shell| {
        assert!(shell.app_mut().start_preview());
        shell.settle();
        assert!(shell.app().has_smart_push_pull_chooser());
        let box_name = shell.catalog().format(
            "model-default-box",
            &BTreeMap::from([("number", "1".to_owned())]),
        );
        let occurrence = shell.catalog().format(
            "model-default-occurrence",
            &BTreeMap::from([("name", box_name)]),
        );
        let target_label = shell.catalog().format(
            "choice-smart-push-pull-cut-target",
            &BTreeMap::from([
                ("feature", shell.catalog().text("model-default-extrusion")),
                ("feature_id", "2".to_owned()),
                ("occurrence", occurrence),
                ("occurrence_id", "1".to_owned()),
            ]),
        );
        shell.click_role_and_label(Role::RadioButton, &target_label);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("choice-smart-push-pull-continue"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(
            shell.app().push_pull_preview_exact_evaluator(),
            Some(EXACT_POCKET_EVALUATOR_V1)
        );
    };

    choose_target(&mut shell);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);

    choose_target(&mut shell);
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.1 + 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_capsule_profile())
    );
    let pocket_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), pocket_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), pocket_digest);
    let reopened = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(reopened.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(reopened.pocket_depth_bits, Some(8.0_f64.to_bits()));
    assert!(
        reopened
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_capsule_profile())
    );
}

#[test]
fn slanted_line_profile_negative_push_pull_creates_exact_depth_limited_pocket_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("line-polygon-pocket.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let points = [
        Vec3::new(12.0, 10.0, 20.0),
        Vec3::new(28.0, 13.0, 20.0),
        Vec3::new(25.0, 31.0, 20.0),
        Vec3::new(9.0, 27.0, 20.0),
    ]
    .map(|point| shell.app().viewport_position(point).unwrap());

    shell.click_command(AppCommand::Line);
    shell.click_at(points[0]);
    shell.click_at(points[1]);
    shell.click_at(points[2]);
    shell.click_at(points[3]);
    shell.click_at(points[0]);
    let profile_digest = shell.app().canonical_digest();
    let before = (
        shell.app().document_revision(),
        shell.app().undo_step_count(),
    );

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("-25");
    assert!(!shell.app_mut().start_preview());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().undo_step_count(), before.1);

    shell.app_mut().set_push_pull_distance_input("-8");
    assert!(shell.app_mut().start_preview());
    shell.settle();
    assert!(shell.app().has_smart_push_pull_chooser());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);

    let box_name = shell.catalog().format(
        "model-default-box",
        &BTreeMap::from([("number", "1".to_owned())]),
    );
    let occurrence = shell.catalog().format(
        "model-default-occurrence",
        &BTreeMap::from([("name", box_name)]),
    );
    let target_label = shell.catalog().format(
        "choice-smart-push-pull-cut-target",
        &BTreeMap::from([
            ("feature", shell.catalog().text("model-default-extrusion")),
            ("feature_id", "2".to_owned()),
            ("occurrence", occurrence),
            ("occurrence_id", "1".to_owned()),
        ]),
    );
    assert!(shell.has_role_and_label(Role::RadioButton, &target_label));
    shell.click_role_and_label(Role::RadioButton, &target_label);
    shell.click_role_and_label(
        Role::Button,
        &shell.catalog().text("choice-smart-push-pull-continue"),
    );
    assert!(
        shell.app().has_occurrence_operation_preview(),
        "digest={}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_POCKET_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.press_key(Key::Enter);

    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.1 + 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let pocket_request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(pocket_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    assert_eq!(
        pocket_request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .map(|profile| profile.segments.len()),
        Some(4)
    );
    let pocket_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), pocket_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), pocket_digest);
    let reopened_request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(reopened_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(reopened_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
}

#[test]
fn circular_profile_negative_push_pull_creates_exact_depth_limited_pocket_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("circle-pocket.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let center = Vec3::new(35.0, 25.0, 20.0);

    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    let profile_digest = shell.app().canonical_digest();
    let before = (
        shell.app().document_revision(),
        shell.app().undo_step_count(),
    );

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("-25");
    assert!(!shell.app_mut().start_preview());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().undo_step_count(), before.1);

    shell.app_mut().set_push_pull_distance_input("-8");
    assert!(shell.app_mut().start_preview());
    shell.settle();
    assert!(shell.app().has_smart_push_pull_chooser());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);

    let box_name = shell.catalog().format(
        "model-default-box",
        &BTreeMap::from([("number", "1".to_owned())]),
    );
    let occurrence = shell.catalog().format(
        "model-default-occurrence",
        &BTreeMap::from([("name", box_name)]),
    );
    let target_label = shell.catalog().format(
        "choice-smart-push-pull-cut-target",
        &BTreeMap::from([
            ("feature", shell.catalog().text("model-default-extrusion")),
            ("feature_id", "2".to_owned()),
            ("occurrence", occurrence),
            ("occurrence_id", "1".to_owned()),
        ]),
    );
    assert!(shell.has_role_and_label(Role::RadioButton, &target_label));
    shell.click_role_and_label(Role::RadioButton, &target_label);
    shell.click_role_and_label(
        Role::Button,
        &shell.catalog().text("choice-smart-push-pull-continue"),
    );
    assert!(
        shell.app().has_occurrence_operation_preview(),
        "digest={}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_POCKET_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.press_key(Key::Enter);

    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.1 + 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let pocket_request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(pocket_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    assert!(
        pocket_request
            .boolean
            .as_ref()
            .is_some_and(|boolean| boolean.circle.is_some())
    );
    let pocket_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), pocket_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), pocket_digest);
    let reopened_request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(reopened_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(reopened_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    assert!(
        reopened_request
            .boolean
            .as_ref()
            .is_some_and(|boolean| boolean.circle.is_some())
    );
}

#[test]
fn circular_profile_positive_push_pull_creates_exact_extrusion_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("circle-extrusion.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let center = Vec3::new(135.0, 25.0, 20.0);

    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    let profile_digest = shell.app().canonical_digest();
    let before = (
        shell.app().document_revision(),
        shell.app().undo_step_count(),
    );

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("0");
    assert!(!shell.app_mut().start_preview());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().undo_step_count(), before.1);

    shell.app_mut().set_push_pull_distance_input("-7");
    assert!(!shell.app_mut().start_preview());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().undo_step_count(), before.1);

    shell.app_mut().set_push_pull_distance_input("7");
    assert!(shell.app_mut().start_preview());
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_CIRCLE_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.app_mut().cancel_preview();
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);

    shell.app_mut().set_push_pull_distance_input("7");
    assert!(shell.app_mut().start_preview());
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.1 + 1);
    let definition_id = shell.app().selected_reference().unwrap().definition_id;
    let request =
        ExactFeatureChainRequest::from_snapshot(&shell.app().document_snapshot(), definition_id)
            .unwrap();
    assert_eq!(request.evaluator(), EXACT_CIRCLE_EVALUATOR_V1);
    assert_eq!(f64::from_bits(request.height_bits), 7.0);
    assert!(request.circle.is_some());

    let extrusion_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), extrusion_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), extrusion_digest);
    let reopened_request =
        ExactFeatureChainRequest::from_snapshot(&shell.app().document_snapshot(), definition_id)
            .unwrap();
    assert_eq!(reopened_request.evaluator(), EXACT_CIRCLE_EVALUATOR_V1);
    assert_eq!(f64::from_bits(reopened_request.height_bits), 7.0);
    assert!(reopened_request.circle.is_some());
}

#[test]
fn semicircular_arc_profile_positive_push_pull_creates_exact_extrusion_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("arc-extrusion.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let start = Vec3::new(135.0, 25.0, 20.0);
    let end = Vec3::new(150.0, 0.0, 20.0);
    let chord = end - start;
    let chord_length = chord.x.hypot(chord.y);
    let midpoint = (start + end) * 0.5;
    let normal = Vec3::new(-chord.y / chord_length, chord.x / chord_length, 0.0);
    let bulge = midpoint + normal * (chord_length * 0.5);

    shell.click_command(AppCommand::Arc);
    shell.click_at(shell.app().viewport_position(start).unwrap());
    shell.click_at(shell.app().viewport_position(end).unwrap());
    shell.click_at(shell.app().viewport_position(bulge).unwrap());
    assert_eq!(shell.app().arc_profile_count(), 1);
    let profile_digest = shell.app().canonical_digest();
    let before = (
        shell.app().document_revision(),
        shell.app().undo_step_count(),
    );

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("0");
    assert!(!shell.app_mut().start_preview());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().undo_step_count(), before.1);

    shell.app_mut().set_push_pull_distance_input("-7");
    assert!(!shell.app_mut().start_preview());
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().undo_step_count(), before.1);

    shell.app_mut().set_push_pull_distance_input("7");
    assert!(shell.app_mut().start_preview());
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_ARC_PROFILE_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.app_mut().cancel_preview();
    assert_eq!(shell.app().document_revision(), before.0);
    assert_eq!(shell.app().canonical_digest(), profile_digest);

    shell.app_mut().set_push_pull_distance_input("7");
    assert!(shell.app_mut().start_preview());
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.1 + 1);
    let definition_id = shell.app().selected_reference().unwrap().definition_id;
    let request =
        ExactFeatureChainRequest::from_snapshot(&shell.app().document_snapshot(), definition_id)
            .unwrap();
    assert_eq!(request.evaluator(), EXACT_ARC_PROFILE_EVALUATOR_V1);
    assert_eq!(f64::from_bits(request.height_bits), 7.0);
    assert!(request.mixed_profile.is_some());

    let extrusion_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), extrusion_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), extrusion_digest);
    let reopened_request =
        ExactFeatureChainRequest::from_snapshot(&shell.app().document_snapshot(), definition_id)
            .unwrap();
    assert_eq!(reopened_request.evaluator(), EXACT_ARC_PROFILE_EVALUATOR_V1);
    assert_eq!(f64::from_bits(reopened_request.height_bits), 7.0);
    assert!(reopened_request.mixed_profile.is_some());
}
