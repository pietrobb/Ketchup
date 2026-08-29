use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentId, DocumentStore,
    EdgeFinishKind, FeatureId, FeatureKind, Snapshot,
};
use ketchup_core::exact_product::{
    BODY_SUBSHAPE_REF_SCHEMA_V1, BodyResultIdentity, BodySubshapeRef, EXACT_PRODUCT_SCHEMA_V1,
    ExactBodyPackage, ExactFaceRole, ExactResultRegistry, ImportedExactPackage, ReferenceStability,
    canonical_reference_lineage_digest,
};
use ketchup_core::import::{
    ImportLengthUnit, StepImportEvidence, StepImportMesh, StepMeshTriangle, plan_step_import,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{PrincipalPlane, WorkplaneSpec};
use ketchup_core::topology::{
    MAX_GENERATED_TOPOLOGICAL_REFERENCES, TopologicalElementKind, TopologicalElementRef,
    TopologicalReferenceError, TopologicalReferenceQuarantineReason,
    TopologicalReferenceResolution, TopologicalReferenceStability,
    canonical_topological_lineage_digest, publish_generated_topological_references,
    resolve_topological_reference,
};
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const PRODUCER: FeatureId = FeatureId(10);

fn snapshot() -> Snapshot {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Topology fixture".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PRODUCER,
                definition_id: DEFINITION,
                name: "Producer".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
        ]))
        .unwrap();
    document.current()
}

fn generic_reference(snapshot: &Snapshot, kind: TopologicalElementKind) -> TopologicalElementRef {
    TopologicalElementRef::new(
        snapshot.document_id(),
        DEFINITION,
        PRODUCER,
        PRODUCER,
        kind,
        format!("source/{}", kind.token()),
        format!("result/{}", kind.token()),
        TopologicalReferenceStability::Guaranteed,
        "evaluator.v1",
        "backend.v1",
        "1e-7-mm",
        "result-a",
        "geometry-a",
    )
    .unwrap()
}

fn legacy_face(snapshot: &Snapshot) -> BodySubshapeRef {
    let role = ExactFaceRole::Top;
    BodySubshapeRef {
        schema: BODY_SUBSHAPE_REF_SCHEMA_V1.to_owned(),
        document_id: snapshot.document_id(),
        definition_id: DEFINITION,
        profile_feature_id: PRODUCER,
        producer_feature_id: PRODUCER,
        semantic_role: role.semantic_role().to_owned(),
        source_element_id: role.source_element_id().to_owned(),
        expected_type: role.expected_type().to_owned(),
        expected_cardinality: 1,
        stability: ReferenceStability::Guaranteed,
        canonical_input_digest: "canonical".into(),
        exact_input_digest: "exact".into(),
        result_fingerprint: "result".into(),
        evaluator: "evaluator.v1".into(),
        backend: "backend.v1".into(),
        tolerance: "1e-7-mm".into(),
        lineage_digest: canonical_reference_lineage_digest(
            snapshot.document_id(),
            PRODUCER,
            role.semantic_role(),
            role.source_element_id(),
            role.expected_type(),
        ),
        corroborating_geometry_fingerprint: "geometry".into(),
    }
}

#[test]
fn face_edge_and_vertex_share_one_role_neutral_identity_contract() {
    let snapshot = snapshot();
    let references = [
        generic_reference(&snapshot, TopologicalElementKind::Face),
        generic_reference(&snapshot, TopologicalElementKind::Edge),
        generic_reference(&snapshot, TopologicalElementKind::Vertex),
    ];

    for reference in references {
        assert!(reference.has_valid_lineage());
        assert_eq!(
            reference.lineage_digest,
            canonical_topological_lineage_digest(&reference)
        );
        assert!(!reference.producer_element_id.is_empty());
    }
}

