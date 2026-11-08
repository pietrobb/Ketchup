//! thermal_check(): U, the inner surface and condensation of a build-up to
//! STN 73 0540-2 or ČSN 73 0540-2 by the site's country (review 2026-11-08 §5.3).
use ketchup_program::thermal::{THERMAL_FAILED, THERMAL_NOT_VERIFIED, ThermalCheck};
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("thermal.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

fn error(source: &str) -> String {
    match run("thermal.star", source, &BTreeMap::new()) {
        Ok(_) => panic!("thermal_check() was accepted:\n{source}"),
        Err(error) => format!("{error:#}"),
    }
}

fn only(report: &Report) -> &ThermalCheck {
    assert_eq!(report.thermal.buildups.len(), 1, "{:#?}", report.thermal);
    &report.thermal.buildups[0]
}

fn kinds(report: &Report) -> Vec<&str> {
    report.issues.iter().map(|issue| issue.kind).collect()
}

/// Gypsum, a vapour retarder (s_d 2 m), 160 mm of mineral wool between C24
/// studs at 625, 60 mm of wood fibre board and a ventilated cladding.
const TIMBER_WALL: &str = r#"
WALL = [
    {"name": "gypsum", "thickness": 12.5, "material": "gypsum board", "lambda": 0.25, "mu": 10},
    {"name": "retarder", "thickness": 0.2, "material": "membrane", "lambda": 0.2, "sd": 2},
    {"name": "frame", "thickness": 160, "material": "C24", "spacing": 625, "stud": 60,
     "lambda": 0.13, "mu": 50, "infill": "mineral wool", "infill_lambda": 0.035, "infill_mu": 1},
    {"name": "wood fibre", "thickness": 60, "material": "wood fibre board", "lambda": 0.045, "mu": 5},
    {"name": "battens", "thickness": 24, "material": "battens", "ventilated": True},
    {"name": "cladding", "thickness": 20, "material": "larch cladding"},
]
"#;

fn wall(site: &str, layers: &str, extra: &str) -> String {
    format!(
        "{TIMBER_WALL}\n{site}\nT = thermal_check(\"wall S\", layers = {layers}, element = \"wall\"{extra})\n"
    )
}

#[test]
fn a_timber_wall_passes_the_slovak_requirement_with_u_from_the_bounds_of_en_iso_6946() {
    let report = report(&wall("site(\"SK\")", "WALL", ", theta_e = -15"));
    let check = only(&report);
    // R_si = R_se = 0.13 behind the ventilated battens; R'_T 5.591, R''_T 5.270.
    let u = check.u_w_m2k.expect("U");
    assert!((u - 0.1842).abs() < 5e-4, "{check:#?}");
    assert!((check.r_m2k_w.unwrap() - 5.171).abs() < 2e-3, "{check:#?}");
    assert_eq!(check.transmittance, "pass", "{check:#?}");
    assert!(
        check
            .requirement
            .as_deref()
            .unwrap()
            .starts_with("U <= 0.22 W/(m²K), STN 73 0540-2"),
        "{check:#?}"
    );
    // Through the studs: 1 - 0.25 / (0.25 + 0.05 + 0.001 + 1.231 + 1.333 + 0.13).
    assert!((check.f_rsi.unwrap() - 0.9165).abs() < 1e-3, "{check:#?}");
    assert!(
        (check.f_rsi_cr.unwrap() - 0.7893).abs() < 1e-3,
        "{check:#?}"
    );
    assert_eq!(check.surface, "pass");
    assert_eq!(check.condensation, "pass", "{check:#?}");
    assert_eq!(check.status, "pass");
    assert!(check.missing.is_empty(), "{check:#?}");
    assert!(report.issues.is_empty(), "{:#?}", report.issues);
    assert!(report.thermal.basis[1].contains("STN 73 0540-2+Z1+Z2:2019"));
}

#[test]
fn the_returned_struct_gives_u_and_r_to_the_program() {
    let report = report(&format!(
        "{}check(T.u < 0.19 and T.r > 5.1, \"U %s\" % T.u, parts = [], hint = \"\")\n",
        wall("site(\"CZ\")", "WALL", "")
    ));
    assert!(
        !kinds(&report).contains(&"program_condition_failed"),
        "{:#?}",
        report.issues
    );
}

#[test]
fn a_thin_wall_fails_u_and_is_an_error() {
    let thin = r#"[
        {"name": "gypsum", "thickness": 12.5, "lambda": 0.25, "mu": 10},
        {"name": "frame", "thickness": 100, "spacing": 625, "lambda": 0.13, "mu": 50,
         "infill_lambda": 0.035, "infill_mu": 1},
        {"name": "osb", "thickness": 15, "lambda": 0.13, "mu": 50},
    ]"#;
    let report = report(&wall("site(\"SK\")", thin, ", theta_e = -15"));
    let check = only(&report);
    assert!((check.u_w_m2k.unwrap() - 0.3771).abs() < 5e-4, "{check:#?}");
    assert_eq!(check.transmittance, "fail");
    assert_eq!(check.status, "fail");
    // OSB outside the wool without a retarder inside: vapour condenses.
    assert_eq!(check.condenses_in, ["frame", "osb"], "{check:#?}");
    assert_eq!(check.condensation, "not_verified");
    assert!(
        kinds(&report).contains(&THERMAL_FAILED),
        "{:#?}",
        report.issues
    );
    assert!(!report.ok);
    // The Czech requirement for a wall is 0.30: it fails there too.
    let czech = report_of(&wall("site(\"CZ\")", thin, ""));
    assert_eq!(czech.transmittance, "fail", "{czech:#?}");
}

