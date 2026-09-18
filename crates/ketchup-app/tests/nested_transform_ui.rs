mod harness;

use std::{path::PathBuf, time::Duration};

use eframe::egui::{Key, Modifiers, accesskit::Role};
use harness::{Shell, shift};
use ketchup_app::{AppCommand, dialogs::ScriptedFileDialogs};
use ketchup_application::transforms::world_axis_rotation_transform;
use ketchup_core::{
    document::{
        CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
        FeatureKind, GroupId, OccurrenceId, Transform,
    },
    persistence,
};
use ketchup_interaction::Vec3;

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

fn wait_for_stable_references(shell: &mut Shell) {
    for _ in 0..150 {
        shell.settle();
        if shell.app().exact_stable_reference_count() >= 2 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        shell.app().exact_stable_reference_count() >= 2,
        "exact planar placement references were not published"
    );
}

#[test]
fn elevated_corner_move_keeps_its_anchor_plane_in_free_space() {
    let mut shell = Shell::new();
    let original = world(&shell, 1);
    let before = shell.app().canonical_digest();
    let steps = shell.app().undo_step_count();
    let start = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();
    let target = shell
        .app()
        .viewport_position(Vec3::new(-30.0, 0.0, 20.0))
        .unwrap();
    shell.click_command(AppCommand::Move);
    shell.move_pointer(start);
    assert_eq!(
        shell.app().hovered_snap_position(),
        Some(Vec3::new(0.0, 0.0, 20.0))
    );
    assert!(
        shell.viewport_rect().contains(target),
        "target {target:?} outside {:?}",
        shell.viewport_rect()
    );
    shell.click_at(start);
    shell.move_pointer(target);
    assert_ne!(
        shell.app().value_input(),
        "0",
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_at(target);
    assert_transform(
        world(&shell, 1),
        Transform::from_translation(-30.0, 0.0, 0.0)
            .unwrap()
            .compose(original),
    );
    assert_eq!(shell.app().undo_step_count(), steps + 1);
    let after = shell.app().canonical_digest();
    shell.press_key(Key::Escape);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), after);
}
fn fixture() -> (tempfile::TempDir, Shell) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested.ketchup");
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Body".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::from_decimal("20").unwrap(),
                },
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(10),
                name: "Outer".into(),
                parent: None,
                transform: Transform::from_translation(30.0, -40.0, 20.0)
                    .unwrap()
                    .compose(
                        world_axis_rotation_transform(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), 90.0)
                            .unwrap(),
                    ),
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(20),
                name: "Inner".into(),
                parent: Some(GroupId(10)),
                transform: Transform::from_translation(10.0, 20.0, 30.0)
                    .unwrap()
                    .compose(
                        world_axis_rotation_transform(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 90.0)
                            .unwrap(),
                    ),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Moving".into(),
                transform: Transform::identity(),
                parent: Some(GroupId(20)),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "Reference".into(),
                transform: Transform::from_translation(180.0, 90.0, 50.0).unwrap(),
                parent: Some(GroupId(20)),
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    persistence::save_atomic(&path, &document.current()).unwrap();
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    let point = shell.top_face_centre(1);
    shell.double_click_at(point);
    assert_eq!(shell.app().edit_context_depth(), 1);
    shell.click_at(point);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
    (directory, shell)
}

fn world(shell: &Shell, id: u64) -> Transform {
    shell
        .app()
        .document_snapshot()
        .world_transform_for_occurrence(OccurrenceId(id))
        .unwrap()
}

fn assert_transform(actual: Transform, expected: Transform) {
    for (a, e) in actual.matrix().iter().zip(expected.matrix()) {
        assert!((a - e).abs() < 1.0e-6, "{actual:?} != {expected:?}");
    }
}

