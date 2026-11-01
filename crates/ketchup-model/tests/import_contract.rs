use ketchup_model::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, DocumentStore, FeatureKind,
    OccurrenceId, ProposalCommitError, Transform,
};
use ketchup_model::import::{
    IgesImportEvidence, IgesXdeImportEvidence, IgesXdeNodeEvidence, IgesXdePartEvidence,
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportId, ImportLengthUnit,
    ImportOutputRef, ImportReceipt, ImportRejectedPart, ImportRejectedShape, ImportUnitAuthority,
    ImportUnitDecision, StepImportEvidence, StepImportPlanError, StepXdeImportEvidence,
    StepXdeNodeEvidence, StepXdePartEvidence, plan_iges_import, plan_iges_xde_import,
    plan_step_import, plan_step_xde_import,
};
use ketchup_model::persistence;

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
            tags: Default::default(),
            visible: true,
        },
        CanonicalCommand::RecordImport(receipt(id, definition_id, occurrence_id, source)),
    ])
}

#[test]
fn a_receipt_of_any_format_takes_any_nonzero_source_length() {
    let receipt = |format, source_byte_len| {
        ImportReceipt::new(
            ImportId(1),
            format,
            [7; 32],
            source_byte_len,
            "part",
            ImportUnitDecision::new(
                ImportLengthUnit::Millimetre,
                ImportUnitAuthority::UserDeclared,
            ),
            "reader",
            "1",
            Vec::new(),
            vec![ImportOutputRef::Definition(DefinitionId(1))],
        )
    };
    // No format has a source size limit.
    for format in [
        ImportFormat::Stl,
        ImportFormat::Dxf,
        ImportFormat::Step,
        ImportFormat::Iges,
        ImportFormat::SketchupScene,
        ImportFormat::Glb,
    ] {
        assert!(receipt(format, u64::MAX).is_ok(), "{format:?}");
        assert!(receipt(format, 0).is_err());
    }
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
    assert_eq!(reopened.source_schema(), persistence::OLDEST_WRITTEN_SCHEMA);
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
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "0123456789abcdef".to_owned(),
        body_kind: ketchup_model::document::BodyKind::Solid,
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
            .map(|bytes| bytes.as_slice()),
        Some(source.as_slice())
    );
    assert_eq!(
        persistence::save_container(&reopened.snapshot(), reopened.container_data()).unwrap(),
        encoded
    );
}

#[test]
fn exact_step_import_rejects_worker_evidence_from_another_source() {
    let source = b"first STEP source";
    let evidence = StepImportEvidence {
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "fnv1a64:0123456789abcdef".to_owned(),
        body_kind: ketchup_model::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 600.0,
        volume_mm3: 1_000.0,
        bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        backend: "occt-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let document = DocumentStore::new();
    let before = document.current().canonical_digest();
    assert!(plan_step_import(&document.current(), source, "first.step", &evidence).is_ok());
    assert_eq!(
        plan_step_import(
            &document.current(),
            b"other STEP source",
            "second.step",
            &evidence,
        ),
        Err(ketchup_model::import::StepImportPlanError::InvalidWorkerEvidence)
    );
    let mut stale_length = evidence.clone();
    stale_length.source_byte_len += 1;
    assert_eq!(
        plan_step_import(&document.current(), source, "first.step", &stale_length),
        Err(ketchup_model::import::StepImportPlanError::InvalidWorkerEvidence)
    );
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);
}

