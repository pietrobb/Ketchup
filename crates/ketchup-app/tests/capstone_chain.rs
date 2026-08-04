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

use eframe::egui::{Key, Vec2};
use harness::{Shell, ctrl};
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::document::OccurrenceId;

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

    // 2. An exact rectangle becomes a second solid, again in one step.
    shell.click_command(AppCommand::Rectangle);
    shell.click_at(home);
    shell.type_text("1200,800");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().active_box_count(),
        2,
        "the exact rectangle must add a solid"
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
        [1200.0, 800.0, 20.0]
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
        [1200.0, 1300.0, 20.0]
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

    // 6. The two shared occurrences become instances of one named component.
    shell.click_at(solid);
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    let named = shell.app().document_revision();
    shell.click_menu_command("menu-model", AppCommand::MakeComponent);
    assert_eq!(
        shell.app().document_revision(),
        named + 1,
        "Make Component must commit exactly one canonical batch"
    );
    assert_eq!(
        shell.app().definition_count(),
        definitions,
        "naming a component must not clone the definition its instances share"
    );

    // 7. Editing one shared instance changes the shared definition in context.
    shell.double_click_at(solid);
    assert_eq!(
        shell.app().edit_context_depth(),
        1,
        "a double click must enter the component context"
    );
    shell.click_at(solid);
    let shared_edit = shell.app().canonical_digest();
    let shared_revision = shell.app().document_revision();
    shell.click_command(AppCommand::PushPull);
    shell.type_text("125");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().document_revision(),
        shared_revision + 1,
        "editing shared geometry must commit one canonical batch"
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
        assert_eq!([size.x, size.y, size.z], [1200.0, 1300.0, 145.0]);
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
    shell.click_at(solid);
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

    // 10. Editing the still-selected unique instance no longer changes its peer.
    shell.click_command(AppCommand::PushPull);
    shell.type_text("25");
    shell.press_key(Key::Enter);
    let unique_size = shell.app().occurrence_box_geometry(2).unwrap().1;
    let peer_size = shell.app().occurrence_box_geometry(3).unwrap().1;
    assert_eq!(
        [unique_size.x, unique_size.y, unique_size.z],
        [1200.0, 1300.0, 170.0]
    );
    assert_eq!(
        [peer_size.x, peer_size.y, peer_size.z],
        [1200.0, 1300.0, 145.0]
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
    assert_eq!(shell.app().active_box_count(), 1);
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
