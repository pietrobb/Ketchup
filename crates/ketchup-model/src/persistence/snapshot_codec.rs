//! Native snapshot encoding.
//!
//! The product model has one serde definition; this module stores it as CBOR behind the
//! `KETCHUPDOC` magic, a format number and a SHA-256 checksum of the payload. New fields
//! declare `#[serde(default)]` and need no new format number. A new number is needed only
//! when an existing field changes name or meaning; its conversion then goes into [`decode`].

use super::sheet_metal_v1::{SheetMetalV1, bend_path, flange_path};
use super::{MAGIC, PersistenceError};
use crate::document::{FeatureId, ProductModel, Snapshot};
use std::collections::BTreeMap;

/// Format number written after the magic.
pub(super) const SNAPSHOT_FORMAT: u16 = 101;

/// First format this codec wrote. Numbers below it belong to the retired hand-written
/// codec.
pub(super) const FIRST_SNAPSHOT_FORMAT: u16 = 98;

const CHECKSUM_BYTES: usize = 32;

#[derive(serde::Serialize)]
struct EncodedSnapshot<'a> {
    revision_id: u64,
    product: &'a ProductModel,
}

#[derive(serde::Deserialize)]
struct DecodedSnapshot {
    revision_id: u64,
    product: ProductModel,
}

pub(super) fn encode(snapshot: &Snapshot) -> Vec<u8> {
    let mut payload = Vec::new();
    ciborium::into_writer(
        &ciborium::Value::serialized(&EncodedSnapshot {
            revision_id: snapshot.revision_id(),
            product: snapshot.product(),
        })
        .expect("the product model serializes into memory"),
        &mut payload,
    )
    .expect("the product model serializes into memory");
    let mut bytes = Vec::with_capacity(MAGIC.len() + 2 + CHECKSUM_BYTES + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&SNAPSHOT_FORMAT.to_le_bytes());
    bytes.extend_from_slice(&crate::graph::sha256_bytes(&payload));
    bytes.extend_from_slice(&payload);
    bytes
}

/// Rewrites the stored snapshot as a CBOR value tree and reseals its checksum.
#[cfg(feature = "testing")]
pub(crate) fn rewrite(bytes: &[u8], edit: impl FnOnce(&mut ciborium::Value)) -> Vec<u8> {
    let header = MAGIC.len() + 2;
    let mut value: ciborium::Value =
        ciborium::from_reader(&bytes[header + CHECKSUM_BYTES..]).expect("a saved snapshot");
    edit(&mut value);
    let mut payload = Vec::new();
    ciborium::into_writer(&value, &mut payload).expect("a CBOR value serializes into memory");
    let mut rewritten = bytes[..header].to_vec();
    rewritten.extend_from_slice(&crate::graph::sha256_bytes(&payload));
    rewritten.extend_from_slice(&payload);
    rewritten
}

/// A decoded snapshot, and for an older format the digest its writer computed.
pub(super) struct Decoded {
    pub(super) revision_id: u64,
    pub(super) product: ProductModel,
    pub(super) writer_digest: Option<String>,
}

