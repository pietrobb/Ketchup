use ketchup_program::{Issue, Severity, run};
use std::collections::BTreeMap;

const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");

fn issues(source: &str, overrides: &[(&str, f64)]) -> Vec<Issue> {
    let overrides = overrides
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect::<BTreeMap<_, _>>();
    run("house.star", source, &overrides)
        .unwrap_or_else(|error| panic!("{error}"))
        .1
        .issues
}

/// `(part, message)` of every part reaching into a free space.
fn occupied(issues: &[Issue]) -> Vec<(String, String)> {
    issues
        .iter()
        .filter(|issue| issue.kind == "free_space_occupied")
        .map(|issue| (issue.parts[0].clone(), issue.message.clone()))
        .collect()
}

fn changed(from: &str, to: &str) -> String {
    assert!(HOUSE.contains(from), "{from}");
    HOUSE.replacen(from, to, 1)
}

#[test]
fn the_house_keeps_its_landings_headroom_stove_space_and_bed_free() {
    for overrides in [
        &[][..],
        &[("system", 1.0)],
        &[("roof_pitch", 50.0)],
        &[("length", 7400.0)],
    ] {
        let issues = issues(HOUSE, overrides);
        let spaces: Vec<_> = issues
            .iter()
            .filter(|issue| issue.kind.starts_with("free_space"))
            .collect();
        assert!(spaces.is_empty(), "{overrides:?}: {spaces:#?}");
    }
}

#[test]
fn a_window_over_the_bed_is_reported() {
    let source = changed(
        "    (\"east\", \"prízemie východ\"",
        "    (\"east\", \"podkrovie východ\", 1500, 2500, attic + 500, attic + 1500),\n    (\"east\", \"prízemie východ\"",
    );
    let found = occupied(&issues(&source, &[]));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, "okná/podkrovie východ");
    assert!(
        found[0].1.contains("podkrovie/posteľ rám/no okno above"),
        "{found:#?}"
    );
}

#[test]
fn a_stair_ending_at_the_wall_has_no_landing() {
    // The first step 200 mm from the west wall, as the house had it before.
    let found = occupied(&issues(
        &changed("stair_x = WALL + LANDING", "stair_x = WALL + 200"),
        &[],
    ));
    assert!(
        found.iter().any(
            |(part, message)| part.starts_with("konštrukcia/prízemie/stena západ/")
                && message.contains("schody/landing at the foot")
        ),
        "{found:#?}"
    );
}

#[test]
fn a_stair_under_the_low_roof_has_no_headroom_at_the_top() {
    // Along the north wall the roof comes down over the upper steps.
    let found = occupied(&issues(
        &changed(
            "stair_y = B / 2.0 - 100",
            "stair_y = B - WALL - STAIR_W - STRINGER",
        ),
        &[],
    ));
    assert!(
        found.iter().any(
            |(part, message)| part.starts_with("konštrukcia/strecha severná")
                && message.contains("schody/headroom 13")
        ),
        "{found:#?}"
    );
}

#[test]
fn something_in_front_of_the_stove_door_is_reported() {
    let source = changed(
        "no_window_over(bed",
        "box(\"skriňa\", (600, 600, 1800), at = (part_info(stove).max[0] + 400, part_info(stove).min[1], SLAB))\nno_window_over(bed",
    );
    let issues = issues(&source, &[]);
    let found = occupied(&issues);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, "skriňa");
    assert!(found[0].1.contains("kachle/miesto na prikladanie"));
    let issue = issues.iter().find(|i| i.parts == ["skriňa"]).unwrap();
    assert_eq!(issue.severity, Severity::Error);
    assert!(
        issue.hint.contains("1000 mm in front of kachle"),
        "{}",
        issue.hint
    );
}
