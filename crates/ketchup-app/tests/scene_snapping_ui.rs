mod harness;

use eframe::egui::{Key, Vec2};
use harness::Shell;
use ketchup_app::{AppCommand, dialogs::ScriptedFileDialogs};
use ketchup_core::document::FeatureKind;
use ketchup_interaction::{SnapKind, Vec3};

fn last_rectangle(shell: &Shell) -> Vec<Vec3> {
    let snapshot = shell.app().document_snapshot();
    let feature = snapshot.features().last().unwrap();
    let FeatureKind::Profile { points_mm } = feature.kind() else {
        panic!("rectangle profile")
    };
    let occurrence = snapshot
        .scene_query()
        .into_iter()
        .find(|o| o.definition_id == feature.definition_id())
        .unwrap();
    let m = occurrence.transform.matrix();
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

#[test]
fn second_rectangle_snaps_to_own_rectangle_preview_commit_undo_save_open() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("own-rectangle-snap.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .always_discard()
        .queue_save(&saved)
        .queue_open(&saved);
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_command(AppCommand::Rectangle);
    let first = shell
        .app()
        .viewport_position(Vec3::new(25.0, 30.0, 0.0))
        .unwrap();
    let last = shell
        .app()
        .viewport_position(Vec3::new(105.0, 90.0, 0.0))
        .unwrap();
    shell.click_at(first);
    shell.move_pointer(last);
    shell.type_text("80,60");
    shell.press_key(Key::Enter);
    let corners = last_rectangle(&shell);
    let corner = corners[0];
    let midpoint = (corners[1] + corners[2]) * 0.5;
    shell.click_command(AppCommand::Rectangle);
    let before = shell.app().canonical_digest();
    let steps = shell.app().undo_step_count();
    let from = shell.app().viewport_position(corner).unwrap() + Vec2::new(3.0, 1.0);
    let to = shell.app().viewport_position(midpoint).unwrap() + Vec2::new(3.0, 1.0);
    shell.move_pointer(from);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    assert_eq!(shell.app().hovered_snap_position(), Some(corner));
    shell.click_at(from);
    shell.move_pointer(to);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Midpoint));
    assert_eq!(shell.app().hovered_snap_position(), Some(midpoint));
    assert_eq!(shell.app().value_input(), "80,30");
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_at(to);
    let result = last_rectangle(&shell);
    assert!(result.iter().any(|p| p.distance(corner) < 1e-7));
    assert!(result.iter().any(|p| p.distance(midpoint) < 1e-7));
    assert_eq!(shell.app().undo_step_count(), steps + 1);
    let after = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), after);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), after);
}
