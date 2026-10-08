use ketchup_program::joint_check::NO_RATING;
use ketchup_program::loads::MemberLoad;
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");

fn report(source: &str, overrides: &[(&str, f64)]) -> Report {
    let overrides: BTreeMap<String, f64> = overrides
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect();
    run("loads.star", source, &overrides)
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

fn member<'a>(report: &'a Report, name: &str) -> &'a MemberLoad {
    report
        .loads
        .members
        .iter()
        .find(|member| member.part == name)
        .unwrap_or_else(|| panic!("no loads on {name}"))
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 0.15, "{actual} != {expected}");
}

/// A C24 beam 3000 x 100 x 200 mm on two posts, centres 50 and 2950 mm along it.
const BEAM: &str = "
load_path(only = [\"frame\"])
self_weight([\"construction\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"floor\"], source = \"test\")
box(\"post a\", (100, 100, 2000), material = \"C24\", tags = [\"frame\", \"construction\"])
box(\"post b\", (100, 100, 2000), at = (2900, 0, 0), material = \"C24\", tags = [\"frame\", \"construction\"])
box(\"beam\", (3000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\", \"construction\"])
";

#[test]
fn a_beam_passes_its_weight_and_what_lies_on_it_to_its_posts_by_the_lever_rule() {
    let weight = |volume_m3: f64| volume_m3 * 420.0 * 9.81;
    let even = report(BEAM, &[]);
    let beam = member(&even, "beam");
    assert_eq!(beam.missing, Vec::<String>::new());
    close(beam.length_mm, 3000.0);
    close(beam.loads_n["permanent"], weight(0.06));
    let [a, b] = beam.reactions.as_slice() else {
        panic!("{:?}", beam.reactions);
    };
    let (a, b) = if a.part == "post a" { (a, b) } else { (b, a) };
    close(a.loads_n["permanent"], weight(0.06) / 2.0);
    close(b.loads_n["permanent"], weight(0.06) / 2.0);

    // A 300 x 100 x 50 mm board centred 750 mm along the beam with 2 kN/m² on it.
    let loaded = report(
        &format!(
            "{BEAM}box(\"board\", (300, 100, 50), at = (600, 0, 2200), material = \"C24\", tags = [\"construction\", \"floor\"])\n"
        ),
        &[],
    );
    let beam = member(&loaded, "beam");
    let live = 2000.0 * 0.3 * 0.1;
    close(beam.loads_n["imposed"], live);
    let near = (2950.0 - 750.0) / 2900.0;
    for reaction in &beam.reactions {
        let share = if reaction.part == "post a" {
            near
        } else {
            1.0 - near
        };
        close(reaction.loads_n["imposed"], live * share);
        close(
            reaction.loads_n["permanent"],
            weight(0.06) / 2.0 + weight(0.0015) * share,
        );
    }
    // The posts stand on the floor and take their own weight besides.
    close(
        member(&loaded, "post a").loads_n["permanent"],
        weight(0.02) + weight(0.06) / 2.0 + weight(0.0015) * near,
    );
}

#[test]
fn a_deck_passes_what_stands_on_it_to_the_joist_underneath_at_that_place() {
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
material_weight(\"steel\", kg_m3 = 7850, source = \"test\")
weight_scope([\"heavy\"])
for y in (0, 1000):
    box(\"post a %d\" % y, (100, 80, 1000), at = (0, y, 0), material = \"C24\", grounded = True)
    box(\"post b %d\" % y, (100, 80, 1000), at = (2900, y, 0), material = \"C24\", grounded = True)
    box(\"joist %d\" % y, (3000, 80, 200), at = (0, y, 1000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (3000, 1080, 20), at = (0, 0, 1200), material = \"OSB\", tags = [\"deck\"])
box(\"weight\", (100, 80, 100), at = (200, 0, 1220), material = \"steel\", tags = [\"heavy\"])\n",
        &[],
    );
    let weight = 0.0008 * 7850.0 * 9.81;
    let near = member(&report, "joist 0");
    close(near.loads_n["permanent"], weight);
    let patch = near
        .patches
        .iter()
        .find(|patch| patch.source == "deck")
        .expect("the deck's patch");
    close(patch.from_mm, 200.0);
    close(patch.to_mm, 300.0);
    let far = member(&report, "joist 1000");
    assert_eq!(far.loads_n.get("permanent").copied().unwrap_or(0.0), 0.0);
}

#[test]
fn a_deck_shares_its_area_load_by_the_strip_each_joist_is_nearest_to() {
    // Joists 80 mm wide at y = 0 (edge), 500 and 1520 (edge) under a 3000 x 1600
    // deck with 2 kN/m²: the strips end halfway between them, at 290 and 1050.
    let report = report(
        "load_path(only = [\"frame\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"floor\"], source = \"test\")
for y in (0, 500, 1520):
    box(\"post a %d\" % y, (100, 80, 1000), at = (0, y, 0), material = \"C24\", grounded = True)
    box(\"post b %d\" % y, (100, 80, 1000), at = (2900, y, 0), material = \"C24\", grounded = True)
    box(\"joist %d\" % y, (3000, 80, 200), at = (0, y, 1000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (3000, 1600, 20), at = (0, 0, 1200), material = \"OSB\", tags = [\"floor\"])\n",
        &[],
    );
    for (joist, strip_mm) in [
        ("joist 0", 290.0),
        ("joist 500", 760.0),
        ("joist 1520", 550.0),
    ] {
        let expected = 2000.0 * 3.0 * strip_mm / 1000.0;
        let actual = member(&report, joist).loads_n["imposed"];
        assert!(
            (actual - expected).abs() <= expected * 0.03,
            "{joist}: {actual} N, expected {expected} N"
        );
    }
}

#[test]
fn a_beam_over_three_posts_is_continuous_and_its_middle_post_takes_five_quarters() {
    // 6000 mm beam, post centres 50, 3000 and 5950 mm, 1.2 N/mm from the deck.
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
for x in (0, 2950, 5900):
    box(\"post %d\" % x, (100, 100, 2000), at = (x, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (6000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (6000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])\n",
        &[],
    );
    let beam = member(&report, "beam");
    let reaction = |part: &str| {
        beam.reactions
            .iter()
            .find(|reaction| reaction.part == part)
            .unwrap_or_else(|| panic!("{:?}", beam.reactions))
            .loads_n["imposed"]
    };
    let middle = 1.25 * 1.2 * 2950.0;
    assert!(
        (reaction("post 2950") - middle).abs() <= middle * 0.01,
        "{}",
        reaction("post 2950")
    );
    close(
        reaction("post 0") + reaction("post 2950") + reaction("post 5900"),
        1.2 * 6000.0,
    );
}

#[test]
fn a_load_on_an_overhang_lifts_the_far_end_off_its_post_and_says_so() {
    // Posts at 50 and 3000 mm, a steel block centred 4500 mm along the beam.
    let report = report(
        "load_path(only = [\"frame\"])
material_weight(\"steel\", kg_m3 = 7850, source = \"test\")
weight_scope([\"heavy\"])
box(\"post a\", (100, 100, 2000), material = \"C24\", tags = [\"frame\"])
box(\"post b\", (100, 100, 2000), at = (2950, 0, 0), material = \"C24\", tags = [\"frame\"])
box(\"beam\", (4600, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"block\", (100, 100, 100), at = (4450, 0, 2200), material = \"steel\", tags = [\"heavy\"])\n",
        &[],
    );
    // Held at both posts, post a would have to pull down by 1500 / 2950 of
    // the block; resting, the beam lets go of it and tips over post b.
    let weight = 0.001 * 7850.0 * 9.81;
    let beam = member(&report, "beam");
    let reaction = |part: &str| {
        beam.reactions
            .iter()
            .find(|reaction| reaction.part == part)
            .unwrap_or_else(|| panic!("{:?}", beam.reactions))
    };
    assert!(reaction("post a").lifted_off);
    assert!(!reaction("post b").lifted_off);
    close(reaction("post b").loads_n["permanent"], weight);
    assert_eq!(
        beam.missing,
        [
            "it lifts off post a and tips over its last support (EQU 0.9 G + 1.5 Q): no joint holds it down"
        ]
    );
    assert_eq!(
        member(&report, "post a")
            .loads_n
            .get("permanent")
            .copied()
            .unwrap_or(0.0),
        0.0
    );
    // Screwed to post a, the beam stays on it: the screw pulls it down.
    let held = report_with_screw();
    let beam = member(&held, "beam");
    let screw = beam
        .reactions
        .iter()
        .find(|reaction| reaction.part == "post a")
        .unwrap();
    assert!(!screw.lifted_off);
    close(screw.loads_n["permanent"], -weight * 1500.0 / 2950.0);
    assert_eq!(beam.missing, Vec::<String>::new());
    // The pull is not passed down as relief: post a carries nothing of the block.
    assert_eq!(
        member(&held, "post a")
            .loads_n
            .get("permanent")
            .copied()
            .unwrap_or(0.0),
        0.0
    );
}

fn report_with_screw() -> Report {
    report(
        "load_path(only = [\"frame\"])
material_weight(\"steel\", kg_m3 = 7850, source = \"test\")
weight_scope([\"heavy\"])
a = box(\"post a\", (100, 100, 2000), material = \"C24\", tags = [\"frame\"])
box(\"post b\", (100, 100, 2000), at = (2950, 0, 0), material = \"C24\", tags = [\"frame\"])
b = box(\"beam\", (4600, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"block\", (100, 100, 100), at = (4450, 0, 2200), material = \"steel\", tags = [\"heavy\"])
joint(b, a, kind = \"screw\", fastener = \"screw 8 x 200\", fasteners = [(50, 50, 2100)])\n",
        &[],
    )
}

#[test]
fn unknown_weights_and_loads_are_listed_with_the_members_they_reach() {
    let source = format!(
        "{BEAM}box(\"crate\", (300, 100, 50), at = (600, 0, 2200), material = \"mystery\", tags = [\"construction\"])
area_load(\"snow\", kind = \"snow\", kn_m2 = snow_load(0, 40), on = [\"floor\"])
box(\"board\", (300, 100, 50), at = (1600, 0, 2200), material = \"C24\", tags = [\"construction\", \"floor\"])\n"
    );
    let report = report(&source, &[]);
    let expected = [
        "snow (snow) has no value".to_owned(),
        "the weight of material \"mystery\" is not known".to_owned(),
    ];
    assert_eq!(member(&report, "beam").missing, expected);
    assert_eq!(member(&report, "post a").missing, expected);
}

#[test]
fn a_bearing_joint_carries_the_reaction_of_the_part_hanging_on_it() {
    let report = report(
        "load_path(only = [\"frame\"])
self_weight([\"frame\"])
h = box(\"header\", (120, 2000, 240), material = \"C24\", grounded = True, tags = [\"frame\"])
j = box(\"joist\", (3000, 60, 200), at = (120, 500, 40), material = \"C24\", tags = [\"frame\"])
box(\"wall\", (100, 2000, 240), at = (3020, 0, -200), material = \"C24\", grounded = True, tags = [\"frame\"])
arunda(j, h, \"50 B\")\n",
        &[],
    );
    let [check] = report.joints.as_slice() else {
        panic!("{:?}", report.joints);
    };
    // The dovetail at 0 mm and the wall at 2950 mm share the span between them;
    // the last 50 mm rest on the wall alone.
    let half = 3.0 * 0.06 * 0.2 * 420.0 * 9.81 * 2950.0 / 3000.0 / 2.0;
    close(check.load_n.as_ref().expect("load")["permanent"], half);
    assert_eq!(check.missing, Vec::<String>::new());
    assert_eq!(check.status, "not_verified");
}

#[test]
fn every_member_of_the_house_carries_its_loads_once_the_site_snow_is_given() {
    let without = report(HOUSE, &[]);
    let with = report(HOUSE, &[("snow_sk", 1.0)]);
    assert!(
        with.loads.members.len() > 200,
        "{}",
        with.loads.members.len()
    );
    for member in &with.loads.members {
        assert_eq!(member.missing, Vec::<String>::new(), "{}", member.part);
        assert!(
            member
                .loads_n
                .get("permanent")
                .is_some_and(|load| *load > 0.0),
            "{}",
            member.part
        );
    }
    // Without the site's snow every rafter says what it lacks.
    for member in without
        .loads
        .members
        .iter()
        .filter(|member| member.part.contains("/krokva "))
    {
        assert_eq!(
            member.missing,
            ["sneh (snow) has no value"],
            "{}",
            member.part
        );
    }
    // The hangers get their loads; only their rating is missing.
    assert!(with.joints.len() >= 20);
    for check in &with.joints {
        assert!(check.load_n.is_some(), "{}", check.joint);
        assert_eq!(check.missing, [NO_RATING], "{}", check.joint);
    }
    // A rafter carries snow on its plan area: mu1(40°) = 0.8 * 20 / 30.
    let rafter = member(&with, "konštrukcia/strecha južná/krokva 3");
    let plan = rafter.loads_n["roof"] / 400.0;
    close(rafter.loads_n["snow"], plan * 0.8 * 20.0 / 30.0 * 1000.0);
}
