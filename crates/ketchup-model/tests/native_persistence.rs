use ketchup_model::document::{
    CanonicalCommand, CanonicalError, CanonicalOverride, ClassificationCategoryId,
    ClassificationDimensionId, ClassificationError, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind, NodeId, OccurrenceId, OverrideParameterSpec, PortSpec,
    ProposalContext, ProposalPrincipal, RevisionHistoryError, RevisionOrigin, RuleOutput, SlotPath,
    SlotResolution, SlotSegment, Transform,
};
use ketchup_model::persistence::LegacyFeatureKind;
use ketchup_model::persistence::{
    self, LoadDisposition, LoadOutcome, MigrationBlocker, PersistenceError,
};
use ketchup_model::sheet_metal::{SheetMetalBend, SheetMetalSpec};
use ketchup_model::testing::with_document_id;
use ketchup_model::tolerance::TolerancePolicy;
use ketchup_model::tolerance::limits;

#[test]
fn duplicate_extension_keeps_original_payload_and_requirement_after_roundtrip() {
    let mut data = persistence::ContainerData::default();
    let original =
        persistence::ExtensionEntry::new("org.example.audit", "data.bin", false, b"A".to_vec())
            .unwrap();
    data.insert_extension(original.clone()).unwrap();
    assert!(
        data.insert_extension(
            persistence::ExtensionEntry::new("org.example.audit", "data.bin", true, b"B".to_vec())
                .unwrap()
        )
        .is_err()
    );
    assert_eq!(data.extensions().next(), Some(&original));
    let bytes = persistence::save_document_store(&graph_document(), &data).unwrap();
    let (_, reopened) = persistence::load(&bytes)
        .unwrap()
        .into_editable_with_container()
        .unwrap_or_else(|_| panic!("expected editable roundtrip"));
    assert_eq!(reopened.extensions().next(), Some(&original));
}

#[test]
fn source_only_rollback_restores_source_and_overrides_through_save_open() {
    use ketchup_model::document::RuleProgramSource;
    let mut document = graph_document();
    let source = RuleProgramSource {
        variants: Default::default(),
        file_name: "model.star".into(),
        source: "# A\n".into(),
        overrides: std::collections::BTreeMap::from([("H".into(), 10.0)]),
    };
    document.bind_rule_program(source.clone()).unwrap();
    let target = document.current().revision_id();
    let mut changed = source.clone();
    changed.source = "# B\n".into();
    changed.overrides.insert("H".into(), 20.0);
    document
        .replace_rule_program_source(changed.clone())
        .unwrap();
    let before = document.current();
    document
        .rollback_to_revision(
            before.revision_id(),
            &before.canonical_digest(),
            target,
            ProposalPrincipal::ManualClient,
        )
        .unwrap();
    assert_eq!(document.current_rule_program(), Some(&source));
    document.undo().unwrap();
    assert_eq!(document.current_rule_program(), Some(&changed));
    document.redo().unwrap();
    assert_eq!(document.current_rule_program(), Some(&source));
    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    let (reopened, _) = persistence::load(&bytes)
        .unwrap()
        .into_editable_with_container()
        .unwrap_or_else(|_| panic!("expected editable roundtrip"));
    assert_eq!(reopened.current_rule_program(), Some(&source));
    let current = document.current();
    assert!(matches!(
        document.rollback_to_revision(
            current.revision_id(),
            &current.canonical_digest(),
            target,
            ProposalPrincipal::ManualClient
        ),
        Err(RevisionHistoryError::NoOpRollback)
    ));
    // Overrides alone also form a distinct revision with the same product.
    let mut override_only = source.clone();
    override_only.overrides.insert("H".into(), 30.0);
    document.replace_rule_program_source(override_only).unwrap();
    let current = document.current();
    document
        .rollback_to_revision(
            current.revision_id(),
            &current.canonical_digest(),
            target,
            ProposalPrincipal::ManualClient,
        )
        .unwrap();
    assert_eq!(document.current_rule_program(), Some(&source));
}

fn variants(
    entries: &[(&str, &[(&str, f64)])],
) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>> {
    entries
        .iter()
        .map(|(name, values)| {
            let values = values
                .iter()
                .map(|(parameter, value)| ((*parameter).to_owned(), *value))
                .collect();
            ((*name).to_owned(), values)
        })
        .collect()
}

fn program_with_variants() -> ketchup_model::document::RuleProgramSource {
    ketchup_model::document::RuleProgramSource {
        file_name: "cabinet.star".into(),
        source: "w = param('width', 600)\n".into(),
        overrides: std::collections::BTreeMap::from([("width".into(), 800.0)]),
        variants: variants(&[
            ("600", &[("width", 600.0), ("depth", 400.0)]),
            ("800", &[("width", 800.0), ("depth", 400.0)]),
            ("1000", &[("width", 1000.0), ("depth", 560.0)]),
        ]),
    }
}

#[test]
fn program_variants_survive_save_open_and_a_variant_change_is_its_own_revision() {
    let mut document = graph_document();
    let program = program_with_variants();
    document.bind_rule_program(program.clone()).unwrap();
    let bound = document.current();

    let mut fewer = program.clone();
    fewer.variants.remove("1000");
    document.replace_rule_program_source(fewer.clone()).unwrap();
    assert_ne!(document.current().revision_id(), bound.revision_id());
    // Same source and overrides, only the variants differ: the existing source
    // digest of the revision tells them apart.
    document
        .replace_rule_program_source(program.clone())
        .unwrap();
    document.replace_rule_program_source(fewer.clone()).unwrap();
    let digests = document
        .revision_history()
        .skip(2)
        .map(|revision| revision.batch_digest().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(digests.len(), 3);
    assert_ne!(digests[0], digests[1]);
    assert_eq!(digests[0], digests[2]);

    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    let (mut reopened, _) = ketchup_model::testing::load_with_history(&bytes)
        .unwrap()
        .into_editable_with_container()
        .unwrap_or_else(|_| panic!("expected editable roundtrip"));
    assert_eq!(reopened.current_rule_program(), Some(&fewer));
    reopened.undo().unwrap();
    assert_eq!(reopened.current_rule_program(), Some(&program));
    assert_eq!(program.variants.len(), 3);
}

#[test]
fn a_program_without_variants_encodes_as_before_and_an_older_program_opens_without_them() {
    let mut program = program_with_variants();
    program.variants.clear();
    let encoded = String::from_utf8(serde_json::to_vec(&program).unwrap()).unwrap();
    assert_eq!(
        encoded,
        r#"{"file_name":"cabinet.star","source":"w = param('width', 600)\n","overrides":{"width":800.0}}"#,
        "no variants key, so the source digest of every older revision is unchanged"
    );
    let older: ketchup_model::document::RuleProgramSource = serde_json::from_str(&encoded).unwrap();
    assert!(older.variants.is_empty());

    let mut document = graph_document();
    document.bind_rule_program(program.clone()).unwrap();
    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    let (reopened, _) = persistence::load(&bytes)
        .unwrap()
        .into_editable_with_container()
        .unwrap_or_else(|_| panic!("expected editable roundtrip"));
    assert_eq!(reopened.current_rule_program(), Some(&program));
}

/// `bytes` with the program JSON of its only programmed revision replaced by `json`.
fn with_program_json(document: &DocumentStore, bytes: &[u8], json: &str) -> Vec<u8> {
    let (_, offset, len) = container_entry_offsets(bytes, "history.bin");
    let history = &bytes[offset..offset + len];
    let start = history
        .windows(br#"{"file_name""#.len())
        .position(|window| window == br#"{"file_name""#)
        .expect("the history holds the program");
    // A program this small is one inline chunk; its revision names the chunk's
    // length and SHA-256.
    let old_len = u32::from_le_bytes(history[start - 4..start].try_into().unwrap()) as usize;
    let old_checksum = ketchup_model::graph::sha256_bytes(&history[start..start + old_len]);
    let mut patched = history[..start - 4].to_vec();
    patched.extend_from_slice(&(json.len() as u32).to_le_bytes());
    patched.extend_from_slice(json.as_bytes());
    let records_start = patched.len();
    patched.extend_from_slice(&history[start + old_len..]);
    let checksum = records_start
        + patched[records_start..]
            .windows(32)
            .position(|window| window == old_checksum)
            .expect("the revision names the program's checksum");
    patched[checksum - 8..checksum].copy_from_slice(&(json.len() as u64).to_le_bytes());
    patched[checksum..checksum + 32]
        .copy_from_slice(&ketchup_model::graph::sha256_bytes(json.as_bytes()));
    ketchup_model::testing::save_with_history_entry(document, patched)
}

#[test]
fn an_invalid_variant_is_refused_by_name_in_program_json_and_costs_the_history_in_an_undo_step() {
    use ketchup_model::document::{MAX_RULE_PROGRAM_VARIANTS, RuleProgramError};
    let mut document = graph_document();
    let program = program_with_variants();
    document.bind_rule_program(program.clone()).unwrap();
    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    // The untouched splice opens, so each refusal below is the variant's.
    let valid = with_program_json(
        &document,
        &bytes,
        &String::from_utf8(serde_json::to_vec(&program).unwrap()).unwrap(),
    );
    assert!(persistence::load(&valid).is_ok());

    let program_json = |variants: &str| {
        format!(
            r#"{{"file_name":"cabinet.star","source":"w = 1\n","overrides":{{}},"variants":{variants}}}"#
        )
    };
    let too_many = (0..=MAX_RULE_PROGRAM_VARIANTS)
        .map(|index| format!(r#""v{index}":{{"width":{index}}}"#))
        .collect::<Vec<_>>()
        .join(",");
    for (variants, expected) in [
        (
            r#"{"":{"width":600}}"#.to_owned(),
            RuleProgramError::EmptyVariantName,
        ),
        (
            r#"{"  ":{"width":600}}"#.to_owned(),
            RuleProgramError::EmptyVariantName,
        ),
        // serde_json writes NaN as null.
        (
            r#"{"800":{"depth":400,"width":null}}"#.to_owned(),
            RuleProgramError::NonFiniteVariantValue {
                variant: "800".into(),
                parameter: "width".into(),
            },
        ),
        (
            format!("{{{too_many}}}"),
            RuleProgramError::TooManyVariants {
                count: MAX_RULE_PROGRAM_VARIANTS + 1,
            },
        ),
    ] {
        // Review 2026-10-15 status N-5: the file stores the document's program
        // in program.json, so a bad program of an Undo step costs the steps,
        // with the reason, and not the file.
        let damaged = with_program_json(&document, &bytes, &program_json(&variants));
        let reason = discarded_history(&damaged);
        assert!(
            reason.contains(&expected.to_string()),
            "{variants}: {reason}"
        );

        // The program the file stores beside the document is refused by name.
        let (_, offset, len) = container_entry_offsets(&bytes, "history.bin");
        let refused = ketchup_model::testing::save_with_program_entry(
            &document,
            bytes[offset..offset + len].to_vec(),
            program_json(&variants).into_bytes(),
        );
        let error = load_error(&refused);
        assert_eq!(
            error,
            PersistenceError::InvalidRuleProgram(expected.clone()),
            "{variants}"
        );
        assert!(error.to_string().contains(&expected.to_string()), "{error}");
    }

    // The window cannot store such a program either, and nothing changes.
    let before = document.current();
    for (name, value) in [("", 600.0), ("800", f64::NAN), ("800", f64::INFINITY)] {
        let mut invalid = program.clone();
        invalid.variants = variants(&[(name, &[("width", value)])]);
        assert!(matches!(
            document.replace_rule_program_source(invalid),
            Err(CanonicalError::RuleProgram(
                RuleProgramError::EmptyVariantName | RuleProgramError::NonFiniteVariantValue { .. }
            ))
        ));
    }
    let mut crowded = program.clone();
    crowded.variants = (0..=MAX_RULE_PROGRAM_VARIANTS)
        .map(|index| (format!("v{index}"), Default::default()))
        .collect();
    assert_eq!(
        document.replace_rule_program_source(crowded).unwrap_err(),
        CanonicalError::RuleProgram(RuleProgramError::TooManyVariants {
            count: MAX_RULE_PROGRAM_VARIANTS + 1
        })
    );
    assert_eq!(document.current().revision_id(), before.revision_id());
    assert_eq!(document.current_rule_program(), Some(&program));
}

/// Review 2026-10-17 M-9: a `program.json` larger than Save writes would open,
/// and then every checkpoint of the window fails on the same program.
#[test]
fn a_program_json_over_the_save_limit_is_refused_on_open() {
    use ketchup_model::document::MAX_RULE_PROGRAM_BYTES;
    let mut document = graph_document();
    let program = program_with_variants();
    document.bind_rule_program(program.clone()).unwrap();
    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    let (_, offset, len) = container_entry_offsets(&bytes, "history.bin");
    let history = bytes[offset..offset + len].to_vec();
    let mut padded = program.clone();
    padded.source.push_str("\n# ");
    padded.source.push_str(&"x".repeat(MAX_RULE_PROGRAM_BYTES));
    let oversized = ketchup_model::testing::save_with_program_entry(
        &document,
        history.clone(),
        serde_json::to_vec(&padded).unwrap(),
    );
    assert_eq!(load_error(&oversized), PersistenceError::ResourceLimit);
    // The same container with the program at its saved size opens.
    let fitting = ketchup_model::testing::save_with_program_entry(
        &document,
        history,
        serde_json::to_vec(&program).unwrap(),
    );
    assert!(persistence::load(&fitting).is_ok());
}

fn load_error(bytes: &[u8]) -> PersistenceError {
    match persistence::load(bytes) {
        Ok(_) => panic!("invalid document loaded"),
        Err(error) => error,
    }
}

/// Opens a checkpoint whose history is damaged: the current revision stays editable,
/// Undo is empty and the returned reason says why the history was dropped.
fn discarded_history(bytes: &[u8]) -> String {
    let Ok(LoadOutcome::Editable {
        document, audit, ..
    }) = ketchup_model::testing::load_with_history(bytes)
    else {
        panic!("a damaged history must not stop the current revision from opening")
    };
    assert_eq!(document.revision_count(), 1);
    audit
        .history_discarded()
        .expect("discard reason")
        .to_owned()
}

pub(crate) fn container_entry_offsets(bytes: &[u8], target: &str) -> (usize, usize, usize) {
    let mut cursor = 16;
    let count = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    for _ in 0..count {
        let path_len = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        let path = std::str::from_utf8(&bytes[cursor..cursor + path_len]).unwrap();
        cursor += path_len + 1;
        let content_len =
            u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap()) as usize;
        cursor += 8;
        let checksum_offset = cursor;
        cursor += 32;
        let content_offset = cursor;
        cursor += content_len;
        if path == target {
            return (checksum_offset, content_offset, content_len);
        }
    }
    panic!("container entry {target:?} was not found");
}

pub(crate) fn rehash_container_entry(
    bytes: &mut [u8],
    checksum_offset: usize,
    content_offset: usize,
    len: usize,
) {
    let checksum = ketchup_model::graph::sha256_bytes(&bytes[content_offset..content_offset + len]);
    bytes[checksum_offset..checksum_offset + 32].copy_from_slice(&checksum);
}

fn history_snapshot_offsets(bytes: &[u8]) -> Vec<(usize, usize)> {
    let (_, history_offset, history_len) = container_entry_offsets(bytes, "history.bin");
    let history_end = history_offset + history_len;
    let mut search_offset = history_offset;
    let mut snapshots = Vec::new();
    while let Some(relative_offset) = bytes[search_offset..history_end]
        .windows(b"KETCHUPDOC".len())
        .position(|window| window == b"KETCHUPDOC")
    {
        let snapshot_offset = search_offset + relative_offset;
        let length_offset = snapshot_offset.checked_sub(40).unwrap();
        let snapshot_len = usize::try_from(u64::from_le_bytes(
            bytes[length_offset..length_offset + 8].try_into().unwrap(),
        ))
        .unwrap();
        assert!(snapshot_offset + snapshot_len <= history_end);
        snapshots.push((snapshot_offset, snapshot_len));
        search_offset = snapshot_offset + snapshot_len;
    }
    snapshots
}

/// Saves `store` with its whole history through the save pipeline.
fn publish(
    path: &std::path::Path,
    store: &DocumentStore,
    data: &persistence::ContainerData,
    mode: persistence::Publish,
) -> Result<persistence::FileIdentity, persistence::FilePersistenceError> {
    let prepared = persistence::prepare_document(store, data).unwrap();
    persistence::publish_document(path, &prepared, mode).map(|()| prepared.identity())
}

/// Writes the work-recovery checkpoint of `store` as an edit does.
fn checkpoint(
    path: &std::path::Path,
    store: &DocumentStore,
    data: &persistence::ContainerData,
    base: persistence::FileIdentity,
    expected: Option<persistence::FileIdentity>,
) -> Result<persistence::FileIdentity, persistence::FilePersistenceError> {
    persistence::publish_work_recovery(path, store, data, base, expected)
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
                    ketchup_model::document::DerivedIdentity::new(NodeId(2), path).unwrap(),
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

/// Saved before the role-string `Shell` feature was removed; its feature 20 names faces by role.
fn role_string_shell_document_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/persistence/invalid/role-string-shell-schema97.bin"
    ))
    .unwrap()
}

#[test]
fn default_load_rejects_legacy_named_feature_authority_with_typed_migration_error() {
    assert_eq!(
        load_error(&role_string_shell_document_bytes()),
        PersistenceError::LegacyFeatureRequiresMigration {
            feature_id: FeatureId(20),
            kind: LegacyFeatureKind::RoleStringShell,
        }
    );
}

#[test]
fn migration_required_primary_is_not_replaced_by_a_recovery_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.ketchup");
    std::fs::write(&path, role_string_shell_document_bytes()).unwrap();
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
                kind: LegacyFeatureKind::RoleStringShell,
            }
        ))
    ));
}

