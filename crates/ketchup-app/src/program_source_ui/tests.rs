use super::program_source_line_label;
use super::*;
use crate::tests::through_cut_document;
use egui_kittest::kittest::Queryable;

const TABLE: &str = include_str!("../../../../examples/programs/table.star");

fn open_table(directory: &std::path::Path) -> KetchupApp {
    let path = directory.join("table.ketchup");
    let mut session =
        ketchup_application::DocumentSession::new(ketchup_application::SessionSettings::default());
    session
        .apply_rule_program(
            ketchup_model::document::RuleProgramSource {
                variants: Default::default(),
                file_name: "table.star".to_owned(),
                source: TABLE.to_owned(),
                overrides: BTreeMap::new(),
            },
            false,
        )
        .unwrap();
    session
        .save(&path, ketchup_application::SaveOptions { overwrite: false })
        .unwrap();
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        crate::dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));
    assert!(app.open_document_path(&path));
    app
}

fn occurrence_named(app: &KetchupApp, name: &str) -> OccurrenceId {
    app.document
        .current()
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap_or_else(|| panic!("table has no part {name:?}"))
        .id()
}

fn line_label(number: usize, defining: bool) -> String {
    program_source_line_label(number, TABLE.lines().nth(number - 1).unwrap(), defining)
}

fn harness(app: KetchupApp) -> egui_kittest::Harness<'static, KetchupApp> {
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    // The opened program is evaluated in the background; until then the
    // panel says it is planning.
    harness.run_steps(1);
    crate::tests::shell::step_until(
        &mut harness,
        std::time::Duration::from_secs(60),
        "the opened program was never evaluated",
        |app| !app.program_is_planning(),
    );
    harness.run_steps(2);
    harness
}

#[test]
fn program_is_always_shown_and_selection_highlights_its_lines() {
    let directory = tempfile::tempdir().unwrap();
    let app = open_table(directory.path());
    let leg = occurrence_named(&app, "table/leg-back-left");
    let top = occurrence_named(&app, "table/top");
    let mut harness = harness(app);
    let catalog = harness.state().catalog.clone();
    let title = catalog.text("program-source-title");
    // The whole program is visible before anything is selected.
    assert!(harness.query_by_label(&title).is_some());
    assert!(
        harness
            .query_by_label(&catalog.text("program-source-select-hint"))
            .is_some()
    );
    assert!(harness.query_by_label(&line_label(13, false)).is_some());
    // The table's functions are offered as chips; manual CAD panels are not shown.
    for key in ["feature-history-title", "body-title", "assembly-title"] {
        assert!(
            harness.query_by_label(&catalog.text(key)).is_none(),
            "{key}"
        );
    }
    // Groups, components and layers are one tab away.
    harness
        .get_by_label(&catalog.text("dock-tab-layers"))
        .click();
    harness.run_steps(2);
    for key in ["dock-outliner", "dock-tags"] {
        assert!(
            harness
                .query_by_label(&catalog.text(key).to_uppercase())
                .is_some(),
            "{key}"
        );
    }
    assert!(harness.query_by_label(&title).is_none());
    harness
        .get_by_label(&catalog.text("dock-tab-program"))
        .click();
    harness.run_steps(2);

    harness.state_mut().selection.select_occurrence(leg, false);
    harness.run_steps(3);
    assert!(
        harness
            .query_by_label(&catalog.format(
                "program-source-lines",
                &BTreeMap::from([
                    ("part", "table/leg-back-left".to_owned()),
                    ("lines", "13, 17".to_owned()),
                ]),
            ))
            .is_some()
    );
    // The leg's own board() call and the dowel loop that drills it.
    for line in [13, 17] {
        assert!(harness.query_by_label(&line_label(line, true)).is_some());
    }
    assert!(harness.query_by_label(&line_label(9, false)).is_some());

    harness.state_mut().selection.select_occurrence(top, false);
    harness.run_steps(3);
    assert!(
        harness
            .query_by_label(&catalog.format(
                "program-source-lines",
                &BTreeMap::from([
                    ("part", "table/top".to_owned()),
                    ("lines", "9, 17".to_owned()),
                ]),
            ))
            .is_some()
    );
    assert!(harness.query_by_label(&line_label(9, true)).is_some());
    assert!(harness.query_by_label(&line_label(13, false)).is_some());
}

/// The dock at its narrowest, as if dragged there.
fn narrowest_dock(harness: &egui_kittest::Harness<'static, KetchupApp>) {
    harness.ctx.data_mut(|data| {
        data.insert_persisted(
            egui::Id::new("right-dock"),
            egui::containers::panel::PanelState {
                rect: Rect::from_min_size(
                    egui::pos2(1600.0 - 380.0, 0.0),
                    Vec2::new(380.0, 1000.0),
                ),
            },
        );
    });
}