/// Decodes the bytes that follow the magic and a format number of this codec.
pub(super) fn decode(format: u16, body: &[u8]) -> Result<Decoded, PersistenceError> {
    let (checksum, payload) = body
        .split_at_checked(CHECKSUM_BYTES)
        .ok_or(PersistenceError::Truncated)?;
    if crate::graph::sha256_bytes(payload) != checksum {
        return Err(PersistenceError::ChecksumMismatch);
    }
    let invalid =
        |error: &dyn std::fmt::Display| PersistenceError::InvalidPayload(error.to_string());
    if format == SNAPSHOT_FORMAT {
        let decoded: DecodedSnapshot =
            ciborium::from_reader(payload).map_err(|error| invalid(&error))?;
        return Ok(Decoded {
            revision_id: decoded.revision_id,
            product: decoded.product,
            writer_digest: None,
        });
    }
    let mut value: ciborium::Value =
        ciborium::from_reader(payload).map_err(|error| invalid(&error))?;
    let format_98 = format < 99;
    if format_98 && let Some(product) = field(&mut value, "product") {
        rename_format_98_fields(product, |(old, new)| (old, new));
    }
    let sheet_metal = match field(&mut value, "product") {
        Some(product) => read_format_100_sheet_metal(product)?,
        None => BTreeMap::new(),
    };
    let decoded: DecodedSnapshot = value.deserialized().map_err(|error| invalid(&error))?;
    // The writer hashed the identity form under its own field names, and before
    // [`rename_format_99_role_categories`], which the caller applies to every older format.
    let mut writer_form =
        crate::document::identity_form(|| ciborium::Value::serialized(&decoded.product))
            .map_err(|error| invalid(&error))?;
    restore_format_100_sheet_metal(&mut writer_form, sheet_metal);
    if format_98 {
        rename_format_98_fields(&mut writer_form, |(old, new)| (new, old));
    }
    Ok(Decoded {
        revision_id: decoded.revision_id,
        writer_digest: Some(crate::document::digest_product(&writer_form)),
        product: decoded.product,
    })
}

/// Format 99 names the generic pin joint: the product's `dowel_joints` became
/// `pin_joints`, a joint's `dowel` its `pin`, and a recipe joinery item's
/// `dowel_joint_id` its `pin_joint_id`. Values are unchanged. `direction` maps an
/// (old, new) pair to the (from, to) names of this pass.
fn rename_format_98_fields(
    product: &mut ciborium::Value,
    direction: impl Fn((&'static str, &'static str)) -> (&'static str, &'static str),
) {
    let (joints, joint_spec, recipe_joint) = (
        direction(("dowel_joints", "pin_joints")),
        direction(("dowel", "pin")),
        direction(("dowel_joint_id", "pin_joint_id")),
    );
    if let Some(ciborium::Value::Map(entries)) = rename_field(product, joints.0, joints.1) {
        for (_, joint) in entries {
            rename_field(joint, joint_spec.0, joint_spec.1);
        }
    }
    if let Some(ciborium::Value::Map(joinery)) =
        field(product, "assembly_recipe").and_then(|recipe| field(recipe, "joinery"))
    {
        for (_, item) in joinery {
            rename_field(item, recipe_joint.0, recipe_joint.1);
        }
    }
}

/// Format 100 names validator roles by what a part does, not by the product it belongs
/// to. A category of the validator-role classification whose name starts with an old
/// function name gets the new one; frame and group suffixes are kept. A document that
/// already holds the new name keeps the old category unchanged.
pub(super) fn rename_format_99_role_categories(product: &mut ProductModel) {
    const RENAMED: [(&str, &str); 4] = [
        ("furniture.shelf", "physics.beam"),
        ("furniture.case", "physics.freestanding"),
        ("manufacturing.hinge-cup", "manufacturing.cup-bore"),
        ("spatial.furniture", "spatial.occupant"),
    ];
    for dimension in product.classification_dimensions.values_mut() {
        if dimension.name != crate::validation::VALIDATOR_ROLE_DIMENSION_V1 {
            continue;
        }
        let renamed = dimension
            .categories
            .values()
            .filter_map(|category| {
                RENAMED.iter().find_map(|(old, new)| {
                    let rest = category.name.strip_prefix(old)?;
                    (rest.is_empty() || rest.starts_with(['.', ':']))
                        .then(|| (category.id, format!("{new}{rest}")))
                })
            })
            .filter(|(_, name)| {
                !dimension
                    .categories
                    .values()
                    .any(|category| category.name == *name)
            })
            .collect::<Vec<_>>();
        if renamed.is_empty() {
            continue;
        }
        let dimension = std::sync::Arc::make_mut(dimension);
        for (id, name) in renamed {
            if let Some(category) = dimension.categories.get_mut(&id) {
                category.name = name;
            }
        }
    }
}

