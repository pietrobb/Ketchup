use super::program_source_ui::program_source_line_label;
use super::tests::through_cut_document;
use super::*;
use egui_kittest::kittest::Queryable;

const TABLE: &str = include_str!("../../../examples/programs/table.star");

fn open_table(directory: &std::path::Path) -> KetchupApp {
    let path = directory.join("table.ketchup");
    let mut session =
        ketchup_application::DocumentSession::new(ketchup_application::SessionSettings::default());
    session
        .apply_rule_program(
            ketchup_model::document::RuleProgramSource {
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
    harness.run_steps(3);
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
    // Groups, components and layers are shown; manual CAD panels are not.
    for key in ["dock-outliner", "dock-tags"] {
        assert!(
            harness.query_by_label(&catalog.text(key)).is_some(),
            "{key}"
        );
    }
    for key in ["feature-history-title", "body-title", "assembly-title"] {
        assert!(
            harness.query_by_label(&catalog.text(key)).is_none(),
            "{key}"
        );
    }

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

#[test]
fn nested_component_selection_highlights_shared_and_instance_lines() {
    let source = "a=box(\"a\", (20,30,40))\ng=group(\"inner\",[a])\nc=component(\"assembly\",[g])\ninstance(\"second\",c,at=(200,0,0))";
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
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