#[test]
fn nested_move_click_preview_escape_and_typed_commit_are_world_space() {
    let (_directory, mut shell) = fixture();
    let before = shell.app().canonical_digest();
    let steps = shell.app().undo_step_count();
    let original = world(&shell, 1);
    let reference = world(&shell, 2);
    let start = shell.top_face_centre(1);
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let target = shell
        .app()
        .viewport_position(Vec3::new(
            origin.x + size.x * 0.5 + 37.0,
            origin.y + size.y * 0.5,
            origin.z + size.z,
        ))
        .unwrap();
    shell.click_command(AppCommand::Move);
    shell.press_key(Key::ArrowRight);
    shell.click_at(start);
    shell.move_pointer(target);
    assert_eq!(shell.app().canonical_digest(), before);
    assert_ne!(shell.app().value_input(), "0");
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().canonical_digest(), before);
    assert_eq!(shell.app().undo_step_count(), steps);
    shell.click_command(AppCommand::Move);
    shell.click_at(start);
    shell.move_pointer(target);
    shell.type_text("37");
    shell.press_key(Key::Enter);
    assert_transform(
        world(&shell, 1),
        Transform::from_translation(37.0, 0.0, 0.0)
            .unwrap()
            .compose(original),
    );
    assert_eq!(world(&shell, 2), reference);
    assert_eq!(shell.app().undo_step_count(), steps + 1);
    let after = shell.app().canonical_digest();
    shell.press_key(Key::Escape);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), after);
}

#[test]
fn nested_rotate_typed_angle_and_correction_keep_world_pivot_and_single_undo() {
    let (_directory, mut shell) = fixture();
    let before = shell.app().canonical_digest();
    let steps = shell.app().undo_step_count();
    let original = world(&shell, 1);
    let reference = world(&shell, 2);
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let centre = origin + size * 0.5;
    shell.click_command(AppCommand::Rotate);
    shell.press_key(Key::ArrowRight);
    shell.type_text("35");
    shell.press_key(Key::Enter);
    assert_transform(
        world(&shell, 1),
        world_axis_rotation_transform(centre, Vec3::new(1.0, 0.0, 0.0), 35.0)
            .unwrap()
            .compose(original),
    );
    shell.type_text("20");
    shell.press_key(Key::Enter);
    assert_transform(
        world(&shell, 1),
        world_axis_rotation_transform(centre, Vec3::new(1.0, 0.0, 0.0), 20.0)
            .unwrap()
            .compose(original),
    );
    assert_eq!(world(&shell, 2), reference);
    assert_eq!(shell.app().undo_step_count(), steps + 1);
    let after = shell.app().canonical_digest();
    shell.press_key(Key::Escape);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), after);
}

#[test]
fn nested_align_dialog_preview_cancel_confirm_matches_world_geometry() {
    let (_directory, mut shell) = fixture();
    shell.click_at_with(shell.top_face_centre(2), shift());
    let before = shell.app().canonical_digest();
    let steps = shell.app().undo_step_count();
    let reference = world(&shell, 1);
    let original = world(&shell, 2);
    let origin = shell.app().occurrence_box_geometry(2).unwrap().0;
    for cancel in [true, false] {
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-preview"),
        );
        let preview = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2))
            .unwrap();
        assert_eq!(shell.app().canonical_digest(), before);
        if cancel {
            shell.click_role_and_label(
                Role::Button,
                &shell.catalog().text("dialog-align-occurrences-cancel"),
            );
            assert_eq!(shell.app().canonical_digest(), before);
            assert_eq!(shell.app().undo_step_count(), steps);
        } else {
            shell.click_role_and_label(
                Role::Button,
                &shell.catalog().text("dialog-align-occurrences-confirm"),
            );
            let delta = preview.0 - origin;
            assert_transform(
                world(&shell, 2),
                Transform::from_translation(delta.x, delta.y, delta.z)
                    .unwrap()
                    .compose(original),
            );
            assert_eq!(world(&shell, 1), reference);
            let actual = shell.app().occurrence_box_geometry(2).unwrap();
            assert!((actual.0.x - preview.0.x).abs() < 1.0e-6);
            assert!((actual.0.y - preview.0.y).abs() < 1.0e-6);
            assert!((actual.0.z - preview.0.z).abs() < 1.0e-6);
            assert_eq!(shell.app().undo_step_count(), steps + 1);
        }
    }
    let after = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), after);
}

