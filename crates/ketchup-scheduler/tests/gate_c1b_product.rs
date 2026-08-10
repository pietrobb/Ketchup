use ketchup_core::bottle_m6::ExactRevolveRequest;
use ketchup_core::document::{
    BooleanOperation, BottleControlDimension, BottleEdgeFinishKind, CanonicalCommand,
    CanonicalError, CommandBatch, DefinitionId, DerivedIdentity, Dimension, DimensionDisplayUnit,
    DimensionPresentation, DimensionReferenceHealth, DocumentId, DocumentStore, EvaluationIdentity,
    FeatureId, FeatureKind, FeatureParameterBinding, FeatureParameterFreshness,
    FeatureParameterSlot, FeatureParameterStaleReason, FeatureParameterTarget, NodeId,
    OccurrenceId, PersistentDimension, PersistentDimensionId, PersistentDimensionTarget, PortSpec,
    RuleOutput, SlotPath, SlotSegment, Transform,
};
use ketchup_core::exact_product::{
    EXACT_BOOLEAN_UNION_EVALUATOR_V1, EXACT_THROUGH_CUT_EVALUATOR_V1, ExactBodyPackage,
    ExactFaceRole, ExactFeatureChainRequest, ExactProductError, ExactReferenceQuarantineReason,
    ExactReferenceResolution, ExactResultRegistry, canonical_reference_lineage_digest,
};
use ketchup_exact::{
    ExactBackend, RectangleExtrudeSpec, ReferenceResolution, StabilityClass,
    capture_guaranteed_references, resolve_subshape_reference,
};
use ketchup_interaction::exact_projection::ExactInteractionProjection;
use ketchup_interaction::{Ray, Vec3};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::collections::BTreeMap;
use std::sync::Arc;

const PROFILE: FeatureId = FeatureId(11);
const EXTRUSION: FeatureId = FeatureId(12);
const DEFINITION: DefinitionId = DefinitionId(10);
const CUT_PROFILE: FeatureId = FeatureId(14);
const TOOL_EXTRUSION: FeatureId = FeatureId(15);
const THROUGH_CUT: FeatureId = FeatureId(16);
const BOTTLE_DEFINITION: DefinitionId = DefinitionId(30);
const BOTTLE_PROFILE: FeatureId = FeatureId(31);
const BOTTLE_REVOLVE: FeatureId = FeatureId(32);
const BOTTLE_OCCURRENCE: OccurrenceId = OccurrenceId(33);
const BOTTLE_SHELL: FeatureId = FeatureId(34);
const BOTTLE_CONTROL: FeatureId = FeatureId(35);
const BOTTLE_FINISH: FeatureId = FeatureId(36);

#[test]
fn preregistered_c1b_product_corpus_has_zero_wrong_identities_and_survives_save_open() {
    let corpus = include_str!("fixtures/c1b/rectangle-v1.tsv");
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut observed_cases = 0;
    let mut observed_roles = 0;

    for line in corpus
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
    {
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 4, "malformed preregistered C1b row: {line}");
        let case_id = fields[0];
        let width = fields[1].parse::<f64>().unwrap();
        let depth = fields[2].parse::<f64>().unwrap();
        let height = fields[3].parse::<f64>().unwrap();
        let mut document = rectangle_document(width, depth, height);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        let package = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(package.vertices.len(), 8);
        assert_eq!(package.triangles.len(), 12);

        let direct = ExactBackend::new()
            .extrude_rectangle(RectangleExtrudeSpec {
                width_mm: width,
                depth_mm: depth,
                height_mm: height,
            })
            .unwrap();
        let direct_references = capture_guaranteed_references(
            &direct,
            &snapshot.document_id().0.to_string(),
            &EXTRUSION.0.to_string(),
        )
        .unwrap();
        let results =
            ExactResultRegistry::accept(&snapshot, [Arc::new(package.clone().into())]).unwrap();
        let projection = ExactInteractionProjection::from_snapshot(&snapshot, &results);
        assert_eq!(projection.occurrence_count(), 1);

        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
        ] {
            let hit = projection
                .exact_pick(ray_for(role, width, depth, height))
                .unwrap_or_else(|| panic!("{case_id}: exact pick missed {role:?}"));
            assert_eq!(hit.target.body.role(), Some(role), "{case_id}");
            assert!(hit.target.body.has_valid_lineage(), "{case_id}");
            let direct_reference = direct_references
                .iter()
                .find(|reference| reference.semantic_role == role.semantic_role())
                .unwrap();
            assert_eq!(
                hit.target.body.document_id.0.to_string(),
                direct_reference.document_id,
                "{case_id}: document provenance differs for {role:?}"
            );
            assert_eq!(
                hit.target.body.producer_feature_id.0.to_string(),
                direct_reference.producer_feature_id,
                "{case_id}: producer provenance differs for {role:?}"
            );
            assert_eq!(
                hit.target.body.semantic_role,
                direct_reference.semantic_role
            );
            assert_eq!(
                hit.target.body.source_element_id,
                direct_reference.source_element_id
            );
            assert_eq!(
                hit.target.body.expected_type,
                direct_reference.expected_type
            );
            assert_eq!(direct_reference.stability_class, StabilityClass::Guaranteed);
            assert_eq!(
                hit.target.body.backend, direct_reference.backend_fingerprint,
                "{case_id}: backend provenance differs for {role:?}"
            );
            assert_eq!(
                hit.target.body.lineage_digest, direct_reference.lineage_digest,
                "{case_id}: canonical lineage differs for {role:?}"
            );
            let ReferenceResolution::Resolved { face_ordinal, .. } =
                resolve_subshape_reference(direct_reference, &direct)
            else {
                panic!("{case_id}: direct resolver did not resolve {role:?}");
            };
            let direct_face = direct
                .body
                .topology
                .faces
                .iter()
                .find(|face| face.ordinal == face_ordinal)
                .unwrap();
            assert_eq!(
                hit.target.body.corroborating_geometry_fingerprint,
                direct_face.geometric_fingerprint,
                "{case_id}: interaction and direct resolver disagree for {role:?}"
            );
            observed_roles += 1;
        }

        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        for reference in package.references.clone() {
            document
                .register_exact_reference_evidence(reference)
                .unwrap();
        }
        assert_eq!(document.current().revision_id(), before_revision);
        assert_eq!(document.current().canonical_digest(), before_digest);
        assert_eq!(document.visible_undo_steps(), before_undo);

        let bytes = ketchup_core::persistence::save(&document.current());
        let reopened = match ketchup_core::persistence::load(&bytes).unwrap() {
            ketchup_core::persistence::LoadOutcome::Editable { document, .. } => document,
            ketchup_core::persistence::LoadOutcome::ReviewOnly(_) => {
                panic!("{case_id}: current exact reference evidence must open editable")
            }
        };
        assert_eq!(reopened.current().canonical_digest(), before_digest);
        let mut reopened_references = reopened
            .current()
            .exact_reference_evidence()
            .cloned()
            .collect::<Vec<_>>();
        let mut expected_references = package.references.clone();
        reopened_references.sort_by(|left, right| left.lineage_digest.cmp(&right.lineage_digest));
        expected_references.sort_by(|left, right| left.lineage_digest.cmp(&right.lineage_digest));
        assert_eq!(reopened_references, expected_references, "{case_id}");
        observed_cases += 1;
    }

    assert_eq!(observed_cases, 9);
    assert_eq!(observed_roles, 27);
}

