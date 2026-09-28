//! Checks take the exact answer for pairs whose boxes misstate their solids,
//! and a face pushed outward grows the part's box.
use ketchup_program::{
    COLLISION_UNVERIFIED, ExactPair, ExactShapes, RelationKind, Report, exact_candidates, run,
};

/// A disc seat (radius 150, 20 thick) on one 30 x 30 leg whose corner is at
/// (`x`, `x`): the boxes always touch, the disc only when the leg is under it.
fn seat_on_leg(x: f64) -> String {
    format!(
        "seat = revolve(\"seat\", profile = [(0, 0), (150, 0), (150, 20), (0, 20)], \
         axis = [(0, 0), (0, 1)], at = (0, 0, 400))\n\
         rotate(seat, axis = (1, 0, 0), angle = 90)\n\
         leg = box(\"leg\", (30, 30, 400), at = ({x}, {x}, 0))\n\
         expect_contact(seat, leg)\n"
    )
}

fn checked(source: &str) -> (ketchup_program::Evaluated, Report) {
    run("test.star", source, &Default::default()).expect("the program evaluates")
}

fn kinds(report: &Report) -> Vec<&str> {
    report.issues.iter().map(|issue| issue.kind).collect()
}

#[test]
fn boxes_alone_take_a_leg_under_the_corner_of_a_round_seat_as_support() {
    let (evaluated, report) = checked(&seat_on_leg(120.0));
    assert!(kinds(&report).is_empty(), "{:#?}", report.issues);
    assert_eq!(
        exact_candidates(&evaluated.model)
            .into_iter()
            .collect::<Vec<_>>(),
        ["seat"]
    );
}

#[test]
fn an_exact_gap_at_the_corner_leaves_the_round_seat_floating() {
    let (evaluated, mut report) = checked(&seat_on_leg(120.0));
    let mut exact = ExactShapes::default();
    // The leg's inner corner (120, 120) is 169.7 mm from the axis.
    exact.insert(
        "seat",
        "leg",
        ExactPair {
            distance_mm: Some(19.7),
            ..ExactPair::default()
        },
    );
    report.refine(&evaluated.model, &exact);
    assert_eq!(
        kinds(&report),
        ["expectation_failed", "floating_part"],
        "{:#?}",
        report.issues
    );
    assert!(report.issues[0].message.contains("measured 19.7"));
    assert_eq!(report.issues[1].parts, ["seat"]);
    let relation = &report.relations[0];
    assert_eq!(relation.kind, RelationKind::Gap);
    assert_eq!(relation.gap_mm, Some(19.7));
    assert!(!relation.approx);
}

#[test]
fn an_exact_contact_patch_reports_its_true_area() {
    let (evaluated, mut report) = checked(&seat_on_leg(60.0));
    let mut exact = ExactShapes::default();
    exact.insert(
        "leg",
        "seat",
        ExactPair {
            contact_area_mm2: 900.0,
            distance_mm: Some(0.0),
            ..ExactPair::default()
        },
    );
    report.refine(&evaluated.model, &exact);
    assert!(kinds(&report).is_empty(), "{:#?}", report.issues);
    let relation = &report.relations[0];
    assert_eq!(relation.kind, RelationKind::Contact);
    assert_eq!(relation.area_mm2, Some(900.0));
    assert!(!relation.approx);
}

#[test]
fn apart_by_an_unmeasured_distance_leaves_the_expectation_unverified() {
    let (evaluated, mut report) = checked(&seat_on_leg(120.0));
    let mut exact = ExactShapes::default();
    exact.insert("seat", "leg", ExactPair::default());
    report.refine(&evaluated.model, &exact);
    assert_eq!(
        kinds(&report),
        ["floating_part", "expectation_unverified"],
        "{:#?}",
        report.issues
    );
}

fn board(push: &str) -> String {
    format!(
        "board = extrude(\"board\", distance = 18, profile = [\
         [\"bottom\", [0, 0], [100, 0]], [\"slope\", [100, 0], [0, 100]], \
         [\"left\", [0, 100], [0, 0]]])\n{push}"
    )
}

fn board_bounds(push: &str) -> ([f64; 3], [f64; 3]) {
    let (evaluated, _) = checked(&board(push));
    evaluated.model.parts[0].local_bounds()
}

#[test]
fn a_face_pushed_out_grows_the_box_where_its_neighbours_meet_it() {
    assert_eq!(board_bounds(""), ([0.0, 0.0, 0.0], [100.0, 100.0, 18.0]));
    // The bottom moves to y = -10; the slope, extended, meets it at x = 110.
    let (min, max) =
        board_bounds("push_pull(board, face = \"bottom\", distance = 10, name = \"p\")\n");
    assert_eq!(min.map(|v| (v * 1e9).round() / 1e9), [0.0, -10.0, 0.0]);
    assert_eq!(max.map(|v| (v * 1e9).round() / 1e9), [110.0, 100.0, 18.0]);
    assert_eq!(
        board_bounds("push_pull(board, face = \"end\", distance = 12, name = \"p\")\n").1,
        [100.0, 100.0, 30.0]
    );
    // Pushed in, the box stays as it was.
    assert_eq!(
        board_bounds("push_pull(board, face = \"end\", distance = -6, name = \"p\")\n").1,
        [100.0, 100.0, 18.0]
    );
}

#[test]
fn a_part_grown_by_push_pull_into_a_neighbour_goes_to_the_exact_check() {
    // Before, the box ignored the raised top and never saw the cube.
    let (_, report) = checked(&board(
        "push_pull(board, face = \"end\", distance = 12, name = \"raise\")\n\
         c = box(\"c\", (10, 10, 20), at = (10, 10, 20))\n",
    ));
    assert!(
        kinds(&report).contains(&COLLISION_UNVERIFIED),
        "{:#?}",
        report.issues
    );
}
