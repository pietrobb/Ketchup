//! Declared bracing passes the rules a brace the geometry finds passes (review
//! 2026-11-10 R1110-PG-400, PG-401), and a named `length_axis` does not turn a
//! beam into a short column (PG-402).

use ketchup_program::member_check::MemberCheck;
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("bypass.star", source, &BTreeMap::new())
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

fn ignored(report: &Report) -> Vec<&str> {
    report
        .issues
        .iter()
        .filter(|issue| issue.kind == "declared_bracing_ignored")
        .map(|issue| issue.message.as_str())
        .collect()
}

/// A GL24h 80 x 400 beam (z 2000 - 2400) on two grounded posts over 5.8 m under
/// about 13 kN at its middle, with nothing holding its top sideways.
const BEAM: &str = "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 560, on = [\"pad\"], source = \"test\")
timber_design({\"GL24h\": \"GL24h\"})
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (100, 80, 2000), at = (5800, 0, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (5900, 80, 400), at = (0, 0, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad\", (300, 80, 50), at = (2800, 0, 2400), material = \"GL24h\", tags = [\"pad\"])
";

const PLACES: [i32; 6] = [300, 1200, 2100, 3600, 4500, 5400];

/// PG-400: six grounded blocks 50 x 80 x 50 screwed on the beam's top and
/// named in brace() held it (pass); without brace() they hold nothing. A
/// grounded brace holds only where it reaches across the member beyond its
/// side, as the anchorage of any other brace.
#[test]
fn a_grounded_brace_on_the_member_itself_holds_nothing() {
    let mut blocks = BEAM.to_owned();
    for x in PLACES {
        blocks.push_str(&format!(
            "box(\"block {x}\", (50, 80, 50), at = ({x}, 0, 2400), material = \"GL24h\", grounded = True, tags = [\"block\"])
joint(\"block {x}\", \"beam\", kind = \"screw\")
"
        ));
    }
    blocks.push_str("brace([\"beam\"], by = [\"block\"], edge = \"top\")\n");
    let blocks = report(&blocks);
    assert_ne!(checked(&blocks, "beam").status, "pass");
    assert!(
        ignored(&blocks)
            .iter()
            .any(|message| message.contains("block 300")),
        "{:?}",
        ignored(&blocks)
    );

    // The same blocks a millimetre wider than the beam either side still rest
    // on it, held up by nothing but the beam.
    let mut wider = BEAM.to_owned();
    for x in PLACES {
        wider.push_str(&format!(
            "box(\"block {x}\", (50, 82, 50), at = ({x}, -1, 2400), material = \"GL24h\", grounded = True, tags = [\"block\"])
joint(\"block {x}\", \"beam\", kind = \"screw\")
"
        ));
    }
    wider.push_str("brace([\"beam\"], by = [\"block\"], edge = \"top\")\n");
    let wider = report(&wider);
    assert_ne!(checked(&wider, "beam").status, "pass");
    assert!(
        ignored(&wider)
            .iter()
            .any(|message| message.contains("block 300")),
        "{:?}",
        ignored(&wider)
    );

    // Two grounded laths along its top, held every 10 mm: 562 points.
    let mut laths = BEAM.to_owned();
    laths.push_str(
        "box(\"lath a\", (2750, 80, 50), at = (0, 0, 2400), material = \"GL24h\", grounded = True, tags = [\"lath\"])
box(\"lath b\", (2750, 80, 50), at = (3150, 0, 2400), material = \"GL24h\", grounded = True, tags = [\"lath\"])
joint(\"lath a\", \"beam\", kind = \"screw\")
joint(\"lath b\", \"beam\", kind = \"screw\")
brace([\"beam\"], by = [\"lath\"], edge = \"top\", spacing = 10)
",
    );
    let laths = report(&laths);
    assert_ne!(checked(&laths, "beam").status, "pass");

    // Grounded boards across its top, reaching a metre past its side, hold it.
    let mut across = BEAM.to_owned();
    for x in PLACES {
        across.push_str(&format!(
            "box(\"board {x}\", (50, 1000, 50), at = ({x}, 0, 2400), material = \"GL24h\", grounded = True, tags = [\"board\"])
joint(\"board {x}\", \"beam\", kind = \"screw\")
"
        ));
    }
    across.push_str("brace([\"beam\"], by = [\"board\"], edge = \"top\")\n");
    let across = report(&across);
    assert_eq!(ignored(&across), Vec::<&str>::new());
    assert_eq!(checked(&across, "beam").status, "pass");
}

/// Two 80 mm strips on the beam's top, screwed to it every 300 mm, either side
/// of the load, with `extra`.
fn strips(extra: &str) -> String {
    let screws = |from: i32, to: i32| {
        (from..=to)
            .step_by(300)
            .map(|x| format!("({x}, 40, 2400)"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{BEAM}box(\"strip a\", (2750, 80, 22), at = (0, 0, 2400), material = \"GL24h\", tags = [\"deck\"])
box(\"strip b\", (2750, 80, 22), at = (3150, 0, 2400), material = \"GL24h\", tags = [\"deck\"])
joint(\"strip a\", \"beam\", kind = \"screw\", fasteners = [{}])
joint(\"strip b\", \"beam\", kind = \"screw\", fasteners = [{}])
{extra}",
        screws(150, 2700),
        screws(3300, 5700)
    )
}

/// PG-401: the beam a sheet of its own diaphragm, anchored to its own posts,
/// passed; so did strips anchored to blocks in line with the beam's ends,
/// which PG-303 refuses for a brace. A diaphragm holds a member only where it
/// is anchored across the member beyond its side, and never a sheet of its own.
#[test]
fn a_diaphragm_anchored_in_line_with_the_member_or_on_its_own_posts_holds_nothing() {
    let own_posts = report(&strips(
        "joint(\"beam\", \"post a\", kind = \"screw\")
joint(\"beam\", \"post b\", kind = \"screw\")
diaphragm([\"deck\", \"beam\"], anchored_to = [\"post a\", \"post b\"])
",
    ));
    assert_ne!(checked(&own_posts, "beam").status, "pass");
    assert!(!ignored(&own_posts).is_empty());

    let in_line = report(&strips(
        "box(\"block a\", (100, 80, 50), at = (-100, 0, 2400), material = \"GL24h\", grounded = True)
box(\"block b\", (100, 80, 50), at = (5900, 0, 2400), material = \"GL24h\", grounded = True)
joint(\"strip a\", \"block a\", kind = \"screw\")
joint(\"strip b\", \"block b\", kind = \"screw\")
diaphragm([\"deck\"], anchored_to = [\"block a\", \"block b\"])
",
    ));
    assert_ne!(checked(&in_line, "beam").status, "pass");
    assert!(
        ignored(&in_line)
            .iter()
            .any(|message| message.contains("beam")),
        "{:?}",
        ignored(&in_line)
    );
}

/// [`BEAM`] beside a second beam on its own posts, a deck in two boards on
/// both, tied over the load by a third, screwed to the beams, with `extra`.
fn deck(extra: &str) -> String {
    let screws = |from: i32, to: i32| {
        (from..=to)
            .step_by(300)
            .map(|x| format!("({x}, 40, 2400)"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{BEAM}box(\"post c\", (100, 80, 2000), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
box(\"post d\", (100, 80, 2000), at = (5800, 1000, 0), material = \"GL24h\", grounded = True)
box(\"beam 2\", (5900, 80, 400), at = (0, 1000, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"deck a\", (2750, 1080, 22), at = (0, 0, 2400), material = \"GL24h\", tags = [\"deck\"])
box(\"deck b\", (2850, 1080, 22), at = (3150, 0, 2400), material = \"GL24h\", tags = [\"deck\"])
box(\"deck tie\", (700, 880, 22), at = (2600, 200, 2422), material = \"GL24h\", tags = [\"deck\"])
joint(\"deck tie\", \"deck a\", kind = \"screw\")
joint(\"deck tie\", \"deck b\", kind = \"screw\")
joint(\"deck a\", \"beam\", kind = \"screw\", fasteners = [{}])
joint(\"deck b\", \"beam\", kind = \"screw\", fasteners = [{}])
joint(\"deck a\", \"beam 2\", kind = \"screw\")
joint(\"deck b\", \"beam 2\", kind = \"screw\")
{extra}",
        screws(150, 2700),
        screws(3300, 5700)
    )
}

/// PG-401: a deck anchored to a grounded 10 mm nub on it, right over the beam,
/// passed. A pin that small lets the deck turn about it, and it holds the
/// beam from within its own width; a wall along the deck's end holds both.
#[test]
fn a_diaphragm_pinned_at_one_small_point_holds_nothing() {
    let nub = report(&deck(
        "box(\"nub\", (10, 10, 10), at = (4000, 35, 2422), material = \"GL24h\", grounded = True)
joint(\"deck b\", \"nub\", kind = \"screw\")
diaphragm([\"deck\"], anchored_to = [\"nub\"])
",
    ));
    assert_ne!(checked(&nub, "beam").status, "pass");
    // The nub lies within the beam's width, and clear of the unloaded second
    // beam's, but the deck turns about it.
    let messages = ignored(&nub);
    for (beam, why) in [
        ("of beam.", "within its width"),
        ("of beam 2.", "turns about"),
    ] {
        assert!(
            messages
                .iter()
                .any(|message| message.ends_with(beam) && message.contains(why)),
            "{beam}: {messages:?}"
        );
    }

    let wall = report(&deck(
        "box(\"end wall\", (100, 1080, 2600), at = (6000, 0, 0), material = \"GL24h\", grounded = True)
joint(\"deck b\", \"end wall\", kind = \"screw\")
diaphragm([\"deck\"], anchored_to = [\"end wall\"])
",
    ));
    assert_eq!(ignored(&wall), Vec::<&str>::new());
    assert_eq!(checked(&wall, "beam").status, "pass");
}

/// A GL24h 3 900 x 80 x 400 beam on two grounded posts under 36 kN at its
/// middle, its `length_axis` named `axis` or not.
fn short_beam(axis: Option<&str>) -> String {
    let attributes = axis
        .map(|axis| format!(", attributes = {{\"length_axis\": \"{axis}\"}}"))
        .unwrap_or_default();
    format!(
        "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 1500, on = [\"pad\"], source = \"test\")
timber_design({{\"GL24h\": \"GL24h\"}})
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (100, 80, 2000), at = (3800, 0, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (3900, 80, 400), at = (0, 0, 2000), material = \"GL24h\", tags = [\"frame\"]{attributes})
box(\"pad\", (300, 80, 50), at = (1800, 0, 2400), material = \"GL24h\", tags = [\"pad\"])
box(\"wall\", (3900, 100, 2400), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
"
    )
}

/// PG-402: named along its 400 mm height (3 900 / 400 = 9.75, under the 10x
/// doubt), the beam failing in bending at 1.444 became a 400 mm column at
/// 0.011 and passed. Its bearings lie apart across the named axis, so it is
/// not what the name says and is not verified.
#[test]
fn a_named_length_axis_its_bearings_lie_across_is_not_trusted() {
    let plain = report(&short_beam(None));
    assert_eq!(checked(&plain, "beam").status, "fail");

    let named = report(&short_beam(Some("z")));
    let beam = checked(&named, "beam");
    assert_eq!(beam.status, "not_verified", "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("length_axis")),
        "{:?}",
        beam.missing
    );

    let along = report(&short_beam(Some("x")));
    assert_eq!(checked(&along, "beam").status, "fail");
}
