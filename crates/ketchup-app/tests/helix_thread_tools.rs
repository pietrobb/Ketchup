mod harness;

use eframe::egui::Key;
use harness::Shell;
use ketchup_app::{
    AppCommand, AxisSpec, HelixHandedness, HelixToolParameters, ThreadProfile,
    ThreadToolParameters, dialogs::ScriptedFileDialogs,
};
use ketchup_core::document::FeatureKind;
use std::{path::PathBuf, time::Duration};

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

fn wait_for_exact_bodies(shell: &mut Shell, expected: usize) {
    for _ in 0..2000 {
        shell.settle();
        if shell.app().exact_render_body_count() == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(shell.app().exact_render_body_count(), expected);
}

#[test]
fn manual_helix_and_thread_tools_support_arbitrary_axes_undo_and_save_open() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("helix-thread.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-model", AppCommand::Helix);
    assert!(shell.has_visible_label(&shell.catalog().text("helix-panel-title")));
    assert!(shell.has_visible_label(&shell.catalog().text("helix-axis")));
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);

    let helix = HelixToolParameters {
        axis: AxisSpec::TwoPoints {
            start_mm: [4.0, -3.0, 2.0],
            end_mm: [5.0, -1.0, 5.0],
        },
        radius_mm: 7.0,
        pitch_mm: 4.5,
        turns: 2.25,
        start_angle_degrees: 37.0,
        handedness: HelixHandedness::Left,
    };
    let undo_before_helix = shell.app().undo_step_count();
    assert!(
        shell.app_mut().create_helix(helix),
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().undo_step_count(), undo_before_helix + 1);
    let snapshot = shell.app().document_snapshot();
    let path = snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::SpatialPath { segments } => Some(segments),
            _ => None,
        })
        .last()
        .unwrap();
    assert_eq!(path.len(), 9);

    assert!(shell.app_mut().undo());
    assert!(shell.app_mut().redo());

    shell.click_menu_command("menu-model", AppCommand::Thread);
    assert!(shell.has_visible_label(&shell.catalog().text("thread-panel-title")));
    assert!(shell.has_visible_label(&shell.catalog().text("thread-profile")));
    shell.press_key(Key::Escape);

    let thread = ThreadToolParameters {
        helix: HelixToolParameters {
            axis: AxisSpec::OriginDirection {
                origin_mm: [28.0, 0.0, 0.0],
                direction: [0.35, 0.2, 1.0],
            },
            radius_mm: 8.0,
            pitch_mm: 6.0,
            turns: 2.0,
            start_angle_degrees: 15.0,
            handedness: HelixHandedness::Right,
        },
        profile_radius_mm: 0.65,
        profile: ThreadProfile::V,
    };
    let undo_before_thread = shell.app().undo_step_count();
    assert!(shell.app_mut().create_thread(thread));
    assert_eq!(shell.app().undo_step_count(), undo_before_thread + 1);
    assert!(
        shell
            .app()
            .document_snapshot()
            .features()
            .any(|feature| matches!(feature.kind(), FeatureKind::Sweep { .. }))
    );

    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the real exact worker is required");
    wait_for_exact_bodies(&mut shell, 3);

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    wait_for_exact_bodies(&mut shell, 3);
}
