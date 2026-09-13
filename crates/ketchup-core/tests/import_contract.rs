use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, DocumentStore, FeatureKind,
    OccurrenceId, ProposalCommitError, Transform,
};
use ketchup_core::import::{
    IgesImportEvidence, IgesXdeImportEvidence, IgesXdeNodeEvidence, IgesXdePartEvidence,
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportId, ImportLengthUnit,
    ImportOutputRef, ImportReceipt, ImportUnitAuthority, ImportUnitDecision, StepImportEvidence,
    StepImportPlanError, StepXdeImportEvidence, StepXdeNodeEvidence, StepXdePartEvidence,
    plan_iges_import, plan_iges_xde_import, plan_step_import, plan_step_xde_import,
};
use ketchup_core::persistence;

fn receipt(id: u64, definition_id: u64, occurrence_id: u64, source: &[u8]) -> ImportReceipt {
    ImportReceipt::from_source_bytes(
        ImportId(id),
        ImportFormat::Stl,
        source,
        "part.stl",
        ImportUnitDecision::new(
            ImportLengthUnit::Millimetre,
            ImportUnitAuthority::UserDeclared,
        ),
        "ketchup-stl",
        "1",
        vec![
            ImportDiagnostic::new(ImportDiagnosticSeverity::Info, "mesh.manifold", None, 1)
                .unwrap(),
        ],
        vec![
            ImportOutputRef::Definition(DefinitionId(definition_id)),
            ImportOutputRef::Occurrence(OccurrenceId(occurrence_id)),
        ],
    )
    .unwrap()
}

fn import_batch(id: u64, definition_id: u64, occurrence_id: u64, source: &[u8]) -> CommandBatch {
    CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(definition_id),
            name: "Imported part".to_owned(),
        },
        CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(occurrence_id),
            definition_id: DefinitionId(definition_id),
            name: "Imported part".to_owned(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        },
        CanonicalCommand::RecordImport(receipt(id, definition_id, occurrence_id, source)),
    ])
}

#[test]
fn reviewed_import_is_one_undoable_persistent_deterministic_batch() {
    let source = b"solid deterministic-import-contract";
    let mut first = DocumentStore::new();
    let before = first.current().canonical_digest();
    let batch = import_batch(1, 1, 1, source);
    let batch_digest = batch.digest();
    let proposal = first.prepare_proposal(batch).unwrap();

    assert_eq!(first.current().canonical_digest(), before);
    assert_eq!(first.visible_undo_steps(), 0);

    first.commit_verified_proposal(&proposal).unwrap();
    let committed = first.current();
    let committed_digest = committed.canonical_digest();
    assert_ne!(committed_digest, before);
    assert_eq!(first.visible_undo_steps(), 1);
    assert_eq!(committed.import_receipts().count(), 1);
    assert_eq!(
        committed.import_receipt(ImportId(1)).unwrap().source_name(),
        "part.stl"
    );

    assert_eq!(first.undo().unwrap().canonical_digest(), before);
    assert_eq!(first.redo().unwrap().canonical_digest(), committed_digest);

    let encoded = persistence::save(&first.current());
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        reopened.snapshot().import_receipt(ImportId(1)).unwrap(),
        &receipt(1, 1, 1, source)
    );
    assert_eq!(persistence::save(&reopened.snapshot()), encoded);

    let second_batch = import_batch(1, 1, 1, source);
    assert_eq!(second_batch.digest(), batch_digest);
}

