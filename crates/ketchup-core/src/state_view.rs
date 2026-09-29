//! Text projections of a snapshot for people and agents.
//!
//! Both views are rendered from the serde form the native file stores, so a new
//! document field appears in them without further code. The complete view has one
//! `path=value` line per leaf. The agent view has one line per top-level entry and
//! renders the document identity: evaluation evidence saved beside it is left out.

use crate::document::{OccurrenceId, Snapshot, identity_form};
use crate::graph::{EvaluationReport, SlotResolution};
use crate::space::ClearanceVolumeId;
use ciborium::Value;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write;

pub const COMPLETE_STATE_VIEW: &str = "ketchup.state-view.complete.v2";
pub const AGENT_STATE_VIEW: &str = "ketchup.state-view.agent.v2";

/// Depth below which the agent view writes a value on one line.
const AGENT_LINE_DEPTH: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticState {
    complete: String,
    agent: String,
}

impl SemanticState {
    #[must_use]
    pub fn complete(&self) -> String {
        self.complete.clone()
    }

    #[must_use]
    pub fn agent(&self) -> String {
        self.agent.clone()
    }
}

/// What the snapshot derives from its content rather than stores.
#[derive(Serialize)]
struct Analysis {
    occurrence_dof: BTreeMap<OccurrenceId, crate::assembly::AssemblyDofDiagnostic>,
    clearance_slot_health: BTreeMap<ClearanceVolumeId, SlotResolution>,
}

impl Analysis {
    fn of(snapshot: &Snapshot) -> Self {
        Self {
            occurrence_dof: snapshot
                .occurrences()
                .filter_map(|occurrence| {
                    snapshot
                        .assembly_dof_diagnostic(occurrence.id())
                        .map(|diagnostic| (occurrence.id(), diagnostic))
                })
                .collect(),
            clearance_slot_health: snapshot
                .clearance_volumes()
                .filter_map(|clearance| {
                    clearance
                        .derived_from()
                        .map(|identity| (clearance.id(), snapshot.resolve_slot(identity)))
                })
                .collect(),
        }
    }
}

#[must_use]
pub fn encode_semantic_state(snapshot: &Snapshot) -> SemanticState {
    encode_semantic_state_with_evaluation(snapshot, None)
}

#[must_use]
pub fn encode_semantic_state_with_evaluation(
    snapshot: &Snapshot,
    evaluation: Option<&EvaluationReport>,
) -> SemanticState {
    let saved = form(snapshot.product());
    let identity = identity_form(|| form(snapshot.product()));
    let analysis = form(&Analysis::of(snapshot));
    let evaluation_is_current = evaluation.is_some_and(|report| {
        report.document_id == Some(snapshot.document_id())
            && report.revision_id == Some(snapshot.revision_id())
            && report.canonical_digest.as_deref() == Some(snapshot.canonical_digest().as_str())
    });

    let mut complete = header(COMPLETE_STATE_VIEW, snapshot);
    let mut agent = header(AGENT_STATE_VIEW, snapshot);
    writeln!(agent, "summary.counts={}", counts(&identity)).unwrap();
    match evaluation {
        Some(report) => {
            let report = form(report);
            for output in [&mut complete, &mut agent] {
                writeln!(output, "evaluation.current={evaluation_is_current}").unwrap();
            }
            write_lines(&mut complete, "evaluation", &report, None);
            write_lines(&mut agent, "evaluation", &report, Some(AGENT_LINE_DEPTH));
        }
        None => {
            for output in [&mut complete, &mut agent] {
                writeln!(output, "evaluation=not_supplied").unwrap();
            }
        }
    }
    write_lines(&mut complete, "", &saved, None);
    write_lines(&mut complete, "analysis", &analysis, None);
    write_lines(&mut agent, "", &identity, Some(AGENT_LINE_DEPTH));
    write_lines(&mut agent, "analysis", &analysis, Some(AGENT_LINE_DEPTH));
    writeln!(agent, "intended_actions=canonical_command_batch_only").unwrap();
    SemanticState { complete, agent }
}

