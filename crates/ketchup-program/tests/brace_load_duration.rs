//! Review 2026-11-25 R1125og-PG-1001 and PG-1004: the braces of a member buckle
//! at the design strength of each ultimate combination, and a brace of a
//! material naming no product has no known k_mod.

use ketchup_program::member_check::MemberCheck;
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("design.star", source, &BTreeMap::new())
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

/// A GL24h 80 x 400 beam over 5.8 m under `permanent` and `imposed` kN/m² on a
/// pad at mid-span, its top held by six strips `side` x `side` of `material`
/// screwed to it and to a grounded wall.
fn braced_beam(permanent: f64, imposed: f64, side: f64, material: &str) -> String {
    let mut source = format!(
        "load_path(only = [\"frame\"], carriers = [\"pad\"])
area_load(\"pg\", kind = \"permanent\", kn_m2 = {permanent}, on = [\"pad\"], source = \"test\")
area_load(\"pq\", kind = \"imposed\", kn_m2 = {imposed}, on = [\"pad\"], source = \"test\")
timber_design({{\"GL24h\": \"GL24h\", \"C24\": \"C24\"}})
timber_strength(\"bare C24\", strength_class = \"C24\", service_class = 1, glulam = False, fm_k = 24, fv_k = 4.0, fc0_k = 21, fc90_k = 2.5, e0_mean = 11000, e0_05 = 7400, source = \"test\")
box(\"post a\", (100, 80, 2000), material = \"GL24h\", grounded = True)
box(\"post b\", (100, 80, 2000), at = (5800, 0, 0), material = \"GL24h\", grounded = True)
box(\"beam\", (5900, 80, 400), at = (0, 0, 2000), material = \"GL24h\", tags = [\"frame\"])
box(\"pad\", (300, 80, 50), at = (2800, 0, 2400), material = \"GL24h\", tags = [\"pad\"])
box(\"wall\", (5900, 600, 2400), at = (0, 900, 0), material = \"GL24h\", grounded = True)
brace([\"beam\"], by = [\"strip\"], edge = \"top\")
"
    );
    for x in [300, 1200, 2100, 3600, 4500, 5400] {
        source.push_str(&format!(
            "box(\"strip {x}\", ({side}, 1500, {side}), at = ({}, 0, 2400), material = \"{material}\", tags = [\"strip\"])
joint(\"strip {x}\", \"beam\", kind = \"screw\", fasteners = [({x}, 40, 2400)])
joint(\"strip {x}\", \"wall\", kind = \"screw\", fasteners = [({x}, 1480, 2400)])
",
            f64::from(x) - side / 2.0
        ));
    }
    source
}

/// R1125og-PG-1001 (= R1125lo-1): k_mod came from the combination of the
/// largest moment only. Under the permanent load alone C24 strips 20 x 20
/// buckle at 207.133 N (k_mod 0.6) against F_d = 228.454 N and the beam is not
/// verified; adding a small imposed load made 1.35 G + 1.5 Q govern the moment
/// with k_mod 0.8 (276.177 N), and the beam passed, though 1.35 G alone still
/// asks 228.454 N of strips good for 207.133 N at its k_mod.
#[test]
fn a_brace_buckles_under_every_combination_at_its_own_k_mod() {
    for imposed in [0.0, 60.0] {
        let checked = report(&braced_beam(540.0, imposed, 20.0, "C24"));
        let beam = member(&checked, "beam");
        assert_ne!(beam.status, "pass", "imposed {imposed}: {beam:?}");
        let reason = "buckles at k_c f_c,0,d A = 207.133 N (k_mod 0.6, 1.35 permanent) against F_d = 228.454 N";
        assert!(
            beam.missing.iter().any(|why| why.contains(reason)),
            "imposed {imposed}: {reason} in {:?}",
            beam.missing
        );
        assert!(
            beam.held_by.is_empty(),
            "imposed {imposed}: {:?}",
            beam.held_by
        );
    }
    // Strips 30 x 30 hold under both combinations; the calculation names the
    // combination and k_mod each buckles at.
    let checked = report(&braced_beam(540.0, 60.0, 30.0, "C24"));
    let beam = member(&checked, "beam");
    assert_eq!(beam.held_by.len(), 6, "{beam:?}");
    for brace in &beam.held_by {
        let anchorage = brace.anchorage.as_ref().unwrap();
        assert!(
            anchorage.k_mod.is_some() && anchorage.combination.is_some(),
            "{anchorage:?}"
        );
        assert!(anchorage.capacity_n >= anchorage.force_n, "{anchorage:?}");
    }
    let html = ketchup_program::calculation::document(
        "design.star",
        &checked,
        ketchup_program::calculation::Language::Slovak,
    );
    assert!(
        html.contains("<th>k_mod (kombinácia najbližšia vzperu)</th>"),
        "{html}"
    );
    assert!(html.contains("<td>0,60 (1.35 permanent)</td>"), "{html}");
}

/// R1126og-PG-1104: under G 400 + Q 106 kN/m² strips 20 x 20 hold, buckling at
/// 0.207 kN (k_mod 0.6, 1.35 permanent) against the 0.169 kN that combination
/// asks; the calculation printed only the largest F_d, 0.219 kN of 1.35 G +
/// 1.5 Q, so the brace looked too weak for what the check accepted.
#[test]
fn the_calculation_prints_the_force_of_the_combination_a_brace_buckles_in() {
    let checked = report(&braced_beam(400.0, 106.0, 20.0, "C24"));
    let beam = member(&checked, "beam");
    assert_eq!(beam.status, "pass", "{beam:?}");
    assert_eq!(beam.held_by.len(), 6, "{beam:?}");
    let html = ketchup_program::calculation::document(
        "design.star",
        &checked,
        ketchup_program::calculation::Language::Slovak,
    );
    let row =
        "<td class=\"n\">0,207 kN</td><td class=\"n\">0,169 kN</td><td>0,60 (1.35 permanent)</td>";
    assert_eq!(html.matches(row).count(), 6, "{html}");
    assert!(html.contains("<th>F_d (tej kombinácie)</th>"), "{html}");
}

/// R1125og-PG-1004: a brace of a material naming no product took the k_mod and
/// gamma_M of solid timber, though a member of it is not verified for that.
#[test]
fn a_brace_of_a_material_naming_no_product_leaves_the_member_not_verified() {
    let named = report(&braced_beam(100.0, 0.0, 30.0, "C24"));
    assert_eq!(member(&named, "beam").held_by.len(), 6);
    let bare = report(&braced_beam(100.0, 0.0, 30.0, "bare C24"));
    let beam = member(&bare, "beam");
    assert_eq!(beam.status, "not_verified", "{beam:?}");
    assert!(
        beam.missing
            .iter()
            .any(|why| why.contains("timber_strength(\"bare C24\") names no product")),
        "{:?}",
        beam.missing
    );
}
