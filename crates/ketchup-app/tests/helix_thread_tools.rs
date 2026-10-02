use crate::harness;

use eframe::egui::Key;
use harness::{Shell, ctrl};
use ketchup_app::{AppCommand, dialogs::ScriptedFileDialogs};
use ketchup_interaction::Vec3;
use ketchup_model::document::FeatureKind;
use std::path::PathBuf;

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
    shell.wait_until(|shell| {
        shell.settle();
        shell.app().exact_render_body_count() == expected
    });
    assert_eq!(shell.app().exact_render_body_count(), expected);
}

#[test]
fn manual_helix_tool_makes_a_path_or_sweeps_a_profile_with_preview_undo_and_save_open() {
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
    // A helix without a profile is a path, not a body.
    assert!(
        !snapshot
            .features()
            .any(|feature| matches!(feature.kind(), FeatureKind::Sweep { .. }))
    );

    assert!(shell.app_mut().undo());
    assert!(shell.app_mut().redo());

    // The same tool sweeps a custom tooth (a trapezoid, crest > 0) into a thread body.
    open_from_command_search(&mut shell, AppCommand::Helix);
    assert!(shell.has_visible_label(&shell.catalog().text("helix-panel-title")));
    assert!(shell.has_visible_label(&shell.catalog().text("helix-selected-edge-ready")));
    shell.click_button_label(&shell.catalog().text("helix-use-selected-edge"));
    replace_text(&mut shell, "helix-radius", "8");
    replace_text(&mut shell, "helix-pitch", "6");
    replace_text(&mut shell, "helix-turns", "2");
    let tooth = format!(
        "{}: {}",
        shell.catalog().text("helix-profile"),
        shell.catalog().text("helix-profile-tooth")
    );
    shell.click_button_label(&tooth);
    replace_text(&mut shell, "helix-tooth-depth", "1.5");
    replace_text(&mut shell, "helix-tooth-width", "7");
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("digest-helix-invalid"),
        "a tooth wider than the pitch does not fit between the turns"
    );
    replace_text(&mut shell, "helix-tooth-width", "4");
    replace_text(&mut shell, "helix-tooth-crest", "1");
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("digest-helix-live")
    );
    let undo_before_thread = shell.app().undo_step_count();
    shell.click_button_label(&shell.catalog().text("action-create-helix"));
    assert_eq!(shell.app().undo_step_count(), undo_before_thread + 1);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("digest-helix-body-committed")
    );
    let snapshot = shell.app().document_snapshot();
    let profile = snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::Profile { segments, closed } => Some((segments.len(), *closed)),
            _ => None,
        })
        .last()
        .unwrap();
    assert_eq!(
        profile,
        (4, true),
        "the trapezoid tooth is four closed lines"
    );
    assert!(
        snapshot
            .features()
            .any(|feature| matches!(feature.kind(), FeatureKind::Sweep { up: Some(_), .. }))
    );
    wait_for_exact_bodies(&mut shell, 2);

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    wait_for_exact_bodies(&mut shell, 2);
}