#[test]
fn exact_reference_health_is_explicit_across_transform_mutation_conflict_and_quarantine() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = rectangle_document(100.0, 60.0, 18.0);
    let initial = document.current();
    let initial_request = ExactFeatureChainRequest::from_snapshot(&initial, DEFINITION).unwrap();
    let initial_package = supervisor.evaluate_rectangle(&initial_request).unwrap();
    let durable_top = initial_package
        .reference(ExactFaceRole::Top)
        .unwrap()
        .clone();
    let initial_registry =
        ExactResultRegistry::accept(&initial, [Arc::new(initial_package.into())]).unwrap();
    assert!(matches!(
        initial_registry.resolve_reference(&initial, &durable_top),
        ExactReferenceResolution::Resolved { reference }
            if reference.lineage_digest == durable_top.lineage_digest
    ));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(13),
                transform: Transform::from_translation(25.0, -10.0, 5.0).unwrap(),
            },
        ]))
        .unwrap();
    let transformed = document.current();
    let transformed_request =
        ExactFeatureChainRequest::from_snapshot(&transformed, DEFINITION).unwrap();
    let transformed_package = supervisor.evaluate_rectangle(&transformed_request).unwrap();
    let transformed_top = transformed_package
        .reference(ExactFaceRole::Top)
        .unwrap()
        .clone();
    assert_ne!(
        durable_top.canonical_input_digest,
        transformed_top.canonical_input_digest
    );
    assert_eq!(durable_top.lineage_digest, transformed_top.lineage_digest);
    let transformed_registry =
        ExactResultRegistry::accept(&transformed, [Arc::new(transformed_package.clone().into())])
            .unwrap();
    assert_eq!(
        transformed_registry.resolve_reference(&transformed, &durable_top),
        ExactReferenceResolution::Resolved {
            reference: Box::new(transformed_top.clone()),
        }
    );

    let mut cut_document = boolean_document(
        100.0,
        60.0,
        18.0,
        [30.0, 20.0, 20.0, 15.0],
        BooleanOperation::Cut,
    );
    let cut_snapshot = cut_document.current();
    let cut_request = ExactFeatureChainRequest::from_snapshot(&cut_snapshot, DEFINITION).unwrap();
    let cut_package = supervisor.evaluate_rectangle(&cut_request).unwrap();
    let removed_wall = cut_package
        .reference(ExactFaceRole::CutWest)
        .unwrap()
        .clone();
    cut_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature { id: THROUGH_CUT },
            CanonicalCommand::CreateFeature {
                id: THROUGH_CUT,
                definition_id: DEFINITION,
                name: "Bounded Boolean union".to_owned(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Union,
                    target: EXTRUSION,
                    tool: TOOL_EXTRUSION,
                },
            },
        ]))
        .unwrap();
    let union_snapshot = cut_document.current();
    let union_request =
        ExactFeatureChainRequest::from_snapshot(&union_snapshot, DEFINITION).unwrap();
    let union_package = supervisor.evaluate_rectangle(&union_request).unwrap();
    let union_registry =
        ExactResultRegistry::accept(&union_snapshot, [Arc::new(union_package.into())]).unwrap();
    assert_eq!(
        union_registry.resolve_reference(&union_snapshot, &removed_wall),
        ExactReferenceResolution::Lost
    );

    let mut incompatible_package = transformed_package.clone();
    incompatible_package
        .identity
        .backend
        .push_str("-incompatible");
    for reference in &mut incompatible_package.references {
        reference.backend = incompatible_package.identity.backend.clone();
    }
    let incompatible_registry = ExactResultRegistry::accept(
        &transformed,
        [Arc::new(incompatible_package.clone().into())],
    )
    .unwrap();
    assert_eq!(
        incompatible_registry.resolve_reference(&transformed, &durable_top),
        ExactReferenceResolution::Quarantined {
            reason: ExactReferenceQuarantineReason::IncompatibleEvaluationEnvelope,
        }
    );

    let ambiguous_registry = ExactResultRegistry::accept(
        &transformed,
        [
            Arc::new(transformed_package.into()),
            Arc::new(incompatible_package.into()),
        ],
    )
    .unwrap();
    assert_eq!(
        ambiguous_registry.resolve_reference(&transformed, &durable_top),
        ExactReferenceResolution::Ambiguous { candidate_count: 2 }
    );

    let mut invalid = durable_top.clone();
    invalid.lineage_digest.push_str("-tampered");
    assert_eq!(
        transformed_registry.resolve_reference(&transformed, &invalid),
        ExactReferenceResolution::Quarantined {
            reason: ExactReferenceQuarantineReason::InvalidLineage,
        }
    );

    let mut foreign = durable_top;
    foreign.document_id = DocumentId(999);
    foreign.lineage_digest = canonical_reference_lineage_digest(
        foreign.document_id,
        foreign.producer_feature_id,
        &foreign.semantic_role,
        &foreign.source_element_id,
        &foreign.expected_type,
    );
    assert_eq!(
        transformed_registry.resolve_reference(&transformed, &foreign),
        ExactReferenceResolution::Quarantined {
            reason: ExactReferenceQuarantineReason::WrongDocument,
        }
    );
}

