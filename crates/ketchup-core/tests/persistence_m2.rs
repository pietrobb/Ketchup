use ketchup_core::document::{
    CanonicalCommand, CanonicalOverride, CommandBatch, Dimension, DocumentStore, NodeId,
    OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotResolution, SlotSegment,
};
use ketchup_core::persistence::{self, LoadDisposition, PersistenceError};

fn load_error(bytes: &[u8]) -> PersistenceError {
    match persistence::load(bytes) {
        Ok(_) => panic!("invalid document loaded"),
        Err(error) => error,
    }
}

fn graph_document() -> DocumentStore {
    let segment = SlotSegment::new(NodeId(2), "items", "a").unwrap();
    let path = SlotPath::new(vec![segment.clone()]).unwrap();
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "p".into(),
                dimension: Dimension::new("2", 2.0).unwrap(),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: NodeId(2),
                name: "r".into(),
                expression: "$1 * 3".into(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("items").unwrap()],
                outputs: vec![RuleOutput::new(segment, vec![]).unwrap()],
                override_parameters: vec![OverrideParameterSpec::replace("authored_only").unwrap()],
            },
            CanonicalCommand::UpsertOverride(
                CanonicalOverride::new(
                    1,
                    ketchup_core::document::DerivedIdentity::new(NodeId(2), path).unwrap(),
                    "authored_only",
                    7.0,
                    SlotResolution::Resolved,
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    store
}

#[test]
fn schema_three_checks_checksum_before_payload_decode_and_rejects_envelopes() {
    let bytes = persistence::save(&graph_document().current());
    let mut corrupt = bytes.clone();
    let payload = 16 + u32::from_le_bytes(corrupt[12..16].try_into().unwrap()) as usize;
    corrupt[payload] ^= 0xff;
    assert_eq!(load_error(&corrupt), PersistenceError::ChecksumMismatch);
    assert_eq!(
        load_error(&bytes[..bytes.len() - 1]),
        PersistenceError::InvalidEnvelopeLength
    );
    let mut unsupported = bytes;
    unsupported[10..12].copy_from_slice(&99_u16.to_le_bytes());
    assert_eq!(
        load_error(&unsupported),
        PersistenceError::UnsupportedSchema(99)
    );
    assert_eq!(
        load_error(&vec![0; 32 * 1024 * 1024 + 1]),
        PersistenceError::ResourceLimit
    );
}

#[test]
fn exact_override_declarations_round_trip_and_lossless_load_is_editable() {
    let store = graph_document();
    let expected = store.current().canonical_digest();
    let loaded = persistence::load(&persistence::save(&store.current())).unwrap();
    assert_eq!(loaded.disposition(), LoadDisposition::EditableLossless);
    let snapshot = loaded.snapshot();
    assert_eq!(snapshot.canonical_digest(), expected);
    let rule = snapshot.evaluator_node(NodeId(2)).unwrap();
    assert_eq!(rule.allowed_parameters()[0].name(), "authored_only");
    assert_ne!(
        rule.allowed_parameters()[0].name(),
        rule.output_ports()[0].name()
    );
    assert!(loaded.editable_document().is_some());
    assert!(loaded.review_candidate().is_none());
}

#[test]
fn unresolved_override_load_is_a_read_only_audited_review_candidate() {
    let mut store = graph_document();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetRuleOutputs {
            id: NodeId(2),
            outputs: vec![],
        }]))
        .unwrap();
    let loaded = persistence::load(&persistence::save(&store.current())).unwrap();
    assert_eq!(loaded.disposition(), LoadDisposition::ReviewOnly);
    assert!(loaded.editable_document().is_none());
    assert!(loaded.review_candidate().is_some());
    assert_eq!(
        loaded.audit().override_health[0].audited,
        SlotResolution::Lost { segment_index: 0 }
    );
    assert_eq!(
        loaded.document().canonical_digest(),
        store.current().canonical_digest()
    );
}

#[test]
fn legacy_loss_is_review_only_and_failed_load_does_not_replace_live_state() {
    let mut legacy = b"KETCHUPDOC".to_vec();
    legacy.extend_from_slice(&0_u16.to_le_bytes());
    legacy.extend_from_slice(&7_u64.to_le_bytes());
    legacy.extend_from_slice(&1_u32.to_le_bytes());
    legacy.extend_from_slice(&42_u64.to_le_bytes());
    legacy.extend_from_slice(&1_u32.to_le_bytes());
    legacy.push(b'x');
    legacy.extend_from_slice(&3.5_f64.to_bits().to_le_bytes());
    legacy.extend_from_slice(&0_u32.to_le_bytes());
    let loaded = persistence::load(&legacy).unwrap();
    assert_eq!(loaded.disposition(), LoadDisposition::ReviewOnly);
    assert_eq!(loaded.migration_losses().len(), 1);
    let live = graph_document();
    let digest = live.current().canonical_digest();
    assert!(persistence::load(b"bad").is_err());
    assert_eq!(live.current().canonical_digest(), digest);
}
