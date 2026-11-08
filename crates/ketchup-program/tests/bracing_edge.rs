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

/// A GL24h 80 x 400 rafter at 30 degrees from (0, 2000), drawn as an extrusion
/// of its side or as a turned box, and a board under its upper half screwed to
/// it and to a grounded wall past its side.
fn sloped_rafter(extruded: bool) -> String {
    let (sin, cos) = 30f64.to_radians().sin_cos();
    let rise = 3000.0 * sin / cos;
    let plumb = 400.0 / cos;
    let rafter = if extruded {
        format!(
            "part = extrude(\"rafter\", profile = [(0, 0), (3000, {rise}), (3000, {}), (0, {plumb})],
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
            3000.0 / cos,
            -sin
        )
    };
    // The board's top on the rafter's underside 1800 mm along the level.
    let (x, z) = (1800.0, 2000.0 + 1800.0 * sin / cos);
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