#[test]
fn persistent_exact_dimension_resolves_only_against_current_registered_semantic_reference() {
    const EXACT_HEIGHT: PersistentDimensionId = PersistentDimensionId(1);
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = rectangle_document(100.0, 60.0, 18.0);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertPersistentDimension(
                PersistentDimension::new(
                    EXACT_HEIGHT,
                    "Exact height",
                    PersistentDimensionTarget::ExactFeatureParameter {
                        definition_id: DEFINITION,
                        producer_feature_id: EXTRUSION,
                        semantic_role: ExactFaceRole::Top.semantic_role().to_owned(),
                        source_element_id: ExactFaceRole::Top.source_element_id().to_owned(),
                        slot: FeatureParameterSlot::Height,
                    },
                    DimensionPresentation::new(DimensionDisplayUnit::Millimetres, 2).unwrap(),
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    assert_eq!(
        document
            .current()
            .project_persistent_dimension(EXACT_HEIGHT)
            .unwrap()
            .health,
        DimensionReferenceHealth::Lost
    );

    let request = ExactFeatureChainRequest::from_snapshot(&document.current(), DEFINITION).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    for reference in package.references {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    let resolved = document
        .current()
        .project_persistent_dimension(EXACT_HEIGHT)
        .unwrap();
    assert_eq!(resolved.health, DimensionReferenceHealth::Resolved);
    assert_eq!(resolved.millimetres, Some(18.0));
    assert_eq!(resolved.display_text.as_deref(), Some("18.00 mm"));

    let reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&document.current()))
            .unwrap();
    assert_eq!(reopened.source_schema(), 17);
    assert_eq!(
        reopened
            .snapshot()
            .project_persistent_dimension(EXACT_HEIGHT)
            .unwrap()
            .health,
        DimensionReferenceHealth::Resolved
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("20", 20.0).unwrap(),
            },
        ]))
        .unwrap();
    let unresolved = document
        .current()
        .project_persistent_dimension(EXACT_HEIGHT)
        .unwrap();
    assert_eq!(unresolved.health, DimensionReferenceHealth::Lost);
    assert_eq!(unresolved.millimetres, None);
    assert!(matches!(
        document
            .current()
            .persistent_dimension(EXACT_HEIGHT)
            .unwrap()
            .target,
        PersistentDimensionTarget::ExactFeatureParameter {
            definition_id: DEFINITION,
            producer_feature_id: EXTRUSION,
            ..
        }
    ));
}

#[test]
fn explicit_parameter_recompute_restores_exact_registry_render_pick_and_export() {
    const SOURCE: NodeId = NodeId(201);
    const RULE: NodeId = NodeId(202);
    let identity = EvaluationIdentity::default();
    let segment = SlotSegment::new(RULE, "dimensions", "extrusion_height").unwrap();
    let target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
    };
    let mut document = rectangle_document(100.0, 60.0, 18.0);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: SOURCE,
                name: "Height source".to_owned(),
                dimension: Dimension::new("21", 21.0).unwrap(),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: RULE,
                name: "Extrusion height rule".to_owned(),
                expression: "$201 * 2".to_owned(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("dimensions").unwrap()],
                outputs: vec![RuleOutput::new(segment.clone(), vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target,
                derived_from: DerivedIdentity::new(RULE, SlotPath::new(vec![segment]).unwrap())
                    .unwrap(),
            }),
            CanonicalCommand::RecomputeFeatureParameters {
                identity: identity.clone(),
            },
        ]))
        .unwrap();
    let initial = document.current();
    assert_eq!(
        initial
            .audit_feature_parameter_freshness(&identity)
            .unwrap()[0]
            .freshness,
        FeatureParameterFreshness::Current
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let initial_request = ExactFeatureChainRequest::from_snapshot(&initial, DEFINITION).unwrap();
    let initial_package = supervisor.evaluate_rectangle(&initial_request).unwrap();
    assert_eq!(initial_package.bounds_mm[1][2], 42.0);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: SOURCE,
                dimension: Dimension::new("22", 22.0).unwrap(),
            },
        ]))
        .unwrap();
    let stale = document.current();
    assert_eq!(
        stale.audit_feature_parameter_freshness(&identity).unwrap()[0].freshness,
        FeatureParameterFreshness::Stale(FeatureParameterStaleReason::InputChanged)
    );
    assert!(matches!(
        ExactResultRegistry::accept(&stale, [Arc::new(initial_package.into())]),
        Err(ExactProductError::StaleResult)
    ));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RecomputeFeatureParameters {
                identity: identity.clone(),
            },
        ]))
        .unwrap();
    let recomputed = document.current();
    assert_eq!(
        recomputed
            .audit_feature_parameter_freshness(&identity)
            .unwrap()[0]
            .freshness,
        FeatureParameterFreshness::Current
    );
    let request = ExactFeatureChainRequest::from_snapshot(&recomputed, DEFINITION).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 44.0]]);

    let registry =
        ExactResultRegistry::accept(&recomputed, [Arc::new(package.clone().into())]).unwrap();
    let projection = ExactInteractionProjection::from_snapshot(&recomputed, &registry);
    assert_eq!(projection.occurrence_count(), 1);
    assert_eq!(
        projection
            .exact_pick(ray_for(ExactFaceRole::Top, 100.0, 60.0, 44.0))
            .and_then(|hit| hit.target.body.role()),
        Some(ExactFaceRole::Top)
    );
    let export = ExactBodyPackage::from(package).mesh_export(Transform::identity());
    assert!(
        export
            .mesh_obj
            .contains("v 0.00000000000000000 0.00000000000000000 44.00000000000000000")
    );
    assert!(export.loss_report.contains("producer_feature_id=12"));
}

