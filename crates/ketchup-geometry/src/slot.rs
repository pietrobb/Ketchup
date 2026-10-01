//! Identity of a value a rule node derives: the root rule and the path of output slots to it.

use crate::id::NodeId;
use ketchup_tolerance::limits;
use std::fmt;

#[derive(
    Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct SlotSegment {
    pub producer_rule_id: NodeId,
    pub output_port: String,
    pub semantic_key: String,
}

impl SlotSegment {
    pub fn new(
        producer_rule_id: NodeId,
        output_port: impl Into<String>,
        semantic_key: impl Into<String>,
    ) -> Result<Self, SlotError> {
        if producer_rule_id.0 == 0 {
            return Err(SlotError::ReservedNodeId);
        }
        let output_port = output_port.into();
        let semantic_key = semantic_key.into();
        ensure_semantic_key(&output_port)?;
        ensure_semantic_key(&semantic_key)?;
        Ok(Self {
            producer_rule_id,
            output_port,
            semantic_key,
        })
    }
}

#[derive(
    Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct SlotPath(Vec<SlotSegment>);

impl SlotPath {
    pub fn new(segments: Vec<SlotSegment>) -> Result<Self, SlotError> {
        if segments.is_empty() {
            return Err(SlotError::EmptySlotPath);
        }
        if segments.len() > limits::PATH_SEGMENTS {
            return Err(SlotError::SlotPathLimit);
        }
        Ok(Self(segments))
    }

    #[must_use]
    pub fn segments(&self) -> &[SlotSegment] {
        &self.0
    }
}

#[derive(
    Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct DerivedIdentity {
    pub root_rule_node_id: NodeId,
    pub slot_path: SlotPath,
}

impl DerivedIdentity {
    pub fn new(root_rule_node_id: NodeId, slot_path: SlotPath) -> Result<Self, SlotError> {
        if root_rule_node_id.0 == 0 {
            return Err(SlotError::ReservedNodeId);
        }
        Ok(Self {
            root_rule_node_id,
            slot_path,
        })
    }
}

/// A semantic key or port name: non-empty, at most 256 bytes, no control characters.
pub fn ensure_semantic_key(value: &str) -> Result<(), SlotError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(SlotError::InvalidSemanticKey)
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotError {
    ReservedNodeId,
    InvalidSemanticKey,
    EmptySlotPath,
    SlotPathLimit,
}

impl fmt::Display for SlotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ReservedNodeId => "node ID zero is reserved",
            Self::InvalidSemanticKey => "semantic key is invalid",
            Self::EmptySlotPath => "slot path must not be empty",
            Self::SlotPathLimit => "slot path exceeds its segment limit",
        })
    }
}

impl std::error::Error for SlotError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_identities_reject_reserved_ids_bad_keys_and_bad_paths() {
        let segment = SlotSegment::new(NodeId(1), "out", "width").unwrap();
        assert_eq!(
            SlotSegment::new(NodeId(0), "out", "width"),
            Err(SlotError::ReservedNodeId)
        );
        assert_eq!(
            SlotSegment::new(NodeId(1), "", "width"),
            Err(SlotError::InvalidSemanticKey)
        );
        assert_eq!(
            SlotSegment::new(NodeId(1), "out", "a\nb"),
            Err(SlotError::InvalidSemanticKey)
        );
        assert_eq!(SlotPath::new(Vec::new()), Err(SlotError::EmptySlotPath));
        assert_eq!(
            SlotPath::new(vec![segment.clone(); limits::PATH_SEGMENTS + 1]),
            Err(SlotError::SlotPathLimit)
        );
        let path = SlotPath::new(vec![segment]).unwrap();
        assert_eq!(
            DerivedIdentity::new(NodeId(0), path.clone()),
            Err(SlotError::ReservedNodeId)
        );
        assert_eq!(
            DerivedIdentity::new(NodeId(2), path)
                .unwrap()
                .root_rule_node_id,
            NodeId(2)
        );
    }
}
