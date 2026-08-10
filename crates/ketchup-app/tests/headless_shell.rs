//! Acceptance workflows replayed offscreen, without touching the real pointer.
//!
//! Every assertion reads document state — revision, canonical digest,
//! occurrence and definition counts — because that is the thing the workflow is
//! supposed to change. Painted text is deliberately never asserted on.

mod harness;

use std::collections::BTreeMap;

use eframe::egui::{Key, Pos2, Rect, Vec2};
use harness::{Shell, ctrl};
use ketchup_app::{AlignMode, AppCommand};
use ketchup_core::document::{InstancePath, OccurrenceId};
use ketchup_interaction::{Axis, LocaleCatalog, SnapKind, Vec3};

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
fn localized_shells_fit_and_support_screen_reader_keyboard_focus() {
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0));
    for (locale, catalog) in [
        ("en-US", LocaleCatalog::english()),
        ("sk-SK", LocaleCatalog::slovak()),
        ("pseudo", LocaleCatalog::pseudo()),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert!(shell.viewport_rect().area() > 0.0, "{locale} viewport");

        for key in [
            "menu-file",
            "menu-edit",
            "menu-view",
            "menu-draw",
            "menu-tools",
            "menu-model",
            "menu-window",
            "menu-help",
        ] {
            let rect = shell.menu_rect(key);
            assert!(rect.area() > 0.0, "{locale} {key} has no layout box");
            assert!(
                screen.contains(rect.min) && screen.contains(rect.max),
                "{locale} {key} overflows the 1600 x 1000 acceptance viewport: {rect:?}"
            );
        }

        let visible_rects = shell.visible_accesskit_rects();
        assert!(visible_rects.len() > 20, "{locale} accessibility tree");
        for (node, rect) in visible_rects {
            let frame_tolerance = 1.5;
            assert!(
                rect.min.x >= screen.min.x - frame_tolerance
                    && rect.min.y >= screen.min.y - frame_tolerance
                    && rect.max.x <= screen.max.x + frame_tolerance
                    && rect.max.y <= screen.max.y + frame_tolerance,
                "{locale} publishes {node} outside the acceptance viewport: {rect:?}"
            );
        }

        for command in [
            AppCommand::Select,
            AppCommand::Rectangle,
            AppCommand::PushPull,
            AppCommand::Move,
            AppCommand::Measure,
            AppCommand::Orbit,
            AppCommand::Pan,
        ] {
            assert!(
                shell.offers(command),
                "{locale} must expose {command:?} by its localized AccessKit name"
            );
        }

        shell.focus_command(AppCommand::Rectangle);
        assert!(
            shell.command_is_focused(AppCommand::Rectangle),
            "{locale} AccessKit focus action must reach Rectangle"
        );
        let expected_digest = shell.catalog().format(
            "digest-tool-active",
            &BTreeMap::from([("tool", shell.catalog().text("tool-rectangle"))]),
        );
        shell.press_key(Key::Enter);
        assert_eq!(shell.app().action_digest(), expected_digest, "{locale}");
    }
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
fn viewport_snap_hysteresis_acquires_retains_and_releases_an_endpoint() {
    let mut shell = Shell::new();
    let endpoint = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();

    shell.move_pointer(endpoint + eframe::egui::Vec2::new(7.0, 0.0));
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    shell.move_pointer(endpoint + eframe::egui::Vec2::new(10.0, 0.0));
    assert_eq!(
        shell.app().hovered_snap_kind(),
        Some(SnapKind::Endpoint),
        "the release radius must prevent flicker outside the acquire radius"
    );
    shell.move_pointer(endpoint + eframe::egui::Vec2::new(14.0, 0.0));
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Face));
}

#[test]
fn tab_cycles_overlapping_occurrences_and_click_selects_the_visible_choice() {
    let mut shell = Shell::new();
    let centre = shell.viewport_rect().center();
    shell.click_at(centre);
    assert!(shell.app_mut().copy_selected(Vec3::new(100.0, 0.0, 0.0)));
    assert!(shell.app_mut().move_selected(Vec3::new(-100.0, 0.0, 0.0)));
    shell.move_pointer(centre);

    assert_eq!(shell.app().hovered_overlap_choice(), Some((0, 2)));
    assert_eq!(
        shell.app().hovered_selection().unwrap().instance_path,
        InstancePath::root(OccurrenceId(1))
    );
    shell.press_key(Key::Tab);
    assert_eq!(shell.app().hovered_overlap_choice(), Some((1, 2)));
    assert_eq!(
        shell.app().hovered_selection().unwrap().instance_path,
        InstancePath::root(OccurrenceId(2))
    );
    shell.click_at(centre);
    assert_eq!(
        shell.app().selected_reference().unwrap().instance_path,
        InstancePath::root(OccurrenceId(2))
    );
}

