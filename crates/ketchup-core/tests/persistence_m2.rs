use ketchup_core::document::{
    BOTTLE_SHELL_OPENING_FACE_ROLE, BottleEdgeFinishKind, CanonicalCommand, CanonicalError,
    CanonicalOverride, ClassificationCategoryId, ClassificationDimensionId, CommandBatch,
    DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind, NodeId, OccurrenceId,
    OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotResolution, SlotSegment,
    StableEdgeRole, StableFaceRole, Transform,
};
#[cfg(not(feature = "named-product-fixtures"))]
use ketchup_core::persistence::LegacyFeatureKind;
use ketchup_core::persistence::{self, LoadDisposition, PersistenceError};

fn load_error(bytes: &[u8]) -> PersistenceError {
    match persistence::load(bytes) {
        Ok(_) => panic!("invalid document loaded"),
        Err(error) => error,
    }
}

fn rewrite_envelope_schema(bytes: &mut [u8], schema: u16) {
    let manifest_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let payload_offset = 16 + manifest_length;
    bytes[10..12].copy_from_slice(&schema.to_le_bytes());
    let payload_length = (bytes.len() - payload_offset) as u64;
    bytes[16..24].copy_from_slice(&payload_length.to_le_bytes());
    let checksum = ketchup_core::graph::sha256_bytes(&bytes[payload_offset..]);
    bytes[24..56].copy_from_slice(&checksum);
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

#[derive(Clone, Copy, Debug)]
enum LegacyAuthority {
    BottleProfileControl,
    RoleStringShell,
    BottleEdgeFinish,
}

fn legacy_authority_document(authority: LegacyAuthority) -> DocumentStore {
    const DEFINITION: DefinitionId = DefinitionId(18);
    const PROFILE: FeatureId = FeatureId(19);
    const LEGACY: FeatureId = FeatureId(20);
    const REVOLVE: FeatureId = FeatureId(21);

    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Legacy feature".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: PROFILE,
            definition_id: DEFINITION,
            name: "Profile".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![
                    [0.0, 0.0],
                    [10.0, 0.0],
                    [10.0, 20.0],
                    [5.0, 25.0],
                    [5.0, 30.0],
                    [0.0, 30.0],
                ],
            },
        },
    ];
    match authority {
        LegacyAuthority::BottleProfileControl => commands.extend([
            CanonicalCommand::CreateFeature {
                id: LEGACY,
                definition_id: DEFINITION,
                name: "Legacy authority".to_owned(),
                kind: FeatureKind::BottleProfileControl {
                    profile: PROFILE,
                    body_radius: Dimension::new("10", 10.0).unwrap(),
                    body_height: Dimension::new("20", 20.0).unwrap(),
                    shoulder_rise: Dimension::new("5", 5.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: REVOLVE,
                definition_id: DEFINITION,
                name: "Revolve".to_owned(),
                kind: FeatureKind::full_revolve(LEGACY),
            },
        ]),
        LegacyAuthority::RoleStringShell | LegacyAuthority::BottleEdgeFinish => {
            commands.push(CanonicalCommand::CreateFeature {
                id: REVOLVE,
                definition_id: DEFINITION,
                name: "Revolve".to_owned(),
                kind: FeatureKind::full_revolve(PROFILE),
            });
            let kind = match authority {
                LegacyAuthority::RoleStringShell => FeatureKind::Shell {
                    target: REVOLVE,
                    removed_faces: vec![
                        StableFaceRole::new(BOTTLE_SHELL_OPENING_FACE_ROLE).unwrap(),
                    ],
                    thickness: Dimension::new("2", 2.0).unwrap(),
                },
                LegacyAuthority::BottleEdgeFinish => FeatureKind::BottleEdgeFinish {
                    target: REVOLVE,
                    edges: vec![StableEdgeRole::new("revolve.shoulder").unwrap()],
                    kind: BottleEdgeFinishKind::Fillet,
                    amount: Dimension::new("1", 1.0).unwrap(),
                },
                LegacyAuthority::BottleProfileControl => unreachable!(),
            };
            commands.push(CanonicalCommand::CreateFeature {
                id: LEGACY,
                definition_id: DEFINITION,
                name: "Legacy authority".to_owned(),
                kind,
            });
        }
    }

    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(commands))
        .unwrap_or_else(|error| panic!("{authority:?}: {error:?}"));
    store
}

