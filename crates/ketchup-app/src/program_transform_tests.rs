use super::*;
use ketchup_model::document::{FeatureParameterTarget, RuleProgramSource};

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

fn occurrence(app: &KetchupApp, name: &str) -> Option<OccurrenceId> {
    app.document
        .current()
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .map(|occurrence| occurrence.id())
}

#[test]
fn deleting_a_program_part_removes_its_line_as_one_undo_step() {
    let mut app = app();
    let steps = app.undo_step_count();
    select(&mut app, "a");
    assert!(app.delete_selected());
    assert_eq!(
        source(&app),
        PARTS.replace("box(\"a\", (20, 30, 40), at = (100, 0, 5))\n", "")
    );
    assert_eq!(occurrence(&app, "a"), None);
    select(&mut app, "b");
    assert_close(placed(&app).0, [0.0, 300.0, 0.0]);
    assert_eq!(app.undo_step_count(), steps + 1);
    assert!(app.undo());
    assert_eq!(source(&app), PARTS);
    assert!(occurrence(&app, "a").is_some());
}

#[test]
fn deleting_a_part_of_a_loop_is_refused_with_the_line_and_changes_nothing() {
    let mut app = KetchupApp::new();
    let looped = "for i in range(2):\n    box(\"s%d\" % i, (10, 10, 10), at = (0, 0, 20 * i))\n";
    app.apply_program_source(
        RuleProgramSource {
            variants: Default::default(),
            file_name: "loop.star".into(),
            source: looped.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let revision = app.document_revision();
    select(&mut app, "s1");
    assert!(!app.delete_selected());
    assert!(app.digest.contains("loop on line 1"), "{}", app.digest);
    assert_eq!(source(&app), looped);
    assert_eq!(app.document_revision(), revision);
}

#[test]
fn a_colour_is_written_into_the_creating_call_and_survives_the_next_apply() {
    let mut app = app();
    let steps = app.undo_step_count();
    select(&mut app, "b");
    assert!(app.set_selected_occurrence_color(Some([200, 10, 0])));
    let coloured = PARTS.replace("at = (0, x, 0))", "at = (0, x, 0), color=(200, 10, 0))");
    assert_eq!(source(&app), coloured);
    assert_eq!(app.undo_step_count(), steps + 1);
    let b = occurrence(&app, "b").unwrap();
    // Applying the program again keeps the colour: the program sets it now.
    let program = app.document.current_rule_program().unwrap().clone();
    app.apply_program_source(program, false).unwrap();
    let snapshot = app.document.current();
    assert_eq!(snapshot.occurrence(b).unwrap().color(), Some([200, 10, 0]));
}

/// Review 2026-10-19 WB-3: the parameter and the material that read like the
/// part's name keep it.
#[test]
fn renaming_a_program_part_keeps_its_id() {
    let named = "w = param(\"a\", 20)\n\
                 box(\"a\", (w, 30, 40), at = (100, 0, 5))\n\
                 box(\"b\", (200, 50, 10), material = \"a\")\n";
    let mut app = KetchupApp::new();
    app.apply_program_source(
        RuleProgramSource {
            variants: Default::default(),
            file_name: "named.star".into(),
            source: named.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let a = occurrence(&app, "a").unwrap();
    let b = occurrence(&app, "b").unwrap();
    select(&mut app, "a");
    let source_plan = app.occurrence_rename_source_plan().unwrap();
    let plan = app
        .occurrence_rename_plan(&PendingOccurrenceRename {
            source: source_plan,
            name: "side".into(),
        })
        .unwrap();
    assert!(app.apply_occurrence_rename_plan(plan));
    assert_eq!(
        source(&app),
        format!(
            "{}continue_part(\"side\", was=\"a\")\n",
            named.replace("box(\"a\"", "box(\"side\"")
        )
    );
    assert_eq!(occurrence(&app, "side"), Some(a));
    assert_eq!(occurrence(&app, "b"), Some(b));
    assert_eq!(occurrence(&app, "a"), None);
    assert_eq!(app.document.current().occurrences().count(), 2);
}

/// Review 2026-10-19 P3: renaming a part twice in one batch detached the
/// program, because the second rename named the part by its document name.
#[test]
fn a_part_renamed_twice_in_one_batch_stays_the_programs() {
    let mut app = app();
    let a = occurrence(&app, "a").unwrap();
    let rename = |name: &str| CanonicalCommand::RenameEntity {
        id: a,
        name: name.to_owned(),
    };
    assert_eq!(
        app.commit_program_part_batch(&[rename("p"), rename("q")]),
        Some(true),
        "{}",
        app.digest
    );
    assert_eq!(
        source(&app),
        format!(
            "{}continue_part(\"q\", was=\"a\")\n",
            PARTS.replace("box(\"a\"", "box(\"q\"")
        )
    );
    assert_eq!(occurrence(&app, "q"), Some(a));
}

#[test]
fn mirroring_a_program_part_adds_a_mirrored_line() {
    let mut app = app();
    let a = occurrence(&app, "a").unwrap();
    app.queue_window_edit(
        AssistantCadEditOperation::Mirror {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: vec![a.0],
            },
            plane_origin_mm: [0.0, 0.0, 0.0],
            plane_normal: [1.0, 0.0, 0.0],
        },
        "digest-mirror-created",
        None,
    );
    app.finish_window_edit();
    assert_eq!(
        source(&app),
        format!("{PARTS}mirrored(\"a\", \"a mirrored\", point=(0, 0, 0), normal=(1, 0, 0))\n")
    );
    // x 100..120 across x = 0 is x -120..-100; y 0..30 and z 5..45 stay.
    select(&mut app, "a mirrored");
    let (min, size) = placed(&app);
    assert_close(min, [-120.0, 0.0, 5.0]);
    assert_close(size, [20.0, 30.0, 40.0]);
}

/// Review 2026-10-21 P3: two mirrors of one part in one MCP program get two
/// names, not the same name twice.
#[test]
fn two_mirrors_of_one_part_in_one_program_get_two_names() {
    let app = app();
    let a = occurrence(&app, "a").unwrap();
    let mirror = |normal| AssistantCadEditOperation::Mirror {
        selector: AssistantCadEntitySelector::Occurrences {
            occurrence_ids: vec![a.0],
        },
        plane_origin_mm: [0.0, 0.0, 0.0],
        plane_normal: normal,
    };
    let program = AssistantCadEditProgram {
        operations: vec![mirror([1.0, 0.0, 0.0]), mirror([0.0, 1.0, 0.0])],
    };
    let source = app.program_edit_source(&program).unwrap().unwrap().source;
    assert_eq!(
        source,
        format!(
            "{PARTS}mirrored(\"a\", \"a mirrored\", point=(0, 0, 0), normal=(1, 0, 0))\n\
             mirrored(\"a\", \"a mirrored 2\", point=(0, 0, 0), normal=(0, 1, 0))\n"
        )
    );
}

/// Sets the value at `path` of part `name`'s feature, as the parameter panel does.
fn set_value(app: &mut KetchupApp, name: &str, path: &str, value: f64) -> bool {
    let snapshot = app.document.current();
    let id = occurrence(app, name).unwrap();
    let definition = snapshot.definition(snapshot.occurrence(id).unwrap().definition_id());
    let feature_id = *definition
        .unwrap()
        .feature_ids()
        .iter()
        .find(|id| {
            let kind = snapshot.feature(**id).unwrap().kind();
            kind.parameter_descriptors()
                .iter()
                .any(|made| made.path().as_str() == path)
        })
        .unwrap();
    let target = FeatureParameterTarget::new(feature_id, path, ParameterValueType::Length).unwrap();
    let dimension = Dimension::new(value.to_string(), value).unwrap();
    let command = CanonicalCommand::SetFeatureParameter { target, dimension };
    app.commit_program_part_batch(&[command])
        .expect("a program part")
}

#[test]
fn a_panel_value_of_a_program_part_is_written_into_the_program() {
    let mut app = app();
    let steps = app.undo_step_count();
    assert!(
        set_value(&mut app, "a", "extent.distance", 50.0),
        "{}",
        app.digest
    );
    // 40 high becomes 50: the top goes up by 10, from z 45 to z 55.
    assert_eq!(
        source(&app),
        format!("{PARTS}push_pull(\"a\", face=\"z+\", distance=10, name=\"Length\")\n")
    );
    // Measured in the program: without exact shapes the window's bounds grow
    // every side of a pushed face.
    let model = ketchup_program::evaluate("parts.star", &source(&app), &BTreeMap::new())
        .unwrap()
        .model;
    let (min, max) = model.part("a").unwrap().world_bounds();
    assert_close(min, [100.0, 0.0, 5.0]);
    assert_close(max, [120.0, 30.0, 55.0]);
    assert_eq!(app.undo_step_count(), steps + 1);

    // Typed again, 30 replaces 50 instead of adding to it, and 0 is refused.
    assert!(set_value(&mut app, "a", "extent.distance", 30.0));
    assert_eq!(
        source(&app),
        format!("{PARTS}push_pull(\"a\", face=\"z+\", distance=-10, name=\"Length\")\n")
    );
    let revision = app.document_revision();
    assert!(!set_value(&mut app, "a", "extent.distance", 0.0));
    assert!(app.digest.contains("above 0"), "{}", app.digest);
    assert_eq!(app.document_revision(), revision);

    // A width written in the source is refused with its line; nothing changes.
    let revision = app.document_revision();
    assert!(!set_value(&mut app, "b", "bounds.width", 250.0));
    assert!(app.digest.contains("line 3"), "{}", app.digest);
    assert_eq!(app.document_revision(), revision);
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
