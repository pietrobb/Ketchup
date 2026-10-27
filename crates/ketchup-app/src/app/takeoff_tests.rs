use super::takeoff::PartList;
use crate::*;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

const SOURCE: &str = "for i in range(5):\n    box(\"stena/stĺpik %d\" % i, (60, 140, 2500), at = (i * 600, 0, 0), material = \"drevo\", tags = [\"konštrukcia\"])\nbox(\"obal/stena\", (3000, 160, 2500), at = (0, 400, 0), material = \"drevostavba\", tags = [\"koncept\"])\n";

fn summary(app: &KetchupApp, counted: usize, excluded: usize, exact: usize) -> String {
    app.catalog.format(
        "takeoff-summary",
        &BTreeMap::from([
            ("counted", counted.to_string()),
            ("excluded", excluded.to_string()),
            ("exact", exact.to_string()),
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
            variants: Default::default(),
            file_name: "house.star".into(),
            source: SOURCE.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let window = app.catalog.text("window-material-takeoff");
    let menu = app.catalog.text("menu-window");
    let all = summary(&app, 6, 0, 6);
    let without_concept = summary(&app, 5, 1, 5);
    let concept_layer = app.catalog.format(
        "tags-row",
        &BTreeMap::from([("name", "koncept".to_owned()), ("count", "1".to_owned())]),
    );
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    // The window counts the exact solid volumes once the parts are evaluated,
    // which a warm exact worker of an earlier request may finish within these
    // frames and a cold one may not: wait for it, as a user sees the window.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while harness.state().exact.task.is_some() {
        assert!(
            std::time::Instant::now() < deadline,
            "the parts were never evaluated"
        );
        harness.step();
        std::thread::yield_now();
    }
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
        variants: Default::default(),
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
        variants: Default::default(),
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

    let before_edit = harness.state().canonical_digest();
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
    assert_eq!(harness.state().canonical_digest(), before_edit);
    assert!(harness.query_by_label(&owned).is_some());
    assert!(harness.query_by_label(&undo).is_none());

    // Undo gave the program back, so the next detaching edit warns again
    // (review 2026-10-19 WB-7). Keeping the change answers it for the session.
    rename(harness.state_mut());
    harness.run_steps(3);
    assert!(harness.query_by_label(&detached).is_some());
    harness.get_by_role_and_label(Role::Button, &keep).click();
    harness.run_steps(3);
    assert!(harness.query_by_label(&undo).is_none());
    assert!(harness.state().document.current_rule_program().is_none());
}

/// #2562: the detach notice says which part the edit changed and why the
/// program cannot write that change.
#[test]
fn a_detaching_edit_names_the_part_and_why() {
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "house.star".into(),
        source: SOURCE.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    app.program_evaluations.get_blocking(&program).unwrap();
    let part = app.document.current().occurrences().next().unwrap().clone();
    let twice = Transform::from_matrix([
        2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    app.apply_batch_with_work_recovery(&CommandBatch::new(vec![
        CanonicalCommand::SetOccurrenceTransform {
            id: part.id(),
            transform: part.transform().compose(twice),
        },
    ]))
    .unwrap();
    assert!(app.document.current_rule_program().is_none());
    let scaled = app.catalog.format(
        "program-detached-reason-scaled",
        &BTreeMap::from([("part", part.name().to_owned())]),
    );
    assert_eq!(app.program_detach_notice.as_ref().unwrap().reason, scaled);
}

/// Review 2026-10-09 UNDO-1: "Undo the edit" used to call Undo blindly, so an
/// edit made after the detaching one (by hand or by an MCP client) was undone
/// instead and the program stayed detached. A later edit closes the notice and
/// the document keeps both edits.
#[test]
fn a_later_edit_closes_the_detach_notice_and_both_edits_stay() {
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "house.star".into(),
        source: SOURCE.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program, true).unwrap();
    let undo = app.catalog.text("program-detached-undo");
    let rename = |app: &mut KetchupApp, name: &str| {
        let id = app.document.current().occurrences().next().unwrap().id();
        app.apply_batch_with_work_recovery(&CommandBatch::new(vec![
            CanonicalCommand::RenameEntity {
                id,
                name: name.into(),
            },
        ]))
        .unwrap();
        id
    };
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);

    rename(harness.state_mut(), "detaching");
    harness.run_steps(3);
    assert!(harness.query_by_label(&undo).is_some(), "{undo:?}");

    let id = rename(harness.state_mut(), "later");
    let after_later = harness.state().canonical_digest();
    harness.run_steps(3);
    assert!(harness.query_by_label(&undo).is_none());
    assert!(harness.state().program_detach_notice.is_none());
    assert!(harness.state().document.current_rule_program().is_none());
    assert_eq!(harness.state().canonical_digest(), after_later);
    let snapshot = harness.state().document.current();
    let renamed = snapshot
        .occurrences()
        .find(|occurrence| occurrence.id() == id);
    assert_eq!(renamed.unwrap().name(), "later");
}

/// Review 2026-10-09 BOM-1 (§5.2 step 10): with `production(tags=…)` the
/// takeoff and the cut list count the production parts whether their layer is
/// shown or hidden, and never the concept body, even when it is shown.
#[test]
fn a_production_scope_counts_hidden_production_parts_and_never_the_concept() {
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "house.star".into(),
        source: format!("{SOURCE}production(tags = [\"konštrukcia\"])\n"),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    let construction = app
        .document
        .current()
        .tags()
        .find(|tag| tag.name() == "konštrukcia")
        .unwrap()
        .id();
    assert!(app.set_tag_visibility(construction, false));
    assert_eq!(app.document.current_rule_program(), Some(&program));
    app.program_evaluations.get_blocking(&program).unwrap();

    let takeoff = app.material_takeoff().unwrap();
    assert_eq!((takeoff.counted_parts, takeoff.excluded_parts), (5, 1));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("kusovnik.csv");
    assert!(
        app.export_list_to(PartList::CutList, &path),
        "{}",
        app.digest
    );
    let csv = std::fs::read_to_string(&path).unwrap();
    assert!(csv.contains(",drevo,2500,140,60,5\n"), "{csv}");
    assert!(!csv.contains("obal"), "{csv}");
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
            variants: Default::default(),
            file_name: "house.star".into(),
            source: SOURCE.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vykaz.csv");
    app.value_box.decimal = crate::length_input::DecimalSeparator::Comma;
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

/// UI-9 (review 2026-10-17): CSV numbers follow the system's decimal
/// separator, the one the value fields read, not the interface language: an
/// English window on a Slovak system writes what Excel there splits.
#[test]
fn a_csv_export_follows_the_systems_decimal_separator_not_the_language() {
    use crate::language::UiLanguage::{English, Slovak};
    use crate::length_input::DecimalSeparator::{Comma, Point};
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "house.star".into(),
            source: SOURCE.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vykaz.csv");
    for (language, decimal, row) in [
        (
            English,
            Comma,
            "stena;drevo;140;60;5;12,5;1,75;0,105;blank\n",
        ),
        (
            Slovak,
            Point,
            "stena,drevo,140,60,5,12.5,1.75,0.105,blank\n",
        ),
    ] {
        app.set_ui_language(language);
        app.value_box.decimal = decimal;
        assert!(app.export_material_takeoff_to(&path), "{}", app.digest);
        let csv = std::fs::read_to_string(&path).unwrap();
        assert!(csv.contains(row), "{language:?} {decimal:?}\n{csv}");
    }
}

/// The window exports the hardware list and the bar schedule the program
/// report gives (MCP report sections `hardware` and `bar_schedule`).
#[test]
fn hardware_and_bar_schedule_export_the_report_rows() {
    let mut app = KetchupApp::new();
    let source = "board(\"plate\", (300, 200, 18))\n\
                  bolt(\"b1\", \"M8\", 40, at = (50, 50, 18))\n\
                  bolt(\"b2\", \"M8\", 40, at = (150, 50, 18))\n\
                  rebar(\"r1\", 12, 1000, at = (0, 300, 0), axis = (1, 0, 0))\n\
                  rebar(\"r2\", 12, 1000, at = (0, 350, 0), axis = (1, 0, 0))\n";
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "bars.star".into(),
            source: source.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    app.value_box.decimal = crate::length_input::DecimalSeparator::Comma;
    let directory = tempfile::tempdir().unwrap();
    let (_, report) = ketchup_program::run("bars.star", source, &BTreeMap::new()).unwrap();

    let path = directory.path().join("kovanie.csv");
    assert!(
        app.export_list_to(PartList::Hardware, &path),
        "{}",
        app.digest
    );
    let csv = std::fs::read_to_string(&path).unwrap();
    let mut lines = csv.lines();
    assert_eq!(
        lines.next(),
        Some(
            "\u{feff}item;count;parts;mass_basis;source;standard;thread;thread_from_mm;thread_to_mm"
        )
    );
    let row = lines.next().unwrap();
    assert!(row.starts_with("ISO 4017 M8x40;2;b1, b2;"), "{csv}");
    assert!(row.ends_with(";ISO 4017;M8;'-40.0;0.0"), "{csv}");
    assert_eq!(lines.next(), None, "the bars are not hardware: {csv}");
    assert_eq!(
        csv,
        ketchup_program::bom::hardware_table(&report.bom.hardware)
            .csv(ketchup_program::table::DecimalSeparator::Comma)
    );
    assert!(app.digest.contains("kovanie.csv"), "{}", app.digest);

    let path = directory.path().join("vystuz.csv");
    assert!(app.export_list_to(PartList::BarSchedule, &path));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "\u{feff}position;diameter_mm;grade;shape;cut_length_mm;count;total_length_m;mass_kg;parts\n\
         1;12;B500B;straight;1000;2;2;1,776;r1, r2\n"
    );
    let workbook = directory.path().join("vystuz.xlsx");
    assert!(app.export_list_to(PartList::BarSchedule, &workbook));
    let text = String::from_utf8_lossy(&std::fs::read(&workbook).unwrap()).into_owned();
    assert!(text.contains("<c r=\"E2\"><v>1000</v></c>"), "{text}");
}

/// R9-C step 17: the window exports the cutting plan (MCP report section
/// `cutting`) as the saw's table: the plate cut from a 1250 x 1250 sheet.
#[test]
fn the_cutting_plan_exports_every_piece_on_its_sheet() {
    let mut app = KetchupApp::new();
    let source = "sheet_stock(\"OSB\", [(2500, 1250), (1250, 1250)], kerf = 4, trim = 10)\n\
                  board(\"plate\", (600, 400, 18), material = \"OSB\")\n";
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "sheet.star".into(),
            source: source.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    app.value_box.decimal = crate::length_input::DecimalSeparator::Comma;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rozkroj.csv");
    assert!(
        app.export_list_to(PartList::CuttingPlan, &path),
        "{}",
        app.digest
    );
    // 0.24 of 1.5625 m2; without a grain the plate may lie turned, as
    // best fits the large sheet it was first placed on.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "\u{feff}material;kind;section_mm;stock_no;stock_mm;part;at_mm;size_mm;turned;used_percent;offcut_mm;note\n\
         OSB;sheets;18;1;1250 x 1250;plate;10 x 10;400 x 600;yes;15,4;;\n"
    );
    assert!(app.digest.contains("rozkroj.csv"), "{}", app.digest);
}

/// The stored entries of a ZIP archive the export wrote, by name.
fn zip_entries(archive: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let field = |at: usize, width: usize| {
        archive[at..at + width]
            .iter()
            .rev()
            .fold(0_usize, |value, byte| value << 8 | usize::from(*byte))
    };
    let mut entries = BTreeMap::new();
    let mut at = 0;
    while field(at, 4) == 0x0403_4b50 {
        let (size, name, extra) = (field(at + 18, 4), field(at + 26, 2), field(at + 28, 2));
        let start = at + 30 + name + extra;
        let name = String::from_utf8(archive[at + 30..at + 30 + name].to_vec()).unwrap();
        entries.insert(name, archive[start..start + size].to_vec());
        at = start + size;
    }
    entries
}

/// R9-C step 19: the window exports one CNC DXF per made flat part and a
/// table of what it drew in one ZIP; each drawing reads back with its
/// outline and its hole, and a bar is no CNC part.
#[test]
fn the_cnc_export_writes_a_dxf_per_part_that_reads_back() {
    let mut app = KetchupApp::new();
    let source = "side = board(\"Bočnica\", (18, 400, 720), material = \"DTD\")\n\
                  hole(side, \"x+\", world = (18, 37, 32), diameter = 5, depth = 13)\n\
                  board(\"top\", (600, 400, 18), at = (0, 0, 720), material = \"DTD\")\n\
                  member(\"rail\", (0, 1000, 0), (2000, 1000, 0), (60, 120))\n";
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "cnc.star".into(),
            source: source.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cnc.zip");
    assert!(app.export_cnc_drawings_to(&path), "{}", app.digest);
    assert!(app.digest.contains("cnc.zip"), "{}", app.digest);
    let entries = zip_entries(&std::fs::read(&path).unwrap());
    assert_eq!(
        entries.keys().map(String::as_str).collect::<Vec<_>>(),
        ["Bocnica.dxf", "parts.csv", "top.dxf"]
    );
    let read = |name: &str| {
        ketchup_model::import::inspect_dxf(
            &entries[name],
            ketchup_model::import::DxfImportOptions::new(None),
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"))
    };
    let side = read("Bocnica.dxf");
    let layers = side
        .profiles()
        .iter()
        .map(|profile| profile.layer())
        .collect::<Vec<_>>();
    assert_eq!(layers, ["CONTOUR", "DRILL_D5_Z13"]);
    let top = read("top.dxf");
    assert_eq!(top.profiles().len(), 1, "an outline only");
    let csv = String::from_utf8(entries["parts.csv"].clone()).unwrap();
    assert!(csv.contains("Bočnica,Bocnica.dxf,18,400,720,"), "{csv}");
}

/// R9-C step 18: the window exports the quote (MCP report section `quote`):
/// 2 x 0.096 m2 of board at 25.50 a m2 is 4.90; a program without prices
/// says so instead of writing an empty quote.
#[test]
fn the_quote_exports_priced_lines_and_a_program_without_prices_is_refused() {
    let mut app = KetchupApp::new();
    let program = |source: &str| ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "quote.star".into(),
        source: source.into(),
        overrides: BTreeMap::new(),
    };
    let boards = "board(\"a\", (400, 240, 18), material = \"Board\")\n\
                  board(\"b\", (400, 240, 18), at = (0, 500, 0), material = \"Board\")\n";
    app.apply_program_source(program(boards), true).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ponuka.csv");
    assert!(!app.export_list_to(PartList::Quote, &path));
    assert!(app.digest.contains("price_list()"), "{}", app.digest);
    assert!(!path.exists());
    let priced = format!("{boards}price(\"Board\", 25.5, \"m2\", currency = \"EUR\")\n");
    app.apply_program_source(program(&priced), true).unwrap();
    app.value_box.decimal = crate::length_input::DecimalSeparator::Comma;
    assert!(app.export_list_to(PartList::Quote, &path), "{}", app.digest);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "\u{feff}kind;item;quantity;unit;unit_price;amount;currency;basis;source;note\n\
         material;Board;0,192;m2;25,5;4,9;EUR;blanks;;\n\
         ;total;;;;4,9;EUR;;;\n"
    );
    assert!(app.digest.contains("ponuka.csv"), "{}", app.digest);
}

