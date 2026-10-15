//! One manual modelling session driven by keyboard shortcuts, clicks and typed
//! values: a rectangle drawn on a face, pushed in as a pocket, the part moved,
//! turned and scaled. Every tool first refuses an invalid value without
//! touching the document, every step is one Undo step, Undo walks back through
//! each earlier document and Redo forward again, and the result survives Save
//! and Open.

use crate::harness::contract::{DocumentState, assert_unchanged, save_open_dialogs};
use crate::harness::{Shell, ctrl, ctrl_shift};
use eframe::egui::{Key, Modifiers, Pos2};
use ketchup_app::AppCommand;
use ketchup_interaction::Vec3;
use std::collections::BTreeMap;

fn exact_worker_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

fn screen(shell: &Shell, point: Vec3) -> Pos2 {
    shell.app().viewport_position(point).unwrap()
}

/// Types `input` the way keys arrive: its first character opens the value
/// field, which then takes the rest.
fn type_value(shell: &mut Shell, input: &str) {
    let (first, rest) = input.split_at(1);
    shell.type_text(first);
    if !rest.is_empty() {
        shell.type_text(rest);
    }
    assert_eq!(shell.app().value_input(), input);
}

/// Types `input` + Enter and checks the document did not change; the gesture
/// stays armed, so the next typed value replaces the refused one.
fn refused(shell: &mut Shell, step: &str, input: &str) {
    let before = DocumentState::of(shell);
    type_value(shell, input);
    shell.press_key(Key::Enter);
    assert_unchanged(
        shell,
        &before,
        step,
        &format!("the invalid value {input:?}"),
    );
}

/// Types `input` + Enter and checks it made exactly one new Undo step.
fn applied(shell: &mut Shell, step: &str, input: &str, history: &mut Vec<String>) {
    let undo = shell.app().undo_step_count();
    type_value(shell, input);
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "{step}: one Undo step: {}",
        shell.app().action_digest()
    );
    let digest = shell.app().canonical_digest();
    assert_ne!(Some(&digest), history.last(), "{step} changed nothing");
    history.push(digest);
}

#[test]
fn a_keyboard_driven_session_refuses_bad_values_and_walks_its_history_both_ways() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.ketchup");
    let mut shell = Shell::with_dialogs(save_open_dialogs(&path));
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    assert!(shell.wait_until(|shell| {
        shell.step();
        shell.app().exact_render_body_count() == 1
    }));
    let mut history = vec![shell.app().canonical_digest()];

    // A rectangle on the box's top face, its size typed after the first corner.
    shell.press_key(Key::R);
    shell.click_at(screen(&shell, Vec3::new(20.0, 20.0, 20.0)));
    refused(&mut shell, "Rectangle", "0;20");
    applied(&mut shell, "Rectangle", "30;20", &mut history);

    // The drawn rectangle is selected; Push/Pull sinks it as a pocket.
    shell.press_key(Key::P);
    refused(&mut shell, "Push/Pull", "0");
    applied(&mut shell, "Push/Pull", "-5", &mut history);

    // The part is picked and moved: a base point, the pointer showing the
    // direction, then the typed distance. Rotate likewise takes its centre and
    // reference arm by clicks and its angle typed.
    // The pocketed part is an exact body now; picks wait until it is shown.
    assert!(shell.wait_until(|shell| {
        shell.step();
        shell.app().exact_render_body_count() == 1
    }));
    let base = Vec3::new(80.0, 50.0, 20.0);
    shell.press_key(Key::Space);
    shell.click_at(screen(&shell, base));
    shell.press_key(Key::M);
    shell.click_at(screen(&shell, base));
    shell.move_pointer(screen(&shell, base + Vec3::new(40.0, 0.0, 0.0)));
    refused(&mut shell, "Move", "5+");
    applied(&mut shell, "Move", "150", &mut history);
    // The part now stands 150 mm further along x.
    let centre = Vec3::new(200.0, 30.0, 20.0);
    shell.press_key(Key::Q);
    shell.click_at(screen(&shell, centre));
    shell.click_at(screen(&shell, centre + Vec3::new(25.0, 0.0, 0.0)));
    shell.move_pointer(screen(&shell, centre + Vec3::new(0.0, 25.0, 0.0)));
    refused(&mut shell, "Rotate", "9q");
    applied(&mut shell, "Rotate", "90", &mut history);
    shell.click_menu_command("menu-tools", AppCommand::Scale);
    refused(&mut shell, "Scale", "1");
    applied(&mut shell, "Scale", "2", &mut history);

    // Undo revisits every earlier document in turn, Redo every later one.
    shell.press_key(Key::Escape);
    for expected in history.iter().rev().skip(1) {
        shell.key(Key::Z, ctrl());
        assert_eq!(&shell.app().canonical_digest(), expected, "Undo walks back");
    }
    assert!(!shell.app().can_undo(), "the session made no hidden steps");
    for expected in history.iter().skip(1) {
        shell.key(Key::Y, ctrl());
        assert_eq!(
            &shell.app().canonical_digest(),
            expected,
            "Redo walks forward"
        );
    }

    let saved = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    assert_ne!(shell.app().canonical_digest(), saved);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), saved);
}