#[test]
fn long_program_lines_wrap_inside_the_narrowest_dock() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = open_table(directory.path());
    app.set_ui_language(crate::language::UiLanguage::Slovak);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    narrowest_dock(&harness);
    harness.run_steps(1);
    crate::tests::shell::step_until(
        &mut harness,
        std::time::Duration::from_secs(60),
        "the opened program was never evaluated",
        |app| !app.program_is_planning(),
    );
    harness.run_steps(2);
    let short = harness.get_by_label(&line_label(10, false)).rect();
    let long = harness.get_by_label(&line_label(14, false)).rect();
    assert!(short.left() >= 1600.0 - 380.0, "{short:?}");
    // The whole line stays inside the window, continued below itself.
    assert!(long.right() <= 1600.0, "{long:?}");
    assert!(
        long.height() >= 2.0 * short.height(),
        "{long:?} vs {short:?}"
    );
    for number in 1..=TABLE.lines().count() {
        let line = harness.get_by_label(&line_label(number, false)).rect();
        assert!(line.right() <= 1600.0, "line {number}: {line:?}");
    }
}

#[test]
fn outliner_reports_real_size_of_program_parts() {
    let directory = tempfile::tempdir().unwrap();
    let app = open_table(directory.path());
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(1);
    crate::tests::shell::step_until(
        &mut harness,
        std::time::Duration::from_secs(60),
        "the opened program was never evaluated",
        |app| !app.program_is_planning(),
    );
    let top = harness
        .state()
        .outliner_query()
        .into_iter()
        .find(|definition| definition.name == "table/top")
        .unwrap();
    assert_eq!(top.specification, "1200 × 700 × 25");
}

#[test]
fn nested_component_selection_highlights_shared_and_instance_lines() {
    let source = "a=box(\"a\", (20,30,40))\ng=group(\"inner\",[a])\nc=component(\"assembly\",[g])\ninstance(\"second\",c,at=(200,0,0))";
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "components.star".into(),
            source: source.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let snapshot = app.document.current();
    let root = snapshot
        .occurrences()
        .find(|item| item.name() == "second")
        .unwrap();
    let path = snapshot
        .scene_query()
        .into_iter()
        .find(|part| part.occurrence_id == root.id() && part.occurrence_name == "a")
        .unwrap()
        .instance_path;
    let mut harness = harness(app);
    assert!(
        harness
            .state_mut()
            .enter_occurrence_context(InstancePath::root(root.id()))
    );
    harness
        .state_mut()
        .select_from_outliner(path.clone(), false);
    assert_eq!(
        harness.state().selected_instance_paths(),
        BTreeSet::from([path])
    );
    harness.run_steps(3);
    let catalog = harness.state().catalog.clone();
    assert!(
        harness
            .query_by_label(&catalog.format(
                "program-source-lines",
                &BTreeMap::from([
                    ("part", "second/a".to_owned()),
                    ("lines", "1, 4".to_owned())
                ])
            ))
            .is_some()
    );
    for line in [1, 4] {
        let label = program_source_line_label(line, source.lines().nth(line - 1).unwrap(), true);
        assert!(harness.query_by_label(&label).is_some(), "{label}");
    }
}
#[test]
fn a_long_program_lays_out_only_the_lines_in_view_and_jumps_to_any() {
    let mut source = String::new();
    for number in 1..=3000 {
        source.push_str(&format!("# note {number}\n"));
    }
    source.push_str("def far():\n    return \"far\"\n");
    source.push_str("a=box(\"a\", (20,30,40))\ng=group(\"inner\",[a])\nc=component(\"assembly\",[g])\ninstance(\"second\",c,at=(200,0,0))");
    let label = |number: usize, defining: bool| {
        program_source_line_label(number, source.lines().nth(number - 1).unwrap(), defining)
    };
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "long.star".into(),
            source: source.clone(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let mut harness = harness(app);
    assert!(harness.query_by_label(&label(1, false)).is_some());
    // A frame reads only what the dock shows, not three thousand lines.
    let shown = (1..=source.lines().count())
        .filter(|number| harness.query_by_label(&label(*number, false)).is_some())
        .count();
    assert!(shown < 200, "{shown} lines laid out");
    assert!(harness.query_by_label(&label(3001, false)).is_none());

    harness
        .get_by(|node| node.label().is_some_and(|text| text.starts_with("far()")))
        .click();
    harness.run_steps(3);
    let far = harness.get_by_label(&label(3001, false)).rect();
    assert!(harness.query_by_label(&label(1, false)).is_none());
    assert!(far.top() < 1000.0, "{far:?}");
}

#[test]
fn manual_model_does_not_grow_the_dock_with_a_starlark_section() {
    let mut app = KetchupApp::new();
    app.document = through_cut_document();
    app.reset_document_presentation();
    let occurrence = app.document.current().occurrences().next().unwrap().id();
    let mut harness = harness(app);
    harness
        .state_mut()
        .selection
        .select_occurrence(occurrence, false);
    harness.run_steps(3);
    let title = harness.state().catalog.text("program-source-title");
    assert!(harness.query_by_label(&title).is_none());
}