/// Review 2026-10-09 §5.1 (8) and CSV-1: the cut list is a real table, one row
/// per group of identical visible parts, with a header; CSV follows the
/// system's decimal separator (UI-9) and XLSX is a workbook.
#[test]
fn cut_list_export_is_a_table_of_the_visible_parts_in_csv_or_xlsx() {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
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

    app.value_box.decimal = crate::length_input::DecimalSeparator::Comma;
    let path = directory.path().join("kusovnik.csv");
    assert!(
        app.export_list_to(PartList::CutList, &path),
        "{}",
        app.digest
    );
    let csv = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        csv,
        "\u{feff}position;parts;material;length_mm;width_mm;thickness_mm;count\n\
         1;stena/stĺpik 0, stena/stĺpik 1, stena/stĺpik 2, stena/stĺpik 3, stena/stĺpik 4;drevo;2500;140;60;5\n"
    );
    assert!(app.digest.contains("kusovnik.csv"), "{}", app.digest);

    app.value_box.decimal = crate::length_input::DecimalSeparator::Point;
    assert!(app.export_list_to(PartList::CutList, &path));
    let csv = std::fs::read_to_string(&path).unwrap();
    assert!(csv.starts_with("\u{feff}position,parts,material,"), "{csv}");
    assert!(csv.contains(",drevo,2500,140,60,5\n"), "{csv}");

    let workbook = directory.path().join("kusovnik.xlsx");
    assert!(app.export_list_to(PartList::CutList, &workbook));
    let bytes = std::fs::read(&workbook).unwrap();
    assert!(bytes.starts_with(b"PK\x03\x04"), "a ZIP package");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("<c r=\"D2\"><v>2500</v></c>"), "{text}");
    assert!(
        !text.contains("obal"),
        "the hidden concept is not cut: {text}"
    );
}

