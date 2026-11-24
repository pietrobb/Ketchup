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
#[ignore = "large-house: runs in full suite"]
fn large_house_the_house_keeps_its_landings_headroom_stove_space_and_bed_free() {
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
#[ignore = "large-house: runs in full suite"]
fn large_house_a_window_over_the_bed_is_reported() {
    let source = changed(
        "        (\"east\", \"okno\", \"okná/prízemie východ\"",
        "        (\"east\", \"okno\", \"okná/podkrovie východ\", 1500, attic + 500 - SLAB, 1000, 1000),\n        (\"east\", \"okno\", \"okná/prízemie východ\"",
    );
    let found = occupied(&issues(&source, &[]));
    // The bed's zone reaches y = 2010: the right frame piece (y 2420..2500) is beyond it.
    let parts: Vec<_> = found.iter().map(|(part, _)| part.as_str()).collect();
    assert_eq!(
        parts,
        [
            "okná/podkrovie východ/frame bottom",
            "okná/podkrovie východ/frame top",
            "okná/podkrovie východ/frame left",
            "okná/podkrovie východ/glass"
        ],
        "{found:#?}"
    );
    assert!(
        found
            .iter()
            .all(|(_, message)| message.contains("podkrovie/posteľ rám/no window above")),
        "{found:#?}"
    );
}

#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_a_stair_ending_at_the_wall_has_no_landing() {
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
#[ignore = "large-house: runs in full suite"]
fn large_house_a_stair_under_the_low_roof_has_no_headroom_at_the_top() {
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

/// R1109-PG-308: at 30° and 35° the north roof over a 1000 mm knee wall comes
/// down into the landing at the top of the stair, which is reported, not
/// passed; the knee wall the parameter's doc names for that pitch frees it.
#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_a_low_roof_needs_the_knee_wall_its_doc_names_over_the_stair() {
    for pitch in [30.0, 35.0] {
        let found = occupied(&issues(HOUSE, &[("roof_pitch", pitch)]));
        assert!(!found.is_empty(), "{pitch}°");
        assert!(
            found
                .iter()
                .all(|(part, message)| (part.starts_with("strecha/severná")
                    || part.starts_with("konštrukcia/strecha severná")
                    || part == "konštrukcia/hrebeňová väznica")
                    && message.contains("schody/")),
            "{pitch}°: {found:#?}"
        );
        assert!(
            found
                .iter()
                .any(|(_, message)| message.contains("schody/landing at the top")),
            "{pitch}°: {found:#?}"
        );
    }
    assert!(HOUSE.contains("pri sklone 30° aspoň 1300 mm, pri 35° aspoň 1100 mm"));
    for overrides in [
        &[("roof_pitch", 30.0), ("knee_wall", 1300.0)][..],
        &[("roof_pitch", 35.0), ("knee_wall", 1100.0)],
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
#[ignore = "large-house: runs in full suite"]
fn large_house_something_in_front_of_the_stove_door_is_reported() {
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
