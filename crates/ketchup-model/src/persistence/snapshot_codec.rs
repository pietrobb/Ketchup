//! Native snapshot encoding.
//!
//! The product model has one serde definition; this module stores it as CBOR behind the
//! `KETCHUPDOC` magic, a format number and a SHA-256 checksum of the payload.
//!
//! Schema policy: the saved serde form of [`ProductModel`] is the format. A history
//! revision must encode back to the same CBOR, so any change to that form needs a new
//! [`SNAPSHOT_FORMAT`], its conversion in [`decode`] and a golden file of the previous
//! format: a renamed, moved or reinterpreted field, a changed default, and any new
//! field, also one with `#[serde(default)]` and a `skip_serializing_if` for that
//! default. Format 103 shipped in Kečup 0.1.0, which ignores fields it does not know,
//! so a new field under 103 would be dropped by 0.1.0 and lost at its next Save (PS-4).
//! Format 104 adds underlays, 105 annotations; an older file has none, so it reads
//! without a conversion, and a document without them is still written as 103 when
//! Kečup 0.2.0 reads all of it ([`fits_format_103`]). The
//! optional fields of the current format are listed in `tests/format_golden.rs`, which
//! also fails on any other change. The reader refuses a
//! current-format snapshot with fields it does not know (`PersistenceError::UnknownFields`). The encoder writes the serde
//! form directly, so every map in it must state its length: `#[serde(flatten)]` would
//! write an indefinite-length map.

use super::sheet_metal_v1::{SheetMetalV1, bend_path, flange_path};
use super::{MAGIC, PayloadError, PayloadSection, PersistenceError};
use crate::document::{FeatureId, ProductModel, Snapshot};
use std::collections::BTreeMap;

/// The newest format number written after the magic.
pub(super) const SNAPSHOT_FORMAT: u16 = 105;

/// The oldest format this version writes. A snapshot is written in the oldest format
/// that holds it ([`format_of`]), so Kečup 0.1.0 and 0.2.0, which read 103, open a
/// document without underlays and annotations; any format from this one on reads as
/// written by this version.
pub(super) const OLDEST_WRITTEN_FORMAT: u16 = 103;

/// 105 with annotations, else 104 with underlays or with what Kečup 0.1.0 and 0.2.0
/// refuse to read, else 103.
fn format_of(product: &ProductModel) -> u16 {
    if !product.annotations.is_empty() {
        SNAPSHOT_FORMAT
    } else if !product.underlays.is_empty() || !fits_format_103(product) {
        104
    } else {
        OLDEST_WRITTEN_FORMAT
    }
}

