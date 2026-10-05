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
