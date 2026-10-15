//! Scale grips on the selection box: a corner scales evenly, an edge along
//! two axes and a face along one, each about the opposite grip or, with Ctrl,
//! about the centre; click-move-click; typed factors per axis and lengths;
//! several selected parts and a part picked inside an opened group.

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
    assert_exact_bodies(shell, &[expected]);
}

/// Waits until the exact bodies on screen have the `expected` bounds, in any order.
fn assert_exact_bodies(shell: &mut Shell, expected: &[[[f64; 3]; 2]]) {
    let close = |a: &[[f64; 3]; 2], b: &[[f64; 3]; 2]| {
        (0..2).all(|corner| (0..3).all(|axis| (a[corner][axis] - b[corner][axis]).abs() < 1.0e-3))
    };
    let matches = |actual: &[[[f64; 3]; 2]]| {
        actual.len() == expected.len()
            && expected
                .iter()
                .all(|bounds| actual.iter().any(|body| close(body, bounds)))
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

/// Two boxes with an exact worker connected; returns each box's `origin` and `size`.
fn two_box_shell() -> (Shell, [(Vec3, Vec3); 2]) {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    assert!(shell.app_mut().create_box());
    shell.settle();
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    let boxes = [1, 2].map(|id| shell.app().occurrence_box_geometry(id).unwrap());
    (shell, boxes)
}

fn type_factor(shell: &mut Shell, factor: &str) {
    let undo = shell.app().undo_step_count();
    shell.type_text(factor);
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().undo_step_count(),
        undo + 1,
        "one typed Scale, one Undo step: {}",
        shell.app().action_digest()
    );
}

#[test]
fn several_selected_parts_scale_about_their_shared_centre_in_one_undo_step() {
    let (mut shell, boxes) = two_box_shell();
    let low = Vec3::from(std::array::from_fn(|axis| {
        boxes[0].0.to_array()[axis].min(boxes[1].0.to_array()[axis])
    }));
    let high = Vec3::from(std::array::from_fn(|axis| {
        (boxes[0].0 + boxes[0].1).to_array()[axis].max((boxes[1].0 + boxes[1].1).to_array()[axis])
    }));
    let centre = (low + high) * 0.5;
    // Exact bounds are in each part's own frame, whose origin is the box origin.
    let local = |factor: f64| {
        boxes.map(|(origin, size)| {
            stretched_about(Vec3::new(0.0, 0.0, 0.0), size, centre - origin, [factor; 3])
        })
    };
    let (unscaled, scaled) = (local(1.0), local(2.0));
    assert_exact_bodies(&mut shell, &unscaled);

    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-tools", AppCommand::Scale);
    type_factor(&mut shell, "2");
    assert_exact_bodies(&mut shell, &scaled);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_exact_bodies(&mut shell, &unscaled);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_exact_bodies(&mut shell, &scaled);
}

#[test]
fn a_part_picked_inside_an_opened_group_scales_alone_and_the_group_stays_open() {
    let (mut shell, boxes) = two_box_shell();
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-model", AppCommand::Group);
    shell.double_click_at(shell.top_face_centre(2));
    assert_eq!(shell.app().edit_context_depth(), 1);
    shell.click_at(shell.top_face_centre(2));
    shell.click_menu_command("menu-tools", AppCommand::Scale);
    type_factor(&mut shell, "2");

    // Exact bounds are in each part's own frame, whose origin is the box origin.
    let zero = Vec3::new(0.0, 0.0, 0.0);
    let [(_, first_size), (_, second_size)] = boxes;
    let second_centre = box_point(zero, second_size, [0.0; 3]);
    assert_exact_bodies(
        &mut shell,
        &[
            stretched_about(zero, first_size, zero, [1.0; 3]),
            stretched_about(zero, second_size, second_centre, [2.0; 3]),
        ],
    );
    assert_eq!(
        shell.app().edit_context_depth(),
        1,
        "{}",
        shell.app().action_digest()
    );
}
