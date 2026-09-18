mod harness;

use eframe::egui::{Key, Vec2, accesskit::Role};
use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_core::document::FeatureKind;
use ketchup_core::sketch::{PrincipalPlane, SketchEntity};
use ketchup_interaction::{SnapKind, Vec3};

fn rectangle_tool(shell: &mut Shell, plane: PrincipalPlane) {
    shell.click_command(AppCommand::Rectangle);
    let title = shell.catalog().text("face-workflow-title");
    shell.click_role_and_label(Role::Button, &title);
    let key = match plane {
        PrincipalPlane::Xy => "face-workflow-datum-xy",
        PrincipalPlane::Xz => "face-workflow-datum-xz",
        PrincipalPlane::Yz => "face-workflow-datum-yz",
    };
    let label = shell.catalog().text(key);
    shell.click_role_and_label(Role::RadioButton, &label);
}

fn rectangle_points(shell: &Shell) -> Vec<Vec3> {
    let snapshot = shell.app().document_snapshot();
    let feature = snapshot.features().last().unwrap();
    match feature.kind() {
        FeatureKind::Sketch(sketch) => {
            let FeatureKind::Workplane(workplane) =
                snapshot.feature(sketch.workplane).unwrap().kind()
            else {
                panic!("rectangle needs a workplane")
            };
            let f = workplane.frame;
            sketch
                .entities
                .iter()
                .map(|entity| {
                    let SketchEntity::Line { start_mm: p, .. } = entity else {
                        panic!("rectangle edge")
                    };
                    Vec3::new(
                        f.origin_mm[0] + f.x_axis[0] * p[0] + f.y_axis[0] * p[1],
                        f.origin_mm[1] + f.x_axis[1] * p[0] + f.y_axis[1] * p[1],
                        f.origin_mm[2] + f.x_axis[2] * p[0] + f.y_axis[2] * p[1],
                    )
                })
                .collect()
        }
        FeatureKind::Profile { points_mm } => {
            let occurrence = snapshot
                .occurrences()
                .find(|o| o.definition_id() == feature.definition_id())
                .unwrap();
            let t = snapshot
                .world_transform_for_occurrence(occurrence.id())
                .unwrap();
            let m = t.matrix();
            points_mm
                .iter()
                .map(|p| {
                    Vec3::new(
                        m[0] * p[0] + m[1] * p[1] + m[3],
                        m[4] * p[0] + m[5] * p[1] + m[7],
                        m[8] * p[0] + m[9] * p[1] + m[11],
                    )
                })
                .collect()
        }
        other => panic!("expected rectangle, got {other:?}"),
    }
}

fn assert_corner(points: &[Vec3], expected: Vec3) {
    assert!(
        points.iter().any(|p| {
            let d = *p - expected;
            d.x.abs() < 1e-6 && d.y.abs() < 1e-6 && d.z.abs() < 1e-6
        }),
        "missing {expected:?} in {points:?}"
    );
}

#[test]
fn rectangle_endpoint_and_midpoint_snap_match_committed_corners_on_every_datum() {
    for (plane, start, end) in [
        (
            PrincipalPlane::Xy,
            Vec3::new(0.0, 0.0, 20.0),
            Vec3::new(100.0, 30.0, 20.0),
        ),
        (
            PrincipalPlane::Xz,
            Vec3::new(0.0, 0.0, 20.0),
            Vec3::new(100.0, 0.0, 10.0),
        ),
        (
            PrincipalPlane::Yz,
            Vec3::new(0.0, 0.0, 20.0),
            Vec3::new(0.0, 60.0, 10.0),
        ),
    ] {
        let mut shell = Shell::new();
        rectangle_tool(&mut shell, plane);
        let before = shell.app().canonical_digest();
        let undo = shell.app().undo_step_count();
        let start_screen = shell.app().viewport_position(start).unwrap() + Vec2::new(3.0, 1.0);
        let end_screen = shell.app().viewport_position(end).unwrap() + Vec2::new(3.0, 1.0);
        shell.move_pointer(start_screen);
        assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
        assert_eq!(shell.app().hovered_snap_position(), Some(start));
        shell.click_at(start_screen);
        shell.move_pointer(end_screen);
        assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Midpoint));
        assert_eq!(shell.app().hovered_snap_position(), Some(end));
        assert_eq!(shell.app().canonical_digest(), before);
        let dimensions: Vec<f64> = shell
            .app()
            .value_input()
            .split(',')
            .map(|v| v.trim().parse().unwrap())
            .collect();
        let expected = if plane == PrincipalPlane::Xy {
            [100.0, 30.0]
        } else if plane == PrincipalPlane::Xz {
            [100.0, 10.0]
        } else {
            [60.0, 10.0]
        };
        assert_eq!(dimensions, expected, "live preview dimensions in {plane:?}");
        shell.click_at(end_screen);
        let points = rectangle_points(&shell);
        assert_corner(&points, start);
        assert_corner(&points, end);
        assert_eq!(shell.app().undo_step_count(), undo + 1);
        let after = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), before);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), after);
    }
}