#[test]
fn exact_to_mesh_conversion_creates_one_detached_canonical_authority_with_explicit_loss() {
    const MESH_DEFINITION: DefinitionId = DefinitionId(80);
    const MESH_FEATURE: FeatureId = FeatureId(81);
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = rectangle_document(100.0, 60.0, 18.0);
    let source = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&source, DEFINITION).unwrap();
    let exact = ExactBodyPackage::from(supervisor.evaluate_rectangle(&request).unwrap());
    let batch = exact
        .detached_mesh_conversion_batch(
            &source,
            MESH_DEFINITION,
            "Detached mesh",
            MESH_FEATURE,
            "Canonical mesh body",
        )
        .unwrap();
    assert_eq!(batch.commands().len(), 2);
    let undo_steps = document.visible_undo_steps();

    document.apply_batch(&batch).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_steps + 1);
    let converted = document.current();
    assert_eq!(
        converted.definition(MESH_DEFINITION).unwrap().feature_ids(),
        &[MESH_FEATURE]
    );
    assert!(converted.feature(EXTRUSION).is_some());
    let FeatureKind::MeshBody(spec) = converted.feature(MESH_FEATURE).unwrap().kind() else {
        panic!("conversion destination must be a canonical mesh body");
    };
    assert_eq!(spec.vertices_mm.len(), 8);
    assert_eq!(spec.triangles.len(), 12);
    let loss = spec.exact_conversion_loss_report().unwrap();
    assert!(loss.contains("authority=canonical mesh body"));
    assert!(loss.contains("conversion=exact-to-mesh"));
    assert!(loss.contains("destination_definition_id=80"));
    assert!(loss.contains("destination_feature_id=81"));
    assert!(loss.contains("exact_reference_consequence=Lost"));
    assert!(loss.contains("editability_loss="));
    assert!(loss.contains("topology_loss="));
    assert!(loss.contains("tolerance_loss="));

    let converted_digest = converted.canonical_digest();
    let reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&converted)).unwrap();
    assert_eq!(reopened.source_schema(), 17);
    let reopened_snapshot = reopened.snapshot();
    let reopened_spec = match reopened_snapshot.feature(MESH_FEATURE).unwrap().kind() {
        FeatureKind::MeshBody(spec) => spec,
        _ => panic!("mesh body must survive schema round-trip"),
    };
    assert_eq!(reopened_snapshot.canonical_digest(), converted_digest);
    assert_eq!(reopened_spec, spec);
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        source.canonical_digest()
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        converted_digest
    );

    assert_eq!(
        exact
            .detached_mesh_conversion_batch(
                &document.current(),
                DefinitionId(82),
                "Stale mesh",
                FeatureId(83),
                "Stale body",
            )
            .unwrap_err(),
        ExactProductError::StaleResult
    );

    let mut invalid_spec = spec.clone();
    invalid_spec.triangles.pop();
    let before_invalid = document.current().canonical_digest();
    let invalid = document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(84),
            name: "Invalid mesh".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(85),
            definition_id: DefinitionId(84),
            name: "Open mesh".to_owned(),
            kind: FeatureKind::MeshBody(invalid_spec),
        },
    ]));
    assert!(matches!(invalid, Err(CanonicalError::InvalidMeshBody)));
    assert_eq!(document.current().canonical_digest(), before_invalid);
    assert!(document.current().definition(DefinitionId(84)).is_none());
}

