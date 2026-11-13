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

    // Grounded boards across its top, reaching a metre past its side onto a
    // wall standing on the floor, hold it.
    let mut across = BEAM.to_owned();
    across.push_str(WALL);
    for x in PLACES {
        across.push_str(&format!(
            "box(\"board {x}\", (50, 1100, 50), at = ({x}, 0, 2400), material = \"GL24h\", grounded = True, tags = [\"board\"])
joint(\"board {x}\", \"beam\", kind = \"screw\")
"
        ));
    }
    across.push_str("brace([\"beam\"], by = [\"board\"], edge = \"top\")\n");
    let across = report(&across);
    assert_eq!(ignored(&across), Vec::<&str>::new());
    assert_eq!(checked(&across, "beam").status, "pass");
}

/// A grounded wall beside [`BEAM`], a metre off it, standing on the floor up to
/// the beam's top.
const WALL: &str =
    "box(\"wall\", (5900, 100, 2400), at = (0, 1000, 0), material = \"GL24h\", grounded = True)\n";

/// Six grounded blocks of `size` at y = `y` screwed on the beam's top and
/// named in brace(), with `extra`.
fn blocks(size: [i32; 3], y: i32, extra: &str) -> Report {
    let [dx, dy, dz] = size;
    let mut source = format!("{BEAM}{extra}");
    for x in PLACES {
        source.push_str(&format!(
            "box(\"block {x}\", ({dx}, {dy}, {dz}), at = ({x}, {y}, 2400), material = \"GL24h\", grounded = True, tags = [\"block\"])
joint(\"block {x}\", \"beam\", kind = \"screw\")
"
        ));
    }
    source.push_str("brace([\"beam\"], by = [\"block\"], edge = \"top\")\n");
    report(&source)
}

/// R1112-PG-500: grounded blocks resting on the beam held it when twice its
/// width or wider (160 mm, 420 mm) or set off to one side (25 mm on it, 25 mm
/// beyond), and laths set 40 mm aside: nothing but the beam holds them up. A
/// grounded part anchors only where it stands on something besides the
/// member: the floor, or a grounded part not resting on the member.
#[test]
fn a_grounded_brace_held_up_by_the_member_alone_holds_nothing_however_wide() {
    for (size, y) in [
        ([50, 160, 50], -40),
        ([50, 420, 50], -170),
        ([50, 50, 50], 55),
    ] {
        let wide = blocks(size, y, "");
        assert_ne!(checked(&wide, "beam").status, "pass", "{size:?} at {y}");
        assert!(
            ignored(&wide)
                .iter()
                .any(|message| message.contains("block 300")),
            "{size:?} at {y}: {:?}",
            ignored(&wide)
        );
    }

    let laths = report(&format!(
        "{BEAM}box(\"lath a\", (2750, 80, 50), at = (0, 40, 2400), material = \"GL24h\", grounded = True, tags = [\"lath\"])
box(\"lath b\", (2750, 80, 50), at = (3150, 40, 2400), material = \"GL24h\", grounded = True, tags = [\"lath\"])
joint(\"lath a\", \"beam\", kind = \"screw\")
joint(\"lath b\", \"beam\", kind = \"screw\")
brace([\"beam\"], by = [\"lath\"], edge = \"top\", spacing = 10)
"
    ));
    assert_ne!(checked(&laths, "beam").status, "pass");

    // A part hanging under each 420 mm block beside the beam, reaching lower
    // but resting on nothing, holds it up no more than the beam does
    // (capability review 751 of R1112-PG-500).
    let mut hangers = String::new();
    for x in PLACES {
        hangers.push_str(&format!(
            "box(\"hanger {x}\", (50, 50, 300), at = ({x}, 150, 2100), material = \"GL24h\")\n"
        ));
    }
    let hung = blocks([50, 420, 50], -170, &hangers);
    assert_ne!(checked(&hung, "beam").status, "pass");
    assert!(
        ignored(&hung)
            .iter()
            .any(|message| message.contains("block 300")),
        "{:?}",
        ignored(&hung)
    );

    // A grounded cap on each 420 mm block beside the beam, or two stacked, is
    // held up by the block, which rests on the beam: it carries the block no
    // more than the beam does (capability review 758 of R1112-PG-500).
    for stack in [1, 2] {
        let mut caps = String::new();
        for x in PLACES {
            for level in 0..stack {
                caps.push_str(&format!(
                    "box(\"cap {x} {level}\", (50, 50, 50), at = ({x}, 150, {}), material = \"GL24h\", grounded = True)\n",
                    2450 + 50 * level
                ));
            }
        }
        let capped = blocks([50, 420, 50], -170, &caps);
        assert_ne!(checked(&capped, "beam").status, "pass", "{stack} caps");
        assert!(
            ignored(&capped)
                .iter()
                .any(|message| message.contains("block 300")),
            "{stack} caps: {:?}",
            ignored(&capped)
        );
    }

    // The same 420 mm blocks a grounded post stands under, on the floor
    // beside the beam, hold it.
    let mut posts = String::new();
    for x in PLACES {
        posts.push_str(&format!(
            "box(\"under {x}\", (50, 100, 2400), at = ({x}, 150, 0), material = \"GL24h\", grounded = True)\n"
        ));
    }
    let held = blocks([50, 420, 50], -170, &posts);
    assert_eq!(ignored(&held), Vec::<&str>::new());
    assert_eq!(checked(&held, "beam").status, "pass");
}

/// Deck strips 600 mm wide on [`BEAM`], screwed to it every 300 mm, butting
/// two grounded boards 400 x `across` x 22 across its top in the gaps between
/// them, which anchor the diaphragm; with `extra`.
fn gapped_deck(across: i32, extra: &str) -> Report {
    let screws = |from: i32, to: i32| {
        (from..=to)
            .step_by(300)
            .map(|x| format!("({x}, 40, 2400)"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut source = format!("{BEAM}{extra}");
    for (name, from, to) in [
        ("a", 0, 1200),
        ("b", 1600, 2750),
        ("c", 3150, 4300),
        ("d", 4700, 5900),
    ] {
        source.push_str(&format!(
            "box(\"strip {name}\", ({}, 600, 22), at = ({from}, -260, 2400), material = \"GL24h\", tags = [\"deck\"])
joint(\"strip {name}\", \"beam\", kind = \"screw\", fasteners = [{}])
",
            to - from,
            screws(from + 150, to - 150)
        ));
    }
    for (name, x) in [("plate 1", 1200), ("plate 2", 4300)] {
        source.push_str(&format!(
            "box(\"{name}\", (400, {across}, 22), at = ({x}, -460, 2400), material = \"GL24h\", grounded = True, tags = [\"plate\"])
joint(\"{name}\", \"beam\", kind = \"screw\")
"
        ));
    }
    source.push_str(
        "joint(\"strip a\", \"plate 1\", kind = \"screw\")
joint(\"strip b\", \"plate 1\", kind = \"screw\")
joint(\"strip c\", \"plate 2\", kind = \"screw\")
joint(\"strip d\", \"plate 2\", kind = \"screw\")
diaphragm([\"deck\"], anchored_to = [\"plate\"])
",
    );
    report(&source)
}

/// R1112-PG-501: a diaphragm anchored to grounded boards lying across the
/// beam's top, held up by the beam and the diaphragm's own strips, passed
/// without a warning. Anchored to the same boards reaching onto a wall on the
/// floor, it holds.
#[test]
fn a_diaphragm_anchored_to_boards_on_the_member_holds_nothing() {
    let on_beam = gapped_deck(1000, "");
    assert_ne!(checked(&on_beam, "beam").status, "pass");
    assert!(
        ignored(&on_beam)
            .iter()
            .any(|message| message.ends_with("of beam.") && message.contains("plate")),
        "{:?}",
        ignored(&on_beam)
    );

    // A part hanging under each board beside the beam, resting on nothing,
    // holds them up no more than the beam does (capability review 751).
    let hung = gapped_deck(
        1000,
        "box(\"hanger 1\", (50, 50, 300), at = (1300, 200, 2100), material = \"GL24h\")
box(\"hanger 2\", (50, 50, 300), at = (4400, 200, 2100), material = \"GL24h\")
",
    );
    assert_ne!(checked(&hung, "beam").status, "pass");
    assert!(
        ignored(&hung)
            .iter()
            .any(|message| message.ends_with("of beam.") && message.contains("plate")),
        "{:?}",
        ignored(&hung)
    );

    let on_wall = gapped_deck(1560, WALL);
    assert_eq!(ignored(&on_wall), Vec::<&str>::new());
    assert_eq!(checked(&on_wall, "beam").status, "pass");
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

/// R1112-PG-502: the beam of [`short_beam`] on one grounded U-shaped support
/// instead of two posts (uprights x 0 - 100 and 3 800 - 3 900, joined at the
/// floor) bore on one place, the middle of the support's box, and named along
/// its height passed at 0.011. The places it bears on are where it touches.
#[test]
fn a_named_length_axis_on_one_support_touching_it_far_apart_is_not_trusted() {
    let u_support = |axis: &str| {
        format!(
            "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 1500, on = [\"pad\"], source = \"test\")
timber_design({{\"GL24h\": \"GL24h\"}})
rotate(extrude(\"support\", profile = [(0, 0), (3900, 0), (3900, 2000), (3800, 2000), (3800, 100), (100, 100), (100, 2000), (0, 2000)], distance = 80, material = \"GL24h\", grounded = True), axis = (1, 0, 0), angle = 90)
box(\"beam\", (3900, 80, 400), at = (0, -80, 2000), material = \"GL24h\", tags = [\"frame\"]{axis})
box(\"pad\", (300, 80, 50), at = (1800, -80, 2400), material = \"GL24h\", tags = [\"pad\"])
"
        )
    };
    let plain = report(&u_support(""));
    assert_eq!(checked(&plain, "beam").status, "fail");

    let named = report(&u_support(", attributes = {\"length_axis\": \"z\"}"));
    let beam = checked(&named, "beam");
    assert_eq!(beam.status, "not_verified", "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("length_axis")),
        "{:?}",
        beam.missing
    );
}
