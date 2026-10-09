use ketchup_program::joint_check::{NO_LOAD, NO_RATING, NO_SERVICE_CLASS};
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

#[test]
fn every_bearing_joint_of_the_house_is_listed_with_its_rating_or_as_not_verified() {
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
