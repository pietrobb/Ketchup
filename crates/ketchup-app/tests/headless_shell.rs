//! Acceptance workflows replayed offscreen, without touching the real pointer.
//!
//! Every assertion reads document state — revision, canonical digest,
//! occurrence and definition counts — because that is the thing the workflow is
//! supposed to change. Painted text is deliberately never asserted on.

mod harness;

use eframe::egui::Key;
use harness::{Shell, ctrl};
use ketchup_app::AppCommand;
use ketchup_interaction::Vec3;

#[test]
fn the_designed_shell_lays_itself_out_without_a_window() {
    let shell = Shell::new();

    assert!(
        shell.viewport_rect().area() > 0.0,
        "the viewport must be laid out"
    );
    assert!(
        shell.offers(AppCommand::Select),
        "the tool rail must offer Select under an accessible name, not a glyph"
    );
    assert!(
        shell.offers(AppCommand::Move),
        "the tool rail must offer Move under an accessible name, not a glyph"
    );
    assert_eq!(shell.app().active_box_count(), 1);
}

#[test]
fn every_tool_in_the_rail_is_reachable_by_its_accessible_name() {
    let shell = Shell::new();

    for command in [
        AppCommand::Select,
        AppCommand::Line,
        AppCommand::Rectangle,
        AppCommand::PushPull,
        AppCommand::Move,
        AppCommand::Measure,
        AppCommand::Orbit,
        AppCommand::Pan,
    ] {
        assert!(
            shell.offers(command),
            "{command:?} paints an icon and must still expose an accessible name"
        );
    }
}

#[test]
fn clicking_the_viewport_selects_the_occurrence() {
    let mut shell = Shell::new();
    let centre = shell.viewport_rect().center();

    shell.click_at(centre);

    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "clicking geometry must select exactly the occurrence under the pointer"
    );
}

#[test]
fn a_click_outside_geometry_clears_the_selection() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(rect.center());
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    shell.click_at(rect.left_top() + eframe::egui::Vec2::new(12.0, 12.0));

    assert_eq!(shell.app().selected_occurrence_count(), 0);
}

#[test]
fn copying_an_occurrence_shares_one_definition() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());

    let before = shell.app().document_revision();
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();

    assert_eq!(shell.app().active_box_count(), 2, "a second occurrence");
    assert_eq!(
        shell.app().definition_count(),
        1,
        "Copy must reuse the definition instead of cloning it"
    );
    assert_eq!(
        shell.app().document_revision(),
        before + 1,
        "one gesture must produce exactly one canonical revision"
    );
}

#[test]
fn undo_and_redo_return_the_document_to_identical_canonical_states() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let composed = shell.app().canonical_digest();
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();
    let copied = shell.app().canonical_digest();

    shell.key(Key::Z, ctrl());
    let undone_once = shell.app().canonical_digest();
    shell.key(Key::Y, ctrl());
    let redone_once = shell.app().canonical_digest();

    shell.key(Key::Z, ctrl());
    let undone_twice = shell.app().canonical_digest();
    shell.key(Key::Y, ctrl());
    let redone_twice = shell.app().canonical_digest();

    assert_eq!(
        undone_once, composed,
        "Undo must restore the previous state"
    );
    assert_eq!(redone_once, copied, "Redo must restore the copied state");
    assert_eq!(undone_once, undone_twice, "Undo must be reproducible");
    assert_eq!(redone_once, redone_twice, "Redo must be reproducible");
    assert_ne!(undone_once, redone_once);
}

#[test]
fn a_viewport_drag_in_move_commits_exactly_one_canonical_batch() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(rect.center());
    shell.click_command(AppCommand::Move);

    let before = shell.app().document_revision();
    let from = rect.center();
    shell.drag(from, from + eframe::egui::Vec2::new(120.0, 0.0));

    assert_eq!(
        shell.app().document_revision(),
        before + 1,
        "one completed drag must produce exactly one canonical revision"
    );
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "Move must not create geometry"
    );
}

#[test]
fn the_model_menu_offers_make_unique() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());

    shell.open_menu("menu-model");

    assert!(
        shell.offers(AppCommand::MakeUnique),
        "the Model menu must expose Make Unique"
    );
}

#[test]
fn make_unique_clones_the_definition_in_one_undo_step() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();
    assert_eq!(shell.app().definition_count(), 1);

    let before = shell.app().document_revision();
    shell.click_menu_command("menu-model", AppCommand::MakeUnique);

    assert_eq!(
        shell.app().definition_count(),
        2,
        "Make Unique must clone the definition"
    );
    assert_eq!(
        shell.app().active_box_count(),
        2,
        "and keep both occurrences"
    );
    assert_eq!(
        shell.app().document_revision(),
        before + 1,
        "Make Unique must be one undo step"
    );
}
