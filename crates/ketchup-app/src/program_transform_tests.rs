use super::*;
use ketchup_model::document::RuleProgramSource;

const PARTS: &str = "box(\"a\", (20, 30, 40), at = (100, 0, 5))\n\
                     x = 300\n\
                     box(\"b\", (200, 50, 10), at = (0, x, 0))\n";

fn app() -> KetchupApp {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        RuleProgramSource {
            variants: Default::default(),
            file_name: "parts.star".into(),
            source: PARTS.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    app
}

fn select(app: &mut KetchupApp, name: &str) {
    let id = app
        .document
        .current()
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap()
        .id();
    app.select_from_outliner(InstancePath::root(id), false);
}

fn source(app: &KetchupApp) -> String {
    app.document
        .current_rule_program()
        .expect("the program still owns the document")
        .source
        .clone()
}

/// World minimum corner and size of the selected part.
fn placed(app: &KetchupApp) -> ([f64; 3], [f64; 3]) {
    let measures = app.selection_measures().unwrap();
    (measures.world_min_mm, measures.world_size_mm)
}

fn assert_close(actual: [f64; 3], expected: [f64; 3]) {
    assert!(
        (0..3).all(|axis| (actual[axis] - expected[axis]).abs() < 1.0e-6),
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn moving_a_program_part_moves_it_in_the_program_as_one_undo_step() {
    let mut app = app();
    let steps = app.undo_step_count();
    select(&mut app, "a");
    assert!(app.move_selected(Vec3::new(25.0, -10.0, 0.0)));
    // The literal position is edited in place: 100 + 25, 0 - 10, 5.
    assert_eq!(
        source(&app),
        PARTS.replace("at = (100, 0, 5)", "at = (125, -10, 5)")
    );
    assert_close(placed(&app).0, [125.0, -10.0, 5.0]);
    assert_eq!(app.undo_step_count(), steps + 1);

    // A typed distance corrects the move instead of adding to it.
    let (selection, previous) = app.current_move_correction().unwrap();
    assert!(app.correct_move_delta(selection, previous, Vec3::new(40.0, 0.0, 0.0), false));
    assert_eq!(
        source(&app),
        PARTS.replace("at = (100, 0, 5)", "at = (140, 0, 5)")
    );
    assert_close(placed(&app).0, [140.0, 0.0, 5.0]);
    assert_eq!(app.undo_step_count(), steps + 1);

    assert!(app.undo());
    assert_eq!(source(&app), PARTS);
    select(&mut app, "a");
    assert_close(placed(&app).0, [100.0, 0.0, 5.0]);
}

#[test]
fn a_part_placed_by_an_expression_gets_a_move_line() {
    let mut app = app();
    select(&mut app, "b");
    assert!(app.move_selected(Vec3::new(0.0, 0.0, 12.5)));
    assert_eq!(
        source(&app),
        format!("{PARTS}move(\"b\", by=(0, 0, 12.5))\n")
    );
    // (0, 300, 0) raised by 12.5; the size stays 200 × 50 × 10.
    let (min, size) = placed(&app);
    assert_close(min, [0.0, 300.0, 12.5]);
    assert_close(size, [200.0, 50.0, 10.0]);
}

#[test]
fn turning_a_program_part_writes_a_rotate_line() {
    let mut app = app();
    let steps = app.undo_step_count();
    select(&mut app, "a");
    // A quarter turn about the vertical line through the part's centre (110, 15).
    assert!(app.rotate_selected_around(Vec3::new(110.0, 15.0, 25.0), Axis::Z, 90.0));
    assert_eq!(
        source(&app),
        format!("{PARTS}rotate(\"a\", axis=(0, 0, 1), angle=90, pivot=(110, 15, 5))\n")
    );
    // 20 × 30 becomes 30 × 20 about the same centre: x 110 ± 15, y 15 ± 10.
    let (min, size) = placed(&app);
    assert_close(min, [95.0, 5.0, 5.0]);
    assert_close(size, [30.0, 20.0, 40.0]);
    assert_eq!(app.undo_step_count(), steps + 1);

    assert!(app.correct_last_rotation(-90.0));
    assert!(
        source(&app).ends_with("rotate(\"a\", axis=(0, 0, -1), angle=90, pivot=(110, 15, 5))\n")
    );
    assert_eq!(app.undo_step_count(), steps + 1);
}
