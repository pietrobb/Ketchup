//! Mass, centre of mass and moments of inertia of parts and of their assembly,
//! from the exact solids the worker measured and each part's material density.
//!
//! A density comes from the rules (`validation_rules.json`), from the evaluated
//! program (`material_weight(…, kg_m3=…)`), read at query time, or from the
//! prelude's `MATERIAL_WEIGHTS`; it is never a default. Sources that disagree,
//! a weight per area or an `estimate:` source leave the part not verified. A part without one, without measured mass properties or with a
//! placement that scales it is listed with the reason and left out of the sum,
//! and the assembly then says it is partial.

use crate::validation_rules::{ValidationRules, occurrence_materials};
use ketchup_geometry::linalg::Affine3;
use ketchup_geometry::mass::Mass;
use ketchup_model::document::{InstancePath, ScaledPlacement, Snapshot};
use ketchup_model::exact_product::{ExactBodyPackage, ExactResultRegistry};
use ketchup_model::tolerance::ROUNDING;
use ketchup_program::ProgramModel;
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const CONVENTION: &str = "kg·mm² about the centre of mass in world axes, tensor convention: xx = ∫(y²+z²) dm, xy = −∫x·y dm (OCCT GProp MatrixOfInertia)";

/// The density of `material` with every source that names it, or why there is none.
fn density(
    material: &str,
    rules: &ValidationRules,
    program: Option<&ProgramModel>,
) -> Result<(f64, Vec<Value>), (String, Value)> {
    let mut sources = Vec::new();
    if let Some(density) = rules
        .materials
        .get(material)
        .and_then(|properties| properties.density.as_ref())
    {
        sources.push((
            density.kg_m3,
            json!({"from": "validation_rules.json", "kg_m3": density.kg_m3, "citation": density.source}),
        ));
    }
    let library = ketchup_program::library_material_weights()
        .ok()
        .and_then(|weights| weights.get(material));
    let weights = [
        (
            "program material_weight",
            program.and_then(|model| model.material_weights.get(material)),
        ),
        ("library prelude MATERIAL_WEIGHTS", library),
    ];
    for (from, weight) in weights {
        if let Some(weight) = weight
            && let Some(kg_m3) = weight.kg_m3
        {
            sources.push((
                kg_m3,
                json!({"from": from, "kg_m3": kg_m3, "citation": weight.source}),
            ));
        }
    }
    let listed = Value::Array(sources.iter().map(|(_, source)| source.clone()).collect());
    let per_area = weights
        .iter()
        .find_map(|(from, weight)| weight.filter(|weight| weight.kg_m2.is_some()).map(|_| from));
    let estimate = sources.iter().any(|(_, source)| {
        source["citation"]
            .as_str()
            .is_some_and(|citation| citation.starts_with("estimate:"))
    });
    match sources.as_slice() {
        [] => Err((
            match per_area {
                Some(from) => format!(
                    "{from} gives material {material:?} in kg/m², a weight per area, not a density"
                ),
                None => format!(
                    "no density for material {material:?}: give material_weight({material:?}, kg_m3=…, source=…) in the program"
                ),
            },
            listed,
        )),
        [(first, _), rest @ ..]
            if rest
                .iter()
                .any(|(other, _)| (other - first).abs() > ROUNDING * first.max(*other)) =>
        {
            Err((
                format!("the sources give different densities for material {material:?}"),
                listed,
            ))
        }
        _ if estimate => Err((
            format!(
                "estimate: the density of material {material:?} is an upper bound for loads, not a mean density"
            ),
            listed,
        )),
        [(first, _), ..] => Ok((
            *first,
            sources.into_iter().map(|(_, source)| source).collect(),
        )),
    }
}

fn mass_json(mass: &Mass) -> Value {
    let [xx, yy, zz, xy, xz, yz] = mass.inertia_components();
    json!({"mass_kg": mass.kg, "centroid_mm": mass.centroid_mm,
        "inertia_kg_mm2": {"xx": xx, "yy": yy, "zz": zz, "xy": xy, "xz": xz, "yz": yz},
        "principal_kg_mm2": mass.principal_kg_mm2()})
}

