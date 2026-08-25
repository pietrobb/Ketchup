use ketchup_core::bottle_m6::ExactRevolveRequest;
use ketchup_core::document::{
    BOTTLE_SHELL_OPENING_FACE_ROLE, BOTTLE_SHOULDER_EDGE_ROLE, BooleanOperation,
    BottleControlDimension, BottleEdgeFinishKind, CanonicalCommand, CanonicalError, CommandBatch,
    DefinitionId, DerivedIdentity, Dimension, DimensionDisplayUnit, DimensionPresentation,
    DimensionReferenceHealth, DocumentId, DocumentStore, EvaluationIdentity, FeatureId,
    FeatureKind, FeatureParameterBinding, FeatureParameterFreshness, FeatureParameterSlot,
    FeatureParameterStaleReason, FeatureParameterTarget, LoftSection, MeshAuthority, NodeId,
    OccurrenceId, PersistentDimension, PersistentDimensionId, PersistentDimensionTarget, PortSpec,
    ProfileSegment, RuleOutput, SlotPath, SlotSegment, StableEdgeRole, StableFaceRole, Transform,
};
use ketchup_core::exact_product::{
    EXACT_ARC_PROFILE_EVALUATOR_V1, EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1,
    EXACT_BOOLEAN_SPLIT_EVALUATOR_V1, EXACT_BOOLEAN_UNION_EVALUATOR_V1,
    EXACT_BOX_FINISH_EVALUATOR_V1, EXACT_BOX_SHELL_EVALUATOR_V1, EXACT_CIRCLE_EVALUATOR_V1,
    EXACT_CIRCULAR_CUT_EVALUATOR_V1, EXACT_LINEAR_PROFILE_EVALUATOR_V1, EXACT_LOFT_EVALUATOR_V1,
    EXACT_PLANAR_OFFSET_EVALUATOR_V1, EXACT_POCKET_EVALUATOR_V1, EXACT_SWEEP_EVALUATOR_V1,
    EXACT_THROUGH_CUT_EVALUATOR_V1, ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest,
    ExactLoftRequest, ExactPlanarOffsetRequest, ExactProductError, ExactReferenceQuarantineReason,
    ExactReferenceResolution, ExactRenderPackage, ExactResultRegistry, ExactSweepRequest,
    canonical_reference_lineage_digest,
};
use ketchup_exact::{
    ExactBackend, GeometryErrorCode, PlanarProfileSegment, RectangleExtrudeSpec,
    ReferenceResolution, StabilityClass, capture_guaranteed_references, resolve_subshape_reference,
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
const POCKET: FeatureId = FeatureId(17);
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
    assert_eq!(
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
            CanonicalCommand::SetProfilePoints {
                id: CUT_PROFILE,
                points_mm: vec![[80.0, 0.0], [120.0, 0.0], [120.0, 60.0], [80.0, 60.0]],
            },
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
    assert_eq!(
        reopened.source_schema(),
        ketchup_core::persistence::CURRENT_SCHEMA
    );
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
    assert_eq!(
        reopened.source_schema(),
        ketchup_core::persistence::CURRENT_SCHEMA
    );
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
                ExactFaceRole::CutLinear
                | ExactFaceRole::CutWest
                | ExactFaceRole::CutEast
                | ExactFaceRole::CutSouth
                | ExactFaceRole::CutNorth
                | ExactFaceRole::PocketFloor
                | ExactFaceRole::PocketWest
                | ExactFaceRole::PocketEast
                | ExactFaceRole::PocketSouth
                | ExactFaceRole::PocketNorth => CUT_PROFILE,
                ExactFaceRole::CircleSide
                | ExactFaceRole::ArcSide
                | ExactFaceRole::LinearSide
                | ExactFaceRole::CutCircle
                | ExactFaceRole::RevolveBottom
                | ExactFaceRole::RevolveBody
                | ExactFaceRole::RevolveShoulder
                | ExactFaceRole::RevolveNeck
                | ExactFaceRole::RevolveMouth
                | ExactFaceRole::RevolveSide0
                | ExactFaceRole::RevolveSide1
                | ExactFaceRole::RevolveStart
                | ExactFaceRole::RevolveEnd
                | ExactFaceRole::ShellOuterBottom
                | ExactFaceRole::ShellOuterBody
                | ExactFaceRole::ShellOuterShoulder
                | ExactFaceRole::ShellOuterNeck
                | ExactFaceRole::ShellRim
                | ExactFaceRole::ShellInnerBottom
                | ExactFaceRole::ShellInnerBody
                | ExactFaceRole::ShellInnerShoulder
                | ExactFaceRole::ShellInnerNeck
                | ExactFaceRole::BoxShellOuterBottom
                | ExactFaceRole::BoxShellOuterEast
                | ExactFaceRole::BoxShellRim
                | ExactFaceRole::PlanarOffsetFace
                | ExactFaceRole::SweepStart
                | ExactFaceRole::SweepEnd
                | ExactFaceRole::SweepSide0
                | ExactFaceRole::SweepSide1
                | ExactFaceRole::SweepSide2
                | ExactFaceRole::SweepSide3
                | ExactFaceRole::LoftStart
                | ExactFaceRole::LoftEnd
                | ExactFaceRole::LoftSide => unreachable!(),
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
    assert_eq!(
        loaded.source_schema(),
        ketchup_core::persistence::CURRENT_SCHEMA
    );
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
fn scheduler_evaluates_slanted_polygon_cut_and_rejects_invalid_profiles_before_dispatch() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let points = [[20.0, 15.0], [50.0, 18.0], [45.0, 40.0], [15.0, 37.0]];
    let document = polygon_cut_document(&points, 18.0, true);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    assert_eq!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .map(|profile| profile.segments.len()),
        Some(4)
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    assert_eq!(package.references.len(), 4);
    let wall = package.reference(ExactFaceRole::CutLinear).unwrap();
    assert_eq!(wall.profile_feature_id, CUT_PROFILE);
    assert!(wall.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| { triangle.face_role == Some(ExactFaceRole::CutLinear) })
    );

    let pocket_document = polygon_pocket_document(&points, 18.0, 7.0);
    let pocket_snapshot = pocket_document.current();
    let pocket_request =
        ExactFeatureChainRequest::from_snapshot(&pocket_snapshot, DEFINITION).unwrap();
    assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(pocket_request.pocket_depth_bits, Some(7.0_f64.to_bits()));
    assert_eq!(
        pocket_request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .map(|profile| profile.segments.len()),
        Some(4)
    );
    let pocket_package = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    let repeated_pocket = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    assert!(pocket_package.is_current(&pocket_snapshot));
    assert_eq!(pocket_package.identity, repeated_pocket.identity);
    assert_eq!(pocket_package.references.len(), 5);
    for role in [ExactFaceRole::CutLinear, ExactFaceRole::PocketFloor] {
        let reference = pocket_package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, CUT_PROFILE);
        assert!(reference.has_valid_lineage());
        assert!(
            pocket_package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(role))
        );
    }
    assert_closed_manifold(&pocket_package);

    let self_intersecting = polygon_cut_document(
        &[[20.0, 15.0], [50.0, 40.0], [20.0, 40.0], [50.0, 15.0]],
        18.0,
        true,
    )
    .current();
    assert!(ExactFeatureChainRequest::from_snapshot(&self_intersecting, DEFINITION).is_err());
}

#[test]
fn scheduler_evaluates_exact_line_arc_d_profile_through_cut_over_worker_protocol() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let document = line_arc_d_boolean_document(18.0, BooleanOperation::Cut);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    let wall = package.reference(ExactFaceRole::CutLinear).unwrap();
    assert_eq!(wall.profile_feature_id, CUT_PROFILE);
    assert!(wall.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::CutLinear))
    );
    assert_closed_manifold(&package);
}

