use ketchup_program::joint_check::{
    NO_LOAD, NO_RATING, NO_SERVICE_CLASS, NO_TENSION_RATING, not_bearing,
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
    assert!((rating.load_n - 240.0 * 9.81).abs() < 1e-9, "{rating:?}");
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
