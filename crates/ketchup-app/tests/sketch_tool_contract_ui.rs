//! The shared tool contract applied to every Sketch tool, drawn on the ground
//! beside the default box by click-move-click and by press-drag-release, and
//! the typed sizes every Sketch tool takes or refuses.

use crate::harness::Shell;
use crate::harness::contract::{
    DocumentState, ToolGesture, assert_tool_contract, assert_unchanged,
};
use eframe::egui::{Key, Modifiers, Pos2};
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

/// Types `input` the way keys arrive: its first character opens the value
/// field, which then takes the rest, units and operators included.
fn type_value(shell: &mut Shell, input: &str) {
    let (first, rest) = input.split_at(1);
    shell.type_text(first);
    if !rest.is_empty() {
        shell.type_text(rest);
    }
    assert_eq!(shell.app().value_input(), input);
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

/// The first point is pressed and dragged to the second, held through the
/// preview, and released to commit, or to place the second point of a shape
/// that needs more; the remaining points are clicked.
fn assert_sketch_drag_contract(name: &str, tool: AppCommand, points: &[[f64; 2]]) {
    let [first, second] = [points[0], points[1]];
    let rest = &points[2..];
    let press_and_drag = |shell: &mut Shell| {
        let from = at(shell, first[0], first[1]);
        shell.move_pointer(from);
        shell.set_primary_button(from, Modifiers::NONE, true);
        hover(shell, second[0], second[1]);
    };
    let release = |shell: &mut Shell| {
        let to = at(shell, second[0], second[1]);
        shell.set_primary_button(to, Modifiers::NONE, false);
    };
    assert_tool_contract(
        &sketch_shell,
        &ToolGesture {
            name,
            arm: &|shell| shell.click_command(tool),
            preview: &|shell| {
                press_and_drag(shell);
                if let Some(([x, y], placed)) = rest.split_last() {
                    release(shell);
                    for [x, y] in placed {
                        click(shell, *x, *y);
                    }
                    hover(shell, *x, *y);
                }
            },
            commit: &|shell| match rest.last() {
                Some([x, y]) => click(shell, *x, *y),
                None => release(shell),
            },
        },
        AppCommand::Select,
        &another_client_adds_a_box,
    );
}

#[test]
fn every_sketch_tool_keeps_the_tool_contract_when_dragged() {
    for (name, tool, points) in [
        (
            "Line drag",
            AppCommand::Line,
            &[[10.0, -40.0], [60.0, -40.0]][..],
        ),
        (
            "Rectangle drag",
            AppCommand::Rectangle,
            &[[10.0, -60.0], [60.0, -30.0]],
        ),
        (
            "Circle drag",
            AppCommand::Circle,
            &[[35.0, -45.0], [50.0, -45.0]],
        ),
        (
            "Polygon drag",
            AppCommand::Polygon,
            &[[35.0, -45.0], [50.0, -45.0]],
        ),
        (
            "Arc drag",
            AppCommand::Arc,
            &[[10.0, -40.0], [60.0, -40.0], [35.0, -55.0]],
        ),
        (
            "Ellipse drag",
            AppCommand::Ellipse,
            &[[35.0, -45.0], [55.0, -45.0], [35.0, -35.0]],
        ),
        (
            "Spline drag",
            AppCommand::Spline,
            &[
                [10.0, -40.0],
                [50.0, -40.0],
                [50.0, -60.0],
                [10.0, -60.0],
                [10.0, -40.0],
            ],
        ),
    ] {
        assert_sketch_drag_contract(name, tool, points);
    }
}

#[test]
fn a_typed_size_of_zero_or_less_or_unreadable_is_refused_by_name_and_changes_nothing() {
    let length = "digest-sketch-invalid-length";
    let rectangle = "digest-rectangle-invalid-size";
    for (tool, placed, input, key) in [
        (AppCommand::Line, &[[10.0, -40.0]][..], "0", length),
        (AppCommand::Line, &[[10.0, -40.0]], "5+", length),
        (AppCommand::Circle, &[[35.0, -45.0]], "-5", length),
        (AppCommand::Polygon, &[[35.0, -45.0]], "0", length),
        (AppCommand::Ellipse, &[[35.0, -45.0]], "-5", length),
        (AppCommand::Spline, &[[10.0, -40.0]], "0", length),
        (
            AppCommand::Arc,
            &[[10.0, -40.0], [60.0, -40.0]],
            "0",
            length,
        ),
        (AppCommand::Rectangle, &[[10.0, -60.0]], "0;20", rectangle),
        (AppCommand::Rectangle, &[[10.0, -60.0]], "30", rectangle),
    ] {
        let mut shell = Shell::new();
        shell.click_command(tool);
        for [x, y] in placed {
            click(&mut shell, *x, *y);
        }
        hover(&mut shell, 40.0, -50.0);
        let before = DocumentState::of(&shell);
        type_value(&mut shell, input);
        shell.press_key(Key::Enter);
        let what = format!("{tool:?} typed {input:?}");
        assert_unchanged(&shell, &before, &what, "a refused size");
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text(key),
            "{what} is refused by name"
        );
    }
}

/// Width and depth of the newest drawn rectangle.
fn latest_rectangle_size(shell: &Shell) -> [f64; 2] {
    let (_, segments) = shell.app().latest_profile().unwrap();
    let corners: Vec<[f64; 2]> = segments
        .iter()
        .map(|segment| match segment {
            ketchup_model::document::ProfileSegment::Line { start_mm, .. } => *start_mm,
            other => panic!("a rectangle is straight lines, got {other:?}"),
        })
        .collect();
    let span = |axis: usize| {
        let values = corners.iter().map(|corner| corner[axis]);
        values.clone().fold(f64::MIN, f64::max) - values.fold(f64::MAX, f64::min)
    };
    [span(0), span(1)]
}

#[test]
fn a_typed_rectangle_size_takes_units_arithmetic_and_a_decimal_comma_per_side() {
    for (input, expected) in [
        ("30,20", [30.0, 20.0]),
        ("8 cm;6 cm", [80.0, 60.0]),
        ("2*20;12,5", [40.0, 12.5]),
        ("45x15", [45.0, 15.0]),
    ] {
        let mut shell = Shell::new();
        shell.click_command(AppCommand::Rectangle);
        click(&mut shell, 10.0, -60.0);
        hover(&mut shell, 60.0, -30.0);
        let revision = shell.app().document_revision();
        type_value(&mut shell, input);
        shell.press_key(Key::Enter);
        assert_eq!(
            shell.app().document_revision(),
            revision + 1,
            "{input:?}: {}",
            shell.app().action_digest()
        );
        let size = latest_rectangle_size(&shell);
        assert!(
            (0..2).all(|axis| (size[axis] - expected[axis]).abs() < 1.0e-9),
            "{input:?} drew {size:?}, expected {expected:?}"
        );
    }
}