/// ADR 0010: a part saved where libm rounded its volume one bit away (the
/// Hettich rail 9117663_3 of the drawer example, MSVC against CORE-MATH) is
/// the same body; a measure or a topology further off is another one.
#[test]
fn imported_evidence_a_few_last_bits_away_is_the_same_body() {
    use ketchup_model::import::{IMPORTED_MEASURE_ULPS, ulps_from};
    let recorded = StepImportEvidence {
        source_sha256: [7; 32],
        source_byte_len: 281_205,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "fnv1a64:6b9e8a10cebef52a".to_owned(),
        body_kind: ketchup_model::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [140, 212, 74, 1, 1],
        area_mm2: 70_410.298_543_028_37,
        volume_mm3: 74_890.358_324_392_06,
        bounds_mm: [
            [518.25, -31.0, -40.75],
            [562.000_000_1, 387.6, 9.050_000_000_000_004],
        ],
        backend: "occt-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let mut evaluated = recorded.clone();
    evaluated.result_fingerprint = "fnv1a64:7bb317a07566e095".to_owned();
    evaluated.volume_mm3 = 74_890.358_324_392_07;
    assert!(evaluated.is_same_body_as(&recorded));
    evaluated.bounds_mm[0][1] = ulps_from(-31.0, 1).unwrap();
    assert!(evaluated.is_same_body_as(&recorded));

    let further = IMPORTED_MEASURE_ULPS as i64 + 1;
    let mut volume = evaluated.clone();
    volume.volume_mm3 = ulps_from(recorded.volume_mm3, further).unwrap();
    let mut area = evaluated.clone();
    area.area_mm2 = ulps_from(recorded.area_mm2, -further).unwrap();
    let mut bounds = evaluated.clone();
    bounds.bounds_mm[1][2] = ulps_from(recorded.bounds_mm[1][2], 2).unwrap();
    let mut topology = evaluated.clone();
    topology.topology_counts[2] += 1;
    let mut backend = evaluated.clone();
    backend.backend = "occt-other".to_owned();
    for other in [volume, area, bounds, topology, backend] {
        assert!(!other.is_same_body_as(&recorded), "{other:?}");
    }
    assert_eq!(ulps_from(0.0, -1).map(f64::is_sign_negative), Some(true));
    assert_eq!(ulps_from(f64::MAX, 1), None);
}

/// Review 2026-10-29 EX-60: `ulps_from(-0.0, 0)` gave +0.0, so a body bounded
/// at -0.0 that the window takes for the recorded one (`within_ulps`) was
/// refused by the worker, which searches the recorded fingerprint by
/// `ulps_from` and never reached the other zero. Every value one is within
/// reach of, the other reaches, bit for bit.
#[test]
fn every_value_within_ulps_is_one_ulps_from_reaches() {
    use ketchup_model::import::{ulps_from, within_ulps};
    let tiny = f64::from_bits(1);
    let values = [-1.0, -2.0 * tiny, -tiny, -0.0, 0.0, tiny, 2.0 * tiny, 1.0];
    assert_eq!(
        ulps_from(-0.0, 0).map(f64::to_bits),
        Some((-0.0_f64).to_bits())
    );
    for a in values {
        for b in values {
            for ulps in 0..=2_u64 {
                let reach = ulps.cast_signed();
                let reached = (-reach..=reach)
                    .any(|step| ulps_from(b, step).map(f64::to_bits) == Some(a.to_bits()));
                assert_eq!(
                    within_ulps(a, b, ulps),
                    reached,
                    "{a:e} from {b:e} in {ulps}"
                );
            }
        }
    }
}

#[test]
fn exact_iges_import_is_deterministic_persistent_and_explicit_about_losses() {
    let source = b"IGES exact source";
    let evidence = IgesImportEvidence {
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "fnv1a64:0123456789abcdef".to_owned(),
        body_kind: ketchup_model::document::BodyKind::Solid,
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
        Err(ketchup_model::import::IgesImportPlanError::InvalidWorkerEvidence)
    );
    let mut stale_length = evidence.clone();
    stale_length.source_byte_len += 1;
    assert_eq!(
        plan_iges_import(&document.current(), source, "part.iges", &stale_length),
        Err(ketchup_model::import::IgesImportPlanError::InvalidWorkerEvidence)
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
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: format!("fnv1a64:iges-root-{index}"),
        body_kind: ketchup_model::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 600.0,
        volume_mm3: 1_000.0,
        bounds_mm: [[x, 0.0, 0.0], [x + 10.0, 10.0, 10.0]],
        backend: "occt-iges-xde-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let evidence = IgesXdeImportEvidence {
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        parts: vec![
            IgesXdePartEvidence {
                index: 0,
                name: "Left body".to_owned(),
                name_from_source: true,
                color: Some([255, 0, 0]),
                exact: exact(0, 0.0),
                rejected_shapes: Vec::new(),
            },
            IgesXdePartEvidence {
                index: 1,
                name: "Right body".to_owned(),
                name_from_source: true,
                color: Some([0, 255, 0]),
                exact: exact(1, 40.0),
                rejected_shapes: Vec::new(),
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
        rejected_parts: Vec::new(),
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
    let semantic = ketchup_model::state_view::encode_semantic_state(&committed);
    assert!(
        semantic
            .complete()
            .contains("import_receipts.1.format=\"Iges\"")
    );
    assert_eq!(
        semantic
            .complete()
            .matches(".kind.ImportedExactBody.import_id=1\n")
            .count(),
        2
    );
    assert!(semantic.complete().contains(".source_part_index=0"));
    assert!(semantic.complete().contains(".source_part_index=1"));
    assert!(
        semantic
            .agent()
            .contains("code:\"iges_parametric_reconstruction_unavailable\",subject:none,count:2")
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
        Err(ketchup_model::import::IgesImportPlanError::InvalidWorkerEvidence)
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
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "fnv1a64:part-0".to_owned(),
        body_kind: ketchup_model::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 600.0,
        volume_mm3: 1_000.0,
        bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        backend: "occt-xde-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let evidence = StepXdeImportEvidence {
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        parts: vec![StepXdePartEvidence {
            index: 0,
            name: "Bracket".to_owned(),
            name_from_source: true,
            color: Some([0, 255, 0]),
            exact,
            rejected_shapes: Vec::new(),
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
        rejected_parts: Vec::new(),
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
    assert_eq!(reopened.source_schema(), persistence::OLDEST_WRITTEN_SCHEMA);
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

// Ploopy STEP fix B: a part the worker could not read is left out with the
// nodes placing it, the others imported under their source indices, and the
// part and every solid left out of a part read are named in the receipt.
#[test]
fn step_and_iges_xde_imports_keep_the_parts_read_and_name_the_refused() {
    let source = b"independent XDE source with one unreadable part";
    let exact = |index: u32| StepImportEvidence {
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: format!("fnv1a64:part-{index}"),
        body_kind: ketchup_model::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 600.0,
        volume_mm3: 1_000.0,
        bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        backend: "occt-xde-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let left_out = ImportRejectedShape {
        ordinal: 1,
        kind: "solid".to_owned(),
        source_entity: Some(720),
        reason: "encloses no volume".to_owned(),
    };
    let refused = ImportRejectedPart {
        index: 1,
        name: "Crossed prism".to_owned(),
        reason: "solid 1 (entity #1069) encloses no volume".to_owned(),
    };
    let node = |id: u32, parent_id, part_index, name: &str| StepXdeNodeEvidence {
        id,
        parent_id,
        part_index,
        name: name.to_owned(),
        name_from_source: true,
        color: None,
        transform: Transform::identity(),
    };
    let part =
        |index: u32, name: &str, rejected_shapes: Vec<ImportRejectedShape>| StepXdePartEvidence {
            index,
            name: name.to_owned(),
            name_from_source: true,
            color: None,
            exact: exact(index),
            rejected_shapes,
        };
    let evidence = StepXdeImportEvidence {
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        parts: vec![
            part(0, "Block", Vec::new()),
            part(2, "Pair", vec![left_out.clone()]),
        ],
        nodes: vec![
            node(0, None, None, "Assembly"),
            node(1, Some(0), Some(0), "Block 1"),
            node(2, Some(0), Some(1), "Crossed prism 1"),
            node(3, Some(0), Some(2), "Pair 1"),
        ],
        rejected_parts: vec![refused.clone()],
    };
    let mut document = DocumentStore::new();
    let batch =
        plan_step_xde_import(&document.current(), source, "partly.step", &evidence).unwrap();
    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(
        committed
            .occurrences()
            .map(|occurrence| occurrence.name().to_owned())
            .collect::<Vec<_>>(),
        ["Block 1", "Pair 1"]
    );
    assert_eq!(
        committed
            .features()
            .map(|feature| match feature.kind() {
                FeatureKind::ImportedExactBody(spec) => spec.source_part_index,
                _ => panic!("XDE part must remain exact"),
            })
            .collect::<Vec<_>>(),
        [Some(0), Some(2)]
    );
    let refusals = |receipt: &ImportReceipt| {
        receipt
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().ends_with("_refused"))
            .map(|diagnostic| {
                (
                    diagnostic.code().to_owned(),
                    diagnostic.subject().unwrap().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        refusals(committed.import_receipt(ImportId(1)).unwrap()),
        [
            (
                "step_part_refused".to_owned(),
                "Crossed prism: solid 1 (entity #1069) encloses no volume".to_owned()
            ),
            (
                "step_shape_refused".to_owned(),
                "Pair: solid 2 (entity #720) encloses no volume".to_owned()
            ),
        ]
    );
    // An index both read and refused, or a node placing an index neither
    // read nor refused, is evidence the plan cannot trust.
    for tamper in [
        |evidence: &mut StepXdeImportEvidence| evidence.rejected_parts[0].index = 2,
        |evidence: &mut StepXdeImportEvidence| evidence.nodes[2].part_index = Some(3),
        |evidence: &mut StepXdeImportEvidence| evidence.parts.swap(0, 1),
    ] {
        let mut tampered = evidence.clone();
        tamper(&mut tampered);
        assert_eq!(
            plan_step_xde_import(
                &DocumentStore::new().current(),
                source,
                "partly.step",
                &tampered
            ),
            Err(StepImportPlanError::InvalidWorkerEvidence)
        );
    }

    let iges = IgesXdeImportEvidence {
        source_sha256: evidence.source_sha256,
        source_byte_len: evidence.source_byte_len,
        parts: vec![IgesXdePartEvidence {
            index: 0,
            name: "Block".to_owned(),
            name_from_source: true,
            color: None,
            exact: exact(0),
            rejected_shapes: vec![left_out],
        }],
        nodes: [(0, "Block 1"), (1, "Crossed prism 1")]
            .into_iter()
            .map(|(index, name)| IgesXdeNodeEvidence {
                id: index,
                part_index: index,
                name: name.to_owned(),
                name_from_source: true,
                color: None,
                transform: Transform::identity(),
            })
            .collect(),
        rejected_parts: vec![refused],
    };
    let mut document = DocumentStore::new();
    let batch = plan_iges_xde_import(&document.current(), source, "partly.iges", &iges).unwrap();
    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(committed.occurrences().count(), 1);
    assert_eq!(
        refusals(committed.import_receipt(ImportId(1)).unwrap()),
        [
            (
                "iges_part_refused".to_owned(),
                "Crossed prism: solid 1 (entity #1069) encloses no volume".to_owned()
            ),
            (
                "iges_shape_refused".to_owned(),
                "Block: solid 2 (entity #720) encloses no volume".to_owned()
            ),
        ]
    );
}

#[test]
fn exact_step_plan_refuses_invalid_worker_evidence_without_mutation() {
    let source = b"#1=SI_UNIT(.MILLI.,.METRE.);";
    let document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let valid = StepImportEvidence {
        source_sha256: ketchup_model::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "0123456789abcdef".to_owned(),
        body_kind: ketchup_model::document::BodyKind::Solid,
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
        plan_step_import(&document.current(), source, "part.step", &invalid,),
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
    assert_eq!(error, CanonicalError::DefinitionNotFound(DefinitionId(9)));
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
