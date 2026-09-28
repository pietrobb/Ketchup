use ketchup_program::model::{Face, ProgramModel, ProgramPartBody};
use ketchup_program::{COLLISION_UNVERIFIED, Severity, run, validate};
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
    // 8x30 into the face of an 18 mm side: 12 mm leaves a third; the bottom's
    // edge takes the rest of the dowel, both with 1.5 mm clearance.
    assert!((side_hole.depth_mm - 12.0).abs() < 1e-9);
    assert!((bottom_hole.depth_mm - 21.0).abs() < 1e-9);
    assert!((side_hole.diameter_mm - 8.0).abs() < 1e-9);
}

fn side_and_shelf(thickness: f64, dowel: &str) -> String {
    format!(
        "side = board(\"side\", ({thickness}, 450, 600))\n\
         shelf = board(\"shelf\", (564, 450, {thickness}))\n\
         on(shelf, side, face = \"x+\", align = [\"y-\"])\n\
         move(shelf, by = (0, 0, 200))\n\
         dowels(side, shelf, dowel = \"{dowel}\")\n"
    )
}

fn dowel_depths(model: &ProgramModel) -> (f64, f64) {
    let depth = |part: &str| {
        let part = model.part(part).unwrap();
        let depths: Vec<f64> = part.holes.iter().map(|hole| hole.depth_mm).collect();
        assert!(
            depths.windows(2).all(|pair| pair[0] == pair[1]),
            "{depths:?}"
        );
        depths[0]
    };
    (depth("side"), depth("shelf"))
}

#[test]
fn a_dowel_goes_shallow_into_a_board_face_and_deep_into_the_other_edge() {
    for thickness in [18.0, 19.0] {
        let model = eval(&side_and_shelf(thickness, "8x35"));
        let (face, edge) = dowel_depths(&model);
        assert!(face <= 13.0, "{thickness} mm face hole {face}");
        assert!(thickness - face >= thickness / 3.0 - 1e-9, "{face}");
        assert!(edge > face, "{face} {edge}");
        // Room for the whole dowel plus clearance at both ends.
        assert!(face + edge >= 35.0 + 2.0 * 1.5 - 1e-9, "{face} + {edge}");
        let issues = validate(&model);
        assert!(issues.is_empty(), "{issues:#?}");
    }
    let (face, edge) = dowel_depths(&eval(&side_and_shelf(18.0, "8x35")));
    assert_eq!((face, edge), (12.0, 26.0));
}