#[test]
fn datum_origin_and_endpoint_support_exact_dimensions_and_save_open() {
    use ketchup_app::dialogs::ScriptedFileDialogs;
    for plane in [PrincipalPlane::Xz, PrincipalPlane::Yz] {
        for start in [Vec3::ZERO, Vec3::new(0.0, 0.0, 20.0)] {
            let directory = tempfile::tempdir().unwrap();
            let saved = directory.path().join("datum-rectangle.ketchup");
            let dialogs = ScriptedFileDialogs::new()
                .queue_save(&saved)
                .queue_open(&saved)
                .always_discard();
            let mut shell = Shell::with_dialogs(dialogs);
            rectangle_tool(&mut shell, plane);
            let end = start
                + match plane {
                    PrincipalPlane::Xz => Vec3::new(30.0, 0.0, 15.0),
                    _ => Vec3::new(0.0, 30.0, 15.0),
                };
            let start_screen = shell.app().viewport_position(start).unwrap() + Vec2::new(3.0, 1.0);
            let end_screen = shell.app().viewport_position(end).unwrap();
            shell.click_at(start_screen);
            shell.move_pointer(end_screen);
            shell.type_text("30,15");
            shell.press_key(Key::Enter);
            assert_corner(&rectangle_points(&shell), start);
            assert_corner(&rectangle_points(&shell), end);
            let digest = shell.app().canonical_digest();
            shell.click_menu_command("menu-file", AppCommand::SaveAs);
            shell.click_menu_command("menu-file", AppCommand::New);
            shell.click_menu_command("menu-file", AppCommand::Open);
            assert_eq!(shell.app().canonical_digest(), digest);
            assert_corner(&rectangle_points(&shell), start);
            assert_corner(&rectangle_points(&shell), end);
        }
    }
}

#[test]
fn rectangle_snaps_toggle_and_cancel_do_not_change_the_document() {
    for plane in [PrincipalPlane::Xz, PrincipalPlane::Yz] {
        let mut shell = Shell::new();
        rectangle_tool(&mut shell, plane);
        let before = shell.app().canonical_digest();
        let snaps = shell.catalog().text("face-workflow-snaps");
        shell.click_role_and_label(Role::CheckBox, &snaps);
        let point = shell
            .app()
            .viewport_position(Vec3::new(0.0, 0.0, 20.0))
            .unwrap()
            + Vec2::new(3.0, 1.0);
        shell.move_pointer(point);
        assert_eq!(shell.app().hovered_snap_kind(), None);
        shell.click_at(point);
        shell.move_pointer(point + Vec2::new(30.0, -20.0));
        shell.press_key(Key::Escape);
        assert_eq!(shell.app().canonical_digest(), before);
    }
}

#[test]
fn rectangle_does_not_advertise_out_of_plane_endpoint_as_a_snap() {
    for (plane, point) in [
        (PrincipalPlane::Xy, Vec3::new(100.0, 60.0, 0.0)),
        (PrincipalPlane::Xz, Vec3::new(100.0, 60.0, 20.0)),
        (PrincipalPlane::Yz, Vec3::new(100.0, 60.0, 20.0)),
    ] {
        let mut shell = Shell::new();
        rectangle_tool(&mut shell, plane);
        let anchor = shell
            .app()
            .viewport_position(Vec3::new(0.0, 0.0, 20.0))
            .unwrap();
        shell.click_at(anchor);
        let screen = shell.app().viewport_position(point).unwrap() + Vec2::new(3.0, 1.0);
        shell.move_pointer(screen);
        assert_eq!(
            shell.app().hovered_snap_position(),
            None,
            "out of {plane:?}"
        );
    }
}