#[test]
fn a_named_classification_dimension_is_found_once_or_refused_as_missing_or_ambiguous() {
    let dimension = |id: u64, name: &str| CanonicalCommand::UpsertClassificationDimension {
        id: ClassificationDimensionId(id),
        name: name.to_owned(),
        categories: vec![(ClassificationCategoryId(id), format!("category {id}"))],
    };
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Part".to_owned(),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Part occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
            dimension(1, "side"),
            dimension(2, "system"),
            dimension(3, "system"),
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
        ]))
        .unwrap();
    let snapshot = store.current();

    let side = snapshot.required_classification_dimension("side").unwrap();
    assert_eq!(side.id(), ClassificationDimensionId(1));
    assert_eq!(
        snapshot
            .occurrence_category_names(side)
            .collect::<Result<Vec<_>, _>>(),
        Ok(vec![(
            OccurrenceId(1),
            ClassificationCategoryId(1),
            "category 1"
        )])
    );
    assert!(
        snapshot
            .classification_dimension_named("absent")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        snapshot.required_classification_dimension("absent").err(),
        Some(ClassificationError::DimensionMissing {
            name: "absent".to_owned()
        })
    );
    assert_eq!(
        snapshot.classification_dimension_named("system").err(),
        Some(ClassificationError::DimensionAmbiguous {
            name: "system".to_owned()
        })
    );
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Panel".to_owned(),
                kind: FeatureKind::extrusion(FeatureId(1), Dimension::new("20", 20.0).unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Wall panel occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
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
                tags: Default::default(),
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
    let schema_46 = include_bytes!("fixtures/persistence/legacy/graph-schema46.bin");
    let loaded = persistence::load(schema_46).unwrap();
    let snapshot = with_document_id(&graph_document().current(), loaded.snapshot().document_id());
    let expected_digest = snapshot.canonical_digest();
    let current = persistence::save(&snapshot);

    assert_eq!(loaded.source_schema(), 46);
    assert_eq!(loaded.disposition(), LoadDisposition::EditableLossless);
    assert!(loaded.migration_losses().is_empty());
    assert_eq!(loaded.snapshot().canonical_digest(), expected_digest);
    assert_eq!(persistence::save(&loaded.snapshot()), current);
}

#[test]
fn invalid_utf8_names_the_byte_where_the_string_breaks() {
    let mut bytes = b"KETCHUPCTR".to_vec();
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&[b'a', 0xff, b'b']);
    let error = load_error(&bytes);
    assert!(matches!(error, PersistenceError::InvalidUtf8(cause) if cause.valid_up_to() == 1));
    assert_eq!(
        error.to_string(),
        "document string is not UTF-8: invalid utf-8 sequence of 1 bytes from index 1"
    );
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn schema_three_checks_checksum_before_payload_decode_and_rejects_envelopes() {
    // A document written by the retired field-by-field codec (schema 97).
    let legacy = include_bytes!("fixtures/persistence/legacy/graph-schema97.bin").to_vec();
    let loaded = persistence::load(&legacy).unwrap().snapshot();
    assert_eq!(
        loaded.canonical_digest(),
        with_document_id(&graph_document().current(), loaded.document_id()).canonical_digest()
    );
    let mut corrupt = legacy.clone();
    let payload = 16 + u32::from_le_bytes(corrupt[12..16].try_into().unwrap()) as usize;
    corrupt[payload] ^= 0xff;
    assert_eq!(load_error(&corrupt), PersistenceError::ChecksumMismatch);
    assert_eq!(
        load_error(&legacy[..legacy.len() - 1]),
        PersistenceError::Legacy(ketchup_model::persistence::LegacyError::InvalidEnvelopeLength)
    );

    // The current format seals its payload with one checksum after the format number.
    let bytes = persistence::save(&graph_document().current());
    let checksum = b"KETCHUPDOC".len() + 2;
    let mut corrupt = bytes.clone();
    corrupt[checksum + 32] ^= 0xff;
    assert_eq!(load_error(&corrupt), PersistenceError::ChecksumMismatch);
    assert_eq!(
        load_error(&bytes[..bytes.len() - 1]),
        PersistenceError::ChecksumMismatch
    );
    assert_eq!(
        load_error(&bytes[..checksum + 31]),
        PersistenceError::Truncated
    );
    let newer = persistence::CURRENT_SCHEMA + 1;
    let mut unsupported = bytes;
    unsupported[10..12].copy_from_slice(&newer.to_le_bytes());
    assert_eq!(
        load_error(&unsupported),
        PersistenceError::UnsupportedSchema(newer)
    );
}