#[test]
fn opaque_tokens_are_length_delimited_and_cannot_alias_at_separator_boundaries() {
    let snapshot = snapshot();
    let mut left = generic_reference(&snapshot, TopologicalElementKind::Face);
    left.source_element_id = "a:b".into();
    left.producer_element_id = "c".into();
    left.lineage_digest = canonical_topological_lineage_digest(&left);
    let mut right = generic_reference(&snapshot, TopologicalElementKind::Face);
    right.source_element_id = "a".into();
    right.producer_element_id = "b:c".into();
    right.lineage_digest = canonical_topological_lineage_digest(&right);

    assert!(left.has_valid_lineage());
    assert!(right.has_valid_lineage());
    assert_ne!(left.lineage_digest, right.lineage_digest);
}

#[test]
fn legacy_body_reference_adapts_without_using_an_ordinal_as_identity() {
    let snapshot = snapshot();
    let body = legacy_face(&snapshot);
    let adapted = TopologicalElementRef::from_body_subshape(&body).unwrap();

    assert_eq!(adapted.kind, TopologicalElementKind::Face);
    assert_eq!(adapted.source_feature_id, body.profile_feature_id);
    assert_eq!(adapted.producer_element_id, body.semantic_role);
    assert!(adapted.has_valid_lineage());

    let mut malformed = body;
    malformed.lineage_digest = "forged".into();
    assert_eq!(
        TopologicalElementRef::from_body_subshape(&malformed),
        Err(TopologicalReferenceError::InvalidBodyReference)
    );
}

#[test]
fn recompute_rebinds_by_durable_lineage_not_worker_or_geometry_fingerprint() {
    let snapshot = snapshot();
    let reference = generic_reference(&snapshot, TopologicalElementKind::Edge);
    let mut recomputed = reference.clone();
    recomputed.result_fingerprint = "result-b".into();
    recomputed.corroborating_geometry_fingerprint = "geometry-b".into();

    let resolution = resolve_topological_reference(&snapshot, &reference, [&recomputed]);
    let TopologicalReferenceResolution::Resolved { reference: rebound } = resolution else {
        panic!("one durable candidate must resolve");
    };
    assert_eq!(rebound.lineage_digest, reference.lineage_digest);
    assert_eq!(rebound.result_fingerprint, "result-b");
    assert_eq!(rebound.corroborating_geometry_fingerprint, "geometry-b");
}

#[test]
fn resolver_fails_closed_for_lost_ambiguous_forged_cross_document_and_envelope_changes() {
    let snapshot = snapshot();
    let reference = generic_reference(&snapshot, TopologicalElementKind::Vertex);

    assert_eq!(
        resolve_topological_reference(&snapshot, &reference, []),
        TopologicalReferenceResolution::Lost
    );

    let first = reference.clone();
    let mut second = reference.clone();
    second.result_fingerprint = "other-result".into();
    assert_eq!(
        resolve_topological_reference(&snapshot, &reference, [&first, &second]),
        TopologicalReferenceResolution::Ambiguous { candidate_count: 2 }
    );

    let mut forged = reference.clone();
    forged.lineage_digest = "forged".into();
    assert_eq!(
        resolve_topological_reference(&snapshot, &forged, [&first]),
        TopologicalReferenceResolution::Quarantined {
            reason: TopologicalReferenceQuarantineReason::InvalidLineage
        }
    );

    let mut cross_document = reference.clone();
    cross_document.document_id = DocumentId(snapshot.document_id().0 + 1);
    cross_document.lineage_digest = canonical_topological_lineage_digest(&cross_document);
    assert_eq!(
        resolve_topological_reference(&snapshot, &cross_document, [&first]),
        TopologicalReferenceResolution::Quarantined {
            reason: TopologicalReferenceQuarantineReason::WrongDocument
        }
    );

    let mut incompatible = reference.clone();
    incompatible.backend = "backend.v2".into();
    assert_eq!(
        resolve_topological_reference(&snapshot, &reference, [&incompatible]),
        TopologicalReferenceResolution::Quarantined {
            reason: TopologicalReferenceQuarantineReason::IncompatibleEvaluationEnvelope
        }
    );
}

