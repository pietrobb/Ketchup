use ketchup_program::{Issue, Severity, run};
use std::collections::BTreeMap;

fn source(depth: f64, offset: f64) -> String {
    format!(
        "p=box('divider',(100,100,18))\nhole(p,'z-',at=(40,50),diameter=8,depth=8,id='left')\nhole(p,'z+',at=({},50),diameter=8,depth={depth},id='right')\n",
        40. + offset
    )
}

fn issues(source: &str) -> Vec<Issue> {
    run("opposing.star", source, &BTreeMap::new())
        .unwrap()
        .1
        .issues
        .into_iter()
        .filter(|issue| issue.kind.starts_with("opposing_holes"))
        .collect()
}

#[test]
fn opposing_blind_holes_report_the_remaining_wall_and_meeting_point() {
    for (depth, severity, gap, bounds) in [
        (
            8.,
            Severity::Warning,
            "2 mm",
            ([40., 50., 8.], [40., 50., 10.]),
        ),
        (
            10.,
            Severity::Error,
            "0 mm",
            ([40., 50., 8.], [40., 50., 8.]),
        ),
        (
            11.,
            Severity::Error,
            "0 mm",
            ([40., 50., 7.5], [40., 50., 7.5]),
        ),
    ] {
        let found = issues(&source(depth, 0.));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].severity, severity);
        assert_eq!(found[0].parts, ["divider"]);
        assert!(found[0].message.contains(gap), "{found:?}");
        assert!(found[0].message.contains("left") && found[0].message.contains("right"));
        assert!(found[0].message.contains("3 mm"));
        assert_eq!(found[0].where_mm, Some(bounds));
    }
    for depth in [6., 7.] {
        assert!(issues(&source(depth, 0.)).is_empty());
    }
}

#[test]
fn staggered_holes_use_cylinder_clearance_not_only_centerlines_or_depths() {
    // Radii total 8 mm: overlapping disks, tangent disks, a 2 mm lateral
    // wall, and a safe 3 mm wall even though the depths overlap.
    for (offset, count, severity) in [
        (7., 1, Severity::Error),
        (8., 1, Severity::Error),
        (10., 1, Severity::Warning),
        (11., 0, Severity::Warning),
        (20., 0, Severity::Warning),
    ] {
        let found = issues(&source(11., offset));
        assert_eq!(found.len(), count, "offset {offset}: {found:?}");
        if count > 0 {
            assert_eq!(found[0].severity, severity);
        }
    }
    // 2 mm axial + 2 mm radial clearance: sqrt(8) < 3, not 4.
    let found = issues(&source(8., 10.));
    assert_eq!(found.len(), 1);
    assert!(found[0].message.contains("2.828 mm"), "{found:?}");
    assert!(issues(&source(8., 11.)).is_empty());
}

#[test]
fn finished_holes_and_shared_instances_report_world_locations_independently() {
    let text = source(8., 0.)
        + "mirror(p,axis='z')\nc=component('unit',[p])\ninstance('copy',c,at=(200,0,0),x=(0,0,-1),z=(1,0,0))\n";
    let found = issues(&text);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].where_mm, Some(([40., 50., 8.], [40., 50., 10.])));
    let copy = found.iter().find(|i| i.parts == ["copy/divider"]).unwrap();
    assert_eq!(copy.where_mm, Some(([208., 50., -40.], [210., 50., -40.])));
}

#[test]
fn same_side_different_parts_and_reversed_declaration_order_do_not_confuse_pairs() {
    assert!(issues(&source(8., 0.).replace("'z+'", "'z-'")).is_empty());
    assert!(
        issues(&source(8., 0.).replace(
            "hole(p,'z+'",
            "q=box('other',(100,100,18),at=(200,0,0))\nhole(q,'z+'"
        ))
        .is_empty()
    );
    let text = source(8., 0.);
    let lines: Vec<_> = text.lines().collect();
    let swapped = format!("{}\n{}\n{}\n", lines[0], lines[2], lines[1]);
    let found = issues(&swapped);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].where_mm, Some(([40., 50., 8.], [40., 50., 10.])));
}