/// Presses a shortcut whose command cannot run now and checks it says why
/// without touching the document.
fn shortcut_refused(shell: &mut Shell, key: Key, modifiers: Modifiers, reason_key: &str) {
    let before = DocumentState::of(shell);
    shell.key(key, modifiers);
    assert_unchanged(
        shell,
        &before,
        &format!("{key:?}"),
        "an unavailable shortcut",
    );
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text(reason_key)
    );
}

/// Runs a shortcut and checks it made exactly one new Undo step.
fn shortcut_applied(shell: &mut Shell, key: Key, modifiers: Modifiers, history: &mut Vec<String>) {
    let undo = shell.app().undo_step_count();
    shell.key(key, modifiers);
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "{key:?}: one Undo step: {}",
        shell.app().action_digest()
    );
    history.push(shell.app().canonical_digest());
}

#[test]
fn a_group_is_built_opened_and_taken_apart_by_shortcuts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("groups.ketchup");
    let mut shell = Shell::with_dialogs(save_open_dialogs(&path));
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let mut history = vec![shell.app().canonical_digest()];
    shortcut_refused(
        &mut shell,
        Key::Z,
        ctrl(),
        "command-unavailable-nothing-to-undo",
    );

    // A copy of the box beside it, both grouped.
    shell.click_at(shell.top_face_centre(1));
    shortcut_applied(&mut shell, Key::D, ctrl(), &mut history);
    shell.key(Key::Z, Modifiers::SHIFT);
    shell.key(Key::A, ctrl());
    shortcut_applied(&mut shell, Key::G, ctrl(), &mut history);
    shell.press_key(Key::Escape);

    // Once the exact faces are shown (one body, both parts share it) a click
    // picks the part it lands on.
    assert!(shell.wait_until(|shell| {
        shell.step();
        shell.app().exact_render_body_count() == 1
    }));
    shell.click_at(shell.top_face_centre(1));
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    // Inside the opened group a click picks one part; changing what the group
    // holds is refused by name until the group is left again.
    shell.double_click_at(shell.top_face_centre(2));
    assert_eq!(shell.app().edit_context_depth(), 1);
    shell.click_at(shell.top_face_centre(2));
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    shortcut_refused(
        &mut shell,
        Key::Delete,
        Modifiers::NONE,
        "command-unavailable-open-group",
    );
    shortcut_refused(&mut shell, Key::D, ctrl(), "command-unavailable-open-group");
    shell.press_key(Key::Escape);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().edit_context_depth(), 0);
    // A viewport click picks a part, so the group itself is picked in the
    // outliner, then taken apart by its shortcut.
    shell.click_row(&shell.catalog().format(
        "outliner-group",
        &BTreeMap::from([("name", "Group 1".to_owned()), ("count", "2".to_owned())]),
    ));
    shortcut_applied(&mut shell, Key::G, ctrl_shift(), &mut history);
    assert_eq!(shell.app().group_count(), 0);
    assert_eq!(shell.app().selected_occurrence_count(), 2);
    shell.press_key(Key::Escape);
    shell.click_at(shell.top_face_centre(2));
    shortcut_applied(&mut shell, Key::Delete, Modifiers::NONE, &mut history);
    assert_eq!(shell.app().occurrence_count(), 1);

    for expected in history.iter().rev().skip(1) {
        shell.key(Key::Z, ctrl());
        assert_eq!(&shell.app().canonical_digest(), expected, "Undo walks back");
    }
    for expected in history.iter().skip(1) {
        shell.key(Key::Y, ctrl());
        assert_eq!(
            &shell.app().canonical_digest(),
            expected,
            "Redo walks forward"
        );
    }
    let saved = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), saved);
}