#[test]
fn scheduler_evaluates_canonical_boolean_cut_with_seven_role_evidences() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = boolean_document(
        100.0,
        60.0,
        18.0,
        [30.0, 20.0, 20.0, 15.0],
        BooleanOperation::Cut,
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let cut = request.boolean.as_ref().unwrap();
    assert_eq!(cut.feature_id, THROUGH_CUT);
    assert_eq!(cut.operation, BooleanOperation::Cut);
    assert_eq!(cut.target_feature_id, EXTRUSION);
    assert_eq!(cut.tool_feature_id, TOOL_EXTRUSION);
    assert_eq!(cut.profile_feature_id, CUT_PROFILE);
    assert_eq!(request.producer_feature_id(), THROUGH_CUT);
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity.producer_feature_id, THROUGH_CUT);
    assert_eq!(package.identity.evaluator, EXACT_THROUGH_CUT_EVALUATOR_V1);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    assert_eq!(package.vertices.len(), 16);
    assert_eq!(package.triangles.len(), 32);
    let mut edge_use = BTreeMap::<(u32, u32), usize>::new();
    let mut signed_volume_mm3 = 0.0;
    for triangle in &package.triangles {
        let [a, b, c] = triangle
            .vertex_indices
            .map(|index| package.vertices[index as usize].position_mm);
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        assert!(cross.into_iter().map(|value| value * value).sum::<f64>() > 1.0e-12);
        signed_volume_mm3 += (a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
        for [first, second] in [
            [triangle.vertex_indices[0], triangle.vertex_indices[1]],
            [triangle.vertex_indices[1], triangle.vertex_indices[2]],
            [triangle.vertex_indices[2], triangle.vertex_indices[0]],
        ] {
            *edge_use
                .entry((first.min(second), first.max(second)))
                .or_default() += 1;
        }
    }
    assert!(edge_use.values().all(|count| *count == 2));
    assert!((signed_volume_mm3 - 102_600.0).abs() < 1.0e-6);
    assert_eq!(package.references.len(), 7);
    let export = ExactBodyPackage::from(package.clone()).mesh_export(
        Transform::from_matrix([
            0.0, -1.0, 0.0, 10.0, 1.0, 0.0, 0.0, 20.0, 0.0, 0.0, 1.0, 30.0, 0.0, 0.0, 0.0, 1.0,
        ])
        .unwrap(),
    );
    assert!(
        export
            .mesh_obj
            .contains("v 10.00000000000000000 20.00000000000000000 30.00000000000000000")
    );
    assert!(export.mesh_obj.contains("g through_cut.wall.west"));
    assert!(export.loss_report.contains("producer_feature_id=16"));

    let results =
        ExactResultRegistry::accept(&snapshot, [Arc::new(package.clone().into())]).unwrap();
    let projection = ExactInteractionProjection::from_snapshot(&snapshot, &results);
    let hole_ray = Ray::new(Vec3::new(40.0, 27.5, 30.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    assert!(projection.exact_pick(hole_ray).is_none());
    let cut_wall_ray = Ray::new(Vec3::new(40.0, 27.5, 9.0), Vec3::new(-1.0, 0.0, 0.0)).unwrap();
    assert_eq!(
        projection
            .exact_pick(cut_wall_ray)
            .and_then(|hit| hit.target.body.role()),
        Some(ExactFaceRole::CutWest)
    );

    for role in [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
        ExactFaceRole::CutWest,
        ExactFaceRole::CutEast,
        ExactFaceRole::CutSouth,
        ExactFaceRole::CutNorth,
    ] {
        let matching = package
            .references
            .iter()
            .filter(|reference| reference.role() == Some(role))
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "expected one durable {role:?} evidence");
        assert_eq!(matching[0].producer_feature_id, THROUGH_CUT);
        assert_eq!(
            matching[0].profile_feature_id,
            match role {
                ExactFaceRole::Top | ExactFaceRole::Bottom | ExactFaceRole::East => PROFILE,
                ExactFaceRole::CutWest
                | ExactFaceRole::CutEast
                | ExactFaceRole::CutSouth
                | ExactFaceRole::CutNorth => CUT_PROFILE,
                ExactFaceRole::RevolveBottom
                | ExactFaceRole::RevolveBody
                | ExactFaceRole::RevolveShoulder
                | ExactFaceRole::RevolveNeck
                | ExactFaceRole::RevolveMouth
                | ExactFaceRole::ShellOuterBottom
                | ExactFaceRole::ShellOuterBody
                | ExactFaceRole::ShellOuterShoulder
                | ExactFaceRole::ShellOuterNeck
                | ExactFaceRole::ShellRim
                | ExactFaceRole::ShellInnerBottom
                | ExactFaceRole::ShellInnerBody
                | ExactFaceRole::ShellInnerShoulder
                | ExactFaceRole::ShellInnerNeck => unreachable!(),
            }
        );
        assert!(matching[0].has_valid_lineage());
    }

    for reference in package.references.clone() {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    let loaded =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&document.current()))
            .unwrap();
    assert_eq!(loaded.source_schema(), 17);
    let reopened = match loaded {
        ketchup_core::persistence::LoadOutcome::Editable { document, .. } => document,
        ketchup_core::persistence::LoadOutcome::ReviewOnly(_) => {
            panic!("schema-9 Boolean evidence must reopen editable")
        }
    };
    assert_eq!(
        reopened.current().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert_eq!(reopened.current().exact_reference_evidence().count(), 7);
}

#[test]
fn scheduler_evaluates_contained_boolean_union_as_the_target_body() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let document = boolean_document(
        100.0,
        60.0,
        18.0,
        [30.0, 20.0, 20.0, 15.0],
        BooleanOperation::Union,
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let boolean = request.boolean.as_ref().unwrap();
    assert_eq!(boolean.operation, BooleanOperation::Union);
    assert_eq!(request.producer_feature_id(), THROUGH_CUT);
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity.producer_feature_id, THROUGH_CUT);
    assert_eq!(package.identity.evaluator, EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    assert_eq!(package.vertices.len(), 8);
    assert_eq!(package.triangles.len(), 12);
    assert_eq!(package.references.len(), 3);

    let results = ExactResultRegistry::accept(&snapshot, [Arc::new(package.into())]).unwrap();
    let projection = ExactInteractionProjection::from_snapshot(&snapshot, &results);
    let tool_region_ray = Ray::new(Vec3::new(40.0, 27.5, 30.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    assert_eq!(
        projection
            .exact_pick(tool_region_ray)
            .and_then(|hit| hit.target.body.role()),
        Some(ExactFaceRole::Top)
    );
}

#[test]
fn scheduler_evaluates_bottle_revolve_with_deterministic_mesh_and_five_durable_roles() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = bottle_document();
    let snapshot = document.current();
    let request = ExactRevolveRequest::from_snapshot(&snapshot, BOTTLE_DEFINITION).unwrap();

    let first = supervisor.evaluate_revolve(&request).unwrap();
    let second = supervisor.evaluate_revolve(&request).unwrap();
    assert!(first.is_current(&snapshot));
    assert_eq!(first.identity, second.identity);
    assert_eq!(first.vertices, second.vertices);
    assert_eq!(first.triangles, second.triangles);
    for (actual, expected) in first
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([-30.0, -30.0, 0.0, 30.0, 30.0, 155.0])
    {
        assert!((actual - expected).abs() <= 1.0e-6);
    }
    assert_eq!(first.references.len(), 5);
    assert_eq!(first.vertices.len(), 130);
    assert_eq!(first.triangles.len(), 256);

    let results = ExactResultRegistry::accept(&snapshot, [Arc::new(first.clone().into())]).unwrap();
    let projection = ExactInteractionProjection::from_snapshot(&snapshot, &results);
    assert_eq!(projection.occurrence_count(), 1);
    for (role, origin, direction) in [
        (
            ExactFaceRole::RevolveBottom,
            Vec3::new(20.0, 0.0, -10.0),
            Vec3::new(0.0, 0.0, 1.0),
        ),
        (
            ExactFaceRole::RevolveBody,
            Vec3::new(40.0, 0.0, 50.0),
            Vec3::new(-1.0, 0.0, 0.0),
        ),
        (
            ExactFaceRole::RevolveShoulder,
            Vec3::new(40.0, 0.0, 120.0),
            Vec3::new(-1.0, 0.0, 0.0),
        ),
        (
            ExactFaceRole::RevolveNeck,
            Vec3::new(20.0, 0.0, 140.0),
            Vec3::new(-1.0, 0.0, 0.0),
        ),
        (
            ExactFaceRole::RevolveMouth,
            Vec3::new(6.0, 0.0, 165.0),
            Vec3::new(0.0, 0.0, -1.0),
        ),
    ] {
        let hit = projection
            .exact_pick(Ray::new(origin, direction).unwrap())
            .unwrap_or_else(|| panic!("revolve pick missed {role:?}"));
        assert_eq!(hit.target.body.role(), Some(role));
        assert!(hit.target.body.has_valid_lineage());
        assert_eq!(hit.target.body.producer_feature_id, BOTTLE_REVOLVE);
    }

    for reference in first.references.clone() {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    assert_eq!(document.current().exact_reference_evidence().count(), 5);
    let reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&document.current()))
            .unwrap();
    let reopened = match reopened {
        ketchup_core::persistence::LoadOutcome::Editable { document, .. } => document,
        ketchup_core::persistence::LoadOutcome::ReviewOnly(_) => {
            panic!("current M6 bottle must reopen editable")
        }
    };
    assert_eq!(reopened.current().exact_reference_evidence().count(), 5);
}

#[test]
fn scheduler_evaluates_editable_bottle_shell_with_open_mouth_and_current_references() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = bottle_shell_document();
    let snapshot = document.current();
    let request = ExactRevolveRequest::from_snapshot(&snapshot, BOTTLE_DEFINITION).unwrap();
    assert_eq!(request.shell_feature_id, Some(BOTTLE_SHELL));
    assert_eq!(request.producer_feature_id(), BOTTLE_SHELL);
    assert_eq!(request.thickness_mm(), Some(2.0));

    let first = supervisor.evaluate_revolve(&request).unwrap();
    let repeated = supervisor.evaluate_revolve(&request).unwrap();
    assert!(first.is_current(&snapshot));
    assert_eq!(first.identity, repeated.identity);
    assert_eq!(first.vertices, repeated.vertices);
    assert_eq!(first.triangles, repeated.triangles);
    assert_eq!(first.references.len(), 9);
    assert_eq!(first.vertices.len(), 258);
    assert_eq!(first.triangles.len(), 512);
    assert_eq!(first.identity.producer_feature_id, BOTTLE_SHELL);
    assert_eq!(first.identity.shell_feature_id, Some(BOTTLE_SHELL));
    assert!(first.references.iter().all(|reference| {
        reference.producer_feature_id == BOTTLE_SHELL && reference.has_valid_lineage()
    }));

    let top_triangles = first
        .triangles
        .iter()
        .filter(|triangle| triangle.face_role == Some(ExactFaceRole::ShellRim))
        .collect::<Vec<_>>();
    assert_eq!(top_triangles.len(), 64);
    assert!(top_triangles.iter().all(|triangle| {
        triangle.vertex_indices.iter().all(|index| {
            let [x, y, z] = first.vertices[*index as usize].position_mm;
            (z - 155.0).abs() <= 1.0e-9 && x.hypot(y) >= 10.0 - 1.0e-9
        })
    }));
    assert!(first.triangles.iter().all(|triangle| {
        if triangle.face_role == Some(ExactFaceRole::ShellRim) {
            true
        } else {
            triangle.vertex_indices.iter().all(|index| {
                let [x, y, z] = first.vertices[*index as usize].position_mm;
                z != 155.0 || x.hypot(y) != 0.0
            })
        }
    }));

    for reference in first.references.clone() {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: BOTTLE_SHELL,
                dimension: Dimension::new("3", 3.0).unwrap(),
            },
        ]))
        .unwrap();
    assert!(!first.is_current(&document.current()));
    assert!(
        document
            .register_exact_reference_evidence(first.references[0].clone())
            .is_err()
    );

    let changed_request =
        ExactRevolveRequest::from_snapshot(&document.current(), BOTTLE_DEFINITION).unwrap();
    let changed = supervisor.evaluate_revolve(&changed_request).unwrap();
    assert!(changed.is_current(&document.current()));
    assert_ne!(
        changed.identity.canonical_input_digest,
        first.identity.canonical_input_digest
    );
    for reference in changed.references.clone() {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    let reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&document.current()))
            .unwrap();
    assert_eq!(reopened.source_schema(), 17);
    assert_eq!(reopened.snapshot().exact_reference_evidence().count(), 9);
}

