//! The texts of the structural calculation and the thermal check (review
//! 2026-11-10 og PG-403 to PG-410): what governs, the clauses cited, a column,
//! the site in the sheet language, the thermal comparison and the brace of a
//! column.
use ketchup_program::calculation::{Language, document};
use ketchup_program::thermal::THERMAL_FAILED;
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("calculation.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

/// A 4000 mm beam on two grounded posts under a deck with 2 kN/m² imposed
/// load and the snow of a site.
fn beam(site: &str, class: &str) -> String {
    format!(
        "SITE = {site}
load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
area_load(\"snow\", kind = \"snow\", kn_m2 = snow_load(SITE.sk, 0), on = [\"deck\"], source = SITE.source)
timber_design({{\"{class}\": \"{class}\"}})
box(\"post a\", (100, 100, 2000), material = \"{class}\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"{class}\", grounded = True)
box(\"beam\", (4000, 100, 200), at = (0, 0, 2000), material = \"{class}\", tags = [\"frame\"])
box(\"deck\", (4000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
"
    )
}

const CZECH_SITE: &str = "site(\"CZ\", municipality = \"Brno\", altitude = 237, snow_zone = \"II\")";
const SLOVAK_SITE: &str =
    "site(\"SK\", municipality = \"Poprad\", altitude = 672, snow_zone = \"2\")";

/// A C24 post 60 x 140 x 3000 on a grounded slab under a deck.
const POST: &str = "load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"heavy\", kind = \"imposed\", kn_m2 = 200.0, on = [\"deck\"], source = \"test\")
timber_design({\"C24\": \"C24\"})
box(\"slab\", (300, 300, 50), at = (-100, -80, 0), material = \"C24\", grounded = True)
box(\"post\", (60, 140, 3000), at = (0, 0, 50), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (60, 140, 20), at = (0, 0, 3050), material = \"OSB\", tags = [\"deck\"])
";

fn section<'a>(html: &'a str, part: &str) -> &'a str {
    let start = html
        .find(&format!(". {part} ("))
        .unwrap_or_else(|| panic!("no section of {part}:\n{html}"));
    let end = html[start..].find("</section>").expect("the section ends") + start;
    &html[start..end]
}

/// PG-403: "(rozhoduje)" went by the check's name, so every bearing row said
/// it governs when bearing did.
#[test]
fn only_the_highest_row_of_the_governing_check_says_it_governs() {
    let mut report = report(&beam(CZECH_SITE, "C24"));
    let beam = report
        .design
        .members
        .iter_mut()
        .find(|m| m.part == "beam")
        .expect("the beam is checked");
    // A member bearing at several places has a bearing row for each.
    let mut lower = beam
        .checks
        .iter()
        .find(|check| check.name == "bearing")
        .expect("a bearing row")
        .clone();
    lower.utilization /= 2.0;
    beam.checks.push(lower);
    let bearings: Vec<f64> = beam
        .checks
        .iter()
        .filter(|check| check.name == "bearing")
        .map(|check| check.utilization)
        .collect();
    assert!(bearings.len() >= 2, "{beam:?}");
    beam.governing = Some("bearing".to_owned());
    let highest = bearings.iter().copied().fold(f64::MIN, f64::max);
    let html = document("calculation.star", &report, Language::Slovak);
    let text = section(&html, "beam");
    assert_eq!(text.matches("(rozhoduje)").count(), 1, "{text}");
    let row = text
        .lines()
        .find(|line| line.contains("(rozhoduje)"))
        .expect("the governing row");
    let shown = format!("{highest:.3}").replace('.', ",");
    assert!(row.contains(&format!(">{shown}</td>")), "{row}");
}

/// PG-404: k_h of glulam is 3.3 (3.2); the instantaneous deflection does not
/// follow equation (2.2), which gives u_fin.
#[test]
fn bending_cites_the_size_factor_of_its_product_and_u_inst_no_u_fin_equation() {
    let solid = report(&beam(CZECH_SITE, "C24"));
    let html = document("calculation.star", &solid, Language::Slovak);
    let text = section(&html, "beam");
    assert!(text.contains("k_h 3.2 (3.1)"), "{text}");
    assert!(!text.contains("2.2.3 (2.2)"), "{text}");
    assert!(text.contains("EN 1995-1-1 2.2.3, 7.2 tab. 7.2"), "{text}");

    let glulam = report(&beam(CZECH_SITE, "GL24h"));
    let html = document("calculation.star", &glulam, Language::Slovak);
    let text = section(&html, "beam");
    assert!(text.contains("k_h 3.3 (3.2)"), "{text}");
    assert!(!text.contains("k_h 3.2 (3.1)"), "{text}");
}

/// PG-405: a column is not checked for lateral torsional buckling, so its
/// section does not say how it is.
#[test]
fn a_column_says_nothing_of_lateral_torsional_buckling() {
    let report = report(POST);
    let post = report
        .design
        .members
        .iter()
        .find(|m| m.part == "post")
        .expect("the post is checked");
    assert_eq!(post.role, "column", "{post:?}");
    let html = document("calculation.star", &report, Language::Slovak);
    let text = section(&html, "post");
    assert!(text.contains("(stĺp)"), "{text}");
    assert!(!text.contains("klopenie"), "{text}");
    assert!(!text.contains("Zaistené úseky"), "{text}");
}

/// PG-408: brace() on a column said the brace "is not anchored"; the reason is
/// that the check of a column does not use bracing.
#[test]
fn a_brace_declared_on_a_column_says_a_column_is_not_braced() {
    let source = format!(
        "{POST}box(\"wall\", (60, 100, 3000), at = (0, 400, 50), material = \"C24\", grounded = True)
box(\"rail\", (60, 400, 60), at = (0, 140, 1500), material = \"C24\")
joint(\"rail\", \"post\", kind = \"screw\")
joint(\"rail\", \"wall\", kind = \"screw\")
brace([\"post\"], by = [\"rail\"], edge = \"top\")
"
    );
    let report = report(&source);
    let ignored: Vec<&str> = report
        .issues
        .iter()
        .filter(|issue| issue.kind == "declared_bracing_ignored")
        .map(|issue| issue.message.as_str())
        .collect();
    assert!(
        ignored
            .iter()
            .any(|message| message.contains("post is a column")),
        "{ignored:?}"
    );
    assert!(
        ignored
            .iter()
            .all(|message| !message.contains("not anchored")),
        "{ignored:?}"
    );
}

/// The calculation before its basis, which it quotes in English as such.
fn body(html: &str) -> &str {
    &html[..html.find("<h2>Podklady výpočtu</h2>").expect("a basis")]
}

/// PG-406: the site line was English inside a Slovak or Czech calculation.
#[test]
fn the_site_is_written_in_the_sheet_language() {
    let report = report(&beam(SLOVAK_SITE, "C24"));
    let html = document("calculation.star", &report, Language::Slovak);
    let text = body(&html);
    assert!(
        text.contains("Miesto stavby: SK, Poprad, 672 m n. m."),
        "{text}"
    );
    assert!(text.contains("snehová oblasť 2"), "{text}");
    assert!(text.contains("do 1000 m n. m."), "{text}");
    for english in ["above sea level", "snow zone", "recommended values", "at most"] {
        assert!(!text.contains(english), "{english}: {text}");
    }

    let report = self::report(&beam(CZECH_SITE, "C24"));
    let html = document("calculation.star", &report, Language::Czech);
    let text = body(&html);
    assert!(text.contains("Místo stavby: CZ, Brno, 237 m n. m."), "{text}");
    assert!(text.contains("sněhová oblast II"), "{text}");
    assert!(!text.contains("snow zone"), "{text}");
}

/// PG-410: a site below sea level takes A = 0 for s_k, and its source said
/// the altitude given.
#[test]
fn a_site_below_sea_level_names_the_altitude_s_k_used() {
    let report = report(&beam(
        "site(\"SK\", altitude = -20, snow_zone = \"2\")",
        "C24",
    ));
    let site = report
        .design
        .basis
        .iter()
        .find(|line| line.starts_with("site: "))
        .expect("a site line");
    assert!(site.contains("s_k = 0.425 + 0/505"), "{site}");
    assert!(!site.contains("-20/505"), "{site}");
}

/// PG-407 and PG-409: a wall failing only U printed "f_Rsi 0.9 < 0.8"; the
/// Slovak 0.22 is the normalized U_N, not U_r2.
#[test]
fn a_thermal_failure_compares_f_rsi_as_it_is_and_names_u_n() {
    let source = r#"site("SK")
thermal_check("thin", element = "wall", theta_e = -15, layers = [
    {"name": "gypsum", "thickness": 12.5, "lambda": 0.25, "mu": 10},
    {"name": "frame", "thickness": 100, "spacing": 625, "lambda": 0.13, "mu": 50,
     "infill_lambda": 0.035, "infill_mu": 1},
    {"name": "osb", "thickness": 15, "lambda": 0.13, "mu": 50},
])
"#;
    let report = report(source);
    let check = &report.thermal.buildups[0];
    let (f, cr) = (check.f_rsi.unwrap(), check.f_rsi_cr.unwrap());
    assert!(f >= cr, "the surface passes: {check:#?}");
    let requirement = check.requirement.as_deref().unwrap();
    assert!(requirement.contains("U_N"), "{requirement}");
    assert!(!requirement.contains("U_r2"), "{requirement}");
    let failed = report
        .issues
        .iter()
        .find(|issue| issue.kind == THERMAL_FAILED)
        .expect("U fails");
    assert!(
        failed
            .message
            .contains(&format!("f_Rsi {f:.3} >= {cr:.3}")),
        "{}",
        failed.message
    );
    assert!(!report.thermal.basis.iter().any(|line| line.contains("U_r2")));
}