#[test]
fn scheduler_evaluates_exact_capsule_through_cut_and_rejects_unsupported_mixed_operations() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document(18.0, BooleanOperation::Cut);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_capsule_profile())
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    let wall = package.reference(ExactFaceRole::CutLinear).unwrap();
    assert_eq!(wall.profile_feature_id, CUT_PROFILE);
    assert!(wall.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::CutLinear))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("capsule-through-cut.step");
    let stale_path = directory.path().join("stale-capsule-through-cut.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(imported.body.topology.vertex_count, 16);
    assert_eq!(imported.body.topology.edge_count, 24);
    assert_eq!(imported.body.topology.face_count, 10);
    assert_eq!(imported.body.topology.shell_count, 1);
    assert_eq!(imported.body.topology.solid_count, 1);
    let expected_volume = 108_000.0 - (400.0 + 100.0 * std::f64::consts::PI) * 18.0;
    let step_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
    assert!(
        step_volume_error / expected_volume < 0.0003,
        "capsule STEP relative volume error {}; actual={}, expected={expected_volume}",
        step_volume_error / expected_volume,
        imported.body.topology.volume_mm3
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    let direct_segments = [
        PlanarProfileSegment::Line {
            start_mm: [30.0, 20.0],
            end_mm: [50.0, 20.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [50.0, 20.0],
            end_mm: [50.0, 40.0],
            center_mm: [50.0, 30.0],
            clockwise: false,
        },
        PlanarProfileSegment::Line {
            start_mm: [50.0, 40.0],
            end_mm: [30.0, 40.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [30.0, 40.0],
            end_mm: [30.0, 20.0],
            center_mm: [30.0, 30.0],
            clockwise: false,
        },
    ];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let cut = backend
        .cut_mixed_profile(&base.body, &direct_segments, -1.0, 20.0)
        .unwrap();
    let expected_volume = 108_000.0 - (400.0 + 100.0 * std::f64::consts::PI) * 18.0;
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(cut.body.topology.solid_count, 1);

    let rejected = capsule_boolean_document(18.0, BooleanOperation::Union);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Union
        ))
    );
    let broader_mixed = [
        direct_segments[0],
        direct_segments[1],
        direct_segments[2],
        PlanarProfileSegment::Line {
            start_mm: [30.0, 40.0],
            end_mm: [30.0, 20.0],
        },
    ];
    let error = backend
        .cut_mixed_profile(&base.body, &broader_mixed, -1.0, 20.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_rounded_rectangle_through_cut_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Cut,
        rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_rounded_rectangle_profile())
    );

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let cut = backend
        .cut_mixed_profile(&base.body, &rounded_rectangle_planar_segments(), -1.0, 20.0)
        .unwrap();
    let profile_area = 2_144.0 + 64.0 * std::f64::consts::PI;
    let expected_volume = 108_000.0 - profile_area * 18.0;
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(cut.body.topology.solid_count, 1);

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        cut.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    let wall = package.reference(ExactFaceRole::CutLinear).unwrap();
    assert_eq!(wall.profile_feature_id, CUT_PROFILE);
    assert!(wall.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::CutLinear))
    );
    assert!(package.vertices.len() > 128);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("rounded-rectangle-through-cut.step");
    let stale_path = directory
        .path()
        .join("stale-rounded-rectangle-through-cut.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [24, 36, 14, 1, 1]
    );
    let step_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
    assert!(
        step_volume_error / expected_volume < 0.0032,
        "rounded rectangle STEP relative volume error {}; actual={}, expected={expected_volume}",
        step_volume_error / expected_volume,
        imported.body.topology.volume_mm3
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    let rejected_union = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Union,
        rounded_rectangle_segments(),
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&rejected_union.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Union
        ))
    );

    let mut broader_mixed = rounded_rectangle_planar_segments();
    broader_mixed[2] = PlanarProfileSegment::Line {
        start_mm: [80.0, 18.0],
        end_mm: [80.0, 44.0],
    };
    broader_mixed[3] = PlanarProfileSegment::CircularArc {
        start_mm: [80.0, 44.0],
        end_mm: [74.0, 50.0],
        center_mm: [74.0, 44.0],
        clockwise: false,
    };
    broader_mixed[4] = PlanarProfileSegment::Line {
        start_mm: [74.0, 50.0],
        end_mm: [28.0, 50.0],
    };
    let error = backend
        .cut_mixed_profile(&base.body, &broader_mixed, -1.0, 20.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_containing_rounded_rectangle_union_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Union,
        containing_rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_rounded_rectangle_profile())
    );

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let union = backend
        .fuse_mixed_profile(
            &base.body,
            &containing_rounded_rectangle_planar_segments(),
            0.0,
            18.0,
        )
        .unwrap();
    let expected_volume = (9_200.0 + 100.0 * std::f64::consts::PI) * 18.0;
    assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(union.body.topology.solid_count, 1);

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        union.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(
        package.bounds_mm,
        [[-10.0, -10.0, 0.0], [110.0, 70.0, 18.0]]
    );
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("rounded-rectangle-union.step");
    let stale_path = directory.path().join("stale-rounded-rectangle-union.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(imported.body.topology.shell_count, 1);
    assert_eq!(imported.body.topology.solid_count, 1);
    let step_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
    assert!(
        step_volume_error / expected_volume < 0.0032,
        "rounded rectangle union STEP relative volume error {}; actual={}, expected={expected_volume}",
        step_volume_error / expected_volume,
        imported.body.topology.volume_mm3
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    let mut broader_mixed = containing_rounded_rectangle_planar_segments();
    broader_mixed[3] = PlanarProfileSegment::CircularArc {
        start_mm: [110.0, 60.0],
        end_mm: [98.0, 70.0],
        center_mm: [98.0, 60.0],
        clockwise: false,
    };
    broader_mixed[4] = PlanarProfileSegment::Line {
        start_mm: [98.0, 70.0],
        end_mm: [0.0, 70.0],
    };
    let error = backend
        .fuse_mixed_profile(&base.body, &broader_mixed, 0.0, 18.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_side_overlapping_rounded_rectangle_union_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Union,
        side_overlapping_rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(profile.is_line_arc_rounded_rectangle_profile());
    assert_eq!(
        profile.rounded_rectangle_side_overlap_area(100.0, 60.0),
        Some(1_800.0)
    );
    for (dx, dy, expected_overlap_area) in [
        (80.0, 0.0, 1_800.0),
        (-80.0, 0.0, 1_800.0),
        (0.0, 50.0, 2_000.0),
        (0.0, -50.0, 2_000.0),
    ] {
        let oriented = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let oriented_request =
            ExactFeatureChainRequest::from_snapshot(&oriented.current(), DEFINITION).unwrap();
        assert_eq!(
            oriented_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.rounded_rectangle_side_overlap_area(100.0, 60.0)),
            Some(expected_overlap_area)
        );
    }

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let union = backend
        .fuse_mixed_profile(
            &base.body,
            &side_overlapping_rounded_rectangle_planar_segments(),
            0.0,
            18.0,
        )
        .unwrap();
    let expected_volume = (6_000.0 + 9_200.0 + 100.0 * std::f64::consts::PI - 1_800.0) * 18.0;
    assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            union.body.topology.vertex_count,
            union.body.topology.edge_count,
            union.body.topology.face_count,
            union.body.topology.shell_count,
            union.body.topology.solid_count,
        ],
        [20, 30, 12, 1, 1]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        union.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, -10.0, 0.0], [190.0, 70.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("side-overlap-rounded-rectangle-union.step");
    let stale_path = directory
        .path()
        .join("stale-side-overlap-rounded-rectangle-union.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [20, 30, 12, 1, 1]
    );
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    for rejected_segments in [
        translated_containing_rounded_rectangle_segments(110.0, 70.0),
        translated_containing_rounded_rectangle_segments(200.0, 0.0),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_corner_overlapping_rounded_rectangle_union_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Union,
        corner_overlapping_rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let expected_overlap_area = 500.0 + 25.0 * std::f64::consts::PI;
    assert_eq!(
        profile.rounded_rectangle_corner_overlap_area(100.0, 60.0),
        Some(expected_overlap_area)
    );
    assert_eq!(
        profile.rounded_rectangle_side_overlap_area(100.0, 60.0),
        None
    );
    for (dx, dy) in [(80.0, 50.0), (-80.0, 50.0), (80.0, -50.0), (-80.0, -50.0)] {
        let oriented = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let oriented_request =
            ExactFeatureChainRequest::from_snapshot(&oriented.current(), DEFINITION).unwrap();
        assert_eq!(
            oriented_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.rounded_rectangle_corner_overlap_area(100.0, 60.0)),
            Some(expected_overlap_area)
        );
    }

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let union = backend
        .fuse_mixed_profile(
            &base.body,
            &corner_overlapping_rounded_rectangle_planar_segments(),
            0.0,
            18.0,
        )
        .unwrap();
    let expected_volume =
        (6_000.0 + 9_200.0 + 100.0 * std::f64::consts::PI - expected_overlap_area) * 18.0;
    assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            union.body.topology.vertex_count,
            union.body.topology.edge_count,
            union.body.topology.face_count,
            union.body.topology.shell_count,
            union.body.topology.solid_count,
        ],
        [22, 33, 13, 1, 1]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        union.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [190.0, 120.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("corner-overlap-rounded-rectangle-union.step");
    let stale_path = directory
        .path()
        .join("stale-corner-overlap-rounded-rectangle-union.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [22, 33, 13, 1, 1]
    );
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    for rejected_segments in [
        translated_containing_rounded_rectangle_segments(110.0, 70.0),
        translated_containing_rounded_rectangle_segments(200.0, 100.0),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_two_axis_arc_clipped_corner_rounded_rectangle_union_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Union,
        two_axis_arc_clipped_corner_overlapping_rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 10.0_f64;
    let x_distance = 5.0_f64;
    let y_distance = 5.0_f64;
    let x_limit = (radius * radius - y_distance * y_distance).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let expected_overlap_area =
        primitive(x_limit) - primitive(x_distance) - y_distance * (x_limit - x_distance);
    assert_eq!(
        profile.rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(100.0, 60.0),
        Some(expected_overlap_area)
    );
    assert_eq!(
        profile.rounded_rectangle_arc_clipped_corner_overlap_area(100.0, 60.0),
        None
    );
    for (dx, dy) in [
        (105.0, 65.0),
        (-105.0, 65.0),
        (105.0, -65.0),
        (-105.0, -65.0),
    ] {
        let oriented = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let oriented_request =
            ExactFeatureChainRequest::from_snapshot(&oriented.current(), DEFINITION).unwrap();
        assert_eq!(
            oriented_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| {
                    profile.rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(100.0, 60.0)
                }),
            Some(expected_overlap_area)
        );
    }

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let union = backend
        .fuse_mixed_profile(
            &base.body,
            &two_axis_arc_clipped_corner_overlapping_rounded_rectangle_planar_segments(),
            0.0,
            18.0,
        )
        .unwrap();
    let expected_volume =
        (6_000.0 + 9_200.0 + 100.0 * std::f64::consts::PI - expected_overlap_area) * 18.0;
    assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            union.body.topology.vertex_count,
            union.body.topology.edge_count,
            union.body.topology.face_count,
            union.body.topology.shell_count,
            union.body.topology.solid_count,
        ],
        [26, 39, 15, 1, 1]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        union.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [215.0, 135.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("two-axis-arc-clipped-corner-rounded-rectangle-union.step");
    let stale_path = directory
        .path()
        .join("stale-two-axis-arc-clipped-corner-rounded-rectangle-union.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [26, 39, 15, 1, 1]
    );
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    for rejected_segments in [
        translated_containing_rounded_rectangle_segments(110.0, 70.0),
        translated_containing_rounded_rectangle_segments(200.0, 100.0),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_arc_clipped_corner_rounded_rectangle_union_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Union,
        arc_clipped_corner_overlapping_rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 10.0_f64;
    let clip_distance = 5.0_f64;
    let straight_extension = 5.0_f64;
    let expected_overlap_area = (radius - clip_distance) * straight_extension
        + radius * radius * std::f64::consts::FRAC_PI_4
        - 0.5
            * (clip_distance * (radius * radius - clip_distance * clip_distance).sqrt()
                + radius * radius * (clip_distance / radius).asin());
    assert_eq!(
        profile.rounded_rectangle_arc_clipped_corner_overlap_area(100.0, 60.0),
        Some(expected_overlap_area)
    );
    assert_eq!(
        profile.rounded_rectangle_corner_overlap_area(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.rounded_rectangle_side_overlap_area(100.0, 60.0),
        None
    );
    for (dx, dy) in [
        (105.0, 55.0),
        (-105.0, 55.0),
        (105.0, -55.0),
        (-105.0, -55.0),
        (95.0, 65.0),
        (-95.0, 65.0),
        (95.0, -65.0),
        (-95.0, -65.0),
    ] {
        let oriented = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let oriented_request =
            ExactFeatureChainRequest::from_snapshot(&oriented.current(), DEFINITION).unwrap();
        assert_eq!(
            oriented_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| {
                    profile.rounded_rectangle_arc_clipped_corner_overlap_area(100.0, 60.0)
                }),
            Some(expected_overlap_area)
        );
    }

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let union = backend
        .fuse_mixed_profile(
            &base.body,
            &arc_clipped_corner_overlapping_rounded_rectangle_planar_segments(),
            0.0,
            18.0,
        )
        .unwrap();
    let expected_volume =
        (6_000.0 + 9_200.0 + 100.0 * std::f64::consts::PI - expected_overlap_area) * 18.0;
    assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            union.body.topology.vertex_count,
            union.body.topology.edge_count,
            union.body.topology.face_count,
            union.body.topology.shell_count,
            union.body.topology.solid_count,
        ],
        [24, 36, 14, 1, 1]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        union.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [215.0, 125.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("arc-clipped-corner-rounded-rectangle-union.step");
    let stale_path = directory
        .path()
        .join("stale-arc-clipped-corner-rounded-rectangle-union.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [24, 36, 14, 1, 1]
    );
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    for rejected_segments in [
        translated_containing_rounded_rectangle_segments(110.0, 70.0),
        translated_containing_rounded_rectangle_segments(200.0, 100.0),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_containing_capsule_union_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Union,
        containing_capsule_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_capsule_profile())
    );

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let union = backend
        .fuse_mixed_profile(&base.body, &containing_capsule_planar_segments(), 0.0, 18.0)
        .unwrap();
    let expected_volume = (10_000.0 + 2_500.0 * std::f64::consts::PI) * 18.0;
    assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(
        package.bounds_mm,
        [[-50.0, -20.0, 0.0], [150.0, 80.0, 18.0]]
    );
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("capsule-union.step");
    let stale_path = directory.path().join("stale-capsule-union.step");
    let model = [(ExactBodyPackage::from(package), Transform::identity())];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [8, 12, 6, 1, 1]
    );
    let step_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
    assert!(
        step_volume_error / expected_volume < 0.0032,
        "capsule union STEP relative volume error {}; actual={}, expected={expected_volume}",
        step_volume_error / expected_volume,
        imported.body.topology.volume_mm3
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    let partial_segments = containing_capsule_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line {
                mut start_mm,
                mut end_mm,
            } => {
                start_mm[0] += 80.0;
                end_mm[0] += 80.0;
                ProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                mut start_mm,
                mut end_mm,
                mut center_mm,
                clockwise,
            } => {
                start_mm[0] += 80.0;
                end_mm[0] += 80.0;
                center_mm[0] += 80.0;
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                }
            }
        })
        .collect();
    let partial =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, partial_segments);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&partial.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Union
        ))
    );

    let broader_mixed = [
        containing_capsule_planar_segments()[0],
        containing_capsule_planar_segments()[1],
        containing_capsule_planar_segments()[2],
        PlanarProfileSegment::Line {
            start_mm: [0.0, 80.0],
            end_mm: [0.0, -20.0],
        },
    ];
    let error = backend
        .fuse_mixed_profile(&base.body, &broader_mixed, 0.0, 18.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_side_overlapping_rounded_rectangle_intersection_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Intersect,
        side_overlapping_rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert_eq!(
        profile.rounded_rectangle_side_overlap_area(100.0, 60.0),
        Some(1_800.0)
    );
    assert_eq!(
        request.expected_bounds_mm(),
        [[70.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );
    for (dx, dy, expected_bounds) in [
        (80.0, 0.0, [[70.0, 0.0, 0.0], [100.0, 60.0, 18.0]]),
        (-80.0, 0.0, [[0.0, 0.0, 0.0], [30.0, 60.0, 18.0]]),
        (0.0, 50.0, [[0.0, 40.0, 0.0], [100.0, 60.0, 18.0]]),
        (0.0, -50.0, [[0.0, 0.0, 0.0], [100.0, 20.0, 18.0]]),
    ] {
        let oriented = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let oriented_request =
            ExactFeatureChainRequest::from_snapshot(&oriented.current(), DEFINITION).unwrap();
        assert!(
            oriented_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.rounded_rectangle_side_overlap_area(100.0, 60.0))
                .is_some()
        );
        assert_eq!(oriented_request.expected_bounds_mm(), expected_bounds);
    }

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let intersection = backend
        .common_mixed_profile(
            &base.body,
            &side_overlapping_rounded_rectangle_planar_segments(),
            0.0,
            18.0,
        )
        .unwrap();
    let expected_volume = 1_800.0 * 18.0;
    assert!((intersection.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            intersection.body.topology.vertex_count,
            intersection.body.topology.edge_count,
            intersection.body.topology.face_count,
            intersection.body.topology.shell_count,
            intersection.body.topology.solid_count,
        ],
        [8, 12, 6, 1, 1]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        intersection.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[70.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    let side = package.reference(ExactFaceRole::LinearSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::LinearSide))
    );
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("side-overlap-rounded-rectangle-intersection.step");
    let stale_path = directory
        .path()
        .join("stale-side-overlap-rounded-rectangle-intersection.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [8, 12, 6, 1, 1]
    );
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    let rejected = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Intersect,
        translated_containing_rounded_rectangle_segments(200.0, 0.0),
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Intersect
        ))
    );
}

