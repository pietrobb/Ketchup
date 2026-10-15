//! Scale grips on the selection box: a corner scales evenly, an edge along
//! two axes and a face along one, each about the opposite grip or, with Ctrl,
//! about the centre; click-move-click; typed factors per axis and lengths.

use crate::harness::contract::{ToolGesture, assert_tool_contract};
use crate::harness::{Shell, ctrl};
use eframe::egui::{Key, Modifiers, Pos2};
use ketchup_app::AppCommand;
use ketchup_interaction::Vec3;
use std::path::PathBuf;

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

/// The default box selected, Scale armed and an exact worker connected, so
/// the scaled solid arrives on screen. Returns the box `origin` and `size`.
fn armed_scale_shell() -> (Shell, Vec3, Vec3) {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    shell.click_at(shell.top_face_centre(1));
    shell.click_menu_command("menu-tools", AppCommand::Scale);
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    (shell, origin, size)
}

/// The point of the box at `side` (-1, 0 or 1 per world axis).
fn box_point(origin: Vec3, size: Vec3, side: [f64; 3]) -> Vec3 {
    let (origin, size) = (origin.to_array(), size.to_array());
    Vec3::from(std::array::from_fn(|axis| {
        origin[axis] + size[axis] * (side[axis] + 1.0) * 0.5
    }))
}

fn screen(shell: &Shell, point: Vec3) -> Pos2 {
    shell.app().project_to_screen(point, shell.viewport_rect())
}

/// Where the pointer must go for the grip at `grip` to scale by `factor`
/// about `pivot`, measured along the screen line between them.
fn scaled_pointer(shell: &Shell, pivot: Vec3, grip: Vec3, factor: f32) -> Pos2 {
    let (pivot, grip) = (screen(shell, pivot), screen(shell, grip));
    pivot + (grip - pivot) * factor
}

/// The box `origin` + `size` stretched by `factors` about `pivot`.
fn stretched_about(origin: Vec3, size: Vec3, pivot: Vec3, factors: [f64; 3]) -> [[f64; 3]; 2] {
    let pivot = pivot.to_array();
    let corner = |point: Vec3| -> [f64; 3] {
        let point = point.to_array();
        std::array::from_fn(|axis| pivot[axis] + (point[axis] - pivot[axis]) * factors[axis])
    };
    [corner(origin), corner(origin + size)]
}

/// Waits until the one exact body on screen has the `expected` bounds.
fn assert_exact_bounds(shell: &mut Shell, expected: [[f64; 3]; 2]) {
    let matches = |actual: &[[[f64; 3]; 2]]| {
        actual.len() == 1
            && (0..2).all(|corner| {
                (0..3).all(|axis| (actual[0][corner][axis] - expected[corner][axis]).abs() < 1.0e-3)
            })
    };
    let arrived = shell.wait_until(|shell| {
        shell.step();
        matches(&shell.app().exact_render_bounds())
    });
    assert!(
        arrived,
        "exact bounds {:?}, expected {expected:?}: {}",
        shell.app().exact_render_bounds(),
        shell.app().action_digest()
    );
}

/// Drags the grip at `side` to the pointer that scales by 1.5, with
/// `modifiers` held, and returns the factors its preview showed.
fn drag_grip(shell: &mut Shell, side: [f64; 3], pivot: Vec3, modifiers: Modifiers) -> [f64; 3] {
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let grip = box_point(origin, size, side);
    let from = screen(shell, grip);
    let to = scaled_pointer(shell, pivot, grip, 1.5);
    let undo = shell.app().undo_step_count();
    shell.move_pointer(from);
    shell.set_primary_button(from, modifiers, true);
    shell.move_pointer(to);
    let factors = shell
        .app()
        .scale_preview_factors()
        .expect("a grip drag previews its factors");
    shell.set_primary_button(to, Modifiers::NONE, false);
    shell.settle();
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "one grip drag, one Undo step: {}",
        shell.app().action_digest()
    );
    factors
}

fn assert_factors(actual: [f64; 3], expected: [f64; 3]) {
    assert!(
        (0..3).all(|axis| (actual[axis] - expected[axis]).abs() < 0.02),
        "factors {actual:?}, expected {expected:?}"
    );
}

#[test]
fn a_corner_grip_scales_evenly_about_the_opposite_corner() {
    let (mut shell, origin, size) = armed_scale_shell();
    let factors = drag_grip(&mut shell, [1.0; 3], origin, Modifiers::NONE);
    assert_factors(factors, [1.5; 3]);
    assert_exact_bounds(&mut shell, stretched_about(origin, size, origin, factors));
}

