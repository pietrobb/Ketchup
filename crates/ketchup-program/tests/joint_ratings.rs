use ketchup_program::joint_check::{
    NO_LOAD, NO_RATING, NO_SERVICE_CLASS, NO_TENSION_RATING, density_below, density_unknown,
    not_bearing, service_class_outside,
};
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("joints.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

fn rejected(source: &str) -> String {
    match run("joints.star", source, &BTreeMap::new()) {
        Ok(_) => panic!("accepted: {source}"),
        Err(error) => error.to_string(),
    }
}

/// A 60 x 200 mm joist hanging on a 120 x 240 mm header beside it.
const FRAME: &str = "
header = box(\"header\", (120, 2000, 240), grounded = True)
joist = box(\"joist\", (3000, 60, 200), at = (120, 500, 40))
";

#[test]
fn an_arunda_joint_carries_its_published_allowable_load_but_stays_unverified_without_a_load() {
    let report = report(&format!("{FRAME}arunda(joist, header, \"50 B\")\n"));
    assert_eq!(report.errors, 0, "{:?}", report.issues);
    let [check] = report.joints.as_slice() else {
        panic!("{:?}", report.joints);
    };
    assert_eq!(check.parts, ["joist".to_owned(), "header".to_owned()]);
    assert_eq!(check.kind, "Arunda 50 B");
    let rating = check.rating.as_ref().expect("Arunda is rated");
    // 240 kg, the lowest value of the 50 mm template's range.
    assert!(
        (rating.load_n.unwrap() - 240.0 * 9.81).abs() < 1e-9,
        "{rating:?}"
    );
    assert_eq!(rating.basis, "allowable");
    assert!(rating.source.contains("arunda.sk"), "{}", rating.source);
    assert!(rating.note.contains("not an ETA or EC5"), "{}", rating.note);
    assert_eq!(check.status, "not_verified");
    assert_eq!(check.missing, [NO_LOAD]);
}

/// Review 2026-10-09 (P3): the dovetail was missing from BTLx without a word.
#[test]
fn an_arunda_dovetail_that_no_export_cuts_is_a_warning() {
    let report = report(&format!("{FRAME}arunda(joist, header, \"50 B\")\n"));
    assert_eq!(report.errors, 0, "{:?}", report.issues);
    let not_cut = report
        .issues
        .iter()
        .find(|issue| issue.message.contains("dovetail is not cut"))
        .unwrap_or_else(|| panic!("{:?}", report.issues));
    assert_eq!(not_cut.severity, ketchup_program::Severity::Warning);
    assert!(not_cut.message.contains("BTLx"), "{}", not_cut.message);
    assert_eq!(not_cut.parts, ["joist", "header"]);
    assert!(
        rejected(&format!(
            "{FRAME}check(False, \"x\", parts = [joist], hint = \"y\", severity = \"note\")\n"
        ))
        .contains("severity is \"error\" or \"warning\"")
    );
}

#[test]
fn a_hanger_without_a_published_rating_is_listed_as_not_verified() {
    let report = report(&format!(
        "{FRAME}joint(joist, header, kind = \"hanger\", fastener = \"strmeň\", bearing = True,
      rating = connector_rating_of(\"strmeň\"))
joint(joist, header, kind = \"screw\", fastener = \"vrut\")\n"
    ));
    // Only the bearing joint is listed; the screw only holds the joist in place.
    let [check] = report.joints.as_slice() else {
        panic!("{:?}", report.joints);
    };
    assert_eq!(check.fastener.as_deref(), Some("strmeň"));
    assert!(check.rating.is_none());
    assert_eq!(check.status, "not_verified");
    assert_eq!(check.missing, [NO_RATING, NO_LOAD]);
}

#[test]
fn an_arunda_template_must_suit_the_width_of_the_carried_timber() {
    let report = report(&format!("{FRAME}arunda(joist, header, \"120 B\")\n"));
    let issue = report
        .issues
        .iter()
        .find(|issue| {
            issue
                .message
                .contains("template 120 B takes timber 120-200 mm wide")
        })
        .unwrap_or_else(|| panic!("{:?}", report.issues));
    assert!(
        issue.message.contains("joist is 60 x 200 mm"),
        "{}",
        issue.message
    );
}

#[test]
fn a_rating_needs_a_bearing_joint_a_known_basis_and_a_source() {
    let rated = |args: &str| {
        format!(
            "{FRAME}joint(joist, header, kind = \"hanger\"{args},
      rating = connector_rating(load_n = 5000, basis = \"characteristic\", source = \"ETA-00/0000\"))\n"
        )
    };
    assert!(rejected(&rated("")).contains("add bearing=True"));
    let bearing = rated(", bearing = True");
    assert_eq!(report(&bearing).joints[0].missing, [NO_LOAD]);
    assert!(
        rejected(&bearing.replace("characteristic", "guessed"))
            .contains("rating basis must be one of")
    );
    assert!(rejected(&bearing.replace("ETA-00/0000", " ")).contains("rating source must name"));
    assert!(rejected(&bearing.replace("5000", "0")).contains("positive number of newtons"));
    // A bearing joint needs the load it may carry down, not only a pull.
    assert!(
        rejected(&bearing.replace("load_n = 5000", "tension_n = 5000")).contains("needs load_n")
    );
    assert!(
        rejected(&bearing.replace("load_n = 5000", "density_kg_m3 = 350"))
            .contains("neither load_n nor tension_n")
    );
}

/// `hung_source` with the joist on a catalogued connector instead, the header
/// of `header` timber and the materials of `classes` (timber_design).
fn hung_on(connector: &str, header: &str, classes: &str) -> Report {
    let source = hung_source("load_n = 1, basis = \"design\"");
    let (frame, _) = source.split_once("joint(joist").expect("the hanger");
    report(&format!(
        "{}{connector}\n",
        frame
            .replace(
                "timber_design({\"C24\": \"C24\"})",
                &format!("timber_design({classes})")
            )
            .replace(
                "(120, 2000, 240), grounded = True",
                &format!("(120, 2000, 240), material = \"{header}\", grounded = True")
            )
    ))
}

/// Review 2026-10-09 §5.2 step 16: a catalogued ETA joist hanger is checked
/// like any rating, and only where the timber is as dense as the table's.
#[test]
fn a_catalogued_joist_hanger_is_checked_on_its_eta_rating_for_dense_enough_timber() {
    let sae = "connector(joist, header, \"SAE380\")";
    let check = only_check(&hung_on(sae, "C24", "{\"C24\": \"C24\"}"));
    assert_eq!(check.kind, "connector");
    assert_eq!(check.fastener.as_deref(), Some("SAE380"));
    let rating = check.rating.as_ref().expect("catalogued");
    assert_eq!(rating.load_n, Some(22300.0));
    assert_eq!(rating.basis, "characteristic");
    assert_eq!(rating.density_kg_m3, Some(350.0));
    assert!(rating.source.contains("ETA-06/0270"), "{}", rating.source);
    // 1.5 Q against kmod (medium term, 0.8) R1,k / 1.3.
    let q = check.load_n.as_ref().expect("a load")["imposed"];
    let expected = 1.5 * q / (0.8 * 22300.0 / 1.3);
    assert!(
        (check.utilization.unwrap() - expected).abs() < 0.001,
        "{check:?}"
    );
    assert_eq!(check.status, "pass", "{check:?}");

    // C16 (rho_k 310) is lighter than the C24 (350) the table holds for.
    let check = only_check(&hung_on(sae, "C24", "{\"C24\": \"C16\"}"));
    assert_eq!(check.status, "not_verified");
    assert_eq!(
        check.missing,
        [
            density_below("joist", "C16", 310.0, 350.0),
            density_below("header", "C16", 310.0, 350.0)
        ]
    );
    // A header of timber with no strength class: its density is unknown.
    let check = only_check(&hung_on(sae, "spruce", "{\"C24\": \"C24\"}"));
    assert_eq!(check.status, "not_verified");
    assert_eq!(check.missing, [density_unknown("header", 350.0)]);
    // Glulam GL24h (385) is dense enough.
    let check = only_check(&hung_on(
        sae,
        "BSH",
        "{\"C24\": \"C24\", \"BSH\": \"GL24h\"}",
    ));
    assert_eq!(check.status, "pass", "{check:?}");

    assert!(
        rejected(&format!("{FRAME}connector(joist, header, \"SAE999\")\n"))
            .contains("no rating for \"SAE999\"")
    );
}

/// Review 2026-10-09 §5.2 step 16: an angle bracket is not bearing; its
/// published pull is still checked, and a program may override the catalogue.
#[test]
fn a_catalogued_pair_of_angle_brackets_holds_a_member_down_and_its_rating_can_be_overridden() {
    let classes = "timber_design({\"C24\": \"C24\"})\n";
    let abr = |ratings: &str| {
        only_check(&anchored(&format!(
            "{classes}connector(beam, post, \"ABR105 pair CNA4.0x35\"{ratings})"
        )))
    };
    let check = abr("");
    assert_eq!(check.kind, "connector");
    assert!(check.load_n.is_none(), "not bearing: {check:?}");
    let rating = check.rating.as_ref().expect("catalogued");
    assert_eq!(rating.load_n, None);
    assert_eq!(rating.tension_n, Some(8840.0));
    assert!(rating.source.contains("ETA-06/0106"), "{}", rating.source);
    let design = check.tension.as_ref().expect("a pull").design_n;
    assert!((design - 794.0).abs() < 0.5, "{design}");
    // Characteristic: kmod of the permanent load that also lifts, 0.6 in
    // service class 1, over gamma_M 1.3.
    assert!(
        (check.utilization.unwrap() - design / (0.6 * 8840.0 / 1.3)).abs() < 0.001,
        "{check:?}"
    );
    assert_eq!(check.status, "pass", "{check:?}");
    assert!(check.missing.is_empty(), "{:?}", check.missing);

    // The program's own rating replaces the catalogue's.
    let check = abr(", ratings = connector_ratings({\"ABR105 pair CNA4.0x35\": \
         connector_rating(tension_n = 500, basis = \"design\", source = \"site pull test\")})");
    assert_eq!(check.rating.as_ref().unwrap().source, "site pull test");
    assert!(
        (check.utilization.unwrap() - design / 500.0).abs() < 0.001,
        "{check:?}"
    );
    assert_eq!(check.status, "fail");
    // ...and None takes a rating out.
    assert!(
        rejected(&format!(
            "{FRAME}connector(joist, header, \"SAE380\", ratings = connector_ratings({{\"SAE380\": None}}))\n"
        ))
        .contains("no rating for \"SAE380\"")
    );
}

/// Review 2026-10-29 PG-65: a catalogued hanger was rated whatever joist it
/// held. Its ETA tabulates R1,k for some joist widths, the hanger is (A - b)/2
/// deep around a joist b wide, and it must be at least 60 % of the joist deep.
#[test]
fn a_catalogued_hanger_is_rated_only_around_a_joist_it_is_made_for() {
    let misfit = |source: &str, wanted: &str| {
        let report = report(source);
        let issue = report
            .issues
            .iter()
            .find(|issue| issue.message.contains(wanted))
            .unwrap_or_else(|| panic!("{wanted} in {:?}", report.issues));
        assert_eq!(issue.severity, ketchup_program::Severity::Error);
        assert_eq!(issue.parts, ["joist", "header"]);
        report
    };
    // SAE250 around the 60 mm joist is 95 mm deep: the 200 mm joist would tip in it.
    let tipping = misfit(
        &format!("{FRAME}connector(joist, header, \"SAE250\")\n"),
        "SAE250 holds a joist 60 mm wide only 95-158.3 mm high; joist is 200 mm",
    );
    let [check] = tipping.joints.as_slice() else {
        panic!("{:?}", tipping.joints);
    };
    assert!(check.rating.is_none(), "{check:?}");
    assert_eq!(check.status, "not_verified");
    // SAE380 is tabulated for joists 38-100 mm wide.
    misfit(
        &format!("{FRAME}connector(joist, header, \"SAE380\")\n").replace("60, 200)", "120, 200)"),
        "SAE380 is rated for joists 38-100 mm wide; joist is 120 mm",
    );
    // A pair of ABR105 nails its 105 mm flange to the held member's side.
    misfit(
        &format!("{FRAME}connector(joist, header, \"ABR105 pair CNA4.0x35\")\n")
            .replace("60, 200)", "60, 100)"),
        "ABR105 nails its 105 mm flange to the side of a member at least 105 mm high; joist is 100 mm",
    );

    // pick=True takes the catalogued hanger that fits.
    let picked = report(&format!(
        "{FRAME}connector(joist, header, \"SAE\", pick = True)\n"
    ));
    assert_eq!(picked.errors, 0, "{:?}", picked.issues);
    assert_eq!(picked.joints[0].fastener.as_deref(), Some("SAE380"));
    assert!(
        rejected(
            &format!("{FRAME}connector(joist, header, \"SAE\", pick = True)\n")
                .replace("60, 200)", "120, 200)")
        )
        .contains("none of SAE250, SAE380, SAE500 fits")
    );
}

/// Review 2026-10-29 PG-65: the ETAs hold for service classes 1 and 2, and
/// the nails their ratings count were in neither the hardware list nor a quote.
#[test]
fn a_catalogued_connector_holds_in_its_service_classes_with_the_nails_it_counts() {
    let sae = "connector(joist, header, \"SAE380\")";
    let report = hung_on(sae, "C24", "{\"C24\": \"C24\"}, service_class = 3");
    let check = only_check(&report);
    assert_eq!(check.status, "not_verified");
    assert_eq!(check.missing, [service_class_outside("joist", 3, &[1, 2])]);
    assert_eq!(
        check.rating.as_ref().unwrap().service_classes,
        [1, 2],
        "{check:?}"
    );

    let bought = |report: &Report| {
        report
            .bom
            .hardware
            .iter()
            .map(|row| (row.item.clone(), row.count))
            .collect::<Vec<_>>()
    };
    let report = hung_on(sae, "C24", "{\"C24\": \"C24\"}");
    assert_eq!(
        bought(&report),
        [
            ("SAE380".to_owned(), 1),
            ("square twist nail 3.75x30".to_owned(), 34)
        ]
    );
    let report = anchored(
        "timber_design({\"C24\": \"C24\"})\nconnector(beam, post, \"ABR105 pair CNA4.0x50\")",
    );
    assert_eq!(
        bought(&report),
        [("ABR105".to_owned(), 2), ("CNA4.0x50".to_owned(), 48)]
    );
}

/// The joist of `FRAME` on a hanger at the header and a post at its far end,
/// under a 600 mm deck with 2 kN/m² imposed load, its hanger rated `rating`.
fn hung(rating: &str) -> Report {
    report(&hung_source(rating))
}

fn hung_source(rating: &str) -> String {
    format!(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
timber_design({{\"C24\": \"C24\"}})
header = box(\"header\", (120, 2000, 240), grounded = True)
joist = box(\"joist\", (3000, 60, 200), at = (120, 500, 40), material = \"C24\", tags = [\"frame\"])
box(\"post\", (100, 60, 240), at = (3020, 500, -200), material = \"C24\", grounded = True)
box(\"deck\", (3000, 600, 20), at = (120, 230, 240), material = \"OSB\", tags = [\"deck\"])
joint(joist, header, kind = \"hanger\", bearing = True,
      rating = connector_rating({rating}, source = \"ETA-00/0000\"))
"
    )
}

#[test]
fn a_rated_hanger_compares_its_load_with_its_capacity_on_the_rating_basis() {
    let check = |rating: &str| {
        let report = hung(rating);
        let [check] = report.joints.as_slice() else {
            panic!("{:?}", report.joints);
        };
        let imposed = check.load_n.as_ref().expect("a load")["imposed"];
        assert!(imposed > 1000.0, "{:?}", check.load_n);
        (check.clone(), imposed)
    };
    // Allowable: the characteristic load against 1000 N.
    let (allowable, q) = check("load_n = 1000, basis = \"allowable\"");
    assert_eq!(
        allowable.utilization,
        Some((q / 1000.0 * 1000.0).round() / 1000.0)
    );
    assert_eq!(allowable.status, "fail");
    // Characteristic: 1.5 Q against kmod (medium term, 0.8) R_k / 1.3.
    let (characteristic, q) = check("load_n = 5000, basis = \"characteristic\"");
    let expected = 1.5 * q / (0.8 * 5000.0 / 1.3);
    assert!(
        (characteristic.utilization.unwrap() - expected).abs() < 0.001,
        "{characteristic:?}"
    );
    assert_eq!(characteristic.status, "pass");
    assert_eq!(
        characteristic.combination.as_deref(),
        Some("1.35 permanent + 1.5 imposed")
    );
    // Design: 1.5 Q against R_d.
    let (design, q) = check("load_n = 2000, basis = \"design\"");
    assert!(
        (design.utilization.unwrap() - 1.5 * q / 2000.0).abs() < 0.001,
        "{design:?}"
    );
    assert_eq!(design.status, "fail");
}

#[test]
fn a_characteristic_rating_on_parts_without_a_strength_class_is_not_verified() {
    let rating = "load_n = 5000, basis = \"characteristic\"";
    let report =
        report(&hung_source(rating).replace("material = \"C24\"", "material = \"spruce\""));
    let [check] = report.joints.as_slice() else {
        panic!("{:?}", report.joints);
    };
    assert!(check.load_n.is_some(), "{check:?}");
    assert_eq!(check.utilization, None);
    assert_eq!(check.status, "not_verified");
    assert_eq!(check.missing, [NO_SERVICE_CLASS]);
}

/// The balcony beam of `member_loads` (posts centred 50 and 2950 mm, a 1550 mm
/// cantilever, a 1 m deck with g 0.5 and q 2 kN/m²) held down at post a by `joint`.
fn anchored(joint: &str) -> Report {
    report(&format!(
        "load_path(only = [\"frame\"], carriers = [\"deck\"])
weight_scope([\"heavy\"])
area_load(\"finish\", kind = \"permanent\", kn_m2 = 0.5, on = [\"deck\"], source = \"test\")
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
post = box(\"post a\", (100, 100, 2000), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (2900, 0, 0), material = \"C24\", grounded = True)
beam = box(\"beam\", (4500, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (4500, 1000, 20), at = (0, -450, 2200), material = \"OSB\", tags = [\"deck\"])
{joint}
"
    ))
}

/// Post a's reaction by hand, per N/mm: 1450 / 2900 of the 3000 mm between
/// the outer bearing edges, less 800 / 2900 of the 1500 mm cantilever.
const INSIDE: f64 = 1500.0;
const CANTILEVER: f64 = -1500.0 * 800.0 / 2900.0;

fn hold_down(rating: &str) -> String {
    format!(
        "joint(beam, post, kind = \"hold-down\", bearing = True, fasteners = [(50, 50, 2100)],
      rating = connector_rating({rating}, source = \"ETA-00/0000\"))"
    )
}

fn only_check(report: &Report) -> ketchup_program::joint_check::JointCheck {
    let [check] = report.joints.as_slice() else {
        panic!("{:?}", report.joints);
    };
    check.clone()
}

/// Review cbec147 ST-N3: a rated hanger compared |pull| with its downward
/// capacity, and a pull through any other joint was never checked.
#[test]
fn a_joint_holding_a_balcony_down_is_checked_for_its_pull_against_a_tension_rating() {
    let (g, q) = (0.5, 2.0);
    // EQU: 0.9 g inside, 1.1 g and 1.5 q on the cantilever: 794 N of pull.
    let design = -(0.9 * g * INSIDE + 1.1 * g * CANTILEVER + 1.5 * q * CANTILEVER);
    // Characteristic: g whole, q only on the cantilever: 284 N.
    let characteristic = -(g * (INSIDE + CANTILEVER) + q * CANTILEVER);
    assert!((design - 794.0).abs() < 0.1 && (characteristic - 284.5).abs() < 0.1);
    // Downward (whole deck, lever rule): g and q x 4500 x 700 / 2900.
    let down = |load: f64| load * 4500.0 * 700.0 / 2900.0;

    // A design rating that gives only the load downward: the pull is not verified.
    let check = only_check(&anchored(&hold_down("load_n = 5000, basis = \"design\"")));
    let tension = check.tension.as_ref().expect("a pull");
    assert!((tension.design_n - design).abs() < 0.1, "{tension:?}");
    assert!(
        (tension.characteristic_n - characteristic).abs() < 0.1,
        "{tension:?}"
    );
    assert_eq!(tension.lifted_by, ["imposed", "permanent"]);
    let downward = (1.35 * down(g) + 1.5 * down(q)) / 5000.0;
    assert!(
        (check.utilization.unwrap() - downward).abs() < 0.001,
        "{check:?}"
    );
    assert_eq!(check.status, "not_verified");
    assert_eq!(check.missing, [NO_TENSION_RATING]);

    // With tension_n the pull governs: 794 / 700.
    let check = only_check(&anchored(&hold_down(
        "load_n = 5000, basis = \"design\", tension_n = 700",
    )));
    assert!(
        (check.utilization.unwrap() - design / 700.0).abs() < 0.001,
        "{check:?}"
    );
    assert_eq!(check.status, "fail");
    assert!(
        check
            .combination
            .as_deref()
            .unwrap()
            .starts_with("pull, EQU")
    );

    // Allowable: the characteristic pull, 284 / 250.
    let check = only_check(&anchored(&hold_down(
        "load_n = 5000, basis = \"allowable\", tension_n = 250",
    )));
    assert!(
        (check.utilization.unwrap() - characteristic / 250.0).abs() < 0.001,
        "{check:?}"
    );
    assert_eq!(check.status, "fail");
    let check = only_check(&anchored(&hold_down(
        "load_n = 5000, basis = \"allowable\", tension_n = 400",
    )));
    assert_eq!(check.status, "pass", "{check:?}");

    // A screw that is not bearing holds it down too, but is never checked.
    let check = only_check(&anchored(
        "joint(beam, post, kind = \"screw\", fastener = \"screw 8 x 200\", fasteners = [(50, 50, 2100)])",
    ));
    assert_eq!(check.joint, "screw:beam+post a");
    assert_eq!(check.status, "not_verified");
    assert_eq!(
        check.missing,
        [not_bearing(check.tension.as_ref().unwrap().design_n)]
    );
    assert!((check.tension.unwrap().design_n - design).abs() < 0.1);
    assert!(
        rejected(&format!(
            "{FRAME}joint(joist, header, kind = \"hanger\", bearing = True,
      rating = connector_rating(load_n = 5000, basis = \"design\", source = \"ETA\", tension_n = 0))\n"
        ))
        .contains("tension_n must be a positive number")
    );
}

#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_every_bearing_joint_of_the_house_is_listed_with_its_rating_or_as_not_verified() {
    const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");
    let report = report(HOUSE);
    assert!(report.joints.len() >= 20, "{}", report.joints.len());
    // No maker's table for the hangers yet, nor the site's snow where the roof
    // bears: every one says what it lacks.
    let snow = "the load is incomplete: sneh (snow) has no value";
    for check in &report.joints {
        assert_eq!(check.status, "not_verified");
        assert!(check.load_n.is_some(), "{}", check.joint);
        assert_eq!(check.missing[0], NO_RATING, "{}", check.joint);
        assert!(
            check.missing[1..].iter().all(|reason| reason == snow),
            "{:?}",
            check.missing
        );
    }
}