#[test]
fn a_dowel_too_long_for_two_board_faces_fails_with_the_numbers() {
    let source = "\
a = board(\"a\", (400, 300, 18))
b = board(\"b\", (400, 300, 18))
on(b, a)
dowels(a, b, dowel = \"8x35\")
";
    let Err(error) = run("test.star", source, &BTreeMap::new()) else {
        panic!("two 18 mm faces cannot hold a 35 mm dowel");
    };
    let error = error.to_string();
    for needed in [
        "8x35",
        "35 mm",
        "18 mm thick",
        "at most 12 mm",
        "shorter dowel",
    ] {
        assert!(error.contains(needed), "{needed:?} missing in {error}");
    }
}

#[test]
fn a_blind_hole_leaving_too_little_material_is_a_warning() {
    let source = "\
p = box(\"p\", (200, 100, 18))
hole(p, \"z+\", at = (100, 50), diameter = 8, depth = 16)
hole(p, \"z+\", at = (150, 50), diameter = 8, depth = 12)
";
    let issues = validate(&eval(source));
    assert_eq!(issues.len(), 1, "{issues:#?}");
    assert_eq!(issues[0].kind, "hole_wall_too_thin");
    assert!(
        issues[0].message.contains("leaving 2 mm"),
        "{}",
        issues[0].message
    );
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
fn named_profile_segments_are_preserved_and_must_be_unique() {
    let model = eval(
        "extrude(\"angle\", profile=[[\"bottom\", [0, 0], [40, 0]], [\"outer\", [40, 0], [40, 10]], [\"top\", [40, 10], [0, 10]], [\"back\", [0, 10], [0, 0]]], distance=18)",
    );
    let ProgramPartBody::Extrusion { segments, .. } = &model.part("angle").unwrap().body else {
        panic!("expected extrusion");
    };
    assert_eq!(
        segments
            .iter()
            .map(|segment| segment.name.as_str())
            .collect::<Vec<_>>(),
        ["bottom", "outer", "top", "back"]
    );

    let error = run(
        "duplicate.star",
        "extrude(\"angle\", profile=[[\"side\", [0, 0], [40, 0]], [\"side\", [40, 0], [40, 10]], [\"top\", [40, 10], [0, 0]]], distance=18)",
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(
        error.message.contains("unique printable names"),
        "{}",
        error.message
    );
}

#[test]
fn panel_operations_carry_holes_and_pockets_in_panel_coordinates() {
    let model = eval(CABINET);
    let operations = ketchup_program::cad::part_operations(&model);
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

#[test]
fn an_unclosed_bracket_at_the_end_names_its_line() {
    for source in ["a = 1\nb = (1, 2\n", "a = 1\nb = [1,\n\n# end\n"] {
        let error = run("t.star", source, &BTreeMap::new()).unwrap_err();
        assert_eq!(error.code, "syntax_error");
        assert!(!error.message.contains("t.star:1:1"), "{}", error.message);
        assert!(error.message.contains("t.star:2:") || error.message.contains("t.star:4:"));
    }
}

#[test]
fn the_interpreter_leaves_workspace_json_parsing_intact() {
    // serde_json's `arbitrary_precision` is unified into every crate of a
    // build and makes tagged enums reject plain numbers.
    #[derive(serde::Deserialize)]
    #[serde(tag = "kind")]
    enum Tagged {
        Length { mm: f64 },
    }
    let Tagged::Length { mm } = serde_json::from_str(r#"{"kind":"Length","mm":1.5}"#).unwrap();
    assert_eq!(mm, 1.5);
}

#[test]
fn every_part_knows_the_program_lines_that_define_it() {
    let source = r#"top = board("top", (600, 400, 18), at = (0, 0, 382))
def leg(name, x):
    return board(name, (50, 50, 382), at = (x, 0, 0))
left = leg("left leg", 0)
right = leg("right leg", 550)
dowels(
    left,
    top,
    count = 2,
    margin = 10,
    spacing = 25,
)
"#;
    let (evaluated, _) = run("table.star", source, &BTreeMap::new()).unwrap();
    let lines = |part: &str| -> Vec<(usize, usize)> {
        evaluated.part_sources[part]
            .iter()
            .map(|lines| (lines.first, lines.last))
            .collect()
    };
    // Prelude frames are not program lines; the call inside the user's helper
    // and the call of that helper both are.
    assert_eq!(lines("top"), [(1, 1), (6, 12)]);
    assert_eq!(lines("left leg"), [(3, 3), (4, 4), (6, 12)]);
    assert_eq!(lines("right leg"), [(3, 3), (5, 5)]);
    assert_eq!(evaluated.part_sources.len(), 3);
}

const SPLAYED_STOOL: &str = r#"
H = 450
seat = extrude("seat", profile = [(-200, -200), (200, -200), (200, 200), (-200, 200)], distance = 30, at = (0, 0, H))
for i, angle in enumerate([90, 210, 330]):
    a = math.radians(angle)
    foot = (230 * math.cos(a), 230 * math.sin(a), 0)
    top = (120 * math.cos(a), 120 * math.sin(a), H)
    member("leg%d" % (i + 1), foot, top, (40, 40))
"#;

fn assert_close(actual: [f64; 3], expected: [f64; 3]) {
    assert!(
        (0..3).all(|axis| (actual[axis] - expected[axis]).abs() < 1.0e-9),
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn members_run_from_point_to_point_and_rotated_bounds_follow_them() {
    let model = eval(SPLAYED_STOOL);
    let leg = model.part("leg1").unwrap();
    let length = 110.0_f64.hypot(450.0);
    assert!((leg.size_mm[2] - length).abs() < 1.0e-9);
    assert!(leg.is_rotated());
    // The centre line ends exactly at the requested points.
    assert_close(leg.to_world([20.0, 20.0, 0.0]), [0.0, 230.0, 0.0]);
    assert_close(leg.to_world([20.0, 20.0, length]), [0.0, 120.0, 450.0]);
    let (min, max) = leg.world_bounds();
    assert!(
        min[2] < 0.0 && max[2] > 450.0,
        "square ends of a tilted bar stick out"
    );
    assert!((min[0] + 20.0).abs() < 1.0e-9 && (max[0] - 20.0).abs() < 1.0e-9);
}

#[test]
fn tilted_parts_collide_exactly_not_by_their_world_bounds() {
    // The legs reach into the seat; nothing else overlaps.
    let issues = validate(&eval(SPLAYED_STOOL));
    let collisions: Vec<_> = issues
        .iter()
        .filter(|issue| issue.kind == "collision")
        .collect();
    assert_eq!(collisions.len(), 3, "{issues:#?}");
    assert!(
        collisions
            .iter()
            .all(|issue| issue.parts.contains(&"seat".to_owned()))
    );
    // Differently tilted bars whose world bounds overlap in both cases.
    let bars = |end: &str| {
        format!(
            "member(\"a\", (0, 0, 0), (500, 500, 0), (20, 20))\n\
             member(\"b\", (300, 0, 0), {end}, (20, 20))\n"
        )
    };
    assert!(!kinds(&bars("(500, 100, 0)")).contains(&"collision"));
    assert!(kinds(&bars("(100, 400, 0)")).contains(&"collision"));
}

#[test]
fn rotate_turns_a_part_about_a_world_axis_and_pivot() {
    let model = eval(
        "p = box(\"p\", (100, 10, 10), at = (50, 0, 0))\n\
         rotate(p, axis = (0, 0, 1), angle = 90, pivot = (0, 0, 0))\n\
         rotate(p, axis = (0, 0, 1), angle = 90)\n",
    );
    let part = model.part("p").unwrap();
    assert_close(part.at_mm, [0.0, 50.0, 0.0]);
    assert_close(part.to_world([100.0, 0.0, 0.0]), [-100.0, 50.0, 0.0]);
}

const ROUND_STOOL: &str = include_str!("../../../examples/programs/round_stool.star");

#[test]
fn booleans_cut_parts_with_tools_that_are_never_built_or_listed() {
    let (evaluated, report) = run("stool.star", ROUND_STOOL, &BTreeMap::new()).unwrap();
    let model = evaluated.model;
    assert_eq!(model.parts.len(), 7);
    assert_eq!(model.tools.len(), 6, "two trim tools per leg");
    assert_eq!(report.bom.total_parts, 7);
    assert!(report.ok, "{:#?}", report.issues);
    assert_eq!(report.warnings, 0, "{:#?}", report.issues);
    let seat = model.part("seat").unwrap();
    assert_eq!(seat.booleans.len(), 3);
    // The seat subtracts each leg as it was then: trimmed twice and notched by two rails.
    assert_eq!(seat.booleans[0].tool.booleans.len(), 4);
    let (min, max) = seat.world_bounds();
    assert!((min[2] - 420.0).abs() < 1.0e-9 && (max[2] - 450.0).abs() < 1.0e-9);
    assert!((max[0] - 180.0).abs() < 1.0e-9);
}

#[test]
fn a_part_still_collides_with_what_it_did_not_subtract() {
    // The round seat fills only part of its box, so boxes cannot decide these
    // overlaps; they wait for the exact solids instead of being called collisions.
    let without_seat_holes = ROUND_STOOL.replace("    subtract(seat, leg)\n", "    pass\n");
    let issues = validate(&eval(&without_seat_holes));
    let unverified: Vec<_> = issues
        .iter()
        .filter(|issue| issue.kind == COLLISION_UNVERIFIED)
        .collect();
    assert_eq!(unverified.len(), 3, "{issues:#?}");
    assert!(unverified.iter().all(
        |issue| issue.severity == Severity::Warning && issue.parts.contains(&"seat".to_owned())
    ));
    assert!(!issues.iter().any(|issue| issue.kind == "collision"));
    // Intersecting with a tool keeps only what b shares with it; a collision
    // remains only where that kept piece still reaches into a.
    let kept = "a = box(\"a\", (100, 100, 100))\nb = box(\"b\", (100, 100, 100), at = (50, 0, 0))\n\
                keep = box(\"keep\", (40, 100, 100), at = (40, 0, 0), tool = True)\nintersect(b, keep)\n";
    assert!(
        kinds(kept).contains(&"collision"),
        "the kept piece reaches into a"
    );
    let missed = kept.replace("at = (40, 0, 0), tool", "at = (110, 0, 0), tool");
    assert!(!kinds(&missed).contains(&"collision"));
}

#[test]
fn boxes_decide_only_what_boxes_can_decide() {
    let post = "post = box(\"post\", (40, 40, 400))\n";
    // A rectangle extruded is exactly its box: an overlap is a collision.
    let square = format!(
        "{post}s = extrude(\"s\", profile = [(0, 0), (100, 0), (100, 100), (0, 100)], distance = 20, at = (20, 20, 100))\n"
    );
    assert!(kinds(&square).contains(&"collision"));
    // A triangle fills half its box: only the exact solids can tell.
    let triangle = format!(
        "{post}t = extrude(\"t\", profile = [(0, 0), (100, 0), (0, 100)], distance = 20, at = (20, 20, 100))\n"
    );
    let found = kinds(&triangle);
    assert!(
        found.contains(&COLLISION_UNVERIFIED) && !found.contains(&"collision"),
        "{found:?}"
    );
    // Two notches that each miss part of the overlap may still cover it together.
    let rail = "rail = box(\"rail\", (300, 20, 30), at = (20, 10, 200))\n";
    let notches = "a = box(\"a\", (12, 20, 30), at = (20, 10, 200), tool = True)\n\
                   b = box(\"b\", (20, 20, 30), at = (28, 10, 200), tool = True)\n\
                   subtract(post, a)\nsubtract(post, b)\n";
    let found = kinds(&format!("{post}{rail}{notches}"));
    assert!(
        found.contains(&COLLISION_UNVERIFIED) && !found.contains(&"collision"),
        "{found:?}"
    );
}

/// A leg with a rail let into it: the rail reaches 20 mm into the leg.
fn notched_leg(notch: &str, turn: bool) -> String {
    let mut source = format!(
        "leg = box(\"leg\", (40, 40, 400))\n\
         rail = box(\"rail\", (300, 20, 30), at = (20, 10, 200))\n\
         notch = box(\"notch\", {notch}, at = (20, 10, 200), tool = True)\n\
         subtract(leg, notch)\n"
    );
    if turn {
        // The notch was cut before the turn; it turns with the leg.
        for part in ["leg", "rail"] {
            source.push_str(&format!(
                "rotate({part}, axis = (1, 2, 3), angle = 37, pivot = (0, 0, 0))\n"
            ));
        }
    }
    source
}

#[test]
fn a_notch_cut_by_a_tool_makes_room_for_the_part_that_sits_in_it() {
    for turn in [false, true] {
        let fits = kinds(&notched_leg("(20, 20, 30)", turn));
        assert!(!fits.contains(&"collision"), "turned {turn}: {fits:?}");
        let shallow = kinds(&notched_leg("(20, 20, 20)", turn));
        assert!(shallow.contains(&"collision"), "turned {turn}: {shallow:?}");
    }
    // Without the notch the rail collides with the leg.
    let solid = notched_leg("(20, 20, 30)", false).replace("subtract(leg, notch)\n", "");
    assert!(kinds(&solid).contains(&"collision"));
    // place() carries the cut along too: it stays put in the leg's own frame.
    let cut_in_leg = |source: &str| {
        let model = eval(source);
        let leg = model.part("leg").unwrap();
        leg.frame_of(&leg.booleans[0].tool)
    };
    let before = cut_in_leg(&notched_leg("(20, 20, 30)", false));
    let after = cut_in_leg(&format!(
        "{}place(leg, origin = (100, 50, 0), z = (1, 0, 1), x = (0, 1, 0))\n",
        notched_leg("(20, 20, 30)", false)
    ));
    assert!(
        before
            .iter()
            .zip(after)
            .all(|(a, b)| (a - b).abs() < 1.0e-9),
        "{before:?} != {after:?}"
    );
}

/// A side and a shelf doweled together, optionally turned as one unit.
fn doweled_shelf(turn: bool) -> String {
    let mut source = String::from(
        "side = box(\"side\", (18, 400, 600))\n\
         shelf = box(\"shelf\", (500, 400, 18), at = (18, 0, 300))\n",
    );
    if turn {
        source.push_str(
            "for p in (side, shelf):\n\
             \x20   rotate(p, axis = (0, 0, 1), angle = 30, pivot = (0, 0, 0))\n\
             \x20   rotate(p, axis = (1, 0, 0), angle = 10, pivot = (0, 0, 0))\n",
        );
    }
    source.push_str(
        "c = contact(side, shelf)\n\
         if (c.face_a, c.face_b) != (\"x+\", \"x-\"):\n\
         \x20   fail(\"faces %s %s\" % (c.face_a, c.face_b))\n\
         dowels(side, shelf, dowel = \"8x30\")\n",
    );
    source
}

#[test]
fn contact_and_dowels_work_on_rotated_parts_in_their_own_frames() {
    let (flat, flat_report) = run("flat.star", &doweled_shelf(false), &BTreeMap::new()).unwrap();
    let (turned, turned_report) =
        run("turned.star", &doweled_shelf(true), &BTreeMap::new()).unwrap();
    assert!(flat_report.ok, "{:#?}", flat_report.issues);
    assert!(turned_report.ok, "{:#?}", turned_report.issues);
    assert_eq!(turned_report.warnings, 0, "{:#?}", turned_report.issues);
    for name in ["side", "shelf"] {
        let flat = &flat.model.part(name).unwrap().holes;
        let turned = &turned.model.part(name).unwrap().holes;
        assert_eq!(flat.len(), 2);
        assert_eq!(flat.len(), turned.len());
        for (a, b) in flat.iter().zip(turned) {
            assert_eq!(a.face, b.face);
            assert!((a.u_mm - b.u_mm).abs() < 1.0e-6 && (a.v_mm - b.v_mm).abs() < 1.0e-6);
        }
    }
    assert_eq!(turned.model.joints.len(), 1);
}

#[test]
fn contact_between_faces_turned_in_their_plane_is_the_overlap_polygon() {
    let model = eval(
        "base = box(\"base\", (200, 200, 20))\n\
         top = box(\"top\", (100, 100, 20), at = (50, 50, 20))\n\
         rotate(top, axis = (0, 0, 1), angle = 45, pivot = (100, 100, 0))\n",
    );
    let (base, top) = (model.part("base").unwrap(), model.part("top").unwrap());
    let contact = ketchup_program::eval::contact(base, top).unwrap();
    assert_eq!((contact.face_a, contact.face_b), (Face::ZMax, Face::ZMin));
    assert_eq!(contact.points_mm.len(), 4);
    let half_diagonal = 50.0 * std::f64::consts::SQRT_2;
    assert_close(
        contact.origin_mm,
        [100.0 - half_diagonal, 100.0 - half_diagonal, 20.0],
    );
    assert!((contact.size_mm[0] - 2.0 * half_diagonal).abs() < 1.0e-9);
    // Turned further out, the top overhangs the base and the patch is clipped.
    let overhang = eval(
        "base = box(\"base\", (100, 100, 20))\n\
         top = box(\"top\", (100, 100, 20), at = (0, 0, 20))\n\
         rotate(top, axis = (0, 0, 1), angle = 45, pivot = (50, 50, 0))\n",
    );
    let clipped = ketchup_program::eval::contact(
        overhang.part("base").unwrap(),
        overhang.part("top").unwrap(),
    )
    .unwrap();
    assert_eq!(
        clipped.points_mm.len(),
        8,
        "a square cut by a turned square"
    );
    assert!(
        !kinds(
            "base = box(\"base\", (100, 100, 20))\n\
         top = box(\"top\", (100, 100, 20), at = (0, 0, 20))\n\
         rotate(top, axis = (0, 0, 1), angle = 45, pivot = (50, 50, 0))\n"
        )
        .contains(&"floating_part")
    );
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1.0e-9
}

#[test]
fn a_tilted_back_stands_on_a_turned_seat_flush_and_centred_without_coordinates() {
    let source = r#"
seat = box("seat", (400, 380, 30), at = (100, 50, 420))
rotate(seat, axis = (0, 0, 1), angle = 30)
back = box("back", (360, 20, 400))
rotate(back, axis = (1, 0, 0), angle = -12)
rotate(back, axis = (0, 0, 1), angle = 30)
on(back, seat, align = "y+", center = "x")
r = distance(back, seat)
if not r.touching or r.overlap != 0 or r.distance != 0:
    fail("back does not rest on the seat: %r" % r)
if nearest(back).name != "seat":
    fail("nearest part is not the seat")
"#;
    let model = eval(source);
    let (seat, back) = (model.part("seat").unwrap(), model.part("back").unwrap());
    let axis = |index: usize| std::array::from_fn::<f64, 3, _>(|row| seat.rotation[row][index]);
    let minus = |v: [f64; 3]| v.map(|value| -value);
    let (x, y, z) = (axis(0), axis(1), axis(2));
    // Lowest point of the back lies on the seat top ...
    assert!(close(-back.reach(minus(z)), seat.reach(z)));
    // ... its back edge is level with the seat's back edge ...
    assert!(close(back.reach(y), seat.reach(y)));
    // ... and it is centred across the seat.
    let middle = |part: &ketchup_program::model::Part, d: [f64; 3]| {
        (part.reach(d) - part.reach(minus(d))) / 2.0
    };
    assert!(close(middle(back, x), middle(seat, x)));
    // The tilt was kept: the back is still turned relative to the seat.
    assert!(!ketchup_program::frame::same_orientation(
        &back.rotation,
        &seat.rotation
    ));
    assert!(!kinds(source).contains(&"collision"));
}

#[test]
fn distance_is_exact_between_boxes_that_are_apart_diagonally_or_overlap() {
    let model = eval(
        r#"
a = box("a", (100, 100, 100))
b = box("b", (100, 100, 100), at = (110, 110, 0))
c = box("c", (100, 100, 100), at = (95, 0, 0))
d = box("d", (100, 100, 100), at = (0, 0, 300))
rotate(d, axis = (0, 0, 1), angle = 45, pivot = (50, 50, 0))
ab = distance(a, b)
if abs(ab.distance - 10 * math.sqrt(2)) > 1e-9 or ab.overlap != 0 or ab.touching:
    fail("a-b %r" % ab)
ac = distance(a, c)
if abs(ac.overlap - 5) > 1e-9 or ac.distance != 0 or ac.direction != (1.0, 0.0, 0.0):
    fail("a-c %r" % ac)
ad = distance(a, d)
if abs(ad.distance - 200) > 1e-9:
    fail("a-d %r" % ad)
n = nearest(a, among = [b, d])
if n.name != "b":
    fail("nearest %r" % n)
if abs(reach(d, (1, 0, 0)) - (50 + 50 * math.sqrt(2))) > 1e-9:
    fail("reach of a turned box %r" % reach(d, (1, 0, 0)))
"#,
    );
    assert_eq!(model.parts.len(), 4);
}

#[test]
fn reach_follows_round_and_extruded_bodies_not_their_boxes() {
    let model = eval(
        r#"
seat = revolve("seat", profile = [(0, 0), (180, 0), (180, 30), (0, 30)], axis = [(0, 0), (0, 1)], at = (0, 0, 420))
rotate(seat, axis = (1, 0, 0), angle = 90)
wedge = extrude("wedge", profile = [(0, 0), (100, 0), (0, 50)], distance = 20, at = (500, 0, 0))
leg = box("leg", (30, 30, 300))
on(leg, seat, face = "y-", center = "xz")
hanger = box("hanger", (10, 10, 10))
on(hanger, seat, face = (0, 0, -1))
"#,
    );
    let seat = model.part("seat").unwrap();
    let diagonal = [
        std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
        0.0,
    ];
    assert!(
        close(seat.reach(diagonal), 180.0),
        "not the corner of its box"
    );
    assert!(close(seat.reach([0.0, 0.0, 1.0]), 450.0));
    let wedge = model.part("wedge").unwrap();
    // The slanted side lies 100·50/√(100²+50²) from the origin corner.
    let slant = [0.5 / 1.25f64.sqrt(), 1.0 / 1.25f64.sqrt(), 0.0];
    assert!(close(
        wedge.reach(slant) - 500.0 * slant[0],
        50.0 / 1.25f64.sqrt()
    ));
    // The turned seat's bottom is its own y- face; world directions work too.
    let leg = model.part("leg").unwrap();
    assert_close(leg.at_mm, [-15.0, -15.0, 120.0]);
    assert!(close(model.part("hanger").unwrap().at_mm[2], 410.0));
}

#[test]
fn table_legs_placed_by_relations_match_the_coordinate_table() {
    const TABLE: &str = include_str!("../../../examples/programs/table.star");
    let relational = r#"
WIDTH = param("width", 1200, min = 600, max = 2400)
DEPTH = param("depth", 700, min = 400, max = 1200)
HEIGHT = param("height", 720, min = 400, max = 1100)
TOP = param("top_thickness", 25, min = 15, max = 60)
LEG = param("leg_size", 70, min = 40, max = 120)
INSET = param("leg_inset", 45, min = 20, max = 150)

top = board("table/top", (WIDTH, DEPTH, TOP), at = (0, 0, HEIGHT))
for name, faces in [("front-left", ["x-", "y-"]), ("front-right", ["x+", "y-"]),
                    ("back-left", ["x-", "y+"]), ("back-right", ["x+", "y+"])]:
    leg = board("table/leg-" + name, (LEG, LEG, HEIGHT))
    on(leg, top, face = "z-")
    for face in faces:
        flush(leg, top, face, offset = INSET)
    dowels(top, leg, dowel = "8x40", margin = 15)
"#;
    for overrides in [
        BTreeMap::new(),
        BTreeMap::from([("width".to_owned(), 1500.0), ("leg_inset".to_owned(), 80.0)]),
    ] {
        let (expected, _) = run("table.star", TABLE, &overrides).unwrap();
        let (actual, report) = run("relational.star", relational, &overrides).unwrap();
        assert!(report.ok, "{:#?}", report.issues);
        assert_eq!(actual.model.parts.len(), expected.model.parts.len());
        for part in &expected.model.parts {
            let placed = actual.model.part(&part.name).unwrap();
            assert_close(placed.at_mm, part.at_mm);
            assert_eq!(placed.holes.len(), part.holes.len(), "{}", part.name);
        }
    }
}