#[test]
fn a_face_grip_stretches_one_axis_about_the_opposite_face_and_ctrl_about_the_centre() {
    let (mut shell, origin, size) = armed_scale_shell();
    let opposite_face = box_point(origin, size, [-1.0, 0.0, 0.0]);
    let factors = drag_grip(&mut shell, [1.0, 0.0, 0.0], opposite_face, Modifiers::NONE);
    assert_factors(factors, [1.5, 1.0, 1.0]);
    assert_exact_bounds(&mut shell, stretched_about(origin, size, origin, factors));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_exact_bounds(&mut shell, stretched_about(origin, size, origin, [1.0; 3]));
    let centre = box_point(origin, size, [0.0; 3]);
    let factors = drag_grip(&mut shell, [1.0, 0.0, 0.0], centre, ctrl());
    assert_factors(factors, [1.5, 1.0, 1.0]);
    assert_exact_bounds(&mut shell, stretched_about(origin, size, centre, factors));
}

#[test]
fn an_edge_grip_stretches_the_two_axes_it_is_off_centre_in() {
    let (mut shell, origin, size) = armed_scale_shell();
    let opposite_edge = box_point(origin, size, [-1.0, -1.0, 0.0]);
    let factors = drag_grip(&mut shell, [1.0, 1.0, 0.0], opposite_edge, Modifiers::NONE);
    assert_factors(factors, [1.5, 1.5, 1.0]);
    assert_exact_bounds(&mut shell, stretched_about(origin, size, origin, factors));
}

#[test]
fn a_clicked_grip_previews_and_takes_a_typed_length() {
    let (mut shell, origin, size) = armed_scale_shell();
    let grip = box_point(origin, size, [1.0, 0.0, 0.0]);
    let undo = shell.app().undo_step_count();

    shell.click_at(screen(&shell, grip));
    // Shrinking keeps the pointer inside the viewport, which a hover needs.
    shell.move_pointer(scaled_pointer(&shell, origin, grip, 0.8));
    assert_factors(
        shell
            .app()
            .scale_preview_factors()
            .expect("a clicked grip previews as the pointer moves"),
        [0.8, 1.0, 1.0],
    );
    assert_eq!(shell.app().undo_step_count(), undo, "a preview is no edit");

    // A length with a unit sets the size of the one axis the grip drives.
    shell.type_text(&format!("{}", size.x * 1.5));
    shell.type_text(" mm");
    assert_eq!(shell.app().value_input(), format!("{} mm", size.x * 1.5));
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "{}",
        shell.app().action_digest()
    );
    assert_exact_bounds(
        &mut shell,
        stretched_about(origin, size, origin, [1.5, 1.0, 1.0]),
    );
}

#[test]
fn a_typed_factor_per_axis_scales_each_axis_about_the_centre() {
    let (mut shell, origin, size) = armed_scale_shell();
    let undo = shell.app().undo_step_count();
    // The decimal comma stays free because axes are separated by semicolons.
    shell.type_text("1,5;2;1");
    assert_eq!(shell.app().value_input(), "1,5;2;1");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "{}",
        shell.app().action_digest()
    );
    let centre = box_point(origin, size, [0.0; 3]);
    assert_exact_bounds(
        &mut shell,
        stretched_about(origin, size, centre, [1.5, 2.0, 1.0]),
    );

    // A length needs one driven axis; without a lock it is refused by name.
    let before = shell.app().canonical_digest();
    shell.click_command(AppCommand::Select);
    shell.click_menu_command("menu-tools", AppCommand::Scale);
    shell.type_text("200");
    shell.type_text(" mm");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("digest-scale-length-needs-one-axis")
    );
    assert_eq!(shell.app().canonical_digest(), before);
}

fn grip_gesture_shell(dialogs: ketchup_app::dialogs::ScriptedFileDialogs) -> Shell {
    Shell::with_dialogs(dialogs)
}

fn corner_grip(shell: &Shell) -> (Vec3, Vec3) {
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    (origin, box_point(origin, size, [1.0; 3]))
}

#[test]
fn a_grip_click_move_click_keeps_the_tool_contract() {
    assert_tool_contract(
        &grip_gesture_shell,
        &ToolGesture {
            name: "Scale grip",
            arm: &|shell| {
                shell.click_at(shell.top_face_centre(1));
                shell.click_menu_command("menu-tools", AppCommand::Scale);
            },
            preview: &|shell| {
                let (pivot, grip) = corner_grip(shell);
                shell.click_at(screen(shell, grip));
                shell.move_pointer(scaled_pointer(shell, pivot, grip, 0.7));
                assert_factors(
                    shell.app().scale_preview_factors().expect("a grip preview"),
                    [0.7; 3],
                );
            },
            // The preview changes nothing, so the grip is still where it was.
            commit: &|shell| {
                let (pivot, grip) = corner_grip(shell);
                shell.click_at(scaled_pointer(shell, pivot, grip, 0.7));
            },
        },
        AppCommand::Select,
        &|shell| assert!(shell.app_mut().create_box()),
    );
}