/// Whether Kečup 0.1.0 and 0.2.0, which read format 103, accept every import receipt
/// of `product`: at most 1 024 outputs and a source within their reader's limit. A
/// file they refuse as damaged makes them open an older `.recovery` instead and
/// overwrite the newer file at its next Save; format 104 they refuse as newer
/// (review 2026-10-24 MD-20).
fn fits_format_103(product: &ProductModel) -> bool {
    use crate::import::ImportFormat;
    const OUTPUTS: usize = 1_024;
    const MIB: u64 = 1024 * 1024;
    // The bounded ASCII STL of 0.2.0: 8 lines of at most 128 bytes and CR LF a triangle.
    const STL: u64 = (ketchup_tolerance::limits::MESH_TRIANGLES as u64 * 8 + 2) * 130;
    product.import_receipts.values().all(|receipt| {
        let source_limit = match receipt.format() {
            ImportFormat::Stl => STL,
            ImportFormat::Dxf => 8 * MIB,
            ImportFormat::Step
            | ImportFormat::Iges
            | ImportFormat::SketchupScene
            | ImportFormat::Glb => 32 * MIB,
        };
        receipt.outputs().len() <= OUTPUTS && receipt.source_byte_len() <= source_limit
    })
}

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
        &EncodedSnapshot {
            revision_id: snapshot.revision_id(),
            product: snapshot.product(),
        },
        &mut payload,
    )
    .expect("the product model serializes into memory");
    let mut bytes = Vec::with_capacity(MAGIC.len() + 2 + CHECKSUM_BYTES + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&format_of(snapshot.product()).to_le_bytes());
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
    if format >= OLDEST_WRITTEN_FORMAT {
        let decoded: DecodedSnapshot = ciborium::from_reader(payload).map_err(invalid_snapshot)?;
        refuse_unknown_fields(payload, &decoded)?;
        return Ok(Decoded {
            revision_id: decoded.revision_id,
            product: decoded.product,
            writer_digest: None,
        });
    }
    let mut value: ciborium::Value = ciborium::from_reader(payload).map_err(invalid_snapshot)?;
    let format_98 = format < 99;
    if format_98 && let Some(product) = field(&mut value, "product") {
        rename_format_98_fields(product, |(old, new)| (old, new));
    }
    // Each conversion applies only to the formats written before it was introduced.
    let sheet_metal = match field(&mut value, "product") {
        Some(product) if format < 101 => read_format_100_sheet_metal(product)?,
        _ => BTreeMap::new(),
    };
    if let Some(product) = field(&mut value, "product") {
        if format < 102 {
            format_101_occurrence_tags(product, false);
        }
        if format < 103 {
            format_102_joint_participants(product, false);
        }
    }
    let decoded: DecodedSnapshot = value.deserialized().map_err(invalid_snapshot)?;
    // The writer hashed the identity form under its own field names, and before
    // [`rename_format_99_role_categories`], which the caller applies to formats before 100.
    let mut writer_form =
        crate::document::identity_form(|| ciborium::Value::serialized(&decoded.product))
            .map_err(invalid_snapshot)?;
    restore_format_100_sheet_metal(&mut writer_form, sheet_metal);
    if format < 103 {
        format_102_joint_participants(&mut writer_form, true);
    }
    if format < 102 {
        format_101_occurrence_tags(&mut writer_form, true);
    }
    if format_98 {
        rename_format_98_fields(&mut writer_form, |(old, new)| (new, old));
    }
    Ok(Decoded {
        revision_id: decoded.revision_id,
        writer_digest: Some(crate::document::digest_product(&writer_form)),
        product: decoded.product,
    })
}

fn invalid_snapshot(error: impl Into<PayloadError>) -> PersistenceError {
    PersistenceError::payload(PayloadSection::Snapshot, error)
}

/// Refuses a snapshot of the current format that holds fields this version does not
/// know. Serde skips them, so the document would open without them and the next Save
/// would drop them: a newer version wrote them, or the format policy was broken. A
/// document this version wrote encodes back to the same bytes, so only a difference is
/// searched for the unknown fields.
fn refuse_unknown_fields(
    payload: &[u8],
    decoded: &DecodedSnapshot,
) -> Result<(), PersistenceError> {
    let mut known = Vec::with_capacity(payload.len());
    ciborium::into_writer(
        &EncodedSnapshot {
            revision_id: decoded.revision_id,
            product: &decoded.product,
        },
        &mut known,
    )
    .expect("the product model serializes into memory");
    if known == payload {
        return Ok(());
    }
    let stored: ciborium::Value = ciborium::from_reader(payload).map_err(invalid_snapshot)?;
    let known: ciborium::Value =
        ciborium::from_reader(known.as_slice()).map_err(invalid_snapshot)?;
    let mut unknown = Vec::new();
    unknown_fields(&stored, &known, &mut String::new(), &mut unknown);
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(PersistenceError::UnknownFields(unknown))
    }
}

