//! Material takeoff: the counted parts grouped by category, material and
//! cross-section, with piece counts, running length, area and volume.
//!
//! Sizes are the part's blank (as in the cut list). The volume is the exact
//! solid's when the caller knows it, else the blank's; every row says which.
//! The area follows the same basis.

use crate::model::{Part, ProgramModel};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Where a row's volume comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeBasis {
    /// Every part's exact solid volume.
    Exact,
    /// Every part's blank (bounding box in its own frame).
    Blank,
    /// Exact for some parts, blank for the rest.
    Mixed,
}

impl VolumeBasis {
    fn of(exact: usize, count: usize) -> Self {
        match exact {
            0 => Self::Blank,
            exact if exact == count => Self::Exact,
            _ => Self::Mixed,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Blank => "blank",
            Self::Mixed => "mixed",
        }
    }
}

/// Parts of one category and material with the same cross-section.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TakeoffRow {
    pub category: String,
    pub material: String,
    /// The blank's two shorter sides, larger first: width and thickness.
    pub section_mm: [f64; 2],
    pub count: usize,
    /// Sum of the blanks' longest sides.
    pub length_m: f64,
    /// Sum of the largest faces: exact volume over thickness when the exact
    /// solid is known, else the blank's length × width.
    pub area_m2: f64,
    pub volume_m3: f64,
    pub volume_basis: VolumeBasis,
    pub parts: Vec<String>,
}

/// All counted parts of one material.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MaterialTotal {
    pub material: String,
    pub count: usize,
    pub length_m: f64,
    pub area_m2: f64,
    pub volume_m3: f64,
    pub volume_basis: VolumeBasis,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Takeoff {
    pub rows: Vec<TakeoffRow>,
    pub materials: Vec<MaterialTotal>,
    pub counted_parts: usize,
    /// Parts of the model left out, e.g. on hidden layers.
    pub excluded_parts: usize,
    /// Parts named in `counted` that the program did not make (drawn or
    /// imported): the takeoff cannot size them, so it says how many.
    pub outside_program_parts: usize,
    pub exact_volume_parts: usize,
}

/// The part's category: its `category` attribute, else the first segment of
/// a path name (`strecha/krokva 3` → `strecha`), else the name without its
/// trailing number (`Základy · pätky 001` → `Základy · pätky`, `Box #2` → `Box`).
#[must_use]
pub fn category(part: &Part) -> &str {
    if let Some(category) = part.attributes.get("category") {
        return category;
    }
    if let Some((first, _)) = part.name.split_once('/') {
        return first;
    }
    let unnumbered = part
        .name
        .trim_end_matches(|c: char| c.is_ascii_digit() || c == '#');
    let base = unnumbered.trim_end();
    if unnumbered.len() < part.name.len() && base.len() < unnumbered.len() && !base.is_empty() {
        base
    } else {
        &part.name
    }
}

#[derive(Default)]
struct Sums {
    count: usize,
    exact: usize,
    length_mm: f64,
    area_mm2: f64,
    volume_mm3: f64,
}

impl Sums {
    fn add(&mut self, sides: [f64; 3], exact_volume_mm3: Option<f64>) {
        self.count += 1;
        self.length_mm += sides[0];
        // A triangular part has half its blank's face: with the exact
        // solid known, the face is its volume over the thickness.
        self.area_mm2 += match exact_volume_mm3 {
            Some(volume) if sides[2] > 0.0 => volume / sides[2],
            _ => sides[0] * sides[1],
        };
        self.volume_mm3 += exact_volume_mm3.unwrap_or(sides[0] * sides[1] * sides[2]);
        self.exact += usize::from(exact_volume_mm3.is_some());
    }
}

fn round(value: f64, digits: i32) -> f64 {
    let scale = 10_f64.powi(digits);
    (value * scale).round() / scale + 0.0
}

fn key(value: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    {
        (value * 10.0).round() as i64
    }
}

