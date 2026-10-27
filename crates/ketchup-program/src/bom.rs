//! Cut list, hardware list and machining plan derived from a model.

use crate::cut_list::plain_attribute;
use crate::model::ProgramModel;
use crate::table::{Cell, Table};
use ketchup_model::document::HoleThread;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One cut-list row: identical parts (material and dimensions) grouped.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CutListRow {
    pub material: String,
    /// Dimensions sorted from longest to shortest (length, width, thickness).
    pub dimensions_mm: [f64; 3],
    pub count: usize,
    pub parts: Vec<String>,
}

/// Bought items: a joint's named fasteners, and the parts ordered by their
/// designation, grouped by designation and their other attributes.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct HardwareRow {
    pub item: String,
    pub count: usize,
    /// The designated parts this row counts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
    /// Their program attributes besides the designation, e.g. the standard.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
}

/// Bars cut (and bent) to length, e.g. reinforcement, ordered as pieces:
/// identical pieces (grade, diameter, shape, cut length) are one position.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct BarRow {
    pub position: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diameter_mm: Option<f64>,
    pub grade: String,
    /// The bends, "straight" for none.
    pub shape: String,
    /// Developed length of one piece.
    pub cut_length_mm: f64,
    pub count: usize,
    pub total_length_m: f64,
    /// Derived from the pieces' `mass_kg`, not what the bars are ordered by;
    /// omitted when a piece names no mass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mass_kg: Option<f64>,
    pub parts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Operation {
    pub id: String,
    #[serde(deserialize_with = "crate::wire_names::name")]
    pub kind: crate::wire_names::Name,
    pub face: String,
    /// Holes: face coordinates of the centre. Pockets: (u_min, v_min, u_max, v_max).
    pub position_mm: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diameter_mm: Option<f64>,
    pub depth_mm: f64,
    /// Drilling intent; omitted for pockets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub through: Option<bool>,
    /// The thread tapped into a hole, e.g. "M8×1.25 depth 12".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct PartMachining {
    pub part: String,
    pub size_mm: [f64; 3],
    pub operations: Vec<Operation>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Bom {
    pub cut_list: Vec<CutListRow>,
    pub hardware: Vec<HardwareRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bar_schedule: Vec<BarRow>,
    pub machining: Vec<PartMachining>,
    pub total_parts: usize,
    /// The cut list's parts on the stock declared for their material (`stock()`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cutting: Vec<crate::cutting::CuttingPlan>,
}

pub(crate) fn key(value: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    {
        (value * 1000.0).round() as i64
    }
}

type Attributes = BTreeMap<String, String>;

fn bought<'a>(
    hardware: &'a mut BTreeMap<(String, Attributes), HardwareRow>,
    item: &str,
    attributes: Attributes,
) -> &'a mut HardwareRow {
    hardware
        .entry((item.to_owned(), attributes.clone()))
        .or_insert_with(|| HardwareRow {
            item: item.to_owned(),
            count: 0,
            parts: Vec::new(),
            attributes,
        })
}

#[must_use]
pub fn bom(model: &ProgramModel) -> Bom {
    let mut rows: BTreeMap<(String, [i64; 3]), CutListRow> = BTreeMap::new();
    let mut hardware = BTreeMap::new();
    // (designation, bought item) already counted: its other pieces add parts only.
    let mut items: BTreeSet<(String, String)> = BTreeSet::new();
    let mut bars: BTreeMap<(i64, String, String, i64), BarRow> = BTreeMap::new();
    for part in model.parts.iter().filter(|part| model.in_production(part)) {
        let text = |name: &str| part.attributes.get(name).cloned().unwrap_or_default();
        let number = |name: &str| part.attributes.get(name)?.parse::<f64>().ok();
        if let Some(cut_length_mm) = part.cut_length_mm() {
            let diameter_mm = number("diameter_mm");
            let row = bars
                .entry((
                    diameter_mm.map_or(-1, key),
                    text("grade"),
                    text("shape"),
                    key(cut_length_mm),
                ))
                .or_insert_with(|| BarRow {
                    position: 0,
                    diameter_mm,
                    grade: text("grade"),
                    shape: text("shape"),
                    cut_length_mm,
                    count: 0,
                    total_length_m: 0.0,
                    mass_kg: Some(0.0),
                    parts: Vec::new(),
                });
            row.count += 1;
            // In mm until the rows are complete.
            row.total_length_m += cut_length_mm;
            row.mass_kg = row.mass_kg.zip(number("mass_kg")).map(|(sum, kg)| sum + kg);
            row.parts.push(part.name.clone());
            continue;
        }
        if let Some(designation) = part.designation() {
            let attributes = part
                .attributes
                .iter()
                .filter(|(name, _)| {
                    plain_attribute(name) && *name != "designation" && *name != "bought_item"
                })
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            let first_piece = part
                .bought_item()
                .is_none_or(|item| items.insert((designation.to_owned(), item.to_owned())));
            let row = bought(&mut hardware, designation, attributes);
            row.count += usize::from(first_piece);
            row.parts.push(part.name.clone());
            continue;
        }
        // The blank: grown by push_pull and union, unlike the declared size.
        let (min, max) = part.local_bounds();
        let mut dimensions: [f64; 3] = std::array::from_fn(|axis| max[axis] - min[axis]);
        dimensions.sort_by(|left, right| right.total_cmp(left));
        let material = part
            .material
            .clone()
            .unwrap_or_else(|| "unspecified".to_owned());
        let row = rows
            .entry((material.clone(), dimensions.map(key)))
            .or_insert_with(|| CutListRow {
                material,
                dimensions_mm: dimensions,
                count: 0,
                parts: Vec::new(),
            });
        row.count += 1;
        row.parts.push(part.name.clone());
    }
    for joint in model
        .joints
        .iter()
        .filter(|joint| model.joint_in_production(joint))
    {
        if let Some(fastener) = &joint.fastener {
            // A named fastener without positions is one piece, e.g. one hanger.
            bought(&mut hardware, fastener, BTreeMap::new()).count +=
                joint.fasteners_mm.len().max(1);
        }
    }
    let machining = model
        .parts
        .iter()
        .filter(|part| {
            model.in_production(part)
                && (part.holes().next().is_some() || part.pockets().next().is_some())
        })
        .map(|part| PartMachining {
            part: part.name.clone(),
            size_mm: part.size_mm,
            operations: part
                .holes()
                .map(|hole| Operation {
                    id: hole.id.clone(),
                    kind: "drill",
                    face: hole.face.clone(),
                    position_mm: hole.at_mm.to_vec(),
                    diameter_mm: Some(hole.diameter_mm),
                    depth_mm: hole.depth_mm,
                    through: Some(hole.through),
                    thread: hole.thread.as_ref().map(HoleThread::label),
                })
                .chain(part.pockets().map(|pocket| Operation {
                    id: pocket.id.clone(),
                    kind: "pocket",
                    face: pocket.face.clone(),
                    position_mm: pocket.rect_mm.to_vec(),
                    diameter_mm: None,
                    depth_mm: pocket.depth_mm,
                    through: None,
                    thread: None,
                }))
                .collect(),
        })
        .collect();
    let bar_schedule = bars
        .into_values()
        .enumerate()
        .map(|(index, row)| BarRow {
            position: index + 1,
            total_length_m: row.total_length_m / 1000.0,
            mass_kg: row.mass_kg.map(|kg| (kg * 1000.0).round() / 1000.0),
            ..row
        })
        .collect();
    let cut_list: Vec<CutListRow> = rows.into_values().collect();
    let cutting = crate::cutting::plans(
        model,
        cut_list
            .iter()
            .flat_map(|row| row.parts.iter().map(String::as_str)),
    );
    Bom {
        cut_list,
        cutting,
        hardware: hardware.into_values().collect(),
        bar_schedule,
        machining,
        total_parts: model
            .parts
            .iter()
            .filter(|part| model.in_production(part))
            .count(),
    }
}