#[test]
fn explicit_dowel_offsets_pair_both_sides_of_shared_plates_and_announce_positions() {
    for stock in [
        "p=box('shared',(18,300,300))\na=box('back brace',(100,300,18),at=(-100,0,120))\nb=box('base',(100,300,18),at=(18,0,120))\n",
        "p=box('shared',(300,300,18))\na=box('back brace',(300,18,100),at=(0,120,18))\nb=box('plinth',(300,18,100),at=(0,120,-100))\n",
    ] {
        for placement in [
            "",
            "for part in (p,a,b):\n    rotate(part,axis=(1,2,3),angle=31,pivot=(0,0,0))\n    move(part,by=(70,-40,90))\n",
        ] {
            let source =
                format!("{stock}{placement}dowels(p,a,count=2)\ndowels(p,b,count=2,offset=11)\n");
            let (evaluated, report) = run("offset.star", &source, &BTreeMap::new()).unwrap();
            assert!(issues(&source).is_empty(), "{:?}", report.issues);
            assert_eq!(report.bom.hardware[0].count, 4);
            assert_eq!(report.log.len(), 1);
            assert!(report.log[0].contains("explicit row offset 11 mm"));
            assert!(report.log[0].contains("world centres"));
            let shared = evaluated.model.part("shared").unwrap();
            assert_eq!(shared.holes().count(), 4);
            let entries: Vec<_> = shared.holes().map(|hole| hole.entry_mm).collect();
            for shifted in &entries[2..] {
                let nearest = entries[..2]
                    .iter()
                    .map(|old| ketchup_geometry::linalg::distance(*old, *shifted))
                    .fold(f64::INFINITY, f64::min);
                assert!((nearest - 18_f64.hypot(11.)).abs() < 1.0e-6, "{entries:?}");
            }
            for joint in &evaluated.model.joints {
                let other = evaluated.model.part(&joint.parts[1]).unwrap();
                let paired: Vec<_> = shared
                    .holes()
                    .filter(|h| h.id.starts_with(&format!("dowel:{}:", other.name)))
                    .collect();
                assert_eq!(paired.len(), 2);
                for ((left, right), centre) in
                    paired.iter().zip(other.holes()).zip(&joint.fasteners_mm)
                {
                    assert_eq!(left.depth_mm, 12.);
                    assert_eq!(right.depth_mm, 26.);
                    assert!(!left.through && !right.through);
                    for point in [
                        shared.to_world(left.entry_mm),
                        other.to_world(right.entry_mm),
                    ] {
                        for axis in 0..3 {
                            assert!((point[axis] - centre[axis]).abs() < 1.0e-6);
                        }
                    }
                }
            }
            let unsafe_source = source.replace("offset=11", "offset=0");
            let unsafe_issues = issues(&unsafe_source);
            assert_eq!(unsafe_issues.len(), 2, "{unsafe_issues:?}");
            assert!(
                unsafe_issues
                    .iter()
                    .all(|issue| issue.severity == Severity::Error)
            );
            let thin = issues(&source.replace("offset=11", "offset=10"));
            assert_eq!(thin.len(), 2, "{thin:?}");
            assert!(thin.iter().all(|issue| issue.severity == Severity::Warning));
            let impossible = run(
                "offset.star",
                &source.replace("offset=11", "offset=300"),
                &BTreeMap::new(),
            )
            .unwrap();
            assert!(!impossible.1.ok);
            assert!(
                impossible
                    .1
                    .issues
                    .iter()
                    .any(|issue| issue.message.contains("offset 300 mm")
                        && issue.message.contains("no row fits"))
            );
            assert_eq!(impossible.0.model.joints[1].links.len(), 2);
            for (requested, original) in impossible.0.model.joints[1]
                .fasteners_mm
                .iter()
                .zip(&evaluated.model.joints[1].fasteners_mm)
            {
                let shift = (0..3)
                    .map(|axis| (requested[axis] - original[axis]).powi(2))
                    .sum::<f64>()
                    .sqrt();
                assert!(
                    (shift - 289.0).abs() < 0.001,
                    "offset must remain 300 rather than be clamped: {shift}"
                );
            }
        }
    }
}

