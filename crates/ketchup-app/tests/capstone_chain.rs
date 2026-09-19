//! The mission capstone replayed end to end as one continuous session.
//!
//! Where the other acceptance files each prove one workflow in isolation, this
//! one walks the whole Definition of Done in a single document, in the order a
//! user walks it: an exact solid, exact Push/Pull, direct Move and Copy, Group
//! and Ungroup, shared component editing, Make Unique, Measure, visibility,
//! Zoom Fit, Undo/Redo, and save and reopen with stable identity.
//!
//! Every step is a pointer gesture, a menu click or a documented shortcut — no
//! developer entry point is called — and every assertion reads document state
//! rather than painted text.

mod harness;

use eframe::egui::{Key, Vec2, accesskit::Role};
use harness::{Shell, ctrl};
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::document::{FeatureKind, OccurrenceId};
use ketchup_interaction::Vec3;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[test]
fn the_manual_capstone_runs_end_to_end_through_the_designed_shell() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("capstone.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    let home = shell.viewport_rect().center();

    // 1. Direct Move: drag the starting solid aside, one drag, one batch.
    let initial_geometry = shell
        .app()
        .occurrence_box_geometry(1)
        .expect("the starting solid must be canonical");
    shell.click_at(home);
    shell.click_command(AppCommand::Move);
    let start = shell.app().document_revision();
    shell.drag(home, home + Vec2::new(-430.0, 260.0));
    assert_eq!(
        shell.app().document_revision(),
        start + 1,
        "a Move drag must commit exactly one canonical batch"
    );
    let moved_geometry = shell
        .app()
        .occurrence_box_geometry(1)
        .expect("the moved solid must remain canonical");
    assert_ne!(moved_geometry.0, initial_geometry.0);
    assert_eq!(moved_geometry.1, initial_geometry.1);

    // 2. An exact rectangle becomes a profile-only occurrence in one step.
    shell.click_command(AppCommand::Rectangle);
    shell.click_at(home);
    shell.type_text("1200,800");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().active_box_count(),
        2,
        "the exact rectangle must add a profile occurrence"
    );
    assert_eq!(shell.app().document_revision(), start + 2);
    let exact_rectangle = shell
        .app()
        .occurrence_box_geometry(2)
        .expect("the exact rectangle must have canonical geometry");
    assert_eq!(
        [
            exact_rectangle.1.x,
            exact_rectangle.1.y,
            exact_rectangle.1.z,
        ],
        [1200.0, 800.0, 0.0]
    );
    let definitions = shell.app().definition_count();

    // 3. Push/Pull with a typed distance extrudes that rectangle.
    let solid = home + Vec2::new(40.0, -20.0);
    shell.click_command(AppCommand::Select);
    shell.click_at(solid);
    let flat = shell.app().canonical_digest();
    shell.click_command(AppCommand::PushPull);
    shell.type_text("500");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), start + 3);
    assert_ne!(
        shell.app().canonical_digest(),
        flat,
        "exact Push/Pull must change the geometry"
    );
    let extruded = shell
        .app()
        .occurrence_box_geometry(2)
        .expect("the extrusion must remain canonical");
    assert_eq!(
        [extruded.1.x, extruded.1.y, extruded.1.z],
        [1200.0, 800.0, 500.0]
    );

    // 4. The created solid moves with the common click-object/click-destination
    // workflow, then Ctrl+C/Ctrl+V creates a shared occurrence in one batch.
    shell.click_command(AppCommand::Move);
    let before_move = shell
        .app()
        .occurrence_box_geometry(2)
        .expect("the created solid must remain selectable");
    shell.click_at(solid);
    let moved_solid = solid + Vec2::new(300.0, 0.0);
    shell.click_at(moved_solid);
    let solid = moved_solid;
    assert_eq!(shell.app().document_revision(), start + 4);
    assert_ne!(
        shell.app().occurrence_box_geometry(2).unwrap().0,
        before_move.0,
        "click-object/click-destination must move the created solid"
    );

    shell.native_copy();
    assert_eq!(
        shell.app().document_revision(),
        start + 4,
        "Copy only captures the selection"
    );
    shell.native_paste();
    assert_eq!(shell.app().document_revision(), start + 5);
    assert_eq!(
        shell.app().active_box_count(),
        3,
        "Ctrl+V must add an occurrence"
    );
    assert_eq!(
        shell.app().definition_count(),
        definitions,
        "Paste shares the definition — only Make Unique clones it"
    );
    let source_definition = shell
        .app()
        .occurrence_definition_id(OccurrenceId(2))
        .expect("the source occurrence must reference a definition");
    assert_eq!(
        shell.app().occurrence_definition_id(OccurrenceId(3)),
        Some(source_definition),
        "the copied occurrence must reference the same definition"
    );
    assert_ne!(
        shell.app().occurrence_box_geometry(2).unwrap().0,
        shell.app().occurrence_box_geometry(3).unwrap().0,
        "Copy must create a separately placed occurrence"
    );

    // 5. Group the whole model and take it apart again.
    shell.click_command(AppCommand::Select);
    shell.key(Key::A, ctrl());
    assert_eq!(
        shell.app().selected_occurrence_count(),
        3,
        "the documented Select All must select the whole model"
    );
    shell.click_menu_command("menu-model", AppCommand::Group);
    assert_eq!(shell.app().group_count(), 1, "Group must create one group");
    shell.click_menu_command("menu-model", AppCommand::Ungroup);
    assert_eq!(
        shell.app().group_count(),
        0,
        "Ungroup must dissolve it again"
    );

    // 6. Make Component performs one atomic group-to-component conversion.
    let before_component_digest = shell.app().canonical_digest();
    let before_component_revision = shell.app().document_revision();
    shell.click_menu_command("menu-model", AppCommand::Group);
    assert_eq!(shell.app().group_count(), 1);
    let grouped_revision = shell.app().document_revision();
    shell.click_menu_command("menu-model", AppCommand::MakeComponent);
    assert_eq!(
        shell.app().document_revision(),
        grouped_revision + 1,
        "Make Component must commit exactly one canonical batch"
    );
    assert_eq!(shell.app().group_count(), 0);
    assert_eq!(
        shell.app().definition_count(),
        definitions + 1,
        "conversion must create one definition-local component graph"
    );
    shell.key(Key::Z, ctrl());
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().document_revision(), before_component_revision);
    assert_eq!(shell.app().canonical_digest(), before_component_digest);

    // 7. Editing one shared instance changes the shared definition in context.
    // Aim at the moved solid's own top face rather than a fixed pixel offset,
    // so the gesture keeps hitting it whatever the camera projection is.
    shell.click_at(solid);
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    shell.double_click_at(solid);
    assert_eq!(
        shell.app().edit_context_depth(),
        1,
        "a double click must enter the component context"
    );
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    shell.click_at(shell.top_face_centre(2));
    let shared_edit = shell.app().canonical_digest();
    let shared_revision = shell.app().document_revision();
    shell.click_command(AppCommand::PushPull);
    shell.type_text("125");
    shell.press_key(Key::Enter);
    assert!(
        shell.app().document_revision() > shared_revision,
        "editing shared geometry after an undone branch must commit a new canonical revision"
    );
    assert_ne!(
        shell.app().canonical_digest(),
        shared_edit,
        "editing in context must change the shared definition"
    );
    assert_eq!(
        shell.app().definition_count(),
        definitions,
        "editing a shared component must not clone its definition"
    );
    assert_eq!(
        shell.app().occurrence_definition_id(OccurrenceId(2)),
        shell.app().occurrence_definition_id(OccurrenceId(3))
    );
    for occurrence_id in [2, 3] {
        let size = shell
            .app()
            .occurrence_box_geometry(occurrence_id)
            .expect("each shared occurrence must derive the edited geometry")
            .1;
        assert_eq!([size.x, size.y, size.z], [1200.0, 800.0, 625.0]);
    }
    shell.press_key(Key::Escape);
    assert_eq!(
        shell.app().edit_context_depth(),
        1,
        "the first Escape clears the in-context selection"
    );
    shell.press_key(Key::Escape);
    assert_eq!(
        shell.app().edit_context_depth(),
        0,
        "the second Escape must leave the context"
    );

    // 8. Make Unique clones and rebinds only the selected occurrence.
    shell.click_at(shell.top_face_centre(2));
    shell.click_menu_command("menu-model", AppCommand::MakeUnique);
    assert_eq!(
        shell.app().definition_count(),
        definitions + 1,
        "Make Unique must clone the shared definition"
    );
    let unique_definition = shell
        .app()
        .occurrence_definition_id(OccurrenceId(2))
        .expect("the selected occurrence must reference its cloned definition");
    assert_ne!(unique_definition, source_definition);
    assert_eq!(
        shell.app().occurrence_definition_id(OccurrenceId(3)),
        Some(source_definition),
        "the peer occurrence must retain the shared source definition"
    );

    // 9. Undo and Redo step over Make Unique and restore identical identity.
    let unique_digest = shell.app().canonical_digest();
    let unique_revision = shell.app().document_revision();
    shell.key(Key::Z, ctrl());
    assert_eq!(
        shell.app().occurrence_definition_id(OccurrenceId(2)),
        Some(source_definition),
        "Undo must restore sharing"
    );
    assert_eq!(shell.app().document_revision(), unique_revision - 1);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), unique_digest);
    assert_eq!(
        shell.app().occurrence_definition_id(OccurrenceId(2)),
        Some(unique_definition)
    );
    assert_eq!(shell.app().document_revision(), unique_revision);

    // 10. Reselecting the unique instance after history navigation isolates its edit.
    let (origin, size) = shell.app().occurrence_box_geometry(2).unwrap();
    let side_face = shell.app().project_to_screen(
        ketchup_interaction::Vec3::new(origin.x + size.x * 0.5, origin.y, origin.z + size.z * 0.5),
        shell.viewport_rect(),
    );
    shell.click_at(side_face);
    shell.click_command(AppCommand::PushPull);
    shell.type_text("25");
    shell.press_key(Key::Enter);
    let unique_size = shell.app().occurrence_box_geometry(2).unwrap().1;
    let peer_size = shell.app().occurrence_box_geometry(3).unwrap().1;
    assert_eq!(
        [unique_size.x, unique_size.y, unique_size.z],
        [1200.0, 825.0, 625.0]
    );
    assert_eq!(
        [peer_size.x, peer_size.y, peer_size.z],
        [1200.0, 800.0, 625.0]
    );

    // 11. Measure reads the model without creating document history.
    let measured_revision = shell.app().document_revision();
    let measured_digest = shell.app().canonical_digest();
    shell.click_command(AppCommand::Measure);
    shell.click_at(home - Vec2::new(120.0, 0.0));
    shell.click_at(home + Vec2::new(120.0, 0.0));
    assert!(
        shell
            .app()
            .measured_distance_mm()
            .is_some_and(|distance| distance.is_finite() && distance > 0.0),
        "Measure must report a real distance"
    );
    assert_eq!(shell.app().document_revision(), measured_revision);
    assert_eq!(shell.app().canonical_digest(), measured_digest);

    // 12. Visibility is canonical, undoable, and reversible through View.
    shell.click_command(AppCommand::Select);
    shell.click_at(solid);
    let visibility_revision = shell.app().document_revision();
    shell.click_menu_command("menu-view", AppCommand::Hide);
    assert_eq!(shell.app().hidden_occurrence_count(), 1);
    assert_eq!(
        shell.app().document_revision(),
        visibility_revision + 1,
        "Hide must commit exactly one canonical batch"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().hidden_occurrence_count(), 0);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().hidden_occurrence_count(), 1);
    shell.click_menu_command("menu-view", AppCommand::Unhide);
    assert_eq!(shell.app().hidden_occurrence_count(), 0);

    // 13. Zoom Fit changes only the camera, never the canonical document.
    let framed_revision = shell.app().document_revision();
    let framed_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    assert!(shell.app().camera_zoom().is_finite());
    assert_eq!(shell.app().document_revision(), framed_revision);
    assert_eq!(shell.app().canonical_digest(), framed_digest);

    let composed = shell.app().canonical_digest();

    // 14. Save, discard, reopen — identity survives the round trip.
    assert!(shell.app().is_dirty(), "an edited document must be dirty");
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file(), "Save As must write the document");
    assert!(!shell.app().is_dirty());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert_eq!(shell.app().active_box_count(), 0);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(
        shell.app().canonical_digest(),
        composed,
        "the reopened capstone must keep IDs, hierarchy, transforms and sharing"
    );
    assert_eq!(shell.app().active_box_count(), 3);
    assert_eq!(shell.app().definition_count(), definitions + 1);
    assert!(!shell.app().is_dirty());
}

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

