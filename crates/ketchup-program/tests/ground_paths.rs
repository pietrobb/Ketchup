//! An anchorage holds a member sideways only from the ground: a path of
//! contacts to a grounded part standing on the floor that passes through no
//! load-path member, every part on it at least as deep as the member (review
//! 2026-11-13 R1113-PG-600, PG-601, PG-602).

use ketchup_program::member_check::MemberCheck;
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("ground.star", source, &BTreeMap::new())
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
/// about 13 kN at its middle.
const BEAM: &str = "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 560, on = [\"pad\"], source = \"test\")
timber_design({\"GL24h\": \"GL24h\"})
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (100, 80, 2000), at = (5800, 0, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (5900, 80, 400), at = (0, 0, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad\", (300, 80, 50), at = (2800, 0, 2400), material = \"GL24h\", tags = [\"pad\"])
";

const PLACES: [i32; 6] = [300, 1200, 2100, 3600, 4500, 5400];

/// Six grounded blocks 50 x 420 x 50 across the beam's top (y -170 - 250),
/// screwed to it and named in brace(), each with `extra(x)` beside it.
fn blocks(extra: impl Fn(i32) -> String) -> Report {
    let mut source = BEAM.to_owned();
    for x in PLACES {
        source.push_str(&format!(
            "box(\"block {x}\", (50, 420, 50), at = ({x}, -170, 2400), material = \"GL24h\", grounded = True, tags = [\"block\"])
joint(\"block {x}\", \"beam\", kind = \"screw\")
{}",
            extra(x)
        ));
    }
    source.push_str("brace([\"beam\"], by = [\"block\"], edge = \"top\")\n");
    report(&source)
}

/// R1113-PG-600: a small grounded part touching the block (a 1 mm nub on its
/// end, one meeting only its corner edge, a nub just under it), a free rod
/// 1 x 1 x 2 400 standing on the floor under it (grounded too: capability
/// review 761), or a free part beside it with a grounded nub, made the block
/// ground and the beam passed.
#[test]
fn a_small_or_free_part_touching_the_anchorage_is_no_ground() {
    type Beside = fn(i32) -> String;
    let cases: [(&str, Beside); 6] = [
        ("nub on its end", |x| {
            format!(
                "box(\"nub {x}\", (1, 1, 1), at = ({}, 200, 2400), material = \"GL24h\", grounded = True)\n",
                x + 50
            )
        }),
        ("part at its corner edge", |x| {
            format!(
                "box(\"corner {x}\", (50, 50, 50), at = ({}, 250, 2400), material = \"GL24h\", grounded = True)\n",
                x + 50
            )
        }),
        ("nub under it", |x| {
            format!(
                "box(\"nub {x}\", (1, 1, 1), at = ({x}, 200, 2399), material = \"GL24h\", grounded = True)\n"
            )
        }),
        ("free rod on the floor", |x| {
            format!(
                "box(\"rod {x}\", (1, 1, 2400), at = ({}, 200, 0), material = \"GL24h\")\n",
                x + 20
            )
        }),
        ("grounded rod on the floor", |x| {
            format!(
                "box(\"rod {x}\", (1, 1, 2400), at = ({}, 200, 0), material = \"GL24h\", grounded = True)\n",
                x + 20
            )
        }),
        ("free part with a grounded nub", |x| {
            format!(
                "box(\"free {x}\", (50, 50, 50), at = ({}, 150, 2400), material = \"GL24h\")
box(\"nub {x}\", (1, 1, 1), at = ({}, 175, 2400), material = \"GL24h\", grounded = True)\n",
                x + 50,
                x + 100
            )
        }),
    ];
    for (case, extra) in cases {
        let held = blocks(extra);
        assert_ne!(checked(&held, "beam").status, "pass", "{case}");
        assert!(
            ignored(&held)
                .iter()
                .any(|message| message.contains("block 300")),
            "{case}: {:?}",
            ignored(&held)
        );
        if case.starts_with("grounded rod") {
            assert!(
                ignored(&held)
                    .iter()
                    .any(|message| message.contains("rod 300") && message.contains("too slender")),
                "{:?}",
                ignored(&held)
            );
        }
    }

    // A grounded post standing on the floor under each block, beside the
    // beam, holds it.
    let posts = blocks(|x| {
        format!(
            "box(\"under {x}\", (50, 100, 2400), at = ({x}, 150, 0), material = \"GL24h\", grounded = True)\n"
        )
    });
    assert_eq!(ignored(&posts), Vec::<&str>::new());
    assert_eq!(checked(&posts, "beam").status, "pass");

    // So does a grounded wall the blocks are screwed to, through a free board
    // between them as deep as the beam.
    let walled = blocks(|x| {
        let wall = if x == PLACES[0] { WALL } else { "" };
        format!(
            "{wall}box(\"board {x}\", (50, 750, 50), at = ({x}, 250, 2400), material = \"GL24h\")
joint(\"board {x}\", \"block {x}\", kind = \"screw\")\n"
        )
    });
    assert_eq!(ignored(&walled), Vec::<&str>::new());
    assert_eq!(checked(&walled, "beam").status, "pass");
}

/// A grounded wall beside [`BEAM`], a metre off it, standing on the floor up to
/// the top of the blocks on the beam.
const WALL: &str =
    "box(\"wall\", (5900, 100, 2450), at = (0, 1000, 0), material = \"GL24h\", grounded = True)\n";

/// [`BEAM`] beside a second free beam 920 mm off it (y 1 000 - 1 080) on its
/// own posts, loaded alike, with `extra`.
fn two_beams(extra: &str) -> Report {
    report(&format!(
        "{BEAM}box(\"post c\", (100, 80, 2000), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
box(\"post d\", (100, 80, 2000), at = (5800, 1000, 0), material = \"GL24h\", grounded = True)
box(\"beam 2\", (5900, 80, 400), at = (0, 1000, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad 2\", (300, 80, 50), at = (2800, 1000, 2400), material = \"GL24h\", tags = [\"pad\"])
{extra}"
    ))
}

/// R1113-PG-601: the ground was reached through a second free beam and its
/// posts. Six grounded boards across two parallel beams, screwed to both and
/// named in brace() for both, passed both; laths on the first beam screwed to
/// grounded blocks on the second passed the first. Neither beam is ground for
/// the other.
#[test]
fn a_second_free_beam_is_no_ground() {
    let mut boards = String::new();
    for x in PLACES {
        boards.push_str(&format!(
            "box(\"board {x}\", (50, 1080, 50), at = ({x}, 0, 2400), material = \"GL24h\", grounded = True, tags = [\"board\"])
joint(\"board {x}\", \"beam\", kind = \"screw\")
joint(\"board {x}\", \"beam 2\", kind = \"screw\")
"
        ));
    }
    let across = two_beams(&format!(
        "{boards}brace([\"beam\", \"beam 2\"], by = [\"board\"], edge = \"top\")\n"
    ));
    for beam in ["beam", "beam 2"] {
        assert_ne!(checked(&across, beam).status, "pass", "{beam}");
    }

    let mut laths = String::new();
    for x in PLACES {
        laths.push_str(&format!(
            "box(\"block {x}\", (50, 80, 50), at = ({x}, 1000, 2400), material = \"GL24h\", grounded = True)
joint(\"block {x}\", \"beam 2\", kind = \"screw\")
box(\"lath {x}\", (50, 1000, 50), at = ({x}, 0, 2400), material = \"GL24h\", tags = [\"lath\"])
joint(\"lath {x}\", \"beam\", kind = \"screw\")
joint(\"lath {x}\", \"block {x}\", kind = \"screw\")
"
        ));
    }
    let tied = two_beams(&format!(
        "{laths}brace([\"beam\"], by = [\"lath\"], edge = \"top\")\n"
    ));
    assert_ne!(checked(&tied, "beam").status, "pass");
    assert!(
        ignored(&tied)
            .iter()
            .any(|message| message.contains("lath 300")),
        "{:?}",
        ignored(&tied)
    );
}

/// Deck strips 600 mm wide on [`BEAM`], screwed to it every 300 mm, butting
/// two grounded plates of `plate` at (x, `y`, `z`) in the gaps between them,
/// which anchor the diaphragm.
fn gapped_deck(plate: [i32; 3], y: i32, z: i32) -> Report {
    let screws = |from: i32, to: i32| {
        (from..=to)
            .step_by(300)
            .map(|x| format!("({x}, 40, 2400)"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let [dx, dy, dz] = plate;
    let mut source = BEAM.to_owned();
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
            "box(\"{name}\", ({dx}, {dy}, {dz}), at = ({x}, {y}, {z}), material = \"GL24h\", grounded = True, tags = [\"plate\"])\n"
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

/// R1113-PG-602: a grounded part touching nothing under it was still ground.
/// Two grounded plates 400 x 920 x 22 a millimetre over the beam, touching
/// only the deck's strips, anchored its diaphragm; so did plates 400 x 400 x 10
/// beside the beam, flush with the strips' bottom. A grounded part standing on
/// nothing modelled holds nothing, and the report says so.
#[test]
fn a_grounded_part_standing_on_nothing_is_no_ground() {
    for (plate, y, z) in [([400, 920, 22], -460, 2401), ([400, 400, 10], 100, 2400)] {
        let floating = gapped_deck(plate, y, z);
        assert_ne!(checked(&floating, "beam").status, "pass", "{plate:?}");
        assert!(
            ignored(&floating)
                .iter()
                .any(|message| message.contains("plate 1")
                    && message.contains("stands on nothing modelled")),
            "{plate:?}: {:?}",
            ignored(&floating)
        );
    }
}
