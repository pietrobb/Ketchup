use ketchup_program::member_check::MemberCheck;
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");

fn report(source: &str, overrides: &[(&str, f64)]) -> Report {
    let overrides: BTreeMap<String, f64> = overrides
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect();
    run("design.star", source, &overrides)
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

fn member<'a>(report: &'a Report, name: &str) -> &'a MemberCheck {
    report
        .design
        .members
        .iter()
        .find(|member| member.part == name)
        .unwrap_or_else(|| panic!("no check of {name}: {:?}", report.design.members))
}

fn utilization(member: &MemberCheck, check: &str) -> f64 {
    member
        .checks
        .iter()
        .find(|c| c.name == check)
        .unwrap_or_else(|| panic!("no {check} in {:?}", member.checks))
        .utilization
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 0.01 * expected + 0.002,
        "{actual} != {expected}"
    );
}

/// A beam `b` x `h` mm, 4000 mm long, on two grounded 100 mm posts (centres 50
/// and 3950 mm), under a 600 mm wide deck with 2 kN/m² imposed load.
fn beam(material: &str, b: f64, h: f64) -> String {
    format!(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
box(\"post a\", (100, {b}, 2000), material = \"C24\", grounded = True)
box(\"post b\", (100, {b}, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (4000, {b}, {h}), at = (0, 0, 2000), material = \"{material}\", tags = [\"frame\"])
box(\"deck\", (4000, 600, 20), at = (0, {b} / 2.0 - 300, 2000 + {h}), material = \"OSB\", tags = [\"deck\"])
"
    )
}

#[test]
fn a_simply_supported_beam_matches_the_hand_calculation() {
    let report = report(&beam("C24", 100.0, 200.0), &[]);
    let beam = member(&report, "beam");
    assert_eq!(beam.role, "beam");
    assert_eq!(beam.section_mm, Some([100.0, 200.0]));
    assert_eq!(beam.missing, Vec::<String>::new());
    // q = 2 kN/m² x 0.6 m = 1.2 N/mm over the span of 3900 mm between the post centres;
    // medium-term imposed load: kmod 0.8, gamma_M 1.3, kh 1 (h > 150 mm).
    let (q, l) = (1.2, 3900.0);
    let moment = 1.5 * q * l * l / 8.0;
    close(
        utilization(beam, "bending"),
        moment / (100.0 * 200.0 * 200.0 / 6.0) / (0.8 * 24.0 / 1.3),
    );
    let shear = 1.5 * q * l / 2.0;
    close(
        utilization(beam, "shear"),
        1.5 * shear / (0.67 * 100.0 * 200.0) / (0.8 * 4.0 / 1.3),
    );
    let ei = 11000.0 * 100.0 * 200f64.powi(3) / 12.0;
    let w = 5.0 * q * l.powi(4) / (384.0 * ei);
    close(utilization(beam, "deflection_inst"), w / (l / 300.0));
    close(
        utilization(beam, "deflection_fin"),
        w * (1.0 + 0.3 * 0.6) / (l / 250.0),
    );
    // Each post takes half of the whole 4000 mm on its 100 x 100 mm top.
    let reaction = 1.5 * q * 2000.0;
    close(
        utilization(beam, "bearing"),
        reaction / 10_000.0 / (0.8 * 2.5 / 1.3),
    );
    assert_eq!(beam.governing.as_deref(), Some("deflection_inst"));
    assert_eq!(beam.status, "pass");
}

#[test]
fn a_beam_continuous_over_a_middle_post_takes_five_eighths_shear_there() {
    // 6000 mm beam on post centres 50, 3000, 5950 mm: two spans of 2950 mm.
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({\"C24\": \"C24\"})
for x in (0, 2950, 5900):
    box(\"post %d\" % x, (100, 100, 2000), at = (x, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (6000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (6000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
",
        &[],
    );
    let beam = member(&report, "beam");
    let (q, l) = (1.2, 2950.0);
    // Over the middle post: M = -q L² / 8, V = 0.625 q L (a simple span: 0.5 q L).
    let moment = 1.5 * q * l * l / 8.0;
    close(
        utilization(beam, "bending"),
        moment / (100.0 * 200.0 * 200.0 / 6.0) / (0.8 * 24.0 / 1.3),
    );
    let shear = 1.5 * 0.625 * q * l;
    close(
        utilization(beam, "shear"),
        1.5 * shear / (0.67 * 100.0 * 200.0) / (0.8 * 4.0 / 1.3),
    );
}

#[test]
fn imposed_load_on_two_of_three_spans_governs_the_moment_over_the_post_between_them() {
    // 8950 mm beam on post centres 50, 3000, 5950, 8900 mm: three spans of
    // 2950 mm, ending on the outer posts (with no permanent load, imposed load
    // on an overhang past them would tip it).
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({\"C24\": \"C24\"})
for x in (0, 2950, 5900, 8850):
    box(\"post %d\" % x, (100, 100, 2000), at = (x, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (8950, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (8950, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
",
        &[],
    );
    let beam = member(&report, "beam");
    let (q, l) = (1.2, 2950.0);
    // Three-moment equation with q on the first two spans only:
    // 4 M_B + M_C = -q L² / 2, M_B + 4 M_C = -q L² / 4, so M_B = -7/60 q L²
    // (loaded everywhere M_B = -q L² / 10, 14 % less).
    let moment = 1.5 * 7.0 / 60.0 * q * l * l;
    close(
        utilization(beam, "bending"),
        moment / (100.0 * 200.0 * 200.0 / 6.0) / (0.8 * 24.0 / 1.3),
    );
}

#[test]
fn bearings_closer_than_the_beam_is_deep_hold_it_as_one() {
    // A post 20 mm before a long sill, behind a 1 m overhang of a 200 mm deep
    // beam: the overhang's moment is no lever pair over the 70 mm from the
    // post centre to the sill (that would be about 0.6 in shear). With no
    // permanent load, imposed load on the overhang alone would tip the beam
    // (EQU), so it is screwed down to post b.
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({\"C24\": \"C24\"})
box(\"post a\", (100, 100, 2000), at = (1000, 0, 0), material = \"C24\", grounded = True)
box(\"sill\", (1880, 100, 2000), at = (1120, 0, 0), material = \"C24\", grounded = True)
b = box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
beam = box(\"beam\", (4000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (4000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
joint(beam, b, kind = \"screw\", fastener = \"screw 8 x 200\", fasteners = [(3950, 50, 2100)])
",
        &[],
    );
    let beam = member(&report, "beam");
    assert!(
        beam.checks
            .iter()
            .all(|check| check.at != "span 1050-1120 mm"),
        "{:?}",
        beam.checks
    );
    assert!(utilization(beam, "shear") < 0.2, "{:?}", beam.checks);
    assert_eq!(beam.status, "pass");
}

#[test]
fn an_undersized_beam_fails_and_a_beam_without_a_strength_class_is_not_verified() {
    let weak = report(&beam("C24", 60.0, 100.0), &[]);
    let beam = member(&weak, "beam");
    assert_eq!(beam.status, "fail");
    assert!(utilization(beam, "bending") > 2.0, "{:?}", beam.checks);

    let unknown = report(&beam_named("spruce"), &[]);
    let beam = member(&unknown, "beam");
    assert_eq!(beam.status, "not_verified");
    assert!(beam.checks.is_empty());
    assert_eq!(
        beam.missing,
        ["material \"spruce\" has no timber strength class (timber_strength)"]
    );
}

fn beam_named(material: &str) -> String {
    beam(material, 100.0, 200.0)
}

/// A 60 x 140 mm post 500 mm high on a grounded slab under a deck loaded with about
/// 17 kN (enough for a readable utilization), with a
/// 10 mm deep notch from z = 250 to 310 in one side when `notched`.
fn post(notched: bool) -> String {
    let notch = if notched {
        "(60, 200), (50, 200), (50, 260), (60, 260), "
    } else {
        ""
    };
    format!(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"heavy\", kind = \"imposed\", kn_m2 = 2000.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
box(\"slab\", (300, 300, 50), at = (-100, -80, 0), material = \"C24\", grounded = True)
part = extrude(\"post\", profile = [(0, 0), (60, 0), {notch}(60, 500), (0, 500)], distance = 140,
               material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 140, 50), z = (0, -1, 0), x = (1, 0, 0))
box(\"deck\", (60, 140, 20), at = (0, 0, 550), material = \"OSB\", tags = [\"deck\"])
"
    )
}

#[test]
fn a_slender_post_buckles_with_kc_from_its_relative_slenderness() {
    // The post of `post` 3000 mm high: lambda_rel about the 60 mm side.
    let source = post(false)
        .replace("(60, 500), (0, 500)", "(60, 3000), (0, 3000)")
        .replace("at = (0, 0, 550)", "at = (0, 0, 3050)");
    let report = report(&source, &[]);
    let post = member(&report, "post");
    assert_eq!(post.role, "column");
    // C24: fc,0,k 21, E0,05 7400 MPa; solid timber beta_c 0.2 (6.3.2).
    let lambda = 3000.0 * 12f64.sqrt() / 60.0 / std::f64::consts::PI * (21.0f64 / 7400.0).sqrt();
    let k = 0.5 * (1.0 + 0.2 * (lambda - 0.3) + lambda * lambda);
    let kc = 1.0 / (k + (k * k - lambda * lambda).sqrt());
    assert!((kc - 0.108).abs() < 0.001, "{kc}");
    // 60 x 140 mm of deck at 2000 kN/m², 1.5 Q, medium term (kmod 0.8).
    let force = 1.5 * 2000.0e-3 * 60.0 * 140.0;
    close(
        utilization(post, "compression"),
        force / (60.0 * 140.0) / (kc * 0.8 * 21.0 / 1.3),
    );
    assert_eq!(post.status, "fail");
}

#[test]
fn a_beam_notched_over_its_bearing_is_checked_in_shear_with_kv() {
    // The beam of `beam` 100 x 200, its underside notched 40 mm deep over
    // the first 150 mm; the left post reaches up into the notch.
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({\"C24\": \"C24\"})
box(\"post a\", (100, 100, 2040), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
part = extrude(\"beam\", profile = [(0, 40), (150, 40), (150, 0), (4000, 0), (4000, 200), (0, 200)],
               distance = 100, material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 100, 2000), z = (0, -1, 0), x = (1, 0, 0))
box(\"deck\", (4000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
",
        &[],
    );
    let beam = member(&report, "beam");
    // 6.5.2 (6.62), right-angled (i = 0): h_ef = 160, alpha = 0.8, kn = 5,
    // x = 100 mm from the middle of the post (50 mm) to the corner of the
    // notch (150 mm), fig. 6.11 (review 2026-10-17 PG-3).
    let (h, alpha, x) = (200.0f64, 0.8f64, 100.0);
    let kv = 5.0
        / (h.sqrt()
            * ((alpha * (1.0 - alpha)).sqrt()
                + 0.8 * x / h * (1.0 / alpha - alpha * alpha).sqrt()));
    assert!((kv - 0.496).abs() < 0.001, "{kv}");
    // Half of 1.2 N/mm over 4000 mm, 1.5 Q, on the net 100 x 160 mm.
    let reaction = 1.5 * 1.2 * 4000.0 / 2.0;
    close(
        utilization(beam, "shear_notch"),
        1.5 * reaction / (0.67 * 100.0 * 160.0) / (kv * 0.8 * 4.0 / 1.3),
    );
}

/// The span of `beam` with a beam of cross-section `profile` (local XY within
/// 200 x 200, local Y up) extruded 4000 mm along world X, `then` run on it.
fn profiled_beam(profile: &str, then: &str) -> String {
    format!(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
box(\"post a\", (100, 200, 2000), material = \"C24\", grounded = True)
box(\"post b\", (100, 200, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
part = extrude(\"beam\", profile = {profile}, distance = 4000, material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 0, 2000), z = (1, 0, 0), x = (0, 1, 0))
{then}
box(\"deck\", (4000, 600, 20), at = (0, -200, 2200), material = \"OSB\", tags = [\"deck\"])
"
    )
}

#[test]
fn a_hollow_beam_is_checked_in_shear_on_its_walls_at_the_neutral_axis() {
    // Review 2026-10-17 PG-6: a 200 x 200 tube with 20 mm walls. tau = V Q / (I b)
    // at the neutral axis, where only the two walls (b = 40 mm) carry it;
    // 1.5 V / A understates it by a third.
    let report = report(
        &profiled_beam(
            "[(0, 0), (200, 0), (200, 200), (0, 200)]",
            "cut(part, profile = [(20, 20), (180, 20), (180, 180), (20, 180)], through = True, name = \"bore\")",
        ),
        &[],
    );
    let beam = member(&report, "beam");
    assert_eq!(beam.role, "beam");
    assert_eq!(beam.missing, Vec::<String>::new());
    let q_first = 200.0 * 100.0 * 50.0 - 160.0 * 80.0 * 40.0;
    let inertia = (200f64.powi(4) - 160f64.powi(4)) / 12.0;
    let shear = 1.5 * 1.2 * 3900.0 / 2.0;
    let tau = shear * q_first / (inertia * 0.67 * 40.0);
    assert!(
        tau > 1.4 * 1.5 * shear / (0.67 * (200.0 * 200.0 - 160.0 * 160.0)),
        "{tau}"
    );
    close(utilization(beam, "shear"), tau / (0.8 * 4.0 / 1.3));
    close(
        utilization(beam, "bending"),
        1.5 * 1.2 * 3900f64.powi(2) / 8.0 / (inertia / 100.0) / (0.8 * 24.0 / 1.3),
    );
}

#[test]
fn a_z_beam_bends_about_its_principal_axes_not_its_horizontal_one() {
    // Review 2026-10-17 PG-6: a Z 200 x 200 with 50 mm flanges and web under a
    // vertical load bends sideways too (free unsymmetric bending); the second
    // moment about its horizontal axis understates its stress and deflection.
    // Symmetric about its centroid, it does not twist.
    let corners = [
        (100.0, 0.0),
        (200.0, 0.0),
        (200.0, 50.0),
        (125.0, 50.0),
        (125.0, 150.0),
        (100.0, 150.0),
        (100.0, 200.0),
        (0.0, 200.0),
        (0.0, 150.0),
        (75.0, 150.0),
        (75.0, 50.0),
        (100.0, 50.0),
    ];
    let profile = format!(
        "[{}]",
        corners
            .iter()
            .map(|(x, y)| format!("({x}, {y})"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let report = report(&profiled_beam(&profile, ""), &[]);
    let beam = member(&report, "beam");
    assert_eq!(beam.role, "beam");
    assert_eq!(beam.missing, Vec::<String>::new());
    let (_, [cx, cy], ix, iy, ixy) = moments(
        &[
            [100.0, 0.0, 100.0, 50.0],
            [75.0, 50.0, 50.0, 100.0],
            [0.0, 150.0, 100.0, 50.0],
        ],
        &[],
    );
    let det = ix * iy - ixy * ixy;
    // sigma = M (Iy y - Ixy x) / (Ix Iy - Ixy²), largest at a corner.
    let per_moment = corners
        .iter()
        .map(|(x, y)| ((iy * (y - cy) - ixy * (x - cx)) / det).abs())
        .fold(0.0, f64::max);
    let (q, l) = (1.2, 3900.0);
    let bending = 1.5 * q * l * l / 8.0 * per_moment / (0.8 * 24.0 / 1.3);
    let about_horizontal = 1.5 * q * l * l / 8.0 * (200.0 - cy) / ix / (0.8 * 24.0 / 1.3);
    assert!(
        bending > 2.5 * about_horizontal,
        "{bending} {about_horizontal}"
    );
    close(utilization(beam, "bending"), bending);
    // The vertical deflection takes Ix - Ixy² / Iy.
    let w = 5.0 * q * l.powi(4) / (384.0 * 11000.0 * det / iy);
    close(utilization(beam, "deflection_inst"), w / (l / 300.0));
}

#[test]
fn a_beam_whose_load_passes_beside_its_shear_centre_is_not_verified() {
    // Review 2026-10-17 PG-6: a vertical load through the centroid of an L, or
    // of a channel open to the side, passes beside the shear centre and twists
    // the beam; torsion is not checked, so neither is the beam. A channel open
    // upward is symmetric about its vertical axis and only bends.
    let angle = "[(0, 0), (200, 0), (200, 50), (50, 50), (50, 200), (0, 200)]";
    let sideways =
        "[(0, 0), (200, 0), (200, 50), (50, 50), (50, 150), (200, 150), (200, 200), (0, 200)]";
    let upward =
        "[(0, 0), (200, 0), (200, 200), (150, 200), (150, 50), (50, 50), (50, 200), (0, 200)]";
    for (profile, twists) in [(angle, true), (sideways, true), (upward, false)] {
        let report = report(&profiled_beam(profile, ""), &[]);
        let beam = member(&report, "beam");
        assert_eq!(beam.role, "beam");
        let torsion = beam.missing.iter().any(|m| m.contains("shear centre"));
        assert_eq!(torsion, twists, "{profile}: {:?}", beam.missing);
        if twists {
            assert_eq!(beam.status, "not_verified", "{profile}");
            assert!(beam.checks.is_empty(), "{profile}");
        } else {
            assert_eq!(beam.missing, Vec::<String>::new(), "{profile}");
        }
    }
}

/// The 100 x 200 beam of `beam` with `then` run on it (the beam is `part`):
/// its status, check names and what the check could not model.
fn changed_beam(then: &str) -> (String, Vec<String>, Vec<String>) {
    let source = format!(
        "{}{then}\n",
        beam("C24", 100.0, 200.0).replace("box(\"beam\"", "part = box(\"beam\"")
    );
    let report = report(&source, &[]);
    let beam = member(&report, "beam");
    assert_eq!(beam.role, "beam");
    let checks = beam.checks.iter().map(|c| c.name.to_owned()).collect();
    (beam.status.to_string(), checks, beam.missing.clone())
}

#[test]
fn a_beam_notched_pocketed_or_drilled_in_its_span_is_not_checked_as_the_full_section() {
    // Each halves the depth or takes 30 % of it at mid-span, where the moment
    // is largest; on the full section the same beam passes.
    for then in [
        "subtract(part, box(\"notch\", (200, 100, 100), at = (1900, 0, 2000), tool = True), name = \"notch\")",
        "pocket(part, \"z-\", rect = (1900, 20, 2100, 80), depth = 100)",
        "hole(part, \"z+\", world = (2000, 50, 2200), diameter = 60, depth = 200, through = True)",
    ] {
        let (status, checks, missing) = changed_beam(then);
        assert_eq!(status, "not_verified", "{then}: {checks:?} {missing:?}");
        assert!(
            checks.iter().all(|c| c != "bending" && c != "shear"),
            "{then}: {checks:?}"
        );
        assert!(
            missing.len() == 1 && missing[0].contains("section"),
            "{then}: {missing:?}"
        );
    }
    // A bolt hole across it leaves the section all but whole.
    let (status, _, missing) = changed_beam(
        "hole(part, \"y-\", world = (2000, 0, 2100), diameter = 5, depth = 100, through = True)",
    );
    assert_eq!(status, "pass", "{missing:?}");
}

#[test]
fn a_notch_in_a_side_of_a_short_post_governs_its_compression_on_its_thinnest_section() {
    let compression = |notched: bool| {
        let report = report(&post(notched), &[]);
        let post = member(&report, "post");
        assert_eq!(post.role, "column");
        let check = post
            .checks
            .iter()
            .find(|c| c.name == "compression")
            .unwrap_or_else(|| panic!("{:?}", post.checks))
            .clone();
        (check.utilization, check.at)
    };
    let (plain, plain_at) = compression(false);
    let (notched, notched_at) = compression(true);
    assert!(plain_at.starts_with("length"), "{plain_at}");
    // The notch is away from the ends: it buckles on 50 x 140, with kc near 1.
    assert_eq!(notched_at, "length 500 mm, thinnest section 7000 mm²");
    assert!(notched > plain * 1.1, "{notched} {plain}");
    let lambda = 500.0 * 12f64.sqrt() / 50.0 / std::f64::consts::PI * (21.0f64 / 7400.0).sqrt();
    let k = 0.5 * (1.0 + 0.2 * (lambda - 0.3) + lambda * lambda);
    let kc = 1.0 / (k + (k * k - lambda * lambda).sqrt());
    let force = 1.5 * 2000.0e-3 * 60.0 * 140.0;
    close(notched, force / (50.0 * 140.0) / (kc * 0.8 * 21.0 / 1.3));
}

/// The compression check of the post of `post` with each `(from, to)` replaced.
fn post_compression(replacements: &[(&str, &str)]) -> (f64, String) {
    let mut source = post(false);
    for (from, to) in replacements {
        assert!(source.contains(from), "{from}");
        source = source.replace(from, to);
    }
    let report = report(&source, &[]);
    let post = member(&report, "post");
    assert_eq!(post.role, "column");
    let check = post
        .checks
        .iter()
        .find(|c| c.name == "compression")
        .unwrap_or_else(|| panic!("{:?} {:?}", post.checks, post.missing));
    (check.utilization, check.at.clone())
}

#[test]
fn a_lap_at_the_top_of_a_post_is_checked_on_its_net_section() {
    // Review 2026-10-17 PG-5: the 60 x 140 post halved to 30 mm over its top
    // 100 mm carries the whole load there on 30 x 140 mm.
    let (utilization, at) = post_compression(&[("(60, 500)", "(60, 400), (30, 400), (30, 500)")]);
    assert_eq!(at, "net section 4200 mm² at a notch");
    let force = 1.5 * 2000.0e-3 * 60.0 * 140.0;
    close(utilization, force / (30.0 * 140.0) / (0.8 * 21.0 / 1.3));
}

#[test]
fn a_tall_post_notched_at_mid_height_buckles_on_its_thinnest_section() {
    // The post of `post` 3000 mm high, cut to 40 mm over 1400..1600 mm.
    let (utilization, at) = post_compression(&[
        (
            "(60, 500), (0, 500)",
            "(60, 1400), (40, 1400), (40, 1600), (60, 1600), (60, 3000), (0, 3000)",
        ),
        ("at = (0, 0, 550)", "at = (0, 0, 3050)"),
    ]);
    let lambda = 3000.0 * 12f64.sqrt() / 40.0 / std::f64::consts::PI * (21.0f64 / 7400.0).sqrt();
    let k = 0.5 * (1.0 + 0.2 * (lambda - 0.3) + lambda * lambda);
    let kc = 1.0 / (k + (k * k - lambda * lambda).sqrt());
    let force = 1.5 * 2000.0e-3 * 60.0 * 140.0;
    close(
        utilization,
        force / (40.0 * 140.0) / (kc * 0.8 * 21.0 / 1.3),
    );
    assert_eq!(at, "length 3000 mm, thinnest section 5600 mm²");
}

#[test]
fn a_tall_post_with_a_v_notch_buckles_on_the_bottom_of_the_notch() {
    // Capability review 137: the width runs 60 -> 20 -> 60 mm over 1400..1600 mm,
    // so it is thinnest at the corner at 1500 mm, not between corners.
    let (utilization, at) = post_compression(&[
        (
            "(60, 500), (0, 500)",
            "(60, 1400), (20, 1500), (60, 1600), (60, 3000), (0, 3000)",
        ),
        ("at = (0, 0, 550)", "at = (0, 0, 3050)"),
    ]);
    assert_eq!(at, "length 3000 mm, thinnest section 2800 mm²");
    let lambda = 3000.0 * 12f64.sqrt() / 20.0 / std::f64::consts::PI * (21.0f64 / 7400.0).sqrt();
    let k = 0.5 * (1.0 + 0.2 * (lambda - 0.3) + lambda * lambda);
    let kc = 1.0 / (k + (k * k - lambda * lambda).sqrt());
    let force = 1.5 * 2000.0e-3 * 60.0 * 140.0;
    close(
        utilization,
        force / (20.0 * 140.0) / (kc * 0.8 * 21.0 / 1.3),
    );
}

#[test]
fn a_sloped_lap_at_the_top_of_a_post_is_checked_on_its_thinnest_end() {
    // The 60 x 140 post tapers to 30 mm over its top 100 mm.
    let (utilization, at) = post_compression(&[("(60, 500)", "(60, 400), (30, 500)")]);
    assert_eq!(at, "net section 4200 mm² at a notch");
    let force = 1.5 * 2000.0e-3 * 60.0 * 140.0;
    close(utilization, force / (30.0 * 140.0) / (0.8 * 21.0 / 1.3));
}

#[test]
fn a_post_cut_on_a_slope_under_a_sloped_deck_is_no_notch() {
    // A gable stud: the top runs from 450 mm at one side to 500 mm at the
    // other, and a deck 78.1 mm (sqrt(60² + 50²)) wide lies on that slope,
    // loaded per plan area (60 x 140 mm). The tip is where the load bears,
    // not a section of no area: the post buckles on 60 x 140 mm.
    let (utilization, at) = post_compression(&[
        ("(60, 500)", "(60, 450)"),
        (
            "box(\"deck\", (60, 140, 20), at = (0, 0, 550), material = \"OSB\", tags = [\"deck\"])",
            "deck = box(\"deck\", (78.1025, 140, 20), at = (0, 0, 0), material = \"OSB\", tags = [\"deck\"])
place(deck, origin = (60, 140, 500), z = (50, 0, 60), x = (-60, 0, 50))",
        ),
    ]);
    assert_eq!(at, "length 500 mm");
    let lambda = 500.0 * 12f64.sqrt() / 60.0 / std::f64::consts::PI * (21.0f64 / 7400.0).sqrt();
    let k = 0.5 * (1.0 + 0.2 * (lambda - 0.3) + lambda * lambda);
    let kc = 1.0 / (k + (k * k - lambda * lambda).sqrt());
    let force = 1.5 * 2000.0e-3 * 60.0 * 140.0;
    close(
        utilization,
        force / (60.0 * 140.0) / (kc * 0.8 * 21.0 / 1.3),
    );
}

/// A C24 column 3000 mm high of `profile` (local XY, extruded up) within a
/// 200 x 200 mm square, on a grounded slab under a 200 x 200 mm deck loaded
/// with 5000 kN/m² (1.5 Q = 300 kN), `then` run on the column part.
fn column(profile: &str, then: &str) -> String {
    format!(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"heavy\", kind = \"imposed\", kn_m2 = 5000.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
box(\"slab\", (400, 400, 50), at = (-100, -100, 0), material = \"C24\", grounded = True)
part = extrude(\"column\", profile = {profile}, distance = 3000, at = (0, 0, 50),
               material = \"C24\", tags = [\"frame\"])
{then}
box(\"deck\", (200, 200, 20), at = (0, 0, 3050), material = \"OSB\", tags = [\"deck\"])
"
    )
}

/// Area and least second moment of rectangles `(x, y, width, depth)` less
/// rectangles `holes`, by hand: parallel axes and the principal-axis formula.
fn least_inertia(solid: &[[f64; 4]], holes: &[[f64; 4]]) -> (f64, f64) {
    let (area, _, ix, iy, ixy) = moments(solid, holes);
    let least = (ix + iy) / 2.0 - ((ix - iy) / 2.0).hypot(ixy);
    (area, least)
}

/// Area, centroid and centroidal second moments Ix, Iy, Ixy of rectangles
/// `(x, y, width, depth)` less rectangles `holes`, by parallel axes.
fn moments(solid: &[[f64; 4]], holes: &[[f64; 4]]) -> (f64, [f64; 2], f64, f64, f64) {
    let parts: Vec<([f64; 4], f64)> = solid
        .iter()
        .map(|r| (*r, 1.0))
        .chain(holes.iter().map(|r| (*r, -1.0)))
        .collect();
    let area: f64 = parts.iter().map(|(r, s)| s * r[2] * r[3]).sum();
    let cx = parts
        .iter()
        .map(|(r, s)| s * r[2] * r[3] * (r[0] + r[2] / 2.0))
        .sum::<f64>()
        / area;
    let cy = parts
        .iter()
        .map(|(r, s)| s * r[2] * r[3] * (r[1] + r[3] / 2.0))
        .sum::<f64>()
        / area;
    let (mut ix, mut iy, mut ixy) = (0.0, 0.0, 0.0);
    for ([x, y, w, d], s) in &parts {
        let (dx, dy, a) = (x + w / 2.0 - cx, y + d / 2.0 - cy, w * d);
        ix += s * (w * d.powi(3) / 12.0 + a * dy * dy);
        iy += s * (d * w.powi(3) / 12.0 + a * dx * dx);
        ixy += s * a * dx * dy;
    }
    (area, [cx, cy], ix, iy, ixy)
}

/// EC5 6.3.2 compression utilization of the column of `column` with the
/// real area and least radius of gyration (C24, medium term, 1.5 Q).
fn buckling(area: f64, least: f64) -> f64 {
    let radius = (least / area).sqrt();
    let lambda = 3000.0 / radius / std::f64::consts::PI * (21.0f64 / 7400.0).sqrt();
    let k = 0.5 * (1.0 + 0.2 * (lambda - 0.3) + lambda * lambda);
    let kc = 1.0 / (k + (k * k - lambda * lambda).sqrt());
    300_000.0 / area / (kc * 0.8 * 21.0 / 1.3)
}

#[test]
fn a_z_column_buckles_about_its_weak_principal_axis_on_its_real_area() {
    // Z 200 x 200 with 50 mm flanges and web: 15 000 mm², not the 40 000 of
    // its bounding square, and weakest about a tilted principal axis, not
    // about a side. Symmetric about its centroid, its shear centre is there.
    let report = report(
        &column(
            "[(100, 0), (200, 0), (200, 50), (125, 50), (125, 150), (100, 150), (100, 200), \
             (0, 200), (0, 150), (75, 150), (75, 50), (100, 50)]",
            "",
        ),
        &[],
    );
    let column = member(&report, "column");
    assert_eq!(column.role, "column");
    assert_eq!(column.missing, Vec::<String>::new());
    let (area, least) = least_inertia(
        &[
            [100.0, 0.0, 100.0, 50.0],
            [75.0, 50.0, 50.0, 100.0],
            [0.0, 150.0, 100.0, 50.0],
        ],
        &[],
    );
    assert!((area - 15_000.0).abs() < 1e-6, "{area}");
    let expected = buckling(area, least);
    // About the weaker side axis it gives 2.41 and the real area without kc
    // 1.55: both far under the real utilization.
    assert!(expected > 7.0, "{expected}");
    close(utilization(column, "compression"), expected);
    assert_eq!(column.status, "fail");
}

#[test]
fn a_column_whose_shear_centre_is_off_its_centroid_is_not_verified() {
    // Review 2026-10-17 PG-6: an L, or a channel, buckles in a flexural-
    // torsional mode about its shear centre beside the centroid; that is not
    // checked, so neither is the column.
    for profile in [
        "[(0, 0), (200, 0), (200, 50), (50, 50), (50, 200), (0, 200)]",
        "[(0, 0), (200, 0), (200, 50), (50, 50), (50, 150), (200, 150), (200, 200), (0, 200)]",
    ] {
        let report = report(&column(profile, ""), &[]);
        let column = member(&report, "column");
        assert_eq!(column.role, "column");
        assert!(
            column.missing.len() == 1
                && column.missing[0].contains("shear centre")
                && column.missing[0].contains("flexural-torsional"),
            "{profile}: {:?}",
            column.missing
        );
        assert_eq!(column.status, "not_verified", "{profile}");
        assert!(column.checks.is_empty(), "{profile}");
    }
}

#[test]
fn a_hollow_column_is_checked_on_its_walls_alone() {
    // A 200 x 200 tube with 30 mm walls, bored through its whole length.
    let report = report(
        &column(
            "[(0, 0), (200, 0), (200, 200), (0, 200)]",
            "cut(part, profile = [(30, 30), (170, 30), (170, 170), (30, 170)], through = True, name = \"bore\")",
        ),
        &[],
    );
    let column = member(&report, "column");
    assert_eq!(column.role, "column");
    let (area, least) = least_inertia(&[[0.0, 0.0, 200.0, 200.0]], &[[30.0, 30.0, 140.0, 140.0]]);
    assert!((area - 20_400.0).abs() < 1e-6, "{area}");
    let expected = buckling(area, least);
    assert!(expected > 1.2, "{expected}");
    close(utilization(column, "compression"), expected);
    assert_eq!(column.status, "fail");
}

#[test]
fn a_column_whose_section_changes_along_its_length_is_not_verified() {
    // Bored 1000 mm down from the top: the check models no such section.
    let report = report(
        &column(
            "[(0, 0), (200, 0), (200, 200), (0, 200)]",
            "cut(part, profile = [(30, 30), (170, 30), (170, 170), (30, 170)], depth = 1000, name = \"bore\")",
        ),
        &[],
    );
    let column = member(&report, "column");
    assert_eq!(column.role, "column");
    assert_eq!(column.status, "not_verified");
    assert!(
        column.checks.iter().all(|c| c.name != "compression"),
        "{:?}",
        column.checks
    );
    assert_eq!(column.missing.len(), 1, "{:?}", column.missing);
    assert!(
        column.missing[0].contains("section"),
        "{:?}",
        column.missing
    );
}

/// The column of `column` on a 200 x 200 square with `then` applied: its status
/// and what the check could not model.
fn square_column(then: &str) -> (String, Vec<String>) {
    let report = report(
        &column("[(0, 0), (200, 0), (200, 200), (0, 200)]", then),
        &[],
    );
    let column = member(&report, "column");
    assert_eq!(column.role, "column");
    (column.status.to_string(), column.missing.clone())
}

#[test]
fn a_column_bored_along_its_length_is_not_checked_as_the_solid_square() {
    // A 160 mm bore through the whole length leaves a tube of about 19 900 mm²,
    // half of the square the check would otherwise pass at about 0.75.
    for then in [
        "hole(part, \"end\", world = (100, 100, 3050), diameter = 160, depth = 3000, through = True)",
        "hole(part, \"end\", world = (100, 100, 3050), diameter = 160, depth = 1000)",
    ] {
        let (status, missing) = square_column(then);
        assert_eq!(status, "not_verified", "{then}");
        assert!(
            missing.len() == 1 && missing[0].contains("section"),
            "{missing:?}"
        );
    }
}

/// The 200 x 200 x 3000 column of `column` drawn instead as `post` draws its
/// post: a side elevation extruded 200 mm and stood up, `then` run on it.
fn side_column(then: &str) -> (String, Vec<String>) {
    let report = report(
        &format!(
            "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"heavy\", kind = \"imposed\", kn_m2 = 5000.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
box(\"slab\", (400, 400, 50), at = (-100, -100, 0), material = \"C24\", grounded = True)
part = extrude(\"column\", profile = [(0, 0), (200, 0), (200, 3000), (0, 3000)], distance = 200,
               material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 200, 50), z = (0, -1, 0), x = (1, 0, 0))
{then}
box(\"deck\", (200, 200, 20), at = (0, 0, 3050), material = \"OSB\", tags = [\"deck\"])
"
        ),
        &[],
    );
    let column = member(&report, "column");
    assert_eq!(column.role, "column");
    (column.status.to_string(), column.missing.clone())
}

#[test]
fn a_column_drawn_as_its_side_and_bored_or_cut_is_not_checked_as_the_solid_square() {
    for then in [
        "hole(part, \"segment3\", world = (100, 100, 3050), diameter = 160, depth = 3000, through = True)",
        "hole(part, \"segment3\", world = (100, 100, 3050), diameter = 160, depth = 1000)",
        "hole(part, \"segment2\", world = (200, 100, 1500), diameter = 60, depth = 200, through = True)",
        "cut(part, profile = [(30, 30), (170, 30), (170, 2970), (30, 2970)], through = True, name = \"slot\")",
    ] {
        let (status, missing) = side_column(then);
        assert_eq!(status, "not_verified", "{then}");
        assert!(
            missing.len() == 1 && missing[0].contains("section"),
            "{then}: {missing:?}"
        );
    }
    // A bolt hole across it leaves the section all but whole.
    let (status, missing) = side_column(
        "hole(part, \"segment2\", world = (200, 100, 1500), diameter = 10, depth = 200, through = True)",
    );
    assert_ne!(status, "not_verified", "{missing:?}");
    let (plain, _) = side_column("");
    assert_ne!(plain, "not_verified");
}

#[test]
fn a_column_drilled_or_rounded_across_beyond_a_small_size_is_not_verified() {
    for then in [
        "hole(part, \"segment2\", world = (200, 100, 1500), diameter = 60, depth = 200, through = True)",
        "chamfer(part, edges = [[\"segment1\", \"segment2\"], [\"segment3\", \"segment4\"]], distance = 40)",
    ] {
        let (status, missing) = square_column(then);
        assert_eq!(status, "not_verified", "{then}");
        assert!(
            missing.len() == 1 && missing[0].contains("section"),
            "{missing:?}"
        );
    }
    // A bolt hole and an eased arris leave the section all but whole.
    for then in [
        "hole(part, \"segment2\", world = (200, 100, 1500), diameter = 10, depth = 200, through = True)",
        "chamfer(part, edges = [[\"segment2\", \"segment3\"]], distance = 5)",
    ] {
        let (status, missing) = square_column(then);
        assert_ne!(status, "not_verified", "{then}: {missing:?}");
    }
}

#[test]
fn two_small_holes_in_one_section_count_together() {
    // Two 10 mm bolts side by side at 1500 mm take 20 mm of the 200 mm side,
    // 10 %: over the 5 % one of them alone stays within.
    let side_by_side = "hole(part, \"segment2\", world = (200, 60, 1500), diameter = 10, depth = 200, through = True)
hole(part, \"segment2\", world = (200, 140, 1500), diameter = 10, depth = 200, through = True)";
    let (status, missing) = square_column(side_by_side);
    assert_eq!(status, "not_verified");
    assert!(
        missing.len() == 1 && missing[0].contains("together"),
        "{missing:?}"
    );
    // A metre apart they never meet in one section.
    let apart = "hole(part, \"segment2\", world = (200, 100, 1000), diameter = 10, depth = 200, through = True)
hole(part, \"segment2\", world = (200, 100, 2000), diameter = 10, depth = 200, through = True)";
    let (status, missing) = square_column(apart);
    assert_ne!(status, "not_verified", "{missing:?}");
}

#[test]
fn a_beam_across_both_legs_of_one_sill_is_checked_over_the_span_between_them() {
    // One U-shaped sill part, legs 100 mm wide at both ends of a 45 x 145 beam:
    // two bearings 3900 mm apart, not one 4000 mm bed.
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({\"C24\": \"C24\"})
extrude(\"sill\", profile = [(0, 0), (4000, 0), (4000, 600), (3900, 600), (3900, 100), (100, 100),
        (100, 600), (0, 600)], distance = 100, material = \"C24\", grounded = True)
box(\"beam\", (4000, 45, 145), at = (0, 300, 100), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (4000, 600, 20), at = (0, 22.5, 245), material = \"OSB\", tags = [\"deck\"])
",
        &[],
    );
    let loads = report
        .loads
        .members
        .iter()
        .find(|member| member.part == "beam")
        .expect("loads on the beam");
    assert_eq!(loads.reactions.len(), 2, "{:?}", loads.reactions);
    let beam = member(&report, "beam");
    // q = 1.2 N/mm over 3900 mm; kh = (150 / 145)^0.2.
    let moment = 1.5 * 1.2 * 3900f64.powi(2) / 8.0;
    let strength = 0.8 * 24.0 / 1.3 * (150.0f64 / 145.0).powf(0.2);
    close(
        utilization(beam, "bending"),
        moment / (45.0 * 145.0 * 145.0 / 6.0) / strength,
    );
    assert_eq!(beam.status, "fail");
}

#[test]
fn an_incomplete_load_reports_the_known_utilization_but_never_passes() {
    let source = format!(
        "{}area_load(\"snow\", kind = \"snow\", kn_m2 = snow_load(0, 40), on = [\"deck\"])\n",
        beam("C24", 100.0, 200.0)
    );
    let report = report(&source, &[]);
    let beam = member(&report, "beam");
    assert!(beam.utilization.is_some_and(|u| u > 0.3 && u < 1.0));
    assert_eq!(beam.missing, ["snow (snow) has no value"]);
    assert_eq!(beam.status, "not_verified");
}

#[test]
fn every_member_of_the_house_has_a_utilization_and_passes_once_the_snow_is_given() {
    let with = report(HOUSE, &[("snow_sk", 1.5)]);
    let members = &with.design.members;
    assert_eq!(members.len(), with.loads.members.len());
    assert!(members.len() > 200, "{}", members.len());
    assert!(!with.design.basis.is_empty());
    let mut spans = 0;
    for member in members {
        assert_eq!(member.missing, Vec::<String>::new(), "{}", member.part);
        assert_eq!(member.status, "pass", "{member:?}");
        // A pass stands on every check of its role having run with a load,
        // not on checks that were skipped.
        // A plate bedded along its whole length has no span to bend; a beam
        // with a span has every span check.
        let spanning = member.checks.iter().any(|check| check.name == "bending");
        if spanning {
            spans += 1;
        }
        let required: &[&str] = if member.role == "column" {
            &["compression"]
        } else if spanning {
            &["bending", "shear", "deflection_inst", "deflection_fin"]
        } else {
            &[]
        };
        for check in required {
            let value = utilization(member, check);
            assert!((0.0..=1.0).contains(&value), "{check} {value} {member:?}");
        }
        let governing = member
            .checks
            .iter()
            .max_by(|a, b| a.utilization.total_cmp(&b.utilization))
            .expect("checks");
        assert_eq!(
            member.utilization,
            Some(governing.utilization),
            "{member:?}"
        );
        assert_eq!(
            member.governing.as_deref(),
            Some(governing.name),
            "{member:?}"
        );
    }
    let columns = members.iter().filter(|m| m.role == "column").count();
    assert!(columns > 50, "{columns}");
    // Joists, rafters, headers and plates over gaps: fewer means checks were skipped.
    assert!(spans >= 75, "{spans} of {}", members.len());
    // The rafters sit on the wall plate with a birdsmouth: the notch is checked.
    let rafter = member(&with, "konštrukcia/strecha južná/krokva 3");
    for check in [
        "bending",
        "shear",
        "deflection_fin",
        "shear_notch",
        "bearing",
    ] {
        assert!(utilization(rafter, check) > 0.0, "{check}");
    }
    // Without the site's snow nothing under the roof passes.
    let without = report(HOUSE, &[]);
    for member in &without.design.members {
        assert!(member.utilization.is_some(), "{}", member.part);
        if !member.missing.is_empty() {
            assert_eq!(member.status, "not_verified", "{}", member.part);
        }
    }
}
