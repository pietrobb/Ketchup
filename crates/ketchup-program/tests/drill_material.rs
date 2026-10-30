//! PG-4 (review 2026-10-17): how deep a hole may go is the material along its
//! line from the entry, not how far the part reaches before its cuts and
//! pockets.
use ketchup_program::model::{ProgramModel, ProgramOperation};
use ketchup_program::{run, validate};
use std::collections::BTreeMap;

fn eval(source: &str) -> ProgramModel {
    run("test.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .0
        .model
}

fn kinds(model: &ProgramModel) -> Vec<&'static str> {
    validate(model)
        .into_iter()
        .map(|issue| issue.kind)
        .collect()
}

/// A 12 mm hole from the front of an 18 mm side on the line of a 10 mm deep
/// groove in its back meets the groove after 8 mm.
#[test]
fn a_blind_hole_into_a_groove_behind_it_breaks_through() {
    let side = "\
side = board(\"side\", (18, 300, 600))
groove(side, \"x+\", along = \"z\", width = 4, depth = 10, offset = 280)
";
    let into_groove = eval(&format!(
        "{side}hole(side, \"x-\", world = (0, 282, 300), diameter = 12, depth = 12)\n"
    ));
    assert!(
        kinds(&into_groove).contains(&"hole_breaks_through"),
        "{:?}",
        kinds(&into_groove)
    );
    let beside = eval(&format!(
        "{side}hole(side, \"x-\", world = (0, 150, 300), diameter = 12, depth = 12)\n"
    ));
    assert_eq!(kinds(&beside), Vec::<&str>::new());
}

/// PG-11 (review 2026-10-18): a 12 mm hole 12 mm deep at y = 96 in an 18 mm
/// board with a 6 x 10 mm groove at y = 100..106 in its back misses the
/// groove on its axis but cuts 2 mm into it with its rim.
#[test]
fn a_blind_hole_whose_rim_reaches_a_groove_breaks_through() {
    let side = "\
side = board(\"side\", (18, 300, 600))
groove(side, \"x+\", along = \"z\", width = 6, depth = 10, offset = 100)
";
    let hole = |y: f64| {
        eval(&format!(
            "{side}hole(side, \"x-\", world = (0, {y}, 300), diameter = 12, depth = 12)\n"
        ))
    };
    let rim_in_groove = hole(96.0);
    assert!(
        kinds(&rim_in_groove).contains(&"hole_breaks_through"),
        "{:?}",
        kinds(&rim_in_groove)
    );
    // The rim 0.5 mm short of the groove leaves the board whole.
    assert_eq!(kinds(&hole(93.5)), Vec::<&str>::new());
}

/// PG-23 (review 2026-10-24): a 12 mm hole at y = 96 whose rim reaches a
/// 4 mm deep groove at y = 100..106 in the face it is drilled from goes into
/// the groove's floor there, 14 mm of material, not 0 mm.
#[test]
fn a_hole_whose_rim_reaches_a_groove_in_its_entry_face_measures_the_material_past_the_groove() {
    let side = "\
side = board(\"side\", (18, 300, 600))
groove(side, \"x-\", along = \"z\", width = 6, depth = 4, offset = 100)
";
    let hole = |depth: f64| {
        eval(&format!(
            "{side}hole(side, \"x-\", world = (0, 96, 300), diameter = 12, depth = {depth})\n"
        ))
    };
    assert_eq!(kinds(&hole(10.0)), Vec::<&str>::new());
    // 18 mm deep it leaves the board on the axis, past the groove too.
    assert!(
        kinds(&hole(18.0)).contains(&"hole_breaks_through"),
        "{:?}",
        kinds(&hole(18.0))
    );
}

/// Review 2026-10-26 PG-43 (a regression of PG-23): an 8 mm hole 10 mm deep in
/// the middle of a 200 x 200 through cut-out, or in a pocket deeper than the
/// hole, meets no material, yet the line that found none was read as the
/// material all the way, so no issue was raised.
#[test]
fn a_hole_drilled_into_an_opening_or_a_deeper_pocket_is_reported() {
    let top = "top = box(\"top\", (600, 400, 18))\n";
    let opening = eval(&format!(
        "{top}cut(top, profile = [[200, 100], [400, 100], [400, 300], [200, 300]], through = True, name = \"sink\")
hole(top, \"z+\", world = (300, 200, 18), diameter = 8, depth = 10)\n"
    ));
    assert!(
        kinds(&opening).contains(&"hole_breaks_through"),
        "{:?}",
        kinds(&opening)
    );
    let pocket = |depth: f64| {
        eval(&format!(
            "{top}pocket(top, \"z+\", rect = (250, 150, 350, 250), depth = {depth})
hole(top, \"z+\", world = (300, 200, 18), diameter = 8, depth = 10)\n"
        ))
    };
    assert!(
        kinds(&pocket(12.0)).contains(&"hole_breaks_through"),
        "{:?}",
        kinds(&pocket(12.0))
    );
    // A pocket 4 mm deep leaves the hole 6 mm of its depth in the material below.
    assert_eq!(kinds(&pocket(4.0)), Vec::<&str>::new());
}

/// Dowels into the 18 mm leg of an L profile leave the rest behind them in
/// the leg, not in the whole 100 mm the L reaches across.
#[test]
fn dowels_into_the_leg_of_an_l_profile_leave_its_rest() {
    let model = eval(
        "\
l = extrude(\"l\", profile = [(0, 0), (100, 0), (100, 18), (18, 18), (18, 100), (0, 100)], distance = 600)
b = box(\"b\", (70, 50, 600), at = (30, -50, 0))
dowels(l, b, dowel = \"8x35\")
",
    );
    let depths: Vec<f64> = model
        .part("l")
        .unwrap()
        .operations
        .iter()
        .filter_map(|operation| match operation {
            ProgramOperation::Hole(hole) => Some(hole.depth_mm),
            _ => None,
        })
        .collect();
    assert!(!depths.is_empty());
    assert!(depths.iter().all(|&depth| depth <= 12.0), "{depths:?}");
    assert_eq!(kinds(&model), Vec::<&str>::new());
}
