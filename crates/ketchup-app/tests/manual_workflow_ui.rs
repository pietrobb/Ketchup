//! One manual modelling session driven by keyboard shortcuts, clicks and typed
//! values: a rectangle drawn on a face, pushed in as a pocket, the part moved,
//! turned and scaled. Every tool first refuses an invalid value without
//! touching the document, every step is one Undo step, Undo walks back through
//! each earlier document and Redo forward again, and the result survives Save
//! and Open.

use crate::harness::contract::{DocumentState, assert_unchanged, save_open_dialogs};
use crate::harness::{Shell, ctrl};
use eframe::egui::{Key, Pos2};
use ketchup_app::AppCommand;
use ketchup_interaction::Vec3;

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
