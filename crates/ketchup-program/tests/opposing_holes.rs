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
