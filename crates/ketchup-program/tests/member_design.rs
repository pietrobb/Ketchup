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
fn bearings_closer_than_the_beam_is_deep_hold_it_as_one() {
    // A post 20 mm before a long sill, behind a 1 m overhang of a 200 mm deep
    // beam: the overhang's moment is no lever pair over the 70 mm from the
    // post centre to the sill (that would be about 0.6 in shear).
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({\"C24\": \"C24\"})
box(\"post a\", (100, 100, 2000), at = (1000, 0, 0), material = \"C24\", grounded = True)
box(\"sill\", (1880, 100, 2000), at = (1120, 0, 0), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (4000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (4000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
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
    for member in members {
        assert!(member.utilization.is_some(), "{member:?}");
        assert_eq!(member.missing, Vec::<String>::new(), "{}", member.part);
        assert_eq!(member.status, "pass", "{member:?}");
    }
    let columns = members.iter().filter(|m| m.role == "column").count();
    assert!(columns > 50, "{columns}");
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