/// PS-4: a newer Kečup writing a field this version does not know under the same format
/// number must not have it dropped on open and lost at the next Save.
#[test]
fn a_field_this_version_does_not_know_is_refused_not_dropped() {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Shelf".into(),
            },
        ]))
        .unwrap();
    let bytes = persistence::save(&store.current());
    let text = |text: &str| ciborium::Value::Text(text.to_owned());
    let newer = ketchup_model::testing::rewrite_saved_snapshot(&bytes, |saved| {
        let product = ketchup_model::testing::cbor_entry(saved, "product");
        let definition = ketchup_model::testing::cbor_entry(
            ketchup_model::testing::cbor_entry(product, "definitions"),
            1,
        );
        definition
            .as_map_mut()
            .unwrap()
            .push((text("grain"), ciborium::Value::Integer(90.into())));
        product
            .as_map_mut()
            .unwrap()
            .push((text("finish"), text("oiled")));
    });
    assert_eq!(
        load_error(&newer),
        PersistenceError::UnknownFields(vec![
            "product.definitions.1.grain".to_owned(),
            "product.finish".to_owned(),
        ])
    );
    let LoadOutcome::Editable { document, .. } = persistence::load(&bytes).unwrap() else {
        panic!("the file as written is editable")
    };
    assert_eq!(persistence::save(&document.current()), bytes);
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
    assert_eq!(current.source_schema(), persistence::OLDEST_WRITTEN_SCHEMA);
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
            MigrationBlocker::UnknownExtension
        ))
    ));
}

/// Review 2026-10-17 M-8: a folder where the work-recovery checkpoint belongs came back
/// as an `io::Error` holding only a sentence; the error is typed and names the path.
#[test]
fn a_work_recovery_folder_is_a_typed_error_naming_its_path() {
    let directory = tempfile::tempdir().unwrap();
    let document = directory.path().join("part.ketchup");
    let recovery = persistence::work_recovery_path(&document);
    std::fs::create_dir(&recovery).unwrap();
    assert!(matches!(
        persistence::check_work_recovery_identity(&document, None),
        Err(persistence::FilePersistenceError::NotARegularFile(path)) if path == recovery
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
    assert_eq!(*loaded.container_data().blobs()[&blob_hash], blob);
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
fn revision_history_round_trips_with_cursor_and_monotonic_branching() {
    let mut store = graph_document();
    for value in ["5", "8"] {
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(1),
                    dimension: Dimension::new(value, value.parse().unwrap()).unwrap(),
                },
            ]))
            .unwrap();
    }
    store.undo().unwrap();
    let expected_revision_ids = store
        .revision_history()
        .map(ketchup_model::document::Revision::id)
        .collect::<Vec<_>>();
    let expected_digests = store
        .revision_history()
        .map(|revision| revision.snapshot().canonical_digest())
        .collect::<Vec<_>>();
    let expected_cursor = store.history_cursor();
    let expected_next_revision_id = store.next_revision_id();

    let first =
        persistence::save_document_store(&store, &persistence::ContainerData::default()).unwrap();
    let second =
        persistence::save_document_store(&store, &persistence::ContainerData::default()).unwrap();
    assert_eq!(second, first);

    let loaded = ketchup_model::testing::load_with_history(&first).unwrap();
    let container_data = loaded.container_data().clone();
    let mut reopened = loaded.into_editable().ok().unwrap();
    assert_eq!(reopened.history_cursor(), expected_cursor);
    assert_eq!(reopened.next_revision_id(), expected_next_revision_id);
    assert_eq!(reopened.visible_undo_steps(), expected_cursor);
    assert_eq!(reopened.visible_redo_steps(), 1);
    assert_eq!(
        reopened
            .revision_history()
            .map(ketchup_model::document::Revision::id)
            .collect::<Vec<_>>(),
        expected_revision_ids
    );
    assert_eq!(
        reopened
            .revision_history()
            .map(|revision| revision.snapshot().canonical_digest())
            .collect::<Vec<_>>(),
        expected_digests
    );
    assert_eq!(
        persistence::save_document_store(&reopened, &container_data).unwrap(),
        first
    );

    let before = reopened.current().canonical_digest();
    reopened.undo().unwrap();
    assert_ne!(reopened.current().canonical_digest(), before);
    reopened.redo().unwrap();
    assert_eq!(reopened.current().canonical_digest(), before);
    reopened
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("13", 13.0).unwrap(),
            },
        ]))
        .unwrap();
    assert_eq!(reopened.current().revision_id(), expected_next_revision_id);
    assert_eq!(reopened.visible_redo_steps(), 0);
}

/// Review 2026-10-19 P3: an import adds its source to a copy of the container
/// data the previous revision still holds, and adding copied every blob the two
/// shared: three 30 MB STEP imports copied 30 and then 60 MB, and the history
/// kept each copy. The new copy shares the blobs it already had.
#[test]
fn adding_a_blob_to_a_shared_copy_shares_the_blobs_it_had() {
    const BLOB: usize = 1024 * 1024;
    let mut previous = persistence::ContainerData::default();
    let first = previous.insert_import_blob(vec![7; BLOB]).unwrap();
    let mut next = previous.clone();
    let second = next.insert_import_blob(vec![8; BLOB]).unwrap();
    assert_eq!(next.blobs()[&second].len(), BLOB);
    assert!(!previous.blobs().contains_key(&second));
    assert!(
        std::ptr::eq(
            previous.blobs()[&first].as_ptr(),
            next.blobs()[&first].as_ptr()
        ),
        "adding a blob copied the {BLOB}-byte blob the copies shared"
    );
}

/// Review 2026-10-17 M-5: opening a history copies the container data once per
/// revision, and each copy copied every blob: ~12 GB for a 30 MB STEP source and
/// 400 revisions. A copy now shares the blobs it was made from.
#[test]
fn opening_a_history_shares_its_blobs_instead_of_copying_them_per_revision() {
    const BLOB: usize = 1024 * 1024;
    const REVISIONS: u32 = 24;
    let mut store = graph_document();
    for value in 1..=REVISIONS {
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(1),
                    dimension: Dimension::new(value.to_string(), f64::from(value)).unwrap(),
                },
            ]))
            .unwrap();
    }
    let mut container_data = persistence::ContainerData::default();
    let hash = container_data.insert_blob(vec![7; BLOB]).unwrap();
    let bytes = persistence::save_document_store(&store, &container_data).unwrap();

    let loaded = ketchup_model::testing::load_with_history(&bytes).unwrap();
    let opened = loaded.container_data().clone();
    assert_eq!(opened.blobs()[&hash].len(), BLOB);
    let copy = opened.clone();
    assert!(
        std::ptr::eq(opened.blobs()[&hash].as_ptr(), copy.blobs()[&hash].as_ptr()),
        "a copy of the container data copied its {BLOB}-byte blob"
    );
    let reopened = loaded.into_editable().ok().unwrap();
    assert!(reopened.revision_history().count() > REVISIONS as usize);
    assert_eq!(
        persistence::save_document_store(&reopened, &copy).unwrap(),
        bytes
    );
}

#[test]
fn multi_version_history_golden_migrates_losslessly_and_drops_corrupt_history() {
    let mut store = graph_document();
    for value in ["5", "8"] {
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(1),
                    dimension: Dimension::new(value, value.parse().unwrap()).unwrap(),
                },
            ]))
            .unwrap();
    }
    store.undo().unwrap();
    let expected_revision_ids = store
        .revision_history()
        .map(ketchup_model::document::Revision::id)
        .collect::<Vec<_>>();
    let expected_cursor = store.history_cursor();
    let expected_next_revision_id = store.next_revision_id();

    // Written by the retired codec: the same history with snapshots of schemas 46, 68
    // and 97 mixed in one file.
    let golden =
        include_bytes!("fixtures/persistence/legacy/history-schemas-46-68-97-46.bin").to_vec();
    let schemas = [46, 68, 97, 46];
    let snapshot_offsets = history_snapshot_offsets(&golden);
    assert_eq!(snapshot_offsets.len(), expected_revision_ids.len());
    for ((snapshot_offset, _), schema) in snapshot_offsets.iter().zip(schemas) {
        assert_eq!(
            u16::from_le_bytes(
                golden[snapshot_offset + 10..snapshot_offset + 12]
                    .try_into()
                    .unwrap()
            ),
            schema
        );
    }

    let loaded = ketchup_model::testing::load_with_history(&golden).unwrap();
    assert_eq!(loaded.disposition(), LoadDisposition::EditableLossless);
    let document_id = loaded.snapshot().document_id();
    let expected_digests = store
        .revision_history()
        .map(|revision| with_document_id(revision.snapshot(), document_id).canonical_digest())
        .collect::<Vec<_>>();
    let container_data = loaded.container_data().clone();
    let reopened = loaded.into_editable().ok().unwrap();
    assert_eq!(reopened.history_cursor(), expected_cursor);
    assert_eq!(reopened.next_revision_id(), expected_next_revision_id);
    assert_eq!(
        reopened
            .revision_history()
            .map(ketchup_model::document::Revision::id)
            .collect::<Vec<_>>(),
        expected_revision_ids
    );
    assert_eq!(
        reopened
            .revision_history()
            .map(|revision| revision.snapshot().canonical_digest())
            .collect::<Vec<_>>(),
        expected_digests
    );

    // A format-4 history holds the earlier revisions as chunks; each one's first
    // chunk starts with its snapshot envelope, now in the current format.
    let upgraded = persistence::save_document_store(&reopened, &container_data).unwrap();
    let (_, history_offset, history_len) = container_entry_offsets(&upgraded, "history.bin");
    let history = &upgraded[history_offset..history_offset + history_len];
    let envelopes = (0..history.len() - 12)
        .filter(|&offset| history[offset..].starts_with(b"KETCHUPDOC"))
        .collect::<Vec<_>>();
    assert_eq!(envelopes.len(), expected_revision_ids.len() - 1);
    for offset in envelopes {
        assert_eq!(
            u16::from_le_bytes(history[offset + 10..offset + 12].try_into().unwrap()),
            persistence::OLDEST_WRITTEN_SCHEMA
        );
    }
    let round_trip = ketchup_model::testing::load_with_history(&upgraded)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(round_trip.history_cursor(), expected_cursor);
    assert_eq!(round_trip.next_revision_id(), expected_next_revision_id);
    assert_eq!(
        round_trip
            .revision_history()
            .map(|revision| revision.snapshot().canonical_digest())
            .collect::<Vec<_>>(),
        expected_digests
    );
    assert_eq!(
        persistence::save_document_store(&round_trip, &container_data).unwrap(),
        upgraded
    );

    let (snapshot_offset, snapshot_len) = snapshot_offsets[0];
    let mut history_checksum_corrupt = golden.clone();
    history_checksum_corrupt[snapshot_offset + snapshot_len - 1] ^= 0xff;
    let (history_checksum_offset, history_offset, history_len) =
        container_entry_offsets(&history_checksum_corrupt, "history.bin");
    rehash_container_entry(
        &mut history_checksum_corrupt,
        history_checksum_offset,
        history_offset,
        history_len,
    );
    assert_eq!(
        discarded_history(&history_checksum_corrupt),
        PersistenceError::HistoryChecksumMismatch.to_string()
    );

    let mut envelope_checksum_corrupt = golden;
    envelope_checksum_corrupt[snapshot_offset + snapshot_len - 1] ^= 0xff;
    let snapshot_checksum = ketchup_model::graph::sha256_bytes(
        &envelope_checksum_corrupt[snapshot_offset..snapshot_offset + snapshot_len],
    );
    envelope_checksum_corrupt[snapshot_offset - 32..snapshot_offset]
        .copy_from_slice(&snapshot_checksum);
    let (history_checksum_offset, history_offset, history_len) =
        container_entry_offsets(&envelope_checksum_corrupt, "history.bin");
    rehash_container_entry(
        &mut envelope_checksum_corrupt,
        history_checksum_offset,
        history_offset,
        history_len,
    );
    assert_eq!(
        discarded_history(&envelope_checksum_corrupt),
        PersistenceError::ChecksumMismatch.to_string()
    );
}