/// The takeoff of the parts named in `counted`, each with its exact solid
/// volume in mm³ when known. Parts not named are excluded.
#[must_use]
pub fn material_takeoff(model: &ProgramModel, counted: &BTreeMap<String, Option<f64>>) -> Takeoff {
    type RowKey = (String, String, [i64; 2]);
    let mut rows: BTreeMap<RowKey, (TakeoffRow, Sums)> = BTreeMap::new();
    let mut materials: BTreeMap<String, Sums> = BTreeMap::new();
    let mut counted_parts = 0;
    for part in &model.parts {
        let Some(exact_volume) = counted.get(&part.name) else {
            continue;
        };
        let exact_volume = exact_volume.filter(|volume| volume.is_finite() && *volume > 0.0);
        counted_parts += 1;
        let (min, max) = part.local_bounds();
        let mut sides: [f64; 3] = std::array::from_fn(|axis| max[axis] - min[axis]);
        sides.sort_by(|left, right| right.total_cmp(left));
        let material = part
            .material
            .clone()
            .unwrap_or_else(|| "unspecified".to_owned());
        let category = category(part).to_owned();
        let section = [sides[1], sides[2]];
        let (row, sums) = rows
            .entry((category.clone(), material.clone(), section.map(key)))
            .or_insert_with(|| {
                (
                    TakeoffRow {
                        category,
                        material: material.clone(),
                        section_mm: section.map(|side| round(side, 1)),
                        count: 0,
                        length_m: 0.0,
                        area_m2: 0.0,
                        volume_m3: 0.0,
                        volume_basis: VolumeBasis::Blank,
                        parts: Vec::new(),
                    },
                    Sums::default(),
                )
            });
        row.parts.push(part.name.clone());
        sums.add(sides, exact_volume);
        materials
            .entry(material)
            .or_default()
            .add(sides, exact_volume);
    }
    let rows: Vec<TakeoffRow> = rows
        .into_values()
        .map(|(mut row, sums)| {
            row.count = sums.count;
            row.length_m = round(sums.length_mm / 1.0e3, 3);
            row.area_m2 = round(sums.area_mm2 / 1.0e6, 3);
            row.volume_m3 = round(sums.volume_mm3 / 1.0e9, 4);
            row.volume_basis = VolumeBasis::of(sums.exact, sums.count);
            row
        })
        .collect();
    let exact_volume_parts = materials.values().map(|sums| sums.exact).sum();
    Takeoff {
        rows,
        materials: materials
            .into_iter()
            .map(|(material, sums)| MaterialTotal {
                material,
                count: sums.count,
                length_m: round(sums.length_mm / 1.0e3, 3),
                area_m2: round(sums.area_mm2 / 1.0e6, 3),
                volume_m3: round(sums.volume_mm3 / 1.0e9, 4),
                volume_basis: VolumeBasis::of(sums.exact, sums.count),
            })
            .collect(),
        counted_parts,
        excluded_parts: model.parts.len() - counted_parts,
        outside_program_parts: counted.len().saturating_sub(counted_parts),
        exact_volume_parts,
    }
}

/// The takeoff of every part, with blank volumes.
#[must_use]
pub fn material_takeoff_of_all(model: &ProgramModel) -> Takeoff {
    let counted = model
        .parts
        .iter()
        .map(|part| (part.name.clone(), None))
        .collect();
    material_takeoff(model, &counted)
}

/// A text cell. A leading `=`, `+`, `-`, `@` or control character would make
/// a spreadsheet run the name as a formula, so such a name is written after an
/// apostrophe, which the spreadsheet shows as plain text.
fn csv_field(text: &str) -> String {
    let text = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_owned()
    };
    if text.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// A number with a decimal comma, which is what a spreadsheet that splits
/// columns on `;` reads as a number.
fn csv_number(value: f64) -> String {
    value.to_string().replace('.', ",")
}

