//! The shared tool contract applied to Move and Rotate, the tools whose
//! behaviour is already settled; a failure here is a fault of the contract
//! checks, not of the tools.

use crate::harness::Shell;
use crate::harness::contract::{
    ToolGesture, assert_preview_then_escape_change_nothing, assert_tool_contract,
};
use eframe::egui::Pos2;
use ketchup_app::AppCommand;
use ketchup_interaction::Vec3;

fn box_shell(dialogs: ketchup_app::dialogs::ScriptedFileDialogs) -> Shell {
    Shell::with_dialogs(dialogs)
}

fn another_client_adds_a_box(shell: &mut Shell) {
    assert!(shell.app_mut().create_box());
}

fn select_box_and(shell: &mut Shell, tool: AppCommand) {
    shell.click_at(shell.top_face_centre(1));
    shell.click_command(tool);
}

fn screen(shell: &Shell, point: Vec3) -> Pos2 {
    shell.app().project_to_screen(point, shell.viewport_rect())
}

fn box_centre(shell: &Shell) -> (Vec3, Vec3) {
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    (origin + size * 0.5, size)
}

fn move_destination(shell: &Shell) -> Pos2 {
    shell.top_face_centre(1) + eframe::egui::Vec2::new(80.0, 0.0)
}

#[test]
fn move_click_move_click_keeps_the_tool_contract() {
    assert_tool_contract(
        &box_shell,
        &ToolGesture {
            name: "Move",
            arm: &|shell| select_box_and(shell, AppCommand::Move),
            preview: &|shell| {
                shell.click_at(shell.top_face_centre(1));
                shell.move_pointer(move_destination(shell));
            },
            // The preview moves nothing, so the destination is still measured
            // from the box where it was armed.
            commit: &|shell| shell.click_at(move_destination(shell)),
        },
        AppCommand::Select,
        &another_client_adds_a_box,
    );
}

/// The checks must catch a tool that writes the document before its commit.
#[test]
#[should_panic(expected = "Move committed early: the preview must leave the document unchanged")]
fn a_gesture_that_commits_during_its_preview_breaks_the_contract() {
    assert_preview_then_escape_change_nothing(
        &mut Shell::new(),
        &ToolGesture {
            name: "Move committed early",
            arm: &|shell| select_box_and(shell, AppCommand::Move),
            preview: &|shell| {
                shell.click_at(shell.top_face_centre(1));
                shell.click_at(move_destination(shell));
            },
            commit: &|_| {},
        },
    );
}

fn rotate_arm_end(shell: &Shell) -> Pos2 {
    let (centre, size) = box_centre(shell);
    screen(shell, centre + Vec3::new(0.0, size.x * 0.25, 0.0))
}

#[test]
fn rotate_click_move_click_keeps_the_tool_contract() {
    assert_tool_contract(
        &box_shell,
        &ToolGesture {
            name: "Rotate",
            arm: &|shell| select_box_and(shell, AppCommand::Rotate),
            preview: &|shell| {
                let (centre, size) = box_centre(shell);
                shell.click_at(screen(shell, centre));
                shell.click_at(screen(shell, centre + Vec3::new(size.x * 0.25, 0.0, 0.0)));
                shell.move_pointer(rotate_arm_end(shell));
            },
            commit: &|shell| shell.click_at(rotate_arm_end(shell)),
        },
        AppCommand::Select,
        &another_client_adds_a_box,
    );
}