fn report_of(source: &str) -> ThermalCheck {
    only(&report(source)).clone()
}

#[test]
fn a_single_board_fails_the_inner_surface() {
    let board = r#"[{"name": "board", "thickness": 20, "lambda": 0.13, "mu": 50}]"#;
    let check = report_of(&wall("site(\"CZ\")", board, ", theta_e = -15"));
    assert!((check.f_rsi.unwrap() - 0.4367).abs() < 1e-3, "{check:#?}");
    assert_eq!(check.surface, "fail", "{check:#?}");
}

#[test]
fn missing_values_leave_the_criteria_not_verified_and_a_warning() {
    let no_mu = r#"[
        {"name": "gypsum", "thickness": 12.5, "lambda": 0.25},
        {"name": "wool", "thickness": 200, "lambda": 0.035, "mu": 1},
    ]"#;
    let report = report(&wall("", no_mu, ", theta_e = -15"));
    let check = only(&report);
    assert!(check.u_w_m2k.is_some());
    assert_eq!(check.transmittance, "not_verified");
    assert_eq!(check.condensation, "not_verified");
    assert_eq!(check.status, "not_verified");
    let missing = check.missing.join("\n");
    assert!(missing.contains("no site(country)"), "{missing}");
    assert!(missing.contains("layer \"gypsum\" has no mu"), "{missing}");
    assert_eq!(kinds(&report), [THERMAL_NOT_VERIFIED]);
    assert!(report.ok);

    let no_lambda = r#"[{"name": "wool", "thickness": 200, "mu": 1}]"#;
    let check = report_of(&wall("site(\"SK\")", no_lambda, ""));
    assert_eq!(check.u_w_m2k, None);
    let missing = check.missing.join("\n");
    assert!(
        missing.contains("layer \"wool\" has no lambda"),
        "{missing}"
    );
    assert!(missing.contains("no theta_e"), "{missing}");
}

#[test]
fn a_slovak_floor_on_the_ground_is_checked_by_its_resistance() {
    let floor = r#"[
        {"name": "screed", "thickness": 50, "lambda": 1.3, "mu": 50},
        {"name": "xps", "thickness": 100, "lambda": 0.034, "mu": 100},
    ]"#;
    let source = format!(
        "site(\"SK\")\nthermal_check(\"floor\", layers = {floor}, element = \"floor_on_ground\")\n"
    );
    let check = report_of(&source);
    // 0.05 / 1.3 + 0.1 / 0.034 = 2.98 m²K/W, at least 2.50.
    assert!((check.r_m2k_w.unwrap() - 2.980).abs() < 2e-3, "{check:#?}");
    assert_eq!(check.transmittance, "pass");
    assert!(check.requirement.unwrap().starts_with("R >= 2.50"));
    assert_eq!(check.surface, "not_verified");
}

#[test]
fn wrong_inputs_are_refused() {
    let one = r#"[{"name": "wool", "thickness": 200, "lambda": 0.035, "mu": 1}]"#;
    let refused = |source: String, expected: &str| {
        let message = error(&source);
        assert!(message.contains(expected), "{message}");
    };
    refused(
        format!("thermal_check(\"a\", layers = {one}, element = \"window\")"),
        "element must be one of",
    );
    refused(
        "thermal_check(\"a\", layers = [{\"name\": \"x\", \"thickness\": 10, \"lambda\": 0.1, \"r\": 1}], element = \"wall\")".to_owned(),
        "takes lambda or r, not both",
    );
    refused(
        "thermal_check(\"a\", layers = [{\"name\": \"x\", \"thickness\": 10, \"mu\": 0.5}], element = \"wall\")".to_owned(),
        "mu is at least 1",
    );
    refused(
        format!("thermal_check(\"a\", layers = {one}, element = \"wall\", phi_i = 120)"),
        "phi_i must lie in",
    );
    refused(
        format!(
            "thermal_check(\"a\", layers = {one}, element = \"wall\")\nthermal_check(\"a\", layers = {one}, element = \"roof\")"
        ),
        "checked already",
    );
}