#[test]
fn scheduler_evaluates_corner_overlapping_rounded_rectangle_intersection_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Intersect,
        corner_overlapping_rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let expected_area = 500.0 + 25.0 * std::f64::consts::PI;
    assert_eq!(
        profile.rounded_rectangle_corner_overlap_area(100.0, 60.0),
        Some(expected_area)
    );
    assert_eq!(
        request.expected_bounds_mm(),
        [[70.0, 40.0, 0.0], [100.0, 60.0, 18.0]]
    );
    for (dx, dy, expected_bounds) in [
        (80.0, 50.0, [[70.0, 40.0, 0.0], [100.0, 60.0, 18.0]]),
        (-80.0, 50.0, [[0.0, 40.0, 0.0], [30.0, 60.0, 18.0]]),
        (80.0, -50.0, [[70.0, 0.0, 0.0], [100.0, 20.0, 18.0]]),
        (-80.0, -50.0, [[0.0, 0.0, 0.0], [30.0, 20.0, 18.0]]),
    ] {
        let oriented = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let oriented_request =
            ExactFeatureChainRequest::from_snapshot(&oriented.current(), DEFINITION).unwrap();
        assert_eq!(
            oriented_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.rounded_rectangle_corner_overlap_area(100.0, 60.0)),
            Some(expected_area)
        );
        assert_eq!(oriented_request.expected_bounds_mm(), expected_bounds);
    }

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let intersection = backend
        .common_mixed_profile(
            &base.body,
            &corner_overlapping_rounded_rectangle_planar_segments(),
            0.0,
            18.0,
        )
        .unwrap();
    let expected_volume = expected_area * 18.0;
    assert!((intersection.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            intersection.body.topology.vertex_count,
            intersection.body.topology.edge_count,
            intersection.body.topology.face_count,
            intersection.body.topology.shell_count,
            intersection.body.topology.solid_count,
        ],
        [10, 15, 7, 1, 1]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        intersection.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[70.0, 40.0, 0.0], [100.0, 60.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 32);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("corner-overlap-rounded-rectangle-intersection.step");
    let stale_path = directory
        .path()
        .join("stale-corner-overlap-rounded-rectangle-intersection.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [10, 15, 7, 1, 1]
    );
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    for rejected_segments in [
        translated_containing_rounded_rectangle_segments(110.0, 70.0),
        translated_containing_rounded_rectangle_segments(200.0, 100.0),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Intersect
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_two_axis_arc_clipped_corner_rounded_rectangle_intersection_and_step_round_trip()
 {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Intersect,
        two_axis_arc_clipped_corner_overlapping_rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 10.0_f64;
    let x_distance = 5.0_f64;
    let y_distance = 5.0_f64;
    let chord_half = (radius * radius - x_distance * x_distance).sqrt();
    let x_limit = (radius * radius - y_distance * y_distance).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let expected_area =
        primitive(x_limit) - primitive(x_distance) - y_distance * (x_limit - x_distance);
    assert_eq!(
        profile.rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(100.0, 60.0),
        Some(expected_area)
    );
    assert_eq!(
        request.expected_bounds_mm(),
        [
            [105.0 - chord_half, 65.0 - chord_half, 0.0],
            [100.0, 60.0, 18.0]
        ]
    );
    for (dx, dy, expected_bounds) in [
        (
            105.0,
            65.0,
            [
                [105.0 - chord_half, 65.0 - chord_half, 0.0],
                [100.0, 60.0, 18.0],
            ],
        ),
        (
            -105.0,
            65.0,
            [
                [0.0, 65.0 - chord_half, 0.0],
                [-5.0 + chord_half, 60.0, 18.0],
            ],
        ),
        (
            105.0,
            -65.0,
            [
                [105.0 - chord_half, 0.0, 0.0],
                [100.0, -5.0 + chord_half, 18.0],
            ],
        ),
        (
            -105.0,
            -65.0,
            [
                [0.0, 0.0, 0.0],
                [-5.0 + chord_half, -5.0 + chord_half, 18.0],
            ],
        ),
    ] {
        let oriented = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let oriented_request =
            ExactFeatureChainRequest::from_snapshot(&oriented.current(), DEFINITION).unwrap();
        assert_eq!(
            oriented_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| {
                    profile.rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(100.0, 60.0)
                }),
            Some(expected_area)
        );
        assert_eq!(oriented_request.expected_bounds_mm(), expected_bounds);
    }

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let intersection = backend
        .common_mixed_profile(
            &base.body,
            &two_axis_arc_clipped_corner_overlapping_rounded_rectangle_planar_segments(),
            0.0,
            18.0,
        )
        .unwrap();
    let expected_volume = expected_area * 18.0;
    assert!((intersection.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            intersection.body.topology.vertex_count,
            intersection.body.topology.edge_count,
            intersection.body.topology.face_count,
            intersection.body.topology.shell_count,
            intersection.body.topology.solid_count,
        ],
        [6, 9, 5, 1, 1]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        intersection.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(
        package.bounds_mm,
        [
            [105.0 - chord_half, 65.0 - chord_half, 0.0],
            [100.0, 60.0, 18.0]
        ]
    );
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 8);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("two-axis-arc-clipped-corner-rounded-rectangle-intersection.step");
    let stale_path = directory
        .path()
        .join("stale-two-axis-arc-clipped-corner-rounded-rectangle-intersection.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [6, 9, 5, 1, 1]
    );
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    for rejected_segments in [
        translated_containing_rounded_rectangle_segments(106.0, 68.0),
        translated_containing_rounded_rectangle_segments(110.0, 70.0),
        translated_containing_rounded_rectangle_segments(200.0, 100.0),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Intersect
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_arc_clipped_corner_rounded_rectangle_intersection_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Intersect,
        arc_clipped_corner_overlapping_rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 10.0_f64;
    let clip_distance = 5.0_f64;
    let straight_extension = 5.0_f64;
    let chord_half = (radius * radius - clip_distance * clip_distance).sqrt();
    let expected_area = (radius - clip_distance) * straight_extension
        + radius * radius * std::f64::consts::FRAC_PI_4
        - 0.5
            * (clip_distance * (radius * radius - clip_distance * clip_distance).sqrt()
                + radius * radius * (clip_distance / radius).asin());
    assert_eq!(
        profile.rounded_rectangle_arc_clipped_corner_overlap_area(100.0, 60.0),
        Some(expected_area)
    );
    assert_eq!(
        request.expected_bounds_mm(),
        [[95.0, 55.0 - chord_half, 0.0], [100.0, 60.0, 18.0]]
    );
    for (dx, dy, expected_bounds) in [
        (
            105.0,
            55.0,
            [[95.0, 55.0 - chord_half, 0.0], [100.0, 60.0, 18.0]],
        ),
        (
            -105.0,
            55.0,
            [[0.0, 55.0 - chord_half, 0.0], [5.0, 60.0, 18.0]],
        ),
        (
            105.0,
            -55.0,
            [[95.0, 0.0, 0.0], [100.0, 5.0 + chord_half, 18.0]],
        ),
        (
            -105.0,
            -55.0,
            [[0.0, 0.0, 0.0], [5.0, 5.0 + chord_half, 18.0]],
        ),
        (
            95.0,
            65.0,
            [[95.0 - chord_half, 55.0, 0.0], [100.0, 60.0, 18.0]],
        ),
        (
            -95.0,
            65.0,
            [[0.0, 55.0, 0.0], [5.0 + chord_half, 60.0, 18.0]],
        ),
        (
            95.0,
            -65.0,
            [[95.0 - chord_half, 0.0, 0.0], [100.0, 5.0, 18.0]],
        ),
        (
            -95.0,
            -65.0,
            [[0.0, 0.0, 0.0], [5.0 + chord_half, 5.0, 18.0]],
        ),
    ] {
        let oriented = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let oriented_request =
            ExactFeatureChainRequest::from_snapshot(&oriented.current(), DEFINITION).unwrap();
        assert_eq!(
            oriented_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| {
                    profile.rounded_rectangle_arc_clipped_corner_overlap_area(100.0, 60.0)
                }),
            Some(expected_area)
        );
        assert_eq!(oriented_request.expected_bounds_mm(), expected_bounds);
    }

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let intersection = backend
        .common_mixed_profile(
            &base.body,
            &arc_clipped_corner_overlapping_rounded_rectangle_planar_segments(),
            0.0,
            18.0,
        )
        .unwrap();
    let expected_volume = expected_area * 18.0;
    assert!((intersection.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            intersection.body.topology.vertex_count,
            intersection.body.topology.edge_count,
            intersection.body.topology.face_count,
            intersection.body.topology.shell_count,
            intersection.body.topology.solid_count,
        ],
        [8, 12, 6, 1, 1]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        intersection.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(
        package.bounds_mm,
        [[95.0, 55.0 - chord_half, 0.0], [100.0, 60.0, 18.0]]
    );
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 16);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("arc-clipped-corner-rounded-rectangle-intersection.step");
    let stale_path = directory
        .path()
        .join("stale-arc-clipped-corner-rounded-rectangle-intersection.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [8, 12, 6, 1, 1]
    );
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    for rejected_segments in [
        translated_containing_rounded_rectangle_segments(110.0, 60.0),
        translated_containing_rounded_rectangle_segments(200.0, 100.0),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Intersect
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_contained_rounded_rectangle_intersection_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Intersect,
        rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_rounded_rectangle_profile())
    );

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let intersection = backend
        .common_mixed_profile(&base.body, &rounded_rectangle_planar_segments(), 0.0, 18.0)
        .unwrap();
    let expected_volume = (2_144.0 + 64.0 * std::f64::consts::PI) * 18.0;
    assert!((intersection.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(intersection.body.topology.solid_count, 1);

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        intersection.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[20.0, 10.0, 0.0], [80.0, 50.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("rounded-rectangle-intersection.step");
    let stale_path = directory
        .path()
        .join("stale-rounded-rectangle-intersection.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [16, 24, 10, 1, 1]
    );
    let step_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
    assert!(
        step_volume_error / expected_volume < 0.0032,
        "rounded rectangle intersection STEP relative volume error {}; actual={}, expected={expected_volume}",
        step_volume_error / expected_volume,
        imported.body.topology.volume_mm3
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    let partial_segments = rounded_rectangle_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line {
                mut start_mm,
                mut end_mm,
            } => {
                start_mm[0] -= 25.0;
                end_mm[0] -= 25.0;
                ProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                mut start_mm,
                mut end_mm,
                mut center_mm,
                clockwise,
            } => {
                start_mm[0] -= 25.0;
                end_mm[0] -= 25.0;
                center_mm[0] -= 25.0;
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                }
            }
        })
        .collect();
    let partial =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, partial_segments);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&partial.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Intersect
        ))
    );
    let containing = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Intersect,
        containing_rounded_rectangle_segments(),
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&containing.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Intersect
        ))
    );

    let mut broader_mixed = rounded_rectangle_planar_segments();
    broader_mixed[3] = PlanarProfileSegment::CircularArc {
        start_mm: [80.0, 42.0],
        end_mm: [70.0, 50.0],
        center_mm: [70.0, 42.0],
        clockwise: false,
    };
    broader_mixed[4] = PlanarProfileSegment::Line {
        start_mm: [70.0, 50.0],
        end_mm: [28.0, 50.0],
    };
    let error = backend
        .common_mixed_profile(&base.body, &broader_mixed, 0.0, 18.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_contained_capsule_intersection_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let document = capsule_boolean_document(18.0, BooleanOperation::Intersect);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_capsule_profile())
    );

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let intersection = backend
        .common_mixed_profile(&base.body, &capsule_planar_segments(), 0.0, 18.0)
        .unwrap();
    let expected_volume = (400.0 + 100.0 * std::f64::consts::PI) * 18.0;
    assert!((intersection.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[20.0, 20.0, 0.0], [60.0, 40.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("capsule-intersection.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [8, 12, 6, 1, 1]
    );
    let expected_volume = (400.0 + 100.0 * std::f64::consts::PI) * 18.0;
    let step_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
    assert!(
        step_volume_error / expected_volume < 0.0032,
        "capsule intersection STEP relative volume error {}; actual={}, expected={expected_volume}",
        step_volume_error / expected_volume,
        imported.body.topology.volume_mm3
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    let partial_segments = capsule_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line {
                mut start_mm,
                mut end_mm,
            } => {
                start_mm[0] -= 25.0;
                end_mm[0] -= 25.0;
                ProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                mut start_mm,
                mut end_mm,
                mut center_mm,
                clockwise,
            } => {
                start_mm[0] -= 25.0;
                end_mm[0] -= 25.0;
                center_mm[0] -= 25.0;
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                }
            }
        })
        .collect();
    let partial =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, partial_segments);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&partial.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Intersect
        ))
    );

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let broader_mixed = [
        PlanarProfileSegment::Line {
            start_mm: [30.0, 20.0],
            end_mm: [50.0, 20.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [50.0, 20.0],
            end_mm: [50.0, 40.0],
            center_mm: [50.0, 30.0],
            clockwise: false,
        },
        PlanarProfileSegment::Line {
            start_mm: [50.0, 40.0],
            end_mm: [30.0, 40.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [30.0, 40.0],
            end_mm: [30.0, 20.0],
        },
    ];
    let error = backend
        .common_mixed_profile(&base.body, &broader_mixed, -1.0, 20.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_side_overlapping_rounded_rectangle_split_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (dx, dy, expected_overlap_area)) in [
        (80.0, 0.0, 1_800.0),
        (-80.0, 0.0, 1_800.0),
        (0.0, 50.0, 2_000.0),
        (0.0, -50.0, 2_000.0),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Split,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        assert_eq!(
            request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.rounded_rectangle_side_overlap_area(100.0, 60.0)),
            Some(expected_overlap_area)
        );

        let split = backend
            .split_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
                0.0,
                18.0,
            )
            .unwrap();
        assert_eq!(
            [
                split.body.topology.vertex_count,
                split.body.topology.edge_count,
                split.body.topology.face_count,
                split.body.topology.shell_count,
                split.body.topology.solid_count,
            ],
            [12, 20, 11, 2, 2]
        );
        assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            split.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        assert_eq!(package.vertices.len(), 16);
        assert_eq!(package.triangles.len(), 24);
        assert_eq!(package.references.len(), 3);
        let side = package.reference(ExactFaceRole::LinearSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert_eq!(
            package
                .triangles
                .iter()
                .filter(|triangle| triangle.face_role == Some(ExactFaceRole::LinearSide))
                .count(),
            4
        );
        assert_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("side-overlap-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-side-overlap-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = ExactBackend::new()
            .import_step(step_path.to_str().unwrap())
            .unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            [16, 24, 12, 2, 2]
        );
        assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
        assert_eq!(document.current().revision_id(), before_revision);
        assert_eq!(document.current().canonical_digest(), before_digest);
        assert_eq!(document.visible_undo_steps(), before_undo);

        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: EXTRUSION,
                    dimension: Dimension::new("24", 24.0).unwrap(),
                },
            ]))
            .unwrap();
        std::fs::write(&stale_path, b"preserved destination").unwrap();
        assert!(
            supervisor
                .export_current_model_step(&document.current(), &model, &stale_path)
                .is_err()
        );
        assert_eq!(
            std::fs::read(&stale_path).unwrap(),
            b"preserved destination"
        );
    }

    for rejected_segments in [
        translated_containing_rounded_rectangle_segments(110.0, 0.0),
        translated_containing_rounded_rectangle_segments(120.0, 0.0),
        containing_rounded_rectangle_segments(),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Split,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_corner_overlapping_rounded_rectangle_split_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let expected_overlap_area = 500.0 + 25.0 * std::f64::consts::PI;

    for (index, (dx, dy)) in [(80.0, 50.0), (-80.0, 50.0), (80.0, -50.0), (-80.0, -50.0)]
        .into_iter()
        .enumerate()
    {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Split,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        assert_eq!(
            request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.rounded_rectangle_corner_overlap_area(100.0, 60.0)),
            Some(expected_overlap_area)
        );

        let split = backend
            .split_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
                0.0,
                18.0,
            )
            .unwrap();
        let direct_topology = [
            split.body.topology.vertex_count,
            split.body.topology.edge_count,
            split.body.topology.face_count,
            split.body.topology.shell_count,
            split.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [16, 26, 13, 2, 2]);
        assert_eq!(split.body.topology.solid_count, 2);
        assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            split.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        assert_eq!(package.references.len(), 3);
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(package.vertices.len(), 84);
        assert_eq!(package.triangles.len(), 160);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("corner-overlap-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = ExactBackend::new()
            .import_step(step_path.to_str().unwrap())
            .unwrap();
        let imported_topology = [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ];
        assert_eq!(imported_topology, [24, 36, 16, 2, 2]);
        assert_eq!(imported.body.topology.solid_count, 2);
        assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);

        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: EXTRUSION,
                    dimension: Dimension::new("24", 24.0).unwrap(),
                },
            ]))
            .unwrap();
        let stale_path = directory
            .path()
            .join(format!("stale-corner-overlap-split-{index}.step"));
        std::fs::write(&stale_path, b"preserved destination").unwrap();
        assert!(
            supervisor
                .export_current_model_step(&document.current(), &model, &stale_path)
                .is_err()
        );
        assert_eq!(
            std::fs::read(&stale_path).unwrap(),
            b"preserved destination"
        );
    }

    for rejected_segments in [
        far_arc_clipped_corner_overlapping_rounded_rectangle_segments(),
        translated_containing_rounded_rectangle_segments(110.0, 70.0),
        translated_containing_rounded_rectangle_segments(120.0, 0.0),
        containing_rounded_rectangle_segments(),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Split,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_arc_clipped_corner_rounded_rectangle_split_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let radius = 10.0_f64;
    let clip_distance = 5.0_f64;
    let straight_extension = 5.0_f64;
    let expected_overlap_area = (radius - clip_distance) * straight_extension
        + radius * radius * std::f64::consts::FRAC_PI_4
        - 0.5
            * (clip_distance * (radius * radius - clip_distance * clip_distance).sqrt()
                + radius * radius * (clip_distance / radius).asin());

    for (index, (dx, dy)) in [
        (105.0, 55.0),
        (-105.0, 55.0),
        (105.0, -55.0),
        (-105.0, -55.0),
        (95.0, 65.0),
        (-95.0, 65.0),
        (95.0, -65.0),
        (-95.0, -65.0),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Split,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        assert_eq!(
            request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| {
                    profile.rounded_rectangle_arc_clipped_corner_overlap_area(100.0, 60.0)
                }),
            Some(expected_overlap_area)
        );

        let split = backend
            .split_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
                0.0,
                18.0,
            )
            .unwrap();
        let direct_topology = [
            split.body.topology.vertex_count,
            split.body.topology.edge_count,
            split.body.topology.face_count,
            split.body.topology.shell_count,
            split.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [14, 23, 12, 2, 2]);
        assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            split.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        assert_eq!(package.references.len(), 3);
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("arc-clipped-corner-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-arc-clipped-corner-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = ExactBackend::new()
            .import_step(step_path.to_str().unwrap())
            .unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            [20, 30, 14, 2, 2]
        );
        assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
        assert_eq!(document.current().revision_id(), before_revision);
        assert_eq!(document.current().canonical_digest(), before_digest);
        assert_eq!(document.visible_undo_steps(), before_undo);

        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: EXTRUSION,
                    dimension: Dimension::new("24", 24.0).unwrap(),
                },
            ]))
            .unwrap();
        std::fs::write(&stale_path, b"preserved destination").unwrap();
        assert!(
            supervisor
                .export_current_model_step(&document.current(), &model, &stale_path)
                .is_err()
        );
        assert_eq!(
            std::fs::read(&stale_path).unwrap(),
            b"preserved destination"
        );
    }

    for rejected_segments in [
        far_arc_clipped_corner_overlapping_rounded_rectangle_segments(),
        translated_containing_rounded_rectangle_segments(110.0, 60.0),
        translated_containing_rounded_rectangle_segments(200.0, 100.0),
        containing_rounded_rectangle_segments(),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Split,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_two_axis_arc_clipped_corner_rounded_rectangle_split_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let radius = 10.0_f64;
    let x_distance = 5.0_f64;
    let y_distance = 5.0_f64;
    let x_limit = (radius * radius - y_distance * y_distance).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let expected_overlap_area =
        primitive(x_limit) - primitive(x_distance) - y_distance * (x_limit - x_distance);

    for (index, (dx, dy)) in [
        (105.0, 65.0),
        (-105.0, 65.0),
        (105.0, -65.0),
        (-105.0, -65.0),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Split,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        assert_eq!(
            request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| {
                    profile.rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(100.0, 60.0)
                }),
            Some(expected_overlap_area)
        );

        let split = backend
            .split_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
                0.0,
                18.0,
            )
            .unwrap();
        let direct_topology = [
            split.body.topology.vertex_count,
            split.body.topology.edge_count,
            split.body.topology.face_count,
            split.body.topology.shell_count,
            split.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 20, 11, 2, 2]);
        assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            split.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        assert_eq!(package.references.len(), 3);
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(package.vertices.len(), 36);
        assert_eq!(package.triangles.len(), 64);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("two-axis-arc-clipped-corner-split-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-two-axis-arc-clipped-corner-split-{index}.step"
        ));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = ExactBackend::new()
            .import_step(step_path.to_str().unwrap())
            .unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            [16, 24, 12, 2, 2]
        );
        assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);

        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: EXTRUSION,
                    dimension: Dimension::new("24", 24.0).unwrap(),
                },
            ]))
            .unwrap();
        std::fs::write(&stale_path, b"preserved destination").unwrap();
        assert!(
            supervisor
                .export_current_model_step(&document.current(), &model, &stale_path)
                .is_err()
        );
        assert_eq!(
            std::fs::read(&stale_path).unwrap(),
            b"preserved destination"
        );
    }

    for rejected_segments in [
        far_arc_clipped_corner_overlapping_rounded_rectangle_segments(),
        translated_containing_rounded_rectangle_segments(110.0, 70.0),
        translated_containing_rounded_rectangle_segments(200.0, 100.0),
        containing_rounded_rectangle_segments(),
    ] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Split,
            rejected_segments,
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_contained_rounded_rectangle_split_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Split,
        rounded_rectangle_segments(),
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_rounded_rectangle_profile())
    );

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let split = backend
        .split_mixed_profile(&base.body, &rounded_rectangle_planar_segments(), 0.0, 18.0)
        .unwrap();
    let inner_volume = (2_144.0 + 64.0 * std::f64::consts::PI) * 18.0;
    let outer_volume = 108_000.0 - inner_volume;
    assert!(inner_volume > 0.0 && outer_volume > 0.0);
    assert!((inner_volume + outer_volume - 108_000.0).abs() < f64::EPSILON);
    assert_eq!(split.body.topology.solid_count, 2);
    assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        split.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("rounded-rectangle-split.step");
    let stale_path = directory.path().join("stale-rounded-rectangle-split.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [40, 60, 24, 2, 2]
    );
    assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    let partial_segments = rounded_rectangle_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line {
                mut start_mm,
                mut end_mm,
            } => {
                start_mm[0] -= 25.0;
                end_mm[0] -= 25.0;
                ProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                mut start_mm,
                mut end_mm,
                mut center_mm,
                clockwise,
            } => {
                start_mm[0] -= 25.0;
                end_mm[0] -= 25.0;
                center_mm[0] -= 25.0;
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                }
            }
        })
        .collect();
    let partial =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, partial_segments);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&partial.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Split
        ))
    );
    let containing = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Split,
        containing_rounded_rectangle_segments(),
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&containing.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Split
        ))
    );

    let mut broader_mixed = rounded_rectangle_planar_segments();
    broader_mixed[3] = PlanarProfileSegment::CircularArc {
        start_mm: [80.0, 42.0],
        end_mm: [70.0, 50.0],
        center_mm: [70.0, 42.0],
        clockwise: false,
    };
    broader_mixed[4] = PlanarProfileSegment::Line {
        start_mm: [70.0, 50.0],
        end_mm: [28.0, 50.0],
    };
    let error = backend
        .split_mixed_profile(&base.body, &broader_mixed, 0.0, 18.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_contained_capsule_split_and_step_round_trip() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = capsule_boolean_document(18.0, BooleanOperation::Split);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_capsule_profile())
    );

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let split = backend
        .split_mixed_profile(&base.body, &capsule_planar_segments(), 0.0, 18.0)
        .unwrap();
    assert_eq!(split.body.topology.solid_count, 2);
    assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("capsule-split.step");
    let stale_path = directory.path().join("stale-capsule-split.step");
    let model = [(ExactBodyPackage::from(package), Transform::identity())];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(imported.body.topology.shell_count, 2);
    assert_eq!(imported.body.topology.solid_count, 2);
    assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    let partial_segments = capsule_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line {
                mut start_mm,
                mut end_mm,
            } => {
                start_mm[0] -= 25.0;
                end_mm[0] -= 25.0;
                ProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                mut start_mm,
                mut end_mm,
                mut center_mm,
                clockwise,
            } => {
                start_mm[0] -= 25.0;
                end_mm[0] -= 25.0;
                center_mm[0] -= 25.0;
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                }
            }
        })
        .collect();
    let partial =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, partial_segments);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&partial.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Split
        ))
    );

    let broader_mixed = [
        capsule_planar_segments()[0],
        capsule_planar_segments()[1],
        capsule_planar_segments()[2],
        PlanarProfileSegment::Line {
            start_mm: [30.0, 40.0],
            end_mm: [30.0, 20.0],
        },
    ];
    let error = backend
        .split_mixed_profile(&base.body, &broader_mixed, 0.0, 18.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_exact_capsule_pocket_over_worker_protocol() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let document = capsule_pocket_document(18.0, 8.0);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_capsule_profile())
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    for role in [ExactFaceRole::CutLinear, ExactFaceRole::PocketFloor] {
        let reference = package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, CUT_PROFILE);
        assert!(reference.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(role))
        );
    }
    assert!(package.vertices.len() > 64);
    assert_closed_manifold(&package);
}