/// Appends the path of every map entry of `stored` that `known` lacks.
fn unknown_fields(
    stored: &ciborium::Value,
    known: &ciborium::Value,
    path: &mut String,
    unknown: &mut Vec<String>,
) {
    let mut descend =
        |segment: String, stored: &ciborium::Value, known: Option<&ciborium::Value>| {
            let length = path.len();
            if !path.is_empty() {
                path.push('.');
            }
            path.push_str(&segment);
            match known {
                Some(known) => unknown_fields(stored, known, path, unknown),
                None => unknown.push(path.clone()),
            }
            path.truncate(length);
        };
    match (stored, known) {
        (ciborium::Value::Map(stored), ciborium::Value::Map(known)) => {
            // Keys are looked up by hash only; a map of a few hundred thousand entries
            // scanned per key would never finish (review PS-4), and neither would one
            // of as many unknown keys scanned per miss (review 2026-10-16 PS-4).
            let mut index = std::collections::HashMap::with_capacity(known.len());
            for (key, value) in known {
                index.entry(MapKey::of(key)).or_insert(value);
            }
            for (key, value) in stored {
                let segment = match key {
                    ciborium::Value::Text(name) => name.clone(),
                    ciborium::Value::Integer(number) => i128::from(*number).to_string(),
                    other => format!("{other:?}"),
                };
                descend(segment, value, index.get(&MapKey::of(key)).copied());
            }
        }
        (ciborium::Value::Array(stored), ciborium::Value::Array(known))
            if stored.len() == known.len() =>
        {
            for (index, (stored, known)) in stored.iter().zip(known).enumerate() {
                descend(index.to_string(), stored, Some(known));
            }
        }
        (ciborium::Value::Tag(_, stored), ciborium::Value::Tag(_, known)) => {
            unknown_fields(stored, known, path, unknown);
        }
        _ => {}
    }
}

/// A CBOR map key that hashes: text and integer keys as they are, and the
/// structured keys of the product model (an instance path, a local occurrence
/// key) as their CBOR encoding. Values decoded from CBOR encode back one way,
/// save -0.0, which equals 0.0 and is encoded as it, so keys equal as values
/// are equal here.
#[derive(Hash, PartialEq, Eq)]
enum MapKey<'a> {
    Text(&'a str),
    Integer(i128),
    Encoded(Vec<u8>),
}

impl<'a> MapKey<'a> {
    fn of(key: &'a ciborium::Value) -> Self {
        match key {
            ciborium::Value::Text(text) => Self::Text(text),
            ciborium::Value::Integer(number) => Self::Integer(i128::from(*number)),
            other => {
                let mut bytes = Vec::new();
                ciborium::into_writer(&without_negative_zero(other), &mut bytes)
                    .expect("a decoded CBOR value encodes into memory");
                Self::Encoded(bytes)
            }
        }
    }
}

