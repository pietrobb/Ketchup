use ketchup_program::model::{Face, ProgramModel};
use ketchup_program::{Severity, run, validate};
use std::collections::BTreeMap;

const CABINET: &str = include_str!("../../../examples/programs/cabinet.star");

fn eval(source: &str) -> ProgramModel {
    run("test.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .0
        .model
}

fn kinds(source: &str) -> Vec<&'static str> {
    validate(&eval(source))
        .into_iter()
        .map(|issue| issue.kind)
        .collect()
}

fn world_holes(model: &ProgramModel, part: &str, prefix: &str) -> Vec<[i64; 3]> {
    let part = model.part(part).unwrap();
    let mut points: Vec<[i64; 3]> = part
        .holes
        .iter()
        .filter(|hole| hole.id.starts_with(prefix))
        .map(|hole| {
            let world = part.to_world(hole.face.local_point(part.size_mm, hole.u_mm, hole.v_mm));
            #[allow(clippy::cast_possible_truncation)]
            world.map(|value| (value * 1000.0).round() as i64)
        })
        .collect();
    points.sort_unstable();
    points
}

#[test]
fn cabinet_evaluates_without_issues_and_lists_parts_and_hardware() {
    let (_, report) = run("cabinet.star", CABINET, &BTreeMap::new()).unwrap();
    assert!(report.ok, "{:#?}", report.issues);
    assert_eq!(report.warnings, 0, "{:#?}", report.issues);
    assert_eq!(report.bom.total_parts, 7);
    assert_eq!(report.bom.hardware.len(), 1);
    assert_eq!(report.bom.hardware[0].item, "dowel 8x30");
    assert_eq!(report.bom.hardware[0].count, 8);
    let sides = report
        .bom
        .cut_list
        .iter()
        .find(|row| row.dimensions_mm == [720.0, 350.0, 18.0])
        .unwrap();
    assert_eq!(sides.count, 2);
}

#[test]
fn a_wider_cabinet_recomputes_every_dependent_part_and_hole() {
    let overrides = BTreeMap::from([("width".to_owned(), 700.0)]);
    let (evaluated, report) = run("cabinet.star", CABINET, &overrides).unwrap();
    assert!(report.ok, "{:#?}", report.issues);
    let model = evaluated.model;
    assert_eq!(
        model.part("carcass/bottom").unwrap().size_mm,
        [664.0, 350.0, 18.0]
    );
    assert_eq!(
        model.part("carcass/right").unwrap().at_mm,
        [682.0, 0.0, 0.0]
    );
    assert_eq!(model.part("shelves/1").unwrap().size_mm[0], 662.0);
    // The right side's dowel holes moved with it and still meet the bottom's.
    assert_eq!(
        world_holes(&model, "carcass/right", "dowel:carcass/bottom"),
        world_holes(&model, "carcass/bottom", "dowel:carcass/right"),
    );
    assert_eq!(
        world_holes(&model, "carcass/right", "dowel:carcass/bottom")[0][0],
        682_000
    );
}

#[test]
fn changing_the_dowel_margin_moves_the_holes_in_both_boards() {
    let moved = CABINET.replace(
        "dowels(panel, side, dowel = \"8x30\", margin = 50)",
        "dowels(panel, side, dowel = \"8x30\", margin = 80)",
    );
    assert_ne!(moved, CABINET);
    let before = eval(CABINET);
    let after = eval(&moved);
    let left_after = world_holes(&after, "carcass/left", "dowel:carcass/bottom");
    assert_eq!(
        left_after,
        world_holes(&after, "carcass/bottom", "dowel:carcass/left")
    );
    assert_ne!(
        left_after,
        world_holes(&before, "carcass/left", "dowel:carcass/bottom")
    );
    let ys: Vec<i64> = left_after.iter().map(|point| point[1]).collect();
    assert_eq!(ys, vec![80_000, 270_000]);
}

#[test]
fn dowel_holes_enter_the_shared_face_of_each_board() {
    let model = eval(CABINET);
    let side = model.part("carcass/left").unwrap();
    let bottom = model.part("carcass/bottom").unwrap();
    let side_hole = side
        .holes
        .iter()
        .find(|hole| hole.id.starts_with("dowel:"))
        .unwrap();
    let bottom_hole = bottom
        .holes
        .iter()
        .find(|hole| hole.id.starts_with("dowel:carcass/left"))
        .unwrap();
    assert_eq!(side_hole.face, Face::XMax);
    assert_eq!(bottom_hole.face, Face::XMin);
    assert!((side_hole.depth_mm - 16.0).abs() < 1e-9);
    assert!((side_hole.diameter_mm - 8.0).abs() < 1e-9);
}

