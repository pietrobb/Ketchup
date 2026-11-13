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
    let cases: [(&str, Beside); 9] = [
        ("nub on its end", |x| {
            format!(
                "box(\"nub {x}\", (1, 1, 1), at = ({}, 200, 2400), material = \"GL24h\", grounded = True)\n",
                x + 50
            )
        }),
        ("column on the floor at its corner edge", |x| {
            format!(
                "box(\"col {x}\", (400, 400, 2450), at = ({}, 250, 0), material = \"GL24h\", grounded = True)\n",
                x + 50
            )
        }),
        ("column on the floor at its corner edge, turned 1°", |x| {
            format!(
                "box(\"col {x}\", (400, 400, 2450), at = ({0}, 250, 0), material = \"GL24h\", grounded = True)
rotate(\"col {x}\", axis = (0, 0, 1), angle = 1, pivot = ({0}, 250, 0))\n",
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
        ("grounded strip 1 x 4 on the floor", |x| {
            format!(
                "box(\"strip {x}\", (1, 4, 2400), at = ({}, 200, 0), material = \"GL24h\", grounded = True)\n",
                x + 20
            )
        }),
        ("grounded strip 5 x 20 on the floor", |x| {
            format!(
                "box(\"strip {x}\", (5, 20, 2400), at = ({}, 200, 0), material = \"GL24h\", grounded = True)\n",
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
        if let Some(part) = ["rod", "strip"]
            .into_iter()
            .find(|part| case.starts_with(&format!("grounded {part}")))
        {
            assert!(
                ignored(&held)
                    .iter()
                    .any(|message| message.contains(&format!("{part} 300"))
                        && message.contains("too slender")),
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

    // So does each post turned 1° about its middle, its top still flat
    // against the block's bottom (capability review 768).
    let turned = blocks(|x| {
        format!(
            "box(\"under {x}\", (50, 100, 2400), at = ({x}, 150, 0), material = \"GL24h\", grounded = True)
rotate(\"under {x}\", axis = (0, 0, 1), angle = 1, pivot = ({}, 200, 0))\n",
            x + 25
        )
    });
    assert_eq!(ignored(&turned), Vec::<&str>::new());
    assert_eq!(checked(&turned, "beam").status, "pass");

    // So does a grounded wall the blocks are screwed to, through a free board
    // between them as deep as the beam, screwed to both.
    let walled = |to_wall: bool| {
        blocks(|x| {
            let wall = if x == PLACES[0] { WALL } else { "" };
            let screwed = if to_wall {
                format!("joint(\"board {x}\", \"wall\", kind = \"screw\")\n")
            } else {
                String::new()
            };
            format!(
                "{wall}box(\"board {x}\", (50, 750, 50), at = ({x}, 250, 2400), material = \"GL24h\")
joint(\"board {x}\", \"block {x}\", kind = \"screw\")
{screwed}"
            )
        })
    };
    let screwed = walled(true);
    assert_eq!(ignored(&screwed), Vec::<&str>::new());
    assert_eq!(checked(&screwed, "beam").status, "pass");
    // A free board only butting the wall holds nothing sideways (capability
    // review 771).
    let butting = walled(false);
    assert_ne!(checked(&butting, "beam").status, "pass");
    assert!(
        ignored(&butting)
            .iter()
            .any(|message| message.contains("block 300")),
        "{:?}",
        ignored(&butting)
    );
}

/// Capability review 771 (PG-163): a free stud under each block, joined to
/// nothing, holds nothing sideways whether it stands on the bare floor or on a
/// grounded slab lying on it. A grounded stud on the slab holds the beam.
#[test]
fn a_free_stud_is_no_ground_on_the_floor_or_on_a_grounded_slab() {
    const SLAB: &str = "box(\"slab\", (5900, 400, 100), at = (0, 100, 0), material = \"GL24h\", grounded = True)\n";
    let stud = |grounded: &str, slab: bool| {
        blocks(|x| {
            let (base, z, height) = if slab {
                (if x == PLACES[0] { SLAB } else { "" }, 100, 2300)
            } else {
                ("", 0, 2400)
            };
            format!(
                "{base}box(\"under {x}\", (50, 100, {height}), at = ({x}, 150, {z}), material = \"GL24h\"{grounded})\n"
            )
        })
    };
    for (case, slab) in [("on the floor", false), ("on a grounded slab", true)] {
        let held = stud("", slab);
        assert_ne!(checked(&held, "beam").status, "pass", "{case}");
        assert!(
            ignored(&held)
                .iter()
                .any(|message| message.contains("block 300")),
            "{case}: {:?}",
            ignored(&held)
        );
    }
    let grounded = stud(", grounded = True", true);
    assert_eq!(ignored(&grounded), Vec::<&str>::new());
    assert_eq!(checked(&grounded, "beam").status, "pass");
}

/// Capability review 766: a grounded strip 1 x 40 x 2 400 standing on the floor
/// under each block was ground, its height measured against its width and not
/// its thickness. Whatever its width, a part standing on its end more than 60
/// times as tall as it is thick holds nothing sideways; one no more than that
/// holds the beam.
#[test]
fn a_standing_part_is_slender_by_its_height_over_its_thickness() {
    let standing = |(dx, dy): (i32, i32)| {
        blocks(|x| {
            format!(
                "box(\"strip {x}\", ({dx}, {dy}, 2400), at = ({}, 200, 0), material = \"GL24h\", grounded = True)\n",
                x + 20
            )
        })
    };
    for section in [
        (1, 40),
        (40, 1),
        (1, 59),
        (1, 61),
        (10, 39),
        (10, 40),
        (10, 100),
        (39, 100),
    ] {
        let held = standing(section);
        assert_ne!(checked(&held, "beam").status, "pass", "{section:?}");
        assert!(
            ignored(&held)
                .iter()
                .any(|message| message.contains("strip 300") && message.contains("too slender")),
            "{section:?}: {:?}",
            ignored(&held)
        );
    }
    for section in [(40, 100), (100, 40)] {
        let held = standing(section);
        assert_eq!(ignored(&held), Vec::<&str>::new(), "{section:?}");
        assert_eq!(checked(&held, "beam").status, "pass", "{section:?}");
    }
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