fn wait_for_exact_bodies(shell: &mut Shell, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        shell.settle();
        if shell.app().exact_render_body_count() == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(shell.app().exact_render_body_count(), expected);
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

#[test]
fn empty_document_manual_ux_capstone_has_rendered_and_native_exact_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("empty-document-capstone.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    shell.click_menu_command("menu-file", AppCommand::New);
    assert_eq!(shell.app().active_box_count(), 0);
    assert_eq!(shell.app().definition_count(), 0);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the capstone requires the native exact worker");
    wait_for_exact_bodies(&mut shell, 0);

    // Create the exact solid entirely through Rectangle and typed Push/Pull.
    // This legacy XY profile remains useful here because its box bounds give the
    // later pointer gestures an independently measurable target.
    let home = shell.viewport_rect().center();
    shell.click_command(AppCommand::Rectangle);
    shell.click_at(home);
    shell.type_text("120,80");
    shell.press_key(Key::Enter);
    shell.click_command(AppCommand::Select);
    shell.click_at(shell.top_face_centre(1));
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    shell.click_command(AppCommand::PushPull);
    shell.type_text("40");
    shell.press_key(Key::Enter);
    let (_, size) = shell.app().occurrence_box_geometry(1).unwrap();
    for (actual, expected) in [size.x, size.y, size.z]
        .into_iter()
        .zip([120.0, 80.0, 40.0])
    {
        assert!((actual - expected).abs() < 1.0e-9);
    }
    wait_for_exact_bodies(&mut shell, 1);
    assert!(shell.app().exact_render_triangle_count() > 0);
    shell.press_key(Key::Escape);
    shell.click_command(AppCommand::Select);

    // Add a precise rectangle on the vertical XZ datum through the spatial
    // workflow. It stays a sketch so the capstone carries both 3D drawing and
    // solid Push/Pull evidence without hiding either behind a creation API.
    shell.press_key(Key::R);
    let face_workflow = shell.catalog().text("face-workflow-title");
    let xz = shell.catalog().text("face-workflow-datum-xz");
    if !shell.has_role_and_label(Role::RadioButton, &xz) {
        shell.click_role_and_label(Role::Button, &face_workflow);
    }
    shell.click_role_and_label(Role::RadioButton, &xz);
    let start = shell
        .app()
        .viewport_position(Vec3::new(10.0, 0.0, 10.0))
        .unwrap();
    let end = shell
        .app()
        .viewport_position(Vec3::new(40.0, 0.0, 30.0))
        .unwrap();
    shell.click_at(start);
    shell.move_pointer(end);
    shell.type_text("30,20");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().active_box_count(), 2);
    assert!(
        shell
            .app()
            .document_snapshot()
            .features()
            .any(|feature| matches!(feature.kind(), FeatureKind::Sketch(_))),
        "XZ sketch missing after {}: {:?}",
        shell.app().action_digest(),
        shell
            .app()
            .document_snapshot()
            .features()
            .map(|feature| format!("{:?}", feature.kind()))
            .collect::<Vec<_>>()
    );
    shell.click_command(AppCommand::Select);
    shell.click_at(shell.top_face_centre(1));

    // Copy through native UI events and place the copy exactly over the source.
    // The visible overlap target, not an internal selection helper, is cycled by Tab.
    shell.native_copy();
    shell.native_paste();
    assert_eq!(shell.app().active_box_count(), 3);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    shell.click_command(AppCommand::Move);
    let copied = shell.top_face_centre(3);
    let source = shell.top_face_centre(1);
    shell.drag(copied, source);
    let source_geometry = shell.app().occurrence_box_geometry(1).unwrap();
    let copied_geometry = shell.app().occurrence_box_geometry(3).unwrap();
    for (actual, expected) in [
        copied_geometry.0.x,
        copied_geometry.0.y,
        copied_geometry.0.z,
        copied_geometry.1.x,
        copied_geometry.1.y,
        copied_geometry.1.z,
    ]
    .into_iter()
    .zip([
        source_geometry.0.x,
        source_geometry.0.y,
        source_geometry.0.z,
        source_geometry.1.x,
        source_geometry.1.y,
        source_geometry.1.z,
    ]) {
        assert!((actual - expected).abs() < 1.0e-9);
    }
    wait_for_exact_bodies(&mut shell, 1);

    shell.click_command(AppCommand::Select);
    shell.move_pointer(source);
    let first = shell
        .app()
        .hovered_selection()
        .expect("overlapping solids must publish a visible selection")
        .instance_path
        .root_occurrence();
    assert_eq!(shell.app().hovered_overlap_choice(), Some((0, 4)));
    shell.press_key(Key::Tab);
    let second = shell
        .app()
        .hovered_selection()
        .expect("Tab must keep a visible overlap choice")
        .instance_path
        .root_occurrence();
    assert_ne!(first, second);
    if second != OccurrenceId(3) {
        shell.press_key(Key::Tab);
    }
    assert_eq!(
        shell
            .app()
            .hovered_selection()
            .unwrap()
            .instance_path
            .root_occurrence(),
        OccurrenceId(3)
    );
    shell.click_at(source);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(3)));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let separated = shell.app().occurrence_box_geometry(3).unwrap();
    assert!((separated.0 - source_geometry.0).length() > 1.0);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let overlapped = shell.app().occurrence_box_geometry(3).unwrap();
    assert!((overlapped.0 - source_geometry.0).length() < 1.0e-9);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!((shell.app().occurrence_box_geometry(3).unwrap().0 - source_geometry.0).length() > 1.0);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);

    // The same session enters a group and then the shared component definition.
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-model", AppCommand::Group);
    assert_eq!(shell.app().group_count(), 1);
    shell.double_click_at(shell.top_face_centre(1));
    assert_eq!(shell.app().edit_context_depth(), 1);
    shell.double_click_at(shell.top_face_centre(1));
    assert_eq!(shell.app().edit_context_depth(), 2);
    assert!(shell.has_visible_label(&shell.app().edit_context_readout()));
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().edit_context_depth(), 1);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().edit_context_depth(), 0);
    assert_eq!(shell.app().group_count(), 1);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().group_count(), 0);

    // Use native exact face references for a one-time component placement. The
    // offscreen renderer proves both the transient RGBA preview and its commit.
    let exact_deadline = Instant::now() + Duration::from_secs(15);
    while shell.app().exact_stable_reference_count() < 2 && Instant::now() < exact_deadline {
        shell.settle();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(shell.app().exact_stable_reference_count() >= 2);
    let moving_name = shell.app().occurrence_name(OccurrenceId(1)).unwrap();
    let reference_name = shell.app().occurrence_name(OccurrenceId(3)).unwrap();
    shell.click_role_and_label(Role::Button, &shell.catalog().text("assembly-title"));
    let target = shell.catalog().format(
        "assembly-use-endpoint-a",
        &std::collections::BTreeMap::from([("name", reference_name)]),
    );
    shell.click_button_label(&target);
    let moving = shell.catalog().format(
        "assembly-use-endpoint-b",
        &std::collections::BTreeMap::from([("name", moving_name)]),
    );
    shell.click_button_label(&moving);
    for key in ["assembly-reference-a", "assembly-reference-b"] {
        shell.click_role_and_label(Role::ComboBox, &shell.catalog().text(key));
        shell.click_button_label("extrusion.top");
    }
    replace_text(&mut shell, "assembly-distance-value", "12.5");
    assert!(
        shell.app().assembly_planar_placement_frames().is_some(),
        "placement inputs are incomplete: {}",
        shell.app().action_digest()
    );
    let before_placement = shell.app().canonical_digest();
    let before_placement_world = shell
        .app()
        .document_snapshot()
        .world_transform_for_occurrence(OccurrenceId(1))
        .unwrap();
    let before_pixels = shell.render_viewport_pixels();
    assert_eq!(before_pixels.len() % 4, 0);
    assert!(before_pixels.chunks_exact(4).any(|rgba| rgba[3] != 0));
    shell.click_button_label(&shell.catalog().text("assembly-preview-placement"));
    let preview_pixels = shell.render_viewport_pixels();
    assert_eq!(preview_pixels.len(), before_pixels.len());
    assert!(preview_pixels.chunks_exact(4).any(|rgba| rgba[3] != 0));
    assert_eq!(shell.app().canonical_digest(), before_placement);
    let preview_transform = shell
        .app()
        .assembly_preview_world_transform(OccurrenceId(1))
        .unwrap_or_else(|| panic!("placement preview missing: {}", shell.app().action_digest()));
    assert_ne!(preview_transform, before_placement_world);
    shell.click_button_label(&shell.catalog().text("assembly-confirm-preview"));
    let placed = shell.app().canonical_digest();
    assert_ne!(placed, before_placement);
    assert_ne!(shell.render_viewport_pixels(), before_pixels);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before_placement);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), placed);

    // Select a real model edge and drive both tools through command search and
    // their visible forms. Helix preview is rendered before either commit.
    shell.click_command(AppCommand::Select);
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let edge = shell
        .app()
        .viewport_position(origin + Vec3::new(size.x * 0.5, 0.0, size.z))
        .unwrap();
    shell.click_at(edge);
    open_from_command_search(&mut shell, AppCommand::Helix);
    assert!(shell.has_visible_label(&shell.catalog().text("helix-selected-edge-ready")));
    let before_helix_preview = shell.render_viewport_pixels();
    shell.click_button_label(&shell.catalog().text("helix-use-selected-edge"));
    replace_text(&mut shell, "helix-radius", "7");
    replace_text(&mut shell, "helix-pitch", "4.5");
    replace_text(&mut shell, "helix-turns", "2.25");
    let helix_preview = shell.render_viewport_pixels();
    assert_ne!(helix_preview, before_helix_preview);
    shell.click_button_label(&shell.catalog().text("action-create-helix"));
    assert!(
        shell
            .app()
            .document_snapshot()
            .features()
            .any(|feature| matches!(feature.kind(), FeatureKind::SpatialPath { .. }))
    );

    open_from_command_search(&mut shell, AppCommand::Thread);
    assert!(shell.has_visible_label(&shell.catalog().text("helix-selected-edge-ready")));
    shell.click_button_label(&shell.catalog().text("helix-use-selected-edge"));
    replace_text(&mut shell, "helix-radius", "8");
    replace_text(&mut shell, "helix-pitch", "6");
    replace_text(&mut shell, "helix-turns", "2");
    replace_text(&mut shell, "thread-profile-radius", "0.65");
    let exact_before_thread = shell.app().exact_render_body_count();
    shell.click_button_label(&shell.catalog().text("action-create-thread"));
    assert!(
        shell
            .app()
            .document_snapshot()
            .features()
            .any(|feature| matches!(feature.kind(), FeatureKind::Sweep { .. }))
    );
    wait_for_exact_bodies(&mut shell, exact_before_thread + 2);

    let persisted = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert_eq!(shell.app().active_box_count(), 0);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted);
    wait_for_exact_bodies(&mut shell, exact_before_thread + 2);
    assert!(!shell.app().is_dirty());
}

#[test]
fn the_documented_group_shortcuts_do_the_same_as_the_model_menu() {
    let mut shell = Shell::new();
    let home = shell.viewport_rect().center();

    shell.click_at(home);
    shell.click_command(AppCommand::Move);
    shell.drag_with(home, home + Vec2::new(300.0, 0.0), ctrl());
    shell.click_command(AppCommand::Select);
    shell.key(Key::A, ctrl());
    assert_eq!(shell.app().selected_occurrence_count(), 2);

    shell.key(Key::G, ctrl());
    assert_eq!(
        shell.app().group_count(),
        1,
        "the documented Ctrl+G must group the selection"
    );

    shell.key(Key::G, harness::ctrl_shift());
    assert_eq!(
        shell.app().group_count(),
        0,
        "the documented Ctrl+Shift+G must ungroup it"
    );
}
