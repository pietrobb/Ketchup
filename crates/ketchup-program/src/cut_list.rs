//! The cut list as a table: one row per group of identical parts (material,
//! blank dimensions and the program's own attributes), for spreadsheets and
//! cutting optimisers.

use crate::bom::key;
use crate::model::ProgramModel;
use crate::table::{Cell, Table};
use std::collections::{BTreeMap, BTreeSet};

const DIMENSIONS: [&str; 3] = ["length", "width", "thickness"];

/// The program's own attributes. Namespaced keys (`classification:…`,
/// `input:…`) are data for validators, not for the workshop.
fn plain_attribute(name: &str) -> bool {
    !name.contains(':')
}

/// An attribute naming a local axis ("x", "y", "z") is written as the cut-list
/// dimension that axis became, since the cut list orders the dimensions from
/// longest to shortest.
fn workshop_value(value: &str, axis_order: [usize; 3]) -> String {
    let axis = match value {
        "x" | "X" => 0,
        "y" | "Y" => 1,
        "z" | "Z" => 2,
        _ => return value.to_owned(),
    };
    let position = axis_order
        .iter()
        .position(|candidate| *candidate == axis)
        .expect("every axis is ordered");
    DIMENSIONS[position].to_owned()
}

struct Group {
    material: String,
    dimensions_mm: [f64; 3],
    parts: Vec<String>,
}

/// The counted parts (e.g. the visible ones) as cut-list rows: position,
/// part names, material, length, width and thickness of the blank in mm,
/// count, then one column per program attribute any counted part carries.
#[must_use]
pub fn cut_list_table(model: &ProgramModel, counted: impl Fn(&str) -> bool) -> Table {
    let parts: Vec<_> = model
        .parts
        .iter()
        .filter(|part| counted(&part.name))
        .collect();
    let attribute_names: BTreeSet<&str> = parts
        .iter()
        .flat_map(|part| part.attributes.keys())
        .map(String::as_str)
        .filter(|name| plain_attribute(name))
        .collect();
    let mut groups: BTreeMap<(String, [i64; 3], Vec<String>), Group> = BTreeMap::new();
    for part in parts {
        // The blank: grown by push_pull and union, unlike the declared size.
        let (min, max) = part.local_bounds();
        let extent: [f64; 3] = std::array::from_fn(|axis| max[axis] - min[axis]);
        let mut axis_order = [0, 1, 2];
        axis_order.sort_by(|left, right| extent[*right].total_cmp(&extent[*left]));
        let dimensions_mm = axis_order.map(|axis| extent[axis]);
        let material = part
            .material
            .clone()
            .unwrap_or_else(|| "unspecified".to_owned());
        let attributes = attribute_names
            .iter()
            .map(|name| {
                part.attributes
                    .get(*name)
                    .map(|value| workshop_value(value, axis_order))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        groups
            .entry((
                material.clone(),
                dimensions_mm.map(|value| -key(value)),
                attributes,
            ))
            .or_insert_with(|| Group {
                material,
                dimensions_mm,
                parts: Vec::new(),
            })
            .parts
            .push(part.name.clone());
    }
    let mut header: Vec<String> = ["position", "parts", "material"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    header.extend(DIMENSIONS.map(|dimension| format!("{dimension}_mm")));
    header.push("count".to_owned());
    header.extend(attribute_names.iter().map(|name| (*name).to_owned()));
    let rows = groups
        .into_iter()
        .enumerate()
        .map(|(index, ((_, _, attributes), group))| {
            let mut row = vec![
                Cell::Count(index + 1),
                Cell::Text(group.parts.join(", ")),
                Cell::Text(group.material),
            ];
            row.extend(group.dimensions_mm.map(Cell::Number));
            row.push(Cell::Count(group.parts.len()));
            row.extend(attributes.into_iter().map(Cell::Text));
            row
        })
        .collect();
    Table { header, rows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::DecimalSeparator;

    const SOURCE: &str = r#"
board("side_l", [18, 560, 720], material = "oak", grain = "z", edges = "L1")
board("side_r", [18, 560, 720], at = (500, 0, 0), material = "oak", grain = "z", edges = "L1")
board("top", [536, 560, 18], at = (18, 0, 702), material = "oak", grain = "x")
box("hidden", [10, 10, 10], at = (0, 0, 900), material = "steel", attributes = {"classification:x": "y"})
"#;

    #[test]
    fn one_row_per_identical_part_with_axes_written_as_cut_list_dimensions() {
        let model = crate::evaluate("cut.star", SOURCE, &BTreeMap::new())
            .expect("program evaluates")
            .model;
        let table = cut_list_table(&model, |name| name != "hidden");
        assert_eq!(
            table.csv(DecimalSeparator::Comma),
            "\u{feff}position;parts;material;length_mm;width_mm;thickness_mm;count;edges;grain\n\
             1;side_l, side_r;oak;720;560;18;2;L1;length\n\
             2;top;oak;560;536;18;1;;width\n"
        );
        // Namespaced attributes are validator data, not columns.
        let all = cut_list_table(&model, |_| true);
        assert!(
            !all.header.iter().any(|name| name.contains(':')),
            "{:?}",
            all.header
        );
        assert_eq!(all.rows.len(), 3);
    }

    #[test]
    fn edges_must_be_text() {
        let Err(error) = crate::evaluate(
            "cut.star",
            "board(\"b\", [10, 10, 10], edges = 2)\n",
            &BTreeMap::new(),
        ) else {
            panic!("a number is refused");
        };
        let error = error.to_string();
        assert!(error.contains("edges must be text"), "{error}");
    }
}