/// Starts one operation on the selected default box and leaves it half done:
/// its preview showing and a value half typed.
fn start_interrupted(shell: &mut Shell, tool: &str) {
    let corner = Vec3::new(20.0, 20.0, 20.0);
    let base = Vec3::new(80.0, 50.0, 20.0);
    match tool {
        "Move" => {
            shell.press_key(Key::M);
            shell.click_at(screen(shell, base));
            shell.move_pointer(screen(shell, base + Vec3::new(40.0, 0.0, 0.0)));
        }
        "Rotate" => {
            shell.press_key(Key::Q);
            shell.click_at(screen(shell, base));
            shell.click_at(screen(shell, base + Vec3::new(25.0, 0.0, 0.0)));
            shell.move_pointer(screen(shell, base + Vec3::new(0.0, 25.0, 0.0)));
        }
        "Scale" => shell.click_menu_command("menu-tools", AppCommand::Scale),
        "Rectangle" => {
            shell.press_key(Key::R);
            shell.click_at(screen(shell, corner));
            shell.move_pointer(screen(shell, corner + Vec3::new(30.0, 10.0, 0.0)));
        }
        "Push/Pull" => {
            shell.move_pointer(shell.top_face_centre(1));
            shell.press_key(Key::P);
            shell.click_at(shell.top_face_centre(1));
            shell.move_pointer(screen(shell, Vec3::new(50.0, 30.0, 30.0)));
        }
        _ => unreachable!(),
    }
    type_value(shell, "4");
}

#[test]
fn an_interrupted_operation_leaves_document_selection_and_history_as_they_were() {
    let mut shell = Shell::new();
    // One step to undo and one to redo, so an interruption that reaches the
    // history shows.
    shell.click_at(shell.top_face_centre(1));
    shell.key(Key::D, ctrl());
    shell.key(Key::D, ctrl());
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().undo_step_count(), 1);
    assert_eq!(shell.app().redo_step_count(), 1);

    let mut failures = Vec::new();
    for tool in ["Move", "Rotate", "Scale", "Rectangle", "Push/Pull"] {
        for how in ["Escape", "Ctrl+Z", "switching to Line"] {
            shell.press_key(Key::Escape);
            shell.press_key(Key::Escape);
            shell.press_key(Key::Space);
            shell.click_at(shell.top_face_centre(1));
            let before = DocumentState::of(&shell);
            start_interrupted(&mut shell, tool);
            match how {
                "Escape" => shell.press_key(Key::Escape),
                "Ctrl+Z" => shell.key(Key::Z, ctrl()),
                _ => shell.click_command(AppCommand::Line),
            }
            // Nothing of the interrupted operation is left to confirm.
            shell.press_key(Key::Enter);
            if DocumentState::of(&shell) != before {
                failures.push(format!(
                    "{tool} interrupted by {how} changed the document: {}",
                    shell.app().action_digest()
                ));
                shell.press_key(Key::Escape);
                while shell.app().undo_step_count() > before.undo_steps {
                    shell.key(Key::Z, ctrl());
                }
                while shell.app().undo_step_count() < before.undo_steps {
                    shell.key(Key::Y, ctrl());
                }
            } else if !shell.app().value_input().is_empty() {
                failures.push(format!("{tool} interrupted by {how} kept the typed value"));
            }
            if how != "switching to Line"
                && !(shell.app().selected_occurrence_count() == 1
                    && shell
                        .app()
                        .occurrence_is_selected(ketchup_model::document::OccurrenceId(1)))
            {
                failures.push(format!("{tool} interrupted by {how} lost the selection"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");

    // Copy changes nothing; Paste and Cut are one Undo step each, and Undo
    // and Redo walk them both ways.
    shell.press_key(Key::Escape);
    shell.press_key(Key::Escape);
    shell.press_key(Key::Space);
    shell.click_at(shell.top_face_centre(1));
    let parts = shell.app().occurrence_count();
    let mut history = vec![shell.app().canonical_digest()];
    let before = DocumentState::of(&shell);
    shell.key(Key::C, ctrl());
    assert_unchanged(&shell, &before, "Ctrl+C", "copying");
    shortcut_applied(&mut shell, Key::V, ctrl(), &mut history);
    assert_eq!(shell.app().occurrence_count(), parts + 1);
    shortcut_applied(&mut shell, Key::X, ctrl(), &mut history);
    assert_eq!(shell.app().occurrence_count(), parts);
    for expected in history.iter().rev().skip(1) {
        shell.key(Key::Z, ctrl());
        assert_eq!(&shell.app().canonical_digest(), expected, "Undo walks back");
    }
    for expected in history.iter().skip(1) {
        shell.key(Key::Y, ctrl());
        assert_eq!(
            &shell.app().canonical_digest(),
            expected,
            "Redo walks forward"
        );
    }
}
