//! Program 5 face-driven Rectangle to Smart Push/Pull replayed offscreen.

mod harness;

use eframe::egui::{Key, Vec2, accesskit::Role};
use harness::{Shell, alt};
use ketchup_app::{AppCommand, HeadlessFaceWorkflowFailure, dialogs::ScriptedFileDialogs};
use ketchup_core::document::{FeatureId, FeatureKind, InstancePath, OccurrenceId};
use ketchup_core::sketch::{PrincipalPlane, WorkplaneSupport};
use ketchup_interaction::{SnapKind, Vec3};

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
