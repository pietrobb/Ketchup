//! A brace holds a member through its joints as the joints hold: one turning
//! about a single fastener passes on only the force along the brace, paths to
//! the ground sharing a part are one path, and a brace buckles over its whole
//! length to the ground (review 2026-11-20 R1120ac-PG-900 to PG-903).

use ketchup_program::member_check::MemberCheck;
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("brace.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

fn checked<'a>(report: &'a Report, part: &str) -> &'a MemberCheck {
    report
        .design
        .members
        .iter()
        .find(|member| member.part == part)
        .unwrap_or_else(|| panic!("{part} is not checked: {:?}", report.design.members))
}

/// Why `report` holds its beam nowhere a declaration says: the declarations it
/// ignores and the beam's own check.
fn refused(report: &Report) -> Vec<&str> {
    let mut why: Vec<&str> = report
        .issues
        .iter()
        .filter(|issue| issue.kind == "declared_bracing_ignored")
        .map(|issue| issue.message.as_str())
        .collect();
    why.extend(checked(report, "beam").missing.iter().map(String::as_str));
    why
}

/// A GL24h 80 x 400 beam (y 0 - 80, z 2000 - 2400) on two grounded posts over
/// 5.8 m under about 13 kN at its middle, with nothing holding its top sideways.
const BEAM: &str = "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 560, on = [\"pad\"], source = \"test\")
timber_design({\"GL24h\": \"GL24h\"})
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (100, 80, 2000), at = (5800, 0, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (5900, 80, 400), at = (0, 0, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad\", (300, 80, 50), at = (2800, 0, 2400), material = \"GL24h\", tags = [\"pad\"])
";

const PLACES: [i32; 6] = [300, 1200, 2100, 3600, 4500, 5400];

/// [`BEAM`] with `each(x)` at every place and `rest` once.
fn beam_with(each: impl Fn(i32) -> String, rest: &str) -> Report {
    let mut source = format!("{BEAM}{rest}");
    for x in PLACES {
        source.push_str(&each(x));
    }
    report(&source)
}

/// R1120ac-PG-900: an upright hanger 50 x 100 x 600 screwed to the beam's side
/// at its top edge, its upper end held by one screw to a grounded block on a
/// grounded post 200 x 200, passed the beam (C = 143.5 N/mm against 56): the
/// joint the hanger turns about was taken for one fixing it. A part between two
/// joints it turns about carries only the force along it, and an upright one
/// none across the beam: a pendulum.
#[test]
fn a_hanger_on_one_screw_is_a_pendulum() {
    let hanger = |top: &str| {
        beam_with(
            |x| {
                format!(
                    "box(\"hanger {x}\", (50, 100, 600), at = ({x}, 80, 2300), material = \"GL24h\", tags = [\"hanger\"])
box(\"block {x}\", (50, 400, 100), at = ({x}, 80, 2900), material = \"GL24h\", grounded = True)
box(\"column {x}\", (200, 200, 2900), at = ({}, 190, 0), material = \"GL24h\", grounded = True)
joint(\"hanger {x}\", \"beam\", kind = \"screw\")
joint(\"hanger {x}\", \"block {x}\", kind = \"screw\"{})
",
                    x - 75,
                    top.replace("{x}", &(x + 25).to_string())
                )
            },
            "brace([\"beam\"], by = [\"hanger\"], edge = \"top\")\n",
        )
    };
    let pendulum = hanger(", fasteners = [({x}, 130, 2900)]");
    assert_ne!(checked(&pendulum, "beam").status, "pass");
    assert!(
        refused(&pendulum)
            .iter()
            .any(|message| message.contains("hanger")),
        "{:?}",
        refused(&pendulum)
    );
}

/// Review 2026-11-20 ac P3: a hanger screwed twice to the side of a grounded
/// column bends, and so does the column; nothing on the path pushes along its
/// axis, so it has no buckling force (the slanting run from the beam to the
/// screws gave the hanger one in MN). The column stands beside the hanger
/// along the beam: the moment the force across the beam puts on the screws
/// turns about the normal of their contact, against which they slip
/// (R1121ac-PG-1001).
#[test]
fn a_hanger_bent_to_a_column_does_not_buckle() {
    let held = beam_with(
        |x| {
            format!(
                "box(\"hanger {x}\", (50, 100, 600), at = ({x}, 80, 2300), material = \"GL24h\", tags = [\"hanger\"])
box(\"column {x}\", (200, 200, 2900), at = ({}, 100, 0), material = \"GL24h\", grounded = True)
joint(\"hanger {x}\", \"beam\", kind = \"screw\")
joint(\"hanger {x}\", \"column {x}\", kind = \"screw\", fasteners = [({}, 140, 2850), ({}, 140, 2750)])
",
                x + 50,
                x + 50,
                x + 50
            )
        },
        "brace([\"beam\"], by = [\"hanger\"], edge = \"top\")\n",
    );
    let braces: Vec<_> = held
        .loads
        .members
        .iter()
        .flat_map(|load| &load.braced_by)
        .filter(|brace| brace.part.starts_with("hanger"))
        .collect();
    assert_eq!(braces.len(), PLACES.len(), "{braces:?}");
    for brace in braces {
        let anchorage = brace.anchorage.as_ref().expect("a path to the ground");
        assert!(anchorage.stiffness_n_mm > 0.0, "{brace:?}");
        assert_eq!(anchorage.capacity_n, None, "{brace:?}");
    }
}

/// R1120ac-PG-901: a deck on the beam anchored to grounded blocks on one
/// grounded lath standing on a single post passed with enough blocks (eight
/// gave C = 109 N/mm on a post of about 30 alone): anchorages sharing a part on
/// their way to the ground were added up. Paths sharing a part hold as the
/// stiffest of them; the same blocks each on a post of its own hold side by
/// side.
#[test]
fn anchors_on_one_post_hold_as_one() {
    const BLOCKS: [i32; 4] = [1025, 1225, 1425, 1625];
    let deck = |shared: bool| {
        let mut source = format!(
            "{BEAM}box(\"deck a\", (2750, 1100, 20), at = (0, 0, 2400), material = \"GL24h\", tags = [\"deck a\"])
box(\"deck b\", (2750, 1100, 20), at = (3150, 0, 2400), material = \"GL24h\", tags = [\"deck b\"])
joint(\"deck a\", \"beam\", kind = \"screw\")
joint(\"deck b\", \"beam\", kind = \"screw\")
box(\"wall b\", (2750, 400, 2420), at = (3150, 1100, 0), material = \"GL24h\", grounded = True)
joint(\"deck b\", \"wall b\", kind = \"screw\")
diaphragm([\"deck b\"], anchored_to = [\"wall b\"])
"
        );
        if shared {
            source.push_str(
                "box(\"lath\", (800, 400, 400), at = (975, 1100, 1600), material = \"GL24h\", grounded = True)
box(\"post\", (90, 90, 1600), at = (1330, 1255, 0), material = \"GL24h\", grounded = True)
",
            );
        }
        for x in BLOCKS {
            source.push_str(&format!(
                "box(\"block {x}\", (100, 400, 420), at = ({x}, 1100, 2000), material = \"GL24h\", grounded = True)
joint(\"deck a\", \"block {x}\", kind = \"screw\")
"
            ));
            if !shared {
                source.push_str(&format!(
                    "box(\"post {x}\", (120, 120, 2000), at = ({}, 1240, 0), material = \"GL24h\", grounded = True)\n",
                    x - 10
                ));
            }
        }
        let names: Vec<String> = BLOCKS.iter().map(|x| format!("\"block {x}\"")).collect();
        source.push_str(&format!(
            "diaphragm([\"deck a\"], anchored_to = [{}])\n",
            names.join(", ")
        ));
        report(&source)
    };
    let shared = deck(true);
    let beam = checked(&shared, "beam");
    assert_ne!(beam.status, "pass", "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("deck a")
                && reason.contains("too flexible or too slender")),
        "{:?}",
        beam.missing
    );
    let apart = deck(false);
    let beam = checked(&apart, "beam");
    assert_eq!(beam.missing, Vec::<String>::new(), "{beam:?}");
    assert_eq!(beam.status, "pass");
}
/// R1120ac-PG-902: a brace 20 x 1 200 x 20 screwed end to end to a grounded
/// anchor 20 x 1 200 x 20 touching a wall held the beam (877 N), while one rod
/// 20 x 2 400 x 20 buckles at 227 N: the brace's length was forgotten where it
/// met the anchor. A path buckles over the whole length it runs from the
/// member. The anchor is screwed to the wall, as bare it only pushes
/// (R1122ac-PG-1101).
#[test]
fn a_brace_and_its_anchor_end_to_end_buckle_as_one_rod() {
    let two = beam_with(
        |x| {
            format!(
                "box(\"brace {x}\", (20, 1200, 20), at = ({x}, 80, 2380), material = \"GL24h\", tags = [\"brace\"])
box(\"anchor {x}\", (20, 1200, 20), at = ({x}, 1280, 2380), material = \"GL24h\", grounded = True)
joint(\"brace {x}\", \"beam\", kind = \"screw\")
joint(\"brace {x}\", \"anchor {x}\", kind = \"screw\")
joint(\"anchor {x}\", \"wall\", kind = \"screw\")
"
            )
        },
        "box(\"wall\", (5900, 100, 2400), at = (0, 2480, 0), material = \"GL24h\", grounded = True)
brace([\"beam\"], by = [\"brace\"], edge = \"top\")\n",
    );
    let one = beam_with(
        |x| {
            format!(
                "box(\"brace {x}\", (20, 2400, 20), at = ({x}, 80, 2380), material = \"GL24h\", grounded = True, tags = [\"brace\"])
joint(\"brace {x}\", \"beam\", kind = \"screw\")
joint(\"brace {x}\", \"wall\", kind = \"screw\")
"
            )
        },
        "box(\"wall\", (5900, 100, 2400), at = (0, 2480, 0), material = \"GL24h\", grounded = True)
brace([\"beam\"], by = [\"brace\"], edge = \"top\")\n",
    );
    for (case, held) in [("brace and anchor", two), ("one rod", one)] {
        assert_ne!(checked(&held, "beam").status, "pass", "{case}");
        assert!(
            refused(&held)
                .iter()
                .any(|message| message.contains("too slender")),
            "{case}: {:?}",
            refused(&held)
        );
    }
}

/// Review 2026-11-21 R1121ac-PG-1002: a brace 15 x 1 200 x 15 screwed end to
/// end to a grounded anchor 30 x 1 200 x 30 held the beam at 277.6 N against
/// 263 asked and passed: the brace buckled only over its own length, where the
/// stepped rod pinned at both ends buckles at about 112 N (FE). Every part of
/// a path buckles over the whole length of the path: the brace over 2 400 mm,
/// Euler's 69.4 N, k_c f_c,0,k A 67.9 N (R1122ac, review §4 P3), on the safe
/// side.
#[test]
fn a_stepped_path_buckles_over_its_whole_length() {
    let held = beam_with(
        |x| {
            format!(
                "box(\"brace {x}\", (15, 1200, 15), at = ({x}, 80, 2385), material = \"GL24h\", tags = [\"brace\"])
box(\"anchor {x}\", (30, 1200, 30), at = ({}, 1280, 2370), material = \"GL24h\", grounded = True)
joint(\"brace {x}\", \"beam\", kind = \"screw\")
joint(\"brace {x}\", \"anchor {x}\", kind = \"screw\")
joint(\"anchor {x}\", \"wall\", kind = \"screw\")
",
                x - 5
            )
        },
        "box(\"wall\", (5900, 100, 2400), at = (0, 2480, 0), material = \"GL24h\", grounded = True)
brace([\"beam\"], by = [\"brace\"], edge = \"top\")\n",
    );
    let braces: Vec<_> = held
        .loads
        .members
        .iter()
        .flat_map(|load| &load.braced_by)
        .filter(|brace| brace.part.starts_with("brace"))
        .collect();
    assert_eq!(braces.len(), PLACES.len(), "{braces:?}");
    for brace in braces {
        let anchorage = brace.anchorage.as_ref().expect("a path to the ground");
        let capacity = anchorage.capacity_n.expect("the path buckles");
        assert!((capacity - 67.9).abs() < 0.1, "{brace:?}");
        assert_eq!(
            anchorage.buckling_in.as_deref(),
            Some(brace.part.as_str()),
            "{brace:?}"
        );
    }
    assert_ne!(checked(&held, "beam").status, "pass");
    assert!(
        refused(&held)
            .iter()
            .any(|message| message.contains("too slender")),
        "{:?}",
        refused(&held)
    );
}

/// The stiffness each brace tagged `prefix` holds the beam with, N/mm.
fn stiffnesses(report: &Report, prefix: &str) -> Vec<f64> {
    report
        .loads
        .members
        .iter()
        .flat_map(|load| &load.braced_by)
        .filter(|brace| brace.part.starts_with(prefix))
        .map(|brace| {
            brace
                .anchorage
                .as_ref()
                .map_or(0.0, |anchorage| anchorage.stiffness_n_mm)
        })
        .collect()
}

/// Review 2026-11-21 R1121ac-PG-1000: a rod 40 x 40 x 1 000 lying on the beam
/// at 40° in plan, one screw at each end, its far end on a grounded column 20
/// (x) x 400 (y) x 2 400, held the beam at 124.8 N/mm and passed: the column
/// was asked how it holds a force across the beam (its strong way), though the
/// rod, turning on its screws, pushes it along the rod, with sin 40° of it the
/// column's weak way. Bending 3 E I / L³ = 0.666 N/mm that way, the column
/// holds the rod's end 0.623 mm/N along it, and the rod the beam
/// cos² 40° / 0.624 = 0.94 N/mm, against about 56 asked.
#[test]
fn a_strut_pushes_its_anchor_along_itself() {
    let (sin, cos) = 40.0_f64.to_radians().sin_cos();
    let held = beam_with(
        |x| {
            let end = |along: f64| (f64::from(x) - along * sin, 40.0 + along * cos);
            let (column_x, column_y) = end(960.0);
            let (screw_x, screw_y) = end(950.0);
            format!(
                "strut = box(\"strut {x}\", (40, 1000, 40), at = ({}, 0, 2400), material = \"GL24h\", tags = [\"strut\"])
rotate(strut, axis = (0, 0, 1), angle = 40, pivot = ({x}, 40, 2420))
box(\"column {x}\", (20, 400, 2400), at = ({}, {}, 0), material = \"GL24h\", grounded = True)
joint(\"strut {x}\", \"beam\", kind = \"screw\", fasteners = [({x}, 40, 2400)])
joint(\"strut {x}\", \"column {x}\", kind = \"screw\", fasteners = [({screw_x}, {screw_y}, 2400)])
",
                x - 20,
                column_x - 10.0,
                column_y - 200.0,
            )
        },
        "brace([\"beam\"], by = [\"strut\"], edge = \"top\")\n",
    );
    let found = stiffnesses(&held, "strut");
    assert_eq!(found.len(), PLACES.len(), "{found:?}");
    for stiffness in found {
        assert!((0.85..=1.05).contains(&stiffness), "{stiffness}");
    }
    assert_ne!(checked(&held, "beam").status, "pass");
}

/// Review 2026-11-22 R1122ac-PG-1100: a plate 300 x 1 000 x 40 lying on the
/// beam along y, one screw at each end, the beam's at (x - 135, 40) and the
/// far one at (x + 5, 950) on a grounded column 20 (x) x 400 (y) x 2 400, held
/// the beam at 214.9 N/mm and passed: the force was taken along the plate's
/// long axis, though a part turning about a screw at each end pushes along the
/// line through its screws, d = (0.152, 0.988). The column gives along d
/// 0.0231 x 1.503 (its weak way) + 0.977 / 266 (its strong way) + slip, about
/// 0.040 mm/N, and the plate holds the beam about 0.977 / 0.040 = 24 N/mm,
/// against about 56 asked. With both screws on the plate's axis it holds the
/// column's strong way, about 211.
#[test]
fn a_strut_pushes_along_the_line_through_its_screws() {
    let plate = |beam_screw: i32| {
        beam_with(
            |x| {
                format!(
                    "box(\"plate {x}\", (300, 1000, 40), at = ({}, 0, 2400), material = \"GL24h\", tags = [\"plate\"])
box(\"column {x}\", (20, 400, 2400), at = ({}, 600, 0), material = \"GL24h\", grounded = True)
joint(\"plate {x}\", \"beam\", kind = \"screw\", fasteners = [({}, 40, 2400)])
joint(\"plate {x}\", \"column {x}\", kind = \"screw\", fasteners = [({}, 950, 2400)])
",
                    x - 150,
                    x - 5,
                    x + beam_screw,
                    x + 5
                )
            },
            "brace([\"beam\"], by = [\"plate\"], edge = \"top\")\n",
        )
    };
    let slanting = plate(-135);
    let found = stiffnesses(&slanting, "plate");
    assert_eq!(found.len(), PLACES.len(), "{found:?}");
    for stiffness in found {
        assert!((18.0..=30.0).contains(&stiffness), "{stiffness}");
    }
    assert_ne!(checked(&slanting, "beam").status, "pass");
    let straight = plate(5);
    for stiffness in stiffnesses(&straight, "plate") {
        assert!((180.0..=230.0).contains(&stiffness), "{stiffness}");
    }
}

/// R1121ac-PG-1000: a brace 100 x 100 x 1 000 at 30° in plan, screwed twice at
/// each end, fixed on a grounded head 400 x 400 x 400 standing on a column
/// 20 x 400 x 2 000 under where it meets the brace, held the beam at about
/// 100 N/mm: the head was asked how it holds a force where it meets the brace,
/// and the brace's 425 mm across the beam's length, a lever twisting the column,
/// was lost. Twisting 425² L / (G J) = 0.49 mm/N (G J = 7.4e8 N mm²), the
/// column lets the beam give about 2 N/mm.
#[test]
fn a_fixed_brace_twists_its_anchor_by_its_length() {
    let (sin, cos) = 30.0_f64.to_radians().sin_cos();
    let held = beam_with(
        |x| {
            let at = |along: f64| (f64::from(x) - along * sin, 40.0 + along * cos);
            let (head_x, head_y) = at(850.0);
            let (near_x, near_y) = at(700.0);
            let (far_x, far_y) = at(900.0);
            format!(
                "rod = box(\"brace {x}\", (100, 1000, 100), at = ({}, 0, 2400), material = \"GL24h\", tags = [\"brace\"])
rotate(rod, axis = (0, 0, 1), angle = 30, pivot = ({x}, 40, 2450))
box(\"head {x}\", (400, 400, 400), at = ({}, {}, 2000), material = \"GL24h\", grounded = True)
box(\"column {x}\", (20, 400, 2000), at = ({}, {}, 0), material = \"GL24h\", grounded = True)
joint(\"brace {x}\", \"beam\", kind = \"screw\", fasteners = [({}, 14, 2400), ({}, 66, 2400)])
joint(\"brace {x}\", \"head {x}\", kind = \"screw\", fasteners = [({near_x}, {near_y}, 2400), ({far_x}, {far_y}, 2400)])
",
                x - 50,
                head_x - 200.0,
                head_y - 200.0,
                head_x - 10.0,
                head_y - 200.0,
                f64::from(x) + 15.0,
                f64::from(x) - 15.0,
            )
        },
        "brace([\"beam\"], by = [\"brace\"], edge = \"top\")\n",
    );
    // Each brace holds the beam at both its screws.
    let found = stiffnesses(&held, "brace");
    assert_eq!(found.len(), 2 * PLACES.len(), "{found:?}");
    for stiffness in found {
        assert!((1.0..=3.0).contains(&stiffness), "{stiffness}");
    }
    assert_ne!(checked(&held, "beam").status, "pass");
}

/// R1121ac-PG-1000, a diaphragm's way: a deck 700 x 600 x 22 screwed to the
/// beam, its far end screwed to one grounded plate 300 x 300 x 100 on a column
/// 20 (x) x 400 (y) x 2 300 standing 500 mm along the beam from the screws,
/// was asked how the plate holds a force where the deck meets it: the column's
/// strong way. The deck, fixed to its single anchor, turns the column by its
/// 500 mm: twisting 500² L / (G J) ≈ 0.8 mm/N, the column lets the beam give
/// about 1.2 N/mm. Anchors apart from each other would hold the turn as a
/// couple instead.
#[test]
fn a_deck_on_one_anchor_twists_it_by_its_length() {
    let held = beam_with(
        |x| {
            format!(
                "box(\"deck {x}\", (750, 600, 22), at = ({}, 0, 2400), material = \"GL24h\", tags = [\"deck {x}\"])
box(\"plate {x}\", (400, 400, 100), at = ({}, 200, 2300), material = \"GL24h\", grounded = True)
box(\"column {x}\", (20, 400, 2300), at = ({}, 200, 0), material = \"GL24h\", grounded = True)
joint(\"deck {x}\", \"beam\", kind = \"screw\", fasteners = [({x}, 20, 2400), ({x}, 60, 2400)])
joint(\"deck {x}\", \"plate {x}\", kind = \"screw\", fasteners = [({}, 210, 2400), ({}, 590, 2400)])
diaphragm([\"deck {x}\"], anchored_to = [\"plate {x}\"])
",
                x - 50,
                x + 300,
                x + 490,
                x + 310,
                x + 690,
            )
        },
        "",
    );
    let found = stiffnesses(&held, "deck");
    assert!(!found.is_empty(), "{:?}", refused(&held));
    for stiffness in found {
        assert!((0.6..=2.0).contains(&stiffness), "{stiffness}");
    }
    assert_ne!(checked(&held, "beam").status, "pass");
}

/// R1121ac-PG-1000, a diaphragm on two anchors on the same side (capability
/// review 977): a deck 800 x 600 x 22 screwed to the beam and to two grounded
/// plates 200 mm long, 400 and 620 mm along the beam, each on a column 20 (x)
/// x 2 300 as wide as its plate. Plates 200 x 200 from 120 mm across the beam,
/// too far along it to rest the deck on as decking, were each asked for a force
/// where the deck meets them, their strong ways added up: 30.5 + 27.9 = 58.4 N/mm
/// against 56.2 asked. The plates 220 mm apart hold the deck's turn as a couple
/// about 2.3 times the force on the beam, and the columns twist (G J / L about
/// 1.4e5 N mm per radian): a rigid deck on the two lets the beam give about
/// 3.6 N/mm. Plates 400 wide from 200 mm across hold it also as decking resting
/// on one, the column twisted by its 400 mm, about 2 N/mm.
#[test]
fn a_deck_on_two_anchors_on_one_side_holds_its_turn_by_their_spacing() {
    // The plates from `y` across the beam, `wide` across it.
    let on_plates = |y: i32, wide: i32| {
        beam_with(
            |x| {
                format!(
                    "box(\"deck {x}\", (800, 600, 22), at = ({}, 0, 2400), material = \"GL24h\", tags = [\"deck {x}\"])
box(\"plate {x}\", (200, {wide}, 100), at = ({}, {y}, 2300), material = \"GL24h\", grounded = True)
box(\"plate2 {x}\", (200, {wide}, 100), at = ({}, {y}, 2300), material = \"GL24h\", grounded = True)
box(\"column {x}\", (20, {wide}, 2300), at = ({}, {y}, 0), material = \"GL24h\", grounded = True)
box(\"column2 {x}\", (20, {wide}, 2300), at = ({}, {y}, 0), material = \"GL24h\", grounded = True)
joint(\"deck {x}\", \"beam\", kind = \"screw\", fasteners = [({x}, 20, 2400), ({x}, 60, 2400)])
joint(\"deck {x}\", \"plate {x}\", kind = \"screw\", fasteners = [({}, {}, 2400), ({}, {}, 2400)])
joint(\"deck {x}\", \"plate2 {x}\", kind = \"screw\", fasteners = [({}, {}, 2400), ({}, {}, 2400)])
diaphragm([\"deck {x}\"], anchored_to = [\"plate {x}\", \"plate2 {x}\"])
",
                    x - 50,
                    x + 300,
                    x + 520,
                    x + 390,
                    x + 610,
                    x + 310,
                    y + 10,
                    x + 490,
                    y + wide - 10,
                    x + 530,
                    y + 10,
                    x + 710,
                    y + wide - 10,
                )
            },
            "",
        )
    };
    for (y, wide, expected) in [(120, 200, 2.5..=5.0), (200, 400, 1.5..=3.0)] {
        let held = on_plates(y, wide);
        let found = stiffnesses(&held, "deck");
        assert!(!found.is_empty(), "{:?}", refused(&held));
        for stiffness in found {
            assert!(
                expected.contains(&stiffness),
                "plates at y {y}: {stiffness}"
            );
        }
        assert_ne!(checked(&held, "beam").status, "pass", "plates at y {y}");
    }
}

/// R1121ac-PG-1001: an upright hanger 300 x 100 x 600 on the beam's side, its
/// top on a grounded block on a column 200 x 200, the joint naming no
/// screws, was fixed there and held the beam at 85.2 N/mm: the two screws
/// taken at the ends of the contact run mostly along the axis of the moment,
/// which lies in the contact and would pull them out of it. The joint is a
/// hinge, and the upright hanger a pendulum across the force.
#[test]
fn screws_along_the_axis_of_the_moment_make_a_hinge() {
    let held = beam_with(
        |x| {
            format!(
                "box(\"hanger {x}\", (300, 100, 600), at = ({}, 80, 2300), material = \"GL24h\", tags = [\"hanger\"])
box(\"block {x}\", (300, 400, 100), at = ({}, 80, 2900), material = \"GL24h\", grounded = True)
box(\"column {x}\", (200, 200, 2900), at = ({}, 230, 0), material = \"GL24h\", grounded = True)
joint(\"hanger {x}\", \"beam\", kind = \"screw\")
joint(\"hanger {x}\", \"block {x}\", kind = \"screw\")
",
                x - 150,
                x - 150,
                x - 100
            )
        },
        "brace([\"beam\"], by = [\"hanger\"], edge = \"top\")\n",
    );
    for stiffness in stiffnesses(&held, "hanger") {
        assert!(stiffness < 1.0, "{stiffness}");
    }
    assert_ne!(checked(&held, "beam").status, "pass");
}

/// Review 2026-11-21 ac P3: a hanger 50 x 100 x 600 screwed to the side of a
/// grounded block 200 x 400 x 200 on a column bent the block, which was given
/// Euler's load over the path as F_cap, some 29 000 kN (4 669 kN in the
/// review). A piece that stocky crushes long before it buckles (EN 1995-1-1
/// 6.3.2(3), relative slenderness at most 0.3): it holds up to its crushing
/// load f_c,0,k A, 24 x 200 x 200 = 960 kN (R1122ac, review §4 P3).
#[test]
fn a_stocky_block_on_the_path_does_not_buckle() {
    let held = beam_with(
        |x| {
            format!(
                "box(\"hanger {x}\", (50, 100, 600), at = ({x}, 80, 2300), material = \"GL24h\", tags = [\"hanger\"])
box(\"block {x}\", (200, 400, 200), at = ({}, 80, 2700), material = \"GL24h\", grounded = True)
box(\"column {x}\", (200, 200, 2700), at = ({}, 180, 0), material = \"GL24h\", grounded = True)
joint(\"hanger {x}\", \"beam\", kind = \"screw\")
joint(\"hanger {x}\", \"block {x}\", kind = \"screw\")
",
                x + 50,
                x + 50
            )
        },
        "brace([\"beam\"], by = [\"hanger\"], edge = \"top\")\n",
    );
    let braces: Vec<_> = held
        .loads
        .members
        .iter()
        .flat_map(|load| &load.braced_by)
        .filter(|brace| brace.part.starts_with("hanger"))
        .collect();
    assert_eq!(braces.len(), PLACES.len(), "{braces:?}");
    for brace in braces {
        let anchorage = brace.anchorage.as_ref().expect("a path to the ground");
        assert!(anchorage.stiffness_n_mm > 0.0, "{brace:?}");
        assert_eq!(anchorage.capacity_n, Some(960_000.0), "{brace:?}");
        let block = brace.part.replace("hanger", "block");
        assert_eq!(
            anchorage.buckling_in.as_deref(),
            Some(block.as_str()),
            "{brace:?}"
        );
    }
}

/// Review 2026-11-22 R1122ac-PG-1101: a brace 100 x 960 x 100 fixed by two
/// screws at each end to the beam and to the top of a grounded column 400 (x)
/// x 20 (y) x 2 400, whose head bears without a fastener on a grounded block
/// 200 x 400 x 200 on a post 200 x 200, held the beam at 247 N/mm and passed:
/// two grounded parts touching were taken as one, though a bare contact only
/// pushes. Pulled away from the block the column holds alone, its weak way
/// 3 E I / L³ about 0.6 N/mm, as with no block. Screwed to the block, the
/// column pulls it as well.
#[test]
fn a_bare_contact_between_grounded_parts_only_pushes() {
    fn block(x: i32) -> String {
        format!(
            "box(\"block {x}\", (200, 400, 200), at = ({}, 970, 2200), material = \"GL24h\", grounded = True)
box(\"post {x}\", (200, 200, 2200), at = ({}, 1170, 0), material = \"GL24h\", grounded = True)
",
            x - 100,
            x - 100
        )
    }
    let braced = |head: fn(i32) -> String| {
        beam_with(
            |x| {
                format!(
                    "box(\"brace {x}\", (100, 960, 100), at = ({}, 0, 2400), material = \"GL24h\", tags = [\"brace\"])
box(\"column {x}\", (400, 20, 2400), at = ({}, 950, 0), material = \"GL24h\", grounded = True)
joint(\"brace {x}\", \"beam\", kind = \"screw\", fasteners = [({}, 14, 2400), ({}, 66, 2400)])
joint(\"brace {x}\", \"column {x}\", kind = \"screw\", fasteners = [({}, 952, 2400), ({}, 958, 2400)])
{}",
                    x - 50,
                    x - 200,
                    x + 15,
                    x - 15,
                    x - 30,
                    x + 30,
                    head(x)
                )
            },
            "brace([\"beam\"], by = [\"brace\"], edge = \"top\")\n",
        )
    };
    let alone = stiffnesses(&braced(|_| String::new()), "brace");
    assert_eq!(alone.len(), 2 * PLACES.len(), "{alone:?}");
    for stiffness in &alone {
        assert!((0.3..=1.0).contains(stiffness), "{stiffness}");
    }
    let bearing = braced(block);
    assert_eq!(stiffnesses(&bearing, "brace"), alone);
    assert_ne!(checked(&bearing, "beam").status, "pass");
    assert!(
        bearing
            .design
            .basis
            .iter()
            .any(|line| line.contains("holding no pull unless a joint() fastens them")),
        "{:?}",
        bearing.design.basis
    );
    let screwed = braced(|x| {
        format!(
            "{}joint(\"column {x}\", \"block {x}\", kind = \"screw\", fasteners = [({}, 970, 2300), ({}, 970, 2300)])\n",
            block(x),
            x - 50,
            x + 50
        )
    });
    for stiffness in stiffnesses(&screwed, "brace") {
        assert!(stiffness > 10.0, "{stiffness}");
    }
}
