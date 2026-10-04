use crate::*;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

const SOURCE: &str = "for i in range(5):\n    box(\"stena/stĺpik %d\" % i, (60, 140, 2500), at = (i * 600, 0, 0), material = \"drevo\", tags = [\"konštrukcia\"])\nbox(\"obal/stena\", (3000, 160, 2500), at = (0, 400, 0), material = \"drevostavba\", tags = [\"koncept\"])\n";

fn summary(app: &KetchupApp, counted: usize, excluded: usize) -> String {
    app.catalog.format(
        "takeoff-summary",
        &BTreeMap::from([
            ("counted", counted.to_string()),
            ("excluded", excluded.to_string()),
            ("exact", "0".to_owned()),
        ]),
    )
}

/// Window > Material takeoff lists the visible parts by category, material and
/// cross-section; turning a layer off in the dock drops its parts at once.
#[test]
fn takeoff_window_follows_layer_visibility() {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "house.star".into(),
            source: SOURCE.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let window = app.catalog.text("window-material-takeoff");
    let menu = app.catalog.text("menu-window");
    let all = summary(&app, 6, 0);
    let without_concept = summary(&app, 5, 1);
    let concept_layer = app.catalog.format(
        "tags-row",
        &BTreeMap::from([("name", "koncept".to_owned()), ("count", "1".to_owned())]),
    );
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, &menu).click();
    harness.run_steps(2);
    harness.get_by_role_and_label(Role::Button, &window).click();
    harness.run_steps(3);
    assert!(harness.state().takeoff.open);
    assert!(
        harness.query_by_label(&all).is_some(),
        "summary {all:?} missing"
    );
    assert!(harness.query_by_label("140 × 60").is_some());
    assert!(
        harness.query_all_by_label("12.50").next().is_some(),
        "stud running length"
    );
    assert!(harness.query_all_by_label("obal").next().is_some());

    harness
        .get_by_role_and_label(Role::CheckBox, &concept_layer)
        .click();
    harness.run_steps(3);
    assert!(
        harness.query_by_label(&without_concept).is_some(),
        "summary {without_concept:?} missing"
    );
    assert!(harness.query_all_by_label("obal").next().is_none());
    assert!(harness.query_all_by_label("drevostavba").next().is_none());
    assert!(harness.query_by_label("140 × 60").is_some());
}

#[test]
fn takeoff_csv_export_writes_the_visible_parts() {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "house.star".into(),
            source: SOURCE.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vykaz.csv");
    assert!(app.export_material_takeoff_to(&path));
    let csv = std::fs::read_to_string(&path).unwrap();
    assert!(
        csv.contains("stena;drevo;140;60;5;12.5;1.75;0.105;blank\n"),
        "{csv}"
    );
    assert!(csv.contains("obal;drevostavba;"), "{csv}");
    assert!(app.digest.contains("vykaz.csv"));
}