/// PS-1: a small history whose revisions within the Undo window each name the whole
/// `document.bin` many times over (≈ 2.4 GiB assembled) is refused before anything is
/// assembled; the document opens without Undo at once. Revisions outside the window
/// count against no budget (PS-12), so the gigabytes are named inside it.
#[test]
fn a_history_naming_gigabytes_through_shared_chunks_is_refused_before_it_is_assembled() {
    let commands = (1..=600_u64)
        .map(|id| CanonicalCommand::CreateEvaluatorNode {
            id: NodeId(id),
            name: format!("parameter-{id:03}-{}", unique_text(id, 1_024)),
            dimension: Dimension::new(id.to_string(), id as f64).unwrap(),
            dependencies: vec![],
        })
        .collect();
    let mut store = DocumentStore::new();
    store.apply_batch(&CommandBatch::new(commands)).unwrap();
    let saved = persistence::save_document_store_current_snapshot(
        &store,
        &persistence::ContainerData::default(),
    )
    .unwrap();
    let (_, offset, len) = container_entry_offsets(&saved, "document.bin");
    let document = &saved[offset..offset + len];
    let revisions = u32::try_from(ketchup_model::tolerance::limits::UNDO_REVISIONS + 1).unwrap();
    // Each revision names the one chunk about 80 MiB worth of times.
    let repeats = u32::try_from(80 * 1024 * 1024 / len).unwrap();
    let length = u64::from(repeats) * len as u64;
    assert!(u64::from(revisions - 1) * length > 2 * 1024 * 1024 * 1024);

    let mut history = b"KETCHUPHST".to_vec();
    history.extend_from_slice(&4_u16.to_le_bytes());
    history.extend_from_slice(&revisions.to_le_bytes());
    history.extend_from_slice(&(revisions - 1).to_le_bytes());
    history.extend_from_slice(&(u64::from(revisions) + 1).to_le_bytes());
    // One chunk: all of document.bin.
    history.extend_from_slice(&1_u32.to_le_bytes());
    history.push(1);
    history.extend_from_slice(&0_u32.to_le_bytes());
    history.extend_from_slice(&(len as u32).to_le_bytes());
    let checksum = ketchup_model::graph::sha256_bytes(document);
    for index in 0..revisions {
        history.extend_from_slice(&(u64::from(index) + 1).to_le_bytes());
        history.extend_from_slice(&0_u32.to_le_bytes()); // no batch digest
        if index == 0 {
            history.push(0); // initial
        } else {
            history.extend_from_slice(&[1, 0]); // the manual client
        }
        history.extend_from_slice(&[0, 0]); // no checkpoint, no program
        if index + 1 == revisions {
            history.push(0); // document.bin
        } else {
            history.push(1);
            history.extend_from_slice(&length.to_le_bytes());
            history.extend_from_slice(&checksum);
            history.extend_from_slice(&repeats.to_le_bytes());
            history.extend((0..repeats).flat_map(|_| 0_u32.to_le_bytes()));
        }
    }
    assert!(history.len() < 512 * 1024);
    let bytes = ketchup_model::testing::save_with_history_entry(&store, history);

    // Refused on the sizes alone: assembling 20 GB first would not get here.
    let reason = discarded_history(&bytes);
    assert!(
        reason.starts_with("the revisions of history.bin would be at least 20")
            && reason.ends_with("the limit is 2048.0 MB"),
        "{reason}"
    );
}

/// Programs as the window writes them: bound with a model change, the same program
/// again with the next model change, then a source-only edit.
fn programmed_history_document() -> DocumentStore {
    use ketchup_model::document::RuleProgramSource;
    let program = |source: &str| RuleProgramSource {
        file_name: "shelf.star".into(),
        source: source.into(),
        overrides: std::collections::BTreeMap::from([("width".into(), 800.0)]),
        variants: Default::default(),
    };
    let mut document = graph_document();
    document
        .bind_rule_program(program("w = param('width', 600)\n"))
        .unwrap();
    for (id, source) in [
        (900, "w = param('width', 700)\n"),
        (901, "w = param('width', 700)\n"),
    ] {
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateEvaluatorNode {
                    id: NodeId(id),
                    name: format!("depth-{id}"),
                    dimension: Dimension::new("400", 400.0).unwrap(),
                    dependencies: vec![],
                },
            ]))
            .unwrap();
        document.bind_rule_program(program(source)).unwrap();
    }
    document
        .replace_rule_program_source(program("w = param('width', 900)\n"))
        .unwrap();
    document
}

fn history_schema(bytes: &[u8]) -> u16 {
    let (_, offset, _) = container_entry_offsets(bytes, "history.bin");
    u16::from_le_bytes(bytes[offset + 10..offset + 12].try_into().unwrap())
}

fn revision_programs(
    document: &DocumentStore,
) -> Vec<Option<ketchup_model::document::RuleProgramSource>> {
    document
        .revision_history()
        .map(|revision| revision.rule_program().cloned())
        .collect()
}

fn reopened(bytes: &[u8]) -> DocumentStore {
    ketchup_model::testing::load_with_history(bytes)
        .unwrap()
        .into_editable_with_container()
        .unwrap_or_else(|_| panic!("expected editable roundtrip"))
        .0
}

/// PS-2: a history format 4 wrote (each program whole, or named as the one
/// before) opens with every revision's program and saves again as format 5.
#[test]
fn a_format_4_history_opens_with_the_program_of_every_revision() {
    let golden = include_bytes!("fixtures/persistence/legacy/history-format4-programs.bin");
    assert_eq!(history_schema(golden), 4);
    let (_, offset, len) = container_entry_offsets(golden, "history.bin");
    assert_eq!(
        golden[offset..offset + len]
            .windows(br#"{"file_name""#.len())
            .filter(|window| *window == br#"{"file_name""#)
            .count(),
        3,
        "the repeated program is named as the one before"
    );
    let expected = revision_programs(&programmed_history_document());
    assert_eq!(expected.iter().flatten().count(), 4);

    let document = reopened(golden);
    assert_eq!(revision_programs(&document), expected);
    let resaved =
        persistence::save_document_store(&document, &persistence::ContainerData::default())
            .unwrap();
    assert_eq!(history_schema(&resaved), 5);
    assert_eq!(revision_programs(&reopened(&resaved)), expected);
}

/// Review 2026-10-15 status PS-1: a file written before `program.json` keeps its
/// program only in the Undo history. A damaged older step costs the steps, not
/// the program of the revision the file opens at, and not the file.
#[test]
fn a_file_without_program_json_keeps_the_current_program_when_an_older_step_is_damaged() {
    let golden = include_bytes!("fixtures/persistence/legacy/history-format4-programs.bin");
    assert!(
        !golden
            .windows(b"program.json".len())
            .any(|window| window == b"program.json"),
        "the file predates program.json"
    );
    let current = reopened(golden)
        .current_rule_program()
        .cloned()
        .expect("the file opens at a programmed revision");
    assert_eq!(&*current.source, "w = param('width', 900)\n");

    let (checksum_offset, offset, len) = container_entry_offsets(golden, "history.bin");
    let first_program = offset
        + golden[offset..offset + len]
            .windows(br#""source":"w = param('width', 600)"#.len())
            .position(|window| window == br#""source":"w = param('width', 600)"#)
            .expect("the first step's program is stored whole");
    for (name, damage) in [
        // Not a program at all.
        ("malformed", br#""sourcX""#.as_slice()),
        // A program whose source is missing.
        ("incomplete", br#""unused""#.as_slice()),
    ] {
        let mut damaged = golden.to_vec();
        damaged[first_program..first_program + damage.len()].copy_from_slice(damage);
        rehash_container_entry(&mut damaged, checksum_offset, offset, len);
        let Ok(LoadOutcome::Editable {
            document, audit, ..
        }) = persistence::load(&damaged)
        else {
            panic!("{name}: a damaged older step must not refuse the file");
        };
        assert_eq!(document.revision_count(), 1, "{name}");
        assert_eq!(audit.history_discarded(), None, "{name}");
        assert_eq!(document.current_rule_program(), Some(&current), "{name}");
    }

    // The program of the revision the file opens at is refused, never dropped.
    let current_program = offset
        + golden[offset..offset + len]
            .windows(br#""source":"w = param('width', 900)"#.len())
            .position(|window| window == br#""source":"w = param('width', 900)"#)
            .expect("the current program is stored whole");
    let mut damaged = golden.to_vec();
    damaged[current_program..current_program + 8].copy_from_slice(br#""sourcX""#);
    rehash_container_entry(&mut damaged, checksum_offset, offset, len);
    assert!(persistence::load(&damaged).is_err());
}

/// PS-2: a program near the window's 690 KB house edited 60 times. Stored whole
/// in every revision it passed `history.bin`'s 32 MB after ≈ 47 edits, so Save
/// offered to drop Undo; as chunks each edit stores only those around it.
#[test]
fn a_program_edit_stores_only_the_chunks_around_the_change() {
    let line = |index: u64, value: u64| {
        format!(
            "p{index:05} = param('p{index:05}', {value})  # {}\n",
            unique_text(index, 40)
        )
    };
    let mut lines = (0..10_000_u64)
        .map(|index| line(index, index))
        .collect::<Vec<_>>();
    let program = |lines: &[String]| ketchup_model::document::RuleProgramSource {
        file_name: "house.star".into(),
        source: lines.concat(),
        overrides: Default::default(),
        variants: Default::default(),
    };
    let mut document = graph_document();
    document.bind_rule_program(program(&lines)).unwrap();
    let program_len = serde_json::to_vec(&program(&lines)).unwrap().len();
    assert!(program_len > 690 * 1024, "{program_len}");
    for edit in 1..=60_u64 {
        let index = edit * 160;
        lines[index as usize] = line(index, index + 1);
        document
            .replace_rule_program_source(program(&lines))
            .unwrap();
    }

    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    let (_, _, history_len) = container_entry_offsets(&bytes, "history.bin");
    assert!(history_len < 3 * program_len, "{history_len} bytes");
    let reopened = reopened(&bytes);
    assert_eq!(reopened.revision_count(), document.revision_count());
    assert_eq!(revision_programs(&reopened), revision_programs(&document));
}

/// A model of about 34 MB with two Undo steps: creating it and one edit, which
/// makes the 34 MB revision an earlier one.
fn large_model_after_an_edit() -> DocumentStore {
    let commands = (1..=33_u64)
        .map(|id| CanonicalCommand::CreateEvaluatorNode {
            id: NodeId(id),
            name: format!("{id:04}{}", "x".repeat(1024 * 1024 - 8)),
            dimension: Dimension::new(id.to_string(), id as f64).unwrap(),
            dependencies: vec![],
        })
        .collect();
    let mut store = DocumentStore::new();
    store.apply_batch(&CommandBatch::new(commands)).unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("10000", 10_000.0).unwrap(),
            },
        ]))
        .unwrap();
    store
}

fn unique_text(seed: u64, len: usize) -> String {
    let mut state = seed.wrapping_add(1);
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            char::from(b"0123456789abcdef"[(state >> 60) as usize])
        })
        .collect()
}

#[test]
fn thirty_one_edits_keep_thirty_undo_steps_and_a_saved_file_reopens_without_them() {
    let mut store = graph_document();
    assert_eq!(limits::UNDO_REVISIONS, 30);
    for index in 0..31_u64 {
        let value = 10_000 + index;
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(1),
                    dimension: Dimension::new(value.to_string(), value as f64).unwrap(),
                },
            ]))
            .unwrap();
    }
    assert_eq!(store.revision_count(), 31);
    assert_eq!(store.visible_undo_steps(), 30);
    let saved = persistence::prepare_document(&store, &persistence::ContainerData::default())
        .unwrap()
        .bytes()
        .to_vec();
    let reopened = persistence::load(&saved)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(
        reopened.current().canonical_digest(),
        store.current().canonical_digest()
    );
    assert_eq!(reopened.visible_undo_steps(), 0);
    assert_eq!(reopened.visible_redo_steps(), 0);
    // Saving leaves the document's own 30 steps in memory.
    assert_eq!(store.visible_undo_steps(), 30);
    for _ in 0..30 {
        store.undo().unwrap();
    }
    assert!(store.undo().is_none());
}

