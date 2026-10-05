use ketchup_program::{run, validate};
use std::collections::BTreeMap;

/// Messages of the parts the load path reports as not carried.
fn not_carried(source: &str) -> Vec<String> {
    let model = run("load.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .0
        .model;
    validate(&model)
        .into_iter()
        .filter(|issue| issue.kind == "member_not_carried")
        .map(|issue| issue.parts[0].clone())
        .collect()
}

const POSTS: &str = "
load_path(only = [\"frame\"])
box(\"post a\", (100, 100, 2000), tags = [\"frame\"])
box(\"post b\", (100, 100, 2000), at = (2900, 0, 0), tags = [\"frame\"])
";

#[test]
fn a_beam_touching_posts_only_from_the_side_is_not_carried_until_hangers_carry_it() {
    let hanging = format!(
        "{POSTS}beam = box(\"beam\", (2800, 100, 200), at = (100, 0, 1800), tags = [\"frame\"])\n"
    );
    assert_eq!(not_carried(&hanging), ["beam"]);
    let hung = format!(
        "{hanging}joint(beam, \"post a\", kind = \"hanger\", bearing = True)
joint(beam, \"post b\", kind = \"hanger\", bearing = True)\n"
    );
    assert_eq!(not_carried(&hung), Vec::<String>::new());
    // A joint that only holds the beam in place carries nothing.
    let held = format!(
        "{hanging}joint(beam, \"post a\", kind = \"screw\")
joint(beam, \"post b\", kind = \"screw\")\n"
    );
    assert_eq!(not_carried(&held), ["beam"]);
}

#[test]
fn a_beam_must_rest_on_supports_on_both_sides_of_its_centre() {
    let both =
        format!("{POSTS}box(\"beam\", (3000, 100, 200), at = (0, 0, 2000), tags = [\"frame\"])\n");
    assert_eq!(not_carried(&both), Vec::<String>::new());
    // One end on a post, the other end on nothing: it would tip off.
    let one = format!(
        "{POSTS}box(\"beam\", (2000, 100, 200), at = (2500, 0, 2000), tags = [\"frame\"])\n"
    );
    assert_eq!(not_carried(&one), ["beam"]);
    // What rests on an uncarried member is not carried either.
    let stacked =
        format!("{one}box(\"block\", (100, 100, 100), at = (3500, 0, 2200), tags = [\"frame\"])\n");
    assert_eq!(not_carried(&stacked), ["beam", "block"]);
}

#[test]
fn only_members_carriers_and_the_ground_carry_members() {
    // Anything standing on the floor carries (a foundation); above it only
    // members and declared carriers do.
    let on_glass = "
load_path(only = [\"frame\"])
box(\"plate\", (1000, 100, 100), tags = [\"frame\"])
box(\"glass\", (1000, 100, 900), at = (0, 0, 100), material = \"glass\")
box(\"beam\", (1000, 100, 100), at = (0, 0, 1000), tags = [\"frame\"])
";
    assert_eq!(not_carried(on_glass), ["beam"]);
    let on_deck = on_glass
        .replace("material = \"glass\"", "tags = [\"deck\"]")
        .replace(
            "load_path(only = [\"frame\"])",
            "load_path(only = [\"frame\"], carriers = [\"deck\"])",
        );
    assert_eq!(not_carried(&on_deck), Vec::<String>::new());
    // Parts that are not members are never reported.
    let loose = "load_path(only = [\"frame\"])\nbox(\"lamp\", (100, 100, 100), at = (0, 0, 500))\n";
    assert_eq!(not_carried(loose), Vec::<String>::new());
}

#[test]
fn a_notched_member_rests_on_its_seats_although_its_side_contacts_are_larger() {
    // Seated 50 mm deep on each plate (50 x 80 mm²), with a lip down the plate's
    // inner side (80 x 80 mm²): the side contact is the larger of each pair.
    let seated = "
load_path(only = [\"frame\"])
box(\"plate a\", (500, 100, 100), at = (0, 0, 1000), grounded = True, tags = [\"frame\"])
box(\"plate b\", (500, 100, 100), at = (0, 2000, 1000), grounded = True, tags = [\"frame\"])
member = extrude(\"member\", profile = [[50, 1100], [100, 1100], [100, 1020], [150, 1020], [150, 1100],
                                        [1950, 1100], [1950, 1020], [2000, 1020], [2000, 1100], [2050, 1100],
                                        [2050, 1300], [50, 1300]],
                 distance = 80, tags = [\"frame\"])
place(member, origin = (200, 0, 0), z = (1, 0, 0), x = (0, 1, 0))
";
    assert_eq!(not_carried(seated), Vec::<String>::new());
}

#[test]
fn a_framed_wall_stacks_headers_on_the_studs_beside_the_openings_and_sills_on_cripples() {
    let wall = "
load_path(only = [\"frame\"])
buildup(\"wall\", (0, 0, 0), (1, 0, 0), (0, 0, 1), 4000,
        [{\"name\": \"frame\", \"thickness\": 140, \"spacing\": 625, \"tags\": [\"frame\"]}], height = 2600,
        openings = [(600, 0, 900, 2100), (2000, 800, 1200, 1300)])
";
    assert_eq!(not_carried(wall), Vec::<String>::new());
    let model = run("wall.star", wall, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .0
        .model;
    // The headers run over one 60 mm stud on each side.
    for (header, width) in [
        ("wall/frame/header 1", 1020.0),
        ("wall/frame/header 2", 1320.0),
    ] {
        let (min, max) = model.part(header).unwrap().world_bounds();
        assert!(
            (max[0] - min[0] - width).abs() < 1e-6,
            "{header}: {min:?} {max:?}"
        );
    }
}