#[test]
fn scheduler_evaluates_contained_line_arc_d_profile_intersection_over_worker_protocol() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let document = line_arc_d_boolean_document(18.0, BooleanOperation::Intersect);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.bounds_mm, [[20.0, 20.0, 0.0], [40.0, 30.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(
        package.vertices.len() > 8,
        "derived mesh must preserve the arc"
    );
    assert_closed_manifold(&package);

    let union_document = line_arc_d_boolean_document(18.0, BooleanOperation::Union);
    let union_snapshot = union_document.current();
    let union_request =
        ExactFeatureChainRequest::from_snapshot(&union_snapshot, DEFINITION).unwrap();
    assert_eq!(union_request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    let union_package = supervisor.evaluate_rectangle(&union_request).unwrap();
    assert!(union_package.is_current(&union_snapshot));
    for (actual, expected) in union_package
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([-20.0, -100.0, 0.0, 110.0, 160.0, 18.0])
    {
        assert!((actual - expected).abs() < 1.0e-9);
    }
    let union_side = union_package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(union_side.profile_feature_id, CUT_PROFILE);
    assert!(union_side.has_valid_lineage());
    assert!(
        union_package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(union_package.vertices.len() > 8);
    assert_closed_manifold(&union_package);

    let split_document = line_arc_d_boolean_document(18.0, BooleanOperation::Split);
    let split_snapshot = split_document.current();
    let split_request =
        ExactFeatureChainRequest::from_snapshot(&split_snapshot, DEFINITION).unwrap();
    assert_eq!(split_request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    let split_package = supervisor.evaluate_rectangle(&split_request).unwrap();
    let repeated_split = supervisor.evaluate_rectangle(&split_request).unwrap();
    assert!(split_package.is_current(&split_snapshot));
    assert_eq!(split_package.identity, repeated_split.identity);
    assert_eq!(split_package.references, repeated_split.references);
    assert_eq!(
        split_package.bounds_mm,
        [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );
    let split_side = split_package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(split_side.profile_feature_id, CUT_PROFILE);
    assert!(split_side.has_valid_lineage());
    assert!(
        split_package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(split_package.vertices.len() > 16);
    assert_closed_manifold(&split_package);
}

#[test]
fn scheduler_evaluates_exact_line_arc_d_profile_pocket_over_worker_protocol() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let document = line_arc_d_pocket_document(18.0, 8.0);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    assert_eq!(
        package
            .reference(ExactFaceRole::CutLinear)
            .unwrap()
            .profile_feature_id,
        CUT_PROFILE
    );
    assert_eq!(
        package
            .reference(ExactFaceRole::PocketFloor)
            .unwrap()
            .profile_feature_id,
        CUT_PROFILE
    );
    assert!(
        package
            .references
            .iter()
            .all(|reference| reference.has_valid_lineage())
    );
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::PocketFloor))
    );
    assert_closed_manifold(&package);
}

#[test]
fn scheduler_evaluates_contained_slanted_polygon_union_and_rejects_partial_overlap() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let points = [[-20.0, -10.0], [110.0, -15.0], [125.0, 70.0], [-15.0, 80.0]];
    let document = polygon_boolean_document(&points, 18.0, BooleanOperation::Union);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    assert_eq!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .map(|profile| profile.segments.len()),
        Some(4)
    );
    assert_eq!(
        request.expected_bounds_mm(),
        [[-20.0, -15.0, 0.0], [125.0, 80.0, 18.0]]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.bounds_mm, request.expected_bounds_mm());
    assert_eq!(package.vertices.len(), 8);
    assert_eq!(package.triangles.len(), 12);
    assert_eq!(package.references.len(), 3);
    let side = package.reference(ExactFaceRole::LinearSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert_closed_manifold(&package);

    let partial = polygon_boolean_document(
        &[[20.0, 15.0], [115.0, 12.0], [110.0, 45.0], [18.0, 42.0]],
        18.0,
        BooleanOperation::Union,
    )
    .current();
    assert!(ExactFeatureChainRequest::from_snapshot(&partial, DEFINITION).is_err());
}

#[test]
fn scheduler_evaluates_contained_slanted_polygon_intersection_and_rejects_invalid_overlap() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let points = [
        [12.0, 10.0],
        [70.0, 8.0],
        [88.0, 40.0],
        [55.0, 52.0],
        [15.0, 45.0],
    ];
    let document = polygon_boolean_document(&points, 18.0, BooleanOperation::Intersect);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    assert_eq!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .map(|profile| profile.segments.len()),
        Some(5)
    );
    assert_eq!(
        request.expected_bounds_mm(),
        [[12.0, 8.0, 0.0], [88.0, 52.0, 18.0]]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.bounds_mm, request.expected_bounds_mm());
    assert_eq!(package.vertices.len(), 10);
    assert_eq!(package.triangles.len(), 16);
    assert_eq!(package.references.len(), 3);
    let side = package.reference(ExactFaceRole::LinearSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert_closed_manifold(&package);

    for rejected in [
        vec![[20.0, 10.0], [105.0, 8.0], [80.0, 50.0], [15.0, 45.0]],
        vec![[120.0, 10.0], [145.0, 12.0], [140.0, 40.0], [118.0, 35.0]],
        vec![[20.0, 10.0], [70.0, 45.0], [20.0, 45.0], [70.0, 10.0]],
    ] {
        let rejected = polygon_boolean_document(&rejected, 18.0, BooleanOperation::Intersect);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_contained_slanted_polygon_split_and_rejects_invalid_partitions() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let points = [
        [12.0, 10.0],
        [70.0, 8.0],
        [88.0, 40.0],
        [55.0, 52.0],
        [15.0, 45.0],
    ];
    let document = polygon_boolean_document(&points, 18.0, BooleanOperation::Split);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    assert_eq!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .map(|profile| profile.segments.len()),
        Some(5)
    );
    assert_eq!(
        request.expected_bounds_mm(),
        [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, request.expected_bounds_mm());
    assert_eq!(package.vertices.len(), 8);
    assert_eq!(package.triangles.len(), 12);
    assert_eq!(package.references.len(), 3);
    assert!(package.references.iter().all(|reference| {
        reference.has_valid_lineage()
            && matches!(
                reference.role(),
                Some(ExactFaceRole::Top | ExactFaceRole::Bottom | ExactFaceRole::East)
            )
    }));
    assert_closed_manifold(&package);

    for rejected in [
        vec![[20.0, 10.0], [100.0, 8.0], [80.0, 50.0], [15.0, 45.0]],
        vec![[120.0, 10.0], [145.0, 12.0], [140.0, 40.0], [118.0, 35.0]],
        vec![[20.0, 10.0], [70.0, 45.0], [20.0, 45.0], [70.0, 10.0]],
    ] {
        let rejected = polygon_boolean_document(&rejected, 18.0, BooleanOperation::Split);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_exact_circle_extrusion_and_circular_cut_with_typed_stable_faces() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();

    let circle_document = circle_document([30.0, 20.0], 10.0, 18.0);
    let circle_snapshot = circle_document.current();
    let circle_request =
        ExactFeatureChainRequest::from_snapshot(&circle_snapshot, DEFINITION).unwrap();
    assert_eq!(circle_request.evaluator(), EXACT_CIRCLE_EVALUATOR_V1);
    assert_eq!(
        circle_request.expected_bounds_mm(),
        [[20.0, 10.0, 0.0], [40.0, 30.0, 18.0]]
    );
    let circle_package = supervisor.evaluate_rectangle(&circle_request).unwrap();
    assert!(circle_package.is_current(&circle_snapshot));
    assert_eq!(circle_package.vertices.len(), 66);
    assert_eq!(circle_package.triangles.len(), 128);
    assert_closed_manifold(&circle_package);
    for (role, expected_type) in [
        (ExactFaceRole::Top, "planar_face"),
        (ExactFaceRole::Bottom, "planar_face"),
        (ExactFaceRole::CircleSide, "cylindrical_face"),
    ] {
        let reference = circle_package.reference(role).unwrap();
        assert_eq!(reference.expected_type, expected_type);
        assert_eq!(reference.profile_feature_id, PROFILE);
        assert!(reference.has_valid_lineage());
    }

    let cut_document =
        circular_boolean_document(100.0, 60.0, 18.0, [40.0, 30.0], 10.0, BooleanOperation::Cut);
    let cut_snapshot = cut_document.current();
    let cut_request = ExactFeatureChainRequest::from_snapshot(&cut_snapshot, DEFINITION).unwrap();
    assert_eq!(cut_request.evaluator(), EXACT_CIRCULAR_CUT_EVALUATOR_V1);
    assert_eq!(
        cut_request.expected_bounds_mm(),
        [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );
    let cut_package = supervisor.evaluate_rectangle(&cut_request).unwrap();
    assert!(cut_package.is_current(&cut_snapshot));
    assert_eq!(cut_package.vertices.len(), 128);
    assert_eq!(cut_package.triangles.len(), 256);
    assert_closed_manifold(&cut_package);
    for (role, expected_type, profile_id) in [
        (ExactFaceRole::Top, "planar_face", PROFILE),
        (ExactFaceRole::Bottom, "planar_face", PROFILE),
        (ExactFaceRole::East, "planar_face", PROFILE),
        (ExactFaceRole::CutCircle, "cylindrical_face", CUT_PROFILE),
    ] {
        let reference = cut_package.reference(role).unwrap();
        assert_eq!(reference.expected_type, expected_type);
        assert_eq!(reference.profile_feature_id, profile_id);
        assert!(reference.has_valid_lineage());
    }

    for (center, radius) in [
        ([10.0, 30.0], 10.0),
        ([5.0, 30.0], 10.0),
        ([120.0, 30.0], 5.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }

    let pocket_document = circular_pocket_document(100.0, 60.0, 18.0, [40.0, 30.0], 10.0, 7.0);
    let pocket_snapshot = pocket_document.current();
    let pocket_request =
        ExactFeatureChainRequest::from_snapshot(&pocket_snapshot, DEFINITION).unwrap();
    assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(pocket_request.pocket_depth_bits, Some(7.0_f64.to_bits()));
    let pocket_package = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    assert!(pocket_package.is_current(&pocket_snapshot));
    assert_closed_manifold(&pocket_package);
    for (role, expected_type, profile_id) in [
        (ExactFaceRole::Top, "planar_face", PROFILE),
        (ExactFaceRole::Bottom, "planar_face", PROFILE),
        (ExactFaceRole::East, "planar_face", PROFILE),
        (ExactFaceRole::CutCircle, "cylindrical_face", CUT_PROFILE),
    ] {
        let reference = pocket_package.reference(role).unwrap();
        assert_eq!(reference.expected_type, expected_type);
        assert_eq!(reference.profile_feature_id, profile_id);
        assert!(reference.has_valid_lineage());
    }

    let union_document = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [50.0, 30.0],
        70.0,
        BooleanOperation::Union,
    );
    let union_snapshot = union_document.current();
    let union_request =
        ExactFeatureChainRequest::from_snapshot(&union_snapshot, DEFINITION).unwrap();
    assert_eq!(union_request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    assert_eq!(
        union_request.expected_bounds_mm(),
        [[-20.0, -40.0, 0.0], [120.0, 100.0, 18.0]]
    );
    let union_package = supervisor.evaluate_rectangle(&union_request).unwrap();
    let repeated_union = supervisor.evaluate_rectangle(&union_request).unwrap();
    assert!(union_package.is_current(&union_snapshot));
    assert_eq!(union_package.identity, repeated_union.identity);
    assert_eq!(union_package.vertices.len(), 66);
    assert_eq!(union_package.triangles.len(), 128);
    assert_closed_manifold(&union_package);
    for (role, profile_id) in [
        (ExactFaceRole::Top, PROFILE),
        (ExactFaceRole::Bottom, PROFILE),
        (ExactFaceRole::CircleSide, CUT_PROFILE),
    ] {
        let reference = union_package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, profile_id);
        assert!(reference.has_valid_lineage());
    }

    for (center, radius) in [
        ([40.0, 30.0], 10.0),
        ([50.0, 30.0], 55.0),
        ([150.0, 30.0], 10.0),
        ([50.0, 30.0], 58.309_518_948_453_004),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }

    let intersection_document = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [40.0, 30.0],
        10.0,
        BooleanOperation::Intersect,
    );
    let intersection_snapshot = intersection_document.current();
    let intersection_request =
        ExactFeatureChainRequest::from_snapshot(&intersection_snapshot, DEFINITION).unwrap();
    assert_eq!(
        intersection_request.evaluator(),
        EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1
    );
    assert_eq!(
        intersection_request.expected_bounds_mm(),
        [[30.0, 20.0, 0.0], [50.0, 40.0, 18.0]]
    );
    let intersection_package = supervisor
        .evaluate_rectangle(&intersection_request)
        .unwrap();
    assert!(intersection_package.is_current(&intersection_snapshot));
    assert_eq!(intersection_package.vertices.len(), 66);
    assert_eq!(intersection_package.triangles.len(), 128);
    assert_closed_manifold(&intersection_package);
    for (role, profile_id) in [
        (ExactFaceRole::Top, PROFILE),
        (ExactFaceRole::Bottom, PROFILE),
        (ExactFaceRole::CircleSide, CUT_PROFILE),
    ] {
        let reference = intersection_package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, profile_id);
        assert!(reference.has_valid_lineage());
    }

    let split_document = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [40.0, 30.0],
        10.0,
        BooleanOperation::Split,
    );
    let split_snapshot = split_document.current();
    let split_request =
        ExactFeatureChainRequest::from_snapshot(&split_snapshot, DEFINITION).unwrap();
    assert_eq!(split_request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    assert_eq!(
        split_request.expected_bounds_mm(),
        [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );
    let split_package = supervisor.evaluate_rectangle(&split_request).unwrap();
    assert!(split_package.is_current(&split_snapshot));
    assert_eq!(split_package.vertices.len(), 8);
    assert_eq!(split_package.triangles.len(), 12);
    assert_closed_manifold(&split_package);
    for role in [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ] {
        let reference = split_package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, PROFILE);
        assert!(reference.has_valid_lineage());
    }

    for (center, radius) in [
        ([10.0, 30.0], 10.0),
        ([5.0, 30.0], 10.0),
        ([120.0, 30.0], 5.0),
    ] {
        for operation in [BooleanOperation::Intersect, BooleanOperation::Split] {
            let rejected = circular_boolean_document(100.0, 60.0, 18.0, center, radius, operation);
            assert!(
                ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err()
            );
        }
    }
}

#[test]
fn scheduler_evaluates_exact_mixed_line_arc_profile_with_derived_non_authoritative_mesh() {
    const MESH_DEFINITION: DefinitionId = DefinitionId(90);
    const MESH_FEATURE: FeatureId = FeatureId(91);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Mixed profile definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Exact D profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [0.0, 0.0],
                            end_mm: [20.0, 0.0],
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [20.0, 0.0],
                            end_mm: [0.0, 0.0],
                            center_mm: [10.0, 0.0],
                            clockwise: false,
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Exact D extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::new("8", 8.0).unwrap(),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_ARC_PROFILE_EVALUATOR_V1);
    assert_eq!(
        request.expected_bounds_mm(),
        [[0.0, 0.0, 0.0], [20.0, 10.0, 8.0]]
    );
    let mixed = request.mixed_profile.as_ref().unwrap();
    assert_eq!(mixed.segments.len(), 2);
    assert!((f64::from_bits(mixed.area_bits) - 50.0 * std::f64::consts::PI).abs() < 1.0e-9);

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.vertices.len(), 66);
    assert_eq!(package.triangles.len(), 128);
    assert_closed_manifold(&package);
    let arc = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(arc.expected_type, "face");
    assert_eq!(arc.source_element_id, "profile.edge.arc.0");
    assert!(arc.has_valid_lineage());

    let exact = ExactBodyPackage::from(package.clone());
    let conversion = exact
        .detached_mesh_conversion_batch(
            &snapshot,
            MESH_DEFINITION,
            "Derived D mesh",
            MESH_FEATURE,
            "Derived tessellation",
        )
        .unwrap();
    document.apply_batch(&conversion).unwrap();
    let converted = document.current();
    let FeatureKind::MeshBody(spec) = converted.feature(MESH_FEATURE).unwrap().kind() else {
        panic!("detached conversion must create a mesh body");
    };
    let MeshAuthority::ExactConversion(authority) = &spec.authority else {
        panic!("render tessellation must remain an explicit exact conversion");
    };
    assert_eq!(
        authority.source_result_fingerprint,
        package.identity.result_fingerprint
    );
    assert_eq!(authority.source_evaluator, EXACT_ARC_PROFILE_EVALUATOR_V1);
    assert!(
        authority
            .unsupported_semantics
            .contains(&"analytic_surfaces".to_owned())
    );
}