#[test]
fn exact_step_import_is_one_deterministic_persistent_undoable_transaction() {
    let source = b"ISO-10303-21;DATA;#1=SI_UNIT(.MILLI.,.METRE.);ENDSEC;END-ISO-10303-21;";
    let evidence = StepImportEvidence {
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "0123456789abcdef".to_owned(),
        body_kind: ketchup_core::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 600.0,
        volume_mm3: 1_000.0,
        bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        backend: "occt-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let batch = plan_step_import(&document.current(), source, "part.step", &evidence).unwrap();
    assert_eq!(
        batch.digest(),
        plan_step_import(&document.current(), source, "part.step", &evidence)
            .unwrap()
            .digest()
    );
    let proposal = document.prepare_proposal(batch).unwrap();
    document.commit_verified_proposal(&proposal).unwrap();
    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    let exact = committed
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(exact.source_byte_len, source.len() as u64);
    assert_eq!(exact.result_fingerprint, evidence.result_fingerprint);
    assert_eq!(exact.topology_counts, Some(evidence.topology_counts));
    assert_eq!(
        committed
            .import_receipt(ImportId(1))
            .unwrap()
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code())
            .collect::<Vec<_>>(),
        vec![
            "step_exact_brep_preserved",
            "step_color_metadata_unavailable",
            "step_hierarchy_flattened",
            "step_name_metadata_unavailable",
        ]
    );
    let mut container = persistence::ContainerData::default();
    let source_hash = container.insert_import_blob(source.to_vec()).unwrap();
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    let undone =
        persistence::load(&persistence::save_container(&document.current(), &container).unwrap())
            .unwrap();
    assert!(!undone.container_data().blobs().contains_key(&source_hash));
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
    assert!(matches!(
        persistence::load(&persistence::save(&document.current())),
        Err(persistence::PersistenceError::InvalidBlobHash)
    ));

    let encoded = persistence::save_container(&document.current(), &container).unwrap();
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        reopened
            .container_data()
            .blobs()
            .get(&source_hash)
            .map(Vec::as_slice),
        Some(source.as_slice())
    );
    assert_eq!(
        persistence::save_container(&reopened.snapshot(), reopened.container_data()).unwrap(),
        encoded
    );
}

#[test]
fn exact_iges_import_is_deterministic_persistent_and_explicit_about_losses() {
    let source = b"IGES exact source";
    let evidence = IgesImportEvidence {
        source_sha256: ketchup_core::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "fnv1a64:0123456789abcdef".to_owned(),
        body_kind: ketchup_core::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 600.0,
        volume_mm3: 1_000.0,
        bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        backend: "occt-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let batch = plan_iges_import(&document.current(), source, "part.iges", &evidence).unwrap();
    assert_eq!(
        batch.digest(),
        plan_iges_import(&document.current(), source, "part.iges", &evidence)
            .unwrap()
            .digest()
    );
    let mut stale_hash = evidence.clone();
    stale_hash.source_sha256[0] ^= 1;
    assert_eq!(
        plan_iges_import(&document.current(), source, "part.iges", &stale_hash),
        Err(ketchup_core::import::IgesImportPlanError::InvalidWorkerEvidence)
    );
    let mut stale_length = evidence.clone();
    stale_length.source_byte_len += 1;
    assert_eq!(
        plan_iges_import(&document.current(), source, "part.iges", &stale_length),
        Err(ketchup_core::import::IgesImportPlanError::InvalidWorkerEvidence)
    );
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);
    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    let receipt = committed.import_receipt(ImportId(1)).unwrap();
    assert_eq!(receipt.format(), ImportFormat::Iges);
    assert_eq!(receipt.parser_id(), "ketchup-occt-iges");
    assert_eq!(
        receipt
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code())
            .collect::<Vec<_>>(),
        vec![
            "iges_exact_brep_preserved",
            "iges_color_metadata_unavailable",
            "iges_hierarchy_flattened",
            "iges_name_metadata_unavailable",
            "iges_parametric_reconstruction_unavailable",
        ]
    );
    let mut container = persistence::ContainerData::default();
    container.insert_import_blob(source.to_vec()).unwrap();
    let encoded = persistence::save_container(&committed, &container).unwrap();
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        committed.canonical_digest()
    );
    assert_eq!(
        reopened
            .snapshot()
            .import_receipt(ImportId(1))
            .unwrap()
            .format(),
        ImportFormat::Iges
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed.canonical_digest()
    );
}

