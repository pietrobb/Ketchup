//! site(): the snow load on the ground from the national annex of the site's
//! country, and the altitude snow's psi depends on (review 2026-11-08 §5.3).
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("site.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

fn error(source: &str) -> String {
    match run("site.star", source, &BTreeMap::new()) {
        Ok(_) => panic!("site() was accepted:\n{source}"),
        Err(error) => format!("{error:#}"),
    }
}

/// A 4000 mm beam on two grounded posts under a deck with 2 kN/m² imposed load
/// and the snow of `site` (0.8 sk), so imposed leads and snow accompanies it.
fn beam(site: &str) -> String {
    format!(
        "SITE = {site}
load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
area_load(\"snow\", kind = \"snow\", kn_m2 = snow_load(SITE.sk, 0), on = [\"deck\"], source = SITE.source)
timber_design({{\"C24\": \"C24\"}})
box(\"post a\", (100, 100, 2000), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (4000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (4000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
"
    )
}

fn site_line(report: &Report) -> &str {
    let last = report.design.basis.last().expect("a design basis");
    assert!(last.starts_with("site: "), "{:?}", report.design.basis);
    last
}

fn bending(report: &Report) -> (f64, String) {
    let beam = report
        .design
        .members
        .iter()
        .find(|m| m.part == "beam")
        .expect("the beam is checked");
    assert_eq!(beam.status, "pass", "{beam:?}");
    let check = beam
        .checks
        .iter()
        .find(|c| c.name == "bending")
        .expect("bending");
    (check.utilization, check.combination.clone())
}

#[test]
fn a_czech_snow_zone_gives_sk_from_the_annex_and_the_basis_names_it() {
    let report = report(&beam(
        "site(\"CZ\", municipality = \"Brno\", altitude = 237, snow_zone = \"II\")",
    ));
    let line = site_line(&report);
    assert!(line.contains("CZ, Brno, 237 m above sea level"), "{line}");
    assert!(
        line.contains("s_k = 1 kN/m² (ČSN EN 1991-1-3 national annex with Z1:2006, snow zone II)"),
        "{line}"
    );
    assert!(line.contains("snow psi0/psi2 0.5/0"), "{line}");
    // The zone as a number is the same zone; 0.8 * 1.0 kN/m² of snow loads the beam.
    let numbered = report_of_zone("2");
    assert_eq!(bending(&numbered), bending(&report));
    let snow = report
        .loads
        .members
        .iter()
        .find(|m| m.part == "beam")
        .unwrap();
    assert!(
        snow.loads_n.get("snow").is_some_and(|n| *n > 0.0),
        "{snow:?}"
    );
}

fn report_of_zone(zone: &str) -> Report {
    report(&beam(&format!(
        "site(\"CZ\", municipality = \"Brno\", altitude = 237, snow_zone = {zone})"
    )))
}

#[test]
fn a_site_above_1000_m_combines_snow_with_psi0_0_7() {
    let low = report(&beam("site(\"CZ\", altitude = 400, snow_zone = \"I\")"));
    let high = report(&beam("site(\"CZ\", altitude = 1200, snow_zone = \"I\")"));
    let (low_u, low_combination) = bending(&low);
    let (high_u, high_combination) = bending(&high);
    assert!(
        low_combination.contains("1.5 imposed + 0.75 snow"),
        "{low_combination}"
    );
    assert!(
        high_combination.contains("1.5 imposed + 1.05 snow"),
        "{high_combination}"
    );
    assert!(high_u > low_u, "{high_u} <= {low_u}");
    assert!(site_line(&high).contains("snow psi0/psi2 0.7/0.2 (EN 1990 table A1.1, above 1000 m)"));
}

#[test]
fn a_slovak_site_takes_sk_read_off_the_map_and_without_it_says_what_to_read() {
    let given = report(&beam(
        "site(\"SK\", municipality = \"Žilina\", altitude = 342, sk = 1.2, source = \"STN EN 1991-1-3/NA1 map, Žilina\")",
    ));
    assert!(site_line(&given).contains("s_k = 1.2 kN/m² (STN EN 1991-1-3/NA1 map, Žilina)"));
    bending(&given);

    let unknown = report(&beam("site(\"sk\", municipality = \"Žilina\")"));
    let line = site_line(&unknown);
    assert!(
        line.contains("s_k not given: read it for \"Žilina\" off the STN EN 1991-1-3/NA1:2012 map and give sk= and source="),
        "{line}"
    );
    assert!(
        line.contains("altitude not given, taken as at most 1000 m"),
        "{line}"
    );
    let beam = unknown
        .design
        .members
        .iter()
        .find(|m| m.part == "beam")
        .unwrap();
    assert_eq!(beam.status, "not_verified");
    assert_eq!(beam.missing, ["snow (snow) has no value"]);
}

#[test]
fn czech_zone_viii_has_no_value_in_the_annex() {
    let report = report(&beam(
        "site(\"CZ\", municipality = \"Pec pod Sněžkou\", snow_zone = 8)",
    ));
    let line = site_line(&report);
    assert!(line.contains("snow zone VIII of the ČSN EN 1991-1-3 national annex with Z1:2006 map has no value; ask ČHMÚ for \"Pec pod Sněžkou\""), "{line}");
    let given = run(
        "site.star",
        &beam("site(\"CZ\", snow_zone = \"VIII\", sk = 5.1, source = \"ČHMÚ\")"),
        &BTreeMap::new(),
    )
    .unwrap()
    .1;
    assert!(site_line(&given).contains("s_k = 5.1 kN/m² (ČHMÚ)"));
}

#[test]
fn site_refuses_what_its_annex_does_not_give() {
    for (source, expected) in [
        ("site(\"AT\")", "country must be one of [\"SK\", \"CZ\"]"),
        (
            "site(\"SK\", snow_zone = \"II\")",
            "gives s_k by region and altitude on its map, not by a zone",
        ),
        ("site(\"CZ\", snow_zone = \"IX\")", "snow_zone is I to VIII"),
        (
            "site(\"CZ\", snow_zone = \"II\", sk = 1.2, source = \"x\")",
            "give snow_zone or sk, not both",
        ),
        ("site(\"SK\", sk = 1.2)", "name the source of sk"),
        (
            "site(\"SK\", sk = 0, source = \"x\")",
            "sk must be a positive kN/m²",
        ),
        (
            "site(\"SK\", altitude = 9000)",
            "altitude is metres above sea level",
        ),
        ("site(\"SK\")\nsite(\"CZ\")", "a program has one site"),
    ] {
        let message = error(source);
        assert!(message.contains(expected), "{source}: {message}");
    }
}

#[test]
fn without_a_site_the_basis_has_no_site_line() {
    let report = report(&beam("struct(sk = 1.0, source = \"test\")"));
    assert!(
        report
            .design
            .basis
            .iter()
            .all(|line| !line.starts_with("site: "))
    );
    assert!(bending(&report).1.contains("0.75 snow"));
}
