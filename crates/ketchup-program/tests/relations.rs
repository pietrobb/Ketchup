use ketchup_program::{
    COLLISION_UNVERIFIED, Issue, OverlapStatus, Relation, RelationKind, Report, Severity, run,
};
use std::collections::BTreeMap;

const TABLE: &str = include_str!("../../../examples/programs/table.star");
const ROUND_STOOL: &str = include_str!("../../../examples/programs/round_stool.star");

fn report(source: &str) -> Report {
    run("test.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

fn between<'r>(report: &'r Report, a: &str, b: &str) -> &'r Relation {
    report
        .relations
        .iter()
        .find(|relation| {
            relation.parts.contains(&a.to_owned()) && relation.parts.contains(&b.to_owned())
        })
        .unwrap_or_else(|| panic!("no relation {a} - {b}: {:#?}", report.relations))
}

fn json_len(report: &Report) -> usize {
    serde_json::to_vec(&report.relations).unwrap().len()
}

#[test]
fn table_map_shows_each_leg_standing_under_the_top_on_its_dowel_joint() {
    let report = report(TABLE);
    assert_eq!(report.relations.len(), 4, "{:#?}", report.relations);
    for leg in ["front-left", "front-right", "back-left", "back-right"] {
        let leg = format!("table/leg-{leg}");
        let relation = between(&report, "table/top", &leg);
        assert_eq!(relation.parts, ["table/top".to_owned(), leg.clone()]);
        assert_eq!(relation.kind, RelationKind::Contact);
        assert_eq!(relation.faces, Some(["z-".to_owned(), "z+".to_owned()]));
        let material_area = 4900.0 - 2.0 * std::f64::consts::PI * 4.0_f64.powi(2);
        assert!(
            (relation.area_mm2.expect("contact area") - material_area).abs() < 1.0,
            "{relation:?}"
        );
        // The leg lies below the top.
        assert_eq!(relation.direction, [0.0, 0.0, -1.0]);
        assert_eq!(
            relation.joint.as_deref(),
            Some(format!("dowel:table/top+{leg}").as_str())
        );
        assert!(!relation.approx);
    }
    assert!(json_len(&report) < 2048, "{}", json_len(&report));
}

#[test]
fn stool_map_shows_legs_under_the_seat_and_how_deep_rails_enter_legs() {
    let report = report(ROUND_STOOL);
    for leg in ["leg1", "leg2", "leg3"] {
        let relation = between(&report, "seat", leg);
        assert_eq!(relation.kind, RelationKind::Overlap);
        assert_eq!(relation.status, Some(OverlapStatus::Subtracted));
        assert_eq!(relation.cut_in.as_deref(), Some("seat"));
        // From the seat towards the leg points down: the leg is under it.
        assert!(relation.direction[2] < -0.9, "{relation:?}");
        // The round seat is measured on its box.
        assert!(relation.approx);
    }
    for (leg, rail) in [
        ("leg1", "rail1"),
        ("leg2", "rail1"),
        ("leg2", "rail2"),
        ("leg3", "rail2"),
        ("leg3", "rail3"),
        ("leg1", "rail3"),
    ] {
        let relation = between(&report, leg, rail);
        assert_eq!(relation.status, Some(OverlapStatus::Subtracted));
        assert_eq!(relation.cut_in.as_deref(), Some(leg));
        let depth = relation.depth_mm.unwrap();
        // Rails run from leg centre line to leg centre line: they reach at
        // least half the 36 mm leg in, and not through it.
        assert!((18.0..36.0).contains(&depth), "{relation:?}");
        // Rails are horizontal.
        assert!(relation.direction[2].abs() < 1.0e-9, "{relation:?}");
    }
    assert!(
        report
            .relations
            .iter()
            .all(|relation| relation.kind != RelationKind::Overlap
                || relation.status == Some(OverlapStatus::Subtracted))
    );
    assert!(json_len(&report) < 4096, "{}", json_len(&report));
}