/// Review 2026-10-19 WB-7: the detach warning was shown once per window, so
/// after Undo gave the program back, or in the next document, an edit that
/// detached it again did so without a word.
#[test]
fn the_detach_warning_comes_again_after_undo_or_in_another_document() {
    let mut app = KetchupApp::new();
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "house.star".into(),
        source: SOURCE.into(),
        overrides: BTreeMap::new(),
    };
    let detach = |app: &mut KetchupApp, name: &str| {
        app.program_detach_notice = None;
        let id = app.document.current().occurrences().next().unwrap().id();
        app.apply_batch_with_work_recovery(&CommandBatch::new(vec![
            CanonicalCommand::RenameEntity {
                id,
                name: name.into(),
            },
        ]))
        .unwrap();
        assert!(app.document.current_rule_program().is_none());
        app.program_detach_notice.is_some()
    };
    app.apply_program_source(program.clone(), true).unwrap();
    assert!(detach(&mut app, "first"));
    assert!(app.undo());
    assert!(app.document.current_rule_program().is_some());
    assert!(detach(&mut app, "second"), "after Undo");

    // Open puts the read document in place and resets what the old one showed.
    let mut other = KetchupApp::new();
    other.apply_program_source(program, true).unwrap();
    app.document = other.document;
    app.reset_document_presentation();
    assert!(detach(&mut app, "third"), "in another document");
}