#[cfg(not(feature = "named-product-fixtures"))]
#[test]
fn default_load_rejects_legacy_named_feature_authority_with_typed_migration_error() {
    for (authority, kind) in [
        (
            LegacyAuthority::BottleProfileControl,
            LegacyFeatureKind::BottleProfileControl,
        ),
        (
            LegacyAuthority::RoleStringShell,
            LegacyFeatureKind::RoleStringShell,
        ),
        (
            LegacyAuthority::BottleEdgeFinish,
            LegacyFeatureKind::BottleEdgeFinish,
        ),
    ] {
        let bytes = persistence::save(&legacy_authority_document(authority).current());
        assert_eq!(
            load_error(&bytes),
            PersistenceError::LegacyFeatureRequiresMigration {
                feature_id: FeatureId(20),
                kind,
            }
        );
    }
}

#[cfg(not(feature = "named-product-fixtures"))]
#[test]
fn migration_required_primary_is_not_replaced_by_a_recovery_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.ketchup");
    std::fs::write(
        &path,
        persistence::save(
            &legacy_authority_document(LegacyAuthority::BottleProfileControl).current(),
        ),
    )
    .unwrap();
    std::fs::write(
        path.with_extension("ketchup.recovery"),
        persistence::save(&graph_document().current()),
    )
    .unwrap();

    assert!(matches!(
        persistence::load_file(&path),
        Err(persistence::FilePersistenceError::Format(
            PersistenceError::LegacyFeatureRequiresMigration {
                feature_id: FeatureId(20),
                kind: LegacyFeatureKind::BottleProfileControl,
            }
        ))
    ));
}

#[cfg(feature = "named-product-fixtures")]
#[test]
fn named_product_fixture_build_preserves_legacy_feature_decoding() {
    for authority in [
        LegacyAuthority::BottleProfileControl,
        LegacyAuthority::RoleStringShell,
        LegacyAuthority::BottleEdgeFinish,
    ] {
        let bytes = persistence::save(&legacy_authority_document(authority).current());
        let loaded = persistence::load(&bytes).unwrap();
        assert_eq!(loaded.disposition(), LoadDisposition::EditableLossless);
        assert!(matches!(
            loaded.snapshot().feature(FeatureId(20)).unwrap().kind(),
            FeatureKind::BottleProfileControl { .. }
                | FeatureKind::Shell { .. }
                | FeatureKind::BottleEdgeFinish { .. }
        ));
    }
}

#[test]
fn classification_dimensions_and_independent_assignments_round_trip_losslessly() {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Wall panel".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Panel".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("20", 20.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Wall panel occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(1),
                name: "Building side".to_owned(),
                categories: vec![
                    (ClassificationCategoryId(1), "Exterior".to_owned()),
                    (ClassificationCategoryId(2), "Interior".to_owned()),
                ],
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(2),
                name: "Building system".to_owned(),
                categories: vec![
                    (ClassificationCategoryId(3), "Structure".to_owned()),
                    (ClassificationCategoryId(4), "Electrical".to_owned()),
                ],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(2),
                category_id: Some(ClassificationCategoryId(3)),
            },
        ]))
        .unwrap();

    let expected_digest = store.current().canonical_digest();
    let bytes = persistence::save(&store.current());
    let loaded = persistence::load(&bytes).unwrap();
    assert_eq!(loaded.disposition(), LoadDisposition::EditableLossless);
    assert_eq!(loaded.snapshot().canonical_digest(), expected_digest);
    assert_eq!(
        loaded
            .snapshot()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(1)),
        Some(ClassificationCategoryId(1))
    );
    assert_eq!(
        loaded
            .snapshot()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(2)),
        Some(ClassificationCategoryId(3))
    );
    assert_eq!(persistence::save(&loaded.snapshot()), bytes);
}