/// A window or door the program buys as one item (library `window()`,
/// `door()`): its kind by the part tag "window" or "door", the bought item
/// it is, its designation and its pieces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Opening {
    pub kind: String,
    pub item: String,
    pub designation: String,
    pub parts: Vec<String>,
}

/// The windows and doors among the bought parts [`bom`] counts, in the order
/// they come; the hardware list has a row per designation counting them.
#[must_use]
pub fn openings(model: &ProgramModel) -> Vec<Opening> {
    let mut openings: Vec<Opening> = Vec::new();
    for part in model
        .parts
        .iter()
        .filter(|part| model.in_production(part) && part.cut_length_mm().is_none())
    {
        let (Some(designation), Some(item)) = (part.designation(), part.bought_item()) else {
            continue;
        };
        let Some(kind) = ["window", "door"]
            .into_iter()
            .find(|kind| part.tags.contains(*kind))
        else {
            continue;
        };
        match openings
            .iter_mut()
            .find(|opening| opening.item == item && opening.designation == designation)
        {
            Some(opening) => opening.parts.push(part.name.clone()),
            None => openings.push(Opening {
                kind: kind.to_owned(),
                item: item.to_owned(),
                designation: designation.to_owned(),
                parts: vec![part.name.clone()],
            }),
        }
    }
    openings
}

/// The hardware rows as a table for a spreadsheet: item, count, parts, then
/// one column per attribute any row has.
#[must_use]
pub fn hardware_table(rows: &[HardwareRow]) -> Table {
    let names: BTreeSet<&String> = rows.iter().flat_map(|row| row.attributes.keys()).collect();
    let mut header: Vec<String> = ["item", "count", "parts"].map(str::to_owned).into();
    header.extend(names.iter().map(|name| (*name).clone()));
    let rows =
        rows.iter()
            .map(|row| {
                let mut cells = vec![
                    Cell::Text(row.item.clone()),
                    Cell::Count(row.count),
                    Cell::Text(row.parts.join(", ")),
                ];
                cells.extend(names.iter().map(|name| {
                    Cell::Text(row.attributes.get(*name).cloned().unwrap_or_default())
                }));
                cells
            })
            .collect();
    Table { header, rows }
}

/// The bar schedule as a table for the bending shop, one row per position.
#[must_use]
pub fn bar_schedule_table(rows: &[BarRow]) -> Table {
    let header = [
        "position",
        "diameter_mm",
        "grade",
        "shape",
        "cut_length_mm",
        "count",
        "total_length_m",
        "mass_kg",
        "parts",
    ]
    .map(str::to_owned)
    .into();
    let optional = |value: Option<f64>| value.map_or(Cell::Text(String::new()), Cell::Number);
    let rows = rows
        .iter()
        .map(|row| {
            vec![
                Cell::Count(row.position),
                optional(row.diameter_mm),
                Cell::Text(row.grade.clone()),
                Cell::Text(row.shape.clone()),
                Cell::Number(row.cut_length_mm),
                Cell::Count(row.count),
                Cell::Number(row.total_length_m),
                optional(row.mass_kg),
                Cell::Text(row.parts.join(", ")),
            ]
        })
        .collect();
    Table { header, rows }
}
