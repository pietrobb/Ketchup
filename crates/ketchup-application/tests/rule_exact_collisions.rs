//! Box overlaps a rule program cannot decide are settled by the exact solids.
use ketchup_application::{DocumentSession, SessionSettings, verify_rule_program_exact};
use ketchup_model::document::RuleProgramSource;
use ketchup_model::persistence::ContainerData;
use ketchup_program::COLLISION_UNVERIFIED;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

const ROUND_STOOL: &str = include_str!("../../../examples/programs/round_stool.star");

fn verified(source: String) -> (ketchup_program::Report, serde_json::Value) {
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session
        .apply_rule_program(
            RuleProgramSource {
                variants: Default::default(),
                file_name: "stool.star".to_owned(),
                source,
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
    let mut report = applied.report;
    let summary = verify_rule_program_exact(
        &applied.snapshot,
        &applied.model,
        &mut report,
        &ContainerData::default(),
        None,
        ketchup_application::program_isolation::CheckDeadline::after(Duration::from_secs(120)),
        Arc::new(AtomicBool::new(false)),
    )
    .expect("the program left overlaps for the exact check");
    (report, summary)
}

#[test]
fn legs_in_a_round_seat_without_holes_collide_in_the_exact_solids() {
    let source = ROUND_STOOL.replace("    subtract(seat, leg)\n", "    pass\n");
    let (report, summary) = verified(source);
    assert_eq!(summary["state"], "verified", "{summary}");
    assert_eq!(summary["collisions"], 3, "{summary}");
    let collisions: Vec<_> = report
        .issues
        .iter()
        .filter(|issue| issue.kind == "collision")
        .collect();
    assert_eq!(collisions.len(), 3, "{:#?}", report.issues);
    assert!(!report.ok);
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.kind == COLLISION_UNVERIFIED)
    );
}

fn exact_kinds(source: &str) -> (Vec<String>, serde_json::Value) {
    let (report, summary) = verified(source.to_owned());
    assert_eq!(summary["state"], "verified", "{summary}");
    let kinds = report
        .issues
        .iter()
        .map(|issue| issue.kind.to_owned())
        .filter(|kind| kind == "collision" || kind == COLLISION_UNVERIFIED)
        .collect();
    (kinds, summary)
}

#[test]
fn rounded_profile_corners_are_exact_arcs_in_the_solid() {
    // Corner arc centre (40, 40), radius 40: a 10 mm cube in the corner stays
    // outside the arc (its far corner is 42.4 mm from the centre), a cube
    // moved to (8, 8) reaches 31 mm from it and cuts into the top.
    let top = "top = extrude(\"top\", distance = 18, \
               profile = round_corners([[0, 0], [800, 0], [800, 500], [0, 500]], 40))\n";
    let (clear, summary) = exact_kinds(&format!(
        "{top}c = box(\"c\", (10, 10, 10), at = (0, 0, 4))\n"
    ));
    assert!(clear.is_empty(), "{clear:?} {summary}");
    assert_eq!(summary["cleared"], 1, "{summary}");
    let (hit, summary) = exact_kinds(&format!(
        "{top}c = box(\"c\", (10, 10, 10), at = (8, 8, 4))\n"
    ));
    assert_eq!(hit, ["collision"], "{summary}");
}

#[test]
fn an_arched_apron_leaves_its_arch_open() {
    // The arch rises from (0, 0) through (300, 60) to (600, 0); below it the
    // apron is empty.
    let apron = "apron = extrude(\"apron\", distance = 20, profile = [\
                 [\"top\", [600, 120], [0, 120]], [\"left\", [0, 120], [0, 0]], \
                 [\"arch\", [0, 0], [600, 0], {\"through\": (300, 60)}], \
                 [\"right\", [600, 0], [600, 120]]])\n";
    let (clear, summary) = exact_kinds(&format!(
        "{apron}c = box(\"c\", (40, 40, 10), at = (280, 0, 5))\n"
    ));
    assert!(clear.is_empty(), "{clear:?} {summary}");
    let (hit, summary) = exact_kinds(&format!(
        "{apron}c = box(\"c\", (40, 40, 10), at = (280, 30, 5))\n"
    ));
    assert_eq!(hit, ["collision"], "{summary}");
}

#[test]
fn a_curved_rail_swept_around_a_bend_leaves_the_outer_corner_empty() {
    // The rail's 30 x 20 section follows x to (600, 0) and turns to +y about
    // (500, 100) with radius 100, so it fills 85..115 mm from that centre.
    let rail = "rail = sweep(\"rail\", profile = [(-15, 0), (15, 0), (15, 20), (-15, 20)], \
                path = [(0, 0, 0), (600, 0, 0), (600, 400, 0)], bend = 100)\n";
    let (clear, summary) = exact_kinds(&format!(
        "{rail}c = box(\"c\", (20, 20, 10), at = (575, -10, 5))\n"
    ));
    assert!(clear.is_empty(), "{clear:?} {summary}");
    assert_eq!(summary["cleared"], 1, "{summary}");
    let (hit, summary) = exact_kinds(&format!(
        "{rail}c = box(\"c\", (20, 20, 10), at = (540, -10, 5))\n"
    ));
    assert_eq!(hit, ["collision"], "{summary}");
}