#[test]
fn socket_depth_is_how_far_the_part_reaches_in_along_its_own_axis() {
    let report = report(
        "leg = box(\"leg\", (40, 40, 400))\n\
         rail = box(\"rail\", (300, 20, 30), at = (25, 10, 200))\n\
         subtract(leg, rail)\n",
    );
    let relation = between(&report, "leg", "rail");
    assert_eq!(relation.kind, RelationKind::Overlap);
    assert_eq!(relation.status, Some(OverlapStatus::Subtracted));
    assert_eq!(relation.cut_in.as_deref(), Some("leg"));
    assert_eq!(relation.depth_mm, Some(15.0));
    assert_eq!(relation.direction, [1.0, 0.0, 0.0]);
}

#[test]
fn trims_bound_the_socket_depth() {
    // A post would run through a 30 mm seat but is trimmed 20 mm into it.
    let report = report(
        "seat = box(\"seat\", (400, 400, 30), at = (-200, -200, 420))\n\
         post = member(\"post\", (0, 0, 0), (0, 0, 520), (36, 36))\n\
         trim(post, (0, 0, 440), (0, 0, 1))\n\
         subtract(seat, post)\n",
    );
    let relation = between(&report, "seat", "post");
    assert_eq!(relation.status, Some(OverlapStatus::Subtracted));
    assert_eq!(relation.depth_mm, Some(20.0));
    assert_eq!(relation.direction, [0.0, 0.0, -1.0]);
}

#[test]
fn nearby_parts_report_their_clearance_and_far_ones_are_left_out() {
    let report = report(
        "a = box(\"a\", (100, 100, 18))\n\
         b = box(\"b\", (100, 100, 18), at = (103, 0, 0))\n\
         c = box(\"c\", (100, 100, 18), at = (500, 0, 0))\n",
    );
    assert_eq!(report.relations.len(), 1, "{:#?}", report.relations);
    let relation = between(&report, "a", "b");
    assert_eq!(relation.kind, RelationKind::Gap);
    assert_eq!(relation.gap_mm, Some(3.0));
    assert_eq!(relation.direction, [1.0, 0.0, 0.0]);
}

#[test]
fn edge_only_touch_and_plain_collision_are_told_apart() {
    let report = report(
        "a = box(\"a\", (100, 100, 18))\n\
         b = box(\"b\", (100, 100, 18), at = (100, 100, 0))\n\
         c = box(\"c\", (100, 100, 18), at = (90, 0, 0))\n",
    );
    assert_eq!(between(&report, "a", "b").kind, RelationKind::Touch);
    let collision = between(&report, "a", "c");
    assert_eq!(collision.kind, RelationKind::Overlap);
    assert_eq!(collision.status, Some(OverlapStatus::Collision));
    assert_eq!(collision.depth_mm, Some(10.0));
}

#[test]
fn exact_verdicts_settle_unverified_overlaps() {
    // A round seat's box reaches past the circle into the corner post.
    let mut report = report(
        "seat = revolve(\"seat\", profile = [(0, 0), (200, 0), (200, 30), (0, 30)], axis = [(0, 0), (0, 1)], at = (0, 0, 0))\n\
         rotate(seat, axis = (1, 0, 0), angle = 90)\n\
         post = box(\"post\", (20, 20, 30), at = (175, 175, 0))\n",
    );
    assert_eq!(
        between(&report, "seat", "post").status,
        Some(OverlapStatus::Unverified)
    );
    let cleared: Vec<Issue> = report
        .issues
        .iter()
        .filter(|issue| issue.kind != COLLISION_UNVERIFIED)
        .cloned()
        .collect();
    report.set_issues(cleared);
    assert_eq!(
        between(&report, "seat", "post").status,
        Some(OverlapStatus::BoxesOnly)
    );
    let mut collision = report.clone();
    collision.set_issues(vec![Issue {
        severity: Severity::Error,
        kind: "collision",
        parts: vec!["seat".to_owned(), "post".to_owned()],
        message: String::new(),
        where_mm: None,
        hint: String::new(),
    }]);
    assert_eq!(
        between(&collision, "seat", "post").status,
        Some(OverlapStatus::Collision)
    );
}