#[test]
fn scheduler_evaluates_exact_linear_polygon_and_rejects_self_intersection_before_dispatch() {
    const LINEAR_DEFINITION: DefinitionId = DefinitionId(92);
    const LINEAR_PROFILE: FeatureId = FeatureId(93);
    const LINEAR_EXTRUSION: FeatureId = FeatureId(94);
    let commands = |segments| {
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: LINEAR_DEFINITION,
                name: "Linear polygon definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: LINEAR_PROFILE,
                definition_id: LINEAR_DEFINITION,
                name: "Linear polygon".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: LINEAR_EXTRUSION,
                definition_id: LINEAR_DEFINITION,
                name: "Linear polygon extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: LINEAR_PROFILE,
                    height: Dimension::new("8", 8.0).unwrap(),
                },
            },
        ])
    };
    let mut document = DocumentStore::new();
    document
        .apply_batch(&commands(vec![
            ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [20.0, 0.0],
            },
            ProfileSegment::Line {
                start_mm: [20.0, 0.0],
                end_mm: [5.0, 10.0],
            },
            ProfileSegment::Line {
                start_mm: [5.0, 10.0],
                end_mm: [0.0, 0.0],
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, LINEAR_DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_LINEAR_PROFILE_EVALUATOR_V1);
    assert_eq!(
        request.expected_bounds_mm(),
        [[0.0, 0.0, 0.0], [20.0, 10.0, 8.0]]
    );
    let profile = request.mixed_profile.as_ref().unwrap();
    assert!(profile.has_only_line_segments());
    assert_eq!(f64::from_bits(profile.area_bits), 100.0);

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.vertices.len(), 6);
    assert_eq!(package.triangles.len(), 8);
    assert_closed_manifold(&package);
    let side = package.reference(ExactFaceRole::LinearSide).unwrap();
    assert_eq!(side.expected_type, "planar_face");
    assert_eq!(side.source_element_id, "profile.edge.line.0");
    assert!(side.has_valid_lineage());

    let mut invalid = DocumentStore::new();
    invalid
        .apply_batch(&commands(vec![
            ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [10.0, 10.0],
            },
            ProfileSegment::Line {
                start_mm: [10.0, 10.0],
                end_mm: [0.0, 10.0],
            },
            ProfileSegment::Line {
                start_mm: [0.0, 10.0],
                end_mm: [10.0, 0.0],
            },
            ProfileSegment::Line {
                start_mm: [10.0, 0.0],
                end_mm: [0.0, 0.0],
            },
        ]))
        .unwrap();
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&invalid.current(), LINEAR_DEFINITION),
        Err(ExactProductError::UnsupportedProfile)
    );
}

#[test]
fn exact_request_rejects_two_edge_segment_profile_before_worker_dispatch() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Unsupported segment definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Line segment profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [0.0, 0.0],
                            end_mm: [10.0, 0.0],
                        },
                        ProfileSegment::Line {
                            start_mm: [10.0, 0.0],
                            end_mm: [0.0, 0.0],
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Unsupported exact extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::new("18", 18.0).unwrap(),
                },
            },
        ]))
        .unwrap();

    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&document.current(), DEFINITION),
        Err(ExactProductError::UnsupportedProfile)
    );
}

#[test]
fn scheduler_evaluates_canonical_depth_limited_pocket_with_stable_floor_and_walls() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = rectangle_document(100.0, 60.0, 18.0);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Pocket profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[30.0, 20.0], [50.0, 20.0], [50.0, 35.0], [30.0, 35.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "6 mm pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: EXTRUSION,
                    profile: CUT_PROFILE,
                    depth: Dimension::new("6 mm", 6.0).unwrap(),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.producer_feature_id(), POCKET);
    assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(request.pocket_depth_bits, Some(6.0_f64.to_bits()));

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity.producer_feature_id, POCKET);
    assert_eq!(package.identity.evaluator, EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(package.vertices.len(), 16);
    assert_eq!(package.triangles.len(), 28);
    assert_eq!(package.references.len(), 8);
    for role in [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
        ExactFaceRole::PocketFloor,
        ExactFaceRole::PocketWest,
        ExactFaceRole::PocketEast,
        ExactFaceRole::PocketSouth,
        ExactFaceRole::PocketNorth,
    ] {
        let reference = package
            .reference(role)
            .expect("stable pocket face reference");
        assert_eq!(reference.producer_feature_id, POCKET);
        assert_eq!(
            reference.profile_feature_id,
            if matches!(
                role,
                ExactFaceRole::Top | ExactFaceRole::Bottom | ExactFaceRole::East
            ) {
                PROFILE
            } else {
                CUT_PROFILE
            }
        );
        assert!(reference.has_valid_lineage());
    }
    let floor_ray = Ray::new(Vec3::new(40.0, 27.5, 30.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    let results =
        ExactResultRegistry::accept(&snapshot, [Arc::new(package.clone().into())]).unwrap();
    let projection = ExactInteractionProjection::from_snapshot(&snapshot, &results);
    assert_eq!(
        projection
            .exact_pick(floor_ray)
            .and_then(|hit| hit.target.body.role()),
        Some(ExactFaceRole::PocketFloor)
    );

    for reference in package.references.clone() {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    let reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&document.current()))
            .unwrap();
    assert_eq!(
        reopened.source_schema(),
        ketchup_core::persistence::CURRENT_SCHEMA
    );
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        document.current().canonical_digest()
    );
    assert_eq!(reopened.snapshot().exact_reference_evidence().count(), 8);
    assert!(matches!(
        reopened.snapshot().feature(POCKET).unwrap().kind(),
        FeatureKind::Pocket { depth, .. } if depth.millimetres() == 6.0
    ));
}