/// `value` with every -0.0 written as 0.0.
fn without_negative_zero(value: &ciborium::Value) -> ciborium::Value {
    match value {
        ciborium::Value::Float(number) if *number == 0.0 => ciborium::Value::Float(0.0),
        ciborium::Value::Array(items) => {
            ciborium::Value::Array(items.iter().map(without_negative_zero).collect())
        }
        ciborium::Value::Map(entries) => ciborium::Value::Map(
            entries
                .iter()
                .map(|(key, value)| (without_negative_zero(key), without_negative_zero(value)))
                .collect(),
        ),
        ciborium::Value::Tag(tag, inner) => {
            ciborium::Value::Tag(*tag, Box::new(without_negative_zero(inner)))
        }
        other => other.clone(),
    }
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

/// Format 102 lets a part belong to several tags: an occurrence's optional `tag` became
/// the set `tags`. Converts every root and local occurrence of `product` to the new form,
/// or with `back` from it to the old one (which holds at most one tag).
fn format_101_occurrence_tags(product: &mut ciborium::Value, back: bool) {
    for occurrences in ["occurrences", "local_occurrences"] {
        let Some(ciborium::Value::Map(entries)) = field(product, occurrences) else {
            continue;
        };
        for (_, occurrence) in entries {
            let (from, to) = if back {
                ("tags", "tag")
            } else {
                ("tag", "tags")
            };
            let Some(value) = rename_field(occurrence, from, to) else {
                continue;
            };
            *value = match std::mem::replace(value, ciborium::Value::Null) {
                ciborium::Value::Null if !back => ciborium::Value::Array(Vec::new()),
                ciborium::Value::Array(mut tags) if back && tags.len() <= 1 => {
                    tags.pop().unwrap_or(ciborium::Value::Null)
                }
                id if !back => ciborium::Value::Array(vec![id]),
                other => other,
            };
        }
    }
}

/// Format 103 lets a joint join placed parts as well as values a rule derives: each
/// participant became a `JointParticipant`, so an older one (always derived) is
/// `{"Derived": old}`. Converts every joint of `product` to the new form, or with
/// `back` from it to the old one.
fn format_102_joint_participants(product: &mut ciborium::Value, back: bool) {
    const DERIVED: &str = "Derived";
    let Some(ciborium::Value::Map(joints)) = field(product, "joints") else {
        return;
    };
    for (_, joint) in joints {
        for name in ["participant_a", "participant_b"] {
            let Some(participant) = field(joint, name) else {
                continue;
            };
            let old = std::mem::replace(participant, ciborium::Value::Null);
            *participant = match old {
                ciborium::Value::Map(mut variant)
                    if back && variant.len() == 1 && variant[0].0.as_text() == Some(DERIVED) =>
                {
                    variant.pop().expect("one entry").1
                }
                old if !back => {
                    ciborium::Value::Map(vec![(ciborium::Value::Text(DERIVED.to_owned()), old)])
                }
                other => other,
            };
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
    let mut read = BTreeMap::new();
    for (id, kind) in sheet_metal_kinds(product) {
        let invalid = |error| PersistenceError::payload(PayloadSection::SheetMetal(id), error);
        let record: SheetMetalV1 = kind.deserialized().map_err(invalid)?;
        let (spec, bend_of_flange) = record.to_current().ok_or_else(|| {
            PersistenceError::payload(
                PayloadSection::SheetMetal(id),
                PayloadError::UnknownFlangeSide,
            )
        })?;
        let stored = std::mem::replace(kind, ciborium::Value::serialized(&spec).map_err(invalid)?);
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
        let (body, writer_digest) = written_as(&snapshot, as_format_100);
        let decoded = decode(100, &body).unwrap();
        assert_eq!(decoded.writer_digest, Some(writer_digest));
        assert_eq!(
            decoded.product.features[&FeatureId(2)].kind,
            FeatureKind::SheetMetal(current)
        );
    }

    #[test]
    fn format_101_single_tag_reads_as_a_tag_set_under_its_writer_digest() {
        use crate::document::{OccurrenceId, TagId, Transform};
        let mut store = DocumentStore::new();
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Part".into(),
                },
                CanonicalCommand::CreateTag {
                    id: TagId(3),
                    name: "Walls".into(),
                    visible: false,
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(1),
                    definition_id: DefinitionId(1),
                    name: "Tagged".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tags: [TagId(3)].into(),
                    visible: true,
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(1),
                    name: "Untagged".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tags: Default::default(),
                    visible: true,
                },
            ]))
            .unwrap();
        let snapshot = store.current();
        let as_format_101 = |mut product: ciborium::Value| {
            format_101_occurrence_tags(&mut product, true);
            product
        };
        let product = as_format_101(ciborium::Value::serialized(snapshot.product()).unwrap());
        let ciborium::Value::Map(occurrences) = product
            .as_map()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|(key, _)| key.as_text() == Some("occurrences"))
            })
            .map(|(_, value)| value.clone())
            .unwrap()
        else {
            panic!("occurrences map")
        };
        let stored_tags = occurrences
            .iter()
            .map(|(_, occurrence)| {
                occurrence
                    .as_map()
                    .unwrap()
                    .iter()
                    .find(|(key, _)| key.as_text() == Some("tag"))
                    .map(|(_, value)| value.clone())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            stored_tags,
            [ciborium::Value::Integer(3.into()), ciborium::Value::Null]
        );
        let (body, writer_digest) = written_as(&snapshot, as_format_101);
        let decoded = decode(101, &body).unwrap();
        assert_eq!(decoded.writer_digest, Some(writer_digest));
        assert!(
            decoded.product.occurrences[&OccurrenceId(1)]
                .tags
                .iter()
                .eq([&TagId(3)])
        );
        assert!(
            decoded.product.occurrences[&OccurrenceId(2)]
                .tags
                .is_empty()
        );
    }

    /// A bracket in the sheet-metal form of format 101 and later, on a part with `tags`.
    fn bracket_on_tagged_part(tags: &[u64]) -> Snapshot {
        use crate::document::{OccurrenceId, TagId, Transform};
        let (bracket, _) = SheetMetalV1 {
            width: dimension(100.0),
            depth: dimension(50.0),
            thickness: dimension(2.0),
            k_factor: 0.4,
            flanges: vec![flange("MinX", 20.0, 90.0)],
        }
        .to_current()
        .unwrap();
        let mut commands = vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Bracket".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Flange".into(),
                kind: FeatureKind::SheetMetal(bracket),
            },
        ];
        commands.extend(tags.iter().map(|&id| CanonicalCommand::CreateTag {
            id: TagId(id),
            name: format!("Layer {id}"),
            visible: true,
        }));
        commands.push(CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(1),
            definition_id: DefinitionId(1),
            name: "Bracket".into(),
            transform: Transform::identity(),
            parent: None,
            tags: tags.iter().map(|&id| TagId(id)).collect(),
            visible: true,
        });
        let mut store = DocumentStore::new();
        store.apply_batch(&CommandBatch::new(commands)).unwrap();
        store.current()
    }

    /// The body `decode` reads for `snapshot` stored as an older format, where
    /// `as_format` turns the current serde form into that format's form, and the digest
    /// that format's writer computed.
    fn written_as(
        snapshot: &Snapshot,
        as_format: impl Fn(ciborium::Value) -> ciborium::Value,
    ) -> (Vec<u8>, String) {
        let product = as_format(ciborium::Value::serialized(snapshot.product()).unwrap());
        let writer_digest = crate::document::digest_product(&as_format(
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
        (body, writer_digest)
    }

    /// Review PS-4: each stored key looked its counterpart up by scanning the known map,
    /// so an edited file with a map of a few hundred thousand entries never opened.
    #[test]
    fn unknown_fields_of_a_large_map_are_found_in_linear_time() {
        const ENTRIES: u64 = 200_000;
        let entry = |key: u64| {
            (
                ciborium::Value::Integer(key.into()),
                ciborium::Value::Map(vec![(
                    ciborium::Value::Text("name".into()),
                    ciborium::Value::Text(format!("Part {key}")),
                )]),
            )
        };
        let known = ciborium::Value::Map((0..ENTRIES).map(entry).collect());
        // Stored in reverse, so no entry sits where the known map has it.
        let mut stored: Vec<_> = (0..ENTRIES).rev().map(entry).collect();
        stored.push((
            ciborium::Value::Text("finish".into()),
            ciborium::Value::Text("oiled".into()),
        ));
        if let (_, ciborium::Value::Map(fields)) = &mut stored[1] {
            fields.push((
                ciborium::Value::Text("grain".into()),
                ciborium::Value::Integer(90.into()),
            ));
        }
        let mut unknown = Vec::new();
        unknown_fields(
            &ciborium::Value::Map(stored),
            &known,
            &mut String::new(),
            &mut unknown,
        );
        assert_eq!(unknown, [format!("{}.grain", ENTRIES - 2), "finish".into()]);
    }

    /// Review 2026-10-15 status PS-4: maps keyed by a structure (an instance path, a
    /// local occurrence key) were still scanned per key.
    #[test]
    fn unknown_fields_of_a_large_map_with_structured_keys_are_found_in_linear_time() {
        const ENTRIES: u64 = 100_000;
        let entry = |key: u64| {
            (
                ciborium::Value::Array(vec![
                    ciborium::Value::Integer((key / 100).into()),
                    ciborium::Value::Integer((key % 100).into()),
                ]),
                ciborium::Value::Map(vec![(
                    ciborium::Value::Text("x".into()),
                    ciborium::Value::Float(1.0),
                )]),
            )
        };
        let known = ciborium::Value::Map((0..ENTRIES).map(entry).collect());
        let mut stored: Vec<_> = (0..ENTRIES).rev().map(entry).collect();
        if let (_, ciborium::Value::Map(fields)) = &mut stored[1] {
            fields.push((
                ciborium::Value::Text("grain".into()),
                ciborium::Value::Integer(90.into()),
            ));
        }
        let mut unknown = Vec::new();
        unknown_fields(
            &ciborium::Value::Map(stored),
            &known,
            &mut String::new(),
            &mut unknown,
        );
        assert_eq!(unknown.len(), 1, "{unknown:?}");
        assert!(unknown[0].ends_with(".grain"), "{unknown:?}");
    }

    /// Review 2026-10-16 PS-4: a key the hash missed was compared with every known
    /// key, so a map of many keys this version does not know was scanned per key.
    #[test]
    fn many_unknown_keys_of_a_large_map_are_found_in_linear_time() {
        const ENTRIES: u64 = 40_000;
        let text = |key: u64| ciborium::Value::Text(format!("part-{key}"));
        let path = |key: u64| {
            ciborium::Value::Array(vec![
                ciborium::Value::Integer(key.into()),
                ciborium::Value::Float(0.0),
            ])
        };
        let value = || ciborium::Value::Bool(true);
        let known = ciborium::Value::Map(
            (0..ENTRIES)
                .flat_map(|key| [(text(key), value()), (path(key), value())])
                .collect(),
        );
        let stored = ciborium::Value::Map(
            (ENTRIES..2 * ENTRIES)
                .flat_map(|key| [(text(key), value()), (path(key), value())])
                .collect(),
        );
        let mut unknown = Vec::new();
        unknown_fields(&stored, &known, &mut String::new(), &mut unknown);
        assert_eq!(unknown.len() as u64, 2 * ENTRIES);
    }

    /// The three large maps above in release, nightly: scanned per key, each
    /// took minutes.
    #[test]
    #[ignore = "perf: release timing budget, runs nightly"]
    fn perf_unknown_fields_of_large_maps_are_found_in_seconds() {
        for check in [
            unknown_fields_of_a_large_map_are_found_in_linear_time as fn(),
            unknown_fields_of_a_large_map_with_structured_keys_are_found_in_linear_time,
            many_unknown_keys_of_a_large_map_are_found_in_linear_time,
        ] {
            let started = std::time::Instant::now();
            check();
            let elapsed = started.elapsed();
            assert!(elapsed < std::time::Duration::from_secs(30), "{elapsed:?}");
        }
    }

    /// A key equal as a value but written with other bytes still finds its counterpart.
    #[test]
    fn a_structured_key_with_negative_zero_matches_its_known_key() {
        let map = |zero: f64| {
            ciborium::Value::Map(vec![(
                ciborium::Value::Array(vec![ciborium::Value::Float(zero)]),
                ciborium::Value::Text("part".into()),
            )])
        };
        let mut unknown = Vec::new();
        unknown_fields(&map(-0.0), &map(0.0), &mut String::new(), &mut unknown);
        assert!(unknown.is_empty(), "{unknown:?}");
    }

    #[test]
    fn format_101_and_102_sheet_metal_on_tagged_parts_opens_under_its_writer_digest() {
        let as_format_102 = |mut product: ciborium::Value| {
            format_102_joint_participants(&mut product, true);
            product
        };
        let as_format_101 = |product: ciborium::Value| {
            let mut product = as_format_102(product);
            format_101_occurrence_tags(&mut product, true);
            product
        };
        for (format, tags, as_format) in [
            (
                101,
                &[3][..],
                &as_format_101 as &dyn Fn(ciborium::Value) -> ciborium::Value,
            ),
            (102, &[3, 4][..], &as_format_102),
            (102, &[3][..], &as_format_102),
        ] {
            let snapshot = bracket_on_tagged_part(tags);
            let (body, writer_digest) = written_as(&snapshot, as_format);
            let decoded = decode(format, &body)
                .unwrap_or_else(|error| panic!("format {format} {tags:?}: {error}"));
            assert_eq!(
                decoded.writer_digest,
                Some(writer_digest),
                "format {format} {tags:?}"
            );
            assert_eq!(
                decoded.product.features[&FeatureId(2)].kind,
                snapshot.product().features[&FeatureId(2)].kind
            );
            assert_eq!(
                decoded.product.occurrences,
                snapshot.product().occurrences,
                "format {format} {tags:?}"
            );
        }
    }
}
