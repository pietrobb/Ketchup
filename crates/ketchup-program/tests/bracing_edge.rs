//! Which edge of a member a brace holds, in the member's own cross-section
//! (review 2026-11-08 PG-240, PG-141(b)).

use ketchup_program::loads::MemberLoad;
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("bracing.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

fn load<'a>(report: &'a Report, part: &str) -> &'a MemberLoad {
    report
        .loads
        .members
        .iter()
        .find(|load| load.part == part)
        .unwrap_or_else(|| panic!("no loads of {part}: {:?}", report.loads.members))
}

/// A GL24h 80 x 400 beam (z 2000 - 2400) on two grounded posts over 5.8 m under
/// about 13 kN at its middle, a grounded wall 1000 mm beside it, and 50 x 50
/// laths screwed to the beam's side and the wall, their middle `centre_z` high.
fn side_laths(centre_z: f64) -> String {
    let mut source = String::from(
        "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 560, on = [\"pad\"], source = \"test\")
timber_design({\"GL24h\": \"GL24h\"})
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (100, 80, 2000), at = (5800, 0, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (5900, 80, 400), at = (0, 0, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad\", (300, 80, 50), at = (2800, 0, 2400), material = \"GL24h\", tags = [\"pad\"])
box(\"wall\", (5900, 100, 2400), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
",
    );
    for x in [300, 1200, 2100, 3600, 4500, 5400] {
        source.push_str(&format!(
            "box(\"lath {x}\", (50, 920, 50), at = ({x}, 80, {}), material = \"GL24h\")
joint(\"lath {x}\", \"beam\", kind = \"screw\")
joint(\"lath {x}\", \"wall\", kind = \"screw\")
",
            centre_z - 25.0
        ));
    }
    source
}

/// PG-240: laths on the side of the beam 13 or 30 mm above its middle held
/// its compressed top, and the beam passed (rc 0). Bracing at mid-depth does
/// not stop it twisting; only one in the outer quarter of its depth does.
#[test]
fn laths_on_the_side_hold_the_top_only_in_its_outer_quarter() {
    for centre_z in [2213.0, 2230.0] {
        let report = report(&side_laths(centre_z));
        let beam = load(&report, "beam");
        assert!(
            beam.braced_by
                .iter()
                .all(|brace| !brace.part.starts_with("lath")),
            "{centre_z}: {:?}",
            beam.braced_by
        );
        assert_eq!(
            report.statics(),
            ketchup_program::Statics::NotVerified,
            "{centre_z}"
        );
    }
    let near_top = report(&side_laths(2375.0));
    let beam = load(&near_top, "beam");
    assert!(
        beam.braced_by
            .iter()
            .any(|brace| brace.part.starts_with("lath") && brace.edge == "top"),
        "{:?}",
        beam.braced_by
    );
    assert_eq!(near_top.statics(), ketchup_program::Statics::Verified);
}

/// Review 2026-11-08 §5.1: `brace()` declares the edge laths on the beam's side
/// hold, every 300 mm along where they cross it, where they touch it nearer
/// that edge than its middle; 13 mm above the middle it holds nothing, as
/// bracing there does not stop the beam twisting (R1110-PG-403). A declared
/// brace still has to be fastened to the member and anchored to the ground;
/// one that is not is reported and holds nothing.
#[test]
fn a_declared_brace_holds_the_edge_it_names_when_fastened_and_anchored() {
    let laths = (0..6)
        .map(|k| format!("\"lath {}\"", [300, 1200, 2100, 3600, 4500, 5400][k]))
        .collect::<Vec<_>>()
        .join(", ");
    let middle = report(&format!(
        "{}brace([\"beam\"], by = [{laths}], edge = \"top\")\n",
        side_laths(2213.0)
    ));
    assert!(
        load(&middle, "beam")
            .braced_by
            .iter()
            .all(|brace| !brace.part.starts_with("lath")),
        "{:?}",
        load(&middle, "beam").braced_by
    );
    assert_eq!(middle.statics(), ketchup_program::Statics::NotVerified);

    let declared = report(&format!(
        "{}brace([\"beam\"], by = [{laths}], edge = \"top\")\n",
        side_laths(2375.0)
    ));
    let beam = load(&declared, "beam");
    assert_eq!(
        beam.braced_by
            .iter()
            .filter(|brace| brace.part.starts_with("lath") && brace.edge == "top")
            .count(),
        6,
        "{:?}",
        beam.braced_by
    );
    assert_eq!(declared.statics(), ketchup_program::Statics::Verified);

    // The same laths unfastened from the wall anchor nothing.
    let loose = report(&format!(
        "{}brace([\"beam\"], by = [{laths}], edge = \"top\")\n",
        side_laths(2375.0)
            .replace("joint(\"lath", "#joint(\"lath")
            .replacen(
                "#joint(\"lath 300\", \"beam\"",
                "joint(\"lath 300\", \"beam\"",
                1
            )
    ));
    assert_eq!(loose.statics(), ketchup_program::Statics::NotVerified);
    let ignored: Vec<&str> = loose
        .issues
        .iter()
        .filter(|issue| issue.kind == "declared_bracing_ignored")
        .map(|issue| issue.message.as_str())
        .collect();
    assert_eq!(ignored.len(), 6, "{ignored:?}");
    assert!(
        ignored
            .iter()
            .any(|message| message.contains("lath 300") && message.contains("anchored")),
        "{ignored:?}"
    );
    assert!(
        ignored
            .iter()
            .any(|message| message.contains("lath 1200") && message.contains("fastened")),
        "{ignored:?}"
    );
}

/// The beam beside a second one on its own posts, a deck in two boards on
/// both, either side of the load, screwed only to the beams every 300 mm,
/// and a grounded wall past the beams' far ends that the second board butts.
fn deck(extra: &str) -> String {
    let screws = |from: i32, to: i32, y: i32| {
        (from..=to)
            .step_by(300)
            .map(|x| format!("({x}, {y}, 2400)"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 560, on = [\"pad\"], source = \"test\")
timber_design({{\"GL24h\": \"GL24h\"}})
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (100, 80, 2000), at = (5800, 0, 0), material = \"GL24h\", grounded = True)
box(\"post c\", (100, 80, 2000), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
box(\"post d\", (100, 80, 2000), at = (5800, 1000, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (5900, 80, 400), at = (0, 0, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"beam 2\", (5900, 80, 400), at = (0, 1000, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad\", (300, 80, 50), at = (2800, 0, 2400), material = \"GL24h\", tags = [\"pad\"])
box(\"end wall\", (100, 1080, 2600), at = (6000, 0, 0), material = \"GL24h\", grounded = True)
box(\"deck a\", (2750, 1080, 22), at = (0, 0, 2400), material = \"GL24h\", tags = [\"deck\"])
box(\"deck b\", (2850, 1080, 22), at = (3150, 0, 2400), material = \"GL24h\", tags = [\"deck\"])
joint(\"deck a\", \"beam\", kind = \"screw\", fasteners = [{}])
joint(\"deck b\", \"beam\", kind = \"screw\", fasteners = [{}])
joint(\"deck a\", \"beam 2\", kind = \"screw\")
joint(\"deck b\", \"beam 2\", kind = \"screw\")
{extra}",
        screws(150, 2700, 40),
        screws(3300, 5700, 40)
    )
}

/// Review 2026-11-08 §5.1: a deck screwed only to its joists braces nothing,
/// as they can buckle together, so a usual floor was never verified. Declared
/// a diaphragm anchored to the grounded wall it is fastened to, its boards
/// hold every joist they are screwed to; without a fastening to the wall it
/// is reported and holds nothing.
#[test]
fn a_deck_declared_a_diaphragm_anchored_to_a_wall_braces_the_joists() {
    let plain = report(&deck(""));
    assert!(
        load(&plain, "beam")
            .braced_by
            .iter()
            .all(|brace| !brace.part.starts_with("deck")),
        "{:?}",
        load(&plain, "beam").braced_by
    );

    let declared = "diaphragm([\"deck\"], anchored_to = [\"end wall\"])\n";
    let unanchored = report(&deck(declared));
    assert!(
        unanchored
            .issues
            .iter()
            .any(|issue| issue.kind == "declared_bracing_ignored"
                && issue.message.contains("end wall")),
        "{:?}",
        unanchored.issues
    );

    // A board over the gap, clear of the pad, ties the first board to the
    // second: a board reaching no anchor holds nothing (review 2026-11-09 PG-301).
    let anchored = report(&deck(&format!(
        "box(\"deck tie\", (700, 880, 22), at = (2600, 200, 2422), material = \"GL24h\", tags = [\"deck\"])
joint(\"deck tie\", \"deck a\", kind = \"screw\")
joint(\"deck tie\", \"deck b\", kind = \"screw\")
joint(\"deck b\", \"end wall\", kind = \"screw\")\n{declared}"
    )));
    let beam = load(&anchored, "beam");
    let held: Vec<f64> = beam
        .braced_by
        .iter()
        .filter(|brace| brace.part.starts_with("deck") && brace.edge == "top")
        .map(|brace| brace.from_mm)
        .collect();
    assert!(held.len() >= 18, "{:?}", beam.braced_by);
    for (report, status) in [
        (&plain, "not_verified"),
        (&unanchored, "not_verified"),
        (&anchored, "pass"),
    ] {
        let checked = report
            .design
            .members
            .iter()
            .find(|member| member.part == "beam")
            .expect("the beam is checked");
        assert_eq!(checked.status, status, "{checked:?}");
    }
}

/// A GL24h 80 x 400 rafter at 30 degrees from (0, 2000), drawn as an extrusion
/// of its side or as a turned box, and a board under its upper half screwed to
/// it and to a grounded wall past its side.
fn sloped_rafter(extruded: bool) -> String {
    rafter_at(extruded, 30.0, 3000.0)
}

/// The rafter of [`sloped_rafter`] at `degrees`, running `run` mm in plan.
fn rafter_at(extruded: bool, degrees: f64, run: f64) -> String {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let rise = run * sin / cos;
    let plumb = 400.0 / cos;
    let rafter = if extruded {
        format!(
            "part = extrude(\"rafter\", profile = [(0, 0), ({run}, {rise}), ({run}, {}), (0, {plumb})],
               distance = 80, material = \"GL24h\", tags = [\"frame\"])
place(part, origin = (0, 80, 2000), z = (0, -1, 0), x = (1, 0, 0))
",
            rise + plumb
        )
    } else {
        format!(
            "part = box(\"rafter\", ({}, 80, 400), material = \"GL24h\", tags = [\"frame\"])
place(part, origin = (0, 0, 2000), z = ({}, 0, {cos}), x = ({cos}, 0, {sin}))
",
            run / cos,
            -sin
        )
    };
    // The board's top on the rafter's underside 60 % of the run along the level.
    let (x, z) = (0.6 * run, 2000.0 + 0.6 * run * sin / cos);
    format!(
        "load_path(only = [\"frame\"])
timber_design({{\"GL24h\": \"GL24h\"}})
{rafter}board = box(\"board\", (600, 1100, 25), material = \"GL24h\")
place(board, origin = ({}, 0, {}), z = ({}, 0, {cos}), x = ({cos}, 0, {sin}))
box(\"wall\", (4000, 100, 5000), at = (0, 1100, 0), material = \"GL24h\", grounded = True)
joint(\"board\", \"rafter\", kind = \"screw\")
joint(\"board\", \"wall\", kind = \"screw\")
",
        x + 25.0 * sin,
        z - 25.0 * cos,
        -sin
    )
}

/// PG-141(b): a rafter extruded at a slope was taken along its level
/// projection, so a board under its upper half held its top, and lateral
/// torsional buckling passed at 0.886. The edge is read along its slope, and
/// the board holds its bottom, as of the same rafter drawn as a turned box.
#[test]
fn a_board_under_a_rafter_extruded_at_a_slope_holds_its_bottom() {
    for extruded in [true, false] {
        let report = report(&sloped_rafter(extruded));
        let rafter = load(&report, "rafter");
        let edges: Vec<&str> = rafter
            .braced_by
            .iter()
            .filter(|brace| brace.part == "board")
            .map(|brace| brace.edge.as_str())
            .collect();
        assert!(!edges.is_empty(), "{extruded}: {:?}", rafter.braced_by);
        assert!(
            edges.iter().all(|edge| *edge == "bottom"),
            "{extruded}: {edges:?}"
        );
    }
}

/// R1103-PG-146: a rafter steeper than about 72.5 degrees had no top or bottom
/// (the contact had to lie 0.3 of the way up from its centre). R1103-PG-141(b)
/// at a steeper pitch: a rafter extruded from its side whose rise and plumb
/// cut outgrow its run, from about 40 degrees, ran along its vertical local
/// axis, so nothing held it sideways. The board under it holds its bottom at
/// every pitch, and drawn either way while it is a beam; past about 64 degrees
/// an extruded one is a column, as the box is, and runs up its plumb axis.
#[test]
fn a_board_under_a_steep_rafter_holds_its_bottom() {
    for degrees in [35.0_f64, 45.0, 60.0, 70.0, 80.0] {
        let drawn = if degrees < 64.0 {
            &[false, true][..]
        } else {
            &[false]
        };
        for &extruded in drawn {
            // The same 3 464 mm along the slope at every pitch.
            let run = 3464.0 * degrees.to_radians().cos();
            let report = report(&rafter_at(extruded, degrees, run));
            let rafter = load(&report, "rafter");
            let edges: Vec<&str> = rafter
                .braced_by
                .iter()
                .filter(|brace| brace.part == "board")
                .map(|brace| brace.edge.as_str())
                .collect();
            let case = format!("{degrees} degrees, extruded {extruded}");
            assert_eq!(edges, ["bottom"], "{case}: {:?}", rafter.braced_by);
            assert!(rafter.axis[0].abs() > 0.1, "{case}: {:?}", rafter.axis);
        }
    }
}
