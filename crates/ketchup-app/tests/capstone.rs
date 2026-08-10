//! The capstone dependencies of the manual modeler, replayed offscreen.
//!
//! Measure, visibility, Zoom Fit, and the documented shortcuts are driven
//! through the designed shell exactly as a user drives them, and every
//! assertion reads document or camera state rather than painted text.

mod harness;

use eframe::egui::{Key, Vec2};
use harness::{Shell, ctrl};
use ketchup_app::AppCommand;
use ketchup_interaction::Vec3;

#[test]
fn measuring_two_points_reports_a_distance_and_leaves_the_document_alone() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_command(AppCommand::Measure);
    let before = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    shell.click_at(rect.center() - Vec2::new(120.0, 0.0));
    assert_eq!(
        shell.app().measured_distance_mm(),
        None,
        "one point is not a measurement yet"
    );
    shell.click_at(rect.center() + Vec2::new(120.0, 0.0));

    let measured = shell
        .app()
        .measured_distance_mm()
        .expect("two clicked points must produce a distance");
    assert!(
        measured > 0.0 && measured.is_finite(),
        "the measured distance must be a real length, got {measured}"
    );
    assert_eq!(
        shell.app().document_revision(),
        before,
        "Measure is a reading — it must not commit a canonical batch"
    );
    assert_eq!(
        shell.app().canonical_digest(),
        digest,
        "Measure must not change the document identity"
    );
}

#[test]
fn escape_clears_the_measurement() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_command(AppCommand::Measure);
    shell.click_at(rect.center() - Vec2::new(120.0, 0.0));
    shell.click_at(rect.center() + Vec2::new(120.0, 0.0));
    assert!(shell.app().measured_distance_mm().is_some());

    shell.press_key(Key::Escape);

    assert_eq!(
        shell.app().measured_distance_mm(),
        None,
        "Escape must clear the ephemeral measurement"
    );
}

#[test]
fn hiding_and_unhiding_the_selection_is_one_undo_step_each() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert_eq!(shell.app().hidden_occurrence_count(), 0);

    let before = shell.app().document_revision();
    shell.click_menu_command("menu-view", AppCommand::Hide);

    assert_eq!(
        shell.app().hidden_occurrence_count(),
        1,
        "Hide must hide the selected occurrence"
    );
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "hiding must not delete the occurrence"
    );
    assert_eq!(
        shell.app().document_revision(),
        before + 1,
        "Hide must be exactly one undo step"
    );

    shell.click_menu_command("menu-view", AppCommand::Unhide);

    assert_eq!(
        shell.app().hidden_occurrence_count(),
        0,
        "Unhide restores it"
    );
    assert_eq!(shell.app().document_revision(), before + 2);

    shell.key(Key::Z, ctrl());

    assert_eq!(
        shell.app().hidden_occurrence_count(),
        1,
        "Undo must step back over exactly the Unhide"
    );
}

#[test]
fn zoom_fit_frames_the_whole_model_without_changing_it() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(
        shell
            .app_mut()
            .copy_selected(Vec3::new(100_000.0, 0.0, 0.0))
    );
    shell.settle();

    let zoom_before = shell.app().camera_zoom();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);

    assert!(
        shell.app().camera_zoom() < zoom_before,
        "framing a model wider than the viewport must zoom out, was {zoom_before} and stayed {}",
        shell.app().camera_zoom()
    );
    for occurrence_id in [1, 2] {
        let (origin, size) = shell
            .app()
            .occurrence_box_geometry(occurrence_id)
            .expect("the framed occurrence must exist");
        let centre = Vec3::new(
            origin.x + size.x * 0.5,
            origin.y + size.y * 0.5,
            origin.z + size.z * 0.5,
        );
        let screen = shell.app().project_to_screen(centre, shell.viewport_rect());
        assert!(
            shell.viewport_rect().contains(screen),
            "Zoom Fit must keep occurrence {occurrence_id} visible, projected to {screen:?}"
        );
    }
    assert_eq!(
        shell.app().document_revision(),
        revision,
        "the camera is not part of the document"
    );
    assert_eq!(shell.app().canonical_digest(), digest);
}

#[test]
fn wheel_zoom_can_pull_back_far_beyond_the_old_small_model_limit() {
    let mut shell = Shell::new();
    let viewport_centre = shell.viewport_rect().center();

    for _ in 0..30 {
        shell.scroll_at(viewport_centre, -120.0);
    }

    assert!(
        shell.app().camera_zoom() < 0.1,
        "wheel zoom must not stop at the former 0.8 limit, got {}",
        shell.app().camera_zoom()
    );
    assert!(shell.app().camera_zoom().is_finite());
    assert!(shell.app().camera_zoom() > 0.0);
}

#[test]
fn an_exact_rectangle_becomes_a_solid_through_exact_push_pull() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();

    shell.click_command(AppCommand::Rectangle);
    let before = shell.app().document_revision();
    shell.click_at(rect.center() + Vec2::new(0.0, 120.0));
    shell.type_text("3000,2000");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().active_box_count(),
        2,
        "an exact rectangle must produce a second profile occurrence"
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(2).unwrap().1.z,
        0.0,
        "Rectangle must remain profile-only until Push/Pull"
    );
    assert_eq!(
        shell.app().document_revision(),
        before + 1,
        "the exact rectangle must be exactly one undo step"
    );

    let geometry_before = shell.app().canonical_digest();
    let created = shell.app().document_revision();
    shell.click_at(rect.center());
    shell.click_command(AppCommand::PushPull);
    shell.type_text("500");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().document_revision(),
        created + 1,
        "exact Push/Pull must commit exactly one canonical batch"
    );
    assert_ne!(
        shell.app().canonical_digest(),
        geometry_before,
        "exact Push/Pull must change the extruded geometry"
    );

    shell.key(Key::Z, ctrl());

    assert_eq!(
        shell.app().canonical_digest(),
        geometry_before,
        "Undo must step back over exactly the Push/Pull"
    );
}

#[test]
fn the_shortcut_reference_opens_from_the_help_menu_and_from_f1() {
    let mut shell = Shell::new();
    assert!(!shell.app().shortcuts_visible());

    shell.click_menu_command("menu-help", AppCommand::Shortcuts);

    assert!(
        shell.app().shortcuts_visible(),
        "Help must document the shortcuts without a hidden developer control"
    );

    let close = shell.catalog().text("shortcuts-close");
    shell.click_row(&close);
    assert!(!shell.app().shortcuts_visible());

    shell.press_key(Key::F1);

    assert!(
        shell.app().shortcuts_visible(),
        "the documented F1 shortcut must open the same reference"
    );
}