/// Review 2026-10-24 MD-21: naming a checkpoint replaces the current revision with
/// one of the same id and snapshot, and the work-recovery step compared revisions by
/// those alone, so it recorded no change and after a crash the name was gone. The
/// named revision is written again, and the recovered history keeps its name.
#[test]
fn a_checkpoint_named_after_edits_survives_a_crash() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("named.ketchup");
    let container_data = persistence::ContainerData::default();
    let mut store = graph_document();
    let base = publish(
        &path,
        &store,
        &container_data,
        persistence::Publish::Replace,
    )
    .unwrap();
    let names = |store: &DocumentStore| {
        store
            .revision_history()
            .map(|revision| revision.checkpoint().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    let mut recovery = None;
    for value in [3.0, 4.0] {
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(1),
                    dimension: Dimension::new(value.to_string(), value).unwrap(),
                },
            ]))
            .unwrap();
        recovery = Some(checkpoint(&path, &store, &container_data, base, recovery).unwrap());
    }
    let current = store.current();
    store
        .create_checkpoint(
            current.revision_id(),
            &current.canonical_digest(),
            "before the roof",
        )
        .unwrap();
    checkpoint(&path, &store, &container_data, base, recovery).unwrap();

    // A crash: nothing is saved, the next open finds the checkpoint.
    let opened = persistence::load_file_with_source(&path).unwrap();
    assert_eq!(opened.source_path(), persistence::work_recovery_path(&path));
    let recovered = opened.into_parts().0.into_editable().ok().unwrap();
    assert_eq!(names(&recovered), names(&store));
    assert_eq!(
        names(&recovered).last(),
        Some(&Some("before the roof".to_owned()))
    );
}

#[test]
fn work_recovery_of_a_model_over_32_mb_restores_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.ketchup");
    let container_data = persistence::ContainerData::default();
    publish(
        &path,
        &graph_document(),
        &container_data,
        persistence::Publish::Replace,
    )
    .unwrap();
    let base_identity = persistence::read_native_document_identity(&path).unwrap();
    let store = large_model_after_an_edit();

    checkpoint(&path, &store, &container_data, base_identity, None).unwrap();
    let recovered = persistence::load_file_with_source(&path).unwrap();
    assert_eq!(
        recovered.source_path(),
        persistence::work_recovery_path(&path)
    );
    assert_eq!(
        recovered.outcome().snapshot().canonical_digest(),
        store.current().canonical_digest()
    );
}

#[test]
fn explicit_current_snapshot_save_keeps_the_current_revision_only() {
    let store = large_model_after_an_edit();
    let current = store.current();
    let expected_next_revision_id = store.next_revision_id();
    let original_revision_count = store.revision_count();
    // The empty document, the large one and the edit.
    assert_eq!(original_revision_count, 3);

    let first = persistence::save_document_store_current_snapshot(
        &store,
        &persistence::ContainerData::default(),
    )
    .unwrap();
    let second = persistence::save_document_store_current_snapshot(
        &store,
        &persistence::ContainerData::default(),
    )
    .unwrap();
    assert_eq!(second, first);
    assert_eq!(store.revision_count(), original_revision_count);

    let mut reopened = persistence::load(&first)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened.current().revision_id(), current.revision_id());
    assert_eq!(
        reopened.current().canonical_digest(),
        current.canonical_digest()
    );
    assert_eq!(reopened.next_revision_id(), expected_next_revision_id);
    assert_eq!(reopened.revision_count(), 1);
    assert_eq!(reopened.visible_undo_steps(), 0);
    assert_eq!(reopened.visible_redo_steps(), 0);

    reopened
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("20000", 20_000.0).unwrap(),
            },
        ]))
        .unwrap();
    assert_eq!(reopened.current().revision_id(), expected_next_revision_id);

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("compacted.ketchup");
    let prepared =
        persistence::prepare_document(&store, &persistence::ContainerData::default()).unwrap();
    persistence::publish_document(&path, &prepared, persistence::Publish::Replace).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), first);
}

#[test]
fn corrupt_or_incompatible_revision_history_opens_only_the_current_revision() {
    let store = graph_document();
    let encoded =
        persistence::save_document_store(&store, &persistence::ContainerData::default()).unwrap();
    let (checksum_offset, history_offset, history_len) =
        container_entry_offsets(&encoded, "history.bin");

    // An earlier revision's first chunk starts with its snapshot envelope.
    let mut corrupt = encoded.clone();
    let earlier_snapshot = history_offset
        + corrupt[history_offset..history_offset + history_len]
            .windows(b"KETCHUPDOC".len())
            .position(|window| window == b"KETCHUPDOC")
            .unwrap();
    corrupt[earlier_snapshot + 20] ^= 0xff;
    rehash_container_entry(&mut corrupt, checksum_offset, history_offset, history_len);
    assert_eq!(
        discarded_history(&corrupt),
        PersistenceError::HistoryChecksumMismatch.to_string()
    );

    let mut incompatible = encoded.clone();
    incompatible[history_offset + 10..history_offset + 12].copy_from_slice(&0_u16.to_le_bytes());
    rehash_container_entry(
        &mut incompatible,
        checksum_offset,
        history_offset,
        history_len,
    );
    assert_eq!(
        discarded_history(&incompatible),
        PersistenceError::UnsupportedHistorySchema(0).to_string()
    );

    // Review 2026-10-15 status PS-3: a history a newer Kečup wrote is not
    // damaged. Opening without it would let the next Save overwrite it with
    // this writer's single revision, so the file is refused like a newer format.
    let mut newer = encoded;
    newer[history_offset + 10..history_offset + 12].copy_from_slice(&6_u16.to_le_bytes());
    rehash_container_entry(&mut newer, checksum_offset, history_offset, history_len);
    assert!(matches!(
        ketchup_model::testing::load_with_history(&newer),
        Err(PersistenceError::UnsupportedHistorySchema(6))
    ));
}

/// The program that owns the document is part of the document, not of its Undo
/// steps: a history that no longer opens costs the steps, and the program
/// survives the open and the next Save.
#[test]
fn a_discarded_history_keeps_the_current_program_through_open_and_save() {
    let store = programmed_history_document();
    let program = store.current_rule_program().cloned().expect("bound");
    let encoded =
        persistence::save_document_store(&store, &persistence::ContainerData::default()).unwrap();
    let (checksum_offset, history_offset, history_len) =
        container_entry_offsets(&encoded, "history.bin");
    let mut incompatible = encoded;
    incompatible[history_offset + 10..history_offset + 12].copy_from_slice(&0_u16.to_le_bytes());
    rehash_container_entry(
        &mut incompatible,
        checksum_offset,
        history_offset,
        history_len,
    );
    assert_eq!(
        discarded_history(&incompatible),
        PersistenceError::UnsupportedHistorySchema(0).to_string()
    );
    let (reopened, data) = persistence::load(&incompatible)
        .unwrap()
        .into_editable_with_container()
        .unwrap_or_else(|_| panic!("expected editable"));
    assert_eq!(reopened.current_rule_program(), Some(&program));
    for resaved in [
        persistence::save_document_store(&reopened, &data).unwrap(),
        persistence::save_document_store_current_snapshot(&reopened, &data).unwrap(),
    ] {
        let (again, _) = persistence::load(&resaved)
            .unwrap()
            .into_editable_with_container()
            .unwrap_or_else(|_| panic!("expected editable"));
        assert_eq!(again.current_rule_program(), Some(&program));
    }

    // A history that ends at another program than the document's is not its
    // history: the document keeps its own program and loses the Undo steps.
    let mut elsewhere =
        persistence::save_document_store(&store, &persistence::ContainerData::default()).unwrap();
    let (checksum_offset, program_offset, program_len) =
        container_entry_offsets(&elsewhere, "program.json");
    let json = &mut elsewhere[program_offset..program_offset + program_len];
    let width = json
        .windows(3)
        .position(|window| window == b"900")
        .expect("the program names its width");
    json[width + 2] = b'1';
    rehash_container_entry(&mut elsewhere, checksum_offset, program_offset, program_len);
    assert_eq!(
        discarded_history(&elsewhere),
        PersistenceError::InvalidRevisionHistory.to_string()
    );
    let (reopened, _) = persistence::load(&elsewhere)
        .unwrap()
        .into_editable_with_container()
        .unwrap_or_else(|_| panic!("expected editable"));
    assert_eq!(
        reopened
            .current_rule_program()
            .map(|program| &*program.source),
        Some("w = param('width', 901)\n")
    );
}

#[test]
fn revision_catalog_checkpoint_diff_and_rollback_are_auditable_and_persistent() {
    let mut store = graph_document();
    let base = store.current();
    store
        .create_checkpoint(
            base.revision_id(),
            &base.canonical_digest(),
            "Reviewed base",
        )
        .unwrap();

    let proposal = store
        .prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("5", 5.0).unwrap(),
            }]),
            ProposalContext::local_assistant_model(),
        )
        .unwrap();
    store.commit_verified_proposal(&proposal).unwrap();
    let assistant = store.current();
    assert_eq!(
        store.revision_history().last().unwrap().origin(),
        RevisionOrigin::Principal(ProposalPrincipal::LocalAssistant)
    );
    assert!(
        store
            .compare_revisions(base.revision_id(), base.revision_id())
            .unwrap()
            .changes
            .is_empty()
    );
    assert_eq!(
        store
            .compare_revisions(base.revision_id(), assistant.revision_id())
            .unwrap()
            .changes
            .last()
            .unwrap()
            .structure,
        "canonical-document"
    );
    assert_eq!(
        store.create_checkpoint(
            assistant.revision_id(),
            &assistant.canonical_digest(),
            "Reviewed base",
        ),
        Err(RevisionHistoryError::DuplicateCheckpointName)
    );
    store
        .create_checkpoint(
            assistant.revision_id(),
            &assistant.canonical_digest(),
            "Assistant option",
        )
        .unwrap();

    let bytes =
        persistence::save_document_store(&store, &persistence::ContainerData::default()).unwrap();
    let mut reopened = ketchup_model::testing::load_with_history(&bytes)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened.revision_catalog(), store.revision_catalog());

    let rolled_back = reopened
        .rollback_to_revision(
            assistant.revision_id(),
            &assistant.canonical_digest(),
            base.revision_id(),
            ProposalPrincipal::ManualClient,
        )
        .unwrap();
    assert_eq!(
        rolled_back.snapshot().canonical_digest(),
        base.canonical_digest()
    );
    assert_eq!(
        rolled_back.origin(),
        RevisionOrigin::Rollback {
            principal: ProposalPrincipal::ManualClient,
            target_revision: base.revision_id(),
        }
    );
    assert_eq!(
        reopened.undo().unwrap().canonical_digest(),
        assistant.canonical_digest()
    );
    assert_eq!(
        reopened.redo().unwrap().canonical_digest(),
        base.canonical_digest()
    );
}

