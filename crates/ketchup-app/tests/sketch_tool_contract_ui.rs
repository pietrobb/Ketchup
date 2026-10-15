//! The shared tool contract applied to every Sketch tool, drawn on the ground
//! beside the default box by click-move-click.

use crate::harness::Shell;
use crate::harness::contract::{ToolGesture, assert_tool_contract};
use eframe::egui::Pos2;
use ketchup_app::AppCommand;
use ketchup_interaction::Vec3;

fn sketch_shell(dialogs: ketchup_app::dialogs::ScriptedFileDialogs) -> Shell {
    Shell::with_dialogs(dialogs)
}

fn another_client_adds_a_box(shell: &mut Shell) {
    assert!(shell.app_mut().create_box());
}

/// A ground point in front of the default box, clear of its edges.
fn at(shell: &Shell, x: f64, y: f64) -> Pos2 {
    shell.app().viewport_position(Vec3::new(x, y, 0.0)).unwrap()
}

fn click(shell: &mut Shell, x: f64, y: f64) {
    shell.click_at(at(shell, x, y));
}

fn hover(shell: &mut Shell, x: f64, y: f64) {
    shell.move_pointer(at(shell, x, y));
}

fn assert_sketch_contract(name: &str, tool: AppCommand, clicks: &[[f64; 2]]) {
    let (placed, last) = clicks.split_at(clicks.len() - 1);
    let [x, y] = last[0];
    assert_tool_contract(
        &sketch_shell,
        &ToolGesture {
            name,
            arm: &|shell| shell.click_command(tool),
            preview: &|shell| {
                for [x, y] in placed {
                    click(shell, *x, *y);
                }
                hover(shell, x, y);
            },
            commit: &|shell| click(shell, x, y),
        },
        AppCommand::Select,
        &another_client_adds_a_box,
    );
}

#[test]
fn line_click_move_click_keeps_the_tool_contract() {
    assert_sketch_contract("Line", AppCommand::Line, &[[10.0, -40.0], [60.0, -40.0]]);
}

#[test]
fn rectangle_click_move_click_keeps_the_tool_contract() {
    assert_sketch_contract(
        "Rectangle",
        AppCommand::Rectangle,
        &[[10.0, -60.0], [60.0, -30.0]],
    );
}

#[test]
fn circle_click_move_click_keeps_the_tool_contract() {
    assert_sketch_contract(
        "Circle",
        AppCommand::Circle,
        &[[35.0, -45.0], [50.0, -45.0]],
    );
}

#[test]
fn arc_click_move_click_keeps_the_tool_contract() {
    assert_sketch_contract(
        "Arc",
        AppCommand::Arc,
        &[[10.0, -40.0], [60.0, -40.0], [35.0, -55.0]],
    );
}

#[test]
fn polygon_click_move_click_keeps_the_tool_contract() {
    assert_sketch_contract(
        "Polygon",
        AppCommand::Polygon,
        &[[35.0, -45.0], [50.0, -45.0]],
    );
}

#[test]
fn ellipse_click_move_click_keeps_the_tool_contract() {
    assert_sketch_contract(
        "Ellipse",
        AppCommand::Ellipse,
        &[[35.0, -45.0], [55.0, -45.0], [35.0, -35.0]],
    );
}

#[test]
fn spline_click_move_click_keeps_the_tool_contract() {
    // The last click lands on the first point and closes the spline.
    assert_sketch_contract(
        "Spline",
        AppCommand::Spline,
        &[
            [10.0, -40.0],
            [50.0, -40.0],
            [50.0, -60.0],
            [10.0, -60.0],
            [10.0, -40.0],
        ],
    );
}