/// Semicolon-separated rows, then one total per material, for spreadsheets:
/// UTF-8 with a byte order mark (so diacritics survive), decimal commas and
/// the rounded values of the takeoff.
#[must_use]
pub fn takeoff_csv(takeoff: &Takeoff) -> String {
    let mut csv = String::from(
        "\u{feff}category;material;width_mm;thickness_mm;count;length_m;area_m2;volume_m3;volume_basis\n",
    );
    for row in &takeoff.rows {
        let _ = writeln!(
            csv,
            "{};{};{};{};{};{};{};{};{}",
            csv_field(&row.category),
            csv_field(&row.material),
            csv_number(row.section_mm[0]),
            csv_number(row.section_mm[1]),
            row.count,
            csv_number(row.length_m),
            csv_number(row.area_m2),
            csv_number(row.volume_m3),
            row.volume_basis.as_str()
        );
    }
    for total in &takeoff.materials {
        let _ = writeln!(
            csv,
            "TOTAL;{};;;{};{};{};{};{}",
            csv_field(&total.material),
            total.count,
            csv_number(total.length_m),
            csv_number(total.area_m2),
            csv_number(total.volume_m3),
            total.volume_basis.as_str()
        );
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
box("stena/stĺpik 1", (60, 140, 2500), material = "drevo")
box("stena/stĺpik 2", (60, 140, 2500), at = (600, 0, 0), material = "drevo")
box("stena/doska", (1200, 15, 2500), at = (0, 200, 0), material = "OSB")
box("strecha/krokva", (60, 4000, 180), at = (0, 0, 3000), material = "drevo")
"#;

    fn model() -> ProgramModel {
        crate::evaluate("takeoff.star", SOURCE, &BTreeMap::new())
            .expect("program evaluates")
            .model
    }

    #[test]
    fn groups_by_category_material_and_section_with_lengths_areas_and_volumes() {
        let takeoff = material_takeoff_of_all(&model());
        assert_eq!(takeoff.counted_parts, 4);
        assert_eq!(takeoff.excluded_parts, 0);
        let studs = takeoff
            .rows
            .iter()
            .find(|row| row.category == "stena" && row.material == "drevo")
            .unwrap();
        assert_eq!(studs.count, 2);
        assert_eq!(studs.section_mm, [140.0, 60.0]);
        assert_eq!(studs.length_m, 5.0);
        assert_eq!(studs.area_m2, 0.7);
        assert_eq!(studs.volume_m3, 0.042);
        assert_eq!(studs.volume_basis, VolumeBasis::Blank);
        let rafter = takeoff
            .rows
            .iter()
            .find(|row| row.category == "strecha")
            .unwrap();
        assert_eq!((rafter.count, rafter.section_mm), (1, [180.0, 60.0]));
        let wood = takeoff
            .materials
            .iter()
            .find(|total| total.material == "drevo")
            .unwrap();
        assert_eq!(wood.count, 3);
        assert_eq!(wood.volume_m3, 0.0852);
        let osb = takeoff
            .materials
            .iter()
            .find(|total| total.material == "OSB")
            .unwrap();
        assert_eq!((osb.area_m2, osb.volume_m3), (3.0, 0.045));
    }

    #[test]
    fn a_name_without_a_path_is_grouped_without_its_trailing_number() {
        let model = crate::evaluate(
            "numbered.star",
            r#"
box("Prah 001", (60, 140, 2500), material = "drevo")
box("Prah 002", (60, 140, 2500), at = (600, 0, 0), material = "drevo")
box("Box-1 #3", (100, 100, 100), at = (0, 600, 0), material = "drevo")
box("Stĺpik2", (100, 100, 100), at = (600, 600, 0), material = "drevo")
"#,
            &BTreeMap::new(),
        )
        .unwrap()
        .model;
        let categories = model.parts.iter().map(category).collect::<Vec<_>>();
        assert_eq!(categories, ["Prah", "Prah", "Box-1", "Stĺpik2"]);
        let takeoff = material_takeoff_of_all(&model);
        let sills = takeoff
            .rows
            .iter()
            .find(|row| row.category == "Prah")
            .unwrap();
        assert_eq!((sills.count, sills.length_m), (2, 5.0));
    }

    #[test]
    fn excluded_parts_are_not_counted_and_exact_volumes_replace_blanks() {
        let counted = BTreeMap::from([
            ("stena/stĺpik 1".to_owned(), Some(20_000_000.0)),
            ("stena/stĺpik 2".to_owned(), None),
        ]);
        let takeoff = material_takeoff(&model(), &counted);
        assert_eq!((takeoff.counted_parts, takeoff.excluded_parts), (2, 2));
        assert_eq!(takeoff.exact_volume_parts, 1);
        assert_eq!(takeoff.rows.len(), 1);
        let row = &takeoff.rows[0];
        assert_eq!(row.volume_basis, VolumeBasis::Mixed);
        assert_eq!(row.volume_m3, 0.041);
        let csv = takeoff_csv(&takeoff);
        assert!(csv.starts_with("\u{feff}category;material;width_mm"));
        assert!(csv.contains("stena;drevo;140;60;2;5;0,683;0,041;mixed\n"));
        assert!(csv.contains("TOTAL;drevo;;;2;5;0,683;0,041;mixed\n"));
    }

    /// Review 2026-10-09 (P3): the CSV had decimal points beside `;` columns,
    /// no byte order mark, and wrote a name like `=1+1` as a formula.
    #[test]
    fn the_csv_opens_in_a_spreadsheet_as_numbers_and_plain_names() {
        let source = "box(\"=HYPERLINK(1)/doska\", (1000, 200, 25), material = \"@drevo\")\n";
        let model = crate::evaluate("csv.star", source, &BTreeMap::new())
            .expect("program evaluates")
            .model;
        let csv = takeoff_csv(&material_takeoff_of_all(&model));
        assert!(csv.starts_with('\u{feff}'), "{csv}");
        assert!(
            csv.contains("\n'=HYPERLINK(1);'@drevo;200;25;1;1;0,2;0,005;blank\n"),
            "{csv}"
        );
        assert!(
            csv.contains("\nTOTAL;'@drevo;;;1;1;0,2;0,005;blank\n"),
            "{csv}"
        );
    }

    /// Review 2026-10-09 (P3): a triangular part counted twice its area.
    #[test]
    fn an_exact_triangular_part_counts_its_own_area_not_its_blank() {
        let half_blank = 1200.0 * 15.0 * 2500.0 / 2.0;
        let counted = BTreeMap::from([("stena/doska".to_owned(), Some(half_blank))]);
        let takeoff = material_takeoff(&model(), &counted);
        let osb = &takeoff.materials[0];
        assert_eq!((osb.area_m2, osb.volume_m3), (1.5, 0.0225));
    }

    /// Review 2026-10-09 (P3): a visible part the program did not make was
    /// left out without a word.
    #[test]
    fn a_visible_part_not_made_by_the_program_is_reported_not_dropped() {
        let counted = BTreeMap::from([
            ("stena/doska".to_owned(), None),
            ("nakreslený kváder".to_owned(), Some(1.0e6)),
        ]);
        let takeoff = material_takeoff(&model(), &counted);
        assert_eq!(
            (takeoff.counted_parts, takeoff.outside_program_parts),
            (1, 1)
        );
    }
}
