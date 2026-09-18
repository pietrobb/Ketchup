mod harness;

use eframe::egui::Key;
use harness::{Shell, ctrl};
use ketchup_app::{AppCommand, dialogs::ScriptedFileDialogs};
use ketchup_core::document::FeatureKind;
use ketchup_interaction::Vec3;
use std::{path::PathBuf, time::Duration};

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

fn replace_text(shell: &mut Shell, label_key: &str, value: &str) {
    let label = shell.catalog().text(label_key);
    shell.focus_text_input(&label);
    shell.key(Key::A, ctrl());
    shell.type_text(value);
}

fn open_from_command_search(shell: &mut Shell, command: AppCommand) {
    let search = shell.catalog().text("command-search");
    let query = shell.app().command_label(command);
    shell.focus_text_input_once(&search);
    shell.type_text_once(&query);
    shell.click_command(command);
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
fn manual_helix_and_thread_tools_use_selected_geometry_form_preview_undo_and_save_open() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("helix-thread.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the real exact worker is required");
    wait_for_exact_bodies(&mut shell, 1);

    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    open_from_command_search(&mut shell, AppCommand::Helix);
    assert!(shell.has_visible_label(&shell.catalog().text("helix-panel-title")));
    assert!(shell.has_visible_label(&shell.catalog().text("helix-selected-edge-missing")));
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);

    let edge_midpoint = shell
        .app()
        .project_to_screen(Vec3::new(50.0, 0.0, 20.0), shell.viewport_rect());
    shell.click_at(edge_midpoint);
    open_from_command_search(&mut shell, AppCommand::Helix);
    assert!(shell.has_visible_label(&shell.catalog().text("helix-selected-edge-ready")));
    let preview_before_axis = shell.render_viewport_pixels();
    shell.click_button_label(&shell.catalog().text("helix-use-selected-edge"));
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("digest-helix-axis-selected")
    );
    replace_text(&mut shell, "helix-radius", "7");
    replace_text(&mut shell, "helix-pitch", "4.5");
    replace_text(&mut shell, "helix-turns", "2.25");
    replace_text(&mut shell, "helix-start-angle", "37");
    let left_handed = format!(
        "{}: {}",
        shell.catalog().text("helix-handedness"),
        shell.catalog().text("helix-left-handed")
    );
    shell.click_button_label(&left_handed);
    let preview_after_parameters = shell.render_viewport_pixels();
    assert_ne!(preview_before_axis, preview_after_parameters);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);

    for key in [
        "action-create-construction-point",
        "action-create-construction-axis",
        "action-create-construction-plane",
    ] {
        assert!(shell.has_visible_label(&shell.catalog().text(key)));
    }
    let undo_before_axis = shell.app().undo_step_count();
    shell.click_button_label(&shell.catalog().text("action-create-construction-axis"));
    assert_eq!(shell.app().undo_step_count(), undo_before_axis + 1);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("digest-construction-axis-committed")
    );
    assert!(shell.app().document_snapshot().features().any(|feature| {
        matches!(
            feature.kind(),
            FeatureKind::ConstructionAxis {
                origin_mm: [0.0, 0.0, 20.0],
                direction: [100.0, 0.0, 0.0]
            }
        )
    }));
    let undo_before_point = shell.app().undo_step_count();
    shell.click_button_label(&shell.catalog().text("action-create-construction-point"));
    assert_eq!(shell.app().undo_step_count(), undo_before_point + 1);
    assert!(shell.app().document_snapshot().features().any(|feature| {
        matches!(
            feature.kind(),
            FeatureKind::ConstructionPoint {
                position_mm: [0.0, 0.0, 20.0]
            }
        )
    }));
    let undo_before_plane = shell.app().undo_step_count();
    shell.click_button_label(&shell.catalog().text("action-create-construction-plane"));
    assert_eq!(shell.app().undo_step_count(), undo_before_plane + 1);
    assert!(shell.app().document_snapshot().features().any(|feature| {
        matches!(
            feature.kind(),
            FeatureKind::ConstructionPlane {
                origin_mm: [0.0, 0.0, 20.0],
                normal: [100.0, 0.0, 0.0],
                x_direction: [0.0, 100.0, 0.0]
            }
        )
    }));

    let undo_before_helix = shell.app().undo_step_count();
    shell.click_button_label(&shell.catalog().text("action-create-helix"));
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

    open_from_command_search(&mut shell, AppCommand::Thread);
    assert!(shell.has_visible_label(&shell.catalog().text("thread-panel-title")));
    assert!(shell.has_visible_label(&shell.catalog().text("thread-profile")));
    assert!(shell.has_visible_label(&shell.catalog().text("helix-selected-edge-ready")));
    shell.click_button_label(&shell.catalog().text("helix-use-selected-edge"));
    replace_text(&mut shell, "helix-radius", "8");
    replace_text(&mut shell, "helix-pitch", "6");
    replace_text(&mut shell, "helix-turns", "2");
    replace_text(&mut shell, "thread-profile-radius", "0.65");
    let undo_before_thread = shell.app().undo_step_count();
    shell.click_button_label(&shell.catalog().text("action-create-thread"));
    assert_eq!(shell.app().undo_step_count(), undo_before_thread + 1);
    assert!(
        shell
            .app()
            .document_snapshot()
            .features()
            .any(|feature| matches!(feature.kind(), FeatureKind::Sweep { .. }))
    );
    wait_for_exact_bodies(&mut shell, 3);

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    wait_for_exact_bodies(&mut shell, 3);
}
