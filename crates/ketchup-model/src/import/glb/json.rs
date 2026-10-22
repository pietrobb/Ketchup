//! The JSON chunk of a GLB, read into what the import reads and nothing more (review
//! 2026-10-21 P3). A whole `serde_json::Value` of the chunk took 14 to 27 times its size
//! in the window. Keys the import never reads, `extras` among them, are skipped while
//! parsing; keys it only checks for are kept empty; the root arrays it only counts are
//! kept as their length; and the values it keeps are at most [`MAX_JSON_VALUES`], far
//! more than an importable GLB, of at most 512 nodes and meshes, holds.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt;

use serde::de::{DeserializeSeed, Deserializer, Error, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

/// Keys of glTF objects whose values the import reads; any other key is skipped.
const READ_KEYS: &[&str] = &[
    "accessors",
    "alphaMode",
    "asset",
    "attributes",
    "baseColorFactor",
    "buffer",
    "bufferView",
    "bufferViews",
    "buffers",
    "byteLength",
    "byteOffset",
    "byteStride",
    "children",
    "componentType",
    "count",
    "doubleSided",
    "emissiveFactor",
    "extensionsRequired",
    "indices",
    "material",
    "materials",
    "matrix",
    "mesh",
    "meshes",
    "minVersion",
    "mode",
    "name",
    "nodes",
    "normalized",
    "pbrMetallicRoughness",
    "primitives",
    "rotation",
    "scale",
    "scene",
    "scenes",
    "translation",
    "type",
    "uri",
    "version",
];

/// Keys the import only checks are there: kept as an empty object.
const PRESENCE_KEYS: &[&str] = &[
    "baseColorTexture",
    "camera",
    "emissiveTexture",
    "extensions",
    "normalTexture",
    "occlusionTexture",
    "skin",
    "sparse",
    "targets",
    "weights",
];

/// Keys whose object maps names the file chooses, attribute semantics, to values.
const NAMED_KEYS: &[&str] = &["attributes"];

/// Root arrays the import only counts, to name what it leaves out.
pub(super) const COUNTED_KEYS: &[&str] = &[
    "animations",
    "cameras",
    "extensionsUsed",
    "images",
    "samplers",
    "textures",
];

/// The JSON values the import keeps at most; an importable GLB holds some tens of
/// thousands. Kept, they take some tens of bytes each.
pub(super) const MAX_JSON_VALUES: usize = 1 << 18;

/// The JSON chunk as the import reads it.
pub(super) struct GlbJson {
    pub(super) root: Map<String, Value>,
    /// The length of each of [`COUNTED_KEYS`] the root holds.
    pub(super) counts: BTreeMap<&'static str, usize>,
}

/// Why the JSON chunk was not read.
pub(super) enum ReadError {
    /// More than [`MAX_JSON_VALUES`] values the import reads.
    TooLarge,
    Malformed(serde_json::Error),
    /// The root is no JSON object.
    NotObject,
}

pub(super) fn read(bytes: &[u8]) -> Result<GlbJson, ReadError> {
    let budget = Cell::new(MAX_JSON_VALUES);
    let counts = RefCell::new(BTreeMap::new());
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let root = Pruned {
        budget: &budget,
        counts: Some(&counts),
        all_keys: false,
    }
    .deserialize(&mut deserializer)
    .and_then(|root| deserializer.end().map(|()| root))
    .map_err(|error| {
        if budget.get() == 0 {
            ReadError::TooLarge
        } else {
            ReadError::Malformed(error)
        }
    })?;
    match root {
        Value::Object(root) => Ok(GlbJson {
            root,
            counts: counts.into_inner(),
        }),
        _ => Err(ReadError::NotObject),
    }
}

/// A JSON value as the import keeps it; `counts` is there at the root alone.
#[derive(Clone, Copy)]
struct Pruned<'a> {
    budget: &'a Cell<usize>,
    counts: Option<&'a RefCell<BTreeMap<&'static str, usize>>>,
    all_keys: bool,
}

impl Pruned<'_> {
    fn spend<E: Error>(self) -> Result<(), E> {
        let left = self.budget.get();
        if left == 0 {
            return Err(E::custom(format!(
                "the JSON holds more than {MAX_JSON_VALUES} values the import reads"
            )));
        }
        self.budget.set(left - 1);
        Ok(())
    }

    fn child(self, all_keys: bool) -> Self {
        Self {
            counts: None,
            all_keys,
            ..self
        }
    }
}

impl<'de> DeserializeSeed<'de> for Pruned<'_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Pruned<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: Error>(self, value: bool) -> Result<Value, E> {
        self.spend()?;
        Ok(Value::Bool(value))
    }

    fn visit_i64<E: Error>(self, value: i64) -> Result<Value, E> {
        self.spend()?;
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E: Error>(self, value: u64) -> Result<Value, E> {
        self.spend()?;
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E: Error>(self, value: f64) -> Result<Value, E> {
        self.spend()?;
        Ok(Number::from_f64(value).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E: Error>(self, value: &str) -> Result<Value, E> {
        self.spend()?;
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E: Error>(self, value: String) -> Result<Value, E> {
        self.spend()?;
        Ok(Value::String(value))
    }

    fn visit_unit<E: Error>(self) -> Result<Value, E> {
        self.spend()?;
        Ok(Value::Null)
    }

    fn visit_none<E: Error>(self) -> Result<Value, E> {
        self.visit_unit()
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        self.deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        self.spend()?;
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(self.child(false))? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        self.spend()?;
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if let Some(counts) = self.counts
                && let Some(name) = COUNTED_KEYS.iter().find(|name| **name == key)
            {
                let count = map.next_value_seed(Count)?;
                counts.borrow_mut().insert(name, count);
            } else if self.all_keys || READ_KEYS.contains(&key.as_str()) {
                let value = map.next_value_seed(self.child(NAMED_KEYS.contains(&key.as_str())))?;
                object.insert(key, value);
            } else if PRESENCE_KEYS.contains(&key.as_str()) {
                map.next_value::<IgnoredAny>()?;
                self.spend()?;
                object.insert(key, Value::Object(Map::new()));
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(Value::Object(object))
    }
}

/// The length of a JSON array, its elements skipped.
struct Count;

impl<'de> DeserializeSeed<'de> for Count {
    type Value = usize;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<usize, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for Count {
    type Value = usize;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON array")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<usize, A::Error> {
        let mut count = 0_usize;
        while seq.next_element::<IgnoredAny>()?.is_some() {
            count += 1;
        }
        Ok(count)
    }
}
