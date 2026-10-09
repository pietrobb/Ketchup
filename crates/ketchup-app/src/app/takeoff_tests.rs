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

/// Review 2026-10-09 UI-2: hiding a part by hand used to detach the program,
/// so the program panel and the takeoff vanished. Hiding is a view change: the
/// program keeps the document and the takeoff just stops counting the part.
#[test]
fn hiding_a_program_part_keeps_the_program_and_drops_the_part_from_the_takeoff() {
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        file_name: "house.star".into(),
        source: SOURCE.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    let wall = app
        .document
        .current()
        .scene_query()
        .into_iter()
        .find(|part| {
            ketchup_application::rule_program_part_name(
                &app.document.current(),
                &part.instance_path,
            )
            .as_deref()
                == Some("obal/stena")
        })
        .unwrap()
        .instance_path;
    app.select_from_outliner(wall.clone(), false);
    app.dispatch_command(AppCommand::Hide);

    assert!(!snapshot_visible(&app, &wall));
    assert_eq!(app.document.current_rule_program(), Some(&program));
    app.program_evaluations.get_blocking(&program).unwrap();
    let takeoff = app.material_takeoff().unwrap();
    assert_eq!((takeoff.counted_parts, takeoff.excluded_parts), (5, 1));

    assert!(app.undo());
    assert!(snapshot_visible(&app, &wall));
    assert_eq!(app.document.current_rule_program(), Some(&program));
}

/// Review 2026-10-09 §5.1 (2): an edit that detaches the program says so once,
/// with a one-click Undo, and the status bar shows whether a program drives
/// the model or was detached.
#[test]
fn the_first_detaching_edit_says_so_and_undo_keeps_the_program() {
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        file_name: "house.star".into(),
        source: SOURCE.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    let owned = app.catalog.format(
        "status-program-owned",
        &BTreeMap::from([("file", "house.star".to_owned())]),
    );
    let detached = app.catalog.format(
        "status-program-detached",
        &BTreeMap::from([("file", "house.star".to_owned())]),
    );
    let undo = app.catalog.text("program-detached-undo");
    let keep = app.catalog.text("program-detached-keep");
    let rename = |app: &mut KetchupApp| {
        let id = app.document.current().occurrences().next().unwrap().id();
        app.apply_batch_with_work_recovery(&CommandBatch::new(vec![
            CanonicalCommand::RenameEntity {
                id,
                name: "by hand".into(),
            },
        ]))
        .unwrap();
    };
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    assert!(harness.query_by_label(&owned).is_some(), "{owned:?}");
    assert!(harness.query_by_label(&undo).is_none());

    rename(harness.state_mut());
    harness.run_steps(3);
    assert!(harness.state().document.current_rule_program().is_none());
    assert!(harness.query_by_label(&detached).is_some(), "{detached:?}");
    harness.get_by_role_and_label(Role::Button, &undo).click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().document.current_rule_program(),
        Some(&program)
    );
    assert!(harness.query_by_label(&owned).is_some());
    assert!(harness.query_by_label(&undo).is_none());

    // Once per session: the next detaching edit only shows the status chip.
    rename(harness.state_mut());
    harness.run_steps(3);
    assert!(harness.query_by_label(&detached).is_some());
    assert!(harness.query_by_label(&undo).is_none());
    assert!(harness.query_by_label(&keep).is_none());
}

fn snapshot_visible(app: &KetchupApp, path: &InstancePath) -> bool {
    app.document
        .current()
        .occurrence(path.root_occurrence())
        .unwrap()
        .visible()
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
    std::fs::write(&path, "an older takeoff").unwrap();
    assert!(app.export_material_takeoff_to(&path));
    let csv = std::fs::read_to_string(&path).unwrap();
    assert!(
        csv.contains("stena;drevo;140;60;5;12,5;1,75;0,105;blank\n"),
        "{csv}"
    );
    assert!(csv.contains("obal;drevostavba;"), "{csv}");
    assert!(app.digest.contains("vykaz.csv"));
    // Written through a temporary file that replaced the old export.
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

/// Review 2026-10-09 §5.1 (8) and CSV-1: the cut list is a real table, one row
/// per group of identical visible parts, with a header; CSV follows the
/// interface language's decimal separator and XLSX is a workbook.
#[test]
fn cut_list_export_is_a_table_of_the_visible_parts_in_csv_or_xlsx() {
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
    let concept = app
        .document
        .current()
        .tags()
        .find(|tag| tag.name() == "koncept")
        .unwrap()
        .id();
    assert!(app.set_tag_visibility(concept, false));
    let directory = tempfile::tempdir().unwrap();

    app.set_ui_language(crate::language::UiLanguage::Slovak);
    let path = directory.path().join("kusovnik.csv");
    assert!(app.export_cut_list_to(&path), "{}", app.digest);
    let csv = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        csv,
        "\u{feff}position;parts;material;length_mm;width_mm;thickness_mm;count\n\
         1;stena/stĺpik 0, stena/stĺpik 1, stena/stĺpik 2, stena/stĺpik 3, stena/stĺpik 4;drevo;2500;140;60;5\n"
    );
    assert!(app.digest.contains("kusovnik.csv"), "{}", app.digest);

    app.set_ui_language(crate::language::UiLanguage::English);
    assert!(app.export_cut_list_to(&path));
    let csv = std::fs::read_to_string(&path).unwrap();
    assert!(csv.starts_with("\u{feff}position,parts,material,"), "{csv}");
    assert!(csv.contains(",drevo,2500,140,60,5\n"), "{csv}");

    let workbook = directory.path().join("kusovnik.xlsx");
    assert!(app.export_cut_list_to(&workbook));
    let bytes = std::fs::read(&workbook).unwrap();
    assert!(bytes.starts_with(b"PK\x03\x04"), "a ZIP package");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("<c r=\"D2\"><v>2500</v></c>"), "{text}");
    assert!(
        !text.contains("obal"),
        "the hidden concept is not cut: {text}"
    );
}