#[test]
fn ephemeral_identity_never_rebinds_across_result_fingerprints() {
    let snapshot = snapshot();
    let mut reference = generic_reference(&snapshot, TopologicalElementKind::Face);
    reference.stability = TopologicalReferenceStability::Ephemeral;
    reference.lineage_digest = canonical_topological_lineage_digest(&reference);
    let mut recomputed = reference.clone();
    recomputed.result_fingerprint = "result-b".into();

    assert_eq!(
        resolve_topological_reference(&snapshot, &reference, [&recomputed]),
        TopologicalReferenceResolution::Lost
    );
}

#[test]
fn generated_exact_publication_is_complete_deterministic_and_result_bound() {
    let snapshot = snapshot();
    let identity = BodyResultIdentity {
        schema: EXACT_PRODUCT_SCHEMA_V1.into(),
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        definition_id: DEFINITION,
        profile_feature_id: PRODUCER,
        extrusion_feature_id: PRODUCER,
        producer_feature_id: PRODUCER,
        canonical_input_digest: "canonical-a".into(),
        exact_input_digest: "exact-a".into(),
        result_fingerprint: "result-a".into(),
        evaluator: "evaluator.v1".into(),
        backend: "backend.v1".into(),
        tolerance: "1e-7-mm".into(),
    };
    let counts = [4, 5, 3, 1, 1];

    let references = publish_generated_topological_references(&identity, counts).unwrap();
    assert_eq!(
        references,
        publish_generated_topological_references(&identity, counts).unwrap()
    );
    assert_eq!(references.len(), 12);
    assert_eq!(
        references
            .iter()
            .filter(|reference| reference.kind == TopologicalElementKind::Vertex)
            .count(),
        4
    );
    assert_eq!(
        references
            .iter()
            .filter(|reference| reference.kind == TopologicalElementKind::Edge)
            .count(),
        5
    );
    assert_eq!(
        references
            .iter()
            .filter(|reference| reference.kind == TopologicalElementKind::Face)
            .count(),
        3
    );
    assert!(references.iter().all(|reference| {
        reference.has_valid_lineage()
            && reference.source_feature_id == PRODUCER
            && reference.producer_feature_id == PRODUCER
            && reference.stability == TopologicalReferenceStability::Ephemeral
            && reference.source_element_id.starts_with("generated-source/")
            && reference
                .producer_element_id
                .starts_with("generated-result/")
    }));

    let selected = references
        .iter()
        .find(|reference| reference.kind == TopologicalElementKind::Face)
        .unwrap();
    let mut recomputed_identity = identity.clone();
    recomputed_identity.result_fingerprint = "result-b".into();
    let recomputed =
        publish_generated_topological_references(&recomputed_identity, counts).unwrap();
    let candidate = recomputed
        .iter()
        .find(|reference| reference.lineage_digest == selected.lineage_digest)
        .unwrap();
    assert_eq!(
        resolve_topological_reference(&snapshot, selected, [candidate]),
        TopologicalReferenceResolution::Lost
    );
}

#[test]
fn generated_exact_publication_refuses_unbounded_worker_counts() {
    let snapshot = snapshot();
    let identity = BodyResultIdentity {
        schema: EXACT_PRODUCT_SCHEMA_V1.into(),
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        definition_id: DEFINITION,
        profile_feature_id: PRODUCER,
        extrusion_feature_id: PRODUCER,
        producer_feature_id: PRODUCER,
        canonical_input_digest: "canonical".into(),
        exact_input_digest: "exact".into(),
        result_fingerprint: "result".into(),
        evaluator: "evaluator.v1".into(),
        backend: "backend.v1".into(),
        tolerance: "1e-7-mm".into(),
    };

    assert_eq!(
        publish_generated_topological_references(
            &identity,
            [MAX_GENERATED_TOPOLOGICAL_REFERENCES as u32, 1, 1, 1, 1]
        ),
        Err(TopologicalReferenceError::ResourceLimit)
    );
}