/// Review 2026-10-21 UNDO-1: Save writes only the current revision, so a named
/// checkpoint on that revision must come back with the file; one on an earlier
/// revision has no saved model to name and is not kept.
#[test]
fn a_named_checkpoint_on_the_saved_revision_survives_save_and_reopen() {
    let mut store = graph_document();
    let base = store.current();
    store
        .create_checkpoint(base.revision_id(), &base.canonical_digest(), "Draft")
        .unwrap();
    let proposal = store
        .prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("5", 5.0).unwrap(),
            }]),
            ProposalContext::local_assistant_model(),
        )
        .unwrap();
    store.commit_verified_proposal(&proposal).unwrap();
    let saved = store.current();
    store
        .create_checkpoint(
            saved.revision_id(),
            &saved.canonical_digest(),
            "Sent to client",
        )
        .unwrap();

    let prepared =
        persistence::prepare_document(&store, &persistence::ContainerData::default()).unwrap();
    let mut reopened = persistence::load(prepared.bytes())
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let catalog = reopened.revision_catalog();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].revision_id, saved.revision_id());
    assert_eq!(catalog[0].canonical_digest, saved.canonical_digest());
    assert_eq!(catalog[0].checkpoint.as_deref(), Some("Sent to client"));
    assert!(reopened.undo().is_none());

    // A file whose current revision has no checkpoint stays as it was.
    let unnamed = graph_document();
    let unnamed_bytes =
        persistence::prepare_document(&unnamed, &persistence::ContainerData::default()).unwrap();
    let reopened_unnamed = persistence::load(unnamed_bytes.bytes())
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened_unnamed.revision_catalog()[0].checkpoint, None);
}

#[test]
fn revision_metadata_and_rollback_reject_invalid_or_stale_requests_without_mutation() {
    let mut store = graph_document();
    let current = store.current();
    let catalog = store.revision_catalog();
    assert_eq!(
        store.create_checkpoint(current.revision_id(), &current.canonical_digest(), " bad"),
        Err(RevisionHistoryError::InvalidCheckpointName)
    );
    assert_eq!(
        store.create_checkpoint(
            current.revision_id() + 1,
            &current.canonical_digest(),
            "Valid"
        ),
        Err(RevisionHistoryError::Stale)
    );
    for principal in [ProposalPrincipal::Human(0), ProposalPrincipal::Plugin(0)] {
        assert!(matches!(
            store.rollback_to_revision(
                current.revision_id(),
                &current.canonical_digest(),
                current.revision_id(),
                principal,
            ),
            Err(RevisionHistoryError::InvalidPrincipal)
        ));
    }
    assert!(matches!(
        store.rollback_to_revision(
            current.revision_id(),
            &current.canonical_digest(),
            current.revision_id(),
            ProposalPrincipal::ManualClient,
        ),
        Err(RevisionHistoryError::NoOpRollback)
    ));
    assert!(matches!(
        store.rollback_to_revision(
            current.revision_id(),
            &current.canonical_digest(),
            u64::MAX,
            ProposalPrincipal::ManualClient,
        ),
        Err(RevisionHistoryError::RevisionNotFound(id)) if id == u64::MAX
    ));
    assert_eq!(store.revision_catalog(), catalog);
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
    assert!(matches!(
        recovered.audit().notices.as_slice(),
        [persistence::LoadNotice::OpenedBackup { .. }]
    ));
}

/// Review PS-8: when the file and its `.recovery` backup both fail to open, the answer
/// names both failures and the backup's path; without a backup it is the file's own.
#[test]
fn a_damaged_file_whose_backup_is_damaged_too_names_both_failures() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("damaged.ketchup");
    let backup_path = path.with_extension("ketchup.recovery");
    let store = graph_document();
    std::fs::write(&path, b"interrupted replacement").unwrap();
    assert!(matches!(
        persistence::load_file(&path),
        Err(persistence::FilePersistenceError::Format(
            PersistenceError::InvalidMagic
        ))
    ));

    let mut backup =
        persistence::save_container(&store.current(), &persistence::ContainerData::default())
            .unwrap();
    *backup.last_mut().unwrap() ^= 0xff;
    std::fs::write(&backup_path, &backup).unwrap();
    let Err(persistence::FilePersistenceError::BackupUnusable {
        primary,
        backup_path: named,
        backup,
    }) = persistence::load_file(&path)
    else {
        panic!("the backup's failure was not reported");
    };
    assert!(matches!(
        *primary,
        persistence::FilePersistenceError::Format(PersistenceError::InvalidMagic)
    ));
    assert_eq!(named, backup_path);
    assert!(matches!(
        *backup,
        persistence::FilePersistenceError::Format(PersistenceError::ContainerChecksumMismatch(
            ref entry
        )) if entry == "document.bin"
    ));
}

/// A file a newer Kečup wrote is not damaged: opening it says so and never
/// falls back to its older `.recovery`, whose next Save would overwrite the
/// newer work.
#[test]
fn a_primary_from_a_newer_format_is_refused_and_never_replaced_by_its_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let data = persistence::ContainerData::default();
    let mut store = graph_document();
    for (name, schema_offset) in [("container.ketchup", true), ("snapshot.ketchup", false)] {
        let path = directory.path().join(name);
        let save = |store: &DocumentStore| {
            if schema_offset {
                publish(&path, store, &data, persistence::Publish::Replace).unwrap();
            } else {
                persistence::save_atomic(&path, &store.current()).unwrap();
            }
        };
        save(&store);
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(1),
                    dimension: Dimension::new("9", 9.0).unwrap(),
                },
            ]))
            .unwrap();
        save(&store);
        assert!(path.with_extension("ketchup.recovery").is_file());

        let mut newer = std::fs::read(&path).unwrap();
        newer[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
        std::fs::write(&path, newer).unwrap();
        let Err(persistence::FilePersistenceError::Format(error)) = persistence::load_file(&path)
        else {
            panic!("{name}: a newer file must not open from its recovery");
        };
        assert!(
            matches!(
                error,
                PersistenceError::UnsupportedContainerSchema(u16::MAX)
                    | PersistenceError::UnsupportedSchema(u16::MAX)
            ),
            "{name}: {error}"
        );
    }
}

/// A program a newer Kečup stored with a field this version does not know is
/// refused like such a document field: opened without it, the next Save would
/// drop it, and the older `.recovery` must not open in its place either.
#[test]
fn a_program_with_a_field_from_a_newer_writer_is_refused_and_never_replaced_by_its_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("programmed.ketchup");
    let data = persistence::ContainerData::default();
    let mut program = program_with_variants();
    program.file_name = format!("{}.star", "p".repeat(24));
    let mut store = graph_document();
    store.bind_rule_program(program.clone()).unwrap();
    publish(&path, &store, &data, persistence::Publish::Replace).unwrap();
    program.overrides.insert("width".to_owned(), 900.0);
    store.replace_rule_program_source(program.clone()).unwrap();
    publish(&path, &store, &data, persistence::Publish::Replace).unwrap();
    assert!(path.with_extension("ketchup.recovery").is_file());

    // The same entry length, so only the program's content changes.
    let mut newer = std::fs::read(&path).unwrap();
    let (checksum_offset, program_offset, program_len) =
        container_entry_offsets(&newer, "program.json");
    let old = format!(r#""file_name":"{}""#, program.file_name);
    let prefix = r#""file_name":"p.star","units":""#;
    let new = format!(r#"{prefix}{}""#, "m".repeat(old.len() - prefix.len() - 1));
    let json = &mut newer[program_offset..program_offset + program_len];
    let at = json
        .windows(old.len())
        .position(|window| window == old.as_bytes())
        .expect("the entry names the program file");
    json[at..at + old.len()].copy_from_slice(new.as_bytes());
    rehash_container_entry(&mut newer, checksum_offset, program_offset, program_len);
    std::fs::write(&path, newer).unwrap();

    let Err(persistence::FilePersistenceError::Format(error)) = persistence::load_file(&path)
    else {
        panic!("a program with a newer field must not open without it");
    };
    assert_eq!(
        error,
        PersistenceError::UnknownFields(vec!["program.units".to_owned()])
    );
}

/// Review 2026-10-15 S2: publishing checks the checksums of the file it writes and
/// of the file it keeps as `.recovery`, without decoding either document again.
#[test]
fn save_over_a_file_decodes_neither_the_new_nor_the_replaced_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.ketchup");
    let data = persistence::ContainerData::default();
    let mut store = graph_document();
    let first = publish(&path, &store, &data, persistence::Publish::IfAbsent).unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("9", 9.0).unwrap(),
            },
        ]))
        .unwrap();
    let previous = std::fs::read(&path).unwrap();

    let (saved, decodes) = persistence::headless_count_snapshot_decodes(|| {
        publish(
            &path,
            &store,
            &data,
            persistence::Publish::IfUnchanged(first),
        )
    });
    saved.unwrap();
    assert_eq!(decodes, 0, "Save decoded {decodes} document snapshots");
    assert_eq!(
        std::fs::read(path.with_extension("ketchup.recovery")).unwrap(),
        previous
    );
    assert_eq!(
        persistence::load_file(&path)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        store.current().canonical_digest()
    );
}

#[test]
fn conditional_save_rejects_external_replacement_without_touching_primary_or_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.ketchup");
    let container_data = persistence::ContainerData::default();
    let initial = graph_document();
    publish(
        &path,
        &initial,
        &container_data,
        persistence::Publish::Replace,
    )
    .unwrap();
    let opened_identity = persistence::read_native_document_identity(&path).unwrap();
    let source = std::fs::read(&path).unwrap();
    let mut first = persistence::load(&source)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let mut second = persistence::load(&source)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();

    first
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("11", 11.0).unwrap(),
            },
        ]))
        .unwrap();
    let first_identity = publish(
        &path,
        &first,
        &container_data,
        persistence::Publish::IfUnchanged(opened_identity),
    )
    .unwrap();
    let primary_after_first = std::fs::read(&path).unwrap();
    let recovery_after_first = std::fs::read(path.with_extension("ketchup.recovery")).unwrap();

    second
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("12", 12.0).unwrap(),
            },
        ]))
        .unwrap();
    assert!(matches!(
        publish(
            &path,
            &second,
            &container_data,
            persistence::Publish::IfUnchanged(opened_identity)
        ),
        Err(persistence::FilePersistenceError::ExternalConflict)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), primary_after_first);
    assert_eq!(
        std::fs::read(path.with_extension("ketchup.recovery")).unwrap(),
        recovery_after_first
    );

    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        publish(
            &path,
            &second,
            &container_data,
            persistence::Publish::IfUnchanged(first_identity)
        ),
        Err(persistence::FilePersistenceError::ExternalConflict)
    ));
    assert!(!path.exists());
}

