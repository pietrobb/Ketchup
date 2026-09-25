//! Compact, always-current catalog of CAD program operations.
//!
//! The JSON schema is generated at build time from the serde types in
//! `assistant_sidecar.rs` (see `build.rs`). This module renders it in a terse
//! TypeScript-like notation so an AI can compose a program without reading
//! source code.

use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub const CAD_PROGRAM_SCHEMA_JSON: &str =
    include_str!(concat!(env!("OUT_DIR"), "/cad-program-schema.json"));

const USAGE: &str = "Program = {\"operations\":[{\"operation\":\"<name>\", ...fields}]}. \
    `field?` is optional. Type names are resolved by requesting one operation by name.";

#[must_use]
pub fn cad_program_schema() -> Value {
    serde_json::from_str(CAD_PROGRAM_SCHEMA_JSON).expect("build-generated CAD schema")
}

fn definitions(schema: &Value) -> &Map<String, Value> {
    schema["$defs"].as_object().expect("schema definitions")
}

fn operations(schema: &Value) -> &Vec<Value> {
    schema["$defs"]["AssistantCadEditOperation"]["oneOf"]
        .as_array()
        .expect("operation variants")
}

fn operation_name(variant: &Value) -> &str {
    variant["properties"]["operation"]["const"]
        .as_str()
        .expect("operation tag")
}

fn reference_name(value: &Value) -> Option<&str> {
    value["$ref"].as_str()?.strip_prefix("#/$defs/")
}

/// Terse notation: `{a: number, b?: [number; 3]}`, `"x" | "y"`, `Name`.
fn compact(value: &Value, skip_tag: Option<&str>) -> String {
    if let Some(name) = reference_name(value) {
        return name.to_owned();
    }
    if let Some(constant) = value.get("const") {
        return constant.to_string();
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(options) = value[key].as_array() {
            return options
                .iter()
                .map(|option| compact(option, None))
                .collect::<Vec<_>>()
                .join(" | ");
        }
    }
    match value["type"].as_str() {
        Some("object") => {
            let required: BTreeSet<&str> = value["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let fields = value["properties"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(name, _)| Some(name.as_str()) != skip_tag)
                .map(|(name, field)| {
                    let optional = if required.contains(name.as_str()) {
                        ""
                    } else {
                        "?"
                    };
                    format!("{name}{optional}: {}", compact(field, None))
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", fields.join(", "))
        }
        Some("array") => {
            let item = compact(&value["items"], None);
            match (value["minItems"].as_u64(), value["maxItems"].as_u64()) {
                (Some(min), Some(max)) if min == max => format!("[{item}; {min}]"),
                _ => format!("[{item}]"),
            }
        }
        Some(other) => other.to_owned(),
        None => "any".to_owned(),
    }
}

fn collect_references<'a>(value: &'a Value, into: &mut BTreeSet<&'a str>) {
    if let Some(name) = reference_name(value) {
        into.insert(name);
    }
    match value {
        Value::Object(map) => map.values().for_each(|v| collect_references(v, into)),
        Value::Array(items) => items.iter().for_each(|v| collect_references(v, into)),
        _ => {}
    }
}

fn field_notes(variant: &Value) -> Map<String, Value> {
    variant["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, field)| Some((name.clone(), field.get("description")?.clone())))
        .collect()
}

/// Without `name`: every operation with its field shape. With `name`: that
/// operation plus the transitive closure of every type it references.
pub fn cad_operation_catalog(name: Option<&str>) -> Result<Value, &'static str> {
    let schema = cad_program_schema();
    let Some(name) = name else {
        let list: Vec<Value> = operations(&schema)
            .iter()
            .map(|variant| {
                let mut entry = json!({
                    "operation": operation_name(variant),
                    "fields": compact(variant, Some("operation")),
                });
                if let Some(description) = variant.get("description") {
                    entry["description"] = description.clone();
                }
                entry
            })
            .collect();
        return Ok(json!({"usage": USAGE, "count": list.len(), "operations": list}));
    };
    let variant = operations(&schema)
        .iter()
        .find(|variant| operation_name(variant) == name)
        .ok_or("unknown_operation")?;
    let defs = definitions(&schema);
    let mut seen = BTreeSet::new();
    let mut pending = BTreeSet::new();
    collect_references(variant, &mut pending);
    while let Some(next) = pending.pop_first() {
        if seen.insert(next) {
            collect_references(&defs[next], &mut pending);
        }
    }
    let types: Map<String, Value> = seen
        .into_iter()
        .map(|type_name| (type_name.to_owned(), json!(compact(&defs[type_name], None))))
        .collect();
    let mut result = json!({
        "usage": USAGE,
        "operation": name,
        "fields": compact(variant, Some("operation")),
        "types": types,
    });
    if let Some(description) = variant.get("description") {
        result["description"] = description.clone();
    }
    let notes = field_notes(variant);
    if !notes.is_empty() {
        result["field_notes"] = Value::Object(notes);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_every_operation_and_resolves_one_with_its_types() {
        let list = cad_operation_catalog(None).unwrap();
        let names: Vec<&str> = list["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["operation"].as_str().unwrap())
            .collect();
        for expected in [
            "create_panel",
            "create_physical_dowel_joint",
            "delete",
            "transform",
        ] {
            assert!(names.contains(&expected), "{expected} missing");
        }
        let panel = cad_operation_catalog(Some("create_panel")).unwrap();
        assert!(
            panel["fields"]
                .as_str()
                .unwrap()
                .contains("dimensions_mm: [number; 3]")
        );
        assert!(panel["types"]["AssistantPanelHole"].is_string());
        assert!(panel["types"]["AssistantCadRotation"].is_string());
        let dowel = cad_operation_catalog(Some("create_physical_dowel_joint")).unwrap();
        assert!(dowel["field_notes"]["first_insertion_mm"].is_string());
        assert_eq!(
            cad_operation_catalog(Some("nope")),
            Err("unknown_operation")
        );
        // Live bridge frames are 32 KiB including the envelope.
        let size = |value: &Value| serde_json::to_vec(value).unwrap().len();
        let largest = names
            .iter()
            .map(|name| size(&cad_operation_catalog(Some(name)).unwrap()))
            .max()
            .unwrap();
        println!("catalog list {} bytes, largest operation {largest} bytes", size(&list));
        assert!(size(&list) < 28_000 && largest < 28_000);
    }
}