#[test]
fn imported_exact_publication_is_source_bound_best_effort_and_backend_guarded() {
    let source = b"generic exact source";
    let evidence = StepImportEvidence {
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "import-result-a".into(),
        solid_count: 1,
        topology_counts: [4, 6, 4, 1, 1],
        volume_mm3: 1.0,
        bounds_mm: [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
        backend: "occt-import.v1".into(),
        tolerance: "1e-7-mm".into(),
    };
    let mut document = DocumentStore::new();
    document
        .apply_batch(
            &plan_step_import(&document.current(), source, "generic.step", &evidence).unwrap(),
        )
        .unwrap();
    let snapshot = document.current();
    let mesh = StepImportMesh {
        vertices_mm: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ],
        triangles: vec![StepMeshTriangle {
            vertex_indices: [0, 1, 2],
            face_ordinal: 0,
        }],
    };
    let imported =
        ImportedExactPackage::from_snapshot(&snapshot, DEFINITION, source.to_vec(), &mesh).unwrap();
    let package = ExactBodyPackage::Imported(imported.clone());
    let references = package.topological_references();

    assert_eq!(references.len(), 14);
    assert_eq!(
        references
            .iter()
            .filter(|reference| reference.kind == TopologicalElementKind::Vertex)
            .count(),
        4
    );
    assert_eq!(
        references
            .iter()
            .filter(|reference| reference.kind == TopologicalElementKind::Edge)
            .count(),
        6
    );
    assert_eq!(
        references
            .iter()
            .filter(|reference| reference.kind == TopologicalElementKind::Face)
            .count(),
        4
    );
    let source_digest = ketchup_core::graph::sha256_hex(source);
    assert!(references.iter().all(|reference| {
        reference.has_valid_lineage()
            && reference.stability == TopologicalReferenceStability::BestEffort
            && reference.source_element_id.contains(&source_digest)
            && reference.evaluator == "ketchup.imported-step-evaluator.v1"
            && reference.backend == evidence.backend
            && reference.tolerance == evidence.tolerance
    }));
    assert_eq!(
        imported.topological_references,
        ImportedExactPackage::from_snapshot(&snapshot, DEFINITION, source.to_vec(), &mesh)
            .unwrap()
            .topological_references
    );

    let selected = references
        .iter()
        .find(|reference| reference.kind == TopologicalElementKind::Face)
        .unwrap();
    let mut incompatible = selected.clone();
    incompatible.backend = "other-backend".into();
    assert_eq!(
        resolve_topological_reference(&snapshot, selected, [&incompatible]),
        TopologicalReferenceResolution::Quarantined {
            reason: TopologicalReferenceQuarantineReason::IncompatibleEvaluationEnvelope
        }
    );
}