#[test]
fn overlapping_parts_are_reported_with_both_names_and_the_region() {
    let issues = validate(&eval(
        "a = box(\"a\", (100, 100, 18))\nb = box(\"b\", (100, 100, 18), at = (50, 0, 0))\n",
    ));
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, "collision");
    assert_eq!(issues[0].severity, Severity::Error);
    assert_eq!(issues[0].parts, vec!["a".to_owned(), "b".to_owned()]);
    assert_eq!(
        issues[0].where_mm,
        Some(([50.0, 0.0, 0.0], [100.0, 100.0, 18.0]))
    );
}

#[test]
fn a_part_seated_in_a_groove_is_not_a_collision() {
    let source = "\
side = board(\"side\", (18, 300, 600))
groove(side, \"x+\", along = \"z\", width = 4, depth = 8, offset = 280)
back = board(\"back\", (4, 4, 600), at = (10, 280, 0))
";
    assert_eq!(kinds(source), Vec::<&str>::new());
    let unseated = source.replace("offset = 280", "offset = 200");
    assert_eq!(kinds(&unseated), vec!["collision"]);
}

#[test]
fn a_joint_between_parts_that_do_not_touch_is_an_error() {
    let source = "\
a = box(\"a\", (100, 100, 18))
b = box(\"b\", (100, 100, 18), at = (0, 0, 30))
joint(a, b, kind = \"glue\")
";
    assert!(kinds(source).contains(&"joint_without_contact"));
    let pins = source.replace("kind = \"glue\"", "kind = \"pins\", max_gap = 12");
    assert!(!kinds(&pins).contains(&"joint_without_contact"));
}

#[test]
fn holes_that_break_through_or_leave_the_face_are_errors() {
    let source = "\
p = box(\"p\", (200, 100, 18))
hole(p, \"z+\", at = (100, 50), diameter = 8, depth = 20)
hole(p, \"z+\", at = (2, 50), diameter = 8, depth = 10)
";
    let issues = kinds(source);
    assert!(issues.contains(&"hole_breaks_through"));
    assert!(issues.contains(&"hole_outside_face"));
}

#[test]
fn parts_that_touch_nothing_supported_are_warnings() {
    let issues = validate(&eval(
        "box(\"floor\", (100, 100, 18))\nbox(\"floating\", (100, 100, 18), at = (0, 0, 500))\n",
    ));
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, "floating_part");
    assert_eq!(issues[0].severity, Severity::Warning);
}

#[test]
fn params_take_overrides_report_unknown_ones_and_check_ranges() {
    let source = "n = param(\"count\", 2, min = 1, max = 4)\nfor i in range(n):\n    box(\"p%d\" % i, (10, 10, 10), at = (20 * i, 0, 0))\n";
    let overrides = BTreeMap::from([("count".to_owned(), 5.0), ("typo".to_owned(), 1.0)]);
    let (evaluated, report) = run("t.star", source, &overrides).unwrap();
    assert_eq!(evaluated.model.parts.len(), 5);
    assert_eq!(report.unused_overrides, vec!["typo".to_owned()]);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.kind == "param_out_of_range")
    );
}

#[test]
fn errors_name_the_line_and_the_cause() {
    let error = run(
        "broken.star",
        "a = box(\"a\", (1, 2, 3))\nb = box(\"a\", (1, 2, 3))\n",
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert_eq!(error.code, "evaluation_error");
    assert!(error.message.contains("broken.star:2"), "{}", error.message);
    assert!(
        error.message.contains("already exists"),
        "{}",
        error.message
    );

    let error = run("syntax.star", "box(\"a\", (1, 2, 3)\n", &BTreeMap::new()).unwrap_err();
    assert_eq!(error.code, "syntax_error");

    let error = run(
        "dowel.star",
        "a = box(\"a\", (10, 10, 10))\nb = box(\"b\", (10, 10, 10), at = (50, 0, 0))\ndowels(a, b)\n",
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(error.message.contains("do not touch"), "{}", error.message);
}

#[test]
fn divide_splits_a_span_into_rounded_fields_that_add_up() {
    let source = "fields = divide(4930, 12)\nprint(fields)\nprint(sum(fields))\n";
    let (evaluated, _) = run("divide.star", source, &BTreeMap::new()).unwrap();
    assert_eq!(evaluated.log[1], "4930");
}

#[test]
fn panel_operations_carry_holes_and_pockets_in_panel_coordinates() {
    let model = eval(CABINET);
    let operations = ketchup_program::cad::panel_operations(&model);
    assert_eq!(operations.len(), 7);
    let ketchup_core::assistant_sidecar::AssistantCadEditOperation::CreatePanel {
        name,
        holes,
        pockets,
        translation_mm,
        ..
    } = &operations[1]
    else {
        panic!("expected create_panel");
    };
    assert_eq!(name, "carcass/right");
    assert_eq!(*translation_mm, [582.0, 0.0, 0.0]);
    assert_eq!(pockets.len(), 1);
    assert_eq!(pockets[0].inward_unit_local, [1.0, 0.0, 0.0]);
    assert_eq!(pockets[0].min_local_mm[0], 0.0);
    assert!(holes.iter().all(|hole| hole.entry_local_mm[0] == 0.0));
}
