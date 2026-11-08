//! The printable structural calculation in Slovak or Czech (review 2026-11-08
//! §5.3): one HTML page set from a [`Report`], laid out for A4, as an annex to
//! the documents of a building permit. For each member it gives its inputs,
//! each check with the clause and equation it follows, the governing
//! combination and place, and the stretches of its edges that parts hold
//! sideways; then the joints. It prints what the report computed and nothing
//! more: a member left not verified says so with why, and the page leaves the
//! lines for the engineer who signs it.
use crate::Report;
use crate::loads::{LOAD_KINDS, Loads, MemberLoad};
use crate::member_check::{BracingDemand, Check, MemberCheck};
use std::fmt::Write as _;

/// The language of the calculation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Slovak,
    Czech,
}

impl Language {
    /// `sk`, or `cz` / `cs`, in any case.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code.to_ascii_lowercase().as_str() {
            "sk" => Some(Self::Slovak),
            "cz" | "cs" => Some(Self::Czech),
            _ => None,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Slovak => "sk",
            Self::Czech => "cs",
        }
    }

    fn says(self, slovak: &'static str, czech: &'static str) -> &'static str {
        match self {
            Self::Slovak => slovak,
            Self::Czech => czech,
        }
    }
}

/// The name of each load kind of [`LOAD_KINDS`], in its order.
const KIND_NAMES: [[&str; 2]; 4] = [
    ["stále (G)", "stálé (G)"],
    ["úžitkové, kat. A", "užitné, kat. A"],
    ["strecha, kat. H", "střecha, kat. H"],
    ["sneh", "sníh"],
];

fn kind_name(kind: &str, language: Language) -> String {
    LOAD_KINDS
        .iter()
        .position(|known| *known == kind)
        .map_or_else(
            || kind.to_owned(),
            |i| KIND_NAMES[i][usize::from(language == Language::Czech)].to_owned(),
        )
}

/// A check's name and the clauses and equations it follows.
fn check_name(name: &str, language: Language) -> (String, &'static str) {
    let (slovak, czech, clauses) = match name {
        "bending" => (
            "Ohyb",
            "Ohyb",
            "EN 1995-1-1 6.1.6 (6.11), (6.12); k_h 3.2 (3.1)",
        ),
        "lateral_torsional" => (
            "Klopenie",
            "Klopení",
            "EN 1995-1-1 6.3.3 (6.33); σ_m,crit (6.32), k_crit (6.34), l_ef tab. 6.1",
        ),
        "shear" => ("Šmyk", "Smyk", "EN 1995-1-1 6.1.7 (6.13); k_cr 6.1.7(2)"),
        "shear_notch" => (
            "Šmyk pri záreze",
            "Smyk u zářezu",
            "EN 1995-1-1 6.5.2 (6.60); k_v (6.62)",
        ),
        "deflection_inst" => (
            "Okamžitý priehyb",
            "Okamžitý průhyb",
            "EN 1995-1-1 2.2.3 (2.2), 7.2 tab. 7.2; EN 1990 6.5.3 (6.14b)",
        ),
        "deflection_fin" => (
            "Konečný priehyb",
            "Konečný průhyb",
            "EN 1995-1-1 2.2.3 (2.3)–(2.5), 7.2 tab. 7.2; EN 1990 6.5.3 (6.14b)",
        ),
        "compression" => (
            "Tlak so vzperom",
            "Tlak se vzpěrem",
            "EN 1995-1-1 6.3.2 (6.23), (6.24) bez ohybu; k_c (6.25)–(6.29); 6.1.4 (6.2)",
        ),
        "bearing" => (
            "Tlak kolmo na vlákna",
            "Tlak kolmo k vláknům",
            "EN 1995-1-1 6.1.5 (6.3); k_c,90 = 1",
        ),
        _ => return (name.to_owned(), "—"),
    };
    (language.says(slovak, czech).to_owned(), clauses)
}

/// Words of a check's place, in the order they are replaced.
const PLACE_WORDS: [(&str, [&str; 2]); 8] = [
    ("thinnest section", ["najtenší prierez", "nejtenčí průřez"]),
    ("net section", ["oslabený prierez", "oslabený průřez"]),
    ("at a notch", ["v záreze", "v zářezu"]),
    ("top free", ["horný okraj voľný", "horní okraj volný"]),
    ("bottom free", ["dolný okraj voľný", "dolní okraj volný"]),
    ("cantilever", ["konzola", "konzola"]),
    ("span", ["pole", "pole"]),
    ("length", ["dĺžka", "délka"]),
];