#[test]
fn serialized_reference_survives_recompute_undo_redo_and_byte_stable_save_open() {
    let source = b"generic persistent exact source";
    let evidence = StepImportEvidence {
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "persistent-import-result".into(),
        solid_count: 1,
        topology_counts: [4, 6, 4, 1, 1],
        volume_mm3: 1.0,
        bounds_mm: [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
        backend: "occt-import.v1".into(),
        tolerance: "1e-7-mm".into(),
    };
    let mesh = StepImportMesh {
        vertices_mm: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ],
        triangles: vec![StepMeshTriangle {
            vertex_indices: [0, 1, 2],
            face_ordinal: 0,
        }],
    };
    let mut document = DocumentStore::new();
    document
        .apply_batch(
            &plan_step_import(&document.current(), source, "persistent.step", &evidence).unwrap(),
        )
        .unwrap();
    let committed = document.current();
    let imported =
        ImportedExactPackage::from_snapshot(&committed, DEFINITION, source.to_vec(), &mesh)
            .unwrap();
    let selected = imported
        .topological_references
        .iter()
        .find(|reference| reference.kind == TopologicalElementKind::Edge)
        .unwrap()
        .clone();
    let encoded_reference = selected.to_bytes().unwrap();
    let persisted_reference = TopologicalElementRef::from_bytes(&encoded_reference).unwrap();
    assert_eq!(persisted_reference, selected);
    assert_eq!(persisted_reference.to_bytes().unwrap(), encoded_reference);
    let mut forged_encoding = encoded_reference.clone();
    *forged_encoding.last_mut().unwrap() ^= 1;
    assert_eq!(
        TopologicalElementRef::from_bytes(&forged_encoding),
        Err(TopologicalReferenceError::InvalidEncoding)
    );

    let package = ExactBodyPackage::Imported(imported.clone());
    let registry = ExactResultRegistry::accept(&committed, [Arc::new(package.clone())]).unwrap();
    assert!(matches!(
        registry.resolve_topological_reference(&committed, &persisted_reference),
        TopologicalReferenceResolution::Resolved { reference }
            if *reference == persisted_reference
    ));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameDefinition {
                id: DEFINITION,
                name: "Renamed without changing exact evidence".into(),
            },
        ]))
        .unwrap();
    let recomputed = document.current();
    assert_eq!(
        registry.resolve_topological_reference(&recomputed, &persisted_reference),
        TopologicalReferenceResolution::Lost
    );
    let recomputed_registry = ExactResultRegistry::carried_forward(&recomputed, &registry);
    assert!(matches!(
        recomputed_registry.resolve_topological_reference(&recomputed, &persisted_reference),
        TopologicalReferenceResolution::Resolved { reference }
            if *reference == persisted_reference
    ));

    let undone = document.undo().unwrap();
    let undo_registry = ExactResultRegistry::carried_forward(&undone, &recomputed_registry);
    assert!(matches!(
        undo_registry.resolve_topological_reference(&undone, &persisted_reference),
        TopologicalReferenceResolution::Resolved { reference }
            if *reference == persisted_reference
    ));
    let redone = document.redo().unwrap();
    let redo_registry = ExactResultRegistry::carried_forward(&redone, &undo_registry);
    assert!(matches!(
        redo_registry.resolve_topological_reference(&redone, &persisted_reference),
        TopologicalReferenceResolution::Resolved { reference }
            if *reference == persisted_reference
    ));

    let mut container = persistence::ContainerData::default();
    container.insert_import_blob(source.to_vec()).unwrap();
    let document_bytes = persistence::save_container(&redone, &container).unwrap();
    let reopened = persistence::load(&document_bytes).unwrap();
    let reopened_snapshot = reopened.snapshot();
    assert_eq!(
        persistence::save_container(&reopened_snapshot, reopened.container_data()).unwrap(),
        document_bytes
    );
    let reopened_registry =
        ExactResultRegistry::carried_forward(&reopened_snapshot, &redo_registry);
    assert!(matches!(
        reopened_registry.resolve_topological_reference(&reopened_snapshot, &persisted_reference),
        TopologicalReferenceResolution::Resolved { reference }
            if *reference == persisted_reference
    ));

    assert_eq!(
        ExactResultRegistry::default()
            .resolve_topological_reference(&committed, &persisted_reference),
        TopologicalReferenceResolution::Lost
    );
    let mut duplicated = imported;
    duplicated
        .topological_references
        .push(persisted_reference.clone());
    let ambiguous_registry = ExactResultRegistry::accept(
        &committed,
        [Arc::new(ExactBodyPackage::Imported(duplicated))],
    )
    .unwrap();
    assert_eq!(
        ambiguous_registry.resolve_topological_reference(&committed, &persisted_reference),
        TopologicalReferenceResolution::Ambiguous { candidate_count: 2 }
    );
}

fn topology_feature_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Topology feature fixture".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(9),
                definition_id: DEFINITION,
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 30.0], [0.0, 30.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PRODUCER,
                definition_id: DEFINITION,
                name: "Extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(9),
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
        ]))
        .unwrap();
    document
}