fn form(value: &impl Serialize) -> Value {
    Value::serialized(value).expect("document values serialize")
}

fn header(schema: &str, snapshot: &Snapshot) -> String {
    let mut output = String::new();
    writeln!(output, "schema={schema}").unwrap();
    writeln!(output, "source.revision={}", snapshot.revision_id()).unwrap();
    writeln!(
        output,
        "source.canonical_digest={}",
        snapshot.canonical_digest()
    )
    .unwrap();
    output
}

/// Entry count of every top-level collection of `model`.
fn counts(model: &Value) -> String {
    let Value::Map(fields) = model else {
        return String::new();
    };
    fields
        .iter()
        .filter_map(|(name, value)| {
            let count = match value {
                Value::Map(entries) => entries.len(),
                Value::Array(items) => items.len(),
                _ => return None,
            };
            Some(format!("{}:{count}", key(name)))
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Writes `value` as `path=value` lines; below `line_depth` a value stays on one line.
fn write_lines(output: &mut String, path: &str, value: &Value, line_depth: Option<usize>) {
    let branches = match value {
        Value::Map(entries) if !entries.is_empty() && line_depth != Some(0) => entries
            .iter()
            .map(|(name, value)| (key(name), value))
            .collect::<Vec<_>>(),
        Value::Array(items) if items.iter().any(has_fields) && line_depth != Some(0) => items
            .iter()
            .enumerate()
            .map(|(index, value)| (index.to_string(), value))
            .collect(),
        Value::Tag(_, inner) => return write_lines(output, path, inner, line_depth),
        _ => {
            writeln!(output, "{path}={}", inline(value)).unwrap();
            return;
        }
    };
    for (segment, value) in branches {
        let child = if path.is_empty() {
            segment
        } else {
            format!("{path}.{segment}")
        };
        write_lines(output, &child, value, line_depth.map(|depth| depth - 1));
    }
}

fn has_fields(value: &Value) -> bool {
    match value {
        Value::Map(_) => true,
        Value::Array(items) => items.iter().any(has_fields),
        Value::Tag(_, inner) => has_fields(inner),
        _ => false,
    }
}

/// One path segment: a field or variant name as written, any other key on one line.
fn key(value: &Value) -> String {
    match value {
        Value::Text(text)
            if !text.is_empty()
                && text
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_') =>
        {
            text.clone()
        }
        Value::Integer(number) => i128::from(*number).to_string(),
        Value::Array(items) => items.iter().map(key).collect::<Vec<_>>().join(":"),
        Value::Map(entries) => entries
            .iter()
            .map(|(_, value)| key(value))
            .collect::<Vec<_>>()
            .join(":"),
        Value::Tag(_, inner) => key(inner),
        other => inline(other),
    }
}

/// A value on one line; floats keep the shortest text that reads back to the same bits.
fn inline(value: &Value) -> String {
    match value {
        Value::Integer(number) => i128::from(*number).to_string(),
        Value::Float(number) => format!("{number:?}"),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "none".to_owned(),
        Value::Text(text) => format!("{text:?}"),
        Value::Bytes(bytes) => bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(inline).collect::<Vec<_>>().join(",")
        ),
        Value::Map(entries) => format!(
            "{{{}}}",
            entries
                .iter()
                .map(|(name, value)| format!("{}:{}", key(name), inline(value)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Tag(_, inner) => inline(inner),
        _ => "unknown".to_owned(),
    }
}

/// The name of the enum variant `value` serializes as, without serializing its payload.
#[must_use]
pub fn variant_name(value: &impl Serialize) -> Option<&'static str> {
    match value.serialize(VariantName) {
        Err(VariantError::Found(name)) => Some(name),
        _ => None,
    }
}

struct VariantName;

#[derive(Debug)]
enum VariantError {
    Found(&'static str),
    NotAnEnum,
}

impl std::fmt::Display for VariantError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for VariantError {}

impl serde::ser::Error for VariantError {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Self::NotAnEnum
    }
}

macro_rules! not_an_enum {
    ($($method:ident($($argument:ty),*) -> $result:ty;)*) => {
        $(fn $method(self, $(_: $argument),*) -> Result<$result, VariantError> {
            Err(VariantError::NotAnEnum)
        })*
    };
}

impl serde::Serializer for VariantName {
    type Ok = ();
    type Error = VariantError;
    type SerializeSeq = serde::ser::Impossible<(), VariantError>;
    type SerializeTuple = serde::ser::Impossible<(), VariantError>;
    type SerializeTupleStruct = serde::ser::Impossible<(), VariantError>;
    type SerializeTupleVariant = serde::ser::Impossible<(), VariantError>;
    type SerializeMap = serde::ser::Impossible<(), VariantError>;
    type SerializeStruct = serde::ser::Impossible<(), VariantError>;
    type SerializeStructVariant = serde::ser::Impossible<(), VariantError>;

    not_an_enum! {
        serialize_bool(bool) -> ();
        serialize_i8(i8) -> ();
        serialize_i16(i16) -> ();
        serialize_i32(i32) -> ();
        serialize_i64(i64) -> ();
        serialize_u8(u8) -> ();
        serialize_u16(u16) -> ();
        serialize_u32(u32) -> ();
        serialize_u64(u64) -> ();
        serialize_f32(f32) -> ();
        serialize_f64(f64) -> ();
        serialize_char(char) -> ();
        serialize_str(&str) -> ();
        serialize_bytes(&[u8]) -> ();
        serialize_none() -> ();
        serialize_unit() -> ();
        serialize_unit_struct(&'static str) -> ();
        serialize_seq(Option<usize>) -> Self::SerializeSeq;
        serialize_tuple(usize) -> Self::SerializeTuple;
        serialize_tuple_struct(&'static str, usize) -> Self::SerializeTupleStruct;
        serialize_map(Option<usize>) -> Self::SerializeMap;
        serialize_struct(&'static str, usize) -> Self::SerializeStruct;
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), VariantError> {
        value.serialize(self)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), VariantError> {
        value.serialize(self)
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<(), VariantError> {
        Err(VariantError::Found(variant))
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: &T,
    ) -> Result<(), VariantError> {
        Err(VariantError::Found(variant))
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, VariantError> {
        Err(VariantError::Found(variant))
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, VariantError> {
        Err(VariantError::Found(variant))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_keys_and_leaves_render_without_ambiguity() {
        let value = form(&BTreeMap::from([
            ((3_u64, 4_u64), vec![1.5_f64, -0.0]),
            ((5, 6), vec![]),
        ]));
        let mut output = String::new();
        write_lines(&mut output, "root", &value, None);
        assert_eq!(output, "root.3:4=[1.5,-0.0]\nroot.5:6=[]\n");
        assert_eq!(key(&Value::Text("a.b".to_owned())), "\"a.b\"");
    }

    #[test]
    fn agent_depth_folds_deeper_values_onto_one_line() {
        let value = form(&BTreeMap::from([(
            "features",
            BTreeMap::from([(7_u64, BTreeMap::from([("name", "Pad")]))]),
        )]));
        let mut output = String::new();
        write_lines(&mut output, "", &value, Some(2));
        assert_eq!(output, "features.7={name:\"Pad\"}\n");
    }

    #[test]
    fn variant_name_reads_only_the_tag_of_an_enum() {
        #[derive(Serialize)]
        enum Kind {
            Unit,
            Newtype(u8),
            Struct { payload: Vec<u8> },
        }
        assert_eq!(variant_name(&Kind::Unit), Some("Unit"));
        assert_eq!(variant_name(&Kind::Newtype(1)), Some("Newtype"));
        assert_eq!(
            variant_name(&Some(Kind::Struct {
                payload: vec![0; 3]
            })),
            Some("Struct")
        );
        assert_eq!(variant_name(&3_u8), None);
    }
}
