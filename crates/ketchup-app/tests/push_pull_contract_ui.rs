//! The shared tool contract applied to Push/Pull on the default box's top
//! face, by click-move-click and by press-drag-release, and the typed depths
//! it takes, corrects or refuses.

use crate::harness::Shell;
use crate::harness::contract::{
    DocumentState, ToolGesture, assert_tool_contract, assert_unchanged,
};
use eframe::egui::{Key, Modifiers, Pos2, Vec2};
use ketchup_app::AppCommand;
use ketchup_interaction::Vec3;

fn box_shell(dialogs: ketchup_app::dialogs::ScriptedFileDialogs) -> Shell {
    Shell::with_dialogs(dialogs)
}

fn another_client_adds_a_box(shell: &mut Shell) {
    assert!(shell.app_mut().create_box());
}

/// The screen direction in which the default box's top face moves up.
fn up(shell: &Shell) -> Vec2 {
    let top = shell.top_face_centre(1);
    let above = shell
        .app()
        .viewport_position(Vec3::new(50.0, 30.0, 21.0))
        .unwrap();
    (above - top).normalized()
}

fn pulled(shell: &Shell, pixels: f32) -> Pos2 {
    shell.top_face_centre(1) + up(shell) * pixels
}

fn arm(shell: &mut Shell) {
    shell.move_pointer(shell.top_face_centre(1));
    shell.press_key(Key::P);
}

#[test]
fn push_pull_click_move_click_keeps_the_tool_contract() {
    assert_tool_contract(
        &box_shell,
        &ToolGesture {
            name: "Push/Pull",
            arm: &arm,
            preview: &|shell| {
                shell.click_at(shell.top_face_centre(1));
                shell.move_pointer(pulled(shell, 80.0));
            },
            commit: &|shell| shell.click_at(pulled(shell, 80.0)),
        },
        AppCommand::Select,
        &another_client_adds_a_box,
    );
}

#[test]
fn push_pull_press_drag_release_keeps_the_tool_contract() {
    assert_tool_contract(
        &box_shell,
        &ToolGesture {
            name: "Push/Pull drag",
            arm: &arm,
            preview: &|shell| {
                let top = shell.top_face_centre(1);
                shell.set_primary_button(top, Modifiers::NONE, true);
                shell.move_pointer(pulled(shell, 80.0));
            },
            commit: &|shell| {
                let to = pulled(shell, 80.0);
                shell.set_primary_button(to, Modifiers::NONE, false);
            },
        },
        AppCommand::Select,
        &another_client_adds_a_box,
    );
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

fn start_pull(shell: &mut Shell) {
    arm(shell);
    shell.click_at(shell.top_face_centre(1));
    shell.move_pointer(pulled(shell, 80.0));
}

#[test]
fn a_typed_depth_pulls_or_pushes_the_face_and_a_second_value_corrects_it_in_the_same_step() {
    for (input, height) in [("7", 27.0), ("-5", 15.0), ("1 cm", 30.0)] {
        let mut shell = Shell::new();
        start_pull(&mut shell);
        type_value(&mut shell, input);
        shell.press_key(Key::Enter);
        assert_eq!(shell.app().document_height_mm(), height, "typed {input:?}");
        assert_eq!(shell.app().undo_step_count(), 1, "typed {input:?}");
    }

    let mut shell = Shell::new();
    start_pull(&mut shell);
    type_value(&mut shell, "7");
    shell.press_key(Key::Enter);
    type_value(&mut shell, "15");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_height_mm(), 35.0);
    assert_eq!(
        shell.app().undo_step_count(),
        1,
        "a correction replaces the Push/Pull, it is not a second step"
    );
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().document_height_mm(), 20.0);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().document_height_mm(), 35.0);
}

#[test]
fn a_typed_depth_of_zero_or_unreadable_is_refused_by_name_and_keeps_the_gesture() {
    let refused = "digest-push-pull-invalid-distance";
    for input in ["0", "5+", "0 cm"] {
        let mut shell = Shell::new();
        start_pull(&mut shell);
        let before = DocumentState::of(&shell);
        type_value(&mut shell, input);
        shell.press_key(Key::Enter);
        let what = format!("Push/Pull typed {input:?}");
        assert_unchanged(&shell, &before, &what, "a refused depth");
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text(refused),
            "{what} is refused by name"
        );
        assert!(
            shell.app().push_pull_click_anchor_active(),
            "{what} keeps the gesture open for a corrected value"
        );
    }

    // An unreadable correction keeps the Push/Pull it would have replaced.
    let mut shell = Shell::new();
    start_pull(&mut shell);
    type_value(&mut shell, "7");
    shell.press_key(Key::Enter);
    let committed = DocumentState::of(&shell);
    type_value(&mut shell, "5+");
    shell.press_key(Key::Enter);
    assert_unchanged(
        &shell,
        &committed,
        "Push/Pull corrected to 5+",
        "a refused correction",
    );
    assert_eq!(shell.app().action_digest(), shell.catalog().text(refused));
}

#[test]
fn a_pull_snaps_to_the_height_of_a_face_on_another_part() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 0.0, 40.0)));
    shell.settle();
    shell.click_menu_command("menu-model", AppCommand::MakeUnique);
    shell.click_command(AppCommand::ZoomFit);
    shell.press_key(Key::Escape);
    let other_top = shell
        .app()
        .viewport_position(Vec3::new(200.0, 30.0, 60.0))
        .unwrap();
    arm(&mut shell);
    shell.click_at(shell.top_face_centre(1));
    shell.move_pointer(other_top);
    shell.click_at(other_top);
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    assert_eq!(
        (origin.z, size.z),
        (0.0, 60.0),
        "the top face lands level with the other part's top"
    );
}

/// Whether `digest` is the refusal of a Push/Pull that cannot go so far,
/// whatever distance it names.
fn names_the_refusal(shell: &Shell, digest: &str) -> bool {
    let marker = "\u{1}";
    let text = shell.catalog().format(
        "digest-push-pull-not-possible",
        &std::collections::BTreeMap::from([("distance", marker.to_owned())]),
    );
    let (prefix, suffix) = text.split_once(marker).unwrap();
    digest.starts_with(prefix) && digest.ends_with(suffix) && digest.len() > text.len()
}

#[test]
fn a_push_through_the_whole_part_is_refused_by_name_whether_typed_or_dragged() {
    for input in ["-20", "-25"] {
        let mut shell = Shell::new();
        start_pull(&mut shell);
        let before = DocumentState::of(&shell);
        type_value(&mut shell, input);
        shell.press_key(Key::Enter);
        let what = format!("Push/Pull typed {input:?}");
        assert_unchanged(&shell, &before, &what, "a push through the part");
        assert!(
            names_the_refusal(&shell, shell.app().action_digest()),
            "{what} is refused by name, not {:?}",
            shell.app().action_digest()
        );
    }

    let mut shell = Shell::new();
    arm(&mut shell);
    shell.click_at(shell.top_face_centre(1));
    let before = DocumentState::of(&shell);
    shell.move_pointer(pulled(&shell, -300.0));
    let refusal = shell.app().action_digest().to_owned();
    shell.click_at(pulled(&shell, -300.0));
    assert_unchanged(&shell, &before, "Push/Pull dragged through", "the click");
    assert!(
        names_the_refusal(&shell, &refusal),
        "the preview names why it cannot go through, not {refusal:?}"
    );
}