#[test]
fn worker_exports_current_bottle_as_round_trippable_step_and_rejects_stale_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("bottle.step");
    let stale_path = directory.path().join("stale.step");
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = bottle_shell_document();
    let snapshot = document.current();
    let request = ExactRevolveRequest::from_snapshot(&snapshot, BOTTLE_DEFINITION).unwrap();
    let package = supervisor.evaluate_revolve(&request).unwrap();

    supervisor
        .export_revolve_step(&snapshot, &request, &package, &step_path)
        .unwrap();
    let raw = std::fs::read_to_string(&step_path).unwrap();
    assert!(raw.starts_with("ISO-10303-21;"));
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(imported.body.topology.solid_count, 1);
    assert!(imported.body.topology.face_count >= package.references.len() as u32);
    assert!((imported.body.topology.bounds_mm.min.x - package.bounds_mm[0][0]).abs() < 1.0e-6);
    assert!((imported.body.topology.bounds_mm.max.z - package.bounds_mm[1][2]).abs() < 1.0e-6);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: BOTTLE_SHELL,
                dimension: Dimension::new("3", 3.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_revolve_step(&document.current(), &request, &package, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );
}

#[test]
fn scheduler_evaluates_controlled_bottle_fillet_and_chamfer_with_current_roles() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = controlled_finished_bottle_document();
    let snapshot = document.current();
    let request = ExactRevolveRequest::from_snapshot(&snapshot, BOTTLE_DEFINITION).unwrap();
    assert_eq!(request.control_feature_id, Some(BOTTLE_CONTROL));
    assert_eq!(request.shell_feature_id, Some(BOTTLE_SHELL));
    assert_eq!(request.edge_finish_feature_id, Some(BOTTLE_FINISH));
    assert_eq!(request.edge_finish_kind, Some(BottleEdgeFinishKind::Fillet));
    assert_eq!(request.producer_feature_id(), BOTTLE_FINISH);

    let fillet = supervisor.evaluate_revolve(&request).unwrap();
    let repeated = supervisor.evaluate_revolve(&request).unwrap();
    assert!(fillet.is_current(&snapshot));
    assert_eq!(fillet.identity, repeated.identity);
    assert_eq!(fillet.vertices, repeated.vertices);
    assert_eq!(fillet.triangles, repeated.triangles);
    assert_eq!(fillet.references.len(), 9);
    assert_eq!(fillet.identity.control_feature_id, Some(BOTTLE_CONTROL));
    assert_eq!(fillet.identity.edge_finish_feature_id, Some(BOTTLE_FINISH));
    assert!(fillet.references.iter().all(|reference| {
        reference.producer_feature_id == BOTTLE_FINISH && reference.has_valid_lineage()
    }));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBottleControlDimension {
                id: BOTTLE_CONTROL,
                control: BottleControlDimension::BodyRadius,
                dimension: Dimension::new("32", 32.0).unwrap(),
            },
            CanonicalCommand::SetBottleControlDimension {
                id: BOTTLE_CONTROL,
                control: BottleControlDimension::BodyHeight,
                dimension: Dimension::new("120", 120.0).unwrap(),
            },
            CanonicalCommand::SetBottleControlDimension {
                id: BOTTLE_CONTROL,
                control: BottleControlDimension::ShoulderRise,
                dimension: Dimension::new("16", 16.0).unwrap(),
            },
            CanonicalCommand::SetFeatureDimension {
                id: BOTTLE_FINISH,
                dimension: Dimension::new("1.5", 1.5).unwrap(),
            },
            CanonicalCommand::SetBottleEdgeFinishKind {
                id: BOTTLE_FINISH,
                kind: BottleEdgeFinishKind::Chamfer,
            },
        ]))
        .unwrap();
    assert!(!fillet.is_current(&document.current()));

    let changed_request =
        ExactRevolveRequest::from_snapshot(&document.current(), BOTTLE_DEFINITION).unwrap();
    assert_eq!(
        changed_request.edge_finish_kind,
        Some(BottleEdgeFinishKind::Chamfer)
    );
    assert_eq!(
        changed_request.points_mm(),
        vec![
            [0.0, 0.0],
            [32.0, 0.0],
            [32.0, 120.0],
            [12.0, 136.0],
            [12.0, 161.0],
            [0.0, 161.0],
        ]
    );
    let chamfer = supervisor.evaluate_revolve(&changed_request).unwrap();
    assert!(chamfer.is_current(&document.current()));
    assert_ne!(
        chamfer.identity.canonical_input_digest,
        fillet.identity.canonical_input_digest
    );
    assert_ne!(
        chamfer.identity.result_fingerprint,
        fillet.identity.result_fingerprint
    );
    for (actual, expected) in chamfer
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([-32.0, -32.0, 0.0, 32.0, 32.0, 161.0])
    {
        assert!((actual - expected).abs() <= 1.0e-6);
    }
    for reference in chamfer.references.clone() {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    let reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&document.current()))
            .unwrap();
    assert_eq!(reopened.source_schema(), 17);
    assert_eq!(reopened.snapshot().exact_reference_evidence().count(), 9);
}

