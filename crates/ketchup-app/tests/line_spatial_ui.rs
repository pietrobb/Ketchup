mod harness;

use eframe::egui::Key;
use harness::Shell;
use ketchup_app::{AppCommand, dialogs::ScriptedFileDialogs};
use ketchup_core::document::{FeatureKind, ProfileSegment};
use ketchup_interaction::{Axis, Vec3};

fn last_line(shell: &Shell) -> [Vec3; 2] {
    let snapshot = shell.app().document_snapshot();
    let feature = snapshot.features().filter(|feature| matches!(
        feature.kind(), FeatureKind::SegmentProfile { closed: false, segments } if segments.len() == 1
    )).last().expect("the UI must create an open line");
    let FeatureKind::SegmentProfile { segments, .. } = feature.kind() else {
        unreachable!()
    };
    let ProfileSegment::Line { start_mm, end_mm } = segments[0] else {
        panic!("expected line")
    };
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.definition_id() == feature.definition_id())
        .unwrap();
    let transform = snapshot
        .world_transform_for_occurrence(occurrence.id())
        .unwrap();
    let m = transform.matrix();
    [start_mm, end_mm].map(|p| {
        Vec3::new(
            m[0] * p[0] + m[1] * p[1] + m[3],
            m[4] * p[0] + m[5] * p[1] + m[7],
            m[8] * p[0] + m[9] * p[1] + m[11],
        )
    })
}

fn assert_point(actual: Vec3, expected: Vec3) {
    let d = actual - expected;
    assert!(
        d.x.abs() < 1.0e-6 && d.y.abs() < 1.0e-6 && d.z.abs() < 1.0e-6,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn snapped_spatial_lines_preserve_xyz_through_click_cancel_history_and_save_open() {
    for (start, end) in [
        (Vec3::ZERO, Vec3::new(0.0, 0.0, 20.0)),
        (Vec3::new(0.0, 0.0, 20.0), Vec3::new(100.0, 60.0, 0.0)),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let saved = directory.path().join("spatial-line.ketchup");
        let dialogs = ScriptedFileDialogs::new()
            .queue_save(&saved)
            .queue_open(&saved)
            .always_discard();
        let mut shell = Shell::with_dialogs(dialogs);
        let start_screen = shell.app().viewport_position(start).unwrap();
        let end_screen = shell.app().viewport_position(end).unwrap();
        let before = shell.app().canonical_digest();
        let undo = shell.app().undo_step_count();
        shell.click_command(AppCommand::Line);
        shell.click_at(start_screen);
        shell.move_pointer(end_screen);
        assert_eq!(shell.app().canonical_digest(), before);
        let delta = end - start;
        let length = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
        let displayed: f64 = shell.app().value_input().parse().unwrap();
        assert!(
            (displayed - length).abs() < 0.01,
            "displayed {displayed}, expected {length}"
        );
        shell.click_at(end_screen);
        let points = last_line(&shell);
        assert_point(points[0], start);
        assert_point(points[1], end);
        assert_eq!(shell.app().undo_step_count(), undo + 1);
        let after = shell.app().canonical_digest();
        shell.press_key(Key::Escape);
        assert_eq!(shell.app().canonical_digest(), after);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), before);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), after);
        shell.click_menu_command("menu-file", AppCommand::SaveAs);
        shell.click_menu_command("menu-file", AppCommand::New);
        shell.click_menu_command("menu-file", AppCommand::Open);
        assert_eq!(shell.app().canonical_digest(), after);
        assert_point(last_line(&shell)[0], start);
        assert_point(last_line(&shell)[1], end);
    }
}

#[test]
fn retained_endpoint_is_used_by_line_preview_and_commit() {
    let mut shell = Shell::new();
    let start = Vec3::new(100.0, 0.0, 20.0);
    let end = Vec3::new(0.0, 0.0, 20.0);
    let start_screen = shell.app().viewport_position(start).unwrap();
    let end_screen = shell.app().viewport_position(end).unwrap();
    let before = shell.app().canonical_digest();
    shell.click_command(AppCommand::Line);
    shell.click_at(start_screen);
    shell.move_pointer(end_screen + eframe::egui::Vec2::new(7.0, 0.0));
    shell.move_pointer(end_screen + eframe::egui::Vec2::new(10.0, 0.0));
    assert_eq!(shell.app().hovered_snap_position(), Some(end));
    assert_eq!(shell.app().value_input(), "100");
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_at(end_screen + eframe::egui::Vec2::new(10.0, 0.0));
    assert_point(last_line(&shell)[0], start);
    assert_point(last_line(&shell)[1], end);
    let after = shell.app().canonical_digest();
    shell.press_key(Key::Escape);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), after);
}

#[test]
fn vertical_line_typed_length_follows_the_hovered_direction() {
    let mut shell = Shell::new();
    let start = Vec3::ZERO;
    let end = Vec3::new(0.0, 0.0, 20.0);
    let start_screen = shell.app().viewport_position(start).unwrap();
    let end_screen = shell.app().viewport_position(end).unwrap();
    shell.click_command(AppCommand::Line);
    shell.click_at(start_screen);
    shell.move_pointer(end_screen);
    shell.type_text("35");
    shell.press_key(Key::Enter);
    let points = last_line(&shell);
    assert_point(points[0], start);
    assert_point(points[1], Vec3::new(0.0, 0.0, 35.0));
}

#[test]
fn arrow_locked_line_preview_and_exact_commit_share_the_same_world_axis() {
    for (key, axis, direction) in [
        (Key::ArrowRight, Axis::X, Vec3::new(1.0, 0.0, 0.0)),
        (Key::ArrowLeft, Axis::Y, Vec3::new(0.0, 1.0, 0.0)),
        (Key::ArrowUp, Axis::Z, Vec3::new(0.0, 0.0, 1.0)),
    ] {
        let mut shell = Shell::new();
        let start = Vec3::new(0.0, 0.0, 20.0);
        let hover = start + direction * 50.0;
        let before = shell.app().canonical_digest();
        let undo = shell.app().undo_step_count();

        shell.click_command(AppCommand::Line);
        shell.click_at(shell.app().viewport_position(start).unwrap());
        shell.press_key(key);
        shell.move_pointer(shell.app().viewport_position(hover).unwrap());
        assert_eq!(shell.app().line_axis_lock(), Some(axis));
        assert_eq!(shell.app().value_input(), "50");
        assert!(
            shell
                .app()
                .action_digest()
                .contains(&shell.catalog().text(match axis {
                    Axis::X => "axis-name-x",
                    Axis::Y => "axis-name-y",
                    Axis::Z => "axis-name-z",
                }))
        );

        shell.type_text("35");
        shell.press_key(Key::Enter);
        let points = last_line(&shell);
        assert_point(points[0], start);
        assert_point(points[1], start + direction * 35.0);
        assert_eq!(shell.app().line_axis_lock(), Some(axis));
        assert_eq!(shell.app().undo_step_count(), undo + 1);

        let committed = shell.app().canonical_digest();
        shell.press_key(key);
        assert_eq!(shell.app().line_axis_lock(), None);
        shell.press_key(Key::Escape);
        assert_eq!(shell.app().canonical_digest(), committed);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), before);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), committed);
    }
}