#[test]
fn a_tapered_loft_leg_narrows_towards_its_top() {
    // 40 mm square at the floor, 24 mm square (inset 8) at 700 mm: 600 mm up
    // the side is inset 6.86 mm, 50 mm up only 0.57 mm.
    let leg = "leg = loft(\"leg\", sections = [\
               ([(0, 0), (40, 0), (40, 40), (0, 40)], 0), \
               ([(8, 8), (32, 8), (32, 32), (8, 32)], 700)])\n";
    let (clear, summary) = exact_kinds(&format!(
        "{leg}c = box(\"c\", (6, 6, 50), at = (0, 0, 600))\n"
    ));
    assert!(clear.is_empty(), "{clear:?} {summary}");
    assert_eq!(summary["cleared"], 1, "{summary}");
    let (hit, summary) = exact_kinds(&format!(
        "{leg}c = box(\"c\", (6, 6, 50), at = (0, 0, 20))\n"
    ));
    assert_eq!(hit, ["collision"], "{summary}");
}

#[test]
fn a_lofted_tool_hollows_a_tapered_pocket_through_a_block() {
    let (clear, summary) = exact_kinds(
        "block = box(\"block\", (60, 60, 60))\n\
         cavity = loft(\"cavity\", tool = True, sections = [\
         ([(10, 10), (50, 10), (50, 50), (10, 50)], -1), \
         ([(20, 20), (40, 20), (40, 40), (20, 40)], 61)])\n\
         subtract(block, cavity)\n\
         c = box(\"c\", (10, 10, 10), at = (25, 25, 25))\n",
    );
    assert!(clear.is_empty(), "{clear:?} {summary}");
    assert_eq!(summary["cleared"], 1, "{summary}");
}

/// A disc seat (radius 150, 20 thick) on one 30 x 30 leg whose corner is at
/// (`x`, `x`): the boxes always touch, the disc only when the leg is under it.
fn seat_on_leg(x: f64) -> String {
    format!(
        "seat = revolve(\"seat\", profile = [(0, 0), (150, 0), (150, 20), (0, 20)], \
         axis = [(0, 0), (0, 1)], at = (0, 0, 400))\n\
         rotate(seat, axis = (1, 0, 0), angle = 90)\n\
         leg = box(\"leg\", (30, 30, 400), at = ({x}, {x}, 0))\n"
    )
}

fn kinds(report: &ketchup_program::Report) -> Vec<&str> {
    report.issues.iter().map(|issue| issue.kind).collect()
}

#[test]
fn a_leg_under_the_corner_of_a_round_seat_does_not_hold_it() {
    let (report, summary) = verified(seat_on_leg(120.0));
    assert_eq!(summary["state"], "verified", "{summary}");
    assert_eq!(kinds(&report), ["floating_part"], "{:#?}", report.issues);
    assert_eq!(report.issues[0].parts, ["seat"]);
    let relation = &report.relations[0];
    assert_eq!(
        relation.kind,
        ketchup_program::RelationKind::Gap,
        "{relation:?}"
    );
    // The leg's inner corner (120, 120) is 169.71 mm from the axis.
    assert_eq!(relation.gap_mm, Some(19.7), "{relation:?}");
}

#[test]
fn a_leg_under_a_round_seat_touches_it_over_its_whole_top() {
    let (report, summary) = verified(seat_on_leg(60.0));
    assert_eq!(summary["state"], "verified", "{summary}");
    assert!(kinds(&report).is_empty(), "{:#?}", report.issues);
    let relation = &report.relations[0];
    assert_eq!(
        relation.kind,
        ketchup_program::RelationKind::Contact,
        "{relation:?}"
    );
    assert_eq!(relation.area_mm2, Some(900.0), "{relation:?}");
    assert!(!relation.approx);
}

#[test]
fn a_top_raised_by_push_pull_collides_with_the_part_above() {
    let board = "board = extrude(\"board\", distance = 18, \
                 profile = [(0, 0), (300, 0), (300, 100), (0, 100)])\n\
                 push_pull(board, face = \"end\", distance = 12, name = \"raise\")\n";
    let (hit, summary) = exact_kinds(&format!(
        "{board}c = box(\"c\", (10, 10, 20), at = (10, 10, 20))\n"
    ));
    assert_eq!(hit, ["collision"], "{summary}");
    let (clear, summary) = exact_kinds(&format!(
        "{board}c = box(\"c\", (10, 10, 20), at = (10, 10, 30))\n"
    ));
    assert!(clear.is_empty(), "{clear:?} {summary}");
}