#[test]
fn impossible_join_patterns_report_issues_without_aborting_or_inventing_machining() {
    let parts = "a=box('a',(100,100,18))\nb=box('b',(18,100,80),at=(0,0,18))\nhole(a,'z-',at=(70,70),diameter=4,depth=5,id='service')\n";
    for suffix in [
        "dowels(a,b,count=30)\n",
        "dowels(a,b,margin=500)\n",
        "move(b,by=(0,0,20))\ndowels(a,b)\n",
    ] {
        let (evaluated, report) = run(
            "impossible.star",
            &format!("{parts}{suffix}box('later',(10,10,10),at=(300,0,0))\n"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(!report.ok);
        assert_eq!(evaluated.model.parts.len(), 3);
        assert!(evaluated.model.joints.is_empty());
        assert!(report.bom.hardware.is_empty());
        assert_eq!(evaluated.model.part("a").unwrap().holes().count(), 1);
        assert_eq!(
            evaluated
                .model
                .part("a")
                .unwrap()
                .holes()
                .next()
                .unwrap()
                .id,
            "service"
        );
        assert_eq!(evaluated.model.part("b").unwrap().holes().count(), 0);
        let issue = report
            .issues
            .iter()
            .find(|issue| issue.kind == "program_condition_failed")
            .unwrap();
        assert_eq!(issue.parts, ["a", "b"]);
        assert!(issue.hint.contains("No joint or holes were generated"));
    }
    let (_, repaired) = run(
        "repaired.star",
        &format!("{parts}dowels(a,b,count=2,margin=20)\n"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(repaired.ok, "{:?}", repaired.issues);
    assert_eq!(repaired.bom.hardware[0].count, 2);
}

#[test]
fn explicit_through_intent_keeps_drilling_metadata_and_does_not_excuse_other_issues() {
    let base = "p=box('plate',(100,100,18))\nhole(p,'z+',at=(50,50),diameter=8,depth=18,id='bore',through=True)\n";
    for placement in [
        "",
        "mirror(p,axis='z')\nrotate(p,axis=(1,2,3),angle=31,pivot=(0,0,0))\n",
    ] {
        let source = format!("{base}{placement}");
        let (evaluated, report) = run("through.star", &source, &BTreeMap::new()).unwrap();
        assert!(
            !report
                .issues
                .iter()
                .any(|i| i.kind.starts_with("hole_") || i.kind.starts_with("through_hole")),
            "{:?}",
            report.issues
        );
        let operation = &report.bom.machining[0].operations[0];
        assert_eq!(operation.id, "bore");
        assert_eq!(operation.kind, "drill");
        assert_eq!(operation.diameter_mm, Some(8.));
        assert_eq!(operation.depth_mm, 18.);
        assert_eq!(operation.through, Some(true));
        assert_eq!(serde_json::to_value(operation).unwrap()["through"], true);
        let (blind, blind_report) = run(
            "blind.star",
            &source.replace("through=True", "through=False"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(
            blind_report
                .issues
                .iter()
                .any(|i| i.kind == "hole_breaks_through")
        );
        // CAD receives explicit intent without changing the requested dimensions.
        let mut through_cad =
            serde_json::to_value(ketchup_program::cad::part(&evaluated.model.parts[0])).unwrap();
        let blind_cad =
            serde_json::to_value(ketchup_program::cad::part(&blind.model.parts[0])).unwrap();
        assert_eq!(through_cad["holes"][0]["through"], true);
        assert_eq!(blind_cad["holes"][0]["through"], false);
        through_cad["holes"][0]["through"] = false.into();
        assert_eq!(through_cad, blind_cad);
        assert_eq!(
            blind_report.bom.machining[0].operations[0].through,
            Some(false)
        );
    }
    for (source, kind) in [
        (
            base.replace("depth=18", "depth=12"),
            "through_hole_too_shallow",
        ),
        (base.replace("at=(50,50)", "at=(2,50)"), "hole_outside_face"),
        (
            format!("{base}hole(p,'z-',at=(50,50),diameter=8,depth=8,id='blind')\n"),
            "opposing_holes_intersect",
        ),
    ] {
        let (_, report) = run("through.star", &source, &BTreeMap::new()).unwrap();
        assert!(
            report.issues.iter().any(|issue| issue.kind == kind),
            "{:?}",
            report.issues
        );
    }
}