#[test]
fn snapped_measurement_updates_the_value_box_without_mutating_the_document() {
    let mut shell = Shell::new();
    shell.click_command(AppCommand::Measure);
    let left = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();
    let right = shell
        .app()
        .viewport_position(Vec3::new(100.0, 0.0, 20.0))
        .unwrap();
    let centre = shell.viewport_rect().center();
    let left = left + (centre - left).normalized() * 2.0;
    let right = right + (centre - right).normalized() * 2.0;
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    shell.click_at(left);
    shell.move_pointer(right);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    assert_eq!(
        shell.app().hovered_snap_position(),
        Some(Vec3::new(100.0, 0.0, 20.0))
    );
    shell.click_at(right);

    assert_eq!(
        shell.app().measured_points(),
        Some((Vec3::new(0.0, 0.0, 20.0), Vec3::new(100.0, 0.0, 20.0),))
    );
    assert_eq!(shell.app().value_input(), "100");
    assert_eq!(shell.app().measured_distance_mm(), Some(100.0));
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
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
fn exact_align_previews_then_commits_one_transform_batch_with_undo_redo() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    assert!(shell.app_mut().preview_align_occurrences(
        OccurrenceId(2),
        OccurrenceId(1),
        Axis::X,
        AlignMode::Maximum,
    ));
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2)),
        Some((Vec3::new(0.0, 25.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );

    assert!(shell.app_mut().confirm_occurrence_operation_preview());
    let aligned_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(2),
        Some((Vec3::new(0.0, 25.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert_eq!(shell.app().definition_count(), 1);
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app_mut().redo());
    assert_eq!(shell.app().canonical_digest(), aligned_digest);
}

#[test]
fn exact_align_preview_fails_closed_after_the_document_changes() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    assert!(shell.app_mut().preview_align_occurrences(
        OccurrenceId(2),
        OccurrenceId(1),
        Axis::Y,
        AlignMode::Center,
    ));
    assert!(shell.app_mut().move_selected(Vec3::new(0.0, 10.0, 0.0)));
    let changed_digest = shell.app().canonical_digest();

    assert!(!shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), changed_digest);
}

#[test]
fn exact_linear_pattern_previews_then_commits_one_shared_definition_batch() {
    let mut shell = Shell::new();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    assert!(
        shell
            .app_mut()
            .preview_linear_pattern(OccurrenceId(1), Axis::X, 125.0, 4,)
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 1);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2)),
        Some((Vec3::new(125.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(4)),
        Some((Vec3::new(375.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );

    assert!(shell.app_mut().confirm_occurrence_operation_preview());
    let patterned_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 4);
    assert_eq!(shell.app().definition_count(), 1);
    for occurrence_id in 1..=4 {
        assert_eq!(
            shell
                .app()
                .occurrence_definition_id(OccurrenceId(occurrence_id)),
            shell.app().occurrence_definition_id(OccurrenceId(1)),
        );
    }
    assert_eq!(
        shell.app().occurrence_box_geometry(4),
        Some((Vec3::new(375.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 1);
    assert!(shell.app_mut().redo());
    assert_eq!(shell.app().canonical_digest(), patterned_digest);
    assert_eq!(shell.app().active_box_count(), 4);
}

#[test]
fn exact_linear_pattern_rejects_invalid_input_and_stale_confirmation() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let before_digest = shell.app().canonical_digest();
    assert!(
        !shell
            .app_mut()
            .preview_linear_pattern(OccurrenceId(1), Axis::Y, f64::NAN, 3,)
    );
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), before_digest);

    assert!(
        shell
            .app_mut()
            .preview_linear_pattern(OccurrenceId(1), Axis::Y, -80.0, 3,)
    );
    assert!(shell.app_mut().move_selected(Vec3::new(0.0, 10.0, 0.0)));
    let changed_digest = shell.app().canonical_digest();
    assert!(!shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    assert_eq!(shell.app().active_box_count(), 1);
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