/// A 600 x 500 x 150 drawer with 12 mm walls, open at the top, holding a
/// board turned 45° (a 420 mm diamond, 10 thick) on its floor with its left
/// corner at `corner_x`, and a 6 x 6 rod spanning the inside, `rod_extra` mm
/// past the inner face of each side wall.
fn drawer_with_board_and_rod(corner_x: f64, rod_extra: f64) -> String {
    let (cx, inner) = (corner_x + 210.0, 576.0 + 2.0 * rod_extra);
    format!(
        "drawer = box(\"drawer\", (600, 500, 150))\n\
         shell(drawer, thickness = 12, open = [\"z+\"])\n\
         board = extrude(\"board\", distance = 10, at = (0, 0, 12), profile = [\
         ({}, 250), ({cx}, 40), ({}, 250), ({cx}, 460)])\n\
         rod = box(\"rod\", ({inner}, 6, 6), at = ({}, 100, 50))\n",
        cx - 210.0,
        cx + 210.0,
        12.0 - rod_extra,
    )
}

/// N-1 (review status 2026-10-16): the drawer's box holds the board and the
/// rod whole, so the overlap of the boxes is the entire board or rod. Only
/// overlap as thin as the contact fit is a touch, however large the boxes'
/// common face is.
#[test]
fn a_board_corner_cut_into_a_drawer_wall_and_a_rod_too_long_for_it_collide() {
    // The corner reaches 9.5 mm into the 12 mm wall (902.5 mm³), the rod
    // 0.4 mm into each wall (28.8 mm³).
    let (hit, summary) = exact_kinds(&drawer_with_board_and_rod(2.5, 0.4));
    assert_eq!(hit, ["collision", "collision"], "{summary}");
    // At the wall's inner face and 0.005 mm into each wall they touch.
    let (clear, summary) = exact_kinds(&drawer_with_board_and_rod(12.0, 0.005));
    assert!(clear.is_empty(), "{clear:?} {summary}");
}

/// The rim of a 500 mm drawer with 12 mm walls is too narrow for a grid of
/// sample points; it lost its name and the exact check of the drawer failed.
#[test]
fn a_drawer_with_walls_thin_against_its_size_is_checked_on_its_exact_solid() {
    let (report, summary) = verified(
        "drawer = box(\"drawer\", (500, 500, 150))\n\
         shell(drawer, thickness = 12, open = [\"z+\"])\n\
         block = box(\"block\", (100, 100, 50), at = (200, 200, 12))\n"
            .to_owned(),
    );
    assert_eq!(summary["state"], "verified", "{summary}");
    assert!(kinds(&report).is_empty(), "{:#?}", report.issues);
}

/// Review #136: a refine the worker refuses (here no time is left) keeps the
/// report's own issues; a report that left them for the exact check lists them
/// first, so both answer with the same issues, relations and summary.
#[test]
fn a_refused_refine_answers_with_the_issues_the_report_left_for_it() {
    use ketchup_application::program_isolation::{CheckDeadline, report_program_before_exact};
    let source = "t = extrude(\"t\", profile = [(0, 0), (100, 0), (0, 100)], distance = 20)\n\
                  c = box(\"c\", (20, 20, 20), at = (70, 70, 0))\n\
                  d = box(\"d\", (20, 20, 20), at = (0, 0, 20))\n\
                  e = box(\"e\", (5, 5, 5), at = (0, 0, 300))\n";
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session
        .apply_rule_program(
            RuleProgramSource {
                variants: Default::default(),
                file_name: "left.star".to_owned(),
                source: source.to_owned(),
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
    let evaluated = ketchup_program::evaluate("left.star", source, &Default::default()).unwrap();
    let cancel = ketchup_program::CancelToken::default();
    let pending = report_program_before_exact(
        &evaluated,
        CheckDeadline::after(Duration::from_secs(120)),
        &cancel,
    )
    .unwrap();
    assert!(pending.checks_pending());
    assert!(
        !applied.report.relations.is_empty() && applied.report.issues.len() > pending.issues.len(),
        "{:#?}",
        applied.report
    );
    let answer = (
        serde_json::json!({"complete": true, "not_evaluated": [], "checked_pair_count": 0}),
        Default::default(),
    );
    let refused = |mut report: ketchup_program::Report| {
        let (summary, _) = ketchup_application::apply_exact_pairs(
            &mut report,
            &applied.model,
            ketchup_program::ExactShapes::default(),
            &answer,
            CheckDeadline::after(Duration::ZERO),
            &cancel,
        );
        (report, summary)
    };
    let (today, today_summary) = refused(applied.report.clone());
    let (left, left_summary) = refused(pending);
    assert_eq!(today_summary["state"], "incomplete", "{today_summary}");
    assert_eq!(left, today);
    assert_eq!(left_summary, today_summary);
}

#[test]
fn a_part_inside_the_empty_corner_of_a_triangle_is_cleared() {
    // The cube sits in the corner the triangle leaves empty; the boxes overlap.
    let source = "t = extrude(\"t\", profile = [(0, 0), (100, 0), (0, 100)], distance = 20)\n\
                  c = box(\"c\", (20, 20, 20), at = (70, 70, 0))\n"
        .to_owned();
    let (report, summary) = verified(source);
    assert_eq!(summary["state"], "verified", "{summary}");
    assert_eq!(summary["cleared"], 1, "{summary}");
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.kind == COLLISION_UNVERIFIED || issue.kind == "collision"),
        "{:#?}",
        report.issues
    );
}
