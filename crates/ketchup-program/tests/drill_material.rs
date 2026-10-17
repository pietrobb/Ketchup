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