#[test]
fn scheduler_evaluates_overlapping_boolean_union_as_one_larger_exact_body() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let document = boolean_document(
        100.0,
        60.0,
        18.0,
        [80.0, 0.0, 40.0, 60.0],
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
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [120.0, 60.0, 18.0]]);

    let results = ExactResultRegistry::accept(&snapshot, [Arc::new(package.into())]).unwrap();
    let projection = ExactInteractionProjection::from_snapshot(&snapshot, &results);
    let tool_region_ray =
        Ray::new(Vec3::new(110.0, 27.5, 30.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    assert_eq!(
        projection
            .exact_pick(tool_region_ray)
            .and_then(|hit| hit.target.body.role()),
        Some(ExactFaceRole::Top)
    );
}

#[test]
fn scheduler_evaluates_bounded_intersect_with_deterministic_exact_lineage() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let document = boolean_document(
        100.0,
        60.0,
        18.0,
        [70.0, 20.0, 60.0, 40.0],
        BooleanOperation::Intersect,
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.producer_feature_id(), THROUGH_CUT);
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    assert_eq!(
        request.expected_bounds_mm(),
        [[70.0, 20.0, 0.0], [100.0, 60.0, 18.0]]
    );

    let first = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(first.is_current(&snapshot));
    assert_eq!(first.identity, repeated.identity);
    assert_eq!(first.references, repeated.references);
    assert_eq!(first.identity.producer_feature_id, THROUGH_CUT);
    assert_eq!(
        first.identity.evaluator,
        EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1
    );
    assert_eq!(first.bounds_mm, request.expected_bounds_mm());
    assert_eq!(first.vertices.len(), 8);
    assert_eq!(first.triangles.len(), 12);
    assert_eq!(first.references.len(), 3);
    assert!(first.references.iter().all(|reference| {
        reference.has_valid_lineage()
            && reference.matches_request(&request)
            && reference.producer_feature_id == THROUGH_CUT
    }));

    let results = ExactResultRegistry::accept(&snapshot, [Arc::new(first.into())]).unwrap();
    let projection = ExactInteractionProjection::from_snapshot(&snapshot, &results);
    let overlap_ray = Ray::new(Vec3::new(85.0, 40.0, 30.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    assert_eq!(
        projection
            .exact_pick(overlap_ray)
            .and_then(|hit| hit.target.body.role()),
        Some(ExactFaceRole::Top)
    );
}

#[test]
fn scheduler_evaluates_bounded_split_with_deterministic_exact_lineage() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let document = boolean_document(
        100.0,
        60.0,
        18.0,
        [70.0, 20.0, 60.0, 40.0],
        BooleanOperation::Split,
    );
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.producer_feature_id(), THROUGH_CUT);
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    assert_eq!(
        request.expected_bounds_mm(),
        [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );

    let first = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(first.is_current(&snapshot));
    assert_eq!(first.identity, repeated.identity);
    assert_eq!(first.references, repeated.references);
    assert_eq!(first.identity.producer_feature_id, THROUGH_CUT);
    assert_eq!(first.identity.evaluator, EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    assert_eq!(first.bounds_mm, request.expected_bounds_mm());
    assert_eq!(first.references.len(), 3);
    assert!(first.references.iter().all(|reference| {
        reference.has_valid_lineage()
            && reference.matches_request(&request)
            && reference.producer_feature_id == THROUGH_CUT
    }));

    let results = ExactResultRegistry::accept(&snapshot, [Arc::new(first.into())]).unwrap();
    let projection = ExactInteractionProjection::from_snapshot(&snapshot, &results);
    let target_ray = Ray::new(Vec3::new(30.0, 30.0, 30.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    assert_eq!(
        projection
            .exact_pick(target_ray)
            .and_then(|hit| hit.target.body.role()),
        Some(ExactFaceRole::Top)
    );
}

#[test]
fn scheduler_evaluates_bounded_planar_offset_with_deterministic_exact_lineage() {
    const OFFSET_DEFINITION: DefinitionId = DefinitionId(701);
    const OFFSET_PROFILE: FeatureId = FeatureId(702);
    const OFFSET_FEATURE: FeatureId = FeatureId(703);

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut fingerprints = Vec::new();
    for distance_mm in [5.0, -7.5] {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: OFFSET_DEFINITION,
                    name: "Offset profile".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: OFFSET_PROFILE,
                    definition_id: OFFSET_DEFINITION,
                    name: "Source rectangle".to_owned(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[10.0, 20.0], [110.0, 20.0], [110.0, 100.0], [10.0, 100.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: OFFSET_FEATURE,
                    definition_id: OFFSET_DEFINITION,
                    name: "Planar offset".to_owned(),
                    kind: FeatureKind::PlanarOffset {
                        profile: OFFSET_PROFILE,
                        distance: Dimension::new(distance_mm.to_string(), distance_mm).unwrap(),
                    },
                },
            ]))
            .unwrap();
        let snapshot = document.current();
        let request =
            ExactPlanarOffsetRequest::from_snapshot(&snapshot, OFFSET_DEFINITION).unwrap();
        assert_eq!(request.producer_feature_id(), OFFSET_FEATURE);
        assert_eq!(request.evaluator(), EXACT_PLANAR_OFFSET_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            if distance_mm > 0.0 {
                [[5.0, 15.0, 0.0], [115.0, 105.0, 0.0]]
            } else {
                [[17.5, 27.5, 0.0], [102.5, 92.5, 0.0]]
            }
        );

        let first = supervisor.evaluate_planar_offset(&request).unwrap();
        let repeated = supervisor.evaluate_planar_offset(&request).unwrap();
        assert!(first.is_current(&snapshot));
        assert_eq!(first.identity, repeated.identity);
        assert_eq!(first.reference, repeated.reference);
        assert_eq!(first.bounds_mm, request.expected_bounds_mm());
        assert!((first.area_mm2 - request.expected_area_mm2()).abs() <= 1.0e-6);
        assert_eq!(first.vertices.len(), 4);
        assert_eq!(first.triangles.len(), 2);
        assert_eq!(
            first.reference.role(),
            Some(ExactFaceRole::PlanarOffsetFace)
        );
        assert!(first.reference.has_valid_lineage());
        assert!(first.reference.matches_planar_offset_request(&request));
        assert_eq!(first.reference.producer_feature_id, OFFSET_FEATURE);
        assert_eq!(first.reference.profile_feature_id, OFFSET_PROFILE);
        fingerprints.push(first.identity.result_fingerprint);
    }
    assert_ne!(fingerprints[0], fingerprints[1]);
}

#[test]
fn scheduler_evaluates_bounded_sweep_with_deterministic_exact_lineage() {
    const DEFINITION: DefinitionId = DefinitionId(711);
    const PROFILE: FeatureId = FeatureId(712);
    const PATH: FeatureId = FeatureId(713);
    const SWEEP: FeatureId = FeatureId(714);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Sweep definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangular section".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-5.0, -10.0], [5.0, -10.0], [5.0, 10.0], [-5.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Straight path".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [0.0, 0.0],
                        end_mm: [75.0, 100.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Bounded sweep".to_owned(),
                kind: FeatureKind::Sweep {
                    profile: PROFILE,
                    path: PATH,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let request = ExactSweepRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.producer_feature_id(), SWEEP);
    assert_eq!(request.evaluator(), EXACT_SWEEP_EVALUATOR_V1);
    assert_eq!(request.path_length_mm(), 125.0);
    assert_eq!(request.expected_volume_mm3(), 25_000.0);
    assert_eq!(
        request.expected_bounds_mm(),
        [[-4.0, -3.0, -10.0], [79.0, 103.0, 10.0]]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let first = supervisor.evaluate_sweep(&request).unwrap();
    let repeated = supervisor.evaluate_sweep(&request).unwrap();
    assert!(first.is_current(&snapshot));
    assert_eq!(first.identity, repeated.identity);
    assert_eq!(first.references, repeated.references);
    assert_eq!(first.bounds_mm, request.expected_bounds_mm());
    assert!((first.volume_mm3 - request.expected_volume_mm3()).abs() <= 1.0e-6);
    assert_eq!(first.vertices.len(), 8);
    assert_eq!(first.triangles.len(), 12);
    assert_eq!(
        first
            .references
            .iter()
            .map(|reference| reference.role().unwrap())
            .collect::<Vec<_>>(),
        [
            ExactFaceRole::SweepStart,
            ExactFaceRole::SweepEnd,
            ExactFaceRole::SweepSide0,
            ExactFaceRole::SweepSide1,
            ExactFaceRole::SweepSide2,
            ExactFaceRole::SweepSide3,
        ]
    );
    assert!(first.references.iter().all(|reference| {
        reference.has_valid_lineage() && reference.matches_sweep_request(&request)
    }));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameDefinition {
                id: DEFINITION,
                name: "Changed sweep definition".to_owned(),
            },
        ]))
        .unwrap();
    assert!(!first.is_current(&document.current()));
}

#[test]
fn scheduler_evaluates_bounded_spline_loft_with_deterministic_exact_lineage() {
    const DEFINITION: DefinitionId = DefinitionId(721);
    const LOWER: FeatureId = FeatureId(722);
    const UPPER: FeatureId = FeatureId(723);
    const LOFT: FeatureId = FeatureId(724);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Loft definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: LOWER,
                definition_id: DEFINITION,
                name: "Lower spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![
                        [-20.0, -10.0],
                        [20.0, -10.0],
                        [20.0, 10.0],
                        [-20.0, 10.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: UPPER,
                definition_id: DEFINITION,
                name: "Upper spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: LOFT,
                definition_id: DEFINITION,
                name: "Bounded loft".to_owned(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: LOWER,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: UPPER,
                            elevation_mm: 80.0,
                        },
                    ],
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let request = ExactLoftRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.producer_feature_id(), LOFT);
    assert_eq!(request.evaluator(), EXACT_LOFT_EVALUATOR_V1);
    assert_eq!(request.control_point_count(), 8);
    assert_eq!(request.protocol_values().len(), 21);

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let first = supervisor.evaluate_loft(&request).unwrap();
    let repeated = supervisor.evaluate_loft(&request).unwrap();
    assert!(first.is_current(&snapshot));
    assert_eq!(first.identity, repeated.identity);
    assert_eq!(first.references, repeated.references);
    assert_eq!(first.topology_counts, [2, 3, 3, 1, 1]);
    assert!(first.volume_mm3 > 0.0);
    assert_eq!(
        first
            .references
            .iter()
            .map(|reference| reference.role().unwrap())
            .collect::<Vec<_>>(),
        [
            ExactFaceRole::LoftStart,
            ExactFaceRole::LoftEnd,
            ExactFaceRole::LoftSide,
        ]
    );
    assert_eq!(first.references[0].profile_feature_id, LOWER);
    assert_eq!(first.references[1].profile_feature_id, UPPER);
    assert_eq!(first.references[2].profile_feature_id, LOWER);
    assert!(first.references.iter().all(|reference| {
        reference.has_valid_lineage() && reference.matches_loft_request(&request)
    }));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameDefinition {
                id: DEFINITION,
                name: "Changed Loft definition".to_owned(),
            },
        ]))
        .unwrap();
    assert!(!first.is_current(&document.current()));
}

#[test]
fn scheduler_evaluates_general_box_shell_fillet_and_chamfer_with_stable_exact_roles() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut fingerprints = Vec::new();
    for finish in [
        None,
        Some(BottleEdgeFinishKind::Fillet),
        Some(BottleEdgeFinishKind::Chamfer),
    ] {
        let document = general_box_shell_document(finish);
        let snapshot = document.current();
        let request =
            ExactFeatureChainRequest::from_snapshot(&snapshot, DefinitionId(950)).unwrap();
        let shell = request.shell.as_ref().unwrap();
        assert_eq!(
            request.producer_feature_id(),
            shell
                .edge_finish_feature_id
                .unwrap_or(shell.shell_feature_id)
        );
        assert_eq!(
            request.evaluator(),
            if finish.is_some() {
                EXACT_BOX_FINISH_EVALUATOR_V1
            } else {
                EXACT_BOX_SHELL_EVALUATOR_V1
            }
        );

        let first = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(first.is_current(&snapshot));
        assert_eq!(first.identity, repeated.identity);
        assert_eq!(first.references, repeated.references);
        assert_eq!(
            first
                .references
                .iter()
                .map(|reference| reference.role().unwrap())
                .collect::<Vec<_>>(),
            [
                ExactFaceRole::BoxShellRim,
                ExactFaceRole::BoxShellOuterBottom,
                ExactFaceRole::BoxShellOuterEast,
            ]
        );
        assert!(first.references.iter().all(|reference| {
            reference.has_valid_lineage()
                && reference.producer_feature_id == request.producer_feature_id()
                && reference.canonical_input_digest == request.canonical_input_digest
                && reference.evaluator == request.evaluator()
        }));
        fingerprints.push(first.identity.result_fingerprint);
    }
    fingerprints.sort();
    fingerprints.dedup();
    assert_eq!(fingerprints.len(), 3);
}

