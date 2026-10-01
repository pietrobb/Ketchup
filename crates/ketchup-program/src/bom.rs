//! Cut list, hardware list and machining plan derived from a model.

use crate::model::ProgramModel;
use serde::Serialize;
use std::collections::BTreeMap;

/// One cut-list row: identical parts (material and dimensions) grouped.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CutListRow {
    pub material: String,
    /// Dimensions sorted from longest to shortest (length, width, thickness).
    pub dimensions_mm: [f64; 3],
    pub count: usize,
    pub parts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HardwareRow {
    pub item: String,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Operation {
    pub id: String,
    pub kind: &'static str,
    pub face: String,
    /// Holes: face coordinates of the centre. Pockets: (u_min, v_min, u_max, v_max).
    pub position_mm: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diameter_mm: Option<f64>,
    pub depth_mm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PartMachining {
    pub part: String,
    pub size_mm: [f64; 3],
    pub operations: Vec<Operation>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Bom {
    pub cut_list: Vec<CutListRow>,
    pub hardware: Vec<HardwareRow>,
    pub machining: Vec<PartMachining>,
    pub total_parts: usize,
}

fn key(value: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    {
        (value * 1000.0).round() as i64
    }
}

#[must_use]
pub fn bom(model: &ProgramModel) -> Bom {
    let mut rows: BTreeMap<(String, [i64; 3]), CutListRow> = BTreeMap::new();
    for part in &model.parts {
        let mut dimensions = part.size_mm;
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
    let mut hardware: BTreeMap<String, usize> = BTreeMap::new();
    for joint in &model.joints {
        if let Some(fastener) = &joint.fastener {
            *hardware.entry(fastener.clone()).or_default() += joint.fasteners_mm.len();
        }
    }
    let machining = model
        .parts
        .iter()
        .filter(|part| part.holes().next().is_some() || part.pockets().next().is_some())
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
                })
                .chain(part.pockets().map(|pocket| Operation {
                    id: pocket.id.clone(),
                    kind: "pocket",
                    face: pocket.face.clone(),
                    position_mm: pocket.rect_mm.to_vec(),
                    diameter_mm: None,
                    depth_mm: pocket.depth_mm,
                }))
                .collect(),
        })
        .collect();
    Bom {
        cut_list: rows.into_values().collect(),
        hardware: hardware
            .into_iter()
            .map(|(item, count)| HardwareRow { item, count })
            .collect(),
        machining,
        total_parts: model.parts.len(),
    }
}
