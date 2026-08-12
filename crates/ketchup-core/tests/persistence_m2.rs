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

#[test]
fn lossy_legacy_migration_requires_one_explicit_canonical_batch() {
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
    let migration = loaded
        .review_candidate()
        .unwrap()
        .confirm_semantic_migration()
        .unwrap();
    assert_eq!(migration.source_schema(), 0);
    assert_eq!(migration.source_revision_id(), 7);
    assert_eq!(migration.confirmed_revision_id(), 8);
    assert_eq!(migration.losses(), loaded.migration_losses());

    let (document, sidecars) = migration.into_parts();
    assert_eq!(document.revision_count(), 2);
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(document.current().revision_id(), 8);
    assert_eq!(
        document
            .current()
            .evaluator_node(NodeId(42))
            .unwrap()
            .dimension()
            .unwrap()
            .source_token(),
        "3.50000000000000000"
    );
    let current =
        persistence::load(&persistence::save_container(&document.current(), &sidecars).unwrap())
            .unwrap();
    assert_eq!(current.disposition(), LoadDisposition::EditableLossless);
    assert_eq!(current.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(current.snapshot().revision_id(), 8);
}

#[test]
fn required_unknown_extension_cannot_be_confirmed_as_a_semantic_migration() {
    let store = graph_document();
    let mut sidecars = persistence::ContainerData::default();
    sidecars
        .insert_extension(
            persistence::ExtensionEntry::new(
                "com.vendor.required",
                "meaning.bin",
                true,
                b"unknown semantics".to_vec(),
            )
            .unwrap(),
        )
        .unwrap();
    let loaded =
        persistence::load(&persistence::save_container(&store.current(), &sidecars).unwrap())
            .unwrap();
    assert!(matches!(
        loaded
            .review_candidate()
            .unwrap()
            .confirm_semantic_migration(),
        Err(PersistenceError::MigrationNotConfirmable(
            "candidate requires an unknown extension"
        ))
    ));
}

#[test]
fn native_container_preserves_optional_extensions_and_content_addressed_blobs() {
    let store = graph_document();
    let mut sidecars = persistence::ContainerData::default();
    let blob = b"bounded auxiliary geometry".to_vec();
    let blob_hash = sidecars.insert_blob(blob.clone()).unwrap();
    sidecars
        .insert_extension(
            persistence::ExtensionEntry::new(
                "org.example.audit",
                "opaque/payload.bin",
                false,
                vec![0, 1, 2, 255],
            )
            .unwrap(),
        )
        .unwrap();

    let first = persistence::save_container(&store.current(), &sidecars).unwrap();
    let loaded = persistence::load(&first).unwrap();
    assert_eq!(loaded.disposition(), LoadDisposition::EditableLossless);
    assert_eq!(
        loaded.snapshot().canonical_digest(),
        store.current().canonical_digest()
    );
    assert_eq!(loaded.container_data().blobs()[&blob_hash], blob);
    let extension = loaded.container_data().extensions().next().unwrap();
    assert_eq!(extension.namespace(), "org.example.audit");
    assert_eq!(extension.path(), "opaque/payload.bin");
    assert_eq!(extension.bytes(), &[0, 1, 2, 255]);
    assert!(!extension.required());
    assert_eq!(loaded.audit().unknown_extensions.len(), 1);

    let second = persistence::save_container(&loaded.snapshot(), loaded.container_data()).unwrap();
    assert_eq!(
        second, first,
        "unchanged container re-save must be deterministic"
    );
}

#[test]
fn unknown_required_extension_is_preserved_but_blocks_editing() {
    let store = graph_document();
    let mut sidecars = persistence::ContainerData::default();
    sidecars
        .insert_extension(
            persistence::ExtensionEntry::new(
                "com.vendor.required",
                "meaning.bin",
                true,
                b"unknown semantics".to_vec(),
            )
            .unwrap(),
        )
        .unwrap();

    let bytes = persistence::save_container(&store.current(), &sidecars).unwrap();
    let loaded = persistence::load(&bytes).unwrap();
    assert_eq!(loaded.disposition(), LoadDisposition::ReviewOnly);
    assert!(loaded.editable_document().is_none());
    assert_eq!(loaded.audit().unknown_extensions.len(), 1);
    assert!(loaded.audit().unknown_extensions[0].required);
    assert_eq!(
        loaded.container_data().extensions().next().unwrap().bytes(),
        b"unknown semantics"
    );
}

#[test]
fn native_container_fails_closed_on_unsafe_paths_and_corruption() {
    assert!(matches!(
        persistence::ExtensionEntry::new("org.example", "../escape", false, vec![]),
        Err(PersistenceError::InvalidContainerPath(_))
    ));

    let store = graph_document();
    let mut bytes =
        persistence::save_container(&store.current(), &persistence::ContainerData::default())
            .unwrap();
    *bytes.last_mut().unwrap() ^= 0xff;
    assert!(matches!(
        load_error(&bytes),
        PersistenceError::ContainerChecksumMismatch(path) if path == "document.bin"
    ));
}

#[test]
fn interrupted_or_corrupt_primary_recovers_the_last_verified_container() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("recoverable.ketchup");
    let mut store = graph_document();
    let first_digest = store.current().canonical_digest();
    persistence::save_atomic(&path, &store.current()).unwrap();

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("9", 9.0).unwrap(),
            },
        ]))
        .unwrap();
    persistence::save_atomic(&path, &store.current()).unwrap();
    assert!(path.with_extension("ketchup.recovery").is_file());

    std::fs::write(&path, b"interrupted replacement").unwrap();
    let recovered = persistence::load_file(&path).unwrap();
    assert_eq!(recovered.snapshot().canonical_digest(), first_digest);
    assert!(recovered.audit().recovered_from_backup);
}
