use super::selection_mass_ui::SelectionMass;
use super::*;
use egui_kittest::kittest::Queryable;

const PARTS: &str = "material_weight(\"C24\", kg_m3 = 420, source = \"EN 338: C24\")\n\
    box(\"oceľ\", (100, 50, 20), material = \"steel\")\n\
    box(\"trám\", (100, 50, 20), at = (200, 0, 0), material = \"C24\")\n\
    box(\"bez\", (10, 10, 10), at = (400, 0, 0))\n";

fn program() -> ketchup_model::document::RuleProgramSource {
    ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "mass.star".into(),
        source: PARTS.into(),
        overrides: BTreeMap::new(),
    }
}

fn occurrence(app: &KetchupApp, name: &str) -> OccurrenceId {
    app.document
        .current()
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap()
        .id()
}

fn mass_lines(app: &KetchupApp) -> Vec<String> {
    let measures = app.selection_measures().unwrap();
    app.selection_mass_lines(&app.selection_mass(&measures))
}

/// #2476: the dock weighs the selection with the program's densities. While
/// the program is being evaluated it says the mass is being computed (the
/// evaluation is held: a frame that waited for it would hang this test), then
/// shows the mass once the evaluation arrives, never a zero in between.
#[test]
fn the_dock_shows_the_mass_once_the_program_and_solids_are_ready() {
    let mut app = KetchupApp::new();
    app.apply_program_source(program(), true).unwrap();
    let steel = occurrence(&app, "oceľ");
    let beam = occurrence(&app, "trám");
    let bare = occurrence(&app, "bez");
    let catalog = app.catalog.clone();
    let pending = catalog.text("selection-mass-pending");

    // Without exact solids the mass is being computed, like the volume.
    app.selection.select_occurrence(steel, false);
    let measures = app.selection_measures().unwrap();
    assert_eq!(app.selection_mass(&measures), SelectionMass::Pending);
    assert_eq!(mass_lines(&app), vec![pending.clone()]);

    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let held = app.program_evaluations.hold(&program());
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(2);
    assert!(harness.query_by_label(&pending).is_some());
    let weighed = catalog.format(
        "selection-mass",
        &BTreeMap::from([("kg", "0.7850".to_owned())]),
    );
    assert!(harness.query_by_label(&weighed).is_none());
    assert_eq!(harness.state().program_evaluations.on_ui_thread(), 0);

    held.send(crate::program_evaluation::ProgramEvaluations::evaluate_for_test(&program()))
        .unwrap();
    harness.run_steps(2);
    assert!(harness.query_by_label(&pending).is_none());
    assert!(harness.query_by_label(&weighed).is_some());
    let centre = catalog.format(
        "selection-mass-centre",
        &BTreeMap::from([
            ("x", "50.0".to_owned()),
            ("y", "25.0".to_owned()),
            ("z", "10.0".to_owned()),
        ]),
    );
    assert!(harness.query_by_label(&centre).is_some());
    assert_eq!(harness.state().program_evaluations.on_ui_thread(), 0);

    // The beam's C24 density comes from the program; the part without a
    // material leaves the sum incomplete and is named, without a centre.
    let app = harness.state_mut();
    app.selection.select_occurrence(beam, true);
    app.selection.select_occurrence(bare, true);
    let lines = mass_lines(app);
    assert_eq!(
        lines,
        vec![
            catalog.format(
                "selection-mass-partial",
                &BTreeMap::from([
                    ("kg", "0.8270".to_owned()),
                    ("counted", "2".to_owned()),
                    ("total", "3".to_owned()),
                ])
            ),
            catalog.format(
                "selection-mass-missing",
                &BTreeMap::from([
                    ("name", "bez".to_owned()),
                    ("reason", "the part has no material".to_owned()),
                ])
            ),
        ]
    );
}