fn controlled_finished_bottle_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: BOTTLE_DEFINITION,
                name: "Controlled M6 bottle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_PROFILE,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [0.0, 0.0],
                        [30.0, 0.0],
                        [30.0, 110.0],
                        [12.0, 130.0],
                        [12.0, 155.0],
                        [0.0, 155.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_CONTROL,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle controls".to_owned(),
                kind: FeatureKind::BottleProfileControl {
                    profile: BOTTLE_PROFILE,
                    body_radius: Dimension::new("30", 30.0).unwrap(),
                    body_height: Dimension::new("110", 110.0).unwrap(),
                    shoulder_rise: Dimension::new("20", 20.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_REVOLVE,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle revolve".to_owned(),
                kind: FeatureKind::Revolve {
                    profile: BOTTLE_CONTROL,
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_SHELL,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle shell".to_owned(),
                kind: FeatureKind::Shell {
                    target: BOTTLE_REVOLVE,
                    thickness: Dimension::new("2", 2.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_FINISH,
                definition_id: BOTTLE_DEFINITION,
                name: "Shoulder finish".to_owned(),
                kind: FeatureKind::BottleEdgeFinish {
                    target: BOTTLE_SHELL,
                    kind: BottleEdgeFinishKind::Fillet,
                    amount: Dimension::new("2", 2.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: BOTTLE_OCCURRENCE,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn bottle_shell_document() -> DocumentStore {
    let mut document = bottle_document();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: BOTTLE_SHELL,
            definition_id: BOTTLE_DEFINITION,
            name: "Bottle shell".to_owned(),
            kind: FeatureKind::Shell {
                target: BOTTLE_REVOLVE,
                thickness: Dimension::new("2", 2.0).unwrap(),
            },
        }]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn bottle_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: BOTTLE_DEFINITION,
                name: "M6 bottle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_PROFILE,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [0.0, 0.0],
                        [30.0, 0.0],
                        [30.0, 110.0],
                        [12.0, 130.0],
                        [12.0, 155.0],
                        [0.0, 155.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_REVOLVE,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle revolve".to_owned(),
                kind: FeatureKind::Revolve {
                    profile: BOTTLE_PROFILE,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: BOTTLE_OCCURRENCE,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn rectangle_document(width: f64, depth: f64, height: f64) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "C1b rectangle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Exact extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::new(height.to_string(), height).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(13),
                definition_id: DEFINITION,
                name: "C1b occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn boolean_document(
    width: f64,
    depth: f64,
    height: f64,
    cut: [f64; 4],
    operation: BooleanOperation,
) -> DocumentStore {
    let mut document = rectangle_document(width, depth, height);
    let [x, y, cut_width, cut_depth] = cut;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Through-cut profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [x, y],
                        [x + cut_width, y],
                        [x + cut_width, y + cut_depth],
                        [x, y + cut_depth],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "Boolean tool extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: CUT_PROFILE,
                    height: Dimension::new(height.to_string(), height).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: THROUGH_CUT,
                definition_id: DEFINITION,
                name: "Bounded Boolean cut".to_owned(),
                kind: FeatureKind::Boolean {
                    operation,
                    target: EXTRUSION,
                    tool: TOOL_EXTRUSION,
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn ray_for(role: ExactFaceRole, width: f64, depth: f64, height: f64) -> Ray {
    let (origin, direction) = match role {
        ExactFaceRole::Top => (
            Vec3::new(width / 2.0, depth / 2.0, height + 10.0),
            Vec3::new(0.0, 0.0, -1.0),
        ),
        ExactFaceRole::Bottom => (
            Vec3::new(width / 2.0, depth / 2.0, -10.0),
            Vec3::new(0.0, 0.0, 1.0),
        ),
        ExactFaceRole::East => (
            Vec3::new(width + 10.0, depth / 2.0, height / 2.0),
            Vec3::new(-1.0, 0.0, 0.0),
        ),
        ExactFaceRole::CutWest
        | ExactFaceRole::CutEast
        | ExactFaceRole::CutSouth
        | ExactFaceRole::CutNorth
        | ExactFaceRole::RevolveBottom
        | ExactFaceRole::RevolveBody
        | ExactFaceRole::RevolveShoulder
        | ExactFaceRole::RevolveNeck
        | ExactFaceRole::RevolveMouth
        | ExactFaceRole::ShellOuterBottom
        | ExactFaceRole::ShellOuterBody
        | ExactFaceRole::ShellOuterShoulder
        | ExactFaceRole::ShellOuterNeck
        | ExactFaceRole::ShellRim
        | ExactFaceRole::ShellInnerBottom
        | ExactFaceRole::ShellInnerBody
        | ExactFaceRole::ShellInnerShoulder
        | ExactFaceRole::ShellInnerNeck => {
            panic!("non-extrusion roles are outside the extrusion-only C1b corpus")
        }
    };
    Ray::new(origin, direction).unwrap()
}

fn worker_path() -> &'static str {
    env!("CARGO_BIN_EXE_ketchup-exact-worker")
}