/// Where a check governs. A bearing or a notch names the parts it is at,
/// which are kept as written.
fn place(check: &Check, language: Language) -> String {
    if matches!(check.name, "bearing" | "shear_notch") {
        return check.at.clone();
    }
    let index = usize::from(language == Language::Czech);
    let mut at = check.at.clone();
    for (word, said) in PLACE_WORDS {
        at = at.replace(word, said[index]);
    }
    decimal_commas(&at)
}

/// A combination label ("1.35 permanent + 1.5 snow") with the load kinds named.
fn combination(label: &str, language: Language) -> String {
    let terms: Vec<String> = label
        .split(" + ")
        .map(|term| match term.split_once(' ') {
            Some((factor, kind)) => {
                format!("{} · {}", decimal_commas(factor), kind_name(kind, language))
            }
            None => kind_name(term, language),
        })
        .collect();
    terms.join(" + ")
}

/// `text` with the decimal point of its numbers as a comma.
fn decimal_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    chars
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let between_digits = i > 0
                && chars[i - 1].is_ascii_digit()
                && chars.get(i + 1).is_some_and(char::is_ascii_digit);
            if *c == '.' && between_digits { ',' } else { *c }
        })
        .collect()
}

fn number(value: f64, decimals: usize) -> String {
    decimal_commas(&format!("{value:.decimals$}"))
}

fn kilonewtons(newtons: f64) -> String {
    number(newtons / 1000.0, 2)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn status(status: &str, language: Language) -> &'static str {
    match status {
        "pass" => language.says("vyhovuje", "vyhovuje"),
        "fail" => language.says("nevyhovuje", "nevyhovuje"),
        _ => language.says("neoverené", "neověřeno"),
    }
}

fn loads_line(loads: &Loads, language: Language) -> String {
    let terms: Vec<String> = LOAD_KINDS
        .iter()
        .filter_map(|kind| loads.get(*kind).map(|value| (*kind, *value)))
        .chain(
            loads
                .iter()
                .filter(|(kind, _)| !LOAD_KINDS.contains(&kind.as_str()))
                .map(|(kind, value)| (kind.as_str(), *value)),
        )
        .map(|(kind, value)| {
            format!(
                "{} {} kN",
                escape(&kind_name(kind, language)),
                kilonewtons(value)
            )
        })
        .collect();
    if terms.is_empty() {
        "—".to_owned()
    } else {
        terms.join("; ")
    }
}

