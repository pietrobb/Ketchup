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
/// screws gave the hanger one in MN).
#[test]
fn a_hanger_bent_to_a_column_does_not_buckle() {
    let held = beam_with(
        |x| {
            format!(
                "box(\"hanger {x}\", (50, 100, 600), at = ({x}, 80, 2300), material = \"GL24h\", tags = [\"hanger\"])
box(\"column {x}\", (200, 200, 2900), at = ({}, 180, 0), material = \"GL24h\", grounded = True)
joint(\"hanger {x}\", \"beam\", kind = \"screw\")
joint(\"hanger {x}\", \"column {x}\", kind = \"screw\", fasteners = [({}, 180, 2850), ({}, 180, 2750)])
",
                x - 75,
                x + 25,
                x + 25
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
/// member.
#[test]
fn a_brace_and_its_anchor_end_to_end_buckle_as_one_rod() {
    let two = beam_with(
        |x| {
            format!(
                "box(\"brace {x}\", (20, 1200, 20), at = ({x}, 80, 2380), material = \"GL24h\", tags = [\"brace\"])
box(\"anchor {x}\", (20, 1200, 20), at = ({x}, 1280, 2380), material = \"GL24h\", grounded = True)
joint(\"brace {x}\", \"beam\", kind = \"screw\")
joint(\"brace {x}\", \"anchor {x}\", kind = \"screw\")
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
