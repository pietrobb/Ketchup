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

/// PG-20 (review 2026-10-24): a beam resting only on a post at its far end, or
/// hanging on one hanger, came out `pass` as a cantilever clamped there, while
/// the load path said it was not carried. Neither support takes a moment.
#[test]
fn a_beam_on_one_support_off_its_centre_tips_over_it_and_is_not_verified() {
    let one_post = report(
        "load_path(only = [\"frame\"])
self_weight([\"frame\"])
timber_design({\"C24\": \"C24\"})
box(\"post b\", (100, 100, 2000), at = (2900, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (3000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
",
        &[],
    );
    let beam = member(&one_post, "beam");
    assert_eq!(beam.status, "not_verified", "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("rests on post b alone and tips over it")),
        "{:?}",
        beam.missing
    );
    let hanger = report(
        "load_path(only = [\"frame\"])
self_weight([\"frame\"])
timber_design({\"C24\": \"C24\"})
box(\"post\", (100, 100, 2000), material = \"C24\", grounded = True)
box(\"beam\", (1500, 100, 200), at = (100, 0, 1500), material = \"C24\", tags = [\"frame\"])
joint(\"beam\", \"post\", kind = \"hanger\", bearing = True)
",
        &[],
    );
    let beam = member(&hanger, "beam");
    assert_eq!(beam.status, "not_verified", "{beam:?}");
    // Resting over its centre on one wide support it stays put.
    let centred = report(
        "load_path(only = [\"frame\"])
self_weight([\"frame\"])
timber_design({\"C24\": \"C24\"})
box(\"wall\", (1000, 100, 2000), at = (1000, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (3000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
",
        &[],
    );
    let beam = member(&centred, "beam");
    assert!(
        beam.missing.iter().all(|reason| !reason.contains("tips")),
        "{:?}",
        beam.missing
    );
}

/// PG-41 (review 2026-10-26): a 3 m beam balanced on one 100 mm post with
/// imposed load on both arms passed at 0.373 as two clamped cantilevers, from
/// the resultant of all its loads together. A person on one arm tips it: the
/// equilibrium counts every stretch beside the bearing apart (EQU), as for a
/// member on several supports.
#[test]
fn a_beam_balanced_on_one_post_tips_under_load_on_one_arm() {
    let balanced = |load: &str| {
        format!(
            "load_path(only = [\"frame\"], carriers = [\"deck\"])
{load}timber_design({{\"C24\": \"C24\"}})
self_weight([\"frame\"])
box(\"post\", (100, 100, 2000), at = (1450, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (3000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (3000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
"
        )
    };
    let tips = |report: &Report| {
        let beam = member(report, "beam");
        assert_eq!(beam.status, "not_verified", "{beam:?}");
        assert!(
            beam.missing
                .iter()
                .any(|reason| reason.contains("rests on post alone and tips over it")),
            "{:?}",
            beam.missing
        );
    };
    tips(&report(
        &balanced(
            "area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")\n",
        ),
        &[],
    ));
    // Its own weight alone, the same on both arms, still has nothing holding
    // it: 0.9 of one arm against 1.1 of the other.
    tips(&report(&balanced(""), &[]));
}

/// PG-40 (review 2026-10-26): 500 kg hung under a 100 x 160 beam on two posts
/// with no joint reached no member (PG-14b), but the beam did not know it and
/// passed at 0.054 with `ok: true`; with a joint it fails at 1.709. The weight
/// a member is only touched by makes it not verified, and the report says so.
#[test]
fn weight_that_only_touches_a_member_leaves_it_not_verified() {
    let source = |fixing: &str| {
        format!(
            "load_path(only = [\"frame\"])
material_weight(\"steel\", kg_m3 = 7850, source = \"test\")
self_weight([\"frame\", \"construction\"])
timber_design({{\"C24\": \"C24\"}})
box(\"post a\", (100, 100, 2000), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (4000, 100, 160), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"block\", (100, 637, 1000), at = (1950, 0, 1000), material = \"steel\", tags = [\"construction\"])
{fixing}"
        )
    };
    let loose = report(&source(""), &[]);
    let beam = member(&loose, "beam");
    assert_eq!(beam.status, "not_verified", "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("block touches it without a joint")
                && reason.contains("4.91 kN")),
        "{:?}",
        beam.missing
    );
    let issue = loose
        .issues
        .iter()
        .find(|issue| issue.kind == "load_unassigned")
        .unwrap_or_else(|| panic!("no load_unassigned issue: {:?}", loose.issues));
    assert_eq!(issue.severity, ketchup_program::Severity::Warning);
    assert_eq!(issue.parts, ["block", "beam"]);

    let joined = report(
        &source("joint(\"block\", \"beam\", kind = \"screw\")\n"),
        &[],
    );
    assert_eq!(member(&joined, "beam").status, "fail");
    assert_eq!(joined.statics(), ketchup_program::Statics::Failed);
    assert!(
        joined
            .issues
            .iter()
            .all(|issue| issue.kind != "load_unassigned"),
        "{:?}",
        joined.issues
    );
}

/// Review 2026-10-31 P3 (PG-40 rest): the same block hung 0.5 mm under the beam
/// touched nothing, so the beam passed as if it were not there. A loaded part
/// resting on nothing leaves every member it nearly meets not verified.
#[test]
fn weight_hanging_just_clear_of_a_member_leaves_it_not_verified() {
    let loose = report(
        "load_path(only = [\"frame\"])
material_weight(\"steel\", kg_m3 = 7850, source = \"test\")
self_weight([\"frame\", \"construction\"])
timber_design({\"C24\": \"C24\"})
box(\"post a\", (100, 100, 2000), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (4000, 100, 160), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"block\", (100, 637, 999.5), at = (1950, 0, 1000), material = \"steel\", tags = [\"construction\"])
",
        &[],
    );
    // ok counts error issues; the statics say on their own that they are not verified,
    // and the command line returns 3 for it (PG-40: rc 0 and "ok" for a beam not checked).
    assert!(loose.ok);
    assert_eq!(
        loose.statics(),
        ketchup_program::Statics::NotVerified,
        "{:?}",
        loose.design
    );
    let beam = member(&loose, "beam");
    assert_eq!(beam.status, "not_verified", "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("block lies 0.5 mm from it")
                && reason.contains("kN is not counted")),
        "{:?}",
        beam.missing
    );
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

/// A GL24h 80 x 400 beam on two grounded posts (span 5800 mm between their
/// centres) under about 13 kN on a 300 mm pad at its middle; with `battens`,
/// 50 mm battens rest on its top and on a grounded wall beside it, screwed to both.
fn slender_beam(battens: &[f64]) -> String {
    let mut source = String::from(
        "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 560, on = [\"pad\"], source = \"test\")
timber_design({\"GL24h\": \"GL24h\"})
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (100, 80, 2000), at = (5800, 0, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (5900, 80, 400), at = (0, 0, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad\", (300, 80, 50), at = (2800, 0, 2400), material = \"GL24h\", tags = [\"pad\"])
",
    );
    if !battens.is_empty() {
        source.push_str(
            "box(\"wall\", (5900, 100, 2400), at = (0, 1000, 0), material = \"GL24h\", grounded = True)\n",
        );
    }
    for (index, x) in battens.iter().enumerate() {
        source.push_str(&format!(
            "box(\"batten {index}\", (50, 1100, 50), at = ({x}, 0, 2400), material = \"GL24h\")
joint(\"batten {index}\", \"beam\", kind = \"screw\")
joint(\"batten {index}\", \"wall\", kind = \"screw\")
"
        ));
    }
    source
}

/// PG-100 (review 2026-11-01): kcrit = 1 was assumed for every beam, "braced by
/// the decking", without looking for any. A free GL24h 80 x 400 beam over 5.8 m
/// passed at 0.834, while EN 1995-1-1 6.3.3 gives kcrit about 0.78 and fails
/// it. Lateral torsional buckling that may govern leaves it not verified; held
/// along its top by battens that also rest on a wall it passes.
#[test]
fn a_slender_beam_free_along_its_top_is_not_verified_and_passes_when_braced() {
    let free = report(&slender_beam(&[]), &[]);
    let beam = member(&free, "beam");
    close(utilization(beam, "bending"), 0.834);
    assert_eq!(beam.status, "not_verified", "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("lateral torsional buckling")),
        "{:?}",
        beam.missing
    );
    assert_eq!(free.statics(), ketchup_program::Statics::NotVerified);

    let braced = report(
        &slender_beam(&[300.0, 1200.0, 2100.0, 3600.0, 4500.0, 5400.0]),
        &[],
    );
    let beam = member(&braced, "beam");
    close(utilization(beam, "bending"), 0.834);
    assert_eq!(beam.missing, Vec::<String>::new(), "{beam:?}");
    assert_eq!(beam.status, "pass");
}

/// The slender beam of [`slender_beam`] beside a second one 1000 mm away on its
/// own posts, with `extra` lines.
fn slender_pair(extra: &str) -> String {
    format!(
        "{}box(\"post c\", (100, 80, 2000), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
box(\"post d\", (100, 80, 2000), at = (5800, 1000, 0), material = \"GL24h\", grounded = True)
box(\"beam 2\", (5900, 80, 400), at = (0, 1000, 2000), material = \"GL24h\", tags = [\"frame\"])
{extra}",
        slender_beam(&[])
    )
}

fn ltb_not_verified(report: &Report) -> bool {
    member(report, "beam")
        .missing
        .iter()
        .any(|reason| reason.contains("lateral torsional buckling"))
}

/// PG-120 (review 2026-11-02): any part resting on two members braced both
/// where it touched them, whatever edge and however loose. A single plank lying
/// on two otherwise free beams let both pass, though they can buckle together
/// (EN 1995-1-1 9.2.5.3 asks for a stiff, connected bracing). PG-140 (review
/// 2026-11-04): screwed to both by joint() it still braced them, passing a
/// beam at 1.185 with 0.926, though both can still buckle together; it holds
/// the compressed top only when also fastened to a grounded wall.
#[test]
fn a_plank_on_two_free_beams_does_not_brace_them_loose_or_screwed() {
    let plank = "box(\"plank\", (100, 1080, 50), at = (2650, 0, 2400), material = \"GL24h\")\n";
    let loose = report(&slender_pair(plank), &[]);
    assert!(ltb_not_verified(&loose), "{:?}", member(&loose, "beam"));
    assert_eq!(member(&loose, "beam").status, "not_verified");

    let screwed = report(
        &slender_pair(&format!(
            "{plank}joint(\"plank\", \"beam\", kind = \"screw\")\njoint(\"plank\", \"beam 2\", kind = \"screw\")\n"
        )),
        &[],
    );
    let beam = member(&screwed, "beam");
    assert!(ltb_not_verified(&screwed), "{beam:?}");
    assert_eq!(beam.status, "not_verified", "{:?}", beam.checks);
}

/// The plank of a plank across the beam, resting on it and on a grounded wall
/// beside it and screwed to both, holds its compressed top (PG-140).
#[test]
fn a_plank_screwed_to_the_beam_and_a_grounded_wall_braces_its_top() {
    let screwed = report(
        &slender_beam(&[300.0, 1200.0, 2100.0, 3600.0, 4500.0, 5400.0]),
        &[],
    );
    let beam = member(&screwed, "beam");
    assert_eq!(beam.missing, Vec::<String>::new(), "{beam:?}");
    assert_eq!(beam.status, "pass");
    let load = screwed
        .loads
        .members
        .iter()
        .find(|load| load.part == "beam")
        .expect("loads of the beam");
    assert!(
        load.braced_by
            .iter()
            .any(|brace| brace.part == "batten 0" && brace.edge == "top"),
        "{:?}",
        load.braced_by
    );
}

/// The slender beam with two 40 x 40 laths lying along its top, from beyond
/// each end to either side of the pad, each resting on a grounded block past
/// the beam's end, and `extra` lines.
fn laths_along(extra: &str) -> String {
    format!(
        "{}box(\"block a\", (300, 80, 2400), at = (-400, 0, 0), material = \"GL24h\", grounded = True)
box(\"block b\", (300, 80, 2400), at = (6000, 0, 0), material = \"GL24h\", grounded = True)
box(\"lath a\", (3050, 40, 40), at = (-300, 20, 2400), material = \"GL24h\")
box(\"lath b\", (3050, 40, 40), at = (3150, 20, 2400), material = \"GL24h\")
{extra}",
        slender_beam(&[])
    )
}

/// PG-141 (review 2026-11-04): a part leaning on a grounded part braced a
/// member it merely lay on. Two loose laths along the beam's top, each on a
/// grounded block past an end, held it over 0-2750 and 3150-5900 mm without a
/// single joint, and the beam passed. Lying along the beam, a lath moves with it.
#[test]
fn loose_laths_along_the_top_leaning_on_grounded_blocks_do_not_brace_it() {
    let loose = report(&laths_along(""), &[]);
    let beam = member(&loose, "beam");
    assert!(ltb_not_verified(&loose), "{beam:?}");
    assert_eq!(beam.status, "not_verified");

    let anchored_only = report(
        &laths_along(
            "joint(\"lath a\", \"block a\", kind = \"screw\")\njoint(\"lath b\", \"block b\", kind = \"screw\")\n",
        ),
        &[],
    );
    assert!(
        ltb_not_verified(&anchored_only),
        "{:?}",
        member(&anchored_only, "beam")
    );
}

/// PG-142 (review 2026-11-04): where a joint's fasteners are was ignored. A
/// board 1000 mm wide across the beam, on it and on the grounded wall and
/// screwed to both, braced its whole 1000 mm of contact from one screw into
/// the beam; it holds the beam at that screw only.
#[test]
fn a_board_holds_the_beam_at_its_fasteners_only() {
    let board = report(
        &format!(
            "{}box(\"wall\", (5900, 100, 2400), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
box(\"board\", (1000, 1100, 25), at = (1000, 0, 2400), material = \"GL24h\")
joint(\"board\", \"wall\", kind = \"screw\")
joint(\"board\", \"beam\", kind = \"screw\", fasteners = [(1500, 40, 2400)])
",
            slender_beam(&[])
        ),
        &[],
    );
    let load = board
        .loads
        .members
        .iter()
        .find(|load| load.part == "beam")
        .expect("loads of the beam");
    let held: Vec<(f64, f64)> = load
        .braced_by
        .iter()
        .map(|brace| (brace.from_mm, brace.to_mm))
        .collect();
    assert_eq!(held, [(1500.0, 1500.0)], "{:?}", load.braced_by);
}

/// Held at 2700 and 3200 mm by two battens across to the wall, each free
/// stretch is 2.6 m: kcrit 0.94 within the strength.
#[test]
fn two_battens_across_at_the_pad_leave_two_shorter_free_stretches() {
    let middle = report(&slender_beam(&[2675.0, 3175.0]), &[]);
    let beam = member(&middle, "beam");
    assert_eq!(beam.status, "pass", "{beam:?}");
    let buckling = beam
        .checks
        .iter()
        .find(|check| check.name == "lateral_torsional")
        .expect("the free stretches are checked");
    assert!(
        buckling.at.starts_with("top free 100-2700 mm"),
        "{buckling:?}"
    );
}

/// PG-141 (review 2026-11-04, capability review 466): a lath along the beam,
/// screwed to it and anchored only at its own end on a block in line with
/// the beam, still braced it at every screw: with screws at 2700 / 3200 mm or
/// every 300 mm the beam passed at 1.185. Nothing across the beam holds those
/// screws sideways; the lath sways with the beam. The counter-batten on a
/// rafter leaning on a gable (review line 114) is the same case.
#[test]
fn laths_along_the_beam_anchored_in_line_do_not_brace_it_at_any_screw() {
    let anchors = "joint(\"lath a\", \"block a\", kind = \"screw\")\njoint(\"lath b\", \"block b\", kind = \"screw\")\n";
    let at = |a: f64, b: f64| {
        report(
            &laths_along(&format!(
                "{anchors}joint(\"lath a\", \"beam\", kind = \"screw\", fasteners = [({a}, 40, 2400)])
joint(\"lath b\", \"beam\", kind = \"screw\", fasteners = [({b}, 40, 2400)])
"
            )),
            &[],
        )
    };
    let unbraced = |report: &Report| {
        let beam = member(report, "beam");
        assert!(ltb_not_verified(report), "{beam:?}");
        assert_eq!(beam.status, "not_verified", "{:?}", beam.checks);
        let load = report
            .loads
            .members
            .iter()
            .find(|load| load.part == "beam")
            .expect("loads of the beam");
        assert!(
            load.braced_by
                .iter()
                .all(|brace| !brace.part.starts_with("lath")),
            "{:?}",
            load.braced_by
        );
    };
    unbraced(&at(150.0, 5750.0));
    unbraced(&at(2700.0, 3200.0));

    let screws = |from: i32, to: i32| {
        (from..=to)
            .step_by(300)
            .map(|x| format!("({x}, 40, 2400)"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let many = report(
        &laths_along(&format!(
            "{anchors}joint(\"lath a\", \"beam\", kind = \"screw\", fasteners = [{}])
joint(\"lath b\", \"beam\", kind = \"screw\", fasteners = [{}])
",
            screws(150, 2700),
            screws(3200, 5750)
        )),
        &[],
    );
    unbraced(&many);
}

fn braced_by(report: &Report) -> Vec<(String, f64, f64)> {
    report
        .loads
        .members
        .iter()
        .find(|load| load.part == "beam")
        .expect("loads of the beam")
        .braced_by
        .iter()
        .map(|brace| (brace.part.clone(), brace.from_mm, brace.to_mm))
        .collect()
}

fn stray_fastener_parts(report: &Report) -> Vec<Vec<String>> {
    report
        .issues
        .iter()
        .filter(|issue| issue.kind == "fastener_off_contact")
        .map(|issue| issue.parts.clone())
        .collect()
}

/// The slender beam with one 50 mm batten across it at x = 500 to the grounded
/// wall, screwed to the wall, and `beam_joint` joining it to the beam.
fn one_batten(beam_joint: &str, wall_joint: &str) -> String {
    format!(
        "{}box(\"wall\", (5900, 100, 2400), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
box(\"batten\", (50, 1100, 50), at = (500, 0, 2400), material = \"GL24h\")
joint(\"batten\", \"wall\", kind = \"screw\"{wall_joint})
joint(\"batten\", \"beam\", kind = \"screw\"{beam_joint})
",
        slender_beam(&[])
    )
}

/// PG-160 (review 2026-11-04 r19): a joint's fastener points were taken
/// wherever they were. One batten across the beam at x = 500, its joint to the
/// beam naming screws every 300 mm 5 m away (or along the beam's top where the
/// batten is not), held the beam every 300 mm and it passed at 1.185. A
/// fastener holds only where the parts it joins touch; one elsewhere is reported.
#[test]
fn fasteners_away_from_where_the_parts_touch_hold_nothing_and_are_reported() {
    let far = report(
        &one_batten(
            ", fasteners = [(x, -5000, 0) for x in range(300, 5800, 300)]",
            "",
        ),
        &[],
    );
    assert!(ltb_not_verified(&far), "{:?}", member(&far, "beam"));
    assert_eq!(braced_by(&far), Vec::new());
    assert_eq!(stray_fastener_parts(&far), [["batten", "beam"]]);

    let along_top = report(
        &one_batten(
            ", fasteners = [(x, 40, 2400) for x in range(300, 1501, 300)]",
            "",
        ),
        &[],
    );
    assert!(
        ltb_not_verified(&along_top),
        "{:?}",
        member(&along_top, "beam")
    );
    assert_eq!(braced_by(&along_top), Vec::new());
    assert_eq!(stray_fastener_parts(&along_top), [["batten", "beam"]]);

    // The anchorage side too: a screw into the wall 5 m away anchors nothing.
    let far_anchor = report(
        &one_batten(
            ", fasteners = [(525, 40, 2400)]",
            ", fasteners = [(525, -5000, 0)]",
        ),
        &[],
    );
    assert_eq!(braced_by(&far_anchor), Vec::new());
    assert_eq!(stray_fastener_parts(&far_anchor), [["batten", "wall"]]);

    // A screw in the batten's contact, given at the contact or at its head on
    // the batten's top, holds the beam there.
    for z in [2400, 2450] {
        let held = report(
            &one_batten(
                &format!(", fasteners = [(525, 40, {z})]"),
                ", fasteners = [(525, 1050, 2400)]",
            ),
            &[],
        );
        assert_eq!(
            braced_by(&held),
            [("batten".to_owned(), 525.0, 525.0)],
            "{:?}",
            held.issues
        );
        assert_eq!(stray_fastener_parts(&held), Vec::<Vec<String>>::new());
    }
}

/// PG-161 (review 2026-11-04 r19): the contacts of a part with one supporter
/// merged into one cloud whose middle was the anchorage. A 6.7 m lath along
/// the beam's top resting on both arms of one grounded U past the beam's
/// ends was anchored at mid-span, right at its screw into the beam, and the
/// beam passed at 0.908 (really 1.185): the lath sways with the beam. A
/// masonry shell modelled as one body with a counter-batten on gable and eaves
/// wall is this case.
#[test]
fn a_lath_along_the_beam_on_both_arms_of_one_grounded_u_does_not_brace_it() {
    let source = format!(
        "{}box(\"u\", (6700, 80, 2400), at = (-400, 200, 0), material = \"GL24h\", grounded = True)
box(\"u arm a\", (300, 280, 2400), at = (-400, 0, 0), material = \"GL24h\")
box(\"u arm b\", (300, 280, 2400), at = (6000, 0, 0), material = \"GL24h\")
union(\"u\", \"u arm a\")
union(\"u\", \"u arm b\")
box(\"lath\", (6700, 40, 40), at = (-400, 20, 2400), material = \"GL24h\")
joint(\"lath\", \"u\", kind = \"screw\")
joint(\"lath\", \"beam\", kind = \"screw\", fasteners = [(2950, 40, 2400)])
",
        slender_beam(&[])
    );
    let lath = report(&source, &[]);
    let beam = member(&lath, "beam");
    assert!(ltb_not_verified(&lath), "{beam:?}");
    assert_eq!(beam.status, "not_verified", "{:?}", beam.checks);
    assert_eq!(braced_by(&lath), Vec::new());
}

/// PG-162 (review 2026-11-04 r19): any joint() counted as fastening, whatever
/// its kind. Battens joined to the beam and the wall by joint(kind="none")
/// braced it like screws. none, contact and friction fasten nothing.
#[test]
fn joints_of_a_kind_that_fastens_nothing_do_not_brace() {
    let battens = slender_beam(&[300.0, 1200.0, 2100.0, 3600.0, 4500.0, 5400.0]);
    for kind in ["none", "contact", "friction"] {
        let loose = report(
            &battens.replace("kind = \"screw\"", &format!("kind = \"{kind}\"")),
            &[],
        );
        let beam = member(&loose, "beam");
        assert!(ltb_not_verified(&loose), "{kind}: {beam:?}");
        assert_eq!(braced_by(&loose), Vec::new(), "{kind}");
    }
}

/// Review 2026-11-04 r19 P3: a part of a load path's `fixed` tags counted as
/// fastened to whatever it touched, so loose battens tagged fixed braced the
/// beam. A fixed tag passes weight; only a joint() fastens a brace.
#[test]
fn battens_of_the_fixed_tags_without_joints_do_not_brace() {
    let mut source = slender_beam(&[]).replace(
        "carriers = [\"pad\"])",
        "carriers = [\"pad\"], fixed = [\"batten\"])",
    );
    source.push_str(
        "box(\"wall\", (5900, 100, 2400), at = (0, 1000, 0), material = \"GL24h\", grounded = True)\n",
    );
    for x in [300, 1200, 2100, 3600, 4500, 5400] {
        source.push_str(&format!(
            "box(\"batten {x}\", (50, 1100, 50), at = ({x}, 0, 2400), material = \"GL24h\", tags = [\"batten\"])\n"
        ));
    }
    let tagged = report(&source, &[]);
    assert!(ltb_not_verified(&tagged), "{:?}", member(&tagged, "beam"));
    assert_eq!(braced_by(&tagged), Vec::new());
}

/// Review 2026-11-04 r19 P3: an area load on a pad narrower than it is tall
/// found no face up, so the pad and the beam under it carried nothing and
/// nothing said so. It is reported.
#[test]
fn an_area_load_on_a_part_standing_on_edge_is_reported() {
    let source = slender_beam(&[]).replace(
        "box(\"pad\", (300, 80, 50), at = (2800, 0, 2400)",
        "box(\"pad\", (300, 40, 50), at = (2800, 20, 2400)",
    );
    let edge_on = report(&source, &[]);
    let issue = edge_on
        .issues
        .iter()
        .find(|issue| issue.kind == "area_load_edge_on")
        .unwrap_or_else(|| panic!("{:?}", edge_on.issues));
    assert_eq!(issue.parts, ["pad"]);
    assert!(issue.message.contains("point"), "{}", issue.message);
}

/// PG-163 (review 2026-11-04 r19): any part standing on the floor counted as
/// an anchorage. Battens screwed to the beam and to free 40 x 40 studs
/// standing on the floor braced it, though a hinged stud holds nothing
/// sideways. Only a part declared grounded anchors a brace.
#[test]
fn battens_screwed_to_free_studs_on_the_floor_do_not_brace() {
    let mut source = slender_beam(&[]);
    for x in [300, 1200, 2100, 3600, 4500, 5400] {
        source.push_str(&format!(
            "box(\"stud {x}\", (40, 40, 2400), at = ({x}, 1000, 0), material = \"GL24h\")
box(\"batten {x}\", (50, 1040, 50), at = ({x}, 0, 2400), material = \"GL24h\")
joint(\"batten {x}\", \"beam\", kind = \"screw\")
joint(\"batten {x}\", \"stud {x}\", kind = \"screw\")
"
        ));
    }
    let studs = report(&source, &[]);
    let beam = member(&studs, "beam");
    assert!(ltb_not_verified(&studs), "{beam:?}");
    assert_eq!(braced_by(&studs), Vec::new());
}

/// PG-120 (review 2026-11-02): a ceiling board screwed under two beams held
/// their tension edge, yet counted as bracing the compressed top: a GL24h
/// 80 x 400 beam over 5.8 m at 1.185 with kcrit passed.
#[test]
fn a_ceiling_screwed_under_the_beams_does_not_brace_their_compressed_top() {
    let ceiling = report(
        &slender_pair(
            "box(\"ceiling\", (5500, 1080, 20), at = (200, 0, 1980), material = \"OSB\")
joint(\"ceiling\", \"beam\", kind = \"screw\")
joint(\"ceiling\", \"beam 2\", kind = \"screw\")
",
        ),
        &[],
    );
    assert!(ltb_not_verified(&ceiling), "{:?}", member(&ceiling, "beam"));
    assert_eq!(member(&ceiling, "beam").status, "not_verified");
}

/// A GL24h 72 x 400 beam over two spans of 5.8 m on three grounded posts, with
/// a pad loaded at the middle of each span (a wider post in the middle) and battens screwed along its top
/// every 900 mm, and `extra` lines.
fn two_span_beam(extra: &str) -> String {
    let mut source = String::from(
        "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"point\", kind = \"imposed\", kn_m2 = 800, on = [\"pad\"], source = \"test\")
timber_design({\"GL24h\": \"GL24h\"})
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (400, 80, 2000), at = (5650, 0, 0), material = \"GL24h\", grounded = True)
box(\"post c\", (100, 80, 2000), at = (11600, 0, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (11700, 72, 400), at = (0, 4, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad a\", (300, 72, 50), at = (2800, 4, 2400), material = \"GL24h\", tags = [\"pad\"])
box(\"pad b\", (300, 72, 50), at = (8600, 4, 2400), material = \"GL24h\", tags = [\"pad\"])
box(\"wall\", (11700, 100, 2400), at = (0, 1000, 0), material = \"GL24h\", grounded = True)
",
    );
    for step in 0..13 {
        // Each batten holds the beam at its middle (PG-142); the one over a pad
        // goes just before it.
        let x = match 300 + 900 * step {
            3000 => 2740,
            x => x,
        };
        source.push_str(&format!(
            "box(\"batten {x}\", (50, 1100, 50), at = ({x}, 0, 2400), material = \"GL24h\")
joint(\"batten {x}\", \"beam\", kind = \"screw\")
joint(\"batten {x}\", \"wall\", kind = \"screw\")
"
        ));
    }
    source.push_str(extra);
    source
}

/// PG-120 (review 2026-11-02): battens on a beam's top held its bottom too,
/// which a hogging moment over a middle post compresses. Only a ceiling
/// screwed underneath holds that edge.
#[test]
fn battens_on_top_do_not_brace_the_bottom_a_hogging_moment_compresses() {
    let top_only = report(&two_span_beam(""), &[]);
    let beam = member(&top_only, "beam");
    assert!(ltb_not_verified(&top_only), "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("compressed bottom")),
        "{:?}",
        beam.missing
    );

    // Screwed to the beam every 300 mm (PG-142: it holds the beam at its screws).
    let screws = |from: i32, to: i32| {
        (from..=to)
            .step_by(300)
            .map(|x| format!("({x}, 40, 2000)"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let ceiling = report(
        &two_span_beam(&format!(
            "box(\"ceiling a\", (5400, 1260, 20), at = (200, -260, 1980), material = \"OSB\")
box(\"ceiling b\", (5400, 1260, 20), at = (6100, -260, 1980), material = \"OSB\")
joint(\"ceiling a\", \"beam\", kind = \"screw\", fasteners = [{}])
joint(\"ceiling a\", \"wall\", kind = \"screw\")
joint(\"ceiling b\", \"beam\", kind = \"screw\", fasteners = [{}])
joint(\"ceiling b\", \"wall\", kind = \"screw\")
",
            screws(250, 5550),
            screws(6150, 11450)
        )),
        &[],
    );
    let beam = member(&ceiling, "beam");
    assert_eq!(beam.missing, Vec::<String>::new(), "{beam:?}");
    assert_eq!(beam.status, "pass", "{:?}", beam.checks);
}

/// PG-105 (review 2026-11-01): loads on load-path members without
/// timber_design() came out with statics "none", which says there are no
/// members. Each member is listed as not verified for want of a strength class.
#[test]
fn load_path_members_without_timber_design_are_not_verified() {
    let source = beam("C24", 100.0, 200.0).replace("timber_design({\"C24\": \"C24\"})\n", "");
    let unclassed = report(&source, &[]);
    assert_eq!(unclassed.statics(), ketchup_program::Statics::NotVerified);
    let beam = member(&unclassed, "beam");
    assert_eq!(beam.status, "not_verified");
    assert_eq!(
        beam.missing,
        ["material \"C24\" has no timber strength class (timber_strength)"]
    );
}

/// PG-105 (review 2026-11-02, the rest): load_path() and timber_design()
/// without any declared load came out with statics "none" and rc 0, as if
/// there were nothing to check. Each member is not verified for want of loads.
#[test]
fn timber_design_without_any_load_is_not_verified() {
    let source = beam("C24", 100.0, 200.0).replace(
        "area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")\n",
        "",
    );
    let unloaded = report(&source, &[]);
    assert_eq!(unloaded.statics(), ketchup_program::Statics::NotVerified);
    let beam = member(&unloaded, "beam");
    assert_eq!(beam.status, "not_verified");
    assert!(
        beam.missing.iter().any(|reason| reason.contains("no load")),
        "{:?}",
        beam.missing
    );
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

#[test]
fn a_beam_notched_on_its_top_over_its_bearing_is_checked_in_shear_with_kv_1() {
    // Review 2026-10-24 PG-25: the beam of `beam` notched 40 mm deep over
    // the first 150 mm on its top, the side away from the post, instead of
    // its underside. EC5 6.5.2(2), fig. 6.11(b): kv = 1 there; kv of a notch
    // on the post's side was taken. The deck lies on both from 150 mm on.
    let source = |profile: &str, post_a: f64| {
        format!(
            "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
box(\"post a\", (100, 100, {post_a}), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
part = extrude(\"beam\", profile = {profile}, distance = 100, material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 100, 2000), z = (0, -1, 0), x = (1, 0, 0))
box(\"deck\", (3850, 600, 20), at = (150, -250, 2200), material = \"OSB\", tags = [\"deck\"])
"
        )
    };
    let under = report(
        &source(
            "[(0, 40), (150, 40), (150, 0), (4000, 0), (4000, 200), (0, 200)]",
            2040.0,
        ),
        &[],
    );
    let on_top = report(
        &source(
            "[(0, 0), (4000, 0), (4000, 200), (150, 200), (150, 160), (0, 160)]",
            2000.0,
        ),
        &[],
    );
    // The same reaction on the same net 100 x 160 mm; kv of the notch under
    // it as in a_beam_notched_over_its_bearing_is_checked_in_shear_with_kv.
    let (h, alpha, x) = (200.0f64, 0.8f64, 100.0);
    let kv = 5.0
        / (h.sqrt()
            * ((alpha * (1.0 - alpha)).sqrt()
                + 0.8 * x / h * (1.0 / alpha - alpha * alpha).sqrt()));
    close(
        utilization(member(&on_top, "beam"), "shear_notch"),
        utilization(member(&under, "beam"), "shear_notch") * kv,
    );
}

#[test]
fn a_beam_notched_on_its_top_over_an_inner_support_is_checked_with_the_kv_of_its_tension_side() {
    // Review 2026-10-26 PG-44: a beam continuous over a middle post, notched
    // 40 mm deep over it on its top. The moment there hogs, so the top is in
    // tension: fig. 6.11(b), kv = 1, is a simply supported end. The top notch
    // got kv = 1 (0.374) where the same notch under it got 0.671.
    let source = |profile: &str, post_b: f64| {
        format!(
            "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"finish\", kind = \"permanent\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
box(\"post a\", (100, 100, 2000), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, {post_b}), at = (1950, 0, 0), material = \"C24\", grounded = True)
box(\"post c\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
part = extrude(\"beam\", profile = {profile}, distance = 100, material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 100, 2000), z = (0, -1, 0), x = (1, 0, 0))
box(\"deck a\", (1900, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
box(\"deck c\", (1900, 600, 20), at = (2100, -250, 2200), material = \"OSB\", tags = [\"deck\"])
"
        )
    };
    let under = report(
        &source(
            "[(0, 0), (1900, 0), (1900, 40), (2100, 40), (2100, 0), (4000, 0), (4000, 200), (0, 200)]",
            2040.0,
        ),
        &[],
    );
    let on_top = report(
        &source(
            "[(0, 0), (4000, 0), (4000, 200), (2100, 200), (2100, 160), (1900, 160), (1900, 200), (0, 200)]",
            2000.0,
        ),
        &[],
    );
    let under = utilization(member(&under, "beam"), "shear_notch");
    assert!(under > 0.0, "{under}");
    close(utilization(member(&on_top, "beam"), "shear_notch"), under);
}

#[test]
fn a_beam_notched_over_a_bearing_longer_than_400_mm_is_not_verified() {
    // Review 2026-10-19 P3: the beam of `beam` 100 x 200 over two spans, its
    // underside notched 40 mm deep over 1700..2300 mm, where a wall 500 mm
    // long reaches up into the notch. A bearing that long supports the beam
    // along it, and 6.5.2 was skipped without a word, so the beam passed with
    // its notch unchecked. A permanent load alone holds the end posts down.
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"finish\", kind = \"permanent\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({\"C24\": \"C24\"})
box(\"post a\", (100, 100, 2000), material = \"C24\", grounded = True)
box(\"wall b\", (500, 100, 2040), at = (1750, 0, 0), material = \"C24\", grounded = True)
box(\"post c\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
part = extrude(\"beam\", profile = [(0, 0), (1700, 0), (1700, 40), (2300, 40), (2300, 0), (4000, 0), (4000, 200), (0, 200)],
               distance = 100, material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 100, 2000), z = (0, -1, 0), x = (1, 0, 0))
box(\"deck\", (4000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
",
        &[],
    );
    let beam = member(&report, "beam");
    assert!(beam.checks.iter().all(|c| c.name != "shear_notch"));
    assert_eq!(
        beam.missing,
        [
            "it is notched over wall b, a bearing longer than 400 mm, where the line of its reaction is not known: the notch is not checked (EN 1995-1-1 6.5.2)"
        ]
    );
    assert_eq!(beam.status, "not_verified", "{:?}", beam.checks);
}

#[test]
fn a_beam_mirrored_into_its_shape_is_checked_as_the_same_beam_drawn() {
    // Review 2026-10-19 PG-13: the beam notched over post a, under a deck of
    // 8 kN/m² on its half next to post a, drawn with its notch at the other
    // end or on its top and mirrored into place. The check took the profile
    // as drawn, so it found the notch over post b, which carries a quarter of
    // the load, and passed what the beam drawn in place fails.
    let source = |profile: &str, then: &str| {
        format!(
            "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 8.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
box(\"post a\", (100, 100, 2040), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
part = extrude(\"beam\", profile = {profile}, distance = 100, material = \"C24\", tags = [\"frame\"])
{then}
place(part, origin = (0, 100, 2000), z = (0, -1, 0), x = (1, 0, 0))
box(\"deck\", (2000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
"
        )
    };
    let checks = |profile: &str, then: &str| {
        let report = report(&source(profile, then), &[]);
        let member = member(&report, "beam").clone();
        assert_eq!(member.role, "beam", "{then}");
        assert_eq!(member.missing, Vec::<String>::new(), "{then}");
        member
    };
    let drawn = checks(
        "[(0, 40), (150, 40), (150, 0), (4000, 0), (4000, 200), (0, 200)]",
        "",
    );
    assert_eq!(drawn.status, "fail");
    assert!(utilization(&drawn, "shear_notch") > 1.0);
    let mirrored = [
        (
            "[(0, 0), (3850, 0), (3850, 40), (4000, 40), (4000, 200), (0, 200)]",
            "mirror(part, axis = \"x\")",
        ),
        (
            "[(0, 0), (4000, 0), (4000, 200), (150, 200), (150, 160), (0, 160)]",
            "mirror(part, axis = \"y\")",
        ),
    ];
    for (profile, then) in mirrored {
        let member = checks(profile, then);
        for check in &drawn.checks {
            close(utilization(&member, check.name), check.utilization);
            let at = member.checks.iter().find(|c| c.name == check.name);
            assert_eq!(at.map(|c| &c.at), Some(&check.at), "{then}");
        }
        assert_eq!(member.status, drawn.status, "{then}");
    }
}

#[test]
fn a_beam_drawn_from_the_side_bends_on_its_notched_section_at_mid_span() {
    // Review 2026-10-18 PG-8: the beam of `beam` 100 x 200 drawn as its side
    // profile, its underside notched 100 mm deep over 1950..2050 mm, where the
    // moment is largest. It bends there on 100 x 100 mm, not on the 200 mm
    // depth it has elsewhere (on the full depth: 0.35, pass).
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({\"C24\": \"C24\"})
box(\"post a\", (100, 100, 2000), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
part = extrude(\"beam\", profile = [(0, 0), (1950, 0), (1950, 100), (2050, 100), (2050, 0), (4000, 0), (4000, 200), (0, 200)],
               distance = 100, material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 100, 2000), z = (0, -1, 0), x = (1, 0, 0))
box(\"deck\", (4000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
",
        &[],
    );
    let beam = member(&report, "beam");
    assert_eq!(beam.role, "beam");
    // The notch in the side in tension leaves its stress concentration unchecked (PG-15).
    assert_eq!(beam.missing.len(), 1, "{:?}", beam.missing);
    // kh is of the member's depth, 200 mm, not of the notched 100 mm section
    // (EC5 3.2(3); review 2026-10-19 P3): (150 / 200)^0.2 < 1, so 1.
    let kh = 1.0;
    let moment = 1.5 * 1.2 * 3900f64.powi(2) / 8.0;
    close(
        utilization(beam, "bending"),
        moment / (100.0 * 100.0 * 100.0 / 6.0) / (0.8 * 24.0 / 1.3 * kh),
    );
    assert_eq!(beam.status, "fail");
}

#[test]
fn a_notch_in_the_side_in_tension_away_from_the_bearings_is_not_verified() {
    // Review 2026-10-19 PG-15: the beam of the test above at 1 kN/m², its
    // underside notched 100 mm deep at mid-span, passed on the net section
    // with 0.641 (0.695 since kh is of the member's 200 mm depth). EC5
    // 6.5.1(2) ignores the stress concentration of a right-angled notch only
    // on the side in compression; on the top it may.
    let source = |profile: &str| {
        format!(
            "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 1.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
box(\"post a\", (100, 100, 2000), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
part = extrude(\"beam\", profile = {profile}, distance = 100, material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 100, 2000), z = (0, -1, 0), x = (1, 0, 0))
box(\"deck\", (4000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
"
        )
    };
    let below = report(
        &source(
            "[(0, 0), (1950, 0), (1950, 100), (2050, 100), (2050, 0), (4000, 0), (4000, 200), (0, 200)]",
        ),
        &[],
    );
    let below = member(&below, "beam");
    close(utilization(below, "bending"), 0.695);
    assert_eq!(below.status, "not_verified", "{:?}", below.checks);
    assert_eq!(
        below.missing,
        [
            "its depth changes more steeply than 1:10 on its underside, which is in tension, at 1950 mm, away from its bearings: the stress concentration there is not checked (EN 1995-1-1 6.5.1(2))"
        ]
    );
    let above = report(
        &source(
            "[(0, 0), (4000, 0), (4000, 200), (2050, 200), (2050, 100), (1950, 100), (1950, 200), (0, 200)]",
        ),
        &[],
    );
    let above = member(&above, "beam");
    assert_eq!(above.missing, Vec::<String>::new());
    assert_eq!(above.status, "pass", "{:?}", above.checks);
}

/// A 100 x 200 C24 purlin 4000 mm long along world X turned 30° about its
/// grain, its 100 mm underside on two posts cut to that slope (centres 50 and
/// 3950 mm) and a deck on its top face, 10 kN/m²; `beam` builds it.
fn turned_purlin(beam: &str) -> String {
    format!(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 10.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
s, c = 0.5, 0.8660254037844386
for x in (0, 3900):
    post = extrude(\"post %d\" % x, profile = [(0, 0), (100 * c, 0), (100 * c, 2000 + 100 * s), (0, 2000)],
                   distance = 100, material = \"C24\", grounded = True)
    place(post, origin = (x, 0, 0), x = (0, 1, 0), z = (1, 0, 0))
{beam}
deck = box(\"deck\", (4000, 100, 20), material = \"OSB\", tags = [\"deck\"])
place(deck, origin = (0, -200 * s, 2000 + 200 * c), x = (1, 0, 0), z = (0, -s, c))
"
    )
}

#[test]
fn a_beam_turned_about_its_grain_by_place_is_checked_as_the_same_beam_drawn_turned() {
    // Review 2026-10-18 PG-9: the vertical load on a section turned 30° about
    // the grain bends it about both its axes. Drawn turned in its profile the
    // check sees that; turned by place(), as a profile or as a box, it took the
    // load along the section's own depth (bending 0.555 against 0.975).
    let drawn = "part = extrude(\"beam\", profile = [(0, 0), (100 * c, 100 * s), (100 * c - 200 * s, 100 * s + 200 * c), (-200 * s, 200 * c)],
               distance = 4000, material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 0, 2000), z = (1, 0, 0), x = (0, 1, 0))";
    let placed = "part = extrude(\"beam\", profile = [(0, 0), (100, 0), (100, 200), (0, 200)],
               distance = 4000, material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 0, 2000), z = (1, 0, 0), x = (0, c, s))";
    let boxed = "part = box(\"beam\", (4000, 100, 200), material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 0, 2000), x = (1, 0, 0), z = (0, -s, c))";
    let checks = |beam: &str| {
        let report = report(&turned_purlin(beam), &[]);
        let member = member(&report, "beam").clone();
        assert_eq!(member.role, "beam", "{beam}");
        assert_eq!(member.missing, Vec::<String>::new(), "{beam}");
        member
    };
    let reference = checks(drawn);
    // Free bending of the turned rectangle: M (cos 30 / W_strong + sin 30 / W_weak),
    // 1.87 times M / W_strong; q = 10 kN/m² over the deck's plan width 100 cos 30.
    let (s, c) = (0.5, 0.75f64.sqrt());
    let moment = 1.5 * 0.01 * 100.0 * c * 3900f64.powi(2) / 8.0;
    let per_moment = c / (100.0 * 200.0 * 200.0 / 6.0) + s / (200.0 * 100.0 * 100.0 / 6.0);
    close(
        utilization(&reference, "bending"),
        moment * per_moment / (0.8 * 24.0 / 1.3),
    );
    for beam in [placed, boxed] {
        let turned = checks(beam);
        for check in ["bending", "shear", "deflection_inst", "deflection_fin"] {
            close(utilization(&turned, check), utilization(&reference, check));
        }
        assert_eq!(turned.status, reference.status, "{beam}");
    }
}

#[test]
fn a_turned_beam_deflects_by_the_resultant_of_its_vertical_and_sideways_bending() {
    // Review 2026-10-19 PG-16: the purlin turned 30° deflects sideways 0.742
    // times as far as it sinks, so the deflection is 1.245 times the vertical
    // part that alone was checked.
    let report = report(
        &turned_purlin(
            "part = box(\"beam\", (4000, 100, 200), material = \"C24\", tags = [\"frame\"])
place(part, origin = (0, 0, 2000), x = (1, 0, 0), z = (0, -s, c))",
        ),
        &[],
    );
    let beam = member(&report, "beam");
    assert_eq!(beam.missing, Vec::<String>::new());
    // The load along the principal axes: q cos 30 bends the strong one, q sin 30 the weak.
    let (s, c) = (0.5, 0.75f64.sqrt());
    let (q, l) = (0.01 * 100.0 * c, 3900f64);
    let (strong, weak) = (100.0 * 200f64.powi(3) / 12.0, 200.0 * 100f64.powi(3) / 12.0);
    let per_inertia = (c / strong).hypot(s / weak);
    let vertical = c * c / strong + s * s / weak;
    close(per_inertia / vertical, 1.245);
    let w = 5.0 * q * l.powi(4) / (384.0 * 11000.0) * per_inertia;
    close(utilization(beam, "deflection_inst"), w / (l / 300.0));
    close(
        utilization(beam, "deflection_fin"),
        w * (1.0 + 0.3 * 0.6) / (l / 250.0),
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
    // It sinks as if by Ix - Ixy² / Iy and moves sideways Ixy / Iy times as
    // far (review 2026-10-19 PG-16).
    let w = 5.0 * q * l.powi(4) / (384.0 * 11000.0 * det / iy) * (ixy / iy).hypot(1.0);
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

#[test]
fn a_post_leaning_26_degrees_carries_the_vertical_load_along_its_grain() {
    // The 60 x 140 x 500 post of `post` leant 26° in x, on a slab and under a
    // deck that both lie square to its ends. Pinned at both ends, the vertical
    // load V on the deck reaches the post as N = V / cos 26° along it, 11 %
    // more than V. The area load is per plan area: V = 1.5 x 2 N/mm² x
    // 60 cos 26° x 140, so N is that of the upright post.
    let report = report(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"heavy\", kind = \"imposed\", kn_m2 = 2000.0, on = [\"deck\"], source = \"test\")
timber_design({\"C24\": \"C24\"})
s, c = 0.4383711467890774, 0.898794046299167
grain, across = (s, 0, c), (c, 0, -s)
def at(x, y, z):
    return (x * c + z * s, y, 50 - x * s + z * c)
slab = box(\"slab\", (300, 300, 50), at = (0, 0, 0), material = \"C24\", grounded = True)
place(slab, origin = at(-100, -80, -50), z = grain, x = across)
post = box(\"post\", (60, 140, 500), at = (0, 0, 0), material = \"C24\", tags = [\"frame\"])
place(post, origin = at(0, 0, 0), z = grain, x = across)
deck = box(\"deck\", (60, 140, 20), at = (0, 0, 0), material = \"OSB\", tags = [\"deck\"])
place(deck, origin = at(0, 0, 500), z = grain, x = across)
",
        &[],
    );
    let post = member(&report, "post");
    assert_eq!(post.role, "column");
    let lambda = 500.0 * 12f64.sqrt() / 60.0 / std::f64::consts::PI * (21.0f64 / 7400.0).sqrt();
    let k = 0.5 * (1.0 + 0.2 * (lambda - 0.3) + lambda * lambda);
    let kc = 1.0 / (k + (k * k - lambda * lambda).sqrt());
    let vertical = 1.5 * 2000.0e-3 * 60.0 * 26f64.to_radians().cos() * 140.0;
    let along = vertical / 26f64.to_radians().cos();
    close(
        utilization(post, "compression"),
        along / (60.0 * 140.0) / (kc * 0.8 * 21.0 / 1.3),
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
#[ignore = "large-house: runs in full suite"]
fn large_house_every_member_of_the_house_has_a_utilization_and_passes_once_the_snow_is_given() {
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

/// Review 2026-11-01 PG-106: kmod, kdef and gamma_M came from the rows of solid timber
/// for every material, so an OSB/3 member had kmod 0.6 for permanent loads instead of 0.4.
/// They come from the product the class names (EN 1995-1-1 tables 2.3, 3.1 and 3.2).
#[test]
fn a_panel_product_takes_its_own_kmod_kdef_and_gamma_m() {
    let osb = |service_class: u32| {
        beam("panel", 100.0, 200.0).replace(
            "timber_design({\"C24\": \"C24\"})",
            &format!(
                "timber_design({{\"C24\": \"C24\"}})\n\
                 timber_strength(\"panel\", strength_class = \"OSB/3\", \
                 service_class = {service_class}, glulam = False, product = \"osb3\", \
                 fm_k = 24, fv_k = 4.0, fc0_k = 21, fc90_k = 2.5, e0_mean = 11000, \
                 e0_05 = 7400, source = \"test\")"
            ),
        )
    };
    let named = report(&osb(1), &[]);
    let panel = member(&named, "beam");
    // As the C24 beam of the hand calculation, with OSB/3 in service class 1: kmod 0.7 for
    // the medium-term imposed load and gamma_M 1.2.
    let (q, l) = (1.2, 3900.0);
    let moment = 1.5 * q * l * l / 8.0;
    close(
        utilization(panel, "bending"),
        moment / (100.0 * 200.0 * 200.0 / 6.0) / (0.7 * 24.0 / 1.2),
    );
    // OSB/3 is not used in service class 3.
    let error = run("design.star", &osb(3), &BTreeMap::new()).unwrap_err();
    assert!(
        format!("{error:#}").contains("service class 3"),
        "{error:#}"
    );

    // Review 2026-11-02 PG-106: without product= it took solid timber's factors
    // and passed; a class naming no product leaves its members not verified.
    let unnamed = report(&osb(1).replace(" product = \"osb3\",", ""), &[]);
    let panel = member(&unnamed, "beam");
    assert_eq!(panel.status, "not_verified", "{panel:?}");
    assert!(
        panel
            .missing
            .iter()
            .any(|reason| reason.contains("product")),
        "{:?}",
        panel.missing
    );
    // The library's classes name theirs.
    let c24 = report(&beam("C24", 100.0, 200.0), &[]);
    assert_eq!(member(&c24, "beam").status, "pass");
}