#[test]
fn classification_replacement_is_dimension_local_atomic_and_undoable() {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Wall".to_owned(),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Wall occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(1),
                name: "Building side".to_owned(),
                categories: vec![
                    (ClassificationCategoryId(1), "Exterior".to_owned()),
                    (ClassificationCategoryId(2), "Interior".to_owned()),
                ],
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(2),
                name: "Building system".to_owned(),
                categories: vec![
                    (ClassificationCategoryId(3), "Structure".to_owned()),
                    (ClassificationCategoryId(4), "Electrical".to_owned()),
                ],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(2),
                category_id: Some(ClassificationCategoryId(3)),
            },
        ]))
        .unwrap();
    store.discard_history_before_current();
    let before = store.current().canonical_digest();

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(2)),
            },
        ]))
        .unwrap();
    assert_eq!(store.visible_undo_steps(), 1);
    assert_eq!(
        store
            .current()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(1)),
        Some(ClassificationCategoryId(2))
    );
    assert_eq!(
        store
            .current()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(2)),
        Some(ClassificationCategoryId(3))
    );
    assert_eq!(store.undo().unwrap().canonical_digest(), before);

    let stamp = store.current().canonical_digest();
    let missing_dimension = match store.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::SetOccurrenceClassification {
            occurrence_id: OccurrenceId(1),
            dimension_id: ClassificationDimensionId(99),
            category_id: Some(ClassificationCategoryId(1)),
        },
    ])) {
        Ok(_) => panic!("missing classification dimension was accepted"),
        Err(error) => error,
    };
    assert_eq!(
        missing_dimension,
        CanonicalError::ClassificationDimensionNotFound(ClassificationDimensionId(99))
    );
    assert_eq!(store.current().canonical_digest(), stamp);
    let missing_category = match store.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::SetOccurrenceClassification {
            occurrence_id: OccurrenceId(1),
            dimension_id: ClassificationDimensionId(1),
            category_id: Some(ClassificationCategoryId(99)),
        },
    ])) {
        Ok(_) => panic!("missing classification category was accepted"),
        Err(error) => error,
    };
    assert_eq!(
        missing_category,
        CanonicalError::ClassificationCategoryNotFound(
            ClassificationDimensionId(1),
            ClassificationCategoryId(99)
        )
    );
    assert_eq!(store.current().canonical_digest(), stamp);
}

#[test]
fn schema_46_document_without_cubic_data_loads_losslessly_after_schema_47_bump() {
    let snapshot = graph_document().current();
    let expected_digest = snapshot.canonical_digest();
    let schema_47 = persistence::save(&snapshot);
    let mut schema_46 = schema_47.clone();
    rewrite_envelope_schema(&mut schema_46, 46);

    let loaded = persistence::load(&schema_46).unwrap();
    assert_eq!(loaded.source_schema(), 46);
    assert_eq!(loaded.disposition(), LoadDisposition::EditableLossless);
    assert!(loaded.migration_losses().is_empty());
    assert_eq!(loaded.snapshot().canonical_digest(), expected_digest);
    assert_eq!(persistence::save(&loaded.snapshot()), schema_47);
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

#[test]
fn schema_52_occurrences_migrate_with_no_color() {
    let mut store = DocumentStore::new();
    let name = "Schema52UniqueOccurrenceMarker";
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Legacy".into(),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: name.into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let mut bytes = persistence::save(&store.current());
    // Schema 52 has precisely the same record except for the new optional-color byte.
    let name_offset = bytes
        .windows(name.len())
        .position(|b| b == name.as_bytes())
        .unwrap();
    let color_offset = name_offset + name.len() + 16 * 8 + 1 + 1 + 1;
    assert_eq!(bytes.remove(color_offset), 0);
    rewrite_envelope_schema(&mut bytes, 52);
    let loaded = persistence::load(&bytes).unwrap();
    assert_eq!(
        loaded
            .document()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .color(),
        None
    );
    assert_eq!(
        loaded.document().canonical_digest(),
        store.current().canonical_digest()
    );
}