#[test]
fn iges_xde_import_preserves_flat_root_names_colors_and_reports_exact_losses() {
    let source = b"independent IGES XDE flat roots";
    let exact = |index: u32, x: f64| StepImportEvidence {
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: format!("fnv1a64:iges-root-{index}"),
        body_kind: ketchup_core::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 600.0,
        volume_mm3: 1_000.0,
        bounds_mm: [[x, 0.0, 0.0], [x + 10.0, 10.0, 10.0]],
        backend: "occt-iges-xde-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let evidence = IgesXdeImportEvidence {
        source_sha256: ketchup_core::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        parts: vec![
            IgesXdePartEvidence {
                index: 0,
                name: "Left body".to_owned(),
                name_from_source: true,
                color: Some([255, 0, 0]),
                exact: exact(0, 0.0),
            },
            IgesXdePartEvidence {
                index: 1,
                name: "Right body".to_owned(),
                name_from_source: true,
                color: Some([0, 255, 0]),
                exact: exact(1, 40.0),
            },
        ],
        nodes: vec![
            IgesXdeNodeEvidence {
                id: 0,
                part_index: 0,
                name: "Left occurrence".to_owned(),
                name_from_source: true,
                color: Some([255, 0, 0]),
                transform: Transform::identity(),
            },
            IgesXdeNodeEvidence {
                id: 1,
                part_index: 1,
                name: "Right occurrence".to_owned(),
                name_from_source: true,
                color: Some([0, 255, 0]),
                transform: Transform::identity(),
            },
        ],
    };
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let batch = plan_iges_xde_import(&document.current(), source, "roots.iges", &evidence).unwrap();
    assert_eq!(
        batch.digest(),
        plan_iges_xde_import(&document.current(), source, "roots.iges", &evidence)
            .unwrap()
            .digest()
    );
    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(committed.definitions().count(), 2);
    assert_eq!(committed.features().count(), 2);
    assert_eq!(committed.groups().count(), 0);
    let occurrences = committed.occurrences().collect::<Vec<_>>();
    assert_eq!(occurrences.len(), 2);
    assert_eq!(occurrences[0].name(), "Left occurrence");
    assert_eq!(occurrences[0].color(), Some([255, 0, 0]));
    assert_eq!(occurrences[1].name(), "Right occurrence");
    assert_eq!(occurrences[1].color(), Some([0, 255, 0]));
    for (index, feature) in committed.features().enumerate() {
        let FeatureKind::ImportedExactBody(spec) = feature.kind() else {
            panic!("IGES root must remain exact");
        };
        assert_eq!(spec.source_part_index, Some(index as u32));
    }
    let receipt = committed.import_receipt(ImportId(1)).unwrap();
    assert_eq!(receipt.parser_version(), "2");
    let diagnostic_codes = receipt
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code())
        .collect::<Vec<_>>();
    assert!(diagnostic_codes.contains(&"iges_name_metadata_preserved"));
    assert!(diagnostic_codes.contains(&"iges_color_metadata_preserved"));
    assert!(diagnostic_codes.contains(&"iges_hierarchy_unavailable_flat_roots_only"));
    assert!(diagnostic_codes.contains(&"iges_shared_definitions_unavailable_roots_duplicated"));
    assert!(diagnostic_codes.contains(&"iges_local_transforms_baked_into_exact_geometry"));
    let semantic = ketchup_core::state_view::encode_semantic_state(&committed);
    assert_eq!(
        semantic
            .complete_v1()
            .matches(".source_format=iges")
            .count(),
        2
    );
    assert!(semantic.complete_v1().contains(".source_part_index=0"));
    assert!(semantic.complete_v1().contains(".source_part_index=1"));
    assert_eq!(
        semantic
            .agent_v1()
            .matches("source_parametric_history:unavailable")
            .count(),
        2
    );

    let mut invalid = evidence.clone();
    invalid.nodes[1].transform = Transform::from_translation(1.0, 0.0, 0.0).unwrap();
    assert_eq!(
        plan_iges_xde_import(
            &DocumentStore::new().current(),
            source,
            "roots.iges",
            &invalid
        ),
        Err(ketchup_core::import::IgesImportPlanError::InvalidWorkerEvidence)
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed.canonical_digest()
    );
}

