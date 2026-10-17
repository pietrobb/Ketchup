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
    // x = 50 mm (half the bearing).
    let (h, alpha, x) = (200.0f64, 0.8f64, 50.0);
    let kv = 5.0
        / (h.sqrt()
            * ((alpha * (1.0 - alpha)).sqrt()
                + 0.8 * x / h * (1.0 / alpha - alpha * alpha).sqrt()));
    assert!((kv - 0.636).abs() < 0.001, "{kv}");
    // Half of 1.2 N/mm over 4000 mm, 1.5 Q, on the net 100 x 160 mm.
    let reaction = 1.5 * 1.2 * 4000.0 / 2.0;
    close(
        utilization(beam, "shear_notch"),
        1.5 * reaction / (0.67 * 100.0 * 160.0) / (kv * 0.8 * 4.0 / 1.3),
    );
}

#[test]
fn a_notch_in_a_side_of_a_short_post_governs_its_compression_on_the_net_section() {
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
    assert_eq!(notched_at, "net section 7000 mm² at a notch");
    // Short: kc is near 1, so the net 50 x 140 section is what decides.
    assert!(notched > plain * 1.1, "{notched} {plain}");
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
    let least = (ix + iy) / 2.0 - ((ix - iy) / 2.0).hypot(ixy);
    (area, least)
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
fn an_angle_column_buckles_about_its_weak_principal_axis_on_its_real_area() {
    // L 200 x 200 with 50 mm legs: 17 500 mm², not the 40 000 of its bounding
    // square, and weakest about the 45° axis, not about a side.
    let report = report(
        &column(
            "[(0, 0), (200, 0), (200, 50), (50, 50), (50, 200), (0, 200)]",
            "",
        ),
        &[],
    );
    let column = member(&report, "column");
    assert_eq!(column.role, "column");
    let (area, least) = least_inertia(&[[0.0, 0.0, 200.0, 50.0], [0.0, 50.0, 50.0, 150.0]], &[]);
    assert!((area - 17_500.0).abs() < 1e-6, "{area}");
    let expected = buckling(area, least);
    // Buckling of the bounding square gives 0.75 and the real area without
    // kc 1.33: both far under the real utilization.
    assert!(expected > 2.5, "{expected}");
    close(utilization(column, "compression"), expected);
    assert_eq!(column.status, "fail");
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
