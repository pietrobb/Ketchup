//! The printable structural calculation in Slovak and Czech (review 2026-11-08
//! §5.3): for each member its inputs, the clause and equation of each check,
//! the governing combination and the stretches held sideways.
use ketchup_program::calculation::{Language, document};
use ketchup_program::{Report, run};
use std::collections::BTreeMap;

fn report(source: &str) -> Report {
    run("calculation.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .1
}

/// A 4000 mm C24 beam on two grounded posts under a deck with 2 kN/m² imposed
/// load and the snow of a Czech site in zone II.
const CZECH_BEAM: &str = "SITE = site(\"CZ\", municipality = \"Brno\", altitude = 237, snow_zone = \"II\")
load_path(only = [\"frame\"], carriers = [\"deck\"])
area_load(\"live\", kind = \"imposed\", kn_m2 = 2.0, on = [\"deck\"], source = \"test\")
area_load(\"snow\", kind = \"snow\", kn_m2 = snow_load(SITE.sk, 0), on = [\"deck\"], source = SITE.source)
timber_design({\"C24\": \"C24\"})
box(\"post a\", (100, 100, 2000), material = \"C24\", grounded = True)
box(\"post b\", (100, 100, 2000), at = (3900, 0, 0), material = \"C24\", grounded = True)
box(\"beam\", (4000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
box(\"deck\", (4000, 600, 20), at = (0, -250, 2200), material = \"OSB\", tags = [\"deck\"])
";

/// A GL24h 80 x 400 beam over 5.8 m under 13 kN at its middle; with `braced`,
/// battens rest on its top and on a grounded wall beside it, screwed to both.
fn slender_beam(braced: bool) -> String {
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
    if braced {
        source.push_str(
            "box(\"wall\", (5900, 100, 2400), at = (0, 1000, 0), material = \"GL24h\", grounded = True)\n",
        );
        for (index, x) in [300, 1200, 2100, 3600, 4500, 5400].iter().enumerate() {
            source.push_str(&format!(
                "box(\"batten {index}\", (50, 1100, 50), at = ({x}, 0, 2400), material = \"GL24h\")
joint(\"batten {index}\", \"beam\", kind = \"screw\")
joint(\"batten {index}\", \"wall\", kind = \"screw\")
"
            ));
        }
    }
    source
}

/// The text of the member section of `part`.
fn section<'a>(html: &'a str, part: &str) -> &'a str {
    let start = html
        .find(&format!(". {part} ("))
        .unwrap_or_else(|| panic!("no section of {part}:\n{html}"));
    let end = html[start..].find("</section>").expect("the section ends") + start;
    &html[start..end]
}

#[test]
fn a_czech_calculation_names_each_check_by_its_clause_with_the_governing_combination() {
    let report = report(CZECH_BEAM);
    let html = document("calculation.star", &report, Language::Czech);
    assert!(html.starts_with("<!DOCTYPE html>"), "{html}");
    assert!(html.contains("<html lang=\"cs\">"), "{html}");
    assert!(html.contains("@page { size: A4;"), "{html}");
    assert!(
        html.contains("Příloha k dokumentaci pro stavební povolení"),
        "{html}"
    );
    assert!(html.contains("Ověřil a podepsal"), "{html}");
    assert!(html.contains("Místo stavby: CZ, Brno, 237 m"), "{html}");

    let beam = report
        .design
        .members
        .iter()
        .find(|m| m.part == "beam")
        .expect("the beam is checked");
    assert_eq!(beam.status, "pass", "{beam:?}");
    let text = section(&html, "beam");
    assert!(text.contains("(nosník)"), "{text}");
    assert!(text.contains("vyhovuje"), "{text}");
    assert!(
        text.contains("<tr><th>Třída pevnosti</th><td>C24</td></tr>"),
        "{text}"
    );
    assert!(text.contains("100 × 200 mm"), "{text}");
    assert!(
        text.contains("<tr><th>Délka</th><td>4000 mm</td></tr>"),
        "{text}"
    );
    // Inputs: the loads by kind in kN, and each support with its reaction.
    assert!(text.contains("užitné, kat. A "), "{text}");
    assert!(text.contains("sníh "), "{text}");
    assert!(text.contains("post a v "), "{text}");
    assert!(text.contains("post b v "), "{text}");
    // Each check by its clause and equation, its combination with the load
    // kinds named and decimal commas, and its utilization as computed.
    assert!(text.contains("EN 1995-1-1 6.1.6 (6.11), (6.12)"), "{text}");
    assert!(text.contains("EN 1995-1-1 6.1.7 (6.13)"), "{text}");
    assert!(text.contains("EN 1995-1-1 6.1.5 (6.3)"), "{text}");
    assert!(
        text.contains("1,35 · stálé (G) + 1,5 · užitné, kat. A"),
        "{text}"
    );
    assert!(!text.contains("permanent"), "{text}");
    assert!(text.contains("<td>pole 50-3950 mm</td>"), "{text}");
    let governing = beam.governing.as_deref().expect("a governing check");
    let named = match governing {
        "bending" => "Ohyb",
        "shear" => "Smyk",
        "deflection_fin" => "Konečný průhyb",
        "deflection_inst" => "Okamžitý průhyb",
        other => panic!("unexpected governing check {other}"),
    };
    assert!(text.contains(&format!("{named} (rozhoduje)")), "{text}");
    let utilization = format!("{:.3}", beam.utilization.unwrap()).replace('.', ",");
    assert!(text.contains(&format!(">{utilization}</td>")), "{text}");
    // Nothing holds the beam's top sideways in this program.
    assert!(
        text.contains("žádná část nedrží okraj prvku do strany"),
        "{text}"
    );
    // The basis follows, as the check words it.
    assert!(html.contains("Podklady výpočtu"), "{html}");
    assert!(html.contains("EN 1990 6.4.3.2 (6.10)"), "{html}");
}

#[test]
fn a_slovak_calculation_lists_the_stretches_held_and_what_their_braces_take() {
    let report = report(&slender_beam(true));
    let html = document("calculation.star", &report, Language::Slovak);
    assert!(html.contains("<html lang=\"sk\">"), "{html}");
    assert!(
        html.contains("Príloha k dokumentácii pre stavebné povolenie"),
        "{html}"
    );
    let text = section(&html, "beam");
    assert!(text.contains("vyhovuje"), "{text}");
    assert!(
        text.contains("Zaistené úseky (okraj držaný do strany):"),
        "{text}"
    );
    assert!(text.contains("horný okraj"), "{text}");
    assert!(text.contains("(batten 0)"), "{text}");
    assert!(text.contains("(batten 5)"), "{text}");
    assert!(
        text.contains("horný okraj 325 mm (batten 0); horný okraj 1225 mm (batten 1)"),
        "{text}"
    );
    // What the braces of the top must take (9.2.5.2), as the check gives it.
    let beam = report
        .design
        .members
        .iter()
        .find(|m| m.part == "beam")
        .unwrap();
    let [top] = beam.bracing.as_slice() else {
        panic!("one braced edge: {:?}", beam.bracing);
    };
    let kn = |n: f64| format!("{:.2} kN", n / 1000.0).replace('.', ",");
    assert!(text.contains("N_d (9.2.5.2(4))"), "{text}");
    assert!(
        text.contains(&format!(
            "<td>horný</td><td>batten 0, batten 1, batten 2, batten 3, batten 4, batten 5</td><td class=\"n\">{}</td><td class=\"n\">{}</td>",
            kn(top.axial_n),
            kn(top.force_n)
        )),
        "{text}"
    );
    assert!(text.contains("posúdené nie sú"), "{text}");
    assert!(text.contains("<td>Ohyb (rozhoduje)</td>"), "{text}");
}

#[test]
fn a_member_left_not_verified_says_so_with_why() {
    let report = report(&slender_beam(false));
    let html = document("calculation.star", &report, Language::Slovak);
    let text = section(&html, "beam");
    assert!(text.contains("neoverené"), "{text}");
    assert!(text.contains("Neoverené, chýba"), "{text}");
    assert!(
        text.contains("lateral torsional buckling is not verified"),
        "{text}"
    );
    assert!(
        text.contains("žiadna časť nedrží okraj prvku do strany"),
        "{text}"
    );
    assert!(
        html.contains(
            "<tr><td class=\"n\">1</td><td class=\"n\">0</td><td class=\"n\">0</td><td class=\"n\">1</td></tr>"
        ),
        "{html}"
    );
}

#[test]
fn a_program_without_members_says_it_has_none_and_the_language_codes_are_read() {
    let report = report("box(\"a\", (100, 100, 100))\n");
    let html = document("calculation.star", &report, Language::Slovak);
    assert!(html.contains("Program nemá žiadny nosný prvok"), "{html}");
    assert_eq!(Language::from_code("SK"), Some(Language::Slovak));
    assert_eq!(Language::from_code("cz"), Some(Language::Czech));
    assert_eq!(Language::from_code("cs"), Some(Language::Czech));
    assert_eq!(Language::from_code("de"), None);
}