#[test]
fn step_xde_import_preserves_nested_repeated_instances_names_colors_and_round_trip() {
    let source = b"independent STEP XDE assembly bytes";
    let exact = StepImportEvidence {
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "fnv1a64:part-0".to_owned(),
        body_kind: ketchup_core::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 600.0,
        volume_mm3: 1_000.0,
        bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        backend: "occt-xde-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let evidence = StepXdeImportEvidence {
        source_sha256: ketchup_core::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        parts: vec![StepXdePartEvidence {
            index: 0,
            name: "Bracket".to_owned(),
            name_from_source: true,
            color: Some([0, 255, 0]),
            exact,
        }],
        nodes: vec![
            StepXdeNodeEvidence {
                id: 0,
                parent_id: None,
                part_index: None,
                name: "Machine".to_owned(),
                name_from_source: true,
                color: None,
                transform: Transform::identity(),
            },
            StepXdeNodeEvidence {
                id: 1,
                parent_id: Some(0),
                part_index: None,
                name: "Carriage".to_owned(),
                name_from_source: true,
                color: Some([0, 0, 255]),
                transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
            },
            StepXdeNodeEvidence {
                id: 2,
                parent_id: Some(1),
                part_index: Some(0),
                name: "Left bracket".to_owned(),
                name_from_source: true,
                color: Some([255, 0, 0]),
                transform: Transform::from_translation(1.0, 0.0, 0.0).unwrap(),
            },
            StepXdeNodeEvidence {
                id: 3,
                parent_id: Some(1),
                part_index: Some(0),
                name: "Right bracket".to_owned(),
                name_from_source: true,
                color: None,
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
            },
        ],
    };
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let batch =
        plan_step_xde_import(&document.current(), source, "machine.step", &evidence).unwrap();
    assert_eq!(
        batch.digest(),
        plan_step_xde_import(&document.current(), source, "machine.step", &evidence)
            .unwrap()
            .digest()
    );
    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(committed.definitions().count(), 1);
    assert_eq!(committed.features().count(), 1);
    assert_eq!(committed.groups().count(), 2);
    let occurrences = committed.occurrences().collect::<Vec<_>>();
    assert_eq!(occurrences.len(), 2);
    assert_eq!(
        occurrences[0].definition_id(),
        occurrences[1].definition_id()
    );
    assert_eq!(occurrences[0].name(), "Left bracket");
    assert_eq!(occurrences[0].color(), Some([255, 0, 0]));
    assert_eq!(occurrences[1].name(), "Right bracket");
    assert_eq!(occurrences[1].color(), Some([0, 0, 255]));
    let receipt = committed.import_receipt(ImportId(1)).unwrap();
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "step_assembly_color_inherited_by_leaf_instances"
    }));
    let feature = committed.features().next().unwrap();
    let FeatureKind::ImportedExactBody(spec) = feature.kind() else {
        panic!("XDE part must remain exact");
    };
    assert_eq!(spec.source_part_index, Some(0));

    let mut container = persistence::ContainerData::default();
    container.insert_import_blob(source.to_vec()).unwrap();
    let encoded = persistence::save_container(&committed, &container).unwrap();
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        committed.canonical_digest()
    );
    assert_eq!(
        persistence::save_container(&reopened.snapshot(), reopened.container_data()).unwrap(),
        encoded
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed.canonical_digest()
    );

    let mut tampered = evidence;
    tampered.nodes[1].parent_id = Some(3);
    assert_eq!(
        plan_step_xde_import(&document.current(), source, "machine.step", &tampered),
        Err(StepImportPlanError::InvalidWorkerEvidence)
    );
}

#[test]
fn exact_step_plan_refuses_invalid_worker_evidence_without_mutation() {
    let document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let valid = StepImportEvidence {
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "0123456789abcdef".to_owned(),
        body_kind: ketchup_core::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 6.0,
        volume_mm3: 1.0,
        bounds_mm: [[0.0; 3], [1.0; 3]],
        backend: "occt-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let mut invalid = valid;
    invalid.result_fingerprint.clear();
    assert_eq!(
        plan_step_import(
            &document.current(),
            b"#1=SI_UNIT(.MILLI.,.METRE.);",
            "part.step",
            &invalid,
        ),
        Err(StepImportPlanError::InvalidWorkerEvidence)
    );
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);
}

#[test]
fn invalid_or_stale_import_leaves_the_published_state_unchanged() {
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let invalid = CommandBatch::new(vec![CanonicalCommand::RecordImport(receipt(
        1, 9, 9, b"invalid",
    ))]);
    let error = match document.apply_batch(&invalid) {
        Ok(_) => panic!("missing import outputs must reject the entire batch"),
        Err(error) => error,
    };
    assert_eq!(error, CanonicalError::InvalidImportReceipt);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);

    let proposal = document
        .prepare_proposal(import_batch(1, 2, 2, b"stale"))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Concurrent".to_owned(),
            },
        ]))
        .unwrap();
    let concurrent = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    assert!(matches!(
        document.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(document.current().canonical_digest(), concurrent);
    assert_eq!(document.visible_undo_steps(), undo_steps);
}