#[test]
fn scheduler_evaluates_general_polygon_and_segment_revolves_with_stable_exact_roles() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let polygon = general_revolve_document(false);
    let polygon_snapshot = polygon.current();
    let polygon_request =
        ExactRevolveRequest::from_snapshot(&polygon_snapshot, DefinitionId(901)).unwrap();
    assert!(polygon_request.general);
    assert_eq!(polygon_request.axis_start_mm(), [0.0, -10.0]);
    assert_eq!(polygon_request.axis_end_mm(), [0.0, 10.0]);
    assert_eq!(polygon_request.angle_degrees(), 180.0);

    let first = supervisor.evaluate_revolve(&polygon_request).unwrap();
    let repeated = supervisor.evaluate_revolve(&polygon_request).unwrap();
    assert!(first.is_current(&polygon_snapshot));
    assert_eq!(first.identity, repeated.identity);
    assert_eq!(first.references, repeated.references);
    assert_eq!(
        first.identity.result_fingerprint,
        repeated.identity.result_fingerprint
    );
    assert_eq!(first.references.len(), 4);
    assert_eq!(
        first
            .references
            .iter()
            .map(|reference| reference.role().unwrap())
            .collect::<Vec<_>>(),
        [
            ExactFaceRole::RevolveSide0,
            ExactFaceRole::RevolveSide1,
            ExactFaceRole::RevolveStart,
            ExactFaceRole::RevolveEnd,
        ]
    );
    assert!(first.references.iter().all(|reference| {
        reference.has_valid_lineage()
            && reference.canonical_input_digest == polygon_request.canonical_input_digest
            && reference.evaluator == polygon_request.evaluator()
    }));

    let segment = general_revolve_document(true);
    let segment_snapshot = segment.current();
    let segment_request =
        ExactRevolveRequest::from_snapshot(&segment_snapshot, DefinitionId(901)).unwrap();
    assert!(segment_request.general);
    assert!(segment_request.segments.is_some());
    let segment_package = supervisor.evaluate_revolve(&segment_request).unwrap();
    let segment_repeated = supervisor.evaluate_revolve(&segment_request).unwrap();
    assert!(segment_package.is_current(&segment_snapshot));
    assert_eq!(segment_package.identity, segment_repeated.identity);
    assert_eq!(segment_package.references, segment_repeated.references);
    assert_ne!(
        first.identity.result_fingerprint,
        segment_package.identity.result_fingerprint
    );
    assert_ne!(
        polygon_request.canonical_input_digest,
        segment_request.canonical_input_digest
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
    assert_eq!(
        reopened.source_schema(),
        ketchup_core::persistence::CURRENT_SCHEMA
    );
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
fn worker_exports_d_profile_through_cut_as_rereadable_step_and_preserves_stale_destination() {
    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("d-profile-through-cut.step");
    let stale_path = directory.path().join("stale-d-profile.step");
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = line_arc_d_boolean_document(18.0, BooleanOperation::Cut);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let model = [(ExactBodyPackage::from(package), Transform::identity())];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();

    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();

    let raw = std::fs::read_to_string(&step_path).unwrap();
    assert!(raw.starts_with("ISO-10303-21;"));
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(imported.body.topology.vertex_count, 12);
    assert_eq!(imported.body.topology.edge_count, 18);
    assert_eq!(imported.body.topology.face_count, 8);
    assert_eq!(imported.body.topology.shell_count, 1);
    assert_eq!(imported.body.topology.solid_count, 1);
    let expected_volume = 108_000.0 - 900.0 * std::f64::consts::PI;
    let volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
    assert!(
        volume_error < 2.0,
        "STEP round-trip volume error {volume_error} exceeds tolerance; actual={}, expected={expected_volume}",
        imported.body.topology.volume_mm3
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );
}

#[test]
fn worker_exports_d_profile_union_as_rereadable_step_and_preserves_stale_destination() {
    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("d-profile-union.step");
    let stale_path = directory.path().join("stale-d-profile-union.step");
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = line_arc_d_boolean_document(18.0, BooleanOperation::Union);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let model = [(ExactBodyPackage::from(package), Transform::identity())];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();

    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();

    let raw = std::fs::read_to_string(&step_path).unwrap();
    assert!(raw.starts_with("ISO-10303-21;"));
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(imported.body.topology.vertex_count, 4);
    assert_eq!(imported.body.topology.edge_count, 6);
    assert_eq!(imported.body.topology.face_count, 4);
    assert_eq!(imported.body.topology.shell_count, 1);
    assert_eq!(imported.body.topology.solid_count, 1);
    for (actual, expected) in [
        imported.body.topology.bounds_mm.min.x,
        imported.body.topology.bounds_mm.min.y,
        imported.body.topology.bounds_mm.min.z,
        imported.body.topology.bounds_mm.max.x,
        imported.body.topology.bounds_mm.max.y,
        imported.body.topology.bounds_mm.max.z,
    ]
    .into_iter()
    .zip([-20.0, -100.0, 0.0, 110.0, 160.0, 18.0])
    {
        assert!((actual - expected).abs() < 1.0e-6);
    }
    let expected_volume = 152_100.0 * std::f64::consts::PI;
    let volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
    assert!(
        volume_error / expected_volume < 0.0032,
        "STEP round-trip relative volume error {} exceeds tolerance; actual={}, expected={expected_volume}",
        volume_error / expected_volume,
        imported.body.topology.volume_mm3
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );
}

#[test]
fn worker_exports_d_profile_intersection_as_rereadable_step_and_preserves_stale_destination() {
    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("d-profile-intersection.step");
    let stale_path = directory.path().join("stale-d-profile-intersection.step");
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = line_arc_d_boolean_document(18.0, BooleanOperation::Intersect);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let model = [(ExactBodyPackage::from(package), Transform::identity())];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();

    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();

    let raw = std::fs::read_to_string(&step_path).unwrap();
    assert!(raw.starts_with("ISO-10303-21;"));
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(imported.body.topology.vertex_count, 4);
    assert_eq!(imported.body.topology.edge_count, 6);
    assert_eq!(imported.body.topology.face_count, 4);
    assert_eq!(imported.body.topology.shell_count, 1);
    assert_eq!(imported.body.topology.solid_count, 1);
    for (actual, expected) in [
        imported.body.topology.bounds_mm.min.x,
        imported.body.topology.bounds_mm.min.y,
        imported.body.topology.bounds_mm.min.z,
        imported.body.topology.bounds_mm.max.x,
        imported.body.topology.bounds_mm.max.y,
        imported.body.topology.bounds_mm.max.z,
    ]
    .into_iter()
    .zip([20.0, 20.0, 0.0, 40.0, 30.0, 18.0])
    {
        assert!((actual - expected).abs() < 1.0e-6);
    }
    let expected_volume = 900.0 * std::f64::consts::PI;
    let volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
    assert!(
        volume_error / expected_volume < 0.0032,
        "STEP round-trip relative volume error {} exceeds tolerance; actual={}, expected={expected_volume}",
        volume_error / expected_volume,
        imported.body.topology.volume_mm3
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );
}

#[test]
fn worker_exports_d_profile_split_as_two_rereadable_solids_and_preserves_stale_destination() {
    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("d-profile-split.step");
    let stale_path = directory.path().join("stale-d-profile-split.step");
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = line_arc_d_boolean_document(18.0, BooleanOperation::Split);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let model = [(ExactBodyPackage::from(package), Transform::identity())];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();

    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();

    let raw = std::fs::read_to_string(&step_path).unwrap();
    assert!(raw.starts_with("ISO-10303-21;"));
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(imported.body.topology.shell_count, 2);
    assert_eq!(imported.body.topology.solid_count, 2);
    assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 1.0e-6);
    for (actual, expected) in [
        imported.body.topology.bounds_mm.min.x,
        imported.body.topology.bounds_mm.min.y,
        imported.body.topology.bounds_mm.min.z,
        imported.body.topology.bounds_mm.max.x,
        imported.body.topology.bounds_mm.max.y,
        imported.body.topology.bounds_mm.max.z,
    ]
    .into_iter()
    .zip([0.0, 0.0, 0.0, 100.0, 60.0, 18.0])
    {
        assert!((actual - expected).abs() < 1.0e-6);
    }
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    let direct_base = ExactBackend::new()
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let broader_mixed = [
        PlanarProfileSegment::CircularArc {
            start_mm: [20.0, 20.0],
            end_mm: [40.0, 20.0],
            center_mm: [30.0, 20.0],
            clockwise: true,
        },
        PlanarProfileSegment::Line {
            start_mm: [40.0, 20.0],
            end_mm: [30.0, 30.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [30.0, 30.0],
            end_mm: [20.0, 20.0],
        },
    ];
    let error = ExactBackend::new()
        .split_mixed_profile(&direct_base.body, &broader_mixed, 0.0, 18.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn worker_exports_d_profile_pocket_as_rereadable_step_and_preserves_stale_destination() {
    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("d-profile-pocket.step");
    let stale_path = directory.path().join("stale-d-profile-pocket.step");
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = line_arc_d_pocket_document(18.0, 8.0);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let model = [(ExactBodyPackage::from(package), Transform::identity())];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();

    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();

    let raw = std::fs::read_to_string(&step_path).unwrap();
    assert!(raw.starts_with("ISO-10303-21;"));
    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(imported.body.topology.vertex_count, 12);
    assert_eq!(imported.body.topology.edge_count, 18);
    assert_eq!(imported.body.topology.face_count, 9);
    assert_eq!(imported.body.topology.shell_count, 1);
    assert_eq!(imported.body.topology.solid_count, 1);
    let expected_volume = 108_000.0 - 400.0 * std::f64::consts::PI;
    let volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
    assert!(
        volume_error < 1.0,
        "STEP round-trip volume error {volume_error} exceeds tolerance; actual={}, expected={expected_volume}",
        imported.body.topology.volume_mm3
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );
}

#[test]
fn worker_exports_transformed_current_model_as_rereadable_step_and_rejects_stale_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("current-model.step");
    let stale_path = directory.path().join("stale-current-model.step");
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = rectangle_document(100.0, 60.0, 18.0);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let translated = Transform::from_translation(150.0, 25.0, 5.0).unwrap();
    let model = [
        (
            ExactBodyPackage::from(package.clone()),
            Transform::identity(),
        ),
        (ExactBodyPackage::from(package.clone()), translated),
    ];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();

    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();

    let imported = ExactBackend::new()
        .import_step(step_path.to_str().unwrap())
        .unwrap();
    assert_eq!(imported.body.topology.solid_count, 2);
    assert!((imported.body.topology.volume_mm3 - 216_000.0).abs() < 1.0e-6);
    assert!((imported.body.topology.bounds_mm.min.x - package.bounds_mm[0][0]).abs() < 1.0e-6);
    assert!(
        (imported.body.topology.bounds_mm.max.x - (package.bounds_mm[1][0] + 150.0)).abs() < 1.0e-6
    );
    assert!(
        (imported.body.topology.bounds_mm.max.y - (package.bounds_mm[1][1] + 25.0)).abs() < 1.0e-6
    );
    assert!(
        (imported.body.topology.bounds_mm.max.z - (package.bounds_mm[1][2] + 5.0)).abs() < 1.0e-6
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&document.current(), &model, &stale_path)
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
    assert_eq!(
        reopened.source_schema(),
        ketchup_core::persistence::CURRENT_SCHEMA
    );
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
                kind: FeatureKind::full_revolve(BOTTLE_CONTROL),
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_SHELL,
                definition_id: BOTTLE_DEFINITION,
                name: "Bottle shell".to_owned(),
                kind: FeatureKind::Shell {
                    target: BOTTLE_REVOLVE,
                    removed_faces: vec![
                        StableFaceRole::new(BOTTLE_SHELL_OPENING_FACE_ROLE).unwrap(),
                    ],
                    thickness: Dimension::new("2", 2.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_FINISH,
                definition_id: BOTTLE_DEFINITION,
                name: "Shoulder finish".to_owned(),
                kind: FeatureKind::BottleEdgeFinish {
                    target: BOTTLE_SHELL,
                    edges: vec![StableEdgeRole::new(BOTTLE_SHOULDER_EDGE_ROLE).unwrap()],
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
                removed_faces: vec![StableFaceRole::new(BOTTLE_SHELL_OPENING_FACE_ROLE).unwrap()],
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
                kind: FeatureKind::full_revolve(BOTTLE_PROFILE),
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

fn circle_segments(center: [f64; 2], radius: f64) -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::CircularArc {
            start_mm: [center[0] + radius, center[1]],
            end_mm: [center[0] - radius, center[1]],
            center_mm: center,
            clockwise: false,
        },
        ProfileSegment::CircularArc {
            start_mm: [center[0] - radius, center[1]],
            end_mm: [center[0] + radius, center[1]],
            center_mm: center,
            clockwise: false,
        },
    ]
}

fn circle_document(center: [f64; 2], radius: f64, height: f64) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Exact circle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Circle profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: circle_segments(center, radius),
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Circle extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::new(height.to_string(), height).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(13),
                definition_id: DEFINITION,
                name: "Circle occurrence".to_owned(),
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

fn circular_boolean_document(
    width: f64,
    depth: f64,
    height: f64,
    center: [f64; 2],
    radius: f64,
    operation: BooleanOperation,
) -> DocumentStore {
    let mut document = rectangle_document(width, depth, height);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Circular cut profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: circle_segments(center, radius),
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "Circular cut tool".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: CUT_PROFILE,
                    height: Dimension::new(height.to_string(), height).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: THROUGH_CUT,
                definition_id: DEFINITION,
                name: "Circular boolean".to_owned(),
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

fn circular_pocket_document(
    width: f64,
    depth: f64,
    height: f64,
    center: [f64; 2],
    radius: f64,
    pocket_depth: f64,
) -> DocumentStore {
    let mut document = rectangle_document(width, depth, height);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Circular pocket profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: circle_segments(center, radius),
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Circular pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: EXTRUSION,
                    profile: CUT_PROFILE,
                    depth: Dimension::new(pocket_depth.to_string(), pocket_depth).unwrap(),
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn assert_closed_manifold(package: &ExactRenderPackage) {
    let mut edge_use = BTreeMap::<(u32, u32), usize>::new();
    for triangle in &package.triangles {
        let [a, b, c] = triangle
            .vertex_indices
            .map(|index| package.vertices[index as usize].position_mm);
        let first = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let second = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            first[1] * second[2] - first[2] * second[1],
            first[2] * second[0] - first[0] * second[2],
            first[0] * second[1] - first[1] * second[0],
        ];
        assert!(cross.into_iter().map(|value| value * value).sum::<f64>() > 1.0e-12);
        for [from, to] in [
            [triangle.vertex_indices[0], triangle.vertex_indices[1]],
            [triangle.vertex_indices[1], triangle.vertex_indices[2]],
            [triangle.vertex_indices[2], triangle.vertex_indices[0]],
        ] {
            *edge_use.entry((from.min(to), from.max(to))).or_default() += 1;
        }
    }
    assert!(edge_use.values().all(|count| *count == 2));
}

fn assert_consistently_oriented_closed_manifold(package: &ExactRenderPackage) {
    let mut directed_edges = BTreeMap::<(u32, u32), usize>::new();
    for triangle in &package.triangles {
        for [from, to] in [
            [triangle.vertex_indices[0], triangle.vertex_indices[1]],
            [triangle.vertex_indices[1], triangle.vertex_indices[2]],
            [triangle.vertex_indices[2], triangle.vertex_indices[0]],
        ] {
            *directed_edges.entry((from, to)).or_default() += 1;
        }
    }
    assert!(directed_edges.iter().all(|(&(from, to), count)| {
        *count == 1 && directed_edges.get(&(to, from)) == Some(&1)
    }));
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

fn line_arc_d_boolean_document(height: f64, operation: BooleanOperation) -> DocumentStore {
    let mut document = rectangle_document(100.0, 60.0, height);
    let segments = if operation == BooleanOperation::Union {
        vec![
            ProfileSegment::CircularArc {
                start_mm: [-20.0, -100.0],
                end_mm: [-20.0, 160.0],
                center_mm: [-20.0, 30.0],
                clockwise: false,
            },
            ProfileSegment::Line {
                start_mm: [-20.0, 160.0],
                end_mm: [-20.0, -100.0],
            },
        ]
    } else {
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 20.0],
                end_mm: [40.0, 20.0],
                center_mm: [30.0, 20.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 20.0],
                end_mm: [20.0, 20.0],
            },
        ]
    };
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "D cut profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "D cut tool".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: CUT_PROFILE,
                    height: Dimension::new(height.to_string(), height).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: THROUGH_CUT,
                definition_id: DEFINITION,
                name: "D boolean".to_owned(),
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

fn containing_capsule_planar_segments() -> [PlanarProfileSegment; 4] {
    [
        PlanarProfileSegment::Line {
            start_mm: [0.0, -20.0],
            end_mm: [100.0, -20.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [100.0, -20.0],
            end_mm: [100.0, 80.0],
            center_mm: [100.0, 30.0],
            clockwise: false,
        },
        PlanarProfileSegment::Line {
            start_mm: [100.0, 80.0],
            end_mm: [0.0, 80.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [0.0, 80.0],
            end_mm: [0.0, -20.0],
            center_mm: [0.0, 30.0],
            clockwise: false,
        },
    ]
}

fn capsule_planar_segments() -> [PlanarProfileSegment; 4] {
    [
        PlanarProfileSegment::Line {
            start_mm: [30.0, 20.0],
            end_mm: [50.0, 20.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [50.0, 20.0],
            end_mm: [50.0, 40.0],
            center_mm: [50.0, 30.0],
            clockwise: false,
        },
        PlanarProfileSegment::Line {
            start_mm: [50.0, 40.0],
            end_mm: [30.0, 40.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [30.0, 40.0],
            end_mm: [30.0, 20.0],
            center_mm: [30.0, 30.0],
            clockwise: false,
        },
    ]
}

fn containing_capsule_segments() -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::Line {
            start_mm: [0.0, -20.0],
            end_mm: [100.0, -20.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [100.0, -20.0],
            end_mm: [100.0, 80.0],
            center_mm: [100.0, 30.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [100.0, 80.0],
            end_mm: [0.0, 80.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [0.0, 80.0],
            end_mm: [0.0, -20.0],
            center_mm: [0.0, 30.0],
            clockwise: false,
        },
    ]
}

fn capsule_segments() -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::Line {
            start_mm: [30.0, 20.0],
            end_mm: [50.0, 20.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [50.0, 20.0],
            end_mm: [50.0, 40.0],
            center_mm: [50.0, 30.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [50.0, 40.0],
            end_mm: [30.0, 40.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [30.0, 40.0],
            end_mm: [30.0, 20.0],
            center_mm: [30.0, 30.0],
            clockwise: false,
        },
    ]
}

fn rounded_rectangle_segments() -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::Line {
            start_mm: [28.0, 10.0],
            end_mm: [72.0, 10.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [72.0, 10.0],
            end_mm: [80.0, 18.0],
            center_mm: [72.0, 18.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [80.0, 18.0],
            end_mm: [80.0, 42.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [80.0, 42.0],
            end_mm: [72.0, 50.0],
            center_mm: [72.0, 42.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [72.0, 50.0],
            end_mm: [28.0, 50.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [28.0, 50.0],
            end_mm: [20.0, 42.0],
            center_mm: [28.0, 42.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [20.0, 42.0],
            end_mm: [20.0, 18.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [20.0, 18.0],
            end_mm: [28.0, 10.0],
            center_mm: [28.0, 18.0],
            clockwise: false,
        },
    ]
}

fn rounded_rectangle_planar_segments() -> Vec<PlanarProfileSegment> {
    rounded_rectangle_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                PlanarProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            },
        })
        .collect()
}

fn containing_rounded_rectangle_segments() -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::Line {
            start_mm: [0.0, -10.0],
            end_mm: [100.0, -10.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [100.0, -10.0],
            end_mm: [110.0, 0.0],
            center_mm: [100.0, 0.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [110.0, 0.0],
            end_mm: [110.0, 60.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [110.0, 60.0],
            end_mm: [100.0, 70.0],
            center_mm: [100.0, 60.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [100.0, 70.0],
            end_mm: [0.0, 70.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [0.0, 70.0],
            end_mm: [-10.0, 60.0],
            center_mm: [0.0, 60.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [-10.0, 60.0],
            end_mm: [-10.0, 0.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [-10.0, 0.0],
            end_mm: [0.0, -10.0],
            center_mm: [0.0, 0.0],
            clockwise: false,
        },
    ]
}

fn containing_rounded_rectangle_planar_segments() -> Vec<PlanarProfileSegment> {
    containing_rounded_rectangle_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                PlanarProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            },
        })
        .collect()
}

fn translated_containing_rounded_rectangle_segments(dx: f64, dy: f64) -> Vec<ProfileSegment> {
    containing_rounded_rectangle_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line {
                mut start_mm,
                mut end_mm,
            } => {
                start_mm[0] += dx;
                start_mm[1] += dy;
                end_mm[0] += dx;
                end_mm[1] += dy;
                ProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                mut start_mm,
                mut end_mm,
                mut center_mm,
                clockwise,
            } => {
                start_mm[0] += dx;
                start_mm[1] += dy;
                end_mm[0] += dx;
                end_mm[1] += dy;
                center_mm[0] += dx;
                center_mm[1] += dy;
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                }
            }
        })
        .collect()
}

fn translated_containing_rounded_rectangle_planar_segments(
    dx: f64,
    dy: f64,
) -> Vec<PlanarProfileSegment> {
    translated_containing_rounded_rectangle_segments(dx, dy)
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                PlanarProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            },
        })
        .collect()
}

fn side_overlapping_rounded_rectangle_segments() -> Vec<ProfileSegment> {
    translated_containing_rounded_rectangle_segments(80.0, 0.0)
}

fn corner_overlapping_rounded_rectangle_segments() -> Vec<ProfileSegment> {
    translated_containing_rounded_rectangle_segments(80.0, 50.0)
}

fn corner_overlapping_rounded_rectangle_planar_segments() -> Vec<PlanarProfileSegment> {
    corner_overlapping_rounded_rectangle_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                PlanarProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            },
        })
        .collect()
}

fn far_arc_clipped_corner_overlapping_rounded_rectangle_segments() -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::Line {
            start_mm: [80.0, 25.0],
            end_mm: [90.0, 25.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [90.0, 25.0],
            end_mm: [105.0, 40.0],
            center_mm: [90.0, 40.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [105.0, 40.0],
            end_mm: [105.0, 50.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [105.0, 50.0],
            end_mm: [90.0, 65.0],
            center_mm: [90.0, 50.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [90.0, 65.0],
            end_mm: [80.0, 65.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [80.0, 65.0],
            end_mm: [65.0, 50.0],
            center_mm: [80.0, 50.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [65.0, 50.0],
            end_mm: [65.0, 40.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [65.0, 40.0],
            end_mm: [80.0, 25.0],
            center_mm: [80.0, 40.0],
            clockwise: false,
        },
    ]
}

fn arc_clipped_corner_overlapping_rounded_rectangle_segments() -> Vec<ProfileSegment> {
    translated_containing_rounded_rectangle_segments(105.0, 55.0)
}

fn two_axis_arc_clipped_corner_overlapping_rounded_rectangle_segments() -> Vec<ProfileSegment> {
    translated_containing_rounded_rectangle_segments(105.0, 65.0)
}

fn two_axis_arc_clipped_corner_overlapping_rounded_rectangle_planar_segments()
-> Vec<PlanarProfileSegment> {
    two_axis_arc_clipped_corner_overlapping_rounded_rectangle_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                PlanarProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            },
        })
        .collect()
}

fn arc_clipped_corner_overlapping_rounded_rectangle_planar_segments() -> Vec<PlanarProfileSegment> {
    arc_clipped_corner_overlapping_rounded_rectangle_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                PlanarProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            },
        })
        .collect()
}

fn side_overlapping_rounded_rectangle_planar_segments() -> Vec<PlanarProfileSegment> {
    side_overlapping_rounded_rectangle_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                PlanarProfileSegment::Line { start_mm, end_mm }
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            },
        })
        .collect()
}

fn capsule_boolean_document(height: f64, operation: BooleanOperation) -> DocumentStore {
    capsule_boolean_document_with_segments(height, operation, capsule_segments())
}

fn capsule_boolean_document_with_segments(
    height: f64,
    operation: BooleanOperation,
    segments: Vec<ProfileSegment>,
) -> DocumentStore {
    let mut document = rectangle_document(100.0, 60.0, height);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Capsule cut profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "Capsule cut tool".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: CUT_PROFILE,
                    height: Dimension::new(height.to_string(), height).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: THROUGH_CUT,
                definition_id: DEFINITION,
                name: "Capsule boolean".to_owned(),
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

fn capsule_pocket_document(height: f64, pocket_depth: f64) -> DocumentStore {
    let mut document = rectangle_document(100.0, 60.0, height);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Capsule pocket profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: capsule_segments(),
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Capsule pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: EXTRUSION,
                    profile: CUT_PROFILE,
                    depth: Dimension::new(pocket_depth.to_string(), pocket_depth).unwrap(),
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn line_arc_d_pocket_document(height: f64, pocket_depth: f64) -> DocumentStore {
    let mut document = rectangle_document(100.0, 60.0, height);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "D pocket profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::CircularArc {
                            start_mm: [20.0, 20.0],
                            end_mm: [40.0, 20.0],
                            center_mm: [30.0, 20.0],
                            clockwise: true,
                        },
                        ProfileSegment::Line {
                            start_mm: [40.0, 20.0],
                            end_mm: [20.0, 20.0],
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "D pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: EXTRUSION,
                    profile: CUT_PROFILE,
                    depth: Dimension::new(pocket_depth.to_string(), pocket_depth).unwrap(),
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn polygon_cut_document(points: &[[f64; 2]], height: f64, closed: bool) -> DocumentStore {
    let mut document = rectangle_document(100.0, 60.0, height);
    let mut segments = points
        .windows(2)
        .map(|pair| ProfileSegment::Line {
            start_mm: pair[0],
            end_mm: pair[1],
        })
        .collect::<Vec<_>>();
    if closed {
        segments.push(ProfileSegment::Line {
            start_mm: *points.last().unwrap(),
            end_mm: points[0],
        });
    }
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Polygon cut profile".to_owned(),
                kind: FeatureKind::SegmentProfile { segments, closed },
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "Polygon cut tool".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: CUT_PROFILE,
                    height: Dimension::new(height.to_string(), height).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: THROUGH_CUT,
                definition_id: DEFINITION,
                name: "Polygon through cut".to_owned(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Cut,
                    target: EXTRUSION,
                    tool: TOOL_EXTRUSION,
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn polygon_boolean_document(
    points: &[[f64; 2]],
    height: f64,
    operation: BooleanOperation,
) -> DocumentStore {
    let mut document = rectangle_document(100.0, 60.0, height);
    let mut segments = points
        .windows(2)
        .map(|pair| ProfileSegment::Line {
            start_mm: pair[0],
            end_mm: pair[1],
        })
        .collect::<Vec<_>>();
    segments.push(ProfileSegment::Line {
        start_mm: *points.last().unwrap(),
        end_mm: points[0],
    });
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Polygon union profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "Polygon union tool".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: CUT_PROFILE,
                    height: Dimension::new(height.to_string(), height).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: THROUGH_CUT,
                definition_id: DEFINITION,
                name: "Contained polygon boolean".to_owned(),
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

fn polygon_pocket_document(points: &[[f64; 2]], height: f64, pocket_depth: f64) -> DocumentStore {
    let mut document = rectangle_document(100.0, 60.0, height);
    let mut segments = points
        .windows(2)
        .map(|pair| ProfileSegment::Line {
            start_mm: pair[0],
            end_mm: pair[1],
        })
        .collect::<Vec<_>>();
    segments.push(ProfileSegment::Line {
        start_mm: *points.last().unwrap(),
        end_mm: points[0],
    });
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Polygon pocket profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Polygon pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: EXTRUSION,
                    profile: CUT_PROFILE,
                    depth: Dimension::new(pocket_depth.to_string(), pocket_depth).unwrap(),
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn general_box_shell_document(finish: Option<BottleEdgeFinishKind>) -> DocumentStore {
    const DEFINITION: DefinitionId = DefinitionId(950);
    const PROFILE: FeatureId = FeatureId(951);
    const EXTRUSION: FeatureId = FeatureId(952);
    const SHELL: FeatureId = FeatureId(953);
    const FINISH: FeatureId = FeatureId(954);
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "General box shell".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: PROFILE,
            definition_id: DEFINITION,
            name: "Rectangle".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 50.0], [0.0, 50.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: EXTRUSION,
            definition_id: DEFINITION,
            name: "Box".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: PROFILE,
                height: Dimension::new("30", 30.0).unwrap(),
            },
        },
        CanonicalCommand::CreateFeature {
            id: SHELL,
            definition_id: DEFINITION,
            name: "Open top".to_owned(),
            kind: FeatureKind::Shell {
                target: EXTRUSION,
                removed_faces: vec![StableFaceRole::new("extrusion.top").unwrap()],
                thickness: Dimension::new("2", 2.0).unwrap(),
            },
        },
    ];
    if let Some(kind) = finish {
        commands.push(CanonicalCommand::CreateFeature {
            id: FINISH,
            definition_id: DEFINITION,
            name: "Top east finish".to_owned(),
            kind: FeatureKind::BottleEdgeFinish {
                target: SHELL,
                edges: vec![StableEdgeRole::new("shell.edge.top-east").unwrap()],
                kind,
                amount: Dimension::new("1", 1.0).unwrap(),
            },
        });
    }
    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document.discard_history_before_current();
    document
}

fn general_revolve_document(segment_profile: bool) -> DocumentStore {
    let mut document = DocumentStore::new();
    let profile_kind = if segment_profile {
        FeatureKind::SegmentProfile {
            segments: vec![
                ProfileSegment::Line {
                    start_mm: [10.0, 0.0],
                    end_mm: [20.0, 0.0],
                },
                ProfileSegment::CircularArc {
                    start_mm: [20.0, 0.0],
                    end_mm: [10.0, 0.0],
                    center_mm: [15.0, 0.0],
                    clockwise: false,
                },
            ],
            closed: true,
        }
    } else {
        FeatureKind::Profile {
            points_mm: vec![[10.0, 0.0], [20.0, 0.0], [20.0, 5.0], [10.0, 5.0]],
        }
    };
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(901),
                name: "General revolve".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(901),
                definition_id: DefinitionId(901),
                name: "Closed profile".to_owned(),
                kind: profile_kind,
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(902),
                definition_id: DefinitionId(901),
                name: "General revolve".to_owned(),
                kind: FeatureKind::Revolve {
                    profile: FeatureId(901),
                    axis_start_mm: [0.0, -10.0],
                    axis_end_mm: [0.0, 10.0],
                    angle_degrees: if segment_profile { 120.0 } else { 180.0 },
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(901),
                definition_id: DefinitionId(901),
                name: "General revolve occurrence".to_owned(),
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
        ExactFaceRole::CircleSide
        | ExactFaceRole::ArcSide
        | ExactFaceRole::LinearSide
        | ExactFaceRole::CutCircle
        | ExactFaceRole::CutLinear
        | ExactFaceRole::CutWest
        | ExactFaceRole::CutEast
        | ExactFaceRole::CutSouth
        | ExactFaceRole::CutNorth
        | ExactFaceRole::PocketFloor
        | ExactFaceRole::PocketWest
        | ExactFaceRole::PocketEast
        | ExactFaceRole::PocketSouth
        | ExactFaceRole::PocketNorth
        | ExactFaceRole::RevolveBottom
        | ExactFaceRole::RevolveBody
        | ExactFaceRole::RevolveShoulder
        | ExactFaceRole::RevolveNeck
        | ExactFaceRole::RevolveMouth
        | ExactFaceRole::RevolveSide0
        | ExactFaceRole::RevolveSide1
        | ExactFaceRole::RevolveStart
        | ExactFaceRole::RevolveEnd
        | ExactFaceRole::ShellOuterBottom
        | ExactFaceRole::ShellOuterBody
        | ExactFaceRole::ShellOuterShoulder
        | ExactFaceRole::ShellOuterNeck
        | ExactFaceRole::ShellRim
        | ExactFaceRole::ShellInnerBottom
        | ExactFaceRole::ShellInnerBody
        | ExactFaceRole::ShellInnerShoulder
        | ExactFaceRole::ShellInnerNeck
        | ExactFaceRole::BoxShellOuterBottom
        | ExactFaceRole::BoxShellOuterEast
        | ExactFaceRole::BoxShellRim
        | ExactFaceRole::PlanarOffsetFace
        | ExactFaceRole::SweepStart
        | ExactFaceRole::SweepEnd
        | ExactFaceRole::SweepSide0
        | ExactFaceRole::SweepSide1
        | ExactFaceRole::SweepSide2
        | ExactFaceRole::SweepSide3
        | ExactFaceRole::LoftStart
        | ExactFaceRole::LoftEnd
        | ExactFaceRole::LoftSide => {
            panic!("non-extrusion roles are outside the extrusion-only C1b corpus")
        }
    };
    Ray::new(origin, direction).unwrap()
}

fn worker_path() -> &'static str {
    env!("CARGO_BIN_EXE_ketchup-exact-worker")
}