const STYLE: &str = "@page { size: A4; margin: 18mm 15mm; }
body { font-family: 'DejaVu Sans', Arial, sans-serif; font-size: 10pt; color: #000; }
h1 { font-size: 16pt; margin: 0 0 4pt; } h2 { font-size: 12pt; margin: 14pt 0 4pt; }
h3 { font-size: 11pt; margin: 10pt 0 3pt; }
table { border-collapse: collapse; width: 100%; margin: 3pt 0 6pt; }
th, td { border: 0.5pt solid #444; padding: 2pt 4pt; text-align: left; vertical-align: top; }
th { background: #eee; } td.n { text-align: right; white-space: nowrap; }
section.member { break-inside: avoid; page-break-inside: avoid; }
.fail { font-weight: bold; } .note { font-size: 9pt; }
ol.basis { font-size: 8.5pt; }";

/// The calculation of `report` for the program `title`, as one HTML page set.
#[must_use]
pub fn document(title: &str, report: &Report, language: Language) -> String {
    let l = language;
    let mut out = String::new();
    write!(
        out,
        "<!DOCTYPE html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n<title>{} — {}</title>\n<style>\n{STYLE}\n</style>\n</head>\n<body>\n",
        l.code(),
        l.says("Statický výpočet", "Statický výpočet"),
        escape(title)
    )
    .expect("a String takes every write");
    write!(
        out,
        "<h1>{}</h1>\n<p>{}: <b>{}</b><br>{}</p>\n",
        l.says("Statický výpočet", "Statický výpočet"),
        l.says("Program", "Program"),
        escape(title),
        l.says(
            "Príloha k dokumentácii pre stavebné povolenie. Vypočítaná kontrola, nie autorizovaný statický návrh: platí až po overení a podpise autorizovaným stavebným inžinierom.",
            "Příloha k dokumentaci pro stavební povolení. Vypočtená kontrola, nikoli autorizovaný statický návrh: platí až po ověření a podpisu autorizovaným inženýrem."
        )
    )
    .expect("a String takes every write");
    signatures(&mut out, l);
    scope(&mut out, report, l);
    let design = &report.design;
    let summary = design.summary();
    write!(
        out,
        "<h2>{}</h2>\n<table>\n<tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>\n<tr><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>\n</table>\n",
        l.says("Prehľad prvkov", "Přehled prvků"),
        l.says("Prvkov", "Prvků"),
        l.says("Vyhovuje", "Vyhovuje"),
        l.says("Nevyhovuje", "Nevyhovuje"),
        l.says("Neoverené", "Neověřeno"),
        summary.members,
        summary.pass,
        summary.fail,
        summary.not_verified
    )
    .expect("a String takes every write");
    if design.members.is_empty() {
        writeln!(
            out,
            "<p>{}</p>",
            l.says(
                "Program nemá žiadny nosný prvok s triedou pevnosti dreva (load_path a timber_strength).",
                "Program nemá žádný nosný prvek s třídou pevnosti dřeva (load_path a timber_strength)."
            )
        )
    .expect("a String takes every write");
    }
    for (index, member) in design.members.iter().enumerate() {
        let load = report.loads.members.iter().find(|m| m.part == member.part);
        member_section(&mut out, index + 1, member, load, l);
    }
    joints(&mut out, report, l);
    write!(
        out,
        "<h2>{}</h2>\n<p class=\"note\">{}</p>\n<ol class=\"basis\">\n",
        l.says("Podklady výpočtu", "Podklady výpočtu"),
        l.says(
            "Znenie podkladov, ako ich uvádza kontrola (v angličtine):",
            "Znění podkladů, jak je uvádí kontrola (v angličtině):"
        )
    )
    .expect("a String takes every write");
    for line in &design.basis {
        writeln!(out, "<li>{}</li>", escape(line)).expect("a String takes every write");
    }
    out.push_str("</ol>\n</body>\n</html>\n");
    out
}

fn signatures(out: &mut String, l: Language) {
    write!(
        out,
        "<table>\n<tr><th>{}</th><td></td><th>{}</th><td></td></tr>\n<tr><th>{}</th><td></td><th>{}</th><td></td></tr>\n</table>\n",
        l.says("Stavba", "Stavba"),
        l.says("Dátum", "Datum"),
        l.says("Vypracoval", "Vypracoval"),
        l.says("Overil a podpísal", "Ověřil a podepsal")
    )
    .expect("a String takes every write");
}

fn scope(out: &mut String, report: &Report, l: Language) {
    write!(
        out,
        "<h2>{}</h2>\n<ul>\n<li>{}</li>\n<li>{}</li>\n<li>{}</li>\n",
        l.says("Rozsah a predpoklady", "Rozsah a předpoklady"),
        l.says(
            "Posúdené je len zvislé (gravitačné) zaťaženie: stále, úžitkové, strecha a sneh. Vietor, priestorová stabilita (stuženie) a ohyb s osovou silou posúdené nie sú.",
            "Posouzeno je pouze svislé (gravitační) zatížení: stálé, užitné, střecha a sníh. Vítr, prostorová stabilita (ztužení) a ohyb s osovou silou posouzeny nejsou."
        ),
        l.says(
            "Medzné stavy únosnosti: EN 1990 6.4.3.2 (6.10), 1,35 G + 1,5 Q1 + 1,5 ψ0 Qi; použiteľnosť: charakteristická kombinácia EN 1990 6.5.3 (6.14b), w_fin s k_def a ψ2.",
            "Mezní stavy únosnosti: EN 1990 6.4.3.2 (6.10), 1,35 G + 1,5 Q1 + 1,5 ψ0 Qi; použitelnost: charakteristická kombinace EN 1990 6.5.3 (6.14b), w_fin s k_def a ψ2."
        ),
        l.says(
            "Drevené prvky sú posúdené podľa EN 1995-1-1 s k_mod, k_def a γ_M výrobku ich triedy; súčinitele sú odporúčané hodnoty EN, nie hodnoty národnej prílohy.",
            "Dřevěné prvky jsou posouzeny podle EN 1995-1-1 s k_mod, k_def a γ_M výrobku jejich třídy; součinitele jsou doporučené hodnoty EN, nikoli hodnoty národní přílohy."
        )
    )
    .expect("a String takes every write");
    if let Some(site) = report
        .design
        .basis
        .iter()
        .find_map(|line| line.strip_prefix("site: "))
    {
        writeln!(
            out,
            "<li>{}: {}</li>",
            l.says("Miesto stavby", "Místo stavby"),
            escape(site)
        )
        .expect("a String takes every write");
    }
    out.push_str("</ul>\n");
}

fn member_section(
    out: &mut String,
    number_in_list: usize,
    member: &MemberCheck,
    load: Option<&MemberLoad>,
    l: Language,
) {
    let role = if member.role == "column" {
        l.says("stĺp", "sloup")
    } else {
        l.says("nosník", "nosník")
    };
    write!(
        out,
        "<section class=\"member\">\n<h3>{number_in_list}. {} ({role}) — <span class=\"{}\">{}</span></h3>\n<table>\n",
        escape(&member.part),
        member.status,
        status(member.status, l)
    )
    .expect("a String takes every write");
    let row = |out: &mut String, name: &str, value: &str| {
        writeln!(out, "<tr><th>{name}</th><td>{value}</td></tr>")
            .expect("a String takes every write");
    };
    row(
        out,
        l.says("Trieda pevnosti", "Třída pevnosti"),
        &member
            .strength_class
            .as_deref()
            .map_or_else(|| "—".to_owned(), escape),
    );
    row(
        out,
        l.says("Prierez b × h", "Průřez b × h"),
        &member.section_mm.map_or_else(
            || "—".to_owned(),
            |[b, h]| format!("{} × {} mm", number(b, 0), number(h, 0)),
        ),
    );
    if let Some(load) = load {
        row(
            out,
            l.says("Dĺžka", "Délka"),
            &format!("{} mm", number(load.length_mm, 0)),
        );
        row(
            out,
            l.says(
                "Charakteristické zaťaženie spolu",
                "Charakteristické zatížení celkem",
            ),
            &loads_line(&load.loads_n, l),
        );
        let supports: Vec<String> = load
            .reactions
            .iter()
            .map(|reaction| {
                format!(
                    "{} {} {} mm: {}{}",
                    escape(&reaction.part),
                    l.says("pri", "v"),
                    number(reaction.at_mm, 0),
                    loads_line(&reaction.loads_n, l),
                    if reaction.lifted_off {
                        l.says(" (odľahčená)", " (odlehčená)")
                    } else {
                        ""
                    }
                )
            })
            .collect();
        row(
            out,
            l.says("Podpory a reakcie", "Podpory a reakce"),
            &if supports.is_empty() {
                "—".to_owned()
            } else {
                supports.join("<br>")
            },
        );
    }
    out.push_str("</table>\n");
    if !member.checks.is_empty() {
        write!(
            out,
            "<table>\n<tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>\n",
            l.says("Posudok", "Posudek"),
            l.says("Články a vzorce", "Články a vzorce"),
            l.says("Rozhodujúca kombinácia", "Rozhodující kombinace"),
            l.says("Miesto", "Místo"),
            l.says("Využitie", "Využití")
        )
        .expect("a String takes every write");
        for check in &member.checks {
            let (name, clauses) = check_name(check.name, l);
            let governing = member.governing.as_deref() == Some(check.name);
            writeln!(
                out,
                "<tr><td>{}{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"n{}\">{}</td></tr>",
                escape(&name),
                if governing {
                    l.says(" (rozhoduje)", " (rozhoduje)")
                } else {
                    ""
                },
                clauses,
                escape(&combination(&check.combination, l)),
                escape(&place(check, l)),
                if check.utilization > 1.0 { " fail" } else { "" },
                number(check.utilization, 3)
            )
            .expect("a String takes every write");
        }
        out.push_str("</table>\n");
    }
    held(out, load, &member.bracing, l);
    if !member.missing.is_empty() {
        write!(
            out,
            "<p><b>{}</b></p>\n<ul class=\"note\">\n",
            l.says(
                "Neoverené, chýba (znenie kontroly, v angličtine):",
                "Neověřeno, chybí (znění kontroly, v angličtině):"
            )
        )
        .expect("a String takes every write");
        for reason in &member.missing {
            writeln!(out, "<li>{}</li>", escape(reason)).expect("a String takes every write");
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</section>\n");
}

/// The stretches of the member's edges that parts hold sideways, and what
/// the braces of each compressed edge must take.
fn held(out: &mut String, load: Option<&MemberLoad>, demands: &[BracingDemand], l: Language) {
    let edge = |edge: &str| {
        if edge == "top" {
            l.says("horný", "horní")
        } else {
            l.says("dolný", "dolní")
        }
    };
    let braces = load.map_or(&[][..], |load| load.braced_by.as_slice());
    write!(
        out,
        "<p><b>{}</b> ",
        l.says(
            "Zaistené úseky (okraj držaný do strany):",
            "Zajištěné úseky (okraj držený do strany):"
        )
    )
    .expect("a String takes every write");
    if braces.is_empty() {
        writeln!(
            out,
            "{}</p>",
            l.says(
                "žiadna časť nedrží okraj prvku do strany; klopenie sa posudzuje medzi podperami.",
                "žádná část nedrží okraj prvku do strany; klopení se posuzuje mezi podporami."
            )
        )
        .expect("a String takes every write");
    } else {
        let stretches: Vec<String> = braces
            .iter()
            .map(|brace| {
                let at = if brace.to_mm - brace.from_mm < 0.5 {
                    format!("{} mm", number(brace.from_mm, 0))
                } else {
                    format!("{}–{} mm", number(brace.from_mm, 0), number(brace.to_mm, 0))
                };
                format!(
                    "{} {} {at} ({})",
                    edge(&brace.edge),
                    l.says("okraj", "okraj"),
                    escape(&brace.part)
                )
            })
            .collect();
        writeln!(out, "{}</p>", stretches.join("; ")).expect("a String takes every write");
    }
    if !demands.is_empty() {
        write!(
            out,
            "<table>\n<tr><th>{}</th><th>{}</th><th>N_d (9.2.5.2(4))</th><th>F_d (9.35, 9.36)</th><th>C (9.34)</th></tr>\n",
            l.says("Okraj", "Okraj"),
            l.says("Drží ho", "Drží ho"),
        )
    .expect("a String takes every write");
        for demand in demands {
            writeln!(
                out,
                "<tr><td>{}</td><td>{}</td><td class=\"n\">{} kN</td><td class=\"n\">{} kN</td><td class=\"n\">{}</td></tr>",
                edge(&demand.edge),
                escape(&demand.by.join(", ")),
                kilonewtons(demand.axial_n),
                kilonewtons(demand.force_n),
                demand
                    .stiffness_n_mm
                    .map_or_else(|| "—".to_owned(), |c| format!("{} N/mm", number(c, 1)))
            )
    .expect("a String takes every write");
        }
        writeln!(
            out,
            "</table>\n<p class=\"note\">{}</p>",
            l.says(
                "Výstuhy, ich ukotvenie a spojovacie prostriedky na tieto sily posúdené nie sú.",
                "Výztuhy, jejich ukotvení a spojovací prostředky na tyto síly posouzeny nejsou."
            )
        )
        .expect("a String takes every write");
    }
}

fn joints(out: &mut String, report: &Report, l: Language) {
    if report.joints.is_empty() {
        return;
    }
    write!(
        out,
        "<h2>{}</h2>\n<table>\n<tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>\n",
        l.says("Spoje", "Spoje"),
        l.says("Spoj", "Spoj"),
        l.says("Prvky", "Prvky"),
        l.says("Spojovací prostriedok", "Spojovací prostředek"),
        l.says("Kombinácia", "Kombinace"),
        l.says("Využitie", "Využití"),
        l.says("Výsledok", "Výsledek")
    )
    .expect("a String takes every write");
    for joint in &report.joints {
        writeln!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"n\">{}</td><td class=\"{}\">{}</td></tr>",
            escape(&joint.joint),
            escape(&joint.parts.join(", ")),
            joint
                .fastener
                .as_deref()
                .map_or_else(|| "—".to_owned(), escape),
            joint
                .combination
                .as_deref()
                .map_or_else(|| "—".to_owned(), |c| escape(&combination(c, l))),
            joint
                .utilization
                .map_or_else(|| "—".to_owned(), |u| number(u, 3)),
            joint.status,
            status(joint.status, l)
        )
    .expect("a String takes every write");
    }
    out.push_str("</table>\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_escaped_and_numbers_take_decimal_commas() {
        assert_eq!(escape("a<b>&\"c'"), "a&lt;b&gt;&amp;&quot;c&#39;");
        assert_eq!(
            decimal_commas("lef 6.5 mm, v1.2x, 3."),
            "lef 6,5 mm, v1,2x, 3."
        );
        assert_eq!(
            combination("1.35 permanent + 1.5 snow + 1.05 imposed", Language::Slovak),
            "1,35 · stále (G) + 1,5 · sneh + 1,05 · úžitkové, kat. A"
        );
        assert_eq!(combination("permanent", Language::Czech), "stálé (G)");
        let check = Check {
            name: "bearing",
            utilization: 0.5,
            combination: String::new(),
            at: "span 1.5".to_owned(),
        };
        assert_eq!(place(&check, Language::Slovak), "span 1.5");
        let check = Check {
            name: "bending",
            ..check
        };
        assert_eq!(place(&check, Language::Czech), "pole 1,5");
    }
}
