use super::selection_measures_ui::SelectionMeasures;
use super::*;
use egui_kittest::kittest::Queryable;

const PARTS: &str = "a = box(\"a\", (20, 30, 40), at = (100, 0, 5))\n\
    b = box(\"b\", (200, 50, 10), at = (0, 300, 0))\n\
    rotate(b, axis = (0, 0, 1), angle = 30)\n";

fn app() -> KetchupApp {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "measures.star".into(),
            source: PARTS.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    app
}

fn occurrence(app: &KetchupApp, name: &str) -> OccurrenceId {
    app.document
        .current()
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap()
        .id()
}

fn assert_close(actual: [f64; 3], expected: [f64; 3]) {
    for axis in 0..3 {
        assert!(
            (actual[axis] - expected[axis]).abs() < 1.0e-6,
            "{actual:?} != {expected:?}"
        );
    }
}

#[test]
fn selection_reports_size_position_and_exact_volume() {
    let mut app = app();
    assert_eq!(app.selection_measures(), None);
    let a = occurrence(&app, "a");
    let b = occurrence(&app, "b");

    // Before the exact solids exist the volume is pending, never guessed.
    app.selection.select_occurrence(a, false);
    assert_eq!(app.selection_measures().unwrap().volume_mm3, None);

    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let measures = app.selection_measures().unwrap();
    assert_eq!(measures.parts, 1);
    assert_eq!(measures.own_size_mm, None);
    assert_close(measures.world_min_mm, [100.0, 0.0, 5.0]);
    assert_close(measures.world_size_mm, [20.0, 30.0, 40.0]);
    let volume = measures.volume_mm3.unwrap();
    assert!((volume - 24_000.0).abs() < 1.0e-3, "{volume}");
    let catalog = app.catalog.clone();
    assert_eq!(
        app.selection_measure_lines(&measures),
        vec![
            catalog.format(
                "selection-measures-size",
                &BTreeMap::from([("size", "20 × 30 × 40".to_owned())])
            ),
            catalog.format(
                "selection-measures-position",
                &BTreeMap::from([
                    ("x", "100".to_owned()),
                    ("y", "0".to_owned()),
                    ("z", "5".to_owned())
                ])
            ),
            catalog.format(
                "selection-measures-volume",
                &BTreeMap::from([
                    ("litres", "0.024".to_owned()),
                    ("cubic", "0.0000".to_owned())
                ])
            ),
        ]
    );

    // A turned part keeps its own size; the world box grows around it.
    app.selection.select_occurrence(b, false);
    let measures = app.selection_measures().unwrap();
    let (sin, cos) = 30f64.to_radians().sin_cos();
    assert_close(measures.own_size_mm.unwrap(), [200.0, 50.0, 10.0]);
    assert_close(
        measures.world_size_mm,
        [200.0 * cos + 50.0 * sin, 200.0 * sin + 50.0 * cos, 10.0],
    );

    // Several parts: their common box and summed volume.
    app.selection.select_occurrence(a, true);
    let SelectionMeasures {
        parts,
        own_size_mm,
        volume_mm3,
        ..
    } = app.selection_measures().unwrap();
    assert_eq!((parts, own_size_mm), (2, None));
    assert!((volume_mm3.unwrap() - 124_000.0).abs() < 1.0e-3);
}

/// Review 2026-11-04 P3 GUI: the selection card wrote its numbers with a
/// point next to a value preview with the user's comma, and a position just
/// below zero as `-0`.
#[test]
fn the_selection_card_writes_numbers_with_the_users_decimal_separator_and_no_minus_zero() {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "comma.star".into(),
            source: "box(\"a\", (12.5, 30, 40.3), at = (-0.04, 0, 5))\n".into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    app.value_box.decimal = crate::length_input::DecimalSeparator::Comma;
    let a = occurrence(&app, "a");
    app.selection.select_occurrence(a, false);
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let measures = app.selection_measures().unwrap();
    let mut lines = app.selection_measure_lines(&measures);
    lines.extend(
        app.selection_mass_lines(&super::selection_mass_ui::SelectionMass::Ready(
            serde_json::json!({
                "status": "complete",
                "mass_kg": 0.1234,
                "parts_counted": 1,
                "centroid_mm": [6.25, 15.0, 25.125],
                "principal_kg_mm2": [1234.5, 2345.6, 3456.7],
                "missing": [],
            }),
        )),
    );
    let decimal_point = |line: &str| {
        line.chars().collect::<Vec<_>>().windows(3).any(|window| {
            window[0].is_ascii_digit() && window[1] == '.' && window[2].is_ascii_digit()
        })
    };
    for line in &lines {
        assert!(!decimal_point(line), "a decimal point in {line:?}");
        assert!(
            !line.contains("-0 ") && !line.contains("-0,"),
            "minus zero in {line:?}"
        );
    }
    assert!(
        lines.iter().any(|line| line.contains("12,5 × 30 × 40,3")),
        "{lines:?}"
    );

    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(2);
    assert!(harness.query_by_label("12,5").is_some());
    assert!(harness.query_by_label("12.5").is_none());
}

#[test]
fn a_height_that_rounds_to_zero_has_no_sign() {
    assert_eq!(format_height(-0.004), "0");
    assert_eq!(format_height(-0.0), "0");
    assert_eq!(format_height(-0.005_1), "-0.01");
    assert_eq!(format_height(-12.5), "-12.5");
    assert_eq!(format_height(100.0), "100");
}

#[test]
fn the_dock_shows_measures_only_while_something_is_selected() {
    let app = app();
    let a = occurrence(&app, "a");
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(2);
    let title = harness
        .state()
        .catalog
        .text("selection-measures-title")
        .to_uppercase();
    assert!(harness.query_by_label(&title).is_none());
    harness.state_mut().selection.select_occurrence(a, false);
    harness.run_steps(2);
    assert!(harness.query_by_label(&title).is_some());
}