#[test]
fn dirty_work_recovery_is_bound_to_the_primary_and_corruption_is_ignored() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("working.ketchup");
    let work_recovery = persistence::work_recovery_path(&path);
    let container_data = persistence::ContainerData::default();
    let mut store = graph_document();
    publish(
        &path,
        &store,
        &container_data,
        persistence::Publish::Replace,
    )
    .unwrap();
    let primary_bytes = std::fs::read(&path).unwrap();
    let base_identity = persistence::read_native_document_identity(&path).unwrap();

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("17", 17.0).unwrap(),
            },
        ]))
        .unwrap();
    let dirty_digest = store.current().canonical_digest();
    checkpoint(&path, &store, &container_data, base_identity, None).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), primary_bytes);

    let recovered = persistence::load_file_with_source(&path).unwrap();
    assert_eq!(recovered.source_path(), work_recovery);
    assert_eq!(
        recovered.outcome().snapshot().canonical_digest(),
        dirty_digest
    );
    assert!(matches!(
        recovered.outcome().audit().notices.as_slice(),
        [persistence::LoadNotice::OpenedWorkRecovery { .. }]
    ));

    std::fs::write(&work_recovery, b"interrupted checkpoint").unwrap();
    let clean = persistence::load_file_with_source(&path).unwrap();
    assert_eq!(clean.source_path(), path);
    assert_ne!(clean.outcome().snapshot().canonical_digest(), dirty_digest);
    assert!(
        persistence::clear_work_recovery(
            &path,
            Some(persistence::FileIdentity::from_bytes(
                b"interrupted checkpoint"
            ))
        )
        .unwrap()
    );

    checkpoint(&path, &store, &container_data, base_identity, None).unwrap();
    let mut replacement = graph_document();
    replacement
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("23", 23.0).unwrap(),
            },
        ]))
        .unwrap();
    publish(
        &path,
        &replacement,
        &container_data,
        persistence::Publish::Replace,
    )
    .unwrap();
    let replaced = persistence::load_file_with_source(&path).unwrap();
    assert_eq!(replaced.source_path(), path);
    assert_eq!(
        replaced.outcome().snapshot().canonical_digest(),
        replacement.current().canonical_digest()
    );
}

/// A crash while an edit appends its step to the work-recovery checkpoint
/// (idea #49) leaves the step cut short. The open restores the work up to the
/// step before it and says the step was damaged; the next edit writes the whole
/// checkpoint again instead of appending after the damaged step.
#[test]
fn a_work_recovery_step_cut_short_opens_the_steps_before_it_and_says_so() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cut-short.ketchup");
    let work_recovery = persistence::work_recovery_path(&path);
    let container_data = persistence::ContainerData::default();
    let set_dimension = |store: &mut DocumentStore, value: f64| {
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(1),
                    dimension: Dimension::new(value.to_string(), value).unwrap(),
                },
            ]))
            .unwrap();
    };
    let mut store = graph_document();
    publish(
        &path,
        &store,
        &container_data,
        persistence::Publish::Replace,
    )
    .unwrap();
    let base = persistence::read_native_document_identity(&path).unwrap();
    let mut identity = None;
    let mut digests = Vec::new();
    for value in [17.0, 19.0, 23.0] {
        set_dimension(&mut store, value);
        identity = Some(checkpoint(&path, &store, &container_data, base, identity).unwrap());
        digests.push((
            store.current().canonical_digest(),
            store.visible_undo_steps(),
        ));
    }
    let whole = std::fs::read(&work_recovery).unwrap();
    std::fs::write(&work_recovery, &whole[..whole.len() - 5]).unwrap();

    let opened = persistence::load_file_with_source(&path).unwrap();
    assert_eq!(opened.source_path(), work_recovery);
    assert_eq!(opened.outcome().snapshot().canonical_digest(), digests[1].0);
    match opened.outcome().audit().notices.as_slice() {
        [
            persistence::LoadNotice::OpenedWorkRecovery { .. },
            persistence::LoadNotice::WorkRecoveryStepDamaged { reason },
        ] => assert!(reason.contains("step 2"), "{reason}"),
        other => panic!("{other:?}"),
    }

    let opened_identity = opened.work_recovery_identity();
    let mut reopened = opened.into_parts().0.into_editable().ok().unwrap();
    assert_eq!(reopened.visible_undo_steps(), digests[1].1);
    set_dimension(&mut reopened, 29.0);
    checkpoint(&path, &reopened, &container_data, base, opened_identity).unwrap();
    let again = persistence::load_file_with_source(&path).unwrap();
    assert_eq!(
        again.outcome().snapshot().canonical_digest(),
        reopened.current().canonical_digest()
    );
    assert!(matches!(
        again.outcome().audit().notices.as_slice(),
        [persistence::LoadNotice::OpenedWorkRecovery { .. }]
    ));
}

/// Review 2026-10-24 MD-25: the revisions the steps of a checkpoint add count against one
/// budget for the whole open, not a new one per step. Past it the open gives up the
/// oldest Undo steps, as a whole checkpoint keeps fewer, and still opens the last edit
/// with the recent ones.
#[test]
fn a_checkpoint_of_many_steps_opens_within_one_budget_by_giving_up_the_oldest_undo_steps() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("many-steps.ketchup");
    let container_data = persistence::ContainerData::default();
    let mut store = graph_document();
    publish(
        &path,
        &store,
        &container_data,
        persistence::Publish::Replace,
    )
    .unwrap();
    let revision_len = std::fs::read(&path).unwrap().len();
    let base = persistence::read_native_document_identity(&path).unwrap();
    let mut identity = None;
    for value in [17.0, 19.0, 23.0, 29.0, 31.0, 37.0] {
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(1),
                    dimension: Dimension::new(value.to_string(), value).unwrap(),
                },
            ]))
            .unwrap();
        identity = Some(checkpoint(&path, &store, &container_data, base, identity).unwrap());
    }
    let all_steps = store.visible_undo_steps();
    assert!(all_steps >= 6, "{all_steps}");
    let whole = persistence::load_file_with_source(&path).unwrap();
    let whole = whole.into_parts().0.into_editable().ok().unwrap();
    assert_eq!(whole.visible_undo_steps(), all_steps);

    let opened = persistence::headless_with_checkpoint_assembly_budget(3 * revision_len, || {
        persistence::load_file_with_source(&path).unwrap()
    });
    assert_eq!(
        opened.outcome().snapshot().canonical_digest(),
        store.current().canonical_digest()
    );
    assert!(matches!(
        opened.outcome().audit().notices.as_slice(),
        [persistence::LoadNotice::OpenedWorkRecovery { .. }]
    ));
    let opened = opened.into_parts().0.into_editable().ok().unwrap();
    let kept = opened.visible_undo_steps();
    assert!(
        0 < kept && kept < all_steps,
        "{kept} of {all_steps} Undo steps"
    );

    // Undone back past what the budget keeps, the step that returns there is the one
    // the open cannot take; it says so and opens the edit before it.
    let last_edit = store.current().canonical_digest();
    for _ in 0..5 {
        store.undo().unwrap();
    }
    checkpoint(&path, &store, &container_data, base, identity).unwrap();
    let opened = persistence::headless_with_checkpoint_assembly_budget(3 * revision_len, || {
        persistence::load_file_with_source(&path).unwrap()
    });
    match opened.outcome().audit().notices.as_slice() {
        [
            persistence::LoadNotice::OpenedWorkRecovery { .. },
            persistence::LoadNotice::WorkRecoveryStepDamaged { reason },
        ] => assert!(reason.contains("resource limit"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(opened.outcome().snapshot().canonical_digest(), last_edit);
    let whole = persistence::load_file_with_source(&path).unwrap();
    assert_eq!(
        whole.outcome().snapshot().canonical_digest(),
        store.current().canonical_digest()
    );
}

/// Review 2026-10-24 MD-26, the rest of M-12: a blob the file holds that no revision of
/// the opened document names, such as the source of an import an earlier Kečup kept only
/// for Undo steps a file no longer opens with, leaves the file at the next Save instead of
/// being written into every one after.
#[test]
fn a_blob_no_opened_revision_names_leaves_the_file_at_the_next_save() {
    use ketchup_model::underlay::{plan_underlay, plan_underlay_removal};
    let mut store = graph_document();
    let mut container = persistence::ContainerData::default();
    let lines = b"the lines only an old Undo step named".to_vec();
    let orphan = container.insert_import_blob(lines.clone()).unwrap();
    let (batch, id) = plan_underlay(
        &store.current(),
        "Plan",
        &[],
        &[],
        ketchup_model::graph::sha256_bytes(&lines),
        [0.0; 2],
    )
    .unwrap();
    store.apply_batch(&batch).unwrap();
    store
        .apply_batch(&plan_underlay_removal(&store.current(), id).unwrap())
        .unwrap();
    // As an earlier Kečup saved it: with its Undo steps, the blob kept for them.
    // A blob no revision names stays, as a newer Kečup may keep one (see
    // native_container_preserves_optional_extensions_and_content_addressed_blobs).
    let saved = persistence::save_document_store(&store, &container).unwrap();
    let open = |bytes: &[u8]| {
        persistence::load(bytes)
            .unwrap()
            .into_editable_with_container()
            .ok()
            .unwrap()
    };
    let (opened, data) = open(&saved);
    assert!(data.blobs().contains_key(&orphan));
    let again = persistence::save_container(&opened.current(), &data).unwrap();
    let (_, data) = open(&again);
    assert!(
        !data.blobs().contains_key(&orphan),
        "the orphan blob is saved again"
    );
}

/// Review 2026-10-21 LIM-1: every edit checks that neither the file nor its
/// checkpoint changed on disk. With a large blob in the file, an edit after the
/// first hashes its step, not the file, the checkpoint or the blob again, and
/// still refuses to write when another program replaced either file.
#[test]
fn an_edit_after_the_first_hashes_its_step_not_the_file_and_still_sees_a_replaced_one() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.ketchup");
    let work_recovery = persistence::work_recovery_path(&path);
    let blob_len = 4 << 20;
    let mut container_data = persistence::ContainerData::default();
    container_data
        .insert_blob((0..blob_len).map(|index| (index % 251) as u8).collect())
        .unwrap();
    let set_dimension = |store: &mut DocumentStore, value: f64| {
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(1),
                    dimension: Dimension::new(value.to_string(), value).unwrap(),
                },
            ]))
            .unwrap();
    };
    let mut store = graph_document();
    let base = publish(
        &path,
        &store,
        &container_data,
        persistence::Publish::Replace,
    )
    .unwrap();
    let mut identity = None;
    let mut hashed = Vec::new();
    for value in [17.0, 19.0, 23.0, 29.0] {
        set_dimension(&mut store, value);
        let (written, bytes) = ketchup_model::graph::headless_count_hashed_bytes(|| {
            checkpoint(&path, &store, &container_data, base, identity)
        });
        identity = Some(written.unwrap());
        hashed.push(bytes);
    }
    let hashed_bytes = |edit: usize| usize::try_from(hashed[edit]).unwrap();
    assert!(hashed_bytes(0) > blob_len, "{hashed:?}");
    assert!(
        (1..hashed.len()).all(|edit| hashed_bytes(edit) < blob_len / 64),
        "{hashed:?}"
    );
    let opened = persistence::load_file_with_source(&path).unwrap();
    assert_eq!(opened.source_path(), work_recovery);
    assert_eq!(
        opened.outcome().snapshot().canonical_digest(),
        store.current().canonical_digest()
    );
    assert_eq!(opened.work_recovery_identity(), identity);

    // Another program writes the checkpoint.
    let mut changed = std::fs::read(&work_recovery).unwrap();
    changed.push(0);
    std::fs::write(&work_recovery, &changed).unwrap();
    set_dimension(&mut store, 31.0);
    assert!(matches!(
        checkpoint(&path, &store, &container_data, base, identity),
        Err(persistence::FilePersistenceError::ExternalConflict)
    ));
    assert_eq!(std::fs::read(&work_recovery).unwrap(), changed);

    // Another session saves the file itself.
    let changed = persistence::FileIdentity::from_bytes(&changed);
    let rewritten = checkpoint(&path, &store, &container_data, base, Some(changed)).unwrap();
    let mut other = graph_document();
    set_dimension(&mut other, 37.0);
    publish(
        &path,
        &other,
        &container_data,
        persistence::Publish::Replace,
    )
    .unwrap();
    let saved_by_other = std::fs::read(&path).unwrap();
    set_dimension(&mut store, 41.0);
    let recovery_before = std::fs::read(&work_recovery).unwrap();
    assert!(matches!(
        checkpoint(&path, &store, &container_data, base, Some(rewritten)),
        Err(persistence::FilePersistenceError::ExternalConflict)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), saved_by_other);
    assert_eq!(std::fs::read(&work_recovery).unwrap(), recovery_before);
}