#[test]
fn exact_face_placement_flip_offset_preview_cancel_confirm_is_nested_and_one_undo() {
    let (_directory, mut shell) = fixture();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_stable_references(&mut shell);
    let assembly = shell.catalog().text("assembly-title");
    shell.click_role_and_label(Role::Button, &assembly);

    let target = shell.catalog().format(
        "assembly-use-endpoint-a",
        &std::collections::BTreeMap::from([("name", "Reference".to_owned())]),
    );
    shell.click_button_label(&target);
    let source = shell.catalog().format(
        "assembly-use-endpoint-b",
        &std::collections::BTreeMap::from([("name", "Moving".to_owned())]),
    );
    shell.click_button_label(&source);
    for key in ["assembly-reference-a", "assembly-reference-b"] {
        shell.click_role_and_label(Role::ComboBox, &shell.catalog().text(key));
        shell.click_button_label("extrusion.top");
    }
    let offset = shell.catalog().text("assembly-distance-value");
    shell.focus_text_input(&offset);
    shell.key(Key::A, Modifiers::CTRL);
    shell.type_text("12.5");

    let before = shell.app().canonical_digest();
    let steps = shell.app().undo_step_count();
    let original = world(&shell, 1);
    let reference = world(&shell, 2);
    let (reference_origin, reference_normal, source_origin, _) =
        shell.app().assembly_planar_placement_frames().unwrap();
    let placement = shell.catalog().text("assembly-preview-placement");
    let before_pixels = shell.render_viewport_pixels();
    shell.click_button_label(&placement);
    let flipped_pixels = shell.render_viewport_pixels();
    assert!(flipped_pixels != before_pixels);
    let flipped_preview = shell
        .app()
        .assembly_preview_world_transform(OccurrenceId(1))
        .unwrap_or_else(|| panic!("{}", shell.app().action_digest()));
    assert_eq!(shell.app().canonical_digest(), before);
    assert_eq!(shell.app().assembly_mate_count(), 0);
    shell.click_button_label(&shell.catalog().text("assembly-cancel-preview"));
    assert!(shell.render_viewport_pixels() == before_pixels);
    assert_eq!(shell.app().canonical_digest(), before);
    assert_eq!(shell.app().undo_step_count(), steps);

    shell.click_role_and_label(Role::CheckBox, &shell.catalog().text("assembly-reversed"));
    shell.click_button_label(&placement);
    let preview_pixels = shell.render_viewport_pixels();
    assert!(preview_pixels != before_pixels);
    assert!(preview_pixels != flipped_pixels);
    let preview = shell
        .app()
        .assembly_preview_world_transform(OccurrenceId(1))
        .unwrap_or_else(|| panic!("{}", shell.app().action_digest()));
    assert_ne!(preview, flipped_preview);
    let expected_delta = reference_origin + reference_normal * 12.5 - source_origin;
    assert_transform(
        preview,
        Transform::from_translation(expected_delta.x, expected_delta.y, expected_delta.z)
            .unwrap()
            .compose(original),
    );
    assert_eq!(shell.app().canonical_digest(), before);
    assert_eq!(world(&shell, 2), reference);
    shell.click_button_label(&shell.catalog().text("assembly-confirm-preview"));
    assert!(shell.render_viewport_pixels() != before_pixels);
    assert_transform(world(&shell, 1), preview);
    assert_eq!(world(&shell, 2), reference);
    assert_eq!(shell.app().assembly_mate_count(), 0);
    assert_eq!(shell.app().undo_step_count(), steps + 1);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before);
    assert_transform(world(&shell, 1), original);
}