/// A sheet-metal feature as format 100 stored it, and for each of its flanges the bend
/// it became.
struct StoredSheetMetal {
    stored: ciborium::Value,
    bend_of_flange: Vec<usize>,
}

/// Format 101 stores sheet metal as a base polygon with a tree of bends (see
/// [`super::sheet_metal_v1`]). Rewrites each older record and the parameter paths that
/// name its flanges, and returns what was stored.
fn read_format_100_sheet_metal(
    product: &mut ciborium::Value,
) -> Result<BTreeMap<FeatureId, StoredSheetMetal>, PersistenceError> {
    let invalid =
        |error: &dyn std::fmt::Display| PersistenceError::InvalidPayload(error.to_string());
    let mut read = BTreeMap::new();
    for (id, kind) in sheet_metal_kinds(product) {
        let record: SheetMetalV1 = kind.deserialized().map_err(|error| invalid(&error))?;
        let (spec, bend_of_flange) = record
            .to_current()
            .ok_or_else(|| invalid(&"a sheet-metal flange names an unknown side"))?;
        let stored = std::mem::replace(
            kind,
            ciborium::Value::serialized(&spec).map_err(|error| invalid(&error))?,
        );
        read.insert(
            id,
            StoredSheetMetal {
                stored,
                bend_of_flange,
            },
        );
    }
    if !read.is_empty() {
        rename_parameter_paths(product, &|feature, path| {
            bend_path(path, &read.get(&feature)?.bend_of_flange)
        });
    }
    Ok(read)
}

/// Puts back what [`read_format_100_sheet_metal`] rewrote.
fn restore_format_100_sheet_metal(
    product: &mut ciborium::Value,
    mut read: BTreeMap<FeatureId, StoredSheetMetal>,
) {
    if read.is_empty() {
        return;
    }
    rename_parameter_paths(product, &|feature, path| {
        flange_path(path, &read.get(&feature)?.bend_of_flange)
    });
    for (id, kind) in sheet_metal_kinds(product) {
        if let Some(stored) = read.remove(&id) {
            *kind = stored.stored;
        }
    }
}

/// The `SheetMetal` payload of every feature of `product`, by feature.
fn sheet_metal_kinds(product: &mut ciborium::Value) -> Vec<(FeatureId, &mut ciborium::Value)> {
    let Some(ciborium::Value::Map(features)) = field(product, "features") else {
        return Vec::new();
    };
    features
        .iter_mut()
        .filter_map(|(id, feature)| {
            let id = FeatureId(u64::try_from(id.as_integer()?).ok()?);
            let ciborium::Value::Map(kind) = field(feature, "kind")? else {
                return None;
            };
            match kind.as_mut_slice() {
                [(name, payload)] if name.as_text() == Some("SheetMetal") => Some((id, payload)),
                _ => None,
            }
        })
        .collect()
}

/// Renames the `path` of every parameter target (a map with `feature_id`, `path` and
/// `value_type`) for which `rename` gives a new one.
pub(super) fn rename_parameter_paths(
    value: &mut ciborium::Value,
    rename: &dyn Fn(FeatureId, &str) -> Option<String>,
) {
    match value {
        ciborium::Value::Map(entries) => {
            let position = |entries: &[(ciborium::Value, ciborium::Value)], name: &str| {
                entries
                    .iter()
                    .position(|(key, _)| key.as_text() == Some(name))
            };
            if let (Some(feature), Some(path), Some(_)) = (
                position(entries, "feature_id"),
                position(entries, "path"),
                position(entries, "value_type"),
            ) && let Some(renamed) = entries[feature]
                .1
                .as_integer()
                .and_then(|id| u64::try_from(id).ok())
                .zip(entries[path].1.as_text())
                .and_then(|(id, path)| rename(FeatureId(id), path))
            {
                entries[path].1 = ciborium::Value::Text(renamed);
            }
            for (key, item) in entries {
                rename_parameter_paths(key, rename);
                rename_parameter_paths(item, rename);
            }
        }
        ciborium::Value::Array(items) => {
            for item in items {
                rename_parameter_paths(item, rename);
            }
        }
        ciborium::Value::Tag(_, item) => rename_parameter_paths(item, rename),
        _ => {}
    }
}