#[test]
fn stale_session_cleanup_does_not_delete_a_newer_work_recovery_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared-working.ketchup");
    let work_recovery = persistence::work_recovery_path(&path);
    let container_data = persistence::ContainerData::default();
    let mut first = graph_document();
    publish(
        &path,
        &first,
        &container_data,
        persistence::Publish::Replace,
    )
    .unwrap();
    let base_identity = persistence::read_native_document_identity(&path).unwrap();

    first
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("17", 17.0).unwrap(),
            },
        ]))
        .unwrap();
    let first_checkpoint_identity =
        checkpoint(&path, &first, &container_data, base_identity, None).unwrap();

    let preserved = std::fs::read(&work_recovery).unwrap();
    assert!(
        checkpoint(
            &path,
            &graph_document(),
            &container_data,
            base_identity,
            None
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&work_recovery).unwrap(), preserved);
    let mut second = graph_document();
    second
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("23", 23.0).unwrap(),
            },
        ]))
        .unwrap();
    let second_checkpoint_identity = checkpoint(
        &path,
        &second,
        &container_data,
        base_identity,
        Some(first_checkpoint_identity),
    )
    .unwrap();
    let newer_checkpoint = std::fs::read(&work_recovery).unwrap();

    assert!(!persistence::clear_work_recovery(&path, Some(first_checkpoint_identity)).unwrap());

    assert_eq!(std::fs::read(&work_recovery).unwrap(), newer_checkpoint);
    assert!(!persistence::clear_work_recovery(&path, None).unwrap());
    assert_eq!(std::fs::read(&work_recovery).unwrap(), newer_checkpoint);
    assert!(persistence::clear_work_recovery(&path, Some(second_checkpoint_identity)).unwrap());
    assert!(!work_recovery.exists());
}

/// A native file has no size limit of its own (an underlay's lines alone may pass the
/// 64 MiB it once had), but every length it stores must fit in the rest of it.
#[test]
fn a_native_file_has_no_size_limit_but_its_stored_lengths_fit_the_rest_of_it() {
    const BLOB: usize = 65 * 1024 * 1024;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.ketchup");
    let store = graph_document();
    let mut container_data = persistence::ContainerData::default();
    let hash = container_data.insert_blob(vec![7; BLOB]).unwrap();
    let bytes = persistence::save_container(&store.current(), &container_data).unwrap();
    assert!(bytes.len() > BLOB);
    std::fs::write(&path, &bytes).unwrap();
    let loaded = persistence::load_file(&path).unwrap();
    assert_eq!(loaded.container_data().blobs()[&hash].len(), BLOB);

    // The blob's stored length sits just before its checksum, the SHA-256 its name is.
    let checksum = (0..32)
        .map(|index| u8::from_str_radix(&hash[2 * index..2 * index + 2], 16).unwrap())
        .collect::<Vec<_>>();
    let at = bytes
        .windows(32)
        .position(|window| window == checksum.as_slice())
        .unwrap()
        - 8;
    let rest = (bytes.len() - at - 8 - 32) as u64;
    for length in [rest + 1, u64::MAX] {
        let mut damaged = bytes.clone();
        damaged[at..at + 8].copy_from_slice(&length.to_le_bytes());
        assert_eq!(
            persistence::load(&damaged).err(),
            Some(PersistenceError::Truncated),
            "a stored length of {length}"
        );
    }
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
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    // Schema 52 wrote the occurrence record without the optional-color byte.
    let bytes = include_bytes!("fixtures/persistence/legacy/occurrence-schema52.bin");
    let loaded = persistence::load(bytes).unwrap();
    assert_eq!(loaded.source_schema(), 52);
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
        with_document_id(&store.current(), loaded.document().document_id()).canonical_digest()
    );
}

#[test]
fn sheet_metal_schema_81_remains_readable_and_current_schema_is_byte_stable() {
    let definition_id = DefinitionId(1);
    let feature_id = FeatureId(1);
    // Schema 81 stored a 100 x 50 rectangle with a flange on its MaxX side: edge 1.
    let spec = SheetMetalSpec {
        base_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]],
        thickness: Dimension::new("2 mm", 2.0).unwrap(),
        k_factor: 0.4,
        bends: vec![SheetMetalBend {
            parent: None,
            edge: 1,
            length: Dimension::new("30 mm", 30.0).unwrap(),
            angle_degrees: -90.0,
            inner_radius: Dimension::new("3 mm", 3.0).unwrap(),
        }],
    };
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: "Sheet metal part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: feature_id,
                definition_id,
                name: "Base with flange".into(),
                kind: FeatureKind::SheetMetal(spec.clone()),
            },
        ]))
        .unwrap();

    let expected_digest = store.current().canonical_digest();
    let bytes = persistence::save(&store.current());
    assert_eq!(
        u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
        persistence::OLDEST_WRITTEN_SCHEMA
    );
    let loaded = persistence::load(&bytes).unwrap();
    assert_eq!(loaded.disposition(), LoadDisposition::EditableLossless);
    assert_eq!(loaded.snapshot().canonical_digest(), expected_digest);
    assert_eq!(persistence::save(&loaded.snapshot()), bytes);
    assert!(matches!(
        loaded.snapshot().feature(feature_id).unwrap().kind(),
        FeatureKind::SheetMetal(reopened) if reopened == &spec
    ));

    let schema_81 = include_bytes!("fixtures/persistence/legacy/sheet-metal-schema81.bin");
    let loaded_81 = persistence::load(schema_81).unwrap();
    assert_eq!(loaded_81.source_schema(), 81);
    assert_eq!(loaded_81.disposition(), LoadDisposition::EditableLossless);
    assert_eq!(
        loaded_81.snapshot().canonical_digest(),
        with_document_id(&store.current(), loaded_81.snapshot().document_id()).canonical_digest()
    );
    assert!(matches!(
        loaded_81.snapshot().feature(feature_id).unwrap().kind(),
        FeatureKind::SheetMetal(reopened) if reopened == &spec
    ));
}

#[test]
fn document_tolerance_is_saved_undoable_and_part_of_the_digest() {
    let mut store = DocumentStore::new();
    let default_digest = store.current().canonical_digest();
    let default_bytes = persistence::save(&store.current());
    assert_eq!(store.current().tolerance(), TolerancePolicy::default());

    let custom = TolerancePolicy::with_angular(0.01, 1.0e-6).unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetTolerance {
            tolerance: custom,
        }]))
        .unwrap();
    assert_eq!(store.current().tolerance(), custom);
    assert_ne!(store.current().canonical_digest(), default_digest);

    let bytes = persistence::save(&store.current());
    let loaded = persistence::load(&bytes).unwrap();
    assert_eq!(loaded.disposition(), LoadDisposition::EditableLossless);
    assert_eq!(loaded.snapshot().tolerance(), custom);
    assert_eq!(
        loaded.snapshot().canonical_digest(),
        store.current().canonical_digest()
    );

    // Back on the default policy the document stores and hashes exactly as before.
    store.undo().unwrap();
    assert_eq!(store.current().tolerance(), TolerancePolicy::default());
    assert_eq!(store.current().canonical_digest(), default_digest);
    assert_eq!(persistence::save(&store.current()), default_bytes);

    // A stored policy is checked like a constructed one.
    let invalid = ketchup_model::testing::rewrite_saved_snapshot(&bytes, |saved| {
        let product = ketchup_model::testing::cbor_entry(saved, "product");
        let tolerance = ketchup_model::testing::cbor_entry(product, "tolerance");
        *ketchup_model::testing::cbor_entry(tolerance, "linear_mm") = ciborium::Value::Float(0.0);
    });
    assert!(matches!(
        load_error(&invalid),
        PersistenceError::InvalidPayload {
            section: persistence::PayloadSection::Snapshot,
            error: persistence::PayloadError::Data { .. },
        }
    ));
}

/// A lock file that cannot be written in a folder that takes files (another user's on a
/// shared drive, one marked read-only by an unpacked archive) is no read-only folder: it
/// is locked through a read-only handle, so a session holding it keeps the next one out.
/// Only a folder that takes no file opens without a lock (review 2026-10-24 UI-21).
#[test]
fn a_read_only_lock_file_in_a_writable_folder_still_locks_the_file() {
    use ketchup_model::persistence::{FilePersistenceError, WorkRecoveryLock};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.ketchup");
    let mut lock = path.as_os_str().to_owned();
    lock.push(".work-recovery-lock");
    std::fs::write(&lock, b"").unwrap();
    let mut permissions = std::fs::metadata(&lock).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&lock, permissions.clone()).unwrap();

    let held = WorkRecoveryLock::acquire_unless_read_only(&path).unwrap();
    assert!(held.is_some(), "a writable folder takes a checkpoint");
    assert!(matches!(
        WorkRecoveryLock::acquire_unless_read_only(&path),
        Err(FilePersistenceError::ExternalConflict)
    ));
    drop(held);
    let read_only = persistence::headless_with_read_only_folder(directory.path(), || {
        WorkRecoveryLock::acquire_unless_read_only(&path).unwrap()
    });
    assert!(read_only.is_none());

    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&lock, permissions).unwrap();
}

/// A Linux NFS client locks a file not opened for writing only shared, so another
/// user's lock file it cannot write is locked shared through a read-only handle. That
/// still collides with the session holding the file: before, the second user opened and
/// saved the file without a lock while the first edited it (review 2026-10-26 MD-44).
/// From a read-only mount the file opens without a checkpoint, instead of failing to
/// open (review 2026-10-24 UI-21, capability review run 303).
#[test]
fn a_lock_file_locked_only_shared_still_keeps_a_second_user_out() {
    use ketchup_model::persistence::{FilePersistenceError, WorkRecoveryLock};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.ketchup");
    let mut lock = path.as_os_str().to_owned();
    lock.push(".work-recovery-lock");
    let first_user = WorkRecoveryLock::acquire(&path).unwrap();
    let mut permissions = std::fs::metadata(&lock).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&lock, permissions.clone()).unwrap();

    let second_user = persistence::headless_with_read_handle_lock_failure(|| {
        WorkRecoveryLock::acquire_unless_read_only(&path)
    });
    assert!(
        matches!(second_user, Err(FilePersistenceError::ExternalConflict)),
        "{second_user:?}"
    );
    drop(first_user);
    let writable_share = persistence::headless_with_read_handle_lock_failure(|| {
        WorkRecoveryLock::acquire_unless_read_only(&path)
    });
    assert!(matches!(writable_share, Ok(Some(_))), "{writable_share:?}");
    drop(writable_share);
    let read_only_mount = persistence::headless_with_read_handle_lock_failure(|| {
        persistence::headless_with_read_only_folder(directory.path(), || {
            WorkRecoveryLock::acquire_unless_read_only(&path)
        })
    });
    assert!(matches!(read_only_mount, Ok(None)), "{read_only_mount:?}");

    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&lock, permissions).unwrap();
}