/// Mass properties of the visible parts (or of those under `scope`) and of
/// them together. `exact` is the registry bound to `snapshot`, `program` the
/// evaluated program when one owns the document.
#[must_use]
pub fn mass_report(
    snapshot: &Snapshot,
    exact: Option<&ExactResultRegistry>,
    program: Option<&ProgramModel>,
    rules: &ValidationRules,
    scope: Option<&BTreeSet<InstancePath>>,
) -> Value {
    let materials = occurrence_materials(snapshot).unwrap_or_default();
    let mut parts = Vec::new();
    let mut counted = Vec::new();
    let mut missing = Vec::new();
    for occurrence in snapshot.scene_query().into_iter().filter(|occurrence| {
        occurrence.visible
            && scope.is_none_or(|scope| {
                scope.iter().any(|path| {
                    *path == occurrence.instance_path
                        || (path.steps().is_empty()
                            && path.root_occurrence() == occurrence.instance_path.root_occurrence())
                })
            })
    }) {
        let material = materials.get(&occurrence.instance_path.root_occurrence());
        let mut row = json!({"occurrence_id": occurrence.instance_path.root_occurrence().0,
            "name": occurrence.occurrence_name, "material": material});
        let package = exact.and_then(|exact| exact.get_render(snapshot, occurrence.definition_id));
        let solid = match package.map(AsRef::as_ref) {
            None => Err((
                "not_computed",
                "the part has no exact solid yet: re-evaluate the model".to_owned(),
            )),
            Some(ExactBodyPackage::Imported(_)) => Err((
                "not_computed",
                "imported solids carry no mass properties yet".to_owned(),
            )),
            Some(ExactBodyPackage::Graph(graph)) => match &graph.mass {
                Some(unit) => Ok((graph.volume_mm3, **unit)),
                None if graph.volume_mm3 > 0.0 => Err((
                    "not_computed",
                    "the solid was measured before mass properties existed: re-evaluate the model"
                        .to_owned(),
                )),
                None => Err(("not_computed", "the body is not a closed solid".to_owned())),
            },
        };
        let density = match material {
            Some(material) => density(material, rules, program),
            None => Err(("the part has no material".to_owned(), json!([]))),
        };
        row["density"] = match &density {
            Ok((kg_m3, sources)) => json!({"kg_m3": kg_m3, "sources": sources}),
            Err((_, sources)) => json!({"sources": sources}),
        };
        // The solid's volume is of the unplaced definition; a scaling
        // placement would make it wrong, so no volume is reported then.
        let placed = ScaledPlacement::check(occurrence.occurrence_id, occurrence.transform)
            .map_err(|error| ("not_verified", error.to_string()));
        let result = placed.and(solid).and_then(|(volume_mm3, unit)| {
            row["volume_mm3"] = json!(volume_mm3);
            let (kg_m3, _) = density
                .as_ref()
                .map_err(|(reason, _)| ("not_verified", reason.clone()))?;
            unit.placed(
                volume_mm3,
                *kg_m3,
                Affine3::from_row_major(*occurrence.transform.matrix()),
            )
            .ok_or((
                "not_verified",
                "the placement scales or shears the part".to_owned(),
            ))
        });
        match result {
            Ok(mass) => {
                row["status"] = json!("ok");
                if let Value::Object(fields) = mass_json(&mass) {
                    row.as_object_mut()
                        .expect("a row is an object")
                        .extend(fields);
                }
                counted.push(mass);
            }
            Err((status, reason)) => {
                row["status"] = json!(status);
                row["reason"] = json!(reason);
                missing.push(
                    json!({"occurrence_id": row["occurrence_id"], "name": row["name"],
                    "status": status, "reason": reason}),
                );
            }
        }
        parts.push(row);
    }
    let status = match (counted.len(), missing.len()) {
        (_, 0) if !counted.is_empty() => "complete",
        (0, _) => "not_verified",
        _ => "partial",
    };
    let mut assembly = json!({"status": status, "parts_counted": counted.len(),
        "parts_missing": missing.len(), "missing": missing});
    if let Some(sum) = Mass::sum(&counted)
        && let Value::Object(fields) = mass_json(&sum)
    {
        assembly
            .as_object_mut()
            .expect("the assembly is an object")
            .extend(fields);
    }
    json!({"convention": CONVENTION, "assembly": assembly, "parts": parts,
        "hint": "Only status complete is the whole selection; partial sums the counted parts only and lists the missing ones with the reason."})
}