fn feature_reference(
    snapshot: &Snapshot,
    producer: FeatureId,
    kind: TopologicalElementKind,
    ordinal: u32,
) -> TopologicalElementRef {
    TopologicalElementRef::new(
        snapshot.document_id(),
        DEFINITION,
        producer,
        producer,
        kind,
        format!("generated-source/{}/{ordinal}", kind.token()),
        format!("generated-result/{}/{ordinal}", kind.token()),
        TopologicalReferenceStability::Guaranteed,
        "ketchup.exact-brep-graph-evaluator.v1",
        "occt.v1",
        "1e-7-mm",
        format!("result-{}", producer.0),
        format!("geometry-{}-{ordinal}", kind.token()),
    )
    .unwrap()
}

#[test]
fn topology_driven_finish_features_are_canonical_fail_closed_and_losslessly_persistent() {
    let mut document = topology_feature_document();
    let face = feature_reference(
        &document.current(),
        PRODUCER,
        TopologicalElementKind::Face,
        2,
    );
    let before_shell = document.current();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(11),
            definition_id: DEFINITION,
            name: "Topology shell".into(),
            kind: FeatureKind::TopologyShell {
                target: PRODUCER,
                removed_faces: vec![face.clone()],
                thickness: Dimension::from_decimal("2.5").unwrap(),
            },
        }]))
        .unwrap();
    assert_eq!(
        document.current().revision_id(),
        before_shell.revision_id() + 1
    );
    assert_ne!(
        document.current().canonical_digest(),
        before_shell.canonical_digest()
    );

    let edge = feature_reference(
        &document.current(),
        FeatureId(11),
        TopologicalElementKind::Edge,
        4,
    );
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(12),
            definition_id: DEFINITION,
            name: "Topology fillet".into(),
            kind: FeatureKind::TopologyEdgeFinish {
                target: FeatureId(11),
                edges: vec![edge.clone()],
                kind: EdgeFinishKind::Fillet,
                amount: Dimension::from_decimal("1.25").unwrap(),
            },
        }]))
        .unwrap();

    let committed = document.current();
    let bytes = persistence::save(&committed);
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), committed.canonical_digest());
    assert_eq!(persistence::save(&reopened), bytes);
    assert!(matches!(
        reopened.feature(FeatureId(11)).unwrap().kind(),
        FeatureKind::TopologyShell { removed_faces, .. }
            if removed_faces == std::slice::from_ref(&face)
    ));
    assert!(matches!(
        reopened.feature(FeatureId(12)).unwrap().kind(),
        FeatureKind::TopologyEdgeFinish { edges, kind: EdgeFinishKind::Fillet, .. }
            if edges == &[edge]
    ));

    let mut rejected = topology_feature_document();
    let rejected_before = rejected.current();
    let wrong_kind = feature_reference(&rejected_before, PRODUCER, TopologicalElementKind::Edge, 2);
    assert!(
        rejected
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id: DEFINITION,
                name: "Invalid shell".into(),
                kind: FeatureKind::TopologyShell {
                    target: PRODUCER,
                    removed_faces: vec![wrong_kind],
                    thickness: Dimension::from_decimal("2.5").unwrap(),
                },
            }]))
            .is_err()
    );
    assert_eq!(
        rejected.current().revision_id(),
        rejected_before.revision_id()
    );
    assert_eq!(
        rejected.current().canonical_digest(),
        rejected_before.canonical_digest()
    );

    let mut cross_document = face;
    cross_document.document_id = DocumentId(cross_document.document_id.0 + 1);
    cross_document.lineage_digest = canonical_topological_lineage_digest(&cross_document);
    assert!(
        rejected
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id: DEFINITION,
                name: "Cross-document shell".into(),
                kind: FeatureKind::TopologyShell {
                    target: PRODUCER,
                    removed_faces: vec![cross_document],
                    thickness: Dimension::from_decimal("2.5").unwrap(),
                },
            }]))
            .is_err()
    );
    assert_eq!(
        rejected.current().canonical_digest(),
        rejected_before.canonical_digest()
    );
}