fn field<'a>(value: &'a mut ciborium::Value, name: &str) -> Option<&'a mut ciborium::Value> {
    let ciborium::Value::Map(entries) = value else {
        return None;
    };
    entries
        .iter_mut()
        .find(|(key, _)| key.as_text() == Some(name))
        .map(|(_, value)| value)
}

fn rename_field<'a>(
    value: &'a mut ciborium::Value,
    from: &str,
    to: &str,
) -> Option<&'a mut ciborium::Value> {
    let ciborium::Value::Map(entries) = value else {
        return None;
    };
    let (key, value) = entries
        .iter_mut()
        .find(|(key, _)| key.as_text() == Some(from))?;
    *key = ciborium::Value::Text(to.to_owned());
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::super::sheet_metal_v1::FlangeV1;
    use super::*;
    use crate::document::{
        CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureKind,
    };

    fn dimension(value: f64) -> Dimension {
        Dimension::new(format!("{value} mm"), value).unwrap()
    }

    fn flange(edge: &str, length: f64, angle_degrees: f64) -> FlangeV1 {
        FlangeV1 {
            edge: edge.to_owned(),
            length: dimension(length),
            angle_degrees,
            inner_radius: dimension(3.0),
        }
    }

    #[test]
    fn format_100_sheet_metal_reads_as_a_rectangle_with_bends_under_its_writer_digest() {
        let old = SheetMetalV1 {
            width: dimension(100.0),
            depth: dimension(50.0),
            thickness: dimension(2.0),
            k_factor: 0.4,
            flanges: vec![flange("MinX", 20.0, 90.0), flange("MaxX", 30.0, -45.0)],
        };
        let (current, bend_of_flange) = old.to_current().unwrap();
        assert_eq!(
            current.base_mm,
            [[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]]
        );
        assert_eq!(
            current
                .bends
                .iter()
                .map(|bend| bend.edge)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(bend_of_flange, [1, 0]);
        assert_eq!(
            bend_path("flanges.0.length", &bend_of_flange).as_deref(),
            Some("bends.1.length")
        );
        assert_eq!(
            flange_path("bends.1.length", &bend_of_flange).as_deref(),
            Some("flanges.0.length")
        );

        let mut store = DocumentStore::new();
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Bracket".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(2),
                    definition_id: DefinitionId(1),
                    name: "Opposite flanges".into(),
                    kind: FeatureKind::SheetMetal(current.clone()),
                },
            ]))
            .unwrap();
        let snapshot = store.current();
        let as_format_100 = |mut product: ciborium::Value| {
            for (_, kind) in sheet_metal_kinds(&mut product) {
                *kind = ciborium::Value::serialized(&old).unwrap();
            }
            product
        };
        let product = as_format_100(ciborium::Value::serialized(snapshot.product()).unwrap());
        let writer_digest = crate::document::digest_product(&as_format_100(
            crate::document::identity_form(|| ciborium::Value::serialized(snapshot.product()))
                .unwrap(),
        ));
        let mut payload = Vec::new();
        ciborium::into_writer(
            &ciborium::Value::Map(vec![
                (
                    ciborium::Value::Text("revision_id".into()),
                    ciborium::Value::Integer(snapshot.revision_id().into()),
                ),
                (ciborium::Value::Text("product".into()), product),
            ]),
            &mut payload,
        )
        .unwrap();
        let mut body = crate::graph::sha256_bytes(&payload).to_vec();
        body.extend_from_slice(&payload);

        let decoded = decode(100, &body).unwrap();
        assert_eq!(decoded.writer_digest, Some(writer_digest));
        assert_eq!(
            decoded.product.features[&FeatureId(2)].kind,
            FeatureKind::SheetMetal(current)
        );
    }
}
